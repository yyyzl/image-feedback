//! Copying produces durable image/annotation snapshots, not unexplained filenames.
use super::*;

pub(super) fn prepare(
    root: &Path,
    doc: Document,
    asset: &Asset,
    cached: Option<DeliveryCache>,
) -> Result<DeliveryCache, String> {
    let source = delivery::verified_source_path(asset);
    let package = match cached {
        Some(cache) if cache.snapshot == doc && cache.package.is_complete() => cache.package,
        _ => delivery::prepare(root, &doc, asset)?,
    };
    Ok(DeliveryCache {
        snapshot: doc,
        package,
        source,
    })
}
fn text(cache: &DeliveryCache, request: CopyRequest) -> Result<String, String> {
    match request {
        CopyRequest::Feedback => Ok(cache
            .package
            .clipboard_text_with_source(&cache.snapshot, cache.source.as_deref())),
        CopyRequest::OriginalPath => Ok(cache
            .source
            .as_deref()
            .unwrap_or(&cache.package.original)
            .display()
            .to_string()),
        CopyRequest::Base64 => cache.package.embedded_text(&cache.snapshot),
    }
}
pub(super) fn begin(
    ui: &AppWindow,
    e: &mut Editor,
    worker_timer: &Rc<slint::Timer>,
    toast_timer: &slint::Timer,
    request: CopyRequest,
) {
    if e.worker.is_some() || e.confirmation.is_some() || e.message.is_some() {
        return;
    }
    let validation = if request == CopyRequest::Feedback {
        feedback_payload(e).map(|_| ())
    } else {
        e.flush()
    };
    if let Err(error) = validation {
        finish(ui, e, toast_timer, Err(error), request);
        return;
    }
    let (Some(doc), Some(asset)) = (e.doc().cloned(), e.asset.clone()) else {
        finish(ui, e, toast_timer, Err("请先导入图片".into()), request);
        return;
    };
    if request != CopyRequest::Base64 && asset.source_path.is_none() {
        if let Some(cache) = e
            .delivery
            .as_ref()
            .filter(|cache| cache.snapshot == doc && cache.package.is_complete())
            .cloned()
        {
            let payload = text(&cache, request);
            let result = Ok((cache, payload));
            finish(ui, e, toast_timer, result, request);
            return;
        }
    }
    let root = e
        .delivery_root
        .clone()
        .unwrap_or_else(|| app_dir().join("feedback-deliveries"));
    let previous = e.delivery.clone();
    set_status(ui, "正在保存原图、编号参考图与反馈包…");
    start_work(ui, e, worker_timer, move || {
        let result = prepare(&root, doc, &asset, previous).map(|cache| {
            let payload = text(&cache, request);
            (cache, payload)
        });
        Work::Delivered(result, request)
    });
}
pub(super) fn finish(
    ui: &AppWindow,
    e: &mut Editor,
    timer: &slint::Timer,
    result: Result<(DeliveryCache, Result<String, String>), String>,
    request: CopyRequest,
) {
    let result = result.and_then(|(cache, text)| {
        // Keep the prepared files even if another program temporarily owns the clipboard.
        e.delivery = Some(cache);
        let text = text?;
        write_feedback_clipboard(e, &text)
    });
    match result {
        Ok(()) => {
            let (title, message) = match request {
                CopyRequest::Feedback => ("复制成功", "原图路径和精简修改建议已复制。"),
                CopyRequest::OriginalPath => (
                    "原图路径已复制",
                    "优先使用未改变的源文件；本次交付快照也已保存。",
                ),
                CopyRequest::Base64 => (
                    "便携数据已复制",
                    "包含原图和编号参考图。\n接收工具需要支持 Base64 解码。",
                ),
            };
            set_status(ui, title);
            show_toast(ui, timer, true, title, message);
            if request == CopyRequest::OriginalPath {
                ui.set_copy_complete(false);
            }
        }
        Err(error) => {
            set_status(ui, format!("复制失败：{error}"));
            show_toast(ui, timer, false, "复制失败", &error);
        }
    }
    sync(ui, e);
}
pub(super) fn open_folder(ui: &AppWindow, e: &mut Editor) {
    let Some(folder) = e
        .delivery
        .as_ref()
        .map(|cache| cache.package.folder.clone())
    else {
        return;
    };
    if e.clipboard_probe {
        e.opened_delivery = Some(folder);
        return;
    }
    let result = (|| -> Result<(), String> {
        #[cfg(windows)]
        let mut command = std::process::Command::new("explorer.exe");
        #[cfg(target_os = "macos")]
        let mut command = std::process::Command::new("open");
        #[cfg(not(any(windows, target_os = "macos")))]
        let mut command = std::process::Command::new("xdg-open");
        command
            .arg(&folder)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        Ok(())
    })();
    if let Err(error) = result {
        show_message(ui, e, "无法打开反馈包目录", error);
    }
}
