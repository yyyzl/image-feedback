//! Real-window dialog regression. All filesystem outputs stay in the supplied test directory.
use super::*;
use std::time::Instant;

fn fail(ui: &AppWindow, folder: &Path, failure: &Rc<RefCell<Option<String>>>, error: String) {
    *failure.borrow_mut() = Some(error.clone());
    let _ = std::fs::create_dir_all(folder);
    let _ = std::fs::write(folder.join("failed.txt"), error);
    let _ = ui.hide();
}
fn key(ui: &AppWindow, text: slint::SharedString) {
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
}
fn shortcut(ui: &AppWindow, text: &str, shift: bool) {
    use slint::platform::{Key, WindowEvent};
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    if shift {
        ui.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Shift.into(),
        });
    }
    key(ui, text.into());
    if shift {
        ui.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Shift.into(),
        });
    }
    ui.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}
fn close_click(ui: &AppWindow) -> Result<(), String> {
    smoke_request_close(ui)
}
fn cancel(ui: &AppWindow) {
    window_pointer_click(ui, ui.get_confirm_cancel_x(), ui.get_confirm_cancel_y());
}
fn primary(ui: &AppWindow) {
    window_pointer_click(ui, ui.get_confirm_primary_x(), ui.get_confirm_primary_y());
}
fn require(condition: bool, error: &str) -> Result<(), String> {
    if condition { Ok(()) } else { Err(error.into()) }
}

/// 对齐网页版的窗口回归：使用隔离数据和真实 Slint 场景输入，不触碰系统剪贴板。
pub(super) fn start_alignment(
    ui: &AppWindow,
    state: &Rc<RefCell<Editor>>,
    timer: &Rc<slint::Timer>,
    folder: PathBuf,
    failure: Rc<RefCell<Option<String>>>,
) {
    let result = (|| -> Result<(), String> {
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let (mut doc, bytes, name) = {
            let e = state.borrow();
            let asset = e.asset.as_ref().ok_or("alignment image missing")?;
            (
                e.doc().unwrap().clone(),
                asset.original.clone(),
                asset.name.clone(),
            )
        };
        {
            // Capture the empty workspace without exposing a separate project action.
            let mut e = state.borrow_mut();
            if let Some(analysis) = e.analysis.take() {
                analysis.cancel();
            }
            e.history = None;
            e.asset = None;
            e.selected = None;
            e.pending = None;
            e.analysis_loading = false;
            sync(ui, &mut e);
        }
        save_window_snapshot(ui, &folder.join("empty.png"))?;
        let region = doc.add_region(Rect {
            x: 48.,
            y: 199.,
            width: 510.,
            height: 158.,
        })?;
        doc.set_text(
            region,
            "将按钮改为统一描边样式，并调整间距与文字层级".into(),
        )?;
        let block = doc.add_region(Rect {
            x: 780.,
            y: 363.,
            width: 300.,
            height: 286.,
        })?;
        doc.move_annotation(block, 30., 15.)?;
        doc.set_text(block, "主视觉向右移动，保留标题空间".into())?;
        let point = doc.add_point(Point { x: 272., y: 495. })?;
        doc.set_text(point, "按钮颜色更醒目".into())?;
        let global = doc.add_global()?;
        doc.set_text(global, "保持整体简洁，突出主要操作".into())?;
        let asset = Asset::from_bytes(bytes, name)?;
        let thumb = preview(&asset);
        install(ui, &mut state.borrow_mut(), doc, asset, thumb);
        timer.restart();
        ui.invoke_select_note(1);
        Ok(())
    })();
    if let Err(error) = result {
        fail(ui, &folder, &failure, error);
        return;
    }
    alignment_poll(
        ui.as_weak(),
        state.clone(),
        timer.clone(),
        folder,
        failure,
        0,
        Instant::now() + Duration::from_secs(20),
    );
}

