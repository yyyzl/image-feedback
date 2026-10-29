//! Explicit Windows installer handoff. A copied helper survives editor shutdown.
//! It never kills an editor/resident and never installs before a committed handoff.
use image_feedback_slint::update::VerifiedDownload;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(25);
#[derive(Serialize, Deserialize)]
struct Plan {
    package: VerifiedDownload,
    directory: PathBuf,
    original_exe: PathBuf,
    editor_pid: u32,
}

/// Held briefly during ordinary startup, or throughout update exit coordination.
pub fn startup_gate(directory: &Path) -> Result<Option<File>, String> {
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    try_lock(&directory.join("update-install.lock"))
}
fn try_lock(path: &Path) -> Result<Option<File>, String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| e.to_string())?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Keep this handle open until the installer has been launched. On Windows it
/// denies writes/deletion, closing the verify-to-execute replacement window.
fn verify(package: &VerifiedDownload, directory: &Path) -> Result<File, String> {
    if package.sha256.len() != 64 || !package.sha256.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("更新校验记录无效，请重新下载".into());
    }
    let path = package
        .path
        .canonicalize()
        .map_err(|_| "安装包已被移动或删除，请重新下载")?;
    let root = directory
        .join("updates")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !path.starts_with(&root) || path.parent().and_then(Path::parent) != Some(root.as_path()) {
        return Err("安装包不在应用的更新下载目录内".into());
    }
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("安装包名称无效")?;
    let version = name
        .strip_prefix("ImageFeedback-")
        .and_then(|s| s.strip_suffix("-windows-x64-setup.exe"))
        .and_then(|s| semver::Version::parse(s).ok())
        .ok_or("不是受支持的 Windows 安装包")?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err("不是正式版安装包".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1); // FILE_SHARE_READ only
    }
    let mut file = options
        .open(&path)
        .map_err(|e| format!("无法读取安装包：{e}"))?;
    if file.metadata().map_err(|e| e.to_string())?.len() != package.size {
        return Err("安装包大小已改变，请重新下载".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    if format!("{:x}", hash.finalize()) != package.sha256.to_ascii_lowercase() {
        return Err("安装包校验失败，已阻止安装；请重新下载".into());
    }
    Ok(file)
}

pub struct Prepared {
    child: Child,
    input: Option<ChildStdin>,
    committed: bool,
}
impl Prepared {
    /// Only called after the user's feedback decision and successful preflight.
    pub fn commit(mut self) -> Result<(), String> {
        self.input
            .as_mut()
            .ok_or("安装助手连接已断开")?
            .write_all(b"INSTALL\n")
            .map_err(|e| format!("无法交接安装助手：{e}"))?;
        self.committed = true;
        self.input.take();
        Ok(())
    }
}
impl Drop for Prepared {
    fn drop(&mut self) {
        if !self.committed {
            // This is our uncommitted helper only, never the user's application.
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn prepare(package: VerifiedDownload, directory: PathBuf) -> Result<Prepared, String> {
    if !cfg!(windows) {
        return Err("当前平台不支持此安装交接".into());
    }
    let original_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let helpers = directory.join("update-helpers");
    fs::create_dir_all(&helpers).map_err(|e| e.to_string())?;
    // Best-effort cleanup of previous helpers. Windows refuses to remove a running one.
    if let Ok(entries) = fs::read_dir(&helpers) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_file())
                && entry.file_name().to_string_lossy().starts_with("helper-")
            {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let helper = helpers.join(format!("helper-{id}.exe"));
    let mut copy = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&helper)
        .map_err(|e| e.to_string())?;
    std::io::copy(
        &mut File::open(&original_exe).map_err(|e| e.to_string())?,
        &mut copy,
    )
    .map_err(|e| e.to_string())?;
    copy.sync_all().map_err(|e| e.to_string())?;
    drop(copy);
    let mut command = Command::new(&helper);
    command
        .arg("--install-update-helper")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW, pipes still work
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("无法启动安装助手：{e}"))?;
    let input = child.stdin.take();
    let output = child.stdout.take().ok_or("无法连接安装助手")?;
    let mut prepared = Prepared {
        child,
        input,
        committed: false,
    };
    let plan = Plan {
        package,
        directory,
        original_exe,
        editor_pid: std::process::id(),
    };
    serde_json::to_writer(prepared.input.as_mut().ok_or("无法连接安装助手")?, &plan)
        .map_err(|e| e.to_string())?;
    prepared
        .input
        .as_mut()
        .unwrap()
        .write_all(b"\n")
        .map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(output)
            .take(8192)
            .read_line(&mut line)
            .map(|_| line);
        let _ = tx.send(result);
    });
    let line = rx
        .recv_timeout(TIMEOUT)
        .map_err(|_| "安装助手未响应，应用未退出，请重试")?
        .map_err(|e| e.to_string())?;
    if line.trim() != "READY" {
        return Err(line
            .trim()
            .strip_prefix("ERROR ")
            .unwrap_or("安装助手启动失败，应用未退出")
            .to_string());
    }
    Ok(prepared)
}

