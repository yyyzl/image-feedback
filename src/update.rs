//! GitHub Release discovery and user-initiated package downloads.
//!
//! Checksums detect incomplete or corrupted downloads; they do not authenticate
//! the publisher when obtained from the same release. This module never runs an
//! installer or changes the user's editor/session.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use reqwest::{Url, blocking::Client, redirect::Policy};
use semver::Version;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const RELEASES_URL: &str = "https://github.com/yyyzl/image-feedback/releases";
const API: &str = "https://api.github.com/repos/yyyzl/image-feedback/releases";
const MAX_RESPONSE: u64 = 2 * 1024 * 1024;
const MAX_NOTES: usize = 120_000;
const MAX_PACKAGE: u64 = 512 * 1024 * 1024;
const MAX_HISTORY_PAGES: usize = 3;
const CACHE_FILE: &str = "update-check.json";
const CHECK_INTERVAL: u64 = 24 * 60 * 60;
const FAILED_RETRY_INTERVAL: u64 = 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    WindowsX64,
    MacUniversal,
    Unsupported,
}
impl Target {
    pub fn current() -> Self {
        if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            Self::WindowsX64
        } else if cfg!(target_os = "macos") {
            Self::MacUniversal
        } else {
            Self::Unsupported
        }
    }
    fn filename(self, version: &Version) -> Option<String> {
        match self {
            Self::WindowsX64 => Some(format!("ImageFeedback-{version}-windows-x64-setup.exe")),
            Self::MacUniversal => {
                Some(format!("ImageFeedback-{version}-macos-universal-adhoc.dmg"))
            }
            Self::Unsupported => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub url: String,
    pub checksum_url: String,
    pub size: u64,
    pub digest: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub version: String,
    pub notes: String,
    pub has_upgrade_warning: bool,
    pub history_complete: bool,
    pub package: Option<Package>,
}
impl Release {
    /// Validate cached data again before using any of its URLs or filenames.
    pub fn valid(&self, target: Target) -> bool {
        let Some(version) = stable_version(&self.tag) else {
            return false;
        };
        if version.to_string() != self.version || self.notes.len() > MAX_NOTES + 1024 {
            return false;
        }
        self.package
            .as_ref()
            .is_none_or(|package| valid_package(&self.tag, &version, target, package))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Cache {
    pub last_attempt: u64,
    pub last_attempt_succeeded: bool,
    pub latest: Option<Release>,
    pub dismissed_tag: String,
    pub dismissed_at: u64,
}
impl Cache {
    pub fn load(directory: &Path, target: Target, current: &str) -> Self {
        let path = directory.join(CACHE_FILE);
        let Ok(file) = File::open(path) else {
            return Self::default();
        };
        let mut bytes = Vec::new();
        if file.take(1024 * 1024).read_to_end(&mut bytes).is_err() {
            return Self::default();
        }
        let Ok(mut cache) = serde_json::from_slice::<Self>(&bytes) else {
            return Self::default();
        };
        if cache.latest.as_ref().is_some_and(|release| {
            !release.valid(target) || !is_newer(&release.tag, current).unwrap_or(false)
        }) {
            cache.latest = None;
        }
        cache
    }
    pub fn save(&self, directory: &Path) -> Result<(), String> {
        fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).map_err(|e| e.to_string())?;
        let tmp = directory.join(format!("update-check-{}.tmp", hex(&random)));
        let result = (|| -> Result<(), String> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .map_err(|e| e.to_string())?;
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            fs::rename(&tmp, directory.join(CACHE_FILE)).map_err(|e| e.to_string())?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(tmp);
        }
        result
    }
    pub fn due(&self, now: u64) -> bool {
        let interval = if self.last_attempt_succeeded {
            CHECK_INTERVAL
        } else {
            FAILED_RETRY_INTERVAL
        };
        self.last_attempt == 0 || now < self.last_attempt || now - self.last_attempt >= interval
    }
    pub fn should_notify(&self, tag: &str, now: u64) -> bool {
        self.dismissed_tag != tag
            || now < self.dismissed_at
            || now.saturating_sub(self.dismissed_at) >= CHECK_INTERVAL
    }
}
pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn lock_file(directory: &Path, name: &str) -> Result<File, String> {
    fs::create_dir_all(directory).map_err(|_| "无法访问更新状态目录".to_string())?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(name))
        .map_err(|_| "无法访问更新状态锁".to_string())
}
fn modify_cache(
    directory: &Path,
    current: &str,
    target: Target,
    change: impl FnOnce(&mut Cache),
) -> Result<(), String> {
    let lock = lock_file(directory, "update-state.lock")?;
    lock.lock().map_err(|_| "无法锁定更新状态".to_string())?;
    let mut cache = Cache::load(directory, target, current);
    change(&mut cache);
    cache.save(directory)
}
/// Persist a dismissal without replacing a newer check from another process.
pub fn mark_dismissed(
    directory: &Path,
    current: &str,
    target: Target,
    tag: &str,
    at: u64,
) -> Result<(), String> {
    modify_cache(directory, current, target, |cache| {
        if at >= cache.dismissed_at {
            cache.dismissed_tag = tag.to_string();
            cache.dismissed_at = at;
        }
    })
}

#[derive(Clone, Debug, Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

fn stable_version(tag: &str) -> Option<Version> {
    let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return None;
    }
    if tag != version.to_string() && tag != format!("v{version}") {
        return None;
    }
    Some(version)
}
pub fn is_newer(tag: &str, current: &str) -> Result<bool, String> {
    let running = Version::parse(current).map_err(|_| "当前应用版本格式无效".to_string())?;
    let candidate =
        stable_version(tag).ok_or_else(|| "发行版本格式无效或不是正式版".to_string())?;
    Ok(candidate > running)
}
fn allowed_redirect(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some(
                "api.github.com"
                    | "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}
fn client(timeout: Duration) -> Result<Client, String> {
    Client::builder()
        .user_agent(format!(
            "ImageFeedback/{} (github.com/yyyzl/image-feedback)",
            env!("CARGO_PKG_VERSION")
        ))
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || !allowed_redirect(attempt.url()) {
                attempt.error("更新请求重定向到不受信任的位置")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| "无法初始化安全连接".to_string())
}
fn response(client: &Client, url: &str) -> Result<reqwest::blocking::Response, String> {
    let result = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|_| "无法连接 GitHub，请检查网络后重试".to_string())?;
    let status = result.status();
    if status.as_u16() == 403 || status.as_u16() == 429 {
        return Err("GitHub 暂时限制了请求，请稍后重试".into());
    }
    if !status.is_success() {
        return Err(format!("GitHub 返回错误（{}）", status.as_u16()));
    }
    Ok(result)
}
fn get_json<T: DeserializeOwned>(client: &Client, url: &str) -> Result<T, String> {
    let response = response(client, url)?;
    let mut body = Vec::new();
    response
        .take(MAX_RESPONSE + 1)
        .read_to_end(&mut body)
        .map_err(|_| "无法读取更新信息".to_string())?;
    if body.len() as u64 > MAX_RESPONSE {
        return Err("更新信息过长，请前往发行页查看".into());
    }
    serde_json::from_slice(&body).map_err(|_| "GitHub 返回了无法识别的更新信息".into())
}
fn asset_url(tag: &str, name: &str) -> Option<String> {
    // Both tag and filename are constructed from an accepted semantic version.
    if stable_version(tag).is_none() || name.contains('/') || name.contains('\\') {
        return None;
    }
    Some(format!("{RELEASES_URL}/download/{tag}/{name}"))
}
fn valid_package(tag: &str, version: &Version, target: Target, package: &Package) -> bool {
    let Some(name) = target.filename(version) else {
        return false;
    };
    package.name == name
        && package.size > 0
        && package.size <= MAX_PACKAGE
        && asset_url(tag, &name).as_deref() == Some(package.url.as_str())
        && asset_url(tag, &format!("{name}.sha256")).as_deref()
            == Some(package.checksum_url.as_str())
        && package
            .digest
            .as_deref()
            .is_none_or(|d| parse_digest(d).is_some())
}
fn package_for(release: &ApiRelease, version: &Version, target: Target) -> Option<Package> {
    let name = target.filename(version)?;
    let asset = release.assets.iter().find(|a| a.name == name)?;
    let checksum = release
        .assets
        .iter()
        .find(|a| a.name == format!("{name}.sha256"))?;
    if checksum.size == 0 || checksum.size > 4096 {
        return None;
    }
    let package = Package {
        name,
        url: asset.browser_download_url.clone(),
        checksum_url: checksum.browser_download_url.clone(),
        size: asset.size,
        digest: asset.digest.clone(),
    };
    valid_package(&release.tag_name, version, target, &package).then_some(package)
}
fn warning_sections(body: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut current = String::new();
    let mut level = 0;
    for line in body.lines() {
        let trimmed = line.trim_start();
        let heading_level = trimmed.chars().take_while(|&ch| ch == '#').count();
        let heading = trimmed[heading_level..].trim();
        let is_heading = (1..=6).contains(&heading_level)
            && trimmed
                .chars()
                .nth(heading_level)
                .is_some_and(char::is_whitespace);
        if is_heading && level > 0 && heading_level <= level {
            warnings.push(std::mem::take(&mut current));
            level = 0;
        }
        let normalized = heading.to_lowercase();
        if is_heading
            && (heading.contains("升级前请注意") || normalized.contains("breaking change"))
        {
            level = heading_level;
        }
        if level > 0 {
            current.push_str(line);
            current.push('\n');
        }
    }
    if level > 0 {
        warnings.push(current);
    }
    warnings
}
fn append_limited(output: &mut String, input: &str) -> bool {
    let remaining = MAX_NOTES.saturating_sub(output.len());
    if input.len() <= remaining {
        output.push_str(input);
        return true;
    }
    let mut boundary = remaining.min(input.len());
    while !input.is_char_boundary(boundary) {
        boundary -= 1;
    }
    output.push_str(&input[..boundary]);
    false
}
/// Render untrusted Markdown as inert text. Links and image destinations remain
/// visible as text, and raw HTML is never executed or interpreted by the UI.
fn plain_markdown(source: &str) -> (String, bool) {
    let mut output = String::new();
    let mut complete = true;
    let mut destinations: Vec<String> = Vec::new();
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
    for event in Parser::new_ext(source, options) {
        let piece = match event {
            Event::Start(Tag::Item) => "• ".to_string(),
            Event::Start(Tag::Link { dest_url, .. })
            | Event::Start(Tag::Image { dest_url, .. }) => {
                destinations.push(dest_url.to_string());
                String::new()
            }
            Event::End(TagEnd::Link) | Event::End(TagEnd::Image) => destinations
                .pop()
                .map(|url| format!(" ({url})"))
                .unwrap_or_default(),
            Event::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::TableRow,
            ) => "\n\n".to_string(),
            Event::End(TagEnd::Item) => "\n".to_string(),
            Event::End(TagEnd::TableCell) => "  |  ".to_string(),
            Event::Text(text)
            | Event::Code(text)
            | Event::Html(text)
            | Event::InlineHtml(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text) => text.to_string(),
            Event::FootnoteReference(text) => format!("[{text}]"),
            Event::SoftBreak | Event::HardBreak => "\n".to_string(),
            Event::Rule => "\n──────────\n".to_string(),
            Event::TaskListMarker(done) => {
                if done {
                    "[x] ".into()
                } else {
                    "[ ] ".into()
                }
            }
            _ => String::new(),
        };
        if !append_limited(&mut output, &piece) {
            complete = false;
            break;
        }
    }
    (output, complete)
}
fn build_release(
    latest: ApiRelease,
    history: Vec<ApiRelease>,
    complete: bool,
    current: &str,
    target: Target,
) -> Result<Release, String> {
    let latest_version = stable_version(&latest.tag_name).ok_or("最新发行版本格式无效")?;
    let running = Version::parse(current).map_err(|_| "当前应用版本格式无效")?;
    let package = package_for(&latest, &latest_version, target);
    let mut releases = vec![latest.clone()];
    for item in history {
        if item.draft || item.prerelease || item.tag_name == latest.tag_name {
            continue;
        }
        let Some(version) = stable_version(&item.tag_name) else {
            continue;
        };
        if version > running && version < latest_version {
            releases.push(item);
        }
    }
    releases.sort_by_key(|item| std::cmp::Reverse(stable_version(&item.tag_name)));
    releases.dedup_by(|a, b| a.tag_name == b.tag_name);
    let mut warnings = String::new();
    for item in &releases {
        for warning in warning_sections(item.body.as_deref().unwrap_or("")) {
            let (text, _) = plain_markdown(&warning);
            if !append_limited(&mut warnings, &format!("{}\n{}\n", item.tag_name, text)) {
                break;
            }
        }
    }
    let has_upgrade_warning = !warnings.is_empty();
    let mut notes = String::new();
    let mut all_present = complete;
    if has_upgrade_warning {
        all_present &= append_limited(&mut notes, "升级提醒（来自发行说明）\n\n");
        all_present &= append_limited(&mut notes, &warnings);
        all_present &= append_limited(&mut notes, "\n──────────\n\n");
    }
    for item in &releases {
        all_present &= append_limited(&mut notes, &format!("{}\n\n", item.tag_name));
        let body = item.body.as_deref().filter(|body| !body.trim().is_empty());
        if let Some(body) = body {
            let (readable, rendered_completely) = plain_markdown(body);
            all_present &= rendered_completely;
            all_present &= append_limited(&mut notes, &readable);
        } else {
            let _ = append_limited(&mut notes, "此版本没有提供更新说明，请前往发行页查看。");
            all_present = false;
        }
        all_present &= append_limited(&mut notes, "\n\n──────────\n\n");
    }
    if !all_present {
        notes.insert_str(
            0,
            "注意：说明或历史版本未能完整展示，请前往发行页查看完整版本历史。\n\n",
        );
    }
    Ok(Release {
        tag: latest.tag_name,
        version: latest_version.to_string(),
        notes,
        has_upgrade_warning,
        history_complete: all_present,
        package,
    })
}
/// Check the latest stable release. History lookup failures do not invent a
/// complete changelog: the returned release explicitly marks it incomplete.
pub fn check(current: &str, target: Target) -> Result<Option<Release>, String> {
    let client = client(Duration::from_secs(20))?;
    let latest: ApiRelease = get_json(&client, &format!("{API}/latest"))?;
    if latest.draft || latest.prerelease {
        return Err("GitHub 未返回正式版发行信息".into());
    }
    if !is_newer(&latest.tag_name, current)? {
        return Ok(None);
    }
    let mut history = Vec::new();
    let mut complete = false;
    for page in 1..=MAX_HISTORY_PAGES {
        match get_json::<Vec<ApiRelease>>(&client, &format!("{API}?per_page=100&page={page}")) {
            Ok(items) => {
                let done = items.len() < 100;
                history.extend(items);
                if done {
                    complete = true;
                    break;
                }
            }
            Err(_) => break,
        }
    }
    build_release(latest, history, complete, current, target).map(Some)
}
/// Coordinate checks between the resident and editor processes. Automatic
/// attempts never wait on another process; manual requests may wait for it.
pub fn check_coordinated(
    directory: &Path,
    current: &str,
    target: Target,
    manual: bool,
) -> Result<Option<Release>, String> {
    check_coordinated_with(directory, current, target, manual, || {
        check(current, target)
    })
}
/// Startup always checks, except for a result just obtained by the other process.
pub fn check_startup_coordinated(
    directory: &Path,
    current: &str,
    target: Target,
) -> Result<Option<Release>, String> {
    check_coordinated_policy(directory, current, target, true, true, || {
        check(current, target)
    })
}
fn check_coordinated_with(
    directory: &Path,
    current: &str,
    target: Target,
    manual: bool,
    run: impl FnOnce() -> Result<Option<Release>, String>,
) -> Result<Option<Release>, String> {
    check_coordinated_policy(directory, current, target, manual, false, run)
}
fn check_coordinated_policy(
    directory: &Path,
    current: &str,
    target: Target,
    manual: bool,
    startup: bool,
    run: impl FnOnce() -> Result<Option<Release>, String>,
) -> Result<Option<Release>, String> {
    let started = now_seconds();
    let lock = lock_file(directory, "update-request.lock")?;
    let deadline = Instant::now() + Duration::from_secs(95);
    let mut waited = false;
    while lock.try_lock().is_err() {
        if !manual {
            return Err("更新正在后台检查".into());
        }
        if Instant::now() >= deadline {
            return Err("后台检查尚未结束，请稍后重试".into());
        }
        waited = true;
        std::thread::sleep(Duration::from_millis(200));
    }
    let cache = Cache::load(directory, target, current);
    let just_checked = startup
        && cache.last_attempt > 0
        && now_seconds() >= cache.last_attempt
        && now_seconds() - cache.last_attempt < 60;
    if just_checked
        || (!manual && !cache.due(now_seconds()))
        || (manual && waited && cache.last_attempt >= started && cache.last_attempt_succeeded)
    {
        return if cache.last_attempt_succeeded {
            Ok(cache.latest)
        } else {
            Err("最近的后台检查未成功，请稍后重试".into())
        };
    }
    let result = run();
    let at = now_seconds();
    let save_result = modify_cache(directory, current, target, |cache| {
        cache.last_attempt = at;
        cache.last_attempt_succeeded = result.is_ok();
        if let Ok(release) = &result {
            cache.latest = release.clone();
        }
    });
    if let Err(error) = save_result {
        eprintln!("无法保存更新检查状态：{error}");
    }
    result
}
/// The resident checks even while the editor is closed. It never opens a
/// window; the editor reads the shared result after it is reopened.
pub fn start_resident_checks(directory: PathBuf) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(1));
        let mut startup = true;
        while !worker_stop.load(Ordering::Relaxed) {
            if startup {
                let _ = check_startup_coordinated(
                    &directory,
                    env!("CARGO_PKG_VERSION"),
                    Target::current(),
                );
                startup = false;
            } else {
                let _ = check_coordinated(
                    &directory,
                    env!("CARGO_PKG_VERSION"),
                    Target::current(),
                    false,
                );
            }
            std::thread::sleep(Duration::from_secs(15 * 60));
        }
    });
    stop
}
fn parse_digest(value: &str) -> Option<String> {
    let hex = value.strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| hex.to_ascii_lowercase())
}
fn parse_checksum(text: &str, filename: &str) -> Result<String, String> {
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let line = lines.next().ok_or("校验文件为空")?;
    if lines.next().is_some() {
        return Err("校验文件包含多条记录".into());
    }
    let mut parts = line.split_whitespace();
    let hash = parts.next().ok_or("校验文件格式无效")?;
    let name = parts
        .next()
        .ok_or("校验文件缺少文件名")?
        .trim_start_matches('*');
    if parts.next().is_some()
        || name != filename
        || hash.len() != 64
        || !hash.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("校验文件与安装包不匹配".into());
    }
    Ok(hash.to_ascii_lowercase())
}
fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone, Debug)]
pub enum DownloadEvent {
    Progress { received: u64, total: u64 },
    Verifying,
}
struct DownloadDirectory {
    path: PathBuf,
    keep: bool,
}
impl Drop for DownloadDirectory {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
fn unique_download_directory(root: &Path) -> Result<DownloadDirectory, String> {
    let updates = root.join("updates");
    fs::create_dir_all(&updates).map_err(|_| "无法创建更新下载目录".to_string())?;
    for _ in 0..8 {
        let mut random = [0u8; 12];
        getrandom::fill(&mut random).map_err(|_| "无法创建安全下载目录".to_string())?;
        let path = updates.join(hex(&random));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(DownloadDirectory { path, keep: false }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("无法创建更新下载目录".into()),
        }
    }
    Err("无法创建唯一的更新下载目录".into())
}
fn save_stream<R: Read>(
    mut reader: R,
    directory: &Path,
    package: &Package,
    expected: &str,
    cancel: &AtomicBool,
    mut notify: impl FnMut(DownloadEvent),
) -> Result<PathBuf, String> {
    let part = directory.join("download.part");
    let final_path = directory.join(&package.name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)
        .map_err(|_| "无法创建下载文件".to_string())?;
    let mut hash = Sha256::new();
    let mut received = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    let mut last = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("下载已取消".into());
        }
        let count = reader
            .read(&mut buffer)
            .map_err(|_| "下载中断，请检查网络后重试".to_string())?;
        if count == 0 {
            break;
        }
        received = received.checked_add(count as u64).ok_or("下载文件过大")?;
        if received > package.size || received > MAX_PACKAGE {
            return Err("下载文件大小与发行信息不符".into());
        }
        file.write_all(&buffer[..count])
            .map_err(|_| "写入下载文件失败，请检查磁盘空间".to_string())?;
        hash.update(&buffer[..count]);
        if last.elapsed() >= Duration::from_millis(120) {
            notify(DownloadEvent::Progress {
                received,
                total: package.size,
            });
            last = Instant::now();
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("下载已取消".into());
    }
    notify(DownloadEvent::Verifying);
    if received != package.size {
        return Err("下载不完整，请重新下载".into());
    }
    if hex(&hash.finalize()) != expected {
        return Err("下载文件校验失败，已停止使用该文件".into());
    }
    file.sync_all()
        .map_err(|_| "写入下载文件失败".to_string())?;
    drop(file);
    fs::rename(&part, &final_path).map_err(|_| "无法完成下载文件保存".to_string())?;
    Ok(final_path)
}
/// A successfully downloaded package, pinned to the digest checked during download.
/// This proves integrity, not publisher identity. Execution requires explicit consent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedDownload {
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
}

