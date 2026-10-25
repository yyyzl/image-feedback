//! Native parity/contract tests. No Node/JS subprocess is invoked by these tests.
//! Include the module directly until the host opts to export it from its library.
use image_feedback_slint::document;
#[path = "../src/smart_selection.rs"]
mod smart_selection;
use document::{Point, Rect};
use serde::Deserialize;
use smart_selection::{Candidate, detect, detect_with_cancel, pick};
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Goldens {
    schema: String,
    reference_commit: String,
    cases: Vec<Fixture>,
}
#[derive(Deserialize)]
struct Fixture {
    name: String,
    file: String,
    width: u32,
    height: u32,
    scale: f32,
    candidates: Vec<Candidate>,
    probes: Vec<Probe>,
}
#[derive(Deserialize)]
struct Probe {
    point: Point,
    previous: Option<String>,
    expected: Option<String>,
}
fn goldens() -> Goldens {
    serde_json::from_str(include_str!("fixtures/smart-goldens.json")).unwrap()
}
fn pixels(f: &Fixture) -> Vec<u8> {
    // file! is valid in both cargo and standalone rustc runs from the root.
    let root = PathBuf::from(file!()).parent().unwrap().join("fixtures");
    let image = image::open(root.join(&f.file)).unwrap().into_rgba8();
    assert_eq!(image.dimensions(), (f.width, f.height));
    image.into_raw()
}
fn fixture(name: &str) -> Fixture {
    goldens()
        .cases
        .into_iter()
        .find(|f| f.name == name)
        .unwrap()
}
fn analyze(name: &str) -> Vec<Candidate> {
    let f = fixture(name);
    detect(f.width, f.height, &pixels(&f), f.scale).unwrap()
}
fn p(x: f32, y: f32) -> Point {
    Point { x, y }
}
fn r(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}
fn candidate(id: &str, rect: Rect, score: f32, kind: &str) -> Candidate {
    Candidate {
        id: id.into(),
        rect,
        score,
        kind: kind.into(),
        shape: None,
        shape_name: None,
        outline: None,
        role: None,
        line_count: None,
        font_height: None,
    }
}
fn picked<'a>(c: &'a [Candidate], point: Point, previous: Option<&str>) -> Option<&'a Candidate> {
    pick(c, point, previous).map(|i| &c[i])
}
fn same(c: &[Candidate], points: &[Point], kind: &str) -> Candidate {
    let first = picked(c, points[0], None).unwrap();
    assert_eq!(first.kind, kind);
    for &point in points {
        assert_eq!(
            picked(c, point, None).map(|c| c.id.as_str()),
            Some(first.id.as_str()),
            "point={point:?}"
        );
    }
    first.clone()
}
fn equal_candidate(a: &Candidate, b: &Candidate) -> bool {
    a.id == b.id
        && a.rect == b.rect
        && (a.score - b.score).abs() < 0.000_002
        && a.kind == b.kind
        && a.shape == b.shape
        && a.shape_name == b.shape_name
        && a.outline == b.outline
        && a.role == b.role
        && a.line_count == b.line_count
        && a.font_height == b.font_height
}