fn wait_until(
    timeout: Duration,
    mut check: impl FnMut() -> Result<bool, String>,
) -> Result<(), String> {
    let start = Instant::now();
    loop {
        if check()? {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            return Err("等待应用安全退出超时；未运行安装包，也未强制结束进程".into());
        }
        std::thread::sleep(Duration::from_millis(80));
    }
}
fn coordinate(directory: &Path) -> Result<(), String> {
    // Wait for the editor to release its lock before asking the tray to exit.
    wait_until(TIMEOUT, || {
        Ok(super::quit_ipc::try_editor_lock(directory)?.is_some())
    })?;
    if try_lock(&directory.join("resident.lock"))?.is_none() {
        let result = super::quit_ipc::request_named(
            directory,
            super::quit_ipc::RESIDENT_ENDPOINT,
            Some(TIMEOUT),
        )?;
        if result != super::quit_ipc::Decision::Closing {
            return Err("托盘未同意退出；已停止安装，未强制关闭任何进程".into());
        }
    }
    wait_until(TIMEOUT, || {
        Ok(try_lock(&directory.join("resident.lock"))?.is_some()
            && super::quit_ipc::try_editor_lock(directory)?.is_some()
            && !application_running()?)
    })
}
#[cfg(windows)]
fn application_running() -> Result<bool, String> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> *mut core::ffi::c_void;
        fn CloseHandle(handle: *mut core::ffi::c_void) -> i32;
    }
    let name: Vec<u16> = super::platform::running_mutex_name()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let handle = unsafe { OpenMutexW(0x00100000, 0, name.as_ptr()) };
    if handle.is_null() {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(2) {
            Ok(false)
        } else {
            Err(error.to_string())
        }
    } else {
        unsafe {
            CloseHandle(handle);
        }
        Ok(true)
    }
}
#[cfg(not(windows))]
fn application_running() -> Result<bool, String> {
    Ok(false)
}

