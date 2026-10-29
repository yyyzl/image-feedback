//! Debug-only probe for external operating-system pointer acceptance testing.
//! No network, clipboard, or user data; enabled only by an explicit CLI flag.
use super::*;
use std::cell::Cell;

pub(super) fn start(
    args: &[String],
    ui: &AppWindow,
    state: &Rc<RefCell<Editor>>,
) -> Option<Rc<slint::Timer>> {
    let index = args.iter().position(|a| a == "--native-pointer-probe")?;
    let folder = PathBuf::from(args.get(index + 1)?);
    std::fs::create_dir_all(&folder).ok()?;
    let timer = Rc::new(slint::Timer::default());
    let weak = ui.as_weak();
    let state = state.clone();
    let initialized = Cell::new(false);
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(100), move || {
        let Some(ui) = weak.upgrade() else { return; };
        let mut e = state.borrow_mut();
        if ui.get_busy() || e.analysis_loading || e.doc().is_none() { return; }
        if !initialized.get() {
            e.selected = e.doc().and_then(|d| d.annotations.first()).map(|a| a.id);
            sync(&ui, &mut e); ui.invoke_focus_canvas(); initialized.set(true);
        }
        let Some((id, source, target, rotation)) = e.selected_transform() else { return; };
        let data = serde_json::json!({ "id": id, "source": source, "target": target, "rotation": rotation,
            "view": {"x":e.view.x,"y":e.view.y,"scale":e.view.scale},
            "canvas_window_x":ui.get_canvas_window_x(), "canvas_window_y":ui.get_canvas_window_y(),
            "window_scale":ui.window().scale_factor(), "cursor":ui.get_canvas_cursor(),
            "gesture_active":e.gesture.is_some() });
        let _ = std::fs::write(folder.join("state.json"), serde_json::to_vec(&data).unwrap());
        if folder.join("snapshot-request").exists() {
            let _ = save_window_snapshot(&ui, &folder.join("window.png"));
            let _ = std::fs::remove_file(folder.join("snapshot-request"));
        }
    });
    Some(timer)
}