fn alignment_poll(
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
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
        if !ui.get_active_note_visible() {
            if Instant::now() >= deadline {
                let _ = save_window_snapshot(&ui, &folder.join("failed-window.png"));
                fail(
                    &ui,
                    &folder,
                    &failure,
                    format!(
                        "alignment stage {stage} did not reveal the active note: top={}, bottom={}, scroll={}, viewport={}",
                        ui.get_active_note_top(),
                        ui.get_active_note_bottom(),
                        ui.get_note_scroll_y(),
                        ui.get_notes_viewport_height(),
                    ),
                );
            } else {
                alignment_poll(weak, state, timer, folder, failure, stage, deadline);
            }
            return;
        }
        let result = (|| -> Result<(), String> {
            match stage {
                0 => {
                    canvas_input_checks(&ui, &state, &folder)?;
                    save_window_snapshot(&ui, &folder.join("expanded.png"))?;
                    let (x, y) = (ui.get_active_note_header_x(), ui.get_active_note_header_y());
                    window_pointer_click(
                        &ui,
                        ui.get_active_note_editor_x(),
                        ui.get_active_note_editor_y(),
                    );
                    shortcut(&ui, "a", false);
                    key(&ui, "首轮草稿，收起时提交".into());
                    window_pointer_click(&ui, x, y);
                    require(
                        state.borrow().selected.is_none() && state.borrow().pending.is_none(),
                        "header collapse did not finish the text draft",
                    )?;
                    require(
                        state.borrow().doc().unwrap().annotations[0].text == "首轮草稿，收起时提交",
                        "collapsing lost typed feedback",
                    )?;
                    save_window_snapshot(&ui, &folder.join("collapsed.png"))?;
                    window_pointer_click(&ui, x, y);
                }
                1 => {
                    shortcut(&ui, "a", false);
                    key(&ui, "后续".into());
                    ui.invoke_fit_view();
                    key(&ui, "继续".into());
                    require(
                        state
                            .borrow()
                            .pending
                            .as_ref()
                            .is_some_and(|(_, text)| text == "后续继续"),
                        "ordinary model sync reset the editor caret",
                    )?;
                    key(&ui, slint::platform::Key::Delete.into());
                    key(&ui, slint::platform::Key::Backspace.into());
                    require(
                        state.borrow().doc().unwrap().annotations.len() == 4,
                        "text deletion removed an annotation",
                    )?;
                    key(&ui, slint::platform::Key::Escape.into());
                    require(
                        state.borrow().pending.is_none()
                            && state.borrow().doc().unwrap().annotations[0].text == "后续",
                        "Escape failed to cancel only the pending text",
                    )?;
                    window_pointer_click(
                        &ui,
                        ui.get_active_note_editor_x(),
                        ui.get_active_note_editor_y(),
                    );
                    shortcut(&ui, "a", false);
                    key(&ui, "提交并结束".into());
                    let enter: slint::SharedString = slint::platform::Key::Return.into();
                    shortcut(&ui, enter.as_str(), false);
                    require(
                        state.borrow().selected.is_none() && state.borrow().pending.is_none(),
                        "Ctrl/Command+Enter did not finish editing",
                    )?;
                    require(
                        state.borrow().doc().unwrap().annotations[0].text == "提交并结束",
                        "finishing editing lost text",
                    )?;
                    ui.invoke_select_note(1);
                }
                2 => {
                    shortcut(&ui, "a", false);
                    key(&ui, "字".repeat(20_001).into());
                    let enter: slint::SharedString = slint::platform::Key::Return.into();
                    shortcut(&ui, enter.as_str(), false);
                    require(
                        ui.get_confirm_visible() && ui.get_confirm_has_focus(),
                        "failed Ctrl/Command+Enter moved focus behind the error dialog",
                    )?;
                    primary(&ui);
                    window_pointer_click(
                        &ui,
                        ui.get_active_note_header_x(),
                        ui.get_active_note_header_y(),
                    );
                    require(
                        ui.get_confirm_visible() && state.borrow().selected == Some(1),
                        "invalid draft did not block collapse",
                    )?;
                    require(
                        state
                            .borrow()
                            .pending
                            .as_ref()
                            .is_some_and(|(_, text)| text.chars().count() == 20_001),
                        "invalid draft was discarded",
                    )?;
                    save_window_snapshot(&ui, &folder.join("draft-error.png"))?;
                    primary(&ui);
                }
                3 => {
                    ui.invoke_copy_feedback();
                    require(
                        ui.get_toast_visible()
                            && !ui.get_toast_success()
                            && state.borrow().pending.is_some(),
                        "copy validation did not keep the draft and report failure",
                    )?;
                    save_window_snapshot(&ui, &folder.join("copy-error.png"))?;
                    ui.invoke_dismiss_toast();
                    shortcut(&ui, "a", false);
                    key(&ui, "修正后的意见".into());
                    require(
                        state
                            .borrow()
                            .pending
                            .as_ref()
                            .is_some_and(|(_, text)| text == "修正后的意见"),
                        "validation failure did not restore the note editor focus",
                    )?;
                    let (x, y) = (ui.get_active_note_delete_x(), ui.get_active_note_delete_y());
                    window_pointer_click(
                        &ui,
                        ui.get_active_note_header_x(),
                        ui.get_active_note_header_y(),
                    );
                    window_pointer_click(&ui, x, y);
                    require(
                        ui.get_confirm_visible()
                            && ui.get_confirm_choice() == 0
                            && state.borrow().selected.is_none(),
                        "collapsed delete did not use an independent target and safe confirmation",
                    )?;
                    cancel(&ui);
                    require(
                        state.borrow().doc().unwrap().annotations.len() == 4,
                        "cancelling deletion changed data",
                    )?;
                    window_pointer_click(&ui, x, y);
                    primary(&ui);
                    require(
                        state.borrow().doc().unwrap().annotations.len() == 3,
                        "header delete did not remove the note",
                    )?;
                    shortcut(&ui, "z", false);
                    require(
                        state.borrow().doc().unwrap().annotations.len() == 4,
                        "header deletion could not be undone",
                    )?;
                    let mut doc = {
                        let e = state.borrow();
                        let image = &e.doc().unwrap().image;
                        Document::new(image.file_name.clone(), image.width, image.height)?
                    };
                    for index in 1..=1000 {
                        let id = doc.add_global()?;
                        if index == 1000 {
                            doc.set_text(id, "长意见完整保留，收起时仅显示摘要。".repeat(180))?;
                        }
                    }
                    let mut e = state.borrow_mut();
                    e.history = Some(History::new(doc));
                    e.selected = Some(1000);
                    e.revealed_selection = None;
                    sync(&ui, &mut e);
                }
                4 => {
                    require(
                        ui.get_note_count() == 1000
                            && ui
                                .get_notes()
                                .row_data(999)
                                .is_some_and(|row| row.number == 1000),
                        "1000-note numbering was truncated",
                    )?;
                    save_window_snapshot(&ui, &folder.join("thousand-notes.png"))?;
                    // 显式按 Slint 缩放换成物理尺寸，避免环境覆盖缩放时后端仍用系统比例。
                    ui.window().set_size(
                        slint::LogicalSize::new(1000., 688.)
                            .to_physical(ui.window().scale_factor()),
                    );
                }
                5 => {
                    save_window_snapshot(&ui, &folder.join("minimum-window.png"))?;
                    let actual = ui.window().size().to_logical(ui.window().scale_factor());
                    require(
                        (actual.width - 1000.).abs() <= 1. && (actual.height - 688.).abs() <= 1.,
                        "minimum-window test did not reach 1000x688 logical pixels",
                    )?;
                    require(
                        ui.get_toolbar_fits(),
                        "minimum window overlaps import/history controls",
                    )?;
                    require(
                        (ui.get_zoom_right_gap() - 48.).abs() < 1.
                            && (ui.get_toast_right() - ui.get_copy_button_right()).abs() < 1.
                            && ui.get_toast_bottom() < ui.get_copy_button_top(),
                        "minimum window misplaced the canvas controls or copy toast",
                    )?;
                    require(
                        ui.get_active_note_visible() && ui.get_viewport_width() >= 500.,
                        "minimum window clipped the active editor or canvas",
                    )?;
                    ui.invoke_focus_copy();
                    require(
                        ui.get_copy_help_visible(),
                        "cross-device copy help was not keyboard discoverable",
                    )?;
                    save_window_snapshot(&ui, &folder.join("keyboard-copy-help.png"))?;
                    let before = ui.get_note_scroll_y();
                    ui.window()
                        .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                            position: slint::LogicalPosition::new(
                                ui.get_active_note_header_x(),
                                ui.get_active_note_header_y(),
                            ),
                            delta_x: 0.,
                            delta_y: 400.,
                        });
                    let scrolled = ui.get_note_scroll_y();
                    require(
                        scrolled > before + 100.,
                        "manual note-list scroll did not move",
                    )?;
                    ui.invoke_fit_view();
                    finish_alignment(&ui, &folder, failure.clone(), scrolled);
                    return Ok(());
                }
                _ => unreachable!(),
            }
            alignment_poll(
                ui.as_weak(),
                state.clone(),
                timer.clone(),
                folder.clone(),
                failure.clone(),
                stage + 1,
                Instant::now() + Duration::from_secs(20),
            );
            Ok(())
        })();
        if let Err(error) = result {
            fail(&ui, &folder, &failure, error);
        }
    });
}

