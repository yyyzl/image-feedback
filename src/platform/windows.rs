//! 使用 Windows Direct Manipulation 接收精准触控板平移，机械滚轮仍交给 winit。
//! 只为本编辑窗口建立视口，不注册全局钩子、不修改系统触控板设置。
use crate::AppWindow;
use slint::ComponentHandle;
use std::{
    cell::Cell,
    rc::{Rc, Weak},
    sync::{Arc, Mutex},
    time::Duration,
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{DirectManipulation::*, Gdi::ScreenToClient},
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        UI::{
            Input::Pointer::GetPointerType,
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                DM_POINTERHITTEST, GetClientRect, GetCursorPos, PT_POINTER, PT_TOUCHPAD,
                WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEHWHEEL,
                WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_SIZE, WM_SYSKEYDOWN,
            },
        },
    },
    core::{Ref, Result, implement},
};

const SUBCLASS_ID: usize = 0x49465054;
#[derive(Default)]
struct Motion {
    previous: [f32; 2],
    delta: [f32; 2],
    active: bool,
}
impl Motion {
    fn update(&mut self, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        if self.active {
            self.delta[0] += x - self.previous[0];
            self.delta[1] += y - self.previous[1];
        }
        self.previous = [x, y];
    }
}
#[implement(IDirectManipulationViewportEventHandler)]
struct PanEvents {
    motion: Arc<Mutex<Motion>>,
}
impl IDirectManipulationViewportEventHandler_Impl for PanEvents_Impl {
    fn OnViewportStatusChanged(
        &self,
        _: Ref<IDirectManipulationViewport>,
        current: DIRECTMANIPULATION_STATUS,
        _: DIRECTMANIPULATION_STATUS,
    ) -> Result<()> {
        self.motion.lock().unwrap().active =
            current == DIRECTMANIPULATION_RUNNING || current == DIRECTMANIPULATION_INERTIA;
        Ok(())
    }
    fn OnViewportUpdated(&self, _: Ref<IDirectManipulationViewport>) -> Result<()> {
        Ok(())
    }
    fn OnContentUpdated(
        &self,
        _: Ref<IDirectManipulationViewport>,
        content: Ref<IDirectManipulationContent>,
    ) -> Result<()> {
        if let Some(content) = content.as_ref() {
            let mut matrix = [0.; 6];
            unsafe {
                content.GetContentTransform(&mut matrix)?;
            }
            self.motion.lock().unwrap().update(matrix[4], matrix[5]);
        }
        Ok(())
    }
}
struct Apartment(bool);
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

