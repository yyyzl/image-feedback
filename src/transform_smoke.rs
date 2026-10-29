//! Native Slint scene regression for transform interaction and delivery.
use super::*;

fn screen(state: &Rc<RefCell<Editor>>, p: Point) -> Point {
    let e = state.borrow();
    Point {
        x: e.view.x + p.x * e.view.scale,
        y: e.view.y + p.y * e.view.scale,
    }
}
fn drag(ui: &AppWindow, a: Point, b: Point) {
    canvas_press(ui, a.x, a.y, 0);
    canvas_move(ui, b.x, b.y);
    canvas_release(ui, b.x, b.y);
}
fn geometry(state: &Rc<RefCell<Editor>>, id: u64) -> Result<(Rect, Rect, f32), String> {
    match state
        .borrow()
        .doc()
        .and_then(|d| d.annotation(id))
        .map(|a| a.kind.clone())
    {
        Some(AnnotationKind::Block {
            source,
            target,
            rotation,
        }) => Ok((source, target, rotation)),
        _ => Err("transform smoke expected a block".into()),
    }
}
fn rotated_drag(
    ui: &AppWindow,
    state: &Rc<RefCell<Editor>>,
    target: Rect,
    start: Point,
    angle: f32,
) {
    let c = transform::center(target);
    let v = transform::rotate(
        Point {
            x: start.x - c.x,
            y: start.y - c.y,
        },
        angle,
    );
    drag(
        ui,
        screen(state, start),
        screen(
            state,
            Point {
                x: c.x + v.x,
                y: c.y + v.y,
            },
        ),
    );
}
pub(super) fn check_region(
    ui: &AppWindow,
    state: &Rc<RefCell<Editor>>,
    id: u64,
) -> Result<(), String> {
    let before = state.borrow().doc().unwrap().clone();
    let source = match before.annotation(id).unwrap().kind {
        AnnotationKind::Region { rect } => rect,
        _ => return Err("new selection is not a region".into()),
    };
    let corner = transform::corners(source, 0.)[2];
    let c = transform::center(source);
    drag(
        ui,
        screen(state, corner),
        screen(
            state,
            Point {
                x: c.x + (corner.x - c.x) * 1.2,
                y: c.y + (corner.y - c.y) * 1.2,
            },
        ),
    );
    let (actual_source, target, angle) = geometry(state, id)?;
    if actual_source != source || target.width <= source.width || angle != 0. {
        return Err("resizing a fresh region did not create a block".into());
    }
    let top = transform::rotation_handle(target, angle, state.borrow().view.scale);
    rotated_drag(ui, state, target, top, 15.);
    if geometry(state, id)?.2.abs() < 10. {
        return Err("rotation after fresh-region resize failed".into());
    }
    ui.invoke_undo();
    ui.invoke_undo();
    if state.borrow().doc() != Some(&before) {
        return Err("undo did not restore fresh region".into());
    }
    // A fresh region can also be rotated first, without a preceding move/resize.
    let top = transform::rotation_handle(source, 0., state.borrow().view.scale);
    ui.set_shift_held(true);
    rotated_drag(ui, state, source, top, 12.);
    ui.set_shift_held(false);
    let (actual_source, _, angle) = geometry(state, id)?;
    if actual_source != source || (angle - 15.).abs() > 0.1 {
        return Err("fresh-region rotation or Shift snapping failed".into());
    }
    ui.invoke_undo();
    if state.borrow().doc() != Some(&before) {
        return Err("undo after fresh-region rotation failed".into());
    }
    Ok(())
}