fn canvas_input_checks(
    ui: &AppWindow,
    state: &Rc<RefCell<Editor>>,
    folder: &Path,
) -> Result<(), String> {
    use slint::platform::WindowEvent;
    ui.invoke_fit_view();
    let original = state.borrow().doc().unwrap().clone();
    let initial = state.borrow().view;
    require(
        (ui.get_zoom_right_gap() - 48.).abs() < 1.,
        "zoom controls are not anchored near the canvas right edge",
    )?;
    require(
        !ui.get_can_reset_view(),
        "initial view showed a reset action",
    )?;
    let x = ui.get_viewport_width() * 0.35;
    let y = ui.get_viewport_height() * 0.6;
    let position =
        slint::LogicalPosition::new(ui.get_canvas_window_x() + x, ui.get_canvas_window_y() + y);
    require(
        ui.invoke_canvas_accepts_scroll(position.x, position.y),
        "canvas gesture hit region is unavailable",
    )?;
    ui.set_touchpad_scroll(true);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerScrolled {
        position,
        delta_x: 36.,
        delta_y: -24.,
    });
    {
        let e = state.borrow();
        require(
            (e.view.x - initial.x - 36.).abs() < 0.01
                && (e.view.y - initial.y + 24.).abs() < 0.01
                && e.view.scale == initial.scale,
            "touchpad scroll did not pan both axes at unchanged scale",
        )?;
        require(
            e.doc() == Some(&original),
            "view gesture changed document data",
        )?;
    }
    require(ui.get_can_reset_view(), "panned view did not offer reset")?;
    save_window_snapshot(ui, &folder.join("touchpad-pan.png"))?;
    ui.set_touchpad_scroll(false);
    let anchor = state.borrow().point(x, y);
    ui.window().dispatch_event(WindowEvent::PointerScrolled {
        position,
        delta_x: 0.,
        delta_y: 60.,
    });
    {
        let e = state.borrow();
        let after = e.point(x, y);
        require(
            e.view.scale > initial.scale
                && (anchor.x - after.x).abs() < 0.01
                && (anchor.y - after.y).abs() < 0.01,
            "mouse wheel did not zoom around the pointer",
        )?;
    }
    save_window_snapshot(ui, &folder.join("wheel-zoom.png"))?;
    let before = state.borrow().view;
    ui.set_touchpad_scroll(true);
    let controls =
        slint::LogicalPosition::new(ui.get_zoom_plus_center_x(), ui.get_zoom_plus_center_y());
    ui.window().dispatch_event(WindowEvent::PointerScrolled {
        position: controls,
        delta_x: 25.,
        delta_y: 30.,
    });
    {
        let e = state.borrow();
        require(
            e.view.x == before.x && e.view.y == before.y && e.view.scale == before.scale,
            "zoom control scroll leaked to the canvas",
        )?;
    }
    ui.set_touchpad_scroll(false);
    window_pointer_click(
        ui,
        ui.get_reset_view_center_x(),
        ui.get_reset_view_center_y(),
    );
    require(
        !ui.get_can_reset_view()
            && !state
                .borrow()
                .can_reset_view(ui.get_viewport_width(), ui.get_viewport_height()),
        "reset icon did not restore the imported centered view and disappear",
    )?;
    save_window_snapshot(ui, &folder.join("view-reset.png"))?;
    Ok(())
}