/// Retain “install later” across application restarts. Revalidate both metadata
/// and the file location here; the helper rehashes the file before execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadyUpdate {
    pub release: Release,
    pub download: VerifiedDownload,
}
impl ReadyUpdate {
    pub fn load(root: &Path, target: Target, current: &str) -> Option<Self> {
        let file = File::open(root.join("ready-update.json")).ok()?;
        let mut bytes = Vec::new();
        file.take(256 * 1024).read_to_end(&mut bytes).ok()?;
        let ready: Self = serde_json::from_slice(&bytes).ok()?;
        let package = ready.release.package.as_ref()?;
        let path = ready.download.path.canonicalize().ok()?;
        let updates = root.join("updates").canonicalize().ok()?;
        if !ready.release.valid(target)
            || !is_newer(&ready.release.tag, current).ok()?
            || path.parent().and_then(Path::parent) != Some(updates.as_path())
            || path.file_name()?.to_str()? != package.name
            || ready.download.size != package.size
            || fs::metadata(&path).ok()?.len() != package.size
            || ready.download.sha256.len() != 64
            || !ready.download.sha256.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return None;
        }
        Some(ready)
    }
    pub fn save(&self, root: &Path) -> Result<(), String> {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).map_err(|e| e.to_string())?;
        let temporary = root.join(format!("ready-update-{}.tmp", hex(&random)));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            serde_json::to_writer(&mut file, self).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            drop(file);
            fs::rename(&temporary, root.join("ready-update.json")).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}

