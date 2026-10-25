//! Platform integration kept outside document and rendering logic.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{
    activate_editor, configure_editor_backend, install_quit_handler, install_reopen_handler,
    request_native_quit, start_activation_server,
};

#[cfg(not(target_os = "macos"))]
pub fn configure_editor_backend() -> Result<(), String> {
    Ok(())
}
#[cfg(not(target_os = "macos"))]
pub fn install_quit_handler(_: slint::Weak<super::AppWindow>) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
pub struct InstallationGuard(*mut core::ffi::c_void);
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(
        attributes: *mut core::ffi::c_void,
        initial_owner: i32,
        name: *const u16,
    ) -> *mut core::ffi::c_void;
    fn CloseHandle(handle: *mut core::ffi::c_void) -> i32;
}
#[cfg(windows)]
impl Drop for InstallationGuard {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
/// Both tray and editor keep a reference. The installer never forcibly ends either process.
#[cfg(windows)]
pub fn installation_guard() -> Option<InstallationGuard> {
    let name: Vec<u16> = "Local\\ImageFeedback.Running"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let handle = unsafe { CreateMutexW(core::ptr::null_mut(), 0, name.as_ptr()) };
    if handle.is_null() {
        None
    } else {
        Some(InstallationGuard(handle))
    }
}
#[cfg(not(windows))]
pub fn installation_guard() {}
