//! Scene-input regression for newest-note focus, durable images, and optional Base64.
use super::*;
use std::time::Instant;
fn require(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn fail(ui: &AppWindow, folder: &Path, failure: &Rc<RefCell<Option<String>>>, error: String) {
    *failure.borrow_mut() = Some(error.clone());
    let _ = std::fs::write(folder.join("failed.txt"), error);
    let _ = save_window_snapshot(ui, &folder.join("failed-window.png"));
    let _ = ui.hide();
}
fn type_text(ui: &AppWindow, text: &str) {
    use slint::platform::WindowEvent;
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.into() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: text.into() });
}
pub(super) fn start(
    ui: &AppWindow,
    state: &Rc<RefCell<Editor>>,
    timer: &Rc<slint::Timer>,
    folder: PathBuf,
    failure: Rc<RefCell<Option<String>>>,
) {
    let result = (|| -> Result<(), String> {
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        require(
            ui.get_loaded() && !ui.get_busy(),
            "delivery demo was not ready",
        )?;
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
        let pixels = image::load_from_memory(include_bytes!("../assets/demo.png"))
            .map_err(|e| e.to_string())?
            .to_rgba8();
        let asset = Asset::from_rgba(pixels, "剪贴板图片.png")?;
        let doc = Document::new(
            asset.name.clone(),
            asset.pixels.width(),
            asset.pixels.height(),
        )?;
        let thumb = preview(&asset);
        {
            let mut e = state.borrow_mut();
            install(ui, &mut e, doc, asset, thumb);
            e.delivery_root = Some(folder.join("prepared"));
        }
        for index in 0..14 {
            ui.invoke_add_global();
            let token = {
                let e = state.borrow();
                ui_token(&e, e.selected.unwrap())
            };
            ui.invoke_note_edited(token, format!("已有意见 {}", index + 1).into());
            ui.invoke_note_edit_finished(token);
        }
        ui.invoke_choose_tool(0);
        ui.invoke_toggle_smart();
        let position = |x: f32, y: f32| {
            let e = state.borrow();
            (e.view.x + x * e.view.scale, e.view.y + y * e.view.scale)
        };
        let (a, b) = position(200., 200.);
        let (c, d) = position(460., 320.);
        canvas_press(ui, a, b, 0);
        canvas_move(ui, c, d);
        canvas_release(ui, c, d);
        require(
            ui.get_active_note_token() == 15,
            "new rectangle did not become the active newest note",
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        fail(ui, &folder, &failure, error);
        return;
    }
    poll(
        ui.as_weak(),
        state.clone(),
        timer.clone(),
        folder,
        failure,
        0,
        Instant::now() + Duration::from_secs(15),
    );
}
fn poll(
    weak: slint::Weak<AppWindow>,
    state: Rc<RefCell<Editor>>,
    timer: Rc<slint::Timer>,
    folder: PathBuf,
    failure: Rc<RefCell<Option<String>>>,
    stage: u8,
    deadline: Instant,
) {
    slint::Timer::single_shot(Duration::from_millis(70), move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if stage == 0 {
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
        }
        if state.borrow().worker.is_some() || (stage == 0 && !ui.get_active_note_visible()) {
            if Instant::now() > deadline {
                fail(
                    &ui,
                    &folder,
                    &failure,
                    format!(
                        "delivery stage {stage} timed out (visible={}, focused={}, scroll={})",
                        ui.get_active_note_visible(),
                        ui.get_active_editor_focused(),
                        ui.get_note_scroll_y()
                    ),
                );
            } else {
                poll(weak, state, timer, folder, failure, stage, deadline);
            }
            return;
        }
        let result = (|| -> Result<(), String> {
            match stage {
                0 => {
                    require(
                        ui.get_note_scroll_y() < -100.,
                        "newest note was not scrolled into view",
                    )?;
                    save_window_snapshot(&ui, &folder.join("newest-note.png"))?;
                    let before = state.borrow().doc().unwrap().clone();
                    let scale = state.borrow().view.scale;
                    window_pointer_click(
                        &ui,
                        ui.get_zoom_plus_center_x(),
                        ui.get_zoom_plus_center_y(),
                    );
                    require(
                        state.borrow().view.scale > scale && state.borrow().doc() == Some(&before),
                        "floating zoom control edited the canvas or failed to zoom",
                    )?;
                    type_text(&ui, "这条是最新批注");
                    require(
                        state.borrow().pending.as_ref().is_some_and(|(id, text)| {
                            Some(*id) == state.borrow().selected && text.contains("这条是最新批注")
                        }),
                        "keyboard input did not go to the latest note",
                    )?;
                    window_pointer_click(
                        &ui,
                        ui.get_copy_button_center_x(),
                        ui.get_copy_button_center_y(),
                    );
                    require(
                        state.borrow().worker.is_some(),
                        "uncached copy did not prepare a durable snapshot asynchronously",
                    )?;
                }
                1 => {
                    let text = state
                        .borrow()
                        .clipboard_written
                        .clone()
                        .ok_or("feedback was not copied")?;
                    let e = state.borrow();
                    let cache = e.delivery.as_ref().ok_or("prepared snapshot missing")?;
                    require(
                        cache.prepared.is_complete(),
                        "copied image paths are not durable files",
                    )?;
                    require(
                        std::fs::read_dir(&cache.prepared.folder)
                            .map_err(|e| e.to_string())?
                            .count()
                            == 2
                            && !cache.prepared.folder.join("feedback-package.zip").exists(),
                        "copy unexpectedly created an export package",
                    )?;
                    require(
                        (ui.get_toast_right() - ui.get_copy_button_right()).abs() < 1.
                            && ui.get_toast_bottom() < ui.get_copy_button_top(),
                        "copy toast is not aligned above the feedback button",
                    )?;
                    require(
                        text.starts_with(&format!("原图：{}\n", cache.prepared.original.display()))
                            && text
                                .matches(cache.prepared.original.to_str().unwrap())
                                .count()
                                == 1,
                        "compact clipboard must contain exactly one original path",
                    )?;
                    require(
                        ![
                            "reference.png",
                            "feedback-package.zip",
                            "交付文件夹",
                            "原图快照",
                            "本机路径不是公开链接",
                        ]
                        .iter()
                        .any(|extra| text.contains(extra))
                            && text.contains("15. 范围")
                            && text.contains("这条是最新批注"),
                        "clipboard contains delivery boilerplate or omitted newest feedback",
                    )?;
                    require(
                        std::fs::read(&cache.prepared.original).map_err(|e| e.to_string())?
                            == e.asset.as_ref().unwrap().original,
                        "pasted original bytes changed",
                    )?;
                    std::fs::write(folder.join("copied-feedback.md"), &text)
                        .map_err(|e| e.to_string())?;
                    drop(e);
                    save_window_snapshot(&ui, &folder.join("copy-ready.png"))?;
                    ui.invoke_copy_portable_data();
                }
                2 => {
                    use base64::Engine;
                    require(
                        !ui.get_copy_complete(),
                        "Base64 success incorrectly marked the feedback button as copied",
                    )?;
                    let text = state
                        .borrow()
                        .clipboard_written
                        .clone()
                        .ok_or("portable data missing")?;
                    let data: serde_json::Value =
                        serde_json::from_str(&text).map_err(|e| e.to_string())?;
                    require(
                        data["schema"] == "image-feedback-embedded-1",
                        "portable schema missing",
                    )?;
                    require(
                        data["document"]["annotations"].as_array().unwrap().len() == 15,
                        "portable data omitted annotations",
                    )?;
                    let encoded = data["original"]["data_url"]
                        .as_str()
                        .unwrap()
                        .split_once(',')
                        .unwrap()
                        .1;
                    let decoded = base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .map_err(|e| e.to_string())?;
                    require(
                        decoded == state.borrow().asset.as_ref().unwrap().original,
                        "Base64 original did not roundtrip",
                    )?;
                    ui.invoke_copy_original_path();
                    let e = state.borrow();
                    let cache = e.delivery.as_ref().unwrap();
                    require(
                        e.clipboard_written.as_deref() == cache.prepared.original.to_str(),
                        "pasted source path did not point to the saved PNG",
                    )?;
                    drop(e);
                    ui.invoke_open_feedback_folder();
                    require(
                        state.borrow().opened_delivery.is_some(),
                        "snapshot-folder action did not target the delivery",
                    )?;
                    let source = std::path::absolute(folder.join("external-source.png"))
                        .map_err(|e| e.to_string())?;
                    let bytes = state.borrow().asset.as_ref().unwrap().original.clone();
                    std::fs::write(&source, &bytes).map_err(|e| e.to_string())?;
                    let asset = Asset::open(&source)?;
                    let doc = Document::new(
                        asset.name.clone(),
                        asset.pixels.width(),
                        asset.pixels.height(),
                    )?;
                    let thumb = preview(&asset);
                    install(&ui, &mut state.borrow_mut(), doc, asset, thumb);
                    ui.invoke_copy_original_path();
                }
                3 => {
                    let source = std::path::absolute(folder.join("external-source.png"))
                        .map_err(|e| e.to_string())?;
                    require(
                        state.borrow().clipboard_written.as_deref() == source.to_str(),
                        "valid disk source was not preferred",
                    )?;
                    std::fs::write(&source, b"changed after import").map_err(|e| e.to_string())?;
                    ui.invoke_copy_original_path();
                }
                _ => {
                    let e = state.borrow();
                    let cache = e.delivery.as_ref().ok_or("disk snapshot missing")?;
                    require(
                        cache.source.is_none()
                            && e.clipboard_written.as_deref() == cache.prepared.original.to_str(),
                        "changed source did not fall back to the matching immutable snapshot",
                    )?;
                    require(
                        std::fs::read(&cache.prepared.original).map_err(|e| e.to_string())?
                            == e.asset.as_ref().unwrap().original,
                        "source change corrupted the imported pixels",
                    )?;
                    drop(e);
                    let report = serde_json::json!({"result":"passed","method":"real Slint window; scene pointer/keyboard dispatch, isolated clipboard sink, local fixture filesystem","checks":["new rectangle scrolls to note 15","latest input focused and receives text","floating zoom control does not draw annotations","uncached copy prepares original/reference snapshots asynchronously","copied paths exist and original bytes are unchanged","clipboard only contains one original path and concise feedback","Base64 contains both image data and annotations","image path and snapshot-folder actions","unchanged disk origin preferred","changed disk origin falls back to snapshot"]});
                    std::fs::write(
                        folder.join("delivery-report.json"),
                        serde_json::to_vec_pretty(&report).unwrap(),
                    )
                    .map_err(|e| e.to_string())?;
                    let _ = ui.hide();
                    return Ok(());
                }
            }
            poll(
                ui.as_weak(),
                state.clone(),
                timer.clone(),
                folder.clone(),
                failure.clone(),
                stage + 1,
                Instant::now() + Duration::from_secs(15),
            );
            Ok(())
        })();
        if let Err(error) = result {
            fail(&ui, &folder, &failure, error);
        }
    });
}
