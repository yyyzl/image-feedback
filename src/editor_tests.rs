use super::*;
#[test]
fn two_axis_pan_and_reset_only_change_the_view() {
    let mut e = editor();
    e.fit(800., 600.);
    let original = e.doc().unwrap().clone();
    let scale = e.view.scale;
    assert!(!e.can_reset_view(800., 600.));
    e.pan_view(35., -24.);
    assert_eq!(e.view.scale, scale);
    assert!(e.can_reset_view(800., 600.));
    assert_eq!(e.doc(), Some(&original));
    assert!(!e.history.as_ref().unwrap().can_undo());
    e.pan_view(-35., 24.);
    assert!(!e.can_reset_view(800., 600.));
    e.view.scale *= 1.25;
    assert!(e.can_reset_view(800., 600.));
    e.fit(800., 600.);
    assert!(!e.can_reset_view(800., 600.));
    e.pan_view(f32::INFINITY, 10.);
    assert!(!e.can_reset_view(800., 600.));
}
#[test]
fn continuous_input_is_not_guessed_from_small_wheel_deltas() {
    use slint::winit_030::winit::{dpi::PhysicalPosition, event::MouseScrollDelta};
    assert!(continuous_scroll(&MouseScrollDelta::PixelDelta(
        PhysicalPosition::new(-3., 8.)
    )));
    assert!(!continuous_scroll(&MouseScrollDelta::LineDelta(0., 1.)));
    assert!(!continuous_scroll(&MouseScrollDelta::LineDelta(0., 0.01)));
}
#[test]
fn note_rows_reconcile_by_stable_id_when_display_numbers_shift() {
    let row = |stable: &str, token: i32| NoteRow {
        stable_id: stable.into(),
        id: token,
        number: token,
        ..Default::default()
    };
    let model = VecModel::from(vec![
        row("large-10", 1),
        row("large-20", 2),
        row("large-30", 3),
    ]);
    reconcile_note_rows(&model, vec![row("large-20", 1), row("large-30", 2)]);
    assert_eq!(model.row_count(), 2);
    assert_eq!(model.row_data(0).unwrap().stable_id, "large-20");
    assert_eq!(model.row_data(1).unwrap().number, 2);
    reconcile_note_rows(
        &model,
        vec![row("large-10", 1), row("large-20", 2), row("large-30", 3)],
    );
    assert_eq!(model.row_count(), 3);
    assert_eq!(model.row_data(1).unwrap().stable_id, "large-20");
    assert_eq!(model.row_data(1).unwrap().number, 2);
}
#[test]
fn note_toggle_commits_the_draft_once_then_collapses() {
    let mut e = editor();
    e.selected = Some(1);
    e.pending = Some((1, "收起前提交的意见".into()));
    e.toggle_note(1).unwrap();
    assert!(e.selected.is_none());
    assert!(e.pending.is_none());
    assert_eq!(
        e.doc().unwrap().annotation(1).unwrap().text,
        "收起前提交的意见"
    );
    assert!(e.history.as_mut().unwrap().undo());
    assert_eq!(e.doc().unwrap().annotation(1).unwrap().text, "");
    assert!(!e.history.as_ref().unwrap().can_undo());
}

#[test]
fn invalid_draft_blocks_collapse_and_selection_changes() {
    let mut e = editor();
    let second = e
        .history
        .as_mut()
        .unwrap()
        .apply(Document::add_global)
        .unwrap();
    e.selected = Some(1);
    let draft = "字".repeat(20_001);
    e.pending = Some((1, draft.clone()));
    assert!(e.toggle_note(1).is_err());
    assert_eq!(e.selected, Some(1));
    assert!(e.toggle_note(second).is_err());
    assert_eq!(e.selected, Some(1));
    assert_eq!(e.pending.as_ref().unwrap().1, draft);
}

#[test]
fn summary_is_a_single_line_view_without_changing_the_original() {
    let original = "  完整意见\n\t保留第二行　中文空格  ";
    assert_eq!(note_summary(original), "完整意见 保留第二行 中文空格");
    assert_eq!(note_summary(" \n\t　"), "未填写意见");
    assert!(original.contains('\n'));
}

