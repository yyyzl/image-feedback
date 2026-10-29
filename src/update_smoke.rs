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
            ui.set_update_phase(7);
            ui.set_update_title("安装包已准备好".into());
            ui.set_update_subtitle("由你决定何时安装。".into());
            ui.set_update_detail("安装包已通过完整性校验，但不能确认发布者身份。先复制需要保留的反馈并退出编辑器及托盘，再手动运行安装程序；请留意系统发布者提示。".into());
            ui.set_update_visible(true);
            save_window_snapshot(&ui, &folder.join("update-ready.png"))?;
            let report = serde_json::json!({"result":"passed","platform":std::env::consts::OS,"method":"native Slint window with scene-dispatched keyboard and close events; mock release metadata, no network or installer","checks":["long release notes scroll within the dialog","keyboard focus and scrolling","closing update dialog keeps annotations","normal close still invokes existing feedback protection","cancelling close retains feedback","manual installation guidance"],"scroll_content":content,"scroll_viewport":viewport,"images":["update-long.png","update-scrolled.png","update-ready.png"]});
            std::fs::write(
                folder.join("update-report.json"),
                serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })();
        if let Err(error) = result {
            fail(&ui, &folder, &failure, error);
            return;
        }
        let _ = ui.hide();
    });
}