pub struct TouchpadInput {
    ui: slint::Weak<AppWindow>,
    hwnd: HWND,
    manager: IDirectManipulationManager,
    viewport: IDirectManipulationViewport,
    content: IDirectManipulationContent,
    updates: IDirectManipulationUpdateManager,
    cookie: u32,
    motion: Arc<Mutex<Motion>>,
    timer: slint::Timer,
    subclass_data: Cell<usize>,
    // 最后释放 COM apartment，确保所有 COM 接口已先释放。
    _apartment: Apartment,
}
impl TouchpadInput {
    pub fn install(
        ui: &AppWindow,
        window: &slint::winit_030::winit::window::Window,
    ) -> std::result::Result<Rc<Self>, String> {
        use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = window.window_handle().map_err(|e| e.to_string())?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("原生触控板需要 Win32 窗口".into());
        };
        let hwnd = HWND(handle.hwnd.get() as *mut _);
        let apartment =
            Apartment(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok());
        let initialized = (|| -> Result<Self> {
            let manager: IDirectManipulationManager = unsafe {
                CoCreateInstance(&DirectManipulationManager, None, CLSCTX_INPROC_SERVER)?
            };
            let updates = unsafe { manager.GetUpdateManager()? };
            let viewport: IDirectManipulationViewport =
                unsafe { manager.CreateViewport(None, hwnd)? };
            let content: IDirectManipulationContent = unsafe { viewport.GetPrimaryContent()? };
            unsafe {
                viewport.SetViewportOptions(
                    DIRECTMANIPULATION_VIEWPORT_OPTIONS_MANUALUPDATE
                        | DIRECTMANIPULATION_VIEWPORT_OPTIONS_DISABLEPIXELSNAPPING,
                )?;
                viewport.ActivateConfiguration(
                    DIRECTMANIPULATION_CONFIGURATION_INTERACTION
                        | DIRECTMANIPULATION_CONFIGURATION_TRANSLATION_X
                        | DIRECTMANIPULATION_CONFIGURATION_TRANSLATION_Y
                        | DIRECTMANIPULATION_CONFIGURATION_TRANSLATION_INERTIA,
                )?;
                // 足够大的内容范围让图片在两个方向自由平移，实际图片坐标仍由编辑器保存。
                content.SetContentRect(&RECT {
                    left: -10_000_000,
                    top: -10_000_000,
                    right: 10_000_000,
                    bottom: 10_000_000,
                })?;
            }
            let motion = Arc::new(Mutex::new(Motion::default()));
            let handler: IDirectManipulationViewportEventHandler = PanEvents {
                motion: motion.clone(),
            }
            .into();
            let cookie = unsafe { viewport.AddEventHandler(Some(hwnd), &handler)? };
            unsafe {
                let mut rect = RECT::default();
                GetClientRect(hwnd, &mut rect)?;
                viewport.SetViewportRect(&rect)?;
                viewport.Enable()?;
                manager.Activate(hwnd)?;
            }
            Ok(Self {
                ui: ui.as_weak(),
                hwnd,
                manager,
                viewport,
                content,
                updates,
                cookie,
                motion,
                timer: slint::Timer::default(),
                subclass_data: Cell::new(0),
                _apartment: apartment,
            })
        })()
        .map_err(|e| format!("无法启用原生触控板：{e}"))?;
        let bridge = Rc::new(initialized);
        let data = Box::into_raw(Box::new(Rc::downgrade(&bridge))) as usize;
        bridge.subclass_data.set(data);
        if !unsafe { SetWindowSubclass(hwnd, Some(window_message), SUBCLASS_ID, data) }.as_bool() {
            return Err("无法接入原生触控板窗口消息".into());
        }
        Ok(bridge)
    }
    fn contact(self: &Rc<Self>, pointer: u32) -> bool {
        let Some(ui) = self.ui.upgrade() else {
            return false;
        };
        let mut kind = PT_POINTER;
        let mut cursor = POINT::default();
        unsafe {
            if GetPointerType(pointer, &mut kind).is_err()
                || kind != PT_TOUCHPAD
                || GetCursorPos(&mut cursor).is_err()
                || !ScreenToClient(self.hwnd, &mut cursor).as_bool()
            {
                return false;
            }
        }
        let scale = ui.window().scale_factor();
        if !ui.invoke_canvas_accepts_scroll(cursor.x as f32 / scale, cursor.y as f32 / scale) {
            return false;
        }
        let mut matrix = [0.; 6];
        unsafe {
            if self.content.GetContentTransform(&mut matrix).is_err() {
                return false;
            }
        }
        {
            let mut motion = self.motion.lock().unwrap();
            motion.previous = [matrix[4], matrix[5]];
            motion.active = true;
        }
        if unsafe { self.viewport.SetContact(pointer) }.is_err() {
            return false;
        }
        if !self.timer.running() {
            let weak = Rc::downgrade(self);
            self.timer.start(
                slint::TimerMode::Repeated,
                Duration::from_millis(16),
                move || {
                    if let Some(bridge) = weak.upgrade() {
                        bridge.update();
                    }
                },
            );
        }
        true
    }
    fn update(&self) {
        if unsafe { self.updates.Update(None) }.is_err() {
            self.stop();
            return;
        }
        let (dx, dy, active) = {
            let mut motion = self.motion.lock().unwrap();
            let delta = std::mem::take(&mut motion.delta);
            (delta[0], delta[1], motion.active)
        };
        if let Some(ui) = self.ui.upgrade() {
            if dx != 0. || dy != 0. {
                let scale = ui.window().scale_factor();
                ui.invoke_canvas_wheel(dx / scale, dy / scale, 0., 0., true);
            }
        } else {
            self.stop();
            return;
        }
        if !active {
            self.timer.stop();
        }
    }
    fn stop(&self) {
        self.timer.stop();
        let _ = unsafe { self.viewport.Stop() };
        let mut motion = self.motion.lock().unwrap();
        motion.active = false;
        motion.delta = [0., 0.];
    }
    fn resize(&self) {
        // 视口覆盖客户区以接收命中测试；是否接收手势由 contact 的画布区域判断决定。
        let mut rect = RECT::default();
        unsafe {
            if GetClientRect(self.hwnd, &mut rect).is_ok() && rect.right > 0 && rect.bottom > 0 {
                let _ = self.viewport.SetViewportRect(&rect);
            }
        }
    }
}
impl Drop for TouchpadInput {
    fn drop(&mut self) {
        self.stop();
        unsafe {
            let _ = RemoveWindowSubclass(self.hwnd, Some(window_message), SUBCLASS_ID);
            if self.subclass_data.get() != 0 {
                drop(Box::from_raw(self.subclass_data.get() as *mut Weak<Self>));
            }
            let _ = self.viewport.RemoveEventHandler(self.cookie);
            let _ = self.viewport.Abandon();
            let _ = self.manager.Deactivate(self.hwnd);
        }
    }
}
unsafe extern "system" fn window_message(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    // data 由 install 分配，并在解除本窗口子类后释放；只在窗口所属 UI 线程读取。
    let weak = unsafe { &*(data as *const Weak<TouchpadInput>) };
    if let Some(bridge) = weak.upgrade() {
        if message == DM_POINTERHITTEST && bridge.contact(wparam.0 as u32 & 0xffff) {
            return LRESULT(0);
        }
        // 显式操作优先于尚未结束的惯性，避免点击回退后图片继续漂移。
        if matches!(
            message,
            WM_KILLFOCUS
                | WM_LBUTTONDOWN
                | WM_MBUTTONDOWN
                | WM_RBUTTONDOWN
                | WM_MOUSEWHEEL
                | WM_MOUSEHWHEEL
                | WM_KEYDOWN
                | WM_SYSKEYDOWN
        ) {
            bridge.stop();
        }
        if message == WM_SIZE {
            bridge.stop();
            bridge.resize();
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::Motion;
    #[test]
    fn native_pan_keeps_both_axes_and_ignores_idle_transform_resets() {
        let mut motion = Motion::default();
        motion.update(12., -8.);
        assert_eq!(motion.delta, [0., 0.]);
        motion.active = true;
        motion.update(17., -11.);
        assert_eq!(motion.delta, [5., -3.]);
        motion.update(f32::NAN, 10.);
        assert_eq!(motion.delta, [5., -3.]);
        motion.active = false;
        motion.update(0., 0.);
        assert_eq!(motion.delta, [5., -3.]);
    }
}