#[test]
fn toggling_an_unchanged_note_does_not_add_history() {
    let mut e = editor();
    e.toggle_note(1).unwrap();
    assert_eq!(e.selected, Some(1));
    e.toggle_note(1).unwrap();
    assert_eq!(e.selected, None);
    assert!(!e.history.as_ref().unwrap().can_undo());
}
#[test]
fn clear_feedback_keeps_image_and_view_and_undo_restores_all_edits() {
    let mut e = editor();
    e.history
        .as_mut()
        .unwrap()
        .apply(|doc| {
            doc.move_annotation(1, 12., 8.)?;
            doc.add_point(Point { x: 180., y: 120. })?;
            doc.add_global()?;
            Ok(())
        })
        .unwrap();
    e.view.scale = 1.5;
    e.view.x = 42.;
    e.pending = Some((1, "尚未完成的批注草稿".into()));
    let mut expected = e.doc().unwrap().clone();
    expected.set_text(1, "尚未完成的批注草稿".into()).unwrap();
    e.clear_feedback().unwrap();
    assert!(e.doc().unwrap().annotations.is_empty());
    assert_eq!(e.doc().unwrap().image, expected.image);
    assert_eq!(e.doc().unwrap().next_id, expected.next_id);
    assert_eq!((e.view.scale, e.view.x), (1.5, 42.));
    assert!(!e.has_feedback());
    assert!(e.history.as_mut().unwrap().undo());
    assert_eq!(e.doc(), Some(&expected));
    assert!(e.history.as_mut().unwrap().redo());
    assert!(e.doc().unwrap().annotations.is_empty());
}

#[test]
fn failed_clear_keeps_the_invalid_draft_and_existing_edits() {
    let mut e = editor();
    let before = e.doc().cloned();
    e.pending = Some((1, "字".repeat(20_001)));
    assert!(e.clear_feedback().is_err());
    assert_eq!(e.doc(), before.as_ref());
    assert!(e.pending.is_some());
}

#[test]
fn clean_action_does_not_prompt_or_modify_document() {
    let mut e = editor();
    e.history
        .as_mut()
        .unwrap()
        .apply(|doc| {
            doc.annotations.clear();
            Ok(())
        })
        .unwrap();
    let before = e.doc().cloned();
    assert_eq!(
        e.request_action(PendingAction::Demo { annotated: false }),
        Some(PendingAction::Demo { annotated: false })
    );
    assert!(e.confirmation.is_none());
    assert_eq!(e.doc(), before.as_ref());
    assert_eq!(
        e.request_action(PendingAction::New),
        Some(PendingAction::New)
    );
    assert!(e.confirmation.is_none());
    assert_eq!(e.doc(), before.as_ref());
}
#[test]
fn cancel_confirmation_preserves_uncommitted_draft() {
    let mut e = editor();
    let original = e.doc().unwrap().clone();
    e.pending = Some((1, "尚未提交的文字".into()));
    assert!(e.request_action(PendingAction::Close).is_none());
    assert_eq!(e.doc(), Some(&original));
    e.cancel_confirmation();
    assert!(e.confirmation.is_none());
    assert!(e.has_feedback());
    assert_eq!(e.pending.as_ref().unwrap().1, "尚未提交的文字");
}
#[test]
fn cancelling_new_preserves_feedback_and_invalid_draft() {
    let mut e = editor();
    let before = e.doc().unwrap().clone();
    e.pending = Some((1, "字".repeat(20_001)));
    assert!(e.request_action(PendingAction::New).is_none());
    assert_eq!(e.confirmation.as_ref().unwrap().action, PendingAction::New);
    e.cancel_confirmation();
    assert_eq!(e.doc(), Some(&before));
    assert_eq!(e.pending.as_ref().unwrap().1.chars().count(), 20_001);
}