fn finish_alignment(
    ui: &AppWindow,
    folder: &Path,
    failure: Rc<RefCell<Option<String>>>,
    scrolled: f32,
) {
    let weak = ui.as_weak();
    let folder = folder.to_path_buf();
    slint::Timer::single_shot(Duration::from_millis(70), move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let result = (|| -> Result<(), String> {
            require(
                (ui.get_note_scroll_y() - scrolled).abs() < 1.,
                "ordinary redraw pulled the manually scrolled list back to the editor",
            )?;
            std::fs::write(folder.join("ui-report.json"), serde_json::to_vec_pretty(&serde_json::json!({
                "result":"passed", "platform":std::env::consts::OS,
                "native_windows_touchpad_initialized":ui.get_native_touchpad_ready(),
                "method":"real Slint window, scene-dispatched keyboard/pointer events, isolated data and clipboard; not physical IME or screen-reader certification",
                "checks":["collapsed/expanded rows and header toggle", "collapse commits drafts", "model sync preserves caret", "Delete/Backspace edit text only", "Escape cancels pending text", "Ctrl/Command+Enter commits and closes", "invalid text blocks collapse and restores focus", "copy failure preserves the draft", "independent collapsed delete, safe default, cancel and undo", "1000 notes with full text retained", "1000x688 logical window", "keyboard discoverable cross-device help", "manual list scroll survives ordinary redraw", "two-axis touchpad pan preserves scale and document", "mouse wheel zoom preserves pointer anchor", "conditional reset restores imported view", "zoom controls consume scrolling"]
            })).unwrap()).map_err(|e| e.to_string())?;
            Ok(())
        })();
        if let Err(error) = result {
            fail(&ui, &folder, &failure, error);
        } else {
            let _ = ui.hide();
        }
    });
}