pub(super) fn check(
    ui: &AppWindow,
    state: &Rc<RefCell<Editor>>,
    folder: &Path,
    id: u64,
) -> Result<(), String> {
    let baseline = state.borrow().doc().unwrap().clone();
    let (source, original, angle) = geometry(state, id)?;
    let corner = transform::corners(original, angle)[2];
    let c = transform::center(original);
    let start = screen(state, corner);
    canvas_move(ui, start.x, start.y);
    if ![1, 2].contains(&ui.get_canvas_cursor()) {
        return Err("resize handle cursor missing".into());
    }
    let end = Point {
        x: c.x + (corner.x - c.x) * 1.3,
        y: c.y + (corner.y - c.y) * 1.3,
    };
    drag(ui, start, screen(state, end));
    let (_, resized, resized_angle) = geometry(state, id)?;
    if resized.width < original.width * 1.1 || resized_angle != angle {
        return Err("corner did not resize uniformly".into());
    }
    let anchor0 = transform::corners(original, angle)[0];
    let anchor1 = transform::corners(resized, angle)[0];
    if (anchor0.x - anchor1.x).hypot(anchor0.y - anchor1.y) > 0.1 {
        return Err("resize opposite anchor drifted".into());
    }
    let top = transform::rotation_handle(resized, resized_angle, state.borrow().view.scale);
    let point = screen(state, top);
    canvas_move(ui, point.x, point.y);
    if ui.get_canvas_cursor() != 3 {
        return Err("top rotation cursor missing".into());
    }
    rotated_drag(ui, state, resized, top, 22.);
    let (_, mut target, mut rotation) = geometry(state, id)?;
    if (rotation - resized_angle).abs() < 15. || target != resized {
        return Err("top rotation handle failed".into());
    }
    for (i, (sx, sy)) in [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)]
        .into_iter()
        .enumerate()
    {
        let corner = transform::corners(target, rotation)[i];
        let scale = state.borrow().view.scale;
        let offset = transform::rotate(
            Point {
                x: sx * 18. / scale,
                y: sy * 18. / scale,
            },
            rotation,
        );
        let start = Point {
            x: corner.x + offset.x,
            y: corner.y + offset.y,
        };
        let sp = screen(state, start);
        canvas_move(ui, sp.x, sp.y);
        if ui.get_canvas_cursor() != 3 {
            return Err(format!("corner {i} rotation cursor missing"));
        }
        rotated_drag(ui, state, target, start, 7.);
        let (_, next, next_angle) = geometry(state, id)?;
        if (transform::normalize_angle(next_angle - rotation) - 7.).abs() > 1. || target != next {
            return Err(format!("corner {i} rotation failed"));
        }
        target = next;
        rotation = next_angle;
    }
    // A rotated block must still resize about its opposite corner.
    let corners = transform::corners(target, rotation);
    let c = transform::center(target);
    let resized_end = Point {
        x: c.x + (corners[2].x - c.x) * 1.2,
        y: c.y + (corners[2].y - c.y) * 1.2,
    };
    drag(ui, screen(state, corners[2]), screen(state, resized_end));
    let (_, rotated_resize, resized_angle) = geometry(state, id)?;
    let anchor = transform::corners(rotated_resize, resized_angle)[0];
    if resized_angle != rotation
        || rotated_resize.width <= target.width
        || (anchor.x - corners[0].x).hypot(anchor.y - corners[0].y) > 0.1
    {
        return Err("resize after rotation failed or anchor drifted".into());
    }
    target = rotated_resize;
    let c = transform::center(target);
    let sp = screen(state, c);
    drag(
        ui,
        sp,
        Point {
            x: sp.x + 32.,
            y: sp.y + 16.,
        },
    );
    let (_, moved, moved_angle) = geometry(state, id)?;
    if moved.x <= target.x
        || moved.y <= target.y
        || moved_angle != rotation
        || moved.width != target.width
    {
        return Err("drag after resize/rotation changed the transform or failed".into());
    }
    save_window_snapshot(ui, &folder.join("transform.png"))?;
    let after = state.borrow().doc().unwrap().clone();
    let text = storage::feedback_text(&after);
    if !text.contains("等比缩放") || !text.contains("顺时针旋转") {
        return Err("feedback omitted transform".into());
    }
    {
        let e = state.borrow();
        let asset = e.asset.as_ref().unwrap();
        let output = delivery::prepare(&folder.join("transform-images"), &after, asset)?;
        if std::fs::read(&output.original).map_err(|e| e.to_string())? != asset.original {
            return Err("transform changed original image".into());
        }
        let reference = image::open(&output.reference)
            .map_err(|e| e.to_string())?
            .to_rgba8();
        if reference.dimensions() != asset.pixels.dimensions() || reference == asset.pixels {
            return Err("transformed reference invalid".into());
        }
    }
    ui.set_space_held(true);
    let p = screen(state, transform::center(moved));
    let old_view = state.borrow().view;
    drag(
        ui,
        p,
        Point {
            x: p.x + 25.,
            y: p.y + 15.,
        },
    );
    ui.set_space_held(false);
    if state.borrow().doc() != Some(&after) || (state.borrow().view.x - old_view.x).abs() < 10. {
        return Err("space pan interfered with block geometry".into());
    }
    ui.invoke_fit_view();
    ui.invoke_undo();
    if state.borrow().doc() == Some(&after) {
        return Err("transform move undo failed".into());
    }
    ui.invoke_redo();
    if state.borrow().doc() != Some(&after) {
        return Err("transform move redo failed".into());
    }
    for _ in 0..8 {
        ui.invoke_undo();
    }
    if state.borrow().doc() != Some(&baseline) {
        return Err("transform steps were not individually undoable".into());
    }
    // Move partially beyond the image and still reach the control in the viewport.
    let (_, edge_target, edge_angle) = geometry(state, id)?;
    let c = transform::center(edge_target);
    let p = screen(state, c);
    let offset = -edge_target.x - 15.;
    drag(
        ui,
        p,
        screen(
            state,
            Point {
                x: c.x + offset,
                y: c.y,
            },
        ),
    );
    let (_, outside, a) = geometry(state, id)?;
    if outside.x >= 0. || a != edge_angle {
        return Err("partial off-image move failed".into());
    }
    let corner = screen(state, transform::corners(outside, a)[0]);
    canvas_move(ui, corner.x, corner.y);
    if ![1, 2].contains(&ui.get_canvas_cursor()) {
        return Err("off-image corner handle is unreachable".into());
    }
    let c = transform::center(outside);
    let p = screen(state, c);
    drag(
        ui,
        p,
        Point {
            x: p.x + 20.,
            y: p.y,
        },
    );
    if geometry(state, id)?.1.x <= outside.x {
        return Err("partially off-image block cannot be moved again".into());
    }
    ui.invoke_undo();
    ui.invoke_undo();
    if state.borrow().doc() != Some(&baseline) {
        return Err("off-image undo failed".into());
    }
    // Keep the rotation button reachable when the top edge approaches the viewport.
    {
        let mut e = state.borrow_mut();
        let top = e.view.y + edge_target.y * e.view.scale;
        e.pan_view(0., 20. - top);
        sync(ui, &mut e);
    }
    let handle = state.borrow().rotation_handle(edge_target, edge_angle);
    let hs = screen(state, handle);
    if hs.y < 10. || hs.y > 15. {
        return Err("edge rotation handle was not kept inside viewport".into());
    }
    canvas_move(ui, hs.x, hs.y);
    if ui.get_canvas_cursor() != 3 {
        return Err("edge rotation handle cannot be hit".into());
    }
    rotated_drag(ui, state, edge_target, handle, 12.);
    if geometry(state, id)?.2.abs() < 8. {
        return Err("edge rotation handle did not rotate".into());
    }
    ui.invoke_undo();
    ui.invoke_fit_view();
    if state.borrow().doc() != Some(&baseline) {
        return Err("edge handle undo failed".into());
    }
    if source != geometry(state, id)?.0 {
        return Err("source changed during transformations".into());
    }
    Ok(())
}