#[cfg(windows)]
struct ProcessHandle(*mut core::ffi::c_void);
#[cfg(windows)]
impl ProcessHandle {
    fn open(pid: u32) -> Result<Self, String> {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut core::ffi::c_void;
        }
        let handle = unsafe { OpenProcess(0x00100000, 0, pid) };
        if handle.is_null() {
            Err(format!(
                "无法确认应用进程：{}",
                std::io::Error::last_os_error()
            ))
        } else {
            Ok(Self(handle))
        }
    }
    fn wait(&self) -> Result<(), String> {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn WaitForSingleObject(handle: *mut core::ffi::c_void, millis: u32) -> u32;
        }
        if unsafe { WaitForSingleObject(self.0, TIMEOUT.as_millis() as u32) } == 0 {
            Ok(())
        } else {
            Err("等待应用进程结束超时；已停止安装，未强制结束任何进程".into())
        }
    }
}
#[cfg(windows)]
impl Drop for ProcessHandle {
    fn drop(&mut self) {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CloseHandle(handle: *mut core::ffi::c_void) -> i32;
        }
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Entry point runs BEFORE the normal running mutex or any UI is created.
pub fn run_helper() -> Result<(), String> {
    if !cfg!(windows) {
        return Err("此安装助手仅支持 Windows".into());
    }
    let mut input = BufReader::new(std::io::stdin());
    let mut line = String::new();
    input
        .by_ref()
        .take(16384)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    let plan: Plan = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    // Do not accept arbitrary application directories supplied on a command line.
    if plan.directory.canonicalize().map_err(|e| e.to_string())?
        != super::app_dir().canonicalize().map_err(|e| e.to_string())?
    {
        return Err("安装交接目录无效".into());
    }
    let gate = startup_gate(&plan.directory)?.ok_or("另一个更新安装正在进行")?;
    let verified = verify(&plan.package, &plan.directory)?;
    if try_lock(&plan.directory.join("resident.lock"))?.is_none()
        && !plan
            .directory
            .join(super::quit_ipc::RESIDENT_ENDPOINT)
            .is_file()
    {
        return Err("当前托盘不支持安全安装交接。请退出旧托盘并重新打开应用后重试。".into());
    }
    #[cfg(windows)]
    let editor_process = ProcessHandle::open(plan.editor_pid)?;
    #[cfg(windows)]
    let resident_process = if try_lock(&plan.directory.join("resident.lock"))?.is_none() {
        Some(ProcessHandle::open(super::quit_ipc::resident_pid(
            &plan.directory,
        )?)?)
    } else {
        None
    };
    println!("READY");
    std::io::stdout().flush().map_err(|e| e.to_string())?;
    line.clear();
    input
        .by_ref()
        .take(32)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if line != "INSTALL\n" {
        return Ok(());
    } // parent disappeared/cancelled: do nothing
    let result = (|| {
        #[cfg(windows)]
        editor_process.wait()?;
        coordinate(&plan.directory)?;
        #[cfg(windows)]
        if let Some(process) = &resident_process {
            process.wait()?;
        }
        // Keep new ordinary launches blocked throughout installation. Inno's
        // post-install launch waits for this gate without creating the running mutex.
        let status = Command::new(&plan.package.path)
            .arg("/NORESTART")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| format!("无法启动安装程序：{e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "安装未完成（退出码 {:?}）。安装包已保留，可稍后重试。",
                status.code()
            ))
        }
    })();
    drop(gate);
    drop(verified);
    if let Err(error) = result {
        let _ = fs::write(plan.directory.join("last-update-error.txt"), &error);
        // Re-open only when no editor survived. Never disturb a still-running editor.
        if super::quit_ipc::try_editor_lock(&plan.directory).is_ok_and(|lock| lock.is_some()) {
            let _ = Command::new(&plan.original_exe).arg("--editor").spawn();
        }
        let _ = super::show_notice_window(
            "更新安装未完成",
            &format!(
                "{error}\n更新文件仍保留。可重试安装；如应用无法启动，请从官方发行页重新安装。"
            ),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let mut nonce = [0u8; 8];
            getrandom::fill(&mut nonce).unwrap();
            let root = std::env::temp_dir()
                .join(format!("update-install-{:x}", u64::from_le_bytes(nonce)));
            fs::create_dir_all(root.join("updates/test")).unwrap();
            Self(root)
        }
        fn package(&self) -> VerifiedDownload {
            let path = self
                .0
                .join("updates/test/ImageFeedback-0.3.0-windows-x64-setup.exe");
            fs::write(&path, b"not executable test data").unwrap();
            VerifiedDownload {
                path,
                size: b"not executable test data".len() as u64,
                sha256: format!("{:x}", Sha256::digest(b"not executable test data")),
            }
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn validates_integrity_and_rejects_modified_or_missing_files() {
        let temp = Temp::new();
        let mut package = temp.package();
        package.size = fs::metadata(&package.path).unwrap().len();
        assert!(verify(&package, &temp.0).is_ok());
        fs::write(&package.path, b"different data").unwrap();
        assert!(verify(&package, &temp.0).is_err());
        package.size = 14;
        assert!(verify(&package, &temp.0).unwrap_err().contains("校验失败"));
        fs::remove_file(&package.path).unwrap();
        assert!(verify(&package, &temp.0).is_err());
    }
    #[test]
    fn rejects_paths_outside_downloads_and_unexpected_names() {
        let temp = Temp::new();
        let mut package = temp.package();
        let outside = temp.0.join("arbitrary.exe");
        fs::rename(&package.path, &outside).unwrap();
        package.path = outside;
        assert!(verify(&package, &temp.0).is_err());
        let wrong = temp.0.join("updates/test/wrong.exe");
        fs::rename(&package.path, &wrong).unwrap();
        package.path = wrong;
        assert!(verify(&package, &temp.0).is_err());
    }
    #[test]
    fn gate_is_exclusive_and_released_on_failure() {
        let temp = Temp::new();
        let gate = startup_gate(&temp.0).unwrap().unwrap();
        assert!(startup_gate(&temp.0).unwrap().is_none());
        drop(gate);
        assert!(startup_gate(&temp.0).unwrap().is_some());
    }
    #[test]
    fn timeout_never_forces_exit() {
        let mut attempts = 0;
        assert!(
            wait_until(Duration::ZERO, || {
                attempts += 1;
                Ok(false)
            })
            .is_err()
        );
        assert_eq!(attempts, 1);
        assert!(wait_until(Duration::ZERO, || Ok(true)).is_ok());
        assert!(wait_until(Duration::ZERO, || Err("failed".into())).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn verified_handle_prevents_replacement_until_launch_finishes() {
        let temp = Temp::new();
        let mut package = temp.package();
        package.size = fs::metadata(&package.path).unwrap().len();
        let file = verify(&package, &temp.0).unwrap();
        assert!(fs::write(&package.path, b"tampered").is_err());
        assert!(fs::remove_file(&package.path).is_err());
        drop(file);
        assert!(fs::remove_file(&package.path).is_ok());
    }
}