#[test]
fn fixture_goldens_match_all_candidates_metadata_and_picks() {
    let golden = goldens();
    assert_eq!(golden.schema, "smart-selection-parity-1");
    assert_eq!(golden.reference_commit, "6f256a6");
    assert_eq!(golden.cases.len(), 65);
    let mut errors = Vec::new();
    let mut candidate_count = 0;
    let mut probe_count = 0;
    for f in golden.cases {
        let native = detect(f.width, f.height, &pixels(&f), f.scale).unwrap();
        candidate_count += f.candidates.len();
        probe_count += f.probes.len();
        if native.len() != f.candidates.len() {
            errors.push(format!(
                "{} count native={} JS={}",
                f.name,
                native.len(),
                f.candidates.len()
            ));
        }
        for (i, (actual, expected)) in native.iter().zip(&f.candidates).enumerate() {
            if !equal_candidate(actual, expected) {
                errors.push(format!(
                    "{} #{i}\nnative={actual:?}\nJS={expected:?}",
                    f.name
                ));
            }
        }
        for probe in f.probes {
            let actual =
                picked(&native, probe.point, probe.previous.as_deref()).map(|c| c.id.clone());
            if actual != probe.expected {
                errors.push(format!(
                    "{} point={:?} previous={:?}: native={actual:?}, JS={:?}",
                    f.name, probe.point, probe.previous, probe.expected
                ));
            }
        }
    }
    assert_eq!((candidate_count, probe_count), (908, 1868));
    assert!(
        errors.is_empty(),
        "{} mismatches:\n{}",
        errors.len(),
        errors.into_iter().take(30).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn cancellation_is_cooperative_latched_and_never_returns_partial_results() {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};
    let f = fixture("layout-light-1");
    let rgba = pixels(&f);
    let flag = AtomicBool::new(true);
    assert_eq!(
        detect_with_cancel(f.width, f.height, &rgba, f.scale, || flag
            .load(Ordering::Relaxed))
        .unwrap_err(),
        "cancelled"
    );
    flag.store(false, Ordering::Relaxed);
    let calls = Cell::new(0usize);
    let full = detect_with_cancel(f.width, f.height, &rgba, f.scale, || {
        calls.set(calls.get() + 1);
        flag.load(Ordering::Relaxed)
    })
    .unwrap();
    assert_eq!(
        serde_json::to_value(&full).unwrap(),
        serde_json::to_value(detect(f.width, f.height, &rgba, f.scale).unwrap()).unwrap()
    );
    let total = calls.get();
    assert!(total > 100);
    // Exercise early scan, traversal and late grouping/finalization cancellation.
    for stop in [1, 10, total / 16, total / 4, total / 2, total - 2] {
        calls.set(0);
        let result = detect_with_cancel(f.width, f.height, &rgba, f.scale, || {
            calls.set(calls.get() + 1);
            calls.get() == stop
        });
        assert_eq!(
            result.unwrap_err(),
            "cancelled",
            "stop={stop}, total={total}"
        );
        assert_eq!(calls.get(), stop, "a true callback must latch");
    }
    assert!(
        detect_with_cancel(1, 1, &[], 1.0, || true)
            .unwrap_err()
            .contains("budget")
    );
}

#[test]
fn dimensions_byte_lengths_and_scale_are_validated_before_allocation() {
    for (w, h) in [
        (0, 80),
        (7, 80),
        (80, 7),
        (1601, 80),
        (80, 1601),
        (1600, 1600),
        (u32::MAX, u32::MAX),
    ] {
        assert!(detect(w, h, &[], 1.0).is_err());
    }
    for len in [0, 255, 257] {
        assert!(detect(8, 8, &vec![255; len], 1.0).is_err());
    }
    for scale in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        -1.0,
        0.0,
        0.249,
        4.001,
    ] {
        assert!(detect(8, 8, &[255; 256], scale).is_err());
    }
    for scale in [0.25, 1.0, 4.0] {
        assert!(detect(8, 8, &[255; 256], scale).unwrap().is_empty());
    }
}
#[test]
fn valid_axis_and_exact_pixel_budget_boundaries_abstain_on_blank() {
    for (w, h) in [(8, 8), (1600, 8), (8, 1600), (1600, 1250)] {
        assert!(
            detect(w, h, &vec![255; w as usize * h as usize * 4], 1.0)
                .unwrap()
                .is_empty()
        );
    }
    assert!(detect(1600, 1251, &[], 1.0).is_err());
}
#[test]
fn empty_transparent_noise_and_smooth_paint_abstain() {
    for name in [
        "blank-light",
        "blank-dark",
        "hidden-rgb",
        "noise",
        "smooth-photo",
    ] {
        assert!(analyze(name).is_empty(), "{name}");
    }
    let mut hidden = vec![0u8; 64 * 64 * 4];
    let mut state = 123u32;
    for p in hidden.chunks_exact_mut(4) {
        for channel in &mut p[..3] {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *channel = (state >> 24) as u8;
        }
    }
    assert!(detect(64, 64, &hidden, 1.0).unwrap().is_empty());
}
#[test]
fn source_is_immutable_results_are_deterministic_and_json_safe() {
    let f = fixture("layout-light-1");
    let rgba = pixels(&f);
    let before = rgba.clone();
    let first = detect(f.width, f.height, &rgba, f.scale).unwrap();
    let second = detect(f.width, f.height, &rgba, f.scale).unwrap();
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&second).unwrap()
    );
    assert_eq!(rgba, before);
    let serialized = serde_json::to_string(&first).unwrap();
    let roundtrip: Vec<Candidate> = serde_json::from_str(&serialized).unwrap();
    assert_eq!(serialized, serde_json::to_string(&roundtrip).unwrap());
    let _ = pick(
        &first,
        p(100.0, 140.0),
        first.first().map(|c| c.id.as_str()),
    );
    assert_eq!(serialized, serde_json::to_string(&first).unwrap());
}
#[test]
fn optional_fields_use_camel_case_and_omit_none() {
    let c = candidate("a", r(0.0, 0.0, 10.0, 10.0), 0.9, "control");
    let value = serde_json::to_value(&c).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 4);
    let mut c = c;
    c.shape_name = Some("diamond".into());
    c.line_count = Some(2);
    c.font_height = Some(16.0);
    let value = serde_json::to_value(&c).unwrap();
    assert_eq!(value["shapeName"], "diamond");
    assert_eq!(value["lineCount"], 2);
    assert_eq!(value["fontHeight"], 16.0);
}
#[test]
fn light_dark_mixed_text_paragraphs_rows_and_whitespace_are_whole_targets() {
    for dark in ["light", "dark"] {
        for factor in [1.0f32, 0.7] {
            let c = analyze(&format!("layout-{dark}-{factor}"));
            let scaled = |x, y| p(x * factor, y * factor);
            let selected = same(
                &c,
                &[
                    scaled(90.0, 140.0),
                    scaled(215.0, 140.0),
                    scaled(245.0, 140.0),
                ],
                "row",
            );
            let plain = same(
                &c,
                &[
                    scaled(90.0, 284.0),
                    scaled(215.0, 284.0),
                    scaled(245.0, 284.0),
                ],
                "row",
            );
            assert_ne!(selected.id, plain.id);
            for row in [selected, plain] {
                assert!(row.rect.width / factor >= 225.0 && row.rect.width / factor <= 280.0);
                assert!(row.rect.height / factor >= 34.0 && row.rect.height / factor <= 55.0);
            }
            let first = same(
                &c,
                &[
                    scaled(430.0, 112.0),
                    scaled(430.0, 140.0),
                    scaled(1010.0, 140.0),
                ],
                "paragraph",
            );
            let second = same(
                &c,
                &[
                    scaled(430.0, 232.0),
                    scaled(390.0, 260.0),
                    scaled(1010.0, 260.0),
                ],
                "paragraph",
            );
            assert_ne!(first.id, second.id);
            for paragraph in [first, second] {
                assert_eq!(paragraph.role.as_deref(), Some("paragraph"));
                assert!(paragraph.line_count.unwrap() >= 2);
                assert!(paragraph.rect.width / factor > 720.0);
            }
            assert!(pick(&c, scaled(1000.0, 190.0), None).is_none());
            assert!(!c.iter().any(|c| c.kind == "paragraph"
                && c.rect.x < 290.0 * factor
                && c.rect.x + c.rect.width > 350.0 * factor));
        }
    }
}
#[test]
fn paragraph_child_button_wins_even_with_parent_hysteresis() {
    let c = analyze("child-button");
    let paragraph = same(&c, &[p(430.0, 112.0), p(1010.0, 140.0)], "paragraph");
    let button = picked(&c, p(930.0, 142.0), Some(&paragraph.id)).unwrap();
    assert_eq!(button.kind, "control");
    assert_eq!(button.rect, r(880.0, 128.0, 100.0, 28.0));
}
#[test]
fn standalone_lines_do_not_emit_individual_glyphs() {
    for dark in [false, true] {
        let c = analyze(&format!("line-{dark}"));
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, "text");
        assert!(pick(&c, p(80.0, 58.0), None).is_some());
        assert!(pick(&c, p(20.0, 20.0), None).is_none());
    }
}
#[test]
fn closed_rectangles_rounded_cards_and_nested_controls_remain_targets() {
    for dark in [false, true] {
        let c = analyze(&format!("rounded-{dark}"));
        let button = picked(&c, p(95.0, 84.0), None).unwrap();
        assert_eq!(button.rect, r(57.0, 63.0, 146.0, 42.0));
        assert!(button.shape.is_none());
        assert!(!c.iter().any(|c| c.kind == "text"));
        let c = analyze(&format!("thin-card-{dark}"));
        assert!(
            c.iter()
                .any(|c| (c.rect.x - 49.0).abs() <= 2.0 && (c.rect.width - 278.0).abs() <= 2.0)
        );
    }
    let c = analyze("nested");
    let panel = picked(&c, p(389.0, 290.0), None).unwrap();
    let child = picked(&c, p(286.0, 255.0), Some(&panel.id)).unwrap();
    assert_eq!(child.rect, r(221.0, 234.0, 145.0, 44.0));
}
#[test]
fn common_shapes_fill_stroke_and_dark_have_real_contour_hits() {
    for shape in [
        "circle",
        "ellipse",
        "capsule",
        "diamond",
        "triangle",
        "hexagon",
        "trapezoid",
        "parallelogram",
    ] {
        for theme in ["light", "dark"] {
            for paint in ["fill", "stroke"] {
                let c = analyze(&format!("{shape}-{theme}-{paint}"));
                let expected = if shape == "circle" {
                    "ellipse"
                } else if ["ellipse", "capsule"].contains(&shape) {
                    shape
                } else {
                    "polygon"
                };
                let found = c
                    .iter()
                    .find(|c| {
                        c.shape.as_deref() == Some(expected)
                            && (expected != "polygon" || c.shape_name.as_deref() == Some(shape))
                    })
                    .unwrap();
                let r = found.rect;
                assert_eq!(
                    picked(&c, p(r.x + r.width / 2.0, r.y + r.height / 2.0), None)
                        .unwrap()
                        .id,
                    found.id
                );
                assert!(
                    pick(
                        std::slice::from_ref(found),
                        p(r.x + 0.5, r.y + 0.5),
                        Some(&found.id)
                    )
                    .is_none()
                );
            }
        }
    }
}
#[test]
fn template_orientations_textured_avatars_and_spur_refinement_work() {
    for i in 0..9 {
        assert!(
            analyze(&format!("orientation-{i}"))
                .iter()
                .any(|c| c.shape.is_some())
        );
    }
    for dark in [false, true] {
        let c = analyze(&format!("avatar-{dark}"));
        assert_eq!(
            picked(&c, p(130.0, 86.0), None).unwrap().shape.as_deref(),
            Some("ellipse")
        );
    }
    let c = analyze("acute-spurs");
    let triangle = picked(&c, p(114.0, 90.0), None).unwrap();
    assert_eq!(triangle.shape_name.as_deref(), Some("triangle"));
    assert!((triangle.rect.x - 70.0).abs() <= 3.0);
}
#[test]
fn open_boundaries_do_not_become_closed_shapes() {
    for name in ["open-arc", "open-box"] {
        assert!(!analyze(name).iter().any(|c| c.shape.is_some()));
    }
}
#[test]
fn dense_tiling_has_exact_cap_and_layout_reserves_capacity() {
    let c = analyze("candidate-cap");
    assert_eq!(c.len(), 400);
    let c = analyze("dense-layout");
    assert!(c.len() <= 400);
    same(&c, &[p(180.0, 657.0), p(760.0, 685.0)], "paragraph");
}
#[test]
fn picker_uses_stable_ties_exclusive_bounds_and_limited_hysteresis() {
    let a = candidate("a", r(10.0, 20.0, 100.0, 40.0), 0.9, "control");
    let b = candidate("b", r(13.0, 20.0, 100.0, 40.0), 0.94, "control");
    let c = vec![a.clone(), b];
    assert_eq!(pick(&c, p(50.0, 40.0), None), Some(1));
    assert_eq!(pick(&c, p(50.0, 40.0), Some("a")), Some(0));
    assert_eq!(pick(&c, p(111.0, 40.0), Some("a")), Some(1));
    assert_eq!(pick(&c, p(113.0, 40.0), Some("b")), None);
    assert_eq!(pick(&c, p(50.0, 60.0), None), None);
    assert_eq!(pick(&c, p(50.0, 40.0), Some("stale")), Some(1));
    let z = candidate("z", a.rect, a.score, "control");
    assert_eq!(pick(&[z.clone(), a.clone()], p(50.0, 40.0), None), Some(1));
    assert_eq!(pick(&[a, z], p(50.0, 40.0), None), Some(0));
    let mut c = c;
    c[1].score = 1.0;
    assert_eq!(pick(&c, p(50.0, 40.0), Some("a")), Some(1));
}
#[test]
fn picker_rejects_nonfinite_invalid_geometry_and_untrusted_shape_metadata() {
    let base = candidate("shape", r(20.0, 20.0, 80.0, 60.0), 0.95, "control");
    for point in [
        p(f32::NAN, 1.0),
        p(f32::INFINITY, 2.0),
        p(-1.0, 20.0),
        p(20.0, -1.0),
    ] {
        assert!(pick(std::slice::from_ref(&base), point, Some("shape")).is_none());
    }
    for rect in [
        r(-1.0, 0.0, 20.0, 20.0),
        r(0.0, 0.0, 3.0, 20.0),
        r(0.0, 0.0, f32::INFINITY, 20.0),
        r(f32::NAN, 0.0, 20.0, 20.0),
        r(0.0, 0.0, f32::MAX, f32::MAX),
    ] {
        let c = candidate("bad", rect, 0.9, "control");
        assert!(pick(&[c], p(1.0, 1.0), None).is_none());
    }
    let mut invalid = base.clone();
    invalid.score = f32::NAN;
    assert!(pick(&[invalid], p(60.0, 50.0), None).is_none());
    for outline in [
        None,
        Some(vec![[0.0, 0.0], [1.0, 0.0]]),
        Some(vec![[0.0, 0.0], [1.0, 0.0], [f32::NAN, 1.0]]),
        Some(vec![[0.0, 0.0], [1.0, 0.0], [2.0, 1.0]]),
        Some(vec![[0.0, 0.0]; 3]),
        Some(vec![[0.0, 0.0]; 9]),
    ] {
        let mut c = base.clone();
        c.shape = Some("polygon".into());
        c.outline = outline;
        assert!(pick(&[c], p(60.0, 50.0), None).is_none());
    }
    let mut c = base;
    c.shape = Some("unknown".into());
    assert!(pick(&[c], p(60.0, 50.0), None).is_none());
}
#[test]
fn picker_checks_exact_ellipse_capsule_and_polygon_contours() {
    for shape in ["ellipse", "capsule", "polygon"] {
        let mut c = candidate("shape", r(20.0, 20.0, 80.0, 60.0), 0.95, "control");
        c.shape = Some(shape.into());
        if shape == "polygon" {
            c.shape_name = Some("diamond".into());
            c.outline = Some(vec![[0.5, 0.0], [1.0, 0.5], [0.5, 1.0], [0.0, 0.5]]);
        }
        assert_eq!(pick(std::slice::from_ref(&c), p(60.0, 50.0), None), Some(0));
        assert!(pick(std::slice::from_ref(&c), p(21.0, 21.0), Some("shape")).is_none());
        assert!(pick(std::slice::from_ref(&c), p(100.0, 50.0), Some("shape")).is_none());
        if shape == "polygon" {
            assert_eq!(pick(std::slice::from_ref(&c), p(20.0, 50.0), None), Some(0));
            c.shape_name = Some("bogus".into());
            assert!(pick(&[c], p(60.0, 50.0), None).is_none());
        }
    }
}
#[test]
fn picker_never_scans_beyond_400_items() {
    let mut candidates =
        vec![candidate("offscreen", r(100.0, 100.0, 10.0, 10.0), 0.9, "control"); 400];
    candidates.push(candidate(
        "ignored",
        r(0.0, 0.0, 10.0, 10.0),
        1.0,
        "control",
    ));
    assert!(pick(&candidates, p(5.0, 5.0), Some("ignored")).is_none());
}
#[test]
fn independent_inputs_not_present_in_goldens_are_detected() {
    // An unrelated raster guards against a fixture-ID/golden lookup substitute.
    let (w, h) = (311usize, 173usize);
    let mut rgba = vec![255; w * h * 4];
    for y in 39..90 {
        for x in 71..238 {
            let at = (y * w + x) * 4;
            rgba[at..at + 4].copy_from_slice(&[62, 124, 189, 255]);
        }
    }
    let c = detect(w as u32, h as u32, &rgba, 1.0).unwrap();
    let hit = picked(&c, p(100.0, 50.0), None).unwrap();
    assert_eq!(hit.rect, r(71.0, 39.0, 167.0, 51.0));
    // Alpha compositing should produce the same boundary on a translucent fill.
    for y in 39..90 {
        for x in 71..238 {
            rgba[(y * w + x) * 4 + 3] = 128;
        }
    }
    assert_eq!(
        picked(
            &detect(w as u32, h as u32, &rgba, 1.0).unwrap(),
            p(100.0, 50.0),
            None
        )
        .unwrap()
        .rect,
        hit.rect
    );
}
