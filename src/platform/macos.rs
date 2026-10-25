//! Native AppKit integration without replacing winit's application delegate.
//! Runtime-added delegate methods forward app Quit into the same unsaved-change guard
//! as the native red close button. No vendored or modified cargo-registry sources.
use crate::AppWindow;
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::{
    cell::{Cell, RefCell},
    ffi::{c_char, c_void},
    io::{Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
type Obj = *mut c_void;
#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Obj;
    fn sel_registerName(name: *const c_char) -> Obj;
    fn object_getClass(object: Obj) -> Obj;
    fn class_addMethod(
        class: Obj,
        selector: Obj,
        implementation: *const c_void,
        types: *const c_char,
    ) -> bool;
    fn objc_msgSend();
}
unsafe extern "C" {
    fn getuid() -> u32;
}
thread_local! {
    static QUIT:RefCell<Option<Box<dyn Fn()>>>=RefCell::new(None);
    static REOPEN:RefCell<Option<Box<dyn Fn()>>>=RefCell::new(None);
    static QUIT_INSTALLED:Cell<bool>=const {Cell::new(false)};
    static REOPEN_INSTALLED:Cell<bool>=const {Cell::new(false)};
}
// Fixed AppKit selectors only, called on the macOS main/UI thread.
unsafe fn send_object(receiver: Obj, selector: &'static [u8]) -> Obj {
    let send: unsafe extern "C" fn(Obj, Obj) -> Obj =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { send(receiver, sel_registerName(selector.as_ptr().cast())) }
}
unsafe fn send_object_argument(receiver: Obj, selector: &'static [u8], argument: Obj) {
    let send: unsafe extern "C" fn(Obj, Obj, Obj) =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe {
        send(
            receiver,
            sel_registerName(selector.as_ptr().cast()),
            argument,
        )
    };
}
unsafe fn application() -> Obj {
    unsafe {
        send_object(
            objc_getClass(c"NSApplication".as_ptr()),
            b"sharedApplication\0",
        )
    }
}
unsafe extern "C" fn should_terminate(_: Obj, _: Obj, _: Obj) -> usize {
    QUIT.with(|handler| {
        if let Some(handler) = handler.borrow().as_ref() {
            handler();
        }
    });
    // NSTerminateCancel: completion of the guarded window close ends the Rust event loop.
    // Cancel also correctly vetoes logout when the user cancels saving.
    0
}
unsafe extern "C" fn should_reopen(_: Obj, _: Obj, _: Obj, _: bool) -> bool {
    REOPEN.with(|handler| {
        if let Some(handler) = handler.borrow().as_ref() {
            handler();
        }
    });
    false
}
fn add_delegate_method(
    selector: &'static [u8],
    implementation: *const c_void,
    types: &'static [u8],
) -> Result<(), String> {
    unsafe {
        let app = application();
        let delegate = send_object(app, b"delegate\0");
        if app.is_null() || delegate.is_null() {
            return Err("AppKit/winit delegate is not ready".into());
        }
        if !class_addMethod(
            object_getClass(delegate),
            sel_registerName(selector.as_ptr().cast()),
            implementation,
            types.as_ptr().cast(),
        ) {
            return Err(
                "The pinned winit delegate already implements this AppKit lifecycle method".into(),
            );
        }
        // Refresh AppKit's optional-selector cache with the SAME delegate object.
        // Replacing it would violate winit's delegate downcast invariant.
        send_object_argument(app, b"setDelegate:\0", delegate);
    }
    Ok(())
}
pub fn configure_editor_backend() -> Result<(), String> {
    static READY: OnceLock<Result<(), String>> = OnceLock::new();
    READY
        .get_or_init(|| {
            use slint::winit_030::{SlintEvent, winit};
            use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
            let mut builder = winit::event_loop::EventLoop::<SlintEvent>::with_user_event();
            builder.with_activation_policy(ActivationPolicy::Regular);
            slint::BackendSelector::new()
                .with_winit_event_loop_builder(builder)
                .select()
                .map_err(|e| e.to_string())
        })
        .clone()
}
pub fn install_quit_handler(weak: slint::Weak<AppWindow>) -> Result<(), String> {
    QUIT.with(|handler| {
        *handler.borrow_mut() = Some(Box::new(move || {
            if let Some(ui) = weak.upgrade() {
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::CloseRequested);
            }
        }))
    });
    if !QUIT_INSTALLED.with(Cell::get) {
        add_delegate_method(
            b"applicationShouldTerminate:\0",
            should_terminate as *const () as *const c_void,
            b"Q@:@\0",
        )?;
        QUIT_INSTALLED.with(|value| value.set(true));
    }
    Ok(())
}
pub fn install_reopen_handler(handler: impl Fn() + 'static) -> Result<(), String> {
    REOPEN.with(|value| *value.borrow_mut() = Some(Box::new(handler)));
    if !REOPEN_INSTALLED.with(Cell::get) {
        add_delegate_method(
            b"applicationShouldHandleReopen:hasVisibleWindows:\0",
            should_reopen as *const () as *const c_void,
            b"B@:@B\0",
        )?;
        REOPEN_INSTALLED.with(|value| value.set(true));
    }
    Ok(())
}
/// Inspect the actual status-item image on the resident's main thread.
pub fn verify_menu_bar_template(tray: &tray_icon::TrayIcon) -> Result<(), String> {
    let status = tray
        .ns_status_item()
        .ok_or("Native menu-bar item is missing")?;
    unsafe {
        let item = std::ptr::from_ref(&*status).cast_mut().cast::<c_void>();
        let button = send_object(item, b"button\0");
        if button.is_null() {
            return Err("Native menu-bar button is missing".into());
        }
        let image = send_object(button, b"image\0");
        if image.is_null() {
            return Err("Native menu-bar image is missing".into());
        }
        // AppKit BOOL occupies one byte on both supported macOS architectures.
        let is_template: unsafe extern "C" fn(Obj, Obj) -> u8 =
            std::mem::transmute(objc_msgSend as *const ());
        if is_template(image, sel_registerName(c"isTemplate".as_ptr())) == 0 {
            return Err("Menu-bar NSImage was not configured as a template".into());
        }
    }
    Ok(())
}
/// Tests the real AppKit Quit entry, not just a synthetic keyboard event.
pub fn request_native_quit() {
    unsafe {
        send_object_argument(application(), b"terminate:\0", core::ptr::null_mut());
    }
}
fn socket_path() -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    crate::app_dir().hash(&mut hash);
    std::env::temp_dir().join(format!(
        "if-{}-{:x}.sock",
        unsafe { getuid() },
        hash.finish()
    ))
}
pub fn activate_editor() -> bool {
    let Ok(mut stream) = UnixStream::connect(socket_path()) else {
        return false;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    stream.write_all(b"activate\n").is_ok()
}
pub struct ActivationServer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for ActivationServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let awakened = UnixStream::connect(&self.path).is_ok();
        if let Some(thread) = self.thread.take() {
            // If an external temp cleaner removed the endpoint, never hang app quit.
            if awakened || thread.is_finished() {
                let _ = thread.join();
            }
        }
        // Only the private socket created while holding editor.lock is removed.
        if let Ok(meta) = std::fs::symlink_metadata(&self.path) {
            if meta.file_type().is_socket() && meta.uid() == unsafe { getuid() } {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}
pub fn start_activation_server(weak: slint::Weak<AppWindow>) -> Result<ActivationServer, String> {
    let path = socket_path();
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        if !meta.file_type().is_socket() || meta.uid() != unsafe { getuid() } {
            return Err("Unsafe editor activation endpoint".into());
        }
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let thread = std::thread::spawn(move || {
        for connection in listener.incoming() {
            if stopped.load(Ordering::Acquire) {
                break;
            }
            let Ok(mut stream) = connection else {
                break;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            let mut command = [0u8; 9];
            if stream.read_exact(&mut command).is_ok() && &command == b"activate\n" {
                let _ = weak.upgrade_in_event_loop(|ui| {
                    let _ = ui.show();
                    ui.window().with_winit_window(|window| {
                        window.set_visible(true);
                        window.set_minimized(false);
                        window.focus_window();
                    });
                });
            }
        }
    });
    Ok(ActivationServer {
        path,
        stop,
        thread: Some(thread),
    })
}
