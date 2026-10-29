//! Slint integration for background update checks and explicit downloads.
//! Downloads never install automatically. Explicit installation goes through the editor's feedback guard.
use crate::AppWindow;
use image_feedback_slint::update::{self, Cache, DownloadEvent, Release, Target, VerifiedDownload};
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Checking,
    Available,
    Current,
    CheckError,
    Downloading,
    Verifying,
    Ready,
    DownloadError,
    Cancelling,
    Installing,
    InstallError,
}
impl Phase {
    fn code(self) -> i32 {
        match self {
            Self::Idle => 0,
            Self::Checking => 1,
            Self::Available => 2,
            Self::Current => 3,
            Self::CheckError => 4,
            Self::Downloading => 5,
            Self::Verifying => 6,
            Self::Ready => 7,
            Self::DownloadError => 8,
            Self::Cancelling => 9,
            Self::Installing => 10,
            Self::InstallError => 11,
        }
    }
}
enum Active {
    Check { id: u64, manual: bool },
    Download { id: u64, cancel: Arc<AtomicBool> },
}
enum WorkerEvent {
    Checked {
        id: u64,
        result: Result<Option<Release>, String>,
    },
    DownloadProgress {
        id: u64,
        event: DownloadEvent,
    },
    DownloadFinished {
        id: u64,
        result: Result<VerifiedDownload, String>,
    },
}
struct Controller {
    directory: PathBuf,
    target: Target,
    cache: Cache,
    release: Option<Release>,
    downloaded: Option<VerifiedDownload>,
    phase: Phase,
    error: String,
    progress: u64,
    total: u64,
    next_id: u64,
    active: Option<Active>,
    sender: mpsc::Sender<WorkerEvent>,
}
impl Drop for Controller {
    fn drop(&mut self) {
        if let Some(Active::Download { cancel, .. }) = &self.active {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl Controller {
    fn new(directory: PathBuf, sender: mpsc::Sender<WorkerEvent>, use_cache: bool) -> Self {
        let target = Target::current();
        let cache = if use_cache {
            Cache::load(&directory, target, env!("CARGO_PKG_VERSION"))
        } else {
            Cache::default()
        };
        // Do not announce stale cached data as a freshly discovered release.
        let release = if cache.last_attempt_succeeded && !cache.due(update::now_seconds()) {
            cache.latest.clone()
        } else {
            None
        };
        let ready = use_cache
            .then(|| update::ReadyUpdate::load(&directory, target, env!("CARGO_PKG_VERSION")))
            .flatten();
        let release = ready.as_ref().map(|r| r.release.clone()).or(release);
        let downloaded = ready.map(|r| r.download);
        let phase = if downloaded.is_some() {
            Phase::Ready
        } else if release.is_some() {
            Phase::Available
        } else {
            Phase::Idle
        };
        Self {
            directory,
            target,
            cache,
            release,
            downloaded,
            phase,
            error: String::new(),
            progress: 0,
            total: 0,
            next_id: 0,
            active: None,
            sender,
        }
    }
    fn persist_dismissal(&self, tag: String, at: u64) {
        let directory = self.directory.clone();
        let target = self.target;
        std::thread::spawn(move || {
            if let Err(error) =
                update::mark_dismissed(&directory, env!("CARGO_PKG_VERSION"), target, &tag, at)
            {
                eprintln!("无法保存更新提醒状态：{error}");
            }
        });
    }
    fn poll_cache(&mut self, ui: &AppWindow) {
        if self.active.is_some()
            || ui.get_update_visible()
            || matches!(
                self.phase,
                Phase::Ready
                    | Phase::DownloadError
                    | Phase::Downloading
                    | Phase::Verifying
                    | Phase::Cancelling
                    | Phase::Installing
                    | Phase::InstallError
            )
        {
            return;
        }
        let cache = Cache::load(&self.directory, self.target, env!("CARGO_PKG_VERSION"));
        if cache.last_attempt < self.cache.last_attempt {
            return;
        }
        let changed = cache.last_attempt != self.cache.last_attempt
            || cache.last_attempt_succeeded != self.cache.last_attempt_succeeded
            || cache.latest != self.cache.latest;
        if cache.dismissed_at > self.cache.dismissed_at {
            self.cache.dismissed_tag = cache.dismissed_tag.clone();
            self.cache.dismissed_at = cache.dismissed_at;
        }
        if !changed || !cache.last_attempt_succeeded || cache.due(update::now_seconds()) {
            return;
        }
        self.cache = cache;
        self.release = self.cache.latest.clone();
        self.phase = if self.release.is_some() {
            Phase::Available
        } else {
            Phase::Idle
        };
        if let Some(release) = &self.release {
            if self
                .cache
                .should_notify(&release.tag, update::now_seconds())
            {
                ui.set_update_notice_visible(true);
            }
        } else {
            ui.set_update_notice_visible(false);
        }
        self.render(ui);
    }
    fn render(&self, ui: &AppWindow) {
        ui.set_update_phase(self.phase.code());
        ui.set_update_available(self.release.is_some());
        ui.set_update_current_version(format!("v{}", env!("CARGO_PKG_VERSION")).into());
        ui.set_update_version(
            self.release
                .as_ref()
                .map(|r| format!("v{}", r.version))
                .unwrap_or_default()
                .into(),
        );
        ui.set_update_notes(
            self.release
                .as_ref()
                .map(|r| r.notes.as_str())
                .unwrap_or("")
                .into(),
        );
        ui.set_update_has_warning(self.release.as_ref().is_some_and(|r| r.has_upgrade_warning));
        ui.set_update_can_download(self.release.as_ref().is_some_and(|r| r.package.is_some()));
        let percentage = self
            .progress
            .saturating_mul(100)
            .checked_div(self.total)
            .unwrap_or(0)
            .min(100) as i32;
        ui.set_update_progress(percentage);
        let progress = if self.phase == Phase::Cancelling {
            "正在取消，请稍候…".to_string()
        } else if self.phase == Phase::Verifying {
            "正在校验文件完整性…".to_string()
        } else if self.total > 0 {
            format!(
                "{:.1} MB / {:.1} MB · {percentage}%",
                self.progress as f64 / 1_048_576.0,
                self.total as f64 / 1_048_576.0
            )
        } else {
            "正在连接 GitHub…".to_string()
        };
        ui.set_update_progress_label(progress.into());
        let (title, subtitle, detail) = match self.phase {
            Phase::Idle => ("检查更新".to_string(), String::new(), String::new()),
            Phase::Checking => (
                "正在检查更新".into(),
                "正在连接 GitHub，请稍候。".into(),
                "检查不会更改你的图片或批注。".into(),
            ),
            Phase::Available => {
                let version = self
                    .release
                    .as_ref()
                    .map(|r| r.version.as_str())
                    .unwrap_or("");
                let subtitle = if self.release.as_ref().is_some_and(|r| r.package.is_some()) {
                    format!("图片反馈 v{version} 已发布")
                } else {
                    "当前平台的安装包请从发行页获取。".to_string()
                };
                ("发现新版本".into(), subtitle, String::new())
            }
            Phase::Current => (
                "当前已是最新版本".into(),
                format!("图片反馈 v{}", env!("CARGO_PKG_VERSION")),
                "暂时没有可用的正式版更新。".into(),
            ),
            Phase::CheckError => (
                "暂时无法检查更新".into(),
                "没有获得检查结果。".into(),
                self.error.clone(),
            ),
            Phase::Downloading => (
                "正在下载更新".into(),
                "关闭此窗口后可继续编辑。".into(),
                "下载将在后台继续；退出编辑器会中断下载。".into(),
            ),
            Phase::Verifying => (
                "正在检查更新文件".into(),
                "正在校验完整性，请稍候。".into(),
                "校验通过后可选择现在安装或稍后安装，不会自动退出应用。".into(),
            ),
            Phase::Ready => {
                let details = if cfg!(target_os = "macos") {
                    "此版本使用 DMG，尚不支持自动替换。打开后请退出编辑器及菜单栏，将应用拖入 Applications。文件仅通过完整性校验，请留意系统安全提示。"
                } else {
                    "现在安装会关闭编辑器和托盘，并打开安装向导；如有批注会先请你确认。文件已通过完整性校验，但尚无发布者签名，请仅在信任此发行源时继续。"
                };
                (
                    "更新已准备好".into(),
                    "现在安装更新吗？".into(),
                    details.into(),
                )
            }
            Phase::DownloadError => (
                "更新下载未完成".into(),
                "不会自动运行安装程序。".into(),
                self.error.clone(),
            ),
            Phase::InstallError => (
                "未能开始安装".into(),
                "应用未退出，当前反馈仍保留。".into(),
                self.error.clone(),
            ),
            Phase::Installing => (
                "正在准备安装".into(),
                "正在检查安装包和退出交接。".into(),
                "检查完成后将安全退出编辑器和托盘，再打开安装向导；检查失败会保留当前编辑内容。"
                    .into(),
            ),
            Phase::Cancelling => (
                "正在取消下载".into(),
                "请稍候。".into(),
                "正在结束网络请求并清理未完成的下载。".into(),
            ),
        };
        ui.set_update_title(title.into());
        ui.set_update_subtitle(subtitle.into());
        ui.set_update_detail(detail.into());
        let notice = match self.phase {
            Phase::Downloading => format!("正在下载更新 {percentage}%"),
            Phase::Verifying => "正在检查更新文件".to_string(),
            Phase::Cancelling => "正在取消下载".to_string(),
            Phase::Ready | Phase::InstallError => "更新已准备好".to_string(),
            Phase::DownloadError => "更新下载失败".to_string(),
            _ => self
                .release
                .as_ref()
                .map(|r| format!("发现新版本 v{}", r.version))
                .unwrap_or_default(),
        };
        ui.set_update_notice_title(notice.into());
    }
    fn next_id(&mut self) -> u64 {
        self.next_id = self.next_id.wrapping_add(1);
        self.next_id
    }
    fn start_check(&mut self, ui: &AppWindow, manual: bool, startup: bool) {
        if let Some(Active::Check {
            manual: active_manual,
            ..
        }) = &mut self.active
        {
            if manual {
                *active_manual = true;
                self.phase = Phase::Checking;
                self.render(ui);
                ui.set_update_visible(true);
            }
            return;
        }
        if self.active.is_some()
            || matches!(
                self.phase,
                Phase::Ready | Phase::Installing | Phase::InstallError
            )
        {
            if manual {
                self.render(ui);
                ui.set_update_visible(true);
            }
            return;
        }
        if !manual
            && !startup
            && (ui.get_update_visible()
                || ui.get_confirm_visible()
                || !self.cache.due(update::now_seconds()))
        {
            return;
        }
        let id = self.next_id();
        self.active = Some(Active::Check { id, manual });
        if manual {
            self.phase = Phase::Checking;
            self.render(ui);
            ui.set_update_visible(true);
        }
        let target = self.target;
        let directory = self.directory.clone();
        let sender = self.sender.clone();
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                if startup {
                    update::check_startup_coordinated(&directory, env!("CARGO_PKG_VERSION"), target)
                } else {
                    update::check_coordinated(&directory, env!("CARGO_PKG_VERSION"), target, manual)
                }
            })
            .unwrap_or_else(|_| Err("更新检查未能完成，请重试".into()));
            send(&sender, &weak, WorkerEvent::Checked { id, result });
        });
    }
    fn start_download(&mut self, ui: &AppWindow) {
        if self.active.is_some() {
            return;
        }
        let Some(release) = self.release.clone() else {
            return;
        };
        if release.package.is_none() {
            open_release_page(ui);
            return;
        }
        let id = self.next_id();
        let cancel = Arc::new(AtomicBool::new(false));
        self.active = Some(Active::Download {
            id,
            cancel: cancel.clone(),
        });
        self.phase = Phase::Downloading;
        self.progress = 0;
        self.total = release.package.as_ref().map_or(0, |package| package.size);
        self.downloaded = None;
        self.render(ui);
        let directory = self.directory.clone();
        let target = self.target;
        let sender = self.sender.clone();
        let progress_sender = sender.clone();
        let weak = ui.as_weak();
        let progress_weak = weak.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                update::download(&release, target, &directory, cancel, |event| {
                    send(
                        &progress_sender,
                        &progress_weak,
                        WorkerEvent::DownloadProgress { id, event },
                    );
                })
            }))
            .unwrap_or_else(|_| Err("更新下载未能完成，请重试".into()));
            send(&sender, &weak, WorkerEvent::DownloadFinished { id, result });
        });
    }
    fn dismiss(&mut self, ui: &AppWindow) {
        if self.phase == Phase::Installing {
            return;
        }
        ui.set_update_visible(false);
        if matches!(
            self.phase,
            Phase::Downloading
                | Phase::Verifying
                | Phase::Cancelling
                | Phase::Ready
                | Phase::InstallError
        ) {
            ui.set_update_notice_visible(true);
        } else {
            ui.set_update_notice_visible(false);
            if matches!(self.phase, Phase::Available | Phase::Ready)
                && let Some(release) = &self.release
            {
                self.cache.dismissed_tag = release.tag.clone();
                self.cache.dismissed_at = update::now_seconds();
                self.persist_dismissal(self.cache.dismissed_tag.clone(), self.cache.dismissed_at);
            }
        }
    }
    fn dismiss_notice(&mut self, ui: &AppWindow) {
        ui.set_update_notice_visible(false);
        if let Some(release) = &self.release {
            self.cache.dismissed_tag = release.tag.clone();
            self.cache.dismissed_at = update::now_seconds();
            self.persist_dismissal(self.cache.dismissed_tag.clone(), self.cache.dismissed_at);
        }
    }
    fn open(&mut self, ui: &AppWindow) {
        if self.phase == Phase::Idle {
            self.start_check(ui, true, false);
            return;
        }
        self.render(ui);
        ui.set_update_visible(true);
        ui.set_update_notice_visible(false);
    }
    fn primary(&mut self, ui: &AppWindow) -> Option<VerifiedDownload> {
        match self.phase {
            Phase::Available => self.start_download(ui),
            Phase::CheckError => self.start_check(ui, true, false),
            Phase::DownloadError => self.start_download(ui),
            Phase::Ready | Phase::InstallError => {
                if cfg!(windows) {
                    return self.downloaded.clone();
                }
                if let Some(package) = self.downloaded.as_ref()
                    && let Err(error) = open_package(&package.path)
                {
                    ui.set_update_detail(
                        format!("无法打开安装包：{error}。可前往发行页重新下载。").into(),
                    );
                }
            }
            _ => {}
        }
        None
    }
    fn secondary(&mut self, ui: &AppWindow) {
        if self.phase == Phase::InstallError {
            self.start_download(ui);
            return;
        }
        if matches!(self.phase, Phase::Downloading | Phase::Verifying) {
            if let Some(Active::Download { cancel, .. }) = &self.active {
                cancel.store(true, Ordering::Relaxed);
                self.phase = Phase::Cancelling;
                self.render(ui);
            }
        } else {
            open_release_page(ui);
        }
    }
    fn handle(&mut self, ui: &AppWindow, event: WorkerEvent) {
        match event {
            WorkerEvent::Checked { id, result } => {
                let manual = match self.active.as_ref() {
                    Some(Active::Check {
                        id: expected,
                        manual,
                    }) if *expected == id => *manual,
                    _ => return,
                };
                self.active = None;
                if !matches!(&result, Err(error) if error == "更新正在后台检查") {
                    self.cache.last_attempt = update::now_seconds();
                    self.cache.last_attempt_succeeded = result.is_ok();
                }
                match result {
                    Ok(release) => {
                        self.cache.latest = release.clone();
                        self.release = release;
                        self.phase = if self.release.is_some() {
                            Phase::Available
                        } else {
                            Phase::Current
                        };
                        self.error.clear();
                        if let Some(release) = &self.release {
                            if !ui.get_update_visible()
                                && self
                                    .cache
                                    .should_notify(&release.tag, update::now_seconds())
                            {
                                ui.set_update_notice_visible(true);
                            }
                        } else {
                            ui.set_update_notice_visible(false);
                        }
                    }
                    Err(error) => {
                        if manual && ui.get_update_visible() {
                            self.phase = Phase::CheckError;
                            self.error = error;
                        }
                        // Automatic errors never replace the editor or claim it is current.
                        else if self.release.is_some() {
                            self.phase = Phase::Available;
                        } else {
                            self.phase = Phase::Idle;
                        }
                    }
                }
                self.render(ui);
            }
            WorkerEvent::DownloadProgress { id, event } => {
                if !matches!(&self.active, Some(Active::Download { id: expected, .. }) if *expected == id)
                {
                    return;
                }
                if self.phase == Phase::Cancelling {
                    return;
                }
                match event {
                    DownloadEvent::Progress { received, total } => {
                        self.progress = received;
                        self.total = total;
                    }
                    DownloadEvent::Verifying => {
                        self.phase = Phase::Verifying;
                        self.progress = self.total;
                    }
                }
                self.render(ui);
            }
            WorkerEvent::DownloadFinished { id, result } => {
                let cancelled = match self.active.as_ref() {
                    Some(Active::Download {
                        id: expected,
                        cancel,
                    }) if *expected == id => cancel.load(Ordering::Relaxed),
                    _ => return,
                };
                self.active = None;
                if cancelled {
                    if let Ok(path) = result {
                        let _ = std::fs::remove_file(&path.path);
                        if let Some(parent) = path.path.parent() {
                            let _ = std::fs::remove_dir(parent);
                        }
                    }
                    self.phase = Phase::Available;
                    self.downloaded = None;
                    ui.set_update_notice_visible(false);
                } else {
                    match result {
                        Ok(path) => {
                            self.downloaded = Some(path);
                            self.phase = Phase::Ready;
                            if !ui.get_update_visible() {
                                ui.set_update_notice_visible(true);
                            }
                        }
                        Err(error) => {
                            self.error = error;
                            self.phase = Phase::DownloadError;
                            if !ui.get_update_visible() {
                                ui.set_update_notice_visible(true);
                            }
                        }
                    }
                }
                self.render(ui);
            }
        }
    }
}
fn send(sender: &mpsc::Sender<WorkerEvent>, weak: &slint::Weak<AppWindow>, event: WorkerEvent) {
    if sender.send(event).is_ok() {
        let _ = weak.upgrade_in_event_loop(|ui| ui.invoke_update_worker_event());
    }
}
fn open_release_page(ui: &AppWindow) {
    if webbrowser::open(update::RELEASES_URL).is_err() {
        ui.set_update_subtitle("无法打开浏览器，请手动访问 GitHub 发行页。".into());
    }
}
fn open_package(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err("安装包已被移动或删除".into());
    }
    #[cfg(windows)]
    let mut command = std::process::Command::new("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    command
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub struct UpdateHandle {
    timer: Rc<slint::Timer>,
    state: Rc<RefCell<Controller>>,
    smoke: bool,
}
impl UpdateHandle {
    pub fn stop(&self) {
        self.timer.stop();
    }
    /// No network or executable fixture: only used by the explicit native smoke mode.
    pub fn seed_smoke_ready(&self, ui: &AppWindow, missing_path: PathBuf) {
        assert!(self.smoke, "cannot inject updates into a normal session");
        let mut state = self.state.borrow_mut();
        state.phase = Phase::Ready;
        state.downloaded = Some(VerifiedDownload {
            path: missing_path,
            sha256: "0".repeat(64),
            size: 1,
        });
        state.render(ui);
        ui.set_update_visible(true);
    }
}

/// Wire callbacks and return a handle that must be held while the editor runs.
/// Smoke scenarios can disable automatic network access without changing the UI.
pub fn install(
    ui: &AppWindow,
    directory: PathBuf,
    automatic: bool,
    on_install: impl Fn(VerifiedDownload) + 'static,
) -> Rc<UpdateHandle> {
    ui.set_update_install_label(
        if cfg!(windows) {
            "现在安装"
        } else {
            "打开安装包"
        }
        .into(),
    );
    let (sender, receiver) = mpsc::channel();
    let state = Rc::new(RefCell::new(Controller::new(directory, sender, automatic)));
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.on_update_install_started(move || {
            if let Some(ui) = weak.upgrade() {
                let mut controller = state.borrow_mut();
                controller.phase = Phase::Installing;
                controller.render(&ui);
                ui.set_update_visible(true);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.on_update_install_failed(move |error| {
            if let Some(ui) = weak.upgrade() {
                let mut controller = state.borrow_mut();
                controller.phase = Phase::InstallError;
                controller.error = error.to_string();
                controller.render(&ui);
                ui.set_update_notice_visible(true);
                ui.set_update_visible(true);
            }
        });
    }
    {
        let controller = state.borrow();
        controller.render(ui);
        if controller.phase == Phase::Ready
            || controller.release.as_ref().is_some_and(|release| {
                controller
                    .cache
                    .should_notify(&release.tag, update::now_seconds())
            })
        {
            ui.set_update_notice_visible(true);
        }
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_update_worker_event(move || {
            let Some(ui) = weak.upgrade() else { return };
            while let Ok(event) = receiver.try_recv() {
                state.borrow_mut().handle(&ui, event);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_update_check(move || {
            if let Some(ui) = weak.upgrade() {
                state.borrow_mut().start_check(&ui, true, false);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_update_open(move || {
            if let Some(ui) = weak.upgrade() {
                state.borrow_mut().open(&ui);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_update_dismiss(move || {
            if let Some(ui) = weak.upgrade() {
                state.borrow_mut().dismiss(&ui);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_update_dismiss_notice(move || {
            if let Some(ui) = weak.upgrade() {
                state.borrow_mut().dismiss_notice(&ui);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_update_primary(move || {
            if let Some(ui) = weak.upgrade() {
                let package = state.borrow_mut().primary(&ui);
                if let Some(package) = package {
                    on_install(package);
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_update_secondary(move || {
            if let Some(ui) = weak.upgrade() {
                state.borrow_mut().secondary(&ui);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_update_release_page(move || {
            if let Some(ui) = weak.upgrade() {
                open_release_page(&ui);
            }
        });
    }
    let timer = Rc::new(slint::Timer::default());
    if automatic {
        {
            let weak = ui.as_weak();
            let state = state.clone();
            slint::Timer::single_shot(Duration::from_secs(1), move || {
                if let Some(ui) = weak.upgrade() {
                    let mut state = state.borrow_mut();
                    state.poll_cache(&ui);
                    state.start_check(&ui, false, true);
                }
            });
        }
        let weak = ui.as_weak();
        let periodic_state = state.clone();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_secs(30),
            move || {
                if let Some(ui) = weak.upgrade() {
                    let mut state = periodic_state.borrow_mut();
                    state.poll_cache(&ui);
                    state.start_check(&ui, false, false);
                }
            },
        );
    }
    Rc::new(UpdateHandle {
        timer,
        state,
        smoke: !automatic,
    })
}
