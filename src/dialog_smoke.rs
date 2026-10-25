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
fn close_click(ui: &AppWindow) -> Result<(), String> {
    smoke_request_close(ui)
}
fn cancel(ui: &AppWindow) {
    window_pointer_click(ui, ui.get_confirm_cancel_x(), ui.get_confirm_cancel_y());
}
fn primary(ui: &AppWindow) {
    window_pointer_click(ui, ui.get_confirm_save_x(), ui.get_confirm_save_y());
}
fn require(condition: bool, error: &str) -> Result<(), String> {
    if condition { Ok(()) } else { Err(error.into()) }
}

pub(super) fn start(
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
        ui.invoke_add_global();
        let id = state.borrow().selected.ok_or("global note missing")?;
        let token = ui_token(&state.borrow(), id);
        ui.invoke_note_edited(token, "需要保存的批注与位置说明".into());
        ui.invoke_note_edit_finished(token);
        let original = state.borrow().doc().unwrap().clone();
        close_click(ui)?;
        require(
            ui.get_confirm_visible(),
            "custom close did not show the self-drawn save confirmation",
        )?;
        require(
            ui.get_confirm_show_cancel() && ui.get_confirm_show_discard(),
            "save confirmation did not have all three choices",
        )?;
        save_window_snapshot(ui, &folder.join("save-confirmation.png"))?;
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
            ui.get_confirm_choice() == 0,
            "Tab did not stay in the modal navigation",
        )?;
        key(ui, slint::platform::Key::Escape.into());
        require(
            !ui.get_confirm_visible() && state.borrow().dirty(),
            "Escape did not cancel without discarding modifications",
        )?;
        close_click(ui)?;
        cancel(ui);
        require(
            !ui.get_confirm_visible() && state.borrow().doc() == Some(&original),
            "cancel changed the document",
        )?;

        state.borrow_mut().pending = Some((id, "字".repeat(20_001)));
        close_click(ui)?;
        primary(ui);
        require(
            ui.get_confirm_visible()
                && !ui.get_confirm_saving()
                && !ui.get_confirm_error().is_empty(),
            "invalid text did not stay in the save dialog",
        )?;
        require(
            state.borrow().pending.is_some(),
            "invalid draft disappeared",
        )?;
        save_window_snapshot(ui, &folder.join("validation-error.png"))?;
        cancel(ui);
        state.borrow_mut().pending = None;

        ui.invoke_remove_note(token);
        require(
            ui.get_confirm_visible()
                && ui.get_confirm_destructive()
                && !ui.get_confirm_show_discard(),
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

        show_message(
            ui,
            &mut state.borrow_mut(),
            "无法完成操作",
            "这是隔离回归中的错误提示。当前编辑数据仍保留。",
        );
        require(
            ui.get_confirm_visible()
                && !ui.get_confirm_show_cancel()
                && !ui.get_confirm_show_discard(),
            "error notice did not use a single-action self-drawn dialog",
        )?;
        save_window_snapshot(ui, &folder.join("error-notice.png"))?;
        primary(ui);
        require(
            !ui.get_confirm_visible(),
            "error acknowledgement did not dismiss",
        )?;

        let bad = folder.join("directory-not-file.ifp");
        std::fs::create_dir_all(&bad).map_err(|e| e.to_string())?;
        state.borrow_mut().project = Some(bad);
        close_click(ui)?;
        primary(ui);
        require(
            ui.get_confirm_saving() && state.borrow().worker.is_some(),
            "save did not enter the asynchronous saving state",
        )?;
        cancel(ui);
        require(
            state
                .borrow()
                .confirmation
                .as_ref()
                .is_some_and(|c| c.saving),
            "saving could be cancelled midway",
        )?;
        save_window_snapshot(ui, &folder.join("saving.png"))?;
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
            0,
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
    stage: u8,
    deadline: Instant,
) {
    slint::Timer::single_shot(Duration::from_millis(50), move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if state.borrow().worker.is_some() {
            if Instant::now() > deadline {
                fail(
                    &ui,
                    &folder,
                    &failure,
                    "asynchronous dialog operation timed out".into(),
                );
            } else {
                poll(
                    weak, state, timer, folder, failure, expected, stage, deadline,
                );
            }
            return;
        }
        let result = (|| -> Result<(), String> {
            if stage == 0 {
                require(
                    ui.get_confirm_visible()
                        && !ui.get_confirm_saving()
                        && !ui.get_confirm_error().is_empty(),
                    "filesystem save failure dismissed the confirmation",
                )?;
                require(
                    state.borrow().dirty() && state.borrow().doc() == Some(&expected),
                    "failed save changed or discarded modifications",
                )?;
                save_window_snapshot(&ui, &folder.join("save-error.png"))?;
                cancel(&ui);
                state.borrow_mut().project = Some(folder.join("confirmed-save.ifp"));
                ui.invoke_load_example();
                require(
                    ui.get_confirm_visible()
                        && ui.get_confirm_save_label().as_str() == "保存并继续",
                    "replacing the image did not use save-and-continue",
                )?;
                save_window_snapshot(&ui, &folder.join("save-and-continue.png"))?;
                primary(&ui);
                require(
                    ui.get_confirm_saving(),
                    "confirmed save-and-continue did not start saving",
                )?;
                poll(
                    ui.as_weak(),
                    state.clone(),
                    timer.clone(),
                    folder.clone(),
                    failure.clone(),
                    expected.clone(),
                    1,
                    Instant::now() + Duration::from_secs(10),
                );
            } else {
                require(
                    !ui.get_confirm_visible() && state.borrow().confirmation.is_none(),
                    "successful save did not finish the confirmation",
                )?;
                require(
                    state.borrow().doc().unwrap().annotations.is_empty(),
                    "queued image replacement did not execute after saving",
                )?;
                let (saved, _) = storage::open_project(&folder.join("confirmed-save.ifp"))?;
                require(
                    saved == expected,
                    "save-and-continue did not preserve the previous project",
                )?;
                ui.invoke_add_global();
                close_click(&ui)?;
                require(
                    ui.get_confirm_visible(),
                    "final unsaved close did not prompt",
                )?;
                save_window_snapshot(&ui, &folder.join("discard-close.png"))?;
                let report = serde_json::json!({"result":"passed","method":"real Slint window with scene-dispatched pointer and key events; real asynchronous filesystem save failure and successful save; not OS input automation","checks":["self-drawn close/save prompt","modal pointer and keyboard blocking","Tab navigation and Escape cancellation","cancel preserves data","validation failure retains draft","self-drawn delete confirmation and undo","self-drawn error acknowledgement","save-in-progress controls blocked","filesystem failure retains dialog and data","save succeeds before queued image replacement","discard closes only after explicit choice"],"saved_project":"confirmed-save.ifp"});
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
                window_pointer_click(&ui, ui.get_confirm_discard_x(), ui.get_confirm_discard_y());
            }
            Ok(())
        })();
        if let Err(error) = result {
            fail(&ui, &folder, &failure, error);
        }
    });
}