/// Download a matching official asset only after explicit user action.
/// Never install without a separate user confirmation: SHA-256 is not a signature.
pub fn download(
    release: &Release,
    target: Target,
    root: &Path,
    cancel: Arc<AtomicBool>,
    notify: impl FnMut(DownloadEvent),
) -> Result<VerifiedDownload, String> {
    if !release.valid(target) {
        return Err("发行信息无效，请前往官方发行页".into());
    }
    let package = release
        .package
        .as_ref()
        .ok_or("当前平台没有可用的安装包或校验文件")?;
    if cancel.load(Ordering::Relaxed) {
        return Err("下载已取消".into());
    }
    let client = client(Duration::from_secs(180))?;
    let mut checksum_bytes = Vec::new();
    response(&client, &package.checksum_url)?
        .take(4097)
        .read_to_end(&mut checksum_bytes)
        .map_err(|_| "无法读取校验文件".to_string())?;
    if checksum_bytes.len() > 4096 {
        return Err("校验文件过长".into());
    }
    let checksum_text = std::str::from_utf8(&checksum_bytes).map_err(|_| "校验文件格式无效")?;
    let expected = parse_checksum(checksum_text, &package.name)?;
    if package
        .digest
        .as_deref()
        .is_some_and(|digest| parse_digest(digest).as_deref() != Some(expected.as_str()))
    {
        return Err("发行信息与校验文件不一致".into());
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("下载已取消".into());
    }
    let mut directory = unique_download_directory(root)?;
    let package_response = response(&client, &package.url)?;
    if package_response
        .content_length()
        .is_some_and(|length| length != package.size)
    {
        return Err("下载文件大小与发行信息不符".into());
    }
    let path = save_stream(
        package_response,
        &directory.path,
        package,
        &expected,
        &cancel,
        notify,
    )?;
    let download = VerifiedDownload {
        path,
        sha256: expected,
        size: package.size,
    };
    ReadyUpdate {
        release: release.clone(),
        download: download.clone(),
    }
    .save(root)?;
    directory.keep = true;
    Ok(download)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn asset(tag: &str, name: &str, size: u64) -> ApiAsset {
        ApiAsset {
            name: name.into(),
            browser_download_url: asset_url(tag, name).unwrap(),
            size,
            digest: None,
        }
    }
    fn release(tag: &str, body: &str) -> ApiRelease {
        let v = stable_version(tag).unwrap();
        let name = Target::WindowsX64.filename(&v).unwrap();
        ApiRelease {
            tag_name: tag.into(),
            draft: false,
            prerelease: false,
            body: Some(body.into()),
            assets: vec![
                asset(tag, &name, 3),
                asset(tag, &format!("{name}.sha256"), 100),
            ],
        }
    }
    #[test]
    fn compares_semver_and_rejects_pre_release() {
        assert!(is_newer("v0.10.0", "0.9.9").unwrap());
        assert!(!is_newer("v0.2.1", "0.2.1").unwrap());
        assert!(is_newer("v0.3.0-rc.1", "0.2.1").is_err());
        assert!(stable_version("v1.0.0+metadata").is_none());
        assert!(stable_version("v../../bad").is_none());
    }
    #[test]
    fn only_exact_official_platform_assets_are_accepted() {
        let latest = release("v0.3.0", "test");
        let version = stable_version(&latest.tag_name).unwrap();
        let good = package_for(&latest, &version, Target::WindowsX64).unwrap();
        assert!(valid_package(
            &latest.tag_name,
            &version,
            Target::WindowsX64,
            &good
        ));
        assert!(package_for(&latest, &version, Target::MacUniversal).is_none());
        let mut bad = good.clone();
        bad.url = bad.url.replace("github.com", "attacker.example");
        assert!(!valid_package(
            &latest.tag_name,
            &version,
            Target::WindowsX64,
            &bad
        ));
        bad = good.clone();
        bad.name = "../../escape.exe".into();
        assert!(!valid_package(
            &latest.tag_name,
            &version,
            Target::WindowsX64,
            &bad
        ));
        bad = good;
        bad.size = MAX_PACKAGE + 1;
        assert!(!valid_package(
            &latest.tag_name,
            &version,
            Target::WindowsX64,
            &bad
        ));
    }
    #[test]
    fn preserves_cross_version_warnings_and_marks_missing_history() {
        let latest = release("v0.3.0", "# 最新\n性能改进");
        let old = release(
            "v0.2.0",
            "# 发布\n## 升级前请注意\n旧文件不可编辑\n## 其他\n其他内容",
        );
        let result = build_release(latest, vec![old], false, "0.1.0", Target::WindowsX64).unwrap();
        assert!(result.has_upgrade_warning);
        assert!(
            result.notes.find("旧文件不可编辑").unwrap() < result.notes.find("性能改进").unwrap()
        );
        assert!(result.notes.contains("历史版本未能完整展示"));
        assert!(!result.history_complete);
    }
    #[test]
    fn markdown_is_inert_readable_and_keeps_link_destinations() {
        let (text, complete) = plain_markdown(
            "## 升级前请注意\n\n**备份旧项目**，查看[说明](https://example.com/notes)。\n\n<script>test</script>",
        );
        assert!(complete);
        assert!(text.contains("升级前请注意"));
        assert!(text.contains("备份旧项目"));
        assert!(text.contains("https://example.com/notes"));
        assert!(text.contains("<script>test</script>"));
        assert!(!text.contains("## "));
        assert!(!text.contains("**"));
    }
    #[test]
    fn huge_unicode_notes_are_bounded_and_marked_incomplete() {
        let result = build_release(
            release("v0.3.0", &"字".repeat(100_000)),
            vec![],
            true,
            "0.2.0",
            Target::WindowsX64,
        )
        .unwrap();
        assert!(result.notes.len() <= MAX_NOTES + 1024);
        assert!(!result.history_complete);
        assert!(result.notes.contains("未能完整展示"));
    }
    #[test]
    fn checksum_is_strict_and_cannot_reference_another_file() {
        let digest = "a".repeat(64);
        assert_eq!(
            parse_checksum(&format!("{digest}  app.exe\n"), "app.exe").unwrap(),
            digest
        );
        assert!(parse_checksum(&format!("{digest}  other.exe\n"), "app.exe").is_err());
        assert!(
            parse_checksum(&format!("{digest}  app.exe\n{digest}  app.exe"), "app.exe").is_err()
        );
        assert!(parse_checksum("not-a-hash app.exe", "app.exe").is_err());
    }
    #[test]
    fn stream_validates_hash_size_and_cancel_without_committing_bad_files() {
        let root =
            std::env::temp_dir().join(format!("image-feedback-update-test-{}", now_seconds()));
        let dir = unique_download_directory(&root).unwrap();
        let package = Package {
            name: "app.exe".into(),
            url: String::new(),
            checksum_url: String::new(),
            size: 3,
            digest: None,
        };
        let correct = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let path = save_stream(
            Cursor::new(b"abc"),
            &dir.path,
            &package,
            correct,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"abc");
        drop(dir);
        let bad = unique_download_directory(&root).unwrap();
        assert!(
            save_stream(
                Cursor::new(b"abc"),
                &bad.path,
                &package,
                &"0".repeat(64),
                &AtomicBool::new(false),
                |_| {}
            )
            .is_err()
        );
        assert!(!bad.path.join("app.exe").exists());
        let bad_path = bad.path.clone();
        drop(bad);
        assert!(!bad_path.exists());
        let cancelled = unique_download_directory(&root).unwrap();
        assert!(
            save_stream(
                Cursor::new(b"abc"),
                &cancelled.path,
                &package,
                correct,
                &AtomicBool::new(true),
                |_| {}
            )
            .is_err()
        );
        assert!(!cancelled.path.join("app.exe").exists());
        drop(cancelled);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn startup_coalesces_recent_checks_but_does_not_wait_for_the_daily_interval() {
        let root = std::env::temp_dir().join(format!(
            "startup-update-test-{}-{}",
            std::process::id(),
            now_seconds()
        ));
        fs::create_dir_all(&root).unwrap();
        let mut cache = Cache {
            last_attempt: now_seconds(),
            last_attempt_succeeded: true,
            ..Cache::default()
        };
        cache.save(&root).unwrap();
        check_coordinated_policy(&root, "0.2.3", Target::WindowsX64, true, true, || {
            panic!("startup must share a just-finished check")
        })
        .unwrap();
        cache.last_attempt -= 61;
        cache.save(&root).unwrap();
        let mut ran = false;
        check_coordinated_policy(&root, "0.2.3", Target::WindowsX64, true, true, || {
            ran = true;
            Ok(None)
        })
        .unwrap();
        assert!(ran);
        cache.last_attempt = now_seconds();
        cache.last_attempt_succeeded = false;
        cache.save(&root).unwrap();
        assert!(
            check_coordinated_policy(&root, "0.2.3", Target::WindowsX64, true, true, || panic!(
                "startup must not duplicate a just-failed check"
            ))
            .is_err()
        );
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn ready_update_survives_restart_but_rejects_stale_missing_and_wrong_platform_files() {
        let root = std::env::temp_dir().join(format!(
            "ready-update-test-{}-{}",
            std::process::id(),
            now_seconds()
        ));
        let directory = unique_download_directory(&root).unwrap();
        let release = build_release(
            release("v0.3.0", "notes"),
            vec![],
            true,
            "0.2.3",
            Target::WindowsX64,
        )
        .unwrap();
        let path = directory.path.join(&release.package.as_ref().unwrap().name);
        fs::write(&path, b"abc").unwrap();
        let ready = ReadyUpdate {
            release,
            download: VerifiedDownload {
                path: path.clone(),
                size: 3,
                sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
            },
        };
        ready.save(&root).unwrap();
        // Replacing an existing receipt must be atomic and supported on Windows too.
        ready.save(&root).unwrap();
        assert_eq!(
            ReadyUpdate::load(&root, Target::WindowsX64, "0.2.3")
                .unwrap()
                .download,
            ready.download
        );
        assert!(ReadyUpdate::load(&root, Target::WindowsX64, "0.3.0").is_none());
        assert!(ReadyUpdate::load(&root, Target::MacUniversal, "0.2.3").is_none());
        let mut invalid = ready.clone();
        invalid.download.path = root.join("outside.exe");
        fs::write(&invalid.download.path, b"abc").unwrap();
        invalid.save(&root).unwrap();
        assert!(ReadyUpdate::load(&root, Target::WindowsX64, "0.2.3").is_none());
        ready.save(&root).unwrap();
        fs::remove_file(path).unwrap();
        assert!(ReadyUpdate::load(&root, Target::WindowsX64, "0.2.3").is_none());
        drop(directory);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn failed_checks_back_off_and_dismissal_expires() {
        let cache = Cache {
            last_attempt: 100,
            last_attempt_succeeded: false,
            dismissed_tag: "v1.0.0".into(),
            dismissed_at: 100,
            ..Cache::default()
        };
        assert!(!cache.due(1000));
        assert!(cache.due(3700));
        assert!(!cache.should_notify("v1.0.0", 1000));
        assert!(cache.should_notify("v1.0.0", 100 + CHECK_INTERVAL));
        assert!(cache.should_notify("v1.1.0", 101));
        assert!(cache.due(99));
    }
    #[test]
    fn failed_shared_check_is_backed_off_but_manual_check_can_retry() {
        let root = unique_download_directory(&std::env::temp_dir()).unwrap();
        let path = &root.path;
        let failed = check_coordinated_with(path, "0.2.0", Target::WindowsX64, false, || {
            Err("network unavailable".into())
        });
        assert!(failed.is_err());
        let cache = Cache::load(path, Target::WindowsX64, "0.2.0");
        assert!(!cache.last_attempt_succeeded && !cache.due(now_seconds()));
        assert!(
            check_coordinated_with(path, "0.2.0", Target::WindowsX64, false, || panic!(
                "retry must be backed off"
            ))
            .is_err()
        );
        assert!(
            check_coordinated_with(path, "0.2.0", Target::WindowsX64, true, || Ok(None))
                .unwrap()
                .is_none()
        );
        assert!(Cache::load(path, Target::WindowsX64, "0.2.0").last_attempt_succeeded);
    }
    #[test]
    fn coordinated_checks_share_results_and_preserve_concurrent_dismissal() {
        let root = unique_download_directory(&std::env::temp_dir()).unwrap();
        let path = root.path.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let first_path = path.clone();
        let worker = std::thread::spawn(move || {
            check_coordinated_with(&first_path, "0.2.0", Target::WindowsX64, false, || {
                started_tx.send(()).unwrap();
                finish_rx.recv().unwrap();
                Ok(Some(
                    build_release(
                        release("v0.3.0", "新版本"),
                        vec![],
                        true,
                        "0.2.0",
                        Target::WindowsX64,
                    )
                    .unwrap(),
                ))
            })
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let busy = check_coordinated_with(&path, "0.2.0", Target::WindowsX64, false, || {
            panic!("duplicate network request")
        });
        assert_eq!(busy.unwrap_err(), "更新正在后台检查");
        mark_dismissed(&path, "0.2.0", Target::WindowsX64, "v0.3.0", now_seconds()).unwrap();
        finish_tx.send(()).unwrap();
        assert!(worker.join().unwrap().unwrap().is_some());
        let cache = Cache::load(&path, Target::WindowsX64, "0.2.0");
        assert!(cache.last_attempt_succeeded);
        assert_eq!(cache.dismissed_tag, "v0.3.0");
        assert_eq!(cache.latest.as_ref().unwrap().tag, "v0.3.0");
        assert!(
            check_coordinated_with(&path, "0.2.0", Target::WindowsX64, false, || panic!(
                "fresh result must be reused"
            ))
            .unwrap()
            .is_some()
        );
    }
    #[test]
    #[ignore = "downloads the current public Windows release to a temporary directory; never executes it"]
    fn live_github_package_download_and_integrity() {
        let found = check("0.1.0", Target::WindowsX64).unwrap().unwrap();
        let root = unique_download_directory(&std::env::temp_dir()).unwrap();
        let path = download(
            &found,
            Target::WindowsX64,
            &root.path,
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        assert_eq!(
            fs::metadata(path.path).unwrap().len(),
            found.package.unwrap().size
        );
    }
    #[test]
    #[ignore = "requires access to the public GitHub API"]
    fn live_github_release_contract() {
        let found = check("0.1.0", Target::WindowsX64).unwrap().unwrap();
        assert!(is_newer(&found.tag, "0.1.0").unwrap());
        assert!(found.valid(Target::WindowsX64));
        assert!(found.package.is_some());
    }
}
