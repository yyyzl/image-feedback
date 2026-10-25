//! Bounded readiness gate for native-window regressions, not a fixed startup-speed test.
use super::*;
use std::time::Instant;

fn ready(loaded: bool, busy: bool, analysing: bool) -> bool {
    loaded && !busy && !analysing
}

pub(super) fn start(
    weak: slint::Weak<AppWindow>,
    state: Rc<RefCell<Editor>>,
    folder: PathBuf,
    failure: Rc<RefCell<Option<String>>>,
    initial_delay: Duration,
    action: impl FnOnce(&AppWindow) + 'static,
) {
    slint::Timer::single_shot(initial_delay, move || {
        poll(
            weak,
            state,
            folder,
            failure,
            Instant::now() + Duration::from_secs(30),
            action,
        );
    });
}

fn poll(
    weak: slint::Weak<AppWindow>,
    state: Rc<RefCell<Editor>>,
    folder: PathBuf,
    failure: Rc<RefCell<Option<String>>>,
    deadline: Instant,
    action: impl FnOnce(&AppWindow) + 'static,
) {
    let Some(ui) = weak.upgrade() else { return };
    let loaded = ui.get_loaded();
    let busy = ui.get_busy();
    let analysing = state.borrow().analysis_loading;
    if ready(loaded, busy, analysing) {
        action(&ui);
    } else if Instant::now() >= deadline {
        let error = format!(
            "Demo readiness timed out after 30 seconds: loaded={loaded}, busy={busy}, analysing={analysing}"
        );
        *failure.borrow_mut() = Some(error.clone());
        let _ = std::fs::create_dir_all(&folder);
        let _ = std::fs::write(folder.join("failed.txt"), error);
        let _ = save_window_snapshot(&ui, &folder.join("startup-failed.png"));
        let _ = ui.hide();
    } else {
        // Release all state borrows before the next event-loop turn or test callback.
        slint::Timer::single_shot(Duration::from_millis(50), move || {
            poll(weak, state, folder, failure, deadline, action);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::ready;

    #[test]
    fn waits_for_load_file_work_and_native_analysis() {
        for loaded in [false, true] {
            for busy in [false, true] {
                for analysing in [false, true] {
                    assert_eq!(
                        ready(loaded, busy, analysing),
                        loaded && !busy && !analysing
                    );
                }
            }
        }
    }
}