#[test]
fn new_workspace_clears_editing_state_but_preserves_files_and_preferences() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/new-workspace-tests")
        .join(format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    std::fs::create_dir_all(&directory).unwrap();
    let original = directory.join("original.png");
    let reference = directory.join("reference.png");
    let bytes = include_bytes!("../assets/demo.png");
    std::fs::write(&original, bytes).unwrap();
    std::fs::write(&reference, bytes).unwrap();
    let asset = Arc::new(Asset::open(&original).unwrap());
    let mut e = editor();
    e.asset = Some(asset.clone());
    e.selected = Some(1);
    e.pending = Some((1, "待输入的批注".into()));
    e.view.scale = 2.;
    e.view.x = 42.;
    e.view.fit = false;
    e.tool = 2;
    e.smart_enabled = false;
    e.clipboard_written = Some("此前复制的反馈".into());
    e.delivery = Some(DeliveryCache {
        snapshot: e.doc().unwrap().clone(),
        prepared: PreparedFeedback {
            folder: directory.clone(),
            original: original.clone(),
            reference: reference.clone(),
        },
        source: Some(original.clone()),
    });
    e.analysis = Some(AnalysisService::new());
    e.analysis_generation = e.analysis.as_ref().unwrap().submit(asset);
    e.analysis_loading = true;

    e.new_workspace();

    assert!(e.doc().is_none() && e.asset.is_none() && e.pending.is_none());
    assert!(e.selected.is_none() && e.gesture.is_none() && e.delivery.is_none());
    assert_eq!(
        (e.view.scale, e.view.x, e.view.fit, e.tool),
        (1., 0., true, 0)
    );
    assert!(!e.analysis_loading && e.candidates.is_empty() && e.eligible.is_empty());
    assert!(!e.smart_enabled);
    assert_eq!(e.clipboard_written.as_deref(), Some("此前复制的反馈"));
    assert_eq!(std::fs::read(&original).unwrap(), bytes);
    assert_eq!(std::fs::read(&reference).unwrap(), bytes);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn new_is_blocked_while_a_background_task_is_active() {
    let mut e = editor();
    let before = e.doc().unwrap().clone();
    let (_sender, receiver) = mpsc::channel();
    e.worker = Some(receiver);
    assert!(e.request_action(PendingAction::New).is_none());
    assert!(e.confirmation.is_none());
    assert_eq!(e.doc(), Some(&before));
}

#[test]
fn a_second_request_cannot_replace_the_confirmed_intent() {
    let mut e = editor();
    e.pending = Some((1, "draft".into()));
    e.request_action(PendingAction::Close);
    assert!(e.request_action(PendingAction::Paste).is_none());
    assert_eq!(
        e.confirmation.as_ref().unwrap().action,
        PendingAction::Close
    );
}
#[test]
fn discard_confirmation_does_not_silently_commit_invalid_text() {
    let mut e = editor();
    e.pending = Some((1, "x".repeat(20_001)));
    let before = e.doc().unwrap().clone();
    let next = PendingAction::Load {
        path: PathBuf::from("next.png"),
    };
    e.request_action(next.clone());
    assert_eq!(e.discard_confirmation(), Some(next));
    assert_eq!(e.doc(), Some(&before));
    assert_eq!(e.pending.as_ref().unwrap().1.len(), 20_001);
}
#[test]
fn moved_block_without_text_produces_copyable_feedback() {
    let mut e = editor();
    e.history
        .as_mut()
        .unwrap()
        .apply(|d| d.move_annotation(1, 100., 20.))
        .unwrap();
    let before = e.doc().unwrap().clone();
    let (text, count, moved) = feedback_payload(&mut e).unwrap();
    assert_eq!((count, moved), (1, 1));
    assert!(text.contains("移动图块") && text.contains("来源") && text.contains("目标"));
    assert_eq!(e.doc(), Some(&before));
    e.clipboard_probe = true;
    write_feedback_clipboard(&mut e, &text).unwrap();
    assert_eq!(e.clipboard_written.as_deref(), Some(text.as_str()));
}
#[test]
fn clipboard_failure_preserves_last_successful_value_and_allows_retry() {
    let mut e = editor();
    e.clipboard_probe = true;
    write_feedback_clipboard(&mut e, "previous").unwrap();
    e.clipboard_error = Some("clipboard busy".into());
    assert!(write_feedback_clipboard(&mut e, "new text").is_err());
    assert_eq!(e.clipboard_written.as_deref(), Some("previous"));
    e.clipboard_error = None;
    write_feedback_clipboard(&mut e, "new text").unwrap();
    assert_eq!(e.clipboard_written.as_deref(), Some("new text"));
}
#[test]
fn copy_rejects_invalid_pending_text_without_discarding_it() {
    let mut e = editor();
    e.pending = Some((1, "字".repeat(20_001)));
    e.clipboard_probe = true;
    e.clipboard_written = Some("previous".into());
    assert!(feedback_payload(&mut e).is_err());
    assert_eq!(e.pending.as_ref().unwrap().1.chars().count(), 20_001);
    assert_eq!(e.clipboard_written.as_deref(), Some("previous"));
}
fn editor() -> Editor {
    let mut d = Document::new("test.png", 300, 200).unwrap();
    d.add_region(Rect {
        x: 10.2,
        y: 10.2,
        width: 40.2,
        height: 30.2,
    })
    .unwrap();
    Editor {
        history: Some(History::new(d)),
        ..Default::default()
    }
}
#[test]
fn rejected_text_remains_pending_and_dirty() {
    let mut e = editor();
    e.pending = Some((1, "字".repeat(20_001)));
    assert!(e.flush().is_err());
    assert_eq!(e.pending.as_ref().unwrap().1.chars().count(), 20_001);
    assert!(e.has_feedback());
    assert_eq!(e.doc().unwrap().annotation(1).unwrap().text, "");
}
#[test]
fn corrected_text_can_commit_after_rejection() {
    let mut e = editor();
    e.pending = Some((1, "x".repeat(20_001)));
    assert!(e.flush().is_err());
    e.pending = Some((1, "修正后的意见".into()));
    e.flush().unwrap();
    assert!(e.pending.is_none());
    assert_eq!(e.doc().unwrap().annotation(1).unwrap().text, "修正后的意见");
    assert!(e.has_feedback());
}
#[test]
fn ui_tokens_do_not_truncate_large_stable_ids() {
    let mut d = Document::new("x.png", 100, 100).unwrap();
    let id = d.add_global().unwrap();
    d.annotations[0].id = id + u32::MAX as u64;
    d.next_id = d.annotations[0].id + 1;
    d.validate().unwrap();
    let actual = d.annotations[0].id;
    let e = Editor {
        history: Some(History::new(d)),
        ..Default::default()
    };
    assert_eq!(ui_note_id(&e, 1), Some(actual));
    assert_eq!(ui_note_id(&e, 0), None);
    assert_eq!(ui_note_id(&e, -1), None);
    assert_eq!(ui_note_id(&e, 2), None);
}
#[test]
fn fractional_moves_remain_inside_at_both_edges() {
    for (dx, dy) in [(10000., 10000.), (-10000., -10000.), (79.6, 42.2)] {
        let mut e = editor();
        let (dx, dy) = e.bounded_delta(1, dx, dy);
        e.history
            .as_mut()
            .unwrap()
            .apply(|d| d.move_annotation(1, dx, dy))
            .unwrap();
        e.doc().unwrap().validate().unwrap();
    }
}
#[test]
fn clamping_handles_nonfinite_requested_motion() {
    assert_eq!(clamped_axis(10., 20., 100, f32::NAN), 0.);
    assert_eq!(clamped_axis(10., 20., 100, f32::INFINITY), 0.);
}
#[test]
fn unselected_transparent_region_does_not_steal_block_hit() {
    let mut d = Document::new("x.png", 300, 200).unwrap();
    let block = d
        .add_region(Rect {
            x: 10.,
            y: 10.,
            width: 80.,
            height: 80.,
        })
        .unwrap();
    d.move_annotation(block, 120., 0.).unwrap();
    let region = d
        .add_region(Rect {
            x: 100.,
            y: 0.,
            width: 150.,
            height: 150.,
        })
        .unwrap();
    let mut e = Editor {
        history: Some(History::new(d)),
        view: View {
            scale: 1.,
            ..Default::default()
        },
        ..Default::default()
    };
    let p = Point { x: 170., y: 60. };
    assert_eq!(e.hit(p), Some(block));
    e.selected = Some(region);
    assert_eq!(e.hit(p), Some(region));
}
#[test]
fn pins_are_clickable_outside_rect_interior() {
    let e = editor();
    assert_eq!(e.hit(Point { x: 6., y: 6. }), Some(1));
}
#[test]
fn recommendation_is_only_a_click_shortcut() {
    let start = Point { x: 50., y: 50. };
    let candidate = Rect {
        x: 20.,
        y: 20.,
        width: 80.,
        height: 60.,
    };
    assert_eq!(
        selection_result(start, start, 1., Some(candidate), false),
        Some(candidate)
    );
    assert_eq!(
        selection_result(start, start, 1., Some(candidate), true),
        None
    );
    assert_eq!(
        selection_result(start, Point { x: 51., y: 50. }, 1., Some(candidate), true),
        None
    );
    let result =
        selection_result(start, Point { x: 92., y: 85. }, 1., Some(candidate), true).unwrap();
    assert_eq!(
        result,
        Rect {
            x: 50.,
            y: 50.,
            width: 42.,
            height: 35.
        }
    );
}
#[test]
fn long_degenerate_drag_never_adopts_recommendation() {
    let start = Point { x: 50., y: 50. };
    let candidate = Rect {
        x: 20.,
        y: 20.,
        width: 80.,
        height: 60.,
    };
    assert!(
        selection_result(start, Point { x: 100., y: 50. }, 1., Some(candidate), false).is_none()
    );
}
#[test]
fn screen_threshold_is_independent_of_zoom() {
    let c = Rect {
        x: 0.,
        y: 0.,
        width: 100.,
        height: 100.,
    };
    let p = Point { x: 10., y: 10. };
    assert_eq!(
        selection_result(p, Point { x: 12., y: 12. }, 0.5, Some(c), false),
        Some(c)
    );
    assert_ne!(
        selection_result(p, Point { x: 12., y: 12. }, 4., Some(c), false),
        Some(c)
    );
}
#[test]
fn candidate_filter_excludes_sources_and_destinations() {
    fn c(rect: Rect, id: &str) -> Candidate {
        Candidate {
            id: id.into(),
            rect,
            score: 1.,
            kind: "control".into(),
            shape: None,
            shape_name: None,
            outline: None,
            role: None,
            line_count: None,
            font_height: None,
        }
    }
    let mut e = editor();
    e.history
        .as_mut()
        .unwrap()
        .apply(|d| d.move_annotation(1, 100., 0.))
        .unwrap();
    e.candidates = vec![
        c(
            Rect {
                x: 15.,
                y: 15.,
                width: 5.,
                height: 5.,
            },
            "source",
        ),
        c(
            Rect {
                x: 115.,
                y: 15.,
                width: 5.,
                height: 5.,
            },
            "destination",
        ),
        c(
            Rect {
                x: 220.,
                y: 100.,
                width: 30.,
                height: 30.,
            },
            "free",
        ),
    ];
    assert!(e.update_eligibility(true));
    assert_eq!(e.eligible.len(), 1);
    assert_eq!(e.eligible[0].id, "free");
    assert!(!e.update_eligibility(false));
}
#[test]
fn screen_to_original_mapping_is_zoom_invariant() {
    let e = Editor {
        view: View {
            x: 30.,
            y: 50.,
            scale: 0.75,
            ..Default::default()
        },
        ..Default::default()
    };
    let p = e.point(105., 200.);
    assert_eq!(p, Point { x: 100., y: 200. });
}
