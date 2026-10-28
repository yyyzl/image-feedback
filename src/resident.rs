//! Event-driven tray process. This branch never creates a Slint window or loads user pictures.
use std::{
    error::Error,
    fs::File,
    path::PathBuf,
    process::{Child, Command},
    time::{Duration, Instant},
};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::WindowId,
};
#[derive(Debug)]
enum Event {
    Open,
    Menu(MenuId),
    QuitResult(QuitResult),
}
#[derive(Debug)]
enum QuitOutcome {
    Closed(File),
    Cancelled,
    Busy,
    Failed(String),
}
#[derive(Debug)]
struct QuitResult {
    outcome: QuitOutcome,
    child: Option<Child>,
}
struct QuitSmoke {
    folder: PathBuf,
    expect_cancel: bool,
    saw_cancel: bool,
    open_editor: bool,
    open_annotated: bool,
}
struct Resident {
    tray: Option<TrayIcon>,
    open: Option<MenuId>,
    quit: Option<MenuId>,
    quit_item: Option<MenuItem>,
    hint: Option<MenuItem>,
    proxy: EventLoopProxy<Event>,
    quitting: bool,
    exit_lock: Option<File>,
    child: Option<Child>,
    open_initially: bool,
    smoke_output: Option<PathBuf>,
    quit_smoke: Option<QuitSmoke>,
    error: Option<String>,
}
const TEMPLATE_ICON: bool = cfg!(target_os = "macos");
fn icon_pixels() -> Result<image::RgbaImage, image::ImageError> {
    #[cfg(target_os = "macos")]
    let image =
        image::load_from_memory(include_bytes!("../assets/menubar-icon@2x.png"))?.into_rgba8();
    #[cfg(not(target_os = "macos"))]
    let image = image::load_from_memory(include_bytes!("../assets/app-icon.png"))?
        .resize_exact(32, 32, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    Ok(image)
}
fn icon() -> Result<Icon, Box<dyn Error>> {
    let image = icon_pixels()?;
    let (width, height) = image.dimensions();
    Ok(Icon::from_rgba(image.into_raw(), width, height)?)
}
impl Resident {
    fn open(&mut self) {
        if self.quitting {
            return;
        }
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
        match std::env::current_exe().and_then(|exe| {
            let mut command = Command::new(exe);
            command.arg("--editor");
            if self.quit_smoke.as_ref().is_some_and(|s| s.open_annotated) {
                command.arg("--demo");
            }
            command.spawn()
        }) {
            Ok(child) => self.child = Some(child),
            Err(e) => {
                self.error = Some(e.to_string());
            }
        }
    }
    fn quit(&mut self) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        if let Some(item) = self.quit_item.as_ref() {
            item.set_enabled(false);
        }
        if let Some(hint) = self.hint.as_ref() {
            hint.set_text("正在退出；如有批注，请在编辑器中确认");
        }
        let proxy = self.proxy.clone();
        let directory = super::app_dir();
        let child = self.child.take();
        std::thread::spawn(move || {
            let result = coordinate_quit(&directory, child);
            let _ = proxy.send_event(Event::QuitResult(result));
        });
    }
    fn quit_result(&mut self, event_loop: &ActiveEventLoop, result: QuitResult) {
        let QuitResult { outcome, child } = result;
        self.child = child;
        let mut repeat = false;
        if let Some(smoke) = self.quit_smoke.as_mut() {
            let result = match &outcome {
                QuitOutcome::Cancelled if smoke.expect_cancel && !smoke.saw_cancel => {
                    smoke.saw_cancel = true;
                    repeat = true;
                    std::fs::write(smoke.folder.join("cancelled.txt"), "cancelled")
                        .map_err(|e| e.to_string())
                }
                QuitOutcome::Closed(_) if smoke.expect_cancel == smoke.saw_cancel => {
                    let report = serde_json::json!({
                        "result": "passed",
                        "platform": std::env::consts::OS,
                        "cancel_preserved_resident": smoke.saw_cancel,
                        "started_editor": smoke.open_editor,
                        "annotated_demo": smoke.open_annotated,
                        "method": "real resident and editor processes; the native tray menu event is dispatched by the smoke scenario"
                    });
                    serde_json::to_vec_pretty(&report)
                        .map_err(|e| e.to_string())
                        .and_then(|data| {
                            std::fs::write(smoke.folder.join("quit-report.json"), data)
                                .map_err(|e| e.to_string())
                        })
                }
                other => Err(format!("unexpected quit result: {other:?}")),
            };
            if let Err(error) = result {
                let _ = std::fs::write(smoke.folder.join("failed.txt"), &error);
                self.error = Some(error);
                self.tray.take();
                event_loop.exit();
                return;
            }
        }
        match outcome {
            QuitOutcome::Closed(lock) => {
                self.exit_lock = Some(lock);
                self.tray.take();
                event_loop.exit();
            }
            other => {
                self.quitting = false;
                if let Some(item) = self.quit_item.as_ref() {
                    item.set_enabled(true);
                }
                if let Some(hint) = self.hint.as_ref() {
                    hint.set_text(match other {
                        QuitOutcome::Cancelled => "已取消退出，编辑器和常驻入口仍保留",
                        QuitOutcome::Busy => "请先完成编辑器中的操作，再退出",
                        QuitOutcome::Failed(ref error) => {
                            eprintln!("退出图片反馈失败：{error}");
                            "退出未完成，请先关闭编辑器后重试"
                        }
                        QuitOutcome::Closed(_) => unreachable!(),
                    });
                }
            }
        }
        if repeat {
            if let Some(id) = self.quit.clone() {
                let _ = self.proxy.send_event(Event::Menu(id));
            }
        }
    }
}