pub(super) fn start(
    ui: &AppWindow,
    state: &Rc<RefCell<Editor>>,
    timer: &Rc<slint::Timer>,
    folder: PathBuf,
    failure: Rc<RefCell<Option<String>>>,
) {
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
    ui.invoke_add_global();
    let token = ui.get_active_note_token();
    ui.invoke_note_edited(token, "需要保存的批注与位置说明".into());
    ui.invoke_note_edit_finished(token);
    let weak = ui.as_weak();
    let state = state.clone();
    let timer = timer.clone();
    // 等待真实布局和输入框的焦点通知，随后再发送键盘事件。
    slint::Timer::single_shot(Duration::from_millis(70), move || {
        if let Some(ui) = weak.upgrade() {
            start_ready(&ui, &state, &timer, folder, failure);
        }
    });
}

fn start_ready(
    ui: &AppWindow,
    state: &Rc<RefCell<Editor>>,
    timer: &Rc<slint::Timer>,
    folder: PathBuf,
    failure: Rc<RefCell<Option<String>>>,
) {
    let result = (|| -> Result<Document, String> {
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        require(
            ui.get_loaded() && !ui.get_busy(),
            "dialog demo was not ready",
        )?;
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
        let id = state.borrow().selected.ok_or("global note missing")?;
        let token = ui_token(&state.borrow(), id);
        let original = state.borrow().doc().unwrap().clone();
        save_window_snapshot(ui, &folder.join("focused-editor.png"))?;
        // 以真实键入落入批注草稿为准，避免把延迟的焦点状态通知当作输入就绪。
        key(ui, "x".into());
        require(
            state
                .borrow()
                .pending
                .as_ref()
                .is_some_and(|(actual, text)| *actual == id && text.contains('x')),
            "keyboard input did not reach the active note editor",
        )?;
        shortcut(ui, "z", false);
        require(
            state.borrow().doc() == Some(&original),
            "keyboard undo did not restore the uncommitted text draft",
        )?;
        shortcut(ui, "z", false);
        require(
            state
                .borrow()
                .doc()
                .unwrap()
                .annotation(id)
                .unwrap()
                .text
                .is_empty(),
            "Ctrl/Command+Z did not undo text with the note editor focused",
        )?;
        shortcut(ui, "z", true);
        require(
            state.borrow().doc() == Some(&original),
            "Shift+Ctrl/Command+Z did not redo text",
        )?;
        shortcut(ui, "z", false);
        shortcut(ui, "y", false);
        require(
            state.borrow().doc() == Some(&original),
            "Ctrl+Y did not redo text",
        )?;
        shortcut(ui, "z", false);
        shortcut(ui, "z", false);
        require(
            state.borrow().doc().unwrap().annotations.is_empty(),
            "second undo did not remove the new annotation",
        )?;
        shortcut(ui, "z", true);
        shortcut(ui, "z", true);
        require(
            state.borrow().doc() == Some(&original),
            "redo stopped working after the focused editor was removed",
        )?;
        ui.invoke_load_example();
        require(
            ui.get_confirm_visible(),
            "image replacement did not protect current feedback",
        )?;
        shortcut(ui, "z", false);
        require(
            state.borrow().doc() == Some(&original),
            "history shortcut escaped the modal",
        )?;
        cancel(ui);
        require(
            state.borrow().doc() == Some(&original),
            "cancelling replacement changed the document",
        )?;

        close_click(ui)?;
        require(
            ui.get_confirm_visible()
                && ui.get_confirm_show_cancel()
                && ui.get_confirm_destructive(),
            "close did not show a safe two-choice discard confirmation",
        )?;
        require(
            ui.get_confirm_choice() == 0,
            "destructive confirmation did not default to cancel",
        )?;
        save_window_snapshot(ui, &folder.join("close-confirmation.png"))?;
        window_pointer_click(
            ui,
            ui.get_canvas_window_x() + 150.,
            ui.get_canvas_window_y() + 200.,
        );
        require(
            state.borrow().doc() == Some(&original),
            "modal backdrop allowed canvas edits",
        )?;
        key(ui, "x".into());
        require(
            state.borrow().doc() == Some(&original),
            "modal keyboard input edited the background",
        )?;
        key(ui, slint::platform::Key::Tab.into());
        require(
            ui.get_confirm_choice() == 2,
            "Tab did not stay in the modal navigation",
        )?;
        key(ui, slint::platform::Key::Escape.into());
        require(
            !ui.get_confirm_visible() && state.borrow().has_feedback(),
            "Escape discarded modifications",
        )?;

        let (reply, answer) = mpsc::channel();
        request_resident_quit(ui, &mut state.borrow_mut(), timer, reply);
        if !(ui.get_confirm_visible()
            && ui.get_confirm_title() == "退出图片反馈？"
            && ui.get_confirm_choice() == 0)
        {
            let editor = state.borrow();
            return Err(format!(
                "resident quit confirmation: visible={}, title={:?}, choice={}, composing={}, worker={}, existing_confirmation={}, message={}, feedback={}, reply={:?}",
                ui.get_confirm_visible(),
                ui.get_confirm_title(),
                ui.get_confirm_choice(),
                ui.get_text_composing(),
                editor.worker.is_some(),
                editor.confirmation.is_some(),
                editor.message.is_some(),
                editor.has_feedback(),
                answer.try_recv(),
            ));
        }
        save_window_snapshot(ui, &folder.join("resident-quit-confirmation.png"))?;
        key(ui, slint::platform::Key::Escape.into());
        require(
            answer.try_recv() == Ok(quit_ipc::Decision::Cancelled)
                && state.borrow().doc() == Some(&original)
                && !ui.get_confirm_visible(),
            "cancelling resident quit did not preserve the feedback",
        )?;
        ui.invoke_load_example();
        let (reply, answer) = mpsc::channel();
        request_resident_quit(ui, &mut state.borrow_mut(), timer, reply);
        require(
            answer.try_recv() == Ok(quit_ipc::Decision::Busy) && ui.get_confirm_visible(),
            "resident quit interrupted an existing modal operation",
        )?;
        cancel(ui);

        state.borrow_mut().pending = Some((id, "字".repeat(20_001)));
        ui.invoke_load_example();
        require(
            ui.get_confirm_visible(),
            "invalid draft did not protect image replacement",
        )?;
        cancel(ui);
        require(
            state
                .borrow()
                .pending
                .as_ref()
                .is_some_and(|(_, text)| text.chars().count() == 20_001),
            "cancelling replacement discarded an invalid draft",
        )?;
        state.borrow_mut().pending = None;

        ui.invoke_remove_note(token);
        require(
            ui.get_confirm_visible() && ui.get_confirm_destructive(),
            "delete did not use the shared destructive dialog",
        )?;
        save_window_snapshot(ui, &folder.join("delete-confirmation.png"))?;
        cancel(ui);
        require(
            state.borrow().doc() == Some(&original),
            "cancelled delete removed data",
        )?;
        ui.invoke_remove_note(token);
        primary(ui);
        require(
            state.borrow().doc().unwrap().annotations.is_empty(),
            "delete primary action did not remove the note",
        )?;
        ui.invoke_undo();
        require(
            state.borrow().doc() == Some(&original),
            "undo did not recover the deleted note",
        )?;

        window_pointer_click(
            ui,
            ui.get_clear_feedback_center_x(),
            ui.get_clear_feedback_center_y(),
        );
        require(
            ui.get_confirm_visible() && ui.get_confirm_destructive(),
            "clear-current button did not open its confirmation",
        )?;
        save_window_snapshot(ui, &folder.join("clear-confirmation.png"))?;
        cancel(ui);
        require(
            state.borrow().doc() == Some(&original),
            "cancelling clear changed current feedback",
        )?;
        let asset = state.borrow().asset.clone().unwrap();
        window_pointer_click(
            ui,
            ui.get_clear_feedback_center_x(),
            ui.get_clear_feedback_center_y(),
        );
        primary(ui);
        require(
            ui.get_loaded()
                && state.borrow().doc().unwrap().annotations.is_empty()
                && ui.get_note_count() == 0,
            "clear-current did not keep the image and remove feedback",
        )?;
        require(
            Arc::ptr_eq(state.borrow().asset.as_ref().unwrap(), &asset),
            "clear-current replaced the source image",
        )?;
        save_window_snapshot(ui, &folder.join("cleared-feedback.png"))?;
        shortcut(ui, "z", false);
        require(
            state.borrow().doc() == Some(&original),
            "undo did not restore all cleared feedback in one step",
        )?;
        shortcut(ui, "z", true);
        require(
            state.borrow().doc().unwrap().annotations.is_empty(),
            "redo did not clear feedback again",
        )?;
        shortcut(ui, "z", false);
        require(
            state.borrow().doc() == Some(&original),
            "restoring cleared feedback failed",
        )?;

        show_message(
            ui,
            &mut state.borrow_mut(),
            "无法完成操作",
            "这是隔离回归中的错误提示。当前编辑数据仍保留。",
        );
        require(
            ui.get_confirm_visible() && !ui.get_confirm_show_cancel(),
            "error notice did not use a single-action self-drawn dialog",
        )?;
        save_window_snapshot(ui, &folder.join("error-notice.png"))?;
        primary(ui);
        require(
            !ui.get_confirm_visible(),
            "error acknowledgement did not dismiss",
        )?;

        ui.invoke_load_example();
        require(
            ui.get_confirm_visible(),
            "image replacement did not ask before discarding",
        )?;
        cancel(ui);
        require(
            state.borrow().doc() == Some(&original),
            "replacement cancellation lost feedback",
        )?;
        ui.invoke_load_example();
        require(
            ui.get_confirm_visible(),
            "repeated replacement did not prompt",
        )?;
        save_window_snapshot(ui, &folder.join("replace-confirmation.png"))?;
        primary(ui);
        require(
            state.borrow().worker.is_some(),
            "confirmed replacement did not start loading",
        )?;
        Ok(original)
    })();
    match result {
        Ok(expected) => poll(
            ui.as_weak(),
            state.clone(),
            timer.clone(),
            folder,
            failure,
            expected,
            Instant::now() + Duration::from_secs(10),
        ),
        Err(error) => fail(ui, &folder, &failure, error),
    }
}

