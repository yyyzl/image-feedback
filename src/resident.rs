//! Event-driven tray process. This branch never creates a Slint window or loads user pictures.
use std::{
    error::Error,
    process::{Child, Command},
};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::WindowId,
};
#[derive(Debug)]
enum Event {
    Open,
    Menu(MenuId),
}
struct Resident {
    tray: Option<TrayIcon>,
    open: Option<MenuId>,
    quit: Option<MenuId>,
    child: Option<Child>,
    open_initially: bool,
    error: Option<String>,
}
fn icon() -> Result<Icon, Box<dyn Error>> {
    let image = image::load_from_memory(include_bytes!("../assets/app-icon.png"))?
        .resize_exact(32, 32, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    Ok(Icon::from_rgba(image.into_raw(), 32, 32)?)
}
impl Resident {
    fn open(&mut self) {
        #[cfg(target_os = "macos")]
        if super::platform::activate_editor() {
            return;
        }
        if let Some(child) = self.child.as_mut() {
            if let Ok(None) = child.try_wait() {
                focus_pid(child.id());
                return;
            }
        }
        match std::env::current_exe().and_then(|exe| Command::new(exe).arg("--editor").spawn()) {
            Ok(child) => self.child = Some(child),
            Err(e) => {
                self.error = Some(e.to_string());
            }
        }
    }
}
impl ApplicationHandler<Event> for Resident {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.tray.is_some() {
            return;
        }
        let result = (|| -> Result<(), Box<dyn Error>> {
            let menu = Menu::new();
            let open = MenuItem::new("打开图片反馈", true, None);
            let hint = MenuItem::new("关闭编辑器后，常驻入口仍保留", false, None);
            let quit = MenuItem::new("退出常驻入口", true, None);
            self.open = Some(open.id().clone());
            self.quit = Some(quit.id().clone());
            menu.append_items(&[&open, &hint, &PredefinedMenuItem::separator(), &quit])?;
            self.tray = Some(
                TrayIconBuilder::new()
                    .with_tooltip("图片反馈 · 轻量常驻入口")
                    .with_icon(icon()?)
                    .with_menu(Box::new(menu))
                    .with_menu_on_left_click(false)
                    .build()?,
            );
            if self.open_initially {
                self.open();
            }
            Ok(())
        })();
        if let Err(err) = result {
            self.error = Some(err.to_string());
            event_loop.exit();
        }
    }
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        match event {
            Event::Open => self.open(),
            Event::Menu(id) => {
                if self.open.as_ref() == Some(&id) {
                    self.open();
                } else if self.quit.as_ref() == Some(&id) {
                    self.tray.take();
                    event_loop.exit();
                }
            }
        }
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}
pub fn run(open_initially: bool) -> Result<(), Box<dyn Error>> {
    std::fs::create_dir_all(super::app_dir())?;
    let lock = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(super::app_dir().join("resident.lock"))?;
    if lock.try_lock().is_err() {
        #[cfg(target_os = "macos")]
        if super::platform::activate_editor() {
            return Ok(());
        }
        let _ = Command::new(std::env::current_exe()?)
            .arg("--editor")
            .spawn()?;
        return Ok(());
    }
    let mut builder = EventLoop::<Event>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder
            .with_activation_policy(ActivationPolicy::Accessory)
            .with_activate_ignoring_other_apps(false)
            .with_default_menu(false);
    }
    let events = builder.build()?;
    #[cfg(target_os = "macos")]
    {
        let proxy = events.create_proxy();
        super::platform::install_reopen_handler(move || {
            let _ = proxy.send_event(Event::Open);
        })?;
    }
    events.set_control_flow(ControlFlow::Wait);
    let proxy = events.create_proxy();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let _ = proxy.send_event(Event::Menu(e.id));
    }));
    let proxy = events.create_proxy();
    TrayIconEvent::set_event_handler(Some(move |e| {
        if matches!(
            e,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            }
        ) {
            let _ = proxy.send_event(Event::Open);
        }
    }));
    let mut app = Resident {
        tray: None,
        open: None,
        quit: None,
        child: None,
        open_initially,
        error: None,
    };
    events.run_app(&mut app)?;
    // Child is not killed on tray exit: it may contain unsaved changes.
    if let Some(err) = app.error {
        return Err(err.into());
    }
    drop(lock);
    Ok(())
}

#[cfg(windows)]
pub fn focus_pid(pid: u32) {
    type Hwnd = *mut core::ffi::c_void;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(callback: unsafe extern "system" fn(Hwnd, isize) -> i32, data: isize)
        -> i32;
        fn GetWindowThreadProcessId(window: Hwnd, pid: *mut u32) -> u32;
        fn IsWindowVisible(window: Hwnd) -> i32;
        fn ShowWindow(window: Hwnd, cmd: i32) -> i32;
        fn SetForegroundWindow(window: Hwnd) -> i32;
    }
    unsafe extern "system" fn visit(hwnd: Hwnd, pid: isize) -> i32 {
        let mut owner = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut owner);
            if owner == pid as u32 && IsWindowVisible(hwnd) != 0 {
                ShowWindow(hwnd, 9);
                SetForegroundWindow(hwnd);
                return 0;
            }
        }
        1
    }
    unsafe {
        EnumWindows(visit, pid as isize);
    }
}
#[cfg(not(windows))]
pub fn focus_pid(_pid: u32) {}
