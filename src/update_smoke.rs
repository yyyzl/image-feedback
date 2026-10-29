//! Native Slint update-dialog regression; no network or installation occurs.
use super::*;

fn fail(ui: &AppWindow, folder: &Path, failure: &Rc<RefCell<Option<String>>>, message: String) {
    *failure.borrow_mut() = Some(message.clone());
    let _ = std::fs::create_dir_all(folder);
    let _ = std::fs::write(folder.join("failed.txt"), message);
    let _ = save_window_snapshot(ui, &folder.join("failed-window.png"));
    let _ = ui.hide();
}
fn key(ui: &AppWindow, text: slint::SharedString) {
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
}
pub(super) fn start(
    ui: &AppWindow,
    state: Rc<RefCell<Editor>>,
    folder: PathBuf,
    failure: Rc<RefCell<Option<String>>>,
    updates: Rc<update_ui::UpdateHandle>,
) {
    let setup = (|| -> Result<(), String> {
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        {
            let mut editor = state.borrow_mut();
            let history = editor.history.as_mut().ok_or("demo document missing")?;
            let id = history.apply(Document::add_global)?;
            history.apply(|doc| doc.set_text(id, "这条反馈必须在更新提示关闭后保留".into()))?;
            sync(ui, &mut editor);
        }
        ui.window()
            .set_size(slint::LogicalSize::new(1000., 688.).to_physical(ui.window().scale_factor()));
        ui.set_update_phase(2);
        ui.set_update_title("发现新版本".into());
        ui.set_update_subtitle("图片反馈 v0.3.0 已发布".into());
        ui.set_update_current_version("v0.2.1".into());
        ui.set_update_version("v0.3.0".into());
        ui.set_update_can_download(true);
        ui.set_update_has_warning(true);
        ui.set_update_notes(
            format!(
                "升级提醒（来自发行说明）\n\nv0.3.0\n升级前请注意\n旧版项目需要先完成编辑。\n\n{}",
                "• 改进画布缩放、批注输入与更新过程的稳定性。\n".repeat(55)
            )
            .into(),
        );
        ui.set_update_visible(true);
        Ok(())
    })();
    if let Err(error) = setup {
        fail(ui, &folder, &failure, error);
        return;
    }
    let weak = ui.as_weak();
    slint::Timer::single_shot(Duration::from_millis(500), move || {
        let Some(ui) = weak.upgrade() else { return };
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
        let result = (|| -> Result<(), String> {
            if !ui.get_update_visible() || !ui.get_update_has_focus() {
                return Err("update dialog did not receive focus".into());
            }
            let content = ui.get_update_scroll_height();
            let viewport = ui.get_update_scroll_viewport();
            if content <= viewport + 1. {
                return Err(format!(
                    "long release notes did not overflow: content={content} viewport={viewport}"
                ));
            }
            save_window_snapshot(&ui, &folder.join("update-long.png"))?;
            for _ in 0..3 {
                key(&ui, slint::platform::Key::Tab.into());
            }
            key(&ui, slint::platform::Key::DownArrow.into());
            if ui.get_update_scroll_y() >= 0. {
                return Err("keyboard could not scroll update notes".into());
            }
            save_window_snapshot(&ui, &folder.join("update-scrolled.png"))?;
            key(&ui, slint::platform::Key::End.into());
            if (ui.get_update_scroll_y() - (viewport - content)).abs() > 2. {
                return Err("End did not reach the end of the release notes".into());
            }
            key(&ui, slint::platform::Key::Home.into());
            if ui.get_update_scroll_y().abs() > 1. {
                return Err("Home did not return to the start of the release notes".into());
            }
            let before = state
                .borrow()
                .doc()
                .cloned()
                .ok_or("feedback document disappeared")?;
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::CloseRequested);
            if ui.get_update_visible() || ui.get_confirm_visible() {
                return Err(
                    "close should dismiss the update dialog without exiting or discarding feedback"
                        .into(),
                );
            }
            if state.borrow().doc() != Some(&before) {
                return Err("closing update dialog changed feedback".into());
            }
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::CloseRequested);
            if !ui.get_confirm_visible() {
                return Err("normal close did not protect existing feedback".into());
            }
            ui.invoke_confirm_response(0);
            if ui.get_confirm_visible() || state.borrow().doc() != Some(&before) {
                return Err("cancelling close did not preserve feedback".into());
            }
            ui.set_update_phase(5);
            ui.set_update_progress(17);
            ui.set_update_title("正在下载更新".into());
            ui.set_update_subtitle("关闭此窗口后可继续编辑。".into());
            ui.set_update_detail("下载将在后台继续；退出编辑器会中断下载。".into());
            ui.set_update_progress_label("1.5 MB / 8.6 MB · 17%".into());
            ui.set_update_visible(true);
            if ui.get_update_progress_fill_x().abs() > 0.1 {
                return Err("download progress must start at the left edge".into());
            }
            save_window_snapshot(&ui, &folder.join("update-downloading.png"))?;
            ui.set_update_visible(false);
            updates.seed_smoke_ready(&ui, folder.join("missing-package-never-execute.exe"));
            save_window_snapshot(&ui, &folder.join("update-ready.png"))?;
            ui.invoke_update_dismiss();
            if ui.get_update_visible() || !ui.get_update_notice_visible() {
                return Err("install later must retain the ready notice".into());
            }
            save_window_snapshot(&ui, &folder.join("update-later.png"))?;
            ui.invoke_update_check();
            if ui.get_update_phase() != 7 {
                return Err("manual check replaced a ready update".into());
            }
            if cfg!(windows) {
                ui.invoke_update_primary();
                if !ui.get_confirm_visible() || ui.get_confirm_primary_label() != "仍要退出并安装"
                {
                    return Err("install did not invoke the feedback guard".into());
                }
                save_window_snapshot(&ui, &folder.join("update-guard.png"))?;
                ui.invoke_confirm_response(0);
                if ui.get_confirm_visible()
                    || state.borrow().doc() != Some(&before)
                    || state.borrow().worker.is_some()
                {
                    return Err("cancelling install changed feedback or started the helper".into());
                }
                ui.invoke_update_open();
                ui.invoke_update_primary();
                ui.invoke_confirm_response(2);
                if ui.get_update_phase() != 10 || !ui.get_busy() {
                    return Err("confirmed install did not start preflight".into());
                }
                ui.invoke_update_dismiss();
                if !ui.get_update_visible() {
                    return Err("preflight could be dismissed while exit was pending".into());
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            fail(&ui, &folder, &failure, error);
            return;
        }
        if cfg!(windows) {
            wait_for_preflight_failure(&ui, state, folder, failure, 0);
        } else {
            finish(&ui, &folder, &failure, false);
        }
    });
}

fn wait_for_preflight_failure(
    ui: &AppWindow,
    state: Rc<RefCell<Editor>>,
    folder: PathBuf,
    failure: Rc<RefCell<Option<String>>>,
    attempts: u32,
) {
    if ui.get_update_phase() == 11 && state.borrow().worker.is_none() {
        if !state.borrow().has_feedback() || !ui.get_update_visible() || ui.get_busy() {
            fail(
                ui,
                &folder,
                &failure,
                "failed installation did not preserve a usable editor".into(),
            );
            return;
        }
        if let Err(error) = save_window_snapshot(ui, &folder.join("update-install-error.png")) {
            fail(ui, &folder, &failure, error);
            return;
        }
        finish(ui, &folder, &failure, true);
        return;
    }
    if attempts >= 400 {
        fail(
            ui,
            &folder,
            &failure,
            "installation preflight failure timed out".into(),
        );
        return;
    }
    let weak = ui.as_weak();
    slint::Timer::single_shot(Duration::from_millis(100), move || {
        if let Some(ui) = weak.upgrade() {
            wait_for_preflight_failure(&ui, state, folder, failure, attempts + 1);
        }
    });
}

fn finish(
    ui: &AppWindow,
    folder: &Path,
    failure: &Rc<RefCell<Option<String>>>,
    windows_preflight: bool,
) {
    let report = serde_json::json!({
        "result": "passed", "platform": std::env::consts::OS,
        "method": "native Slint events and real helper preflight with a deliberately missing package; no network or installer execution",
        "windows_helper_failure_checked": windows_preflight,
        "checks": ["long release notes scroll", "keyboard focus", "left-aligned progress", "close preserves feedback",
            "install later retains ready notice", "manual check preserves downloaded state",
            "install feedback guard and cancellation", "helper preflight failure preserves the editor"],
        "images": ["update-long.png", "update-scrolled.png", "update-downloading.png", "update-ready.png", "update-later.png", "update-guard.png", "update-install-error.png"]
    });
    let result = serde_json::to_vec_pretty(&report)
        .map_err(|e| e.to_string())
        .and_then(|bytes| {
            std::fs::write(folder.join("update-report.json"), bytes).map_err(|e| e.to_string())
        });
    if let Err(error) = result {
        fail(ui, folder, failure, error);
    } else {
        let _ = ui.hide();
    }
}