fn poll(
    weak: slint::Weak<AppWindow>,
    state: Rc<RefCell<Editor>>,
    timer: Rc<slint::Timer>,
    folder: PathBuf,
    failure: Rc<RefCell<Option<String>>>,
    expected: Document,
    deadline: Instant,
) {
    slint::Timer::single_shot(Duration::from_millis(50), move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if state.borrow().worker.is_some() {
            if Instant::now() > deadline {
                fail(&ui, &folder, &failure, "image replacement timed out".into());
            } else {
                poll(weak, state, timer, folder, failure, expected, deadline);
            }
            return;
        }
        let result = (|| -> Result<(), String> {
            require(
                !ui.get_confirm_visible(),
                "replacement confirmation did not close",
            )?;
            require(
                state
                    .borrow()
                    .doc()
                    .is_some_and(|doc| doc.annotations.is_empty() && doc != &expected),
                "confirmed replacement did not load a clean image",
            )?;
            ui.invoke_add_global();
            close_click(&ui)?;
            require(
                ui.get_confirm_visible(),
                "closing with feedback did not prompt",
            )?;
            save_window_snapshot(&ui, &folder.join("discard-close.png"))?;
            let report = serde_json::json!({
                "result":"passed",
                "method":"real Slint window with scene-dispatched pointer and key events; not OS input automation",
                "checks":[
                    "close and image replacement require an explicit discard",
                    "resident quit cancellation and busy modal preserve feedback",
                    "destructive confirmation defaults to cancel",
                    "modal pointer and keyboard blocking",
                    "Escape and cancel preserve feedback and invalid draft",
                    "self-drawn delete confirmation and undo",
                    "self-drawn error acknowledgement",
                    "confirmed replacement loads a clean image",
                    "clear current keeps image and supports one-step undo/redo",
                    "real key input, undo and redo in focused note editor"
                ]
            });
            std::fs::write(
                folder.join("dialog-report.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .map_err(|e| e.to_string())?;
            let watchdog = ui.as_weak();
            let failed = failure.clone();
            let output = folder.clone();
            slint::Timer::single_shot(Duration::from_millis(500), move || {
                if let Some(ui) = watchdog.upgrade() {
                    fail(
                        &ui,
                        &output,
                        &failed,
                        "discard did not close the editor".into(),
                    );
                }
            });
            primary(&ui);
            Ok(())
        })();
        if let Err(error) = result {
            fail(&ui, &folder, &failure, error);
        }
    });
}
