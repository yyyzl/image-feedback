//! Authenticated, local-only coordination between the resident and editor processes.
//! No image or feedback data is sent over this channel.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

const ENDPOINT_FILE: &str = "editor-control.json";
pub const RESIDENT_ENDPOINT: &str = "resident-control.json";
const TOKEN_BYTES: usize = 32;
const TOKEN_HEX: usize = TOKEN_BYTES * 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Closing,
    Cancelled,
    Busy,
}
impl Decision {
    fn byte(self) -> u8 {
        match self {
            Self::Closing => b'Q',
            Self::Cancelled => b'C',
            Self::Busy => b'B',
        }
    }
    fn from_byte(byte: u8) -> Result<Self, String> {
        match byte {
            b'Q' => Ok(Self::Closing),
            b'C' => Ok(Self::Cancelled),
            b'B' => Ok(Self::Busy),
            _ => Err("编辑器返回了未知退出状态".into()),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Endpoint {
    #[serde(default)]
    pid: u32,
    port: u16,
    token: String,
}
fn read_endpoint(path: &Path) -> Result<Endpoint, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(513)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 512 {
        return Err("编辑器通信信息过长".into());
    }
    let endpoint: Endpoint = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if endpoint.port == 0
        || endpoint.token.len() != TOKEN_HEX
        || !endpoint.token.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("编辑器通信信息无效".into());
    }
    Ok(endpoint)
}

/// Only an editor holding editor.lock may start this server or replace a stale endpoint.
pub struct Server {
    endpoint: Endpoint,
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    pub fn start(
        directory: &Path,
        notify: impl Fn(mpsc::Sender<Decision>) -> Result<(), ()> + Send + 'static,
    ) -> Result<Self, String> {
        Self::start_named(directory, ENDPOINT_FILE, notify)
    }
    /// Caller must hold the corresponding process lock before publishing an endpoint.
    pub fn start_named(
        directory: &Path,
        name: &str,
        notify: impl Fn(mpsc::Sender<Decision>) -> Result<(), ()> + Send + 'static,
    ) -> Result<Self, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| format!("无法建立本机进程通信：{e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let mut random = [0; TOKEN_BYTES];
        getrandom::fill(&mut random).map_err(|e| e.to_string())?;
        let token = random.iter().map(|b| format!("{b:02x}")).collect();
        let endpoint = Endpoint {
            pid: std::process::id(),
            port,
            token,
        };
        let path = directory.join(name);
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if !meta.file_type().is_file() {
                return Err("编辑器通信文件不是普通文件".into());
            }
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(|e| e.to_string())?;
        if let Err(error) = serde_json::to_writer(&mut file, &endpoint) {
            let _ = fs::remove_file(&path);
            return Err(error.to_string());
        }
        drop(file);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let expected = endpoint.token.clone();
        let thread = std::thread::spawn(move || {
            for connection in listener.incoming() {
                if stopped.load(Ordering::Acquire) {
                    break;
                }
                let Ok(mut stream) = connection else {
                    break;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                let mut request = [0u8; TOKEN_HEX + 1];
                if stream.read_exact(&mut request).is_err()
                    || request[0] != b'Q'
                    || &request[1..] != expected.as_bytes()
                {
                    continue;
                }
                let (reply, answer) = mpsc::channel();
                if notify(reply).is_err() {
                    let _ = stream.write_all(&[Decision::Busy.byte()]);
                    continue;
                }
                loop {
                    match answer.recv_timeout(Duration::from_millis(100)) {
                        Ok(decision) => {
                            let _ = stream.write_all(&[decision.byte()]);
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            let _ = stream.write_all(&[Decision::Busy.byte()]);
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if stopped.load(Ordering::Acquire) {
                                break;
                            }
                        }
                    }
                }
            }
        });
        Ok(Self {
            endpoint,
            path,
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let awakened = TcpStream::connect_timeout(
            &SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.endpoint.port).into(),
            Duration::from_millis(200),
        )
        .is_ok();
        if let Some(thread) = self.thread.take() {
            // Do not hang app shutdown if the listener could not be awakened.
            if awakened || thread.is_finished() {
                let _ = thread.join();
            }
        }
        // Do not remove a file replaced by someone else while the editor was running.
        if read_endpoint(&self.path).ok().as_ref() == Some(&self.endpoint) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn resident_pid(directory: &Path) -> Result<u32, String> {
    let pid = read_endpoint(&directory.join(RESIDENT_ENDPOINT))?.pid;
    if pid == 0 {
        return Err("托盘不支持进程退出确认，请重启应用后重试".into());
    }
    Ok(pid)
}

/// Waits for the user's decision without blocking the resident's UI thread.
pub fn request(directory: &Path) -> Result<Decision, String> {
    request_named(directory, ENDPOINT_FILE, None)
}

pub fn request_named(
    directory: &Path,
    name: &str,
    timeout: Option<Duration>,
) -> Result<Decision, String> {
    let endpoint = read_endpoint(&directory.join(name))?;
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, endpoint.port);
    let mut stream = TcpStream::connect_timeout(&address.into(), Duration::from_secs(1))
        .map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(timeout)
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .map_err(|e| e.to_string())?;
    stream.write_all(b"Q").map_err(|e| e.to_string())?;
    stream
        .write_all(endpoint.token.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut response = [0];
    stream
        .read_exact(&mut response)
        .map_err(|e| e.to_string())?;
    Decision::from_byte(response[0])
}

/// Holds the lock when no editor is running; never kills an editor process.
pub fn try_editor_lock(directory: &Path) -> Result<Option<File>, String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("editor.lock"))
        .map_err(|e| e.to_string())?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/quit-ipc-tests")
                .join(format!(
                    "{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn waits_for_the_actual_decision_and_removes_its_endpoint() {
        let temp = Temp::new();
        let (notifications, received) = mpsc::channel::<mpsc::Sender<Decision>>();
        let server = Server::start(&temp.0, move |reply| {
            notifications.send(reply).map_err(|_| ())
        })
        .unwrap();
        let path = temp.0.clone();
        let (finished, result) = mpsc::channel();
        let client = std::thread::spawn(move || finished.send(request(&path)).unwrap());
        let reply = received.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(result.recv_timeout(Duration::from_millis(100)).is_err());
        reply.send(Decision::Cancelled).unwrap();
        assert_eq!(
            result
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .unwrap(),
            Decision::Cancelled
        );
        client.join().unwrap();
        drop(server);
        assert!(!temp.0.join(ENDPOINT_FILE).exists());
    }
    #[test]
    fn rejects_wrong_token_without_notifying_the_editor() {
        let temp = Temp::new();
        let (notifications, received) = mpsc::channel::<mpsc::Sender<Decision>>();
        let server = Server::start(&temp.0, move |reply| {
            notifications.send(reply).map_err(|_| ())
        })
        .unwrap();
        let endpoint = read_endpoint(&temp.0.join(ENDPOINT_FILE)).unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, endpoint.port)).unwrap();
        stream.write_all(&[b'Q'; TOKEN_HEX + 1]).unwrap();
        assert!(received.recv_timeout(Duration::from_millis(200)).is_err());
        drop(server);
    }
    #[test]
    fn drop_does_not_remove_a_replaced_endpoint() {
        let temp = Temp::new();
        let server = Server::start(&temp.0, |_| Ok(())).unwrap();
        let endpoint_path = temp.0.join(ENDPOINT_FILE);
        fs::write(&endpoint_path, b"replacement").unwrap();
        drop(server);
        assert_eq!(fs::read(endpoint_path).unwrap(), b"replacement");
    }
    #[test]
    fn dropping_server_does_not_wait_for_a_pending_user_decision() {
        let temp = Temp::new();
        let (notifications, received) = mpsc::channel::<mpsc::Sender<Decision>>();
        let server = Server::start(&temp.0, move |reply| {
            notifications.send(reply).map_err(|_| ())
        })
        .unwrap();
        let path = temp.0.clone();
        let client = std::thread::spawn(move || request(&path));
        let reply = received.recv_timeout(Duration::from_secs(3)).unwrap();
        drop(server);
        assert!(client.join().unwrap().is_err());
        drop(reply);
    }
    #[test]
    fn resident_endpoint_is_separate_authenticated_and_has_a_bounded_response() {
        let temp = Temp::new();
        let editor = Server::start(&temp.0, |reply| {
            reply.send(Decision::Cancelled).map_err(|_| ())
        })
        .unwrap();
        let resident = Server::start_named(&temp.0, RESIDENT_ENDPOINT, |reply| {
            reply.send(Decision::Closing).map_err(|_| ())
        })
        .unwrap();
        assert_eq!(resident_pid(&temp.0).unwrap(), std::process::id());
        assert_eq!(request(&temp.0).unwrap(), Decision::Cancelled);
        assert_eq!(
            request_named(&temp.0, RESIDENT_ENDPOINT, Some(Duration::from_secs(1))).unwrap(),
            Decision::Closing
        );
        drop(resident);
        assert!(temp.0.join(ENDPOINT_FILE).exists());
        let (tx, rx) = mpsc::channel();
        let resident = Server::start_named(&temp.0, RESIDENT_ENDPOINT, move |reply| {
            tx.send(reply).map_err(|_| ())
        })
        .unwrap();
        assert!(
            request_named(&temp.0, RESIDENT_ENDPOINT, Some(Duration::from_millis(100))).is_err()
        );
        let pending = rx.recv().unwrap();
        drop(resident);
        drop(pending);
        drop(editor);
    }
    #[test]
    fn editor_lock_is_exclusive() {
        let temp = Temp::new();
        let held = try_editor_lock(&temp.0).unwrap().unwrap();
        assert!(try_editor_lock(&temp.0).unwrap().is_none());
        drop(held);
        assert!(try_editor_lock(&temp.0).unwrap().is_some());
    }
}