fn coordinate_quit(directory: &std::path::Path, mut child: Option<Child>) -> QuitResult {
    let mut deadline = Instant::now() + Duration::from_secs(5);
    let mut closing = false;
    let outcome = loop {
        let child_running = match child.as_mut().map(Child::try_wait) {
            Some(Ok(None)) => true,
            Some(Ok(Some(_))) => {
                child = None;
                false
            }
            Some(Err(error)) => break QuitOutcome::Failed(error.to_string()),
            None => false,
        };
        match super::quit_ipc::try_editor_lock(directory) {
            Ok(Some(lock)) if !child_running => break QuitOutcome::Closed(lock),
            Ok(Some(lock)) => {
                // A just-spawned editor may not have acquired its lock yet. Do not
                // hold the lock while waiting for it to start or finish exiting.
                drop(lock);
            }
            Ok(None) if !closing => match super::quit_ipc::request(directory) {
                Ok(super::quit_ipc::Decision::Cancelled) => break QuitOutcome::Cancelled,
                Ok(super::quit_ipc::Decision::Busy) => break QuitOutcome::Busy,
                Ok(super::quit_ipc::Decision::Closing) => {
                    closing = true;
                    deadline = Instant::now() + Duration::from_secs(10);
                }
                Err(_) => {}
            },
            Ok(None) => {}
            Err(error) => break QuitOutcome::Failed(error),
        }
        if Instant::now() >= deadline {
            break QuitOutcome::Failed(if closing {
                "编辑器未能完成关闭".into()
            } else {
                "无法连接仍在运行的编辑器".into()
            });
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    QuitResult { outcome, child }
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
            let quit = MenuItem::new("退出图片反馈", true, None);
            self.open = Some(open.id().clone());
            self.quit = Some(quit.id().clone());
            menu.append_items(&[&open, &hint, &PredefinedMenuItem::separator(), &quit])?;
            self.hint = Some(hint);
            self.quit_item = Some(quit);
            self.tray = Some(
                TrayIconBuilder::new()
                    .with_tooltip("图片反馈 · 轻量常驻入口")
                    .with_icon(icon()?)
                    .with_icon_as_template(TEMPLATE_ICON)
                    .with_menu(Box::new(menu))
                    .with_menu_on_left_click(false)
                    .build()?,
            );
            if let Some(folder) = self.smoke_output.as_ref() {
                #[cfg(target_os = "macos")]
                super::platform::verify_menu_bar_template(self.tray.as_ref().unwrap())?;
                let pixels = icon_pixels()?;
                std::fs::create_dir_all(folder)?;
                let report = serde_json::json!({
                    "result": "passed",
                    "platform": std::env::consts::OS,
                    "template_expected": TEMPLATE_ICON,
                    "native_template_checked": cfg!(target_os = "macos"),
                    "pixel_dimensions": [pixels.width(), pixels.height()],
                    "method": "real native tray creation; macOS reads NSImage.isTemplate",
                    "limitations": "Not a screenshot or visual dark/light mode automation"
                });
                std::fs::write(
                    folder.join("tray-report.json"),
                    serde_json::to_vec_pretty(&report)?,
                )?;
                self.tray.take();
                event_loop.exit();
            } else if let Some(smoke) = self.quit_smoke.as_ref() {
                std::fs::create_dir_all(&smoke.folder)?;
                let open_editor = smoke.open_editor;
                let open_annotated = smoke.open_annotated;
                if open_editor {
                    self.open();
                }
                let proxy = self.proxy.clone();
                let id = self.quit.clone().unwrap();
                if open_annotated {
                    // The demo loads asynchronously; the test must not quit an empty editor.
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs(5));
                        let _ = proxy.send_event(Event::Menu(id));
                    });
                } else {
                    let _ = proxy.send_event(Event::Menu(id));
                }
            } else if self.open_initially {
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
                    self.quit();
                }
            }
            Event::QuitResult(result) => self.quit_result(event_loop, result),
        }
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}
pub fn run(open_initially: bool) -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let smoke_output = args
        .iter()
        .position(|a| a == "--tray-smoke")
        .map(|i| {
            args.get(i + 1)
                .map(PathBuf::from)
                .ok_or("--tray-smoke requires an output directory")
        })
        .transpose()?;
    let quit_smoke = args
        .iter()
        .position(|a| a == "--tray-quit-smoke")
        .map(|i| {
            args.get(i + 1)
                .map(|folder| QuitSmoke {
                    folder: PathBuf::from(folder),
                    expect_cancel: args.iter().any(|a| a == "--expect-quit-cancel"),
                    saw_cancel: false,
                    open_editor: args.iter().any(|a| {
                        a == "--quit-smoke-open-editor" || a == "--quit-smoke-open-annotated"
                    }),
                    open_annotated: args.iter().any(|a| a == "--quit-smoke-open-annotated"),
                })
                .ok_or("--tray-quit-smoke requires an output directory")
        })
        .transpose()?;
    if quit_smoke.is_some() && smoke_output.is_some() {
        return Err("Choose one tray smoke scenario".into());
    }
    std::fs::create_dir_all(super::app_dir())?;
    let lock = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(super::app_dir().join("resident.lock"))?;
    if lock.try_lock().is_err() {
        if smoke_output.is_some() || quit_smoke.is_some() {
            return Err("A resident is already running; it was preserved".into());
        }
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
        quit_item: None,
        hint: None,
        proxy: events.create_proxy(),
        quitting: false,
        exit_lock: None,
        child: None,
        open_initially,
        smoke_output,
        quit_smoke,
        error: None,
    };
    events.run_app(&mut app)?;
    // The editor is never killed: exit only follows a clean close or explicit discard.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_bar_assets_are_transparent_monochrome_at_both_scales() {
        for (bytes, size) in [
            (include_bytes!("../assets/menubar-icon.png").as_slice(), 18),
            (
                include_bytes!("../assets/menubar-icon@2x.png").as_slice(),
                36,
            ),
        ] {
            let pixels = image::load_from_memory(bytes).unwrap().into_rgba8();
            assert_eq!(pixels.dimensions(), (size, size));
            assert!(pixels.pixels().all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0));
            assert!(pixels.pixels().any(|p| p[3] == 255));
            assert!(pixels.pixels().any(|p| p[3] > 0 && p[3] < 255));
            for (x, y) in [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)] {
                assert_eq!(pixels.get_pixel(x, y)[3], 0);
            }
        }
    }

    #[test]
    fn only_macos_uses_the_template_resource() {
        let pixels = icon_pixels().unwrap();
        if cfg!(target_os = "macos") {
            assert!(TEMPLATE_ICON);
            assert_eq!(pixels.dimensions(), (36, 36));
            assert!(pixels.pixels().all(|p| p[0] == p[1] && p[1] == p[2]));
        } else {
            assert!(!TEMPLATE_ICON);
            assert_eq!(pixels.dimensions(), (32, 32));
            assert!(
                pixels
                    .pixels()
                    .any(|p| p[3] != 0 && (p[0] != p[1] || p[1] != p[2]))
            );
        }
    }
}
