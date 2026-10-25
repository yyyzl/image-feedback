//! Pure document/geometry/history core. Image pixels deliberately live elsewhere.
//!
//! Coordinates are in original-image pixels. Rectangle interiors must have
//! positive area; touching source edges do not count as overlap. Out-of-bounds
//! moves are rejected, not clamped. UI display numbers are vector indices + 1,
//! never the stable annotation IDs stored here.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const DOCUMENT_SCHEMA: &str = "image-feedback-1";
pub const HISTORY_LIMIT: usize = 100;
pub const MAX_ANNOTATIONS: usize = 1_000;
pub const MAX_TEXT_CHARS: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnnotationKind {
    Point { point: Point },
    Region { rect: Rect },
    Block { source: Rect, target: Rect },
    Global,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub id: u64,
    pub text: String,
    pub kind: AnnotationKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageInfo {
    pub file_name: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub schema: String,
    pub image: ImageInfo,
    pub annotations: Vec<Annotation>,
    pub next_id: u64,
}

impl Document {
    pub fn new(name: impl Into<String>, width: u32, height: u32) -> Result<Self, String> {
        let document = Self {
            schema: DOCUMENT_SCHEMA.to_owned(),
            image: ImageInfo {
                file_name: name.into(),
                width,
                height,
            },
            annotations: Vec::new(),
            next_id: 1,
        };
        document.validate()?;
        Ok(document)
    }

    /// Validate explicitly after deserializing or directly changing public fields.
    /// Serde itself only checks the JSON's representation, not these invariants.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != DOCUMENT_SCHEMA {
            return Err(format!("unsupported document schema: {}", self.schema));
        }
        if self.image.width == 0 || self.image.height == 0 {
            return Err("image dimensions must be positive".to_owned());
        }
        if self.next_id == 0 {
            return Err("next_id must be positive".to_owned());
        }
        if self.annotations.len() > MAX_ANNOTATIONS {
            return Err(format!("at most {MAX_ANNOTATIONS} annotations are allowed"));
        }

        let mut ids = HashSet::with_capacity(self.annotations.len());
        let mut sources: Vec<&Rect> = Vec::new();
        for annotation in &self.annotations {
            validate_text(&annotation.text)?;
            if annotation.id == 0 || !ids.insert(annotation.id) {
                return Err(format!(
                    "invalid or duplicate annotation ID: {}",
                    annotation.id
                ));
            }
            if annotation.id >= self.next_id {
                return Err("next_id must exceed every annotation ID".to_owned());
            }
            match &annotation.kind {
                AnnotationKind::Point { point } => self.validate_point(*point)?,
                AnnotationKind::Region { rect } => self.validate_rect(*rect)?,
                AnnotationKind::Block { source, target } => {
                    self.validate_rect(*source)?;
                    self.validate_rect(*target)?;
                    if source.width != target.width || source.height != target.height {
                        return Err("a block target must keep its source dimensions".to_owned());
                    }
                    if sources.iter().any(|other| rects_overlap(**other, *source)) {
                        return Err("block source rectangles must not overlap".to_owned());
                    }
                    sources.push(source);
                }
                AnnotationKind::Global => {}
            }
        }
        Ok(())
    }

    pub fn add_region(&mut self, rect: Rect) -> Result<u64, String> {
        self.validate()?;
        self.validate_rect(rect)?;
        self.insert(AnnotationKind::Region { rect })
    }

    pub fn add_point(&mut self, point: Point) -> Result<u64, String> {
        self.validate()?;
        self.validate_point(point)?;
        self.insert(AnnotationKind::Point { point })
    }

    pub fn add_global(&mut self) -> Result<u64, String> {
        self.validate()?;
        self.insert(AnnotationKind::Global)
    }

    /// A region becomes a block only if its representable position changes.
    /// Existing blocks always move their target, never their original source.
    pub fn move_annotation(&mut self, id: u64, dx: f32, dy: f32) -> Result<(), String> {
        self.validate()?;
        if !dx.is_finite() || !dy.is_finite() {
            return Err("movement must be finite".to_owned());
        }
        let index = self
            .annotations
            .iter()
            .position(|annotation| annotation.id == id)
            .ok_or_else(|| format!("annotation not found: {id}"))?;

        // Build and validate the complete replacement before touching the document.
        let replacement = match self.annotations[index].kind.clone() {
            AnnotationKind::Point { point } => {
                let moved = Point {
                    x: point.x + dx,
                    y: point.y + dy,
                };
                self.validate_point(moved)?;
                AnnotationKind::Point { point: moved }
            }
            AnnotationKind::Region { rect } => {
                let target = translated(rect, dx, dy);
                self.validate_rect(target)?;
                if target == rect {
                    return Ok(());
                }
                if self.annotations.iter().any(|annotation| {
                    matches!(
                        &annotation.kind,
                        AnnotationKind::Block { source, .. }
                            if rects_overlap(*source, rect)
                    )
                }) {
                    return Err("region overlaps an existing block source".to_owned());
                }
                AnnotationKind::Block {
                    source: rect,
                    target,
                }
            }
            AnnotationKind::Block { source, target } => {
                let target = translated(target, dx, dy);
                self.validate_rect(target)?;
                AnnotationKind::Block { source, target }
            }
            AnnotationKind::Global => {
                return Err("global annotations cannot be moved".to_owned());
            }
        };
        self.annotations[index].kind = replacement;
        Ok(())
    }

    pub fn set_text(&mut self, id: u64, text: String) -> Result<(), String> {
        self.validate()?;
        validate_text(&text)?;
        let annotation = self
            .annotations
            .iter_mut()
            .find(|annotation| annotation.id == id)
            .ok_or_else(|| format!("annotation not found: {id}"))?;
        annotation.text = text;
        Ok(())
    }

    pub fn remove(&mut self, id: u64) -> Result<(), String> {
        self.validate()?;
        let index = self
            .annotations
            .iter()
            .position(|annotation| annotation.id == id)
            .ok_or_else(|| format!("annotation not found: {id}"))?;
        self.annotations.remove(index);
        // Deliberately do not decrement next_id: deleting does not recycle IDs.
        Ok(())
    }

    pub fn annotation(&self, id: u64) -> Option<&Annotation> {
        self.annotations
            .iter()
            .find(|annotation| annotation.id == id)
    }

    fn insert(&mut self, kind: AnnotationKind) -> Result<u64, String> {
        if self.annotations.len() >= MAX_ANNOTATIONS {
            return Err(format!("at most {MAX_ANNOTATIONS} annotations are allowed"));
        }
        let id = self.next_id;
        let next_id = id
            .checked_add(1)
            .ok_or_else(|| "annotation ID space exhausted".to_owned())?;
        self.annotations.push(Annotation {
            id,
            text: String::new(),
            kind,
        });
        self.next_id = next_id;
        Ok(id)
    }

    fn validate_point(&self, point: Point) -> Result<(), String> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err("point coordinates must be finite".to_owned());
        }
        if point.x < 0.0
            || point.y < 0.0
            || f64::from(point.x) > f64::from(self.image.width)
            || f64::from(point.y) > f64::from(self.image.height)
        {
            return Err("point must lie inside the image bounds".to_owned());
        }
        Ok(())
    }

    fn validate_rect(&self, rect: Rect) -> Result<(), String> {
        if !rect.x.is_finite()
            || !rect.y.is_finite()
            || !rect.width.is_finite()
            || !rect.height.is_finite()
        {
            return Err("rectangle coordinates and dimensions must be finite".to_owned());
        }
        if rect.x < 0.0 || rect.y < 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
            return Err("rectangle needs a nonnegative origin and positive dimensions".to_owned());
        }
        // Use f64 sums so f32 rounding cannot conceal a rectangle exceeding an
        // edge, and u32 image dimensions are not rounded down/up through f32.
        if right(rect) > f64::from(self.image.width) || bottom(rect) > f64::from(self.image.height)
        {
            return Err("rectangle must lie inside the image bounds".to_owned());
        }
        Ok(())
    }
}

fn validate_text(text: &str) -> Result<(), String> {
    // Count Unicode scalar values, not UTF-8 bytes; stop after the first excess.
    if text.chars().take(MAX_TEXT_CHARS + 1).count() > MAX_TEXT_CHARS {
        return Err(format!(
            "annotation text exceeds {MAX_TEXT_CHARS} characters"
        ));
    }
    Ok(())
}

fn translated(rect: Rect, dx: f32, dy: f32) -> Rect {
    Rect {
        x: rect.x + dx,
        y: rect.y + dy,
        ..rect
    }
}

fn right(rect: Rect) -> f64 {
    f64::from(rect.x) + f64::from(rect.width)
}

fn bottom(rect: Rect) -> f64 {
    f64::from(rect.y) + f64::from(rect.height)
}

fn rects_overlap(a: Rect, b: Rect) -> bool {
    f64::from(a.x) < right(b)
        && f64::from(b.x) < right(a)
        && f64::from(a.y) < bottom(b)
        && f64::from(b.y) < bottom(a)
}

/// Transactional, pixel-free document snapshots with up to 100 undo steps.
///
/// A private ID high-water mark prevents an ID abandoned by undo from being
/// reused by a new edit. Undo/redo still restore the exact stored document;
/// the high-water mark is applied only when executing the next transaction.
#[derive(Clone, Debug)]
pub struct History {
    document: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
    next_id_floor: u64,
}

impl History {
    /// Construction is infallible by contract; callers should validate imported
    /// documents first. Transactions will reject an invalid current document.
    pub fn new(document: Document) -> Self {
        let next_id_floor = document.next_id;
        Self {
            document,
            undo: Vec::new(),
            redo: Vec::new(),
            next_id_floor,
        }
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    /// Run edits on an isolated copy. Both callback errors and post-edit
    /// validation errors leave the document and both history stacks untouched.
    /// Panicking callbacks likewise cannot modify the original document.
    pub fn apply<T>(
        &mut self,
        edit: impl FnOnce(&mut Document) -> Result<T, String>,
    ) -> Result<T, String> {
        self.document.validate()?;
        let mut candidate = self.document.clone();
        candidate.next_id = candidate.next_id.max(self.next_id_floor);
        let baseline = candidate.clone();
        let result = edit(&mut candidate)?;
        candidate.validate()?;
        if candidate == baseline {
            return Ok(result);
        }
        if candidate.next_id < baseline.next_id {
            return Err("transactions cannot decrease next_id".to_owned());
        }
        // IDs introduced by a callback must be fresh even if it bypasses the
        // convenience methods and edits the public annotation vector directly.
        for annotation in &candidate.annotations {
            if self.document.annotation(annotation.id).is_none()
                && annotation.id < self.next_id_floor
            {
                return Err("transactions cannot reuse an old annotation ID".to_owned());
            }
        }

        self.next_id_floor = self.next_id_floor.max(candidate.next_id);
        let previous = std::mem::replace(&mut self.document, candidate);
        self.undo.push(previous);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        Ok(result)
    }

    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(previous) => {
                let current = std::mem::replace(&mut self.document, previous);
                self.redo.push(current);
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                let current = std::mem::replace(&mut self.document, next);
                self.undo.push(current);
                true
            }
            None => false,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Replace the document as a new editing session, resetting history and IDs'
    /// session high-water mark. Validate imports before calling this method.
    pub fn replace(&mut self, document: Document) {
        *self = Self::new(document);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn document() -> Document {
        Document::new("original.png", 100, 80).unwrap()
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn block(document: &Document, id: u64) -> (Rect, Rect) {
        match document.annotation(id).unwrap().kind {
            AnnotationKind::Block { source, target } => (source, target),
            _ => panic!("expected block"),
        }
    }

    #[test]
    fn new_sets_schema_image_and_initial_id() {
        let doc = document();
        assert_eq!(doc.schema, DOCUMENT_SCHEMA);
        assert_eq!(doc.image.file_name, "original.png");
        assert_eq!((doc.image.width, doc.image.height), (100, 80));
        assert!(doc.annotations.is_empty());
        assert_eq!(doc.next_id, 1);
        assert!(doc.validate().is_ok());
    }

    #[test]
    fn zero_image_dimensions_are_invalid() {
        assert!(Document::new("x", 0, 80).is_err());
        assert!(Document::new("x", 100, 0).is_err());
        assert!(Document::new("x", 0, 0).is_err());
    }

    #[test]
    fn point_inclusive_boundaries_and_outside_rejection() {
        let mut doc = document();
        doc.add_point(Point { x: 0.0, y: 0.0 }).unwrap();
        doc.add_point(Point { x: 100.0, y: 80.0 }).unwrap();
        let before = doc.clone();
        for point in [
            Point { x: -0.01, y: 0.0 },
            Point { x: 100.01, y: 0.0 },
            Point { x: 0.0, y: -0.01 },
            Point { x: 0.0, y: 80.01 },
        ] {
            assert!(doc.add_point(point).is_err());
            assert_eq!(doc, before);
        }
    }

    #[test]
    fn full_image_region_and_invalid_rectangles() {
        let mut doc = document();
        doc.add_region(rect(0.0, 0.0, 100.0, 80.0)).unwrap();
        let before = doc.clone();
        for invalid in [
            rect(0.0, 0.0, 0.0, 1.0),
            rect(0.0, 0.0, 1.0, 0.0),
            rect(1.0, 1.0, -1.0, 2.0),
            rect(-0.01, 0.0, 1.0, 1.0),
            rect(99.0, 0.0, 2.0, 1.0),
            rect(0.0, 79.0, 1.0, 2.0),
        ] {
            assert!(doc.add_region(invalid).is_err());
            assert_eq!(doc, before);
        }
    }

    #[test]
    fn all_nonfinite_geometry_is_rejected_atomically() {
        let mut doc = document();
        let id = doc.add_point(Point { x: 10.0, y: 10.0 }).unwrap();
        let before = doc.clone();
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(doc.add_point(Point { x: value, y: 1.0 }).is_err());
            assert!(doc.add_point(Point { x: 1.0, y: value }).is_err());
            for invalid in [
                rect(value, 0.0, 1.0, 1.0),
                rect(0.0, value, 1.0, 1.0),
                rect(0.0, 0.0, value, 1.0),
                rect(0.0, 0.0, 1.0, value),
            ] {
                assert!(doc.add_region(invalid).is_err());
            }
            assert!(doc.move_annotation(id, value, 0.0).is_err());
            assert!(doc.move_annotation(id, 0.0, value).is_err());
            assert_eq!(doc, before);
        }
    }

    #[test]
    fn zero_and_unrepresentable_moves_do_not_convert_a_region() {
        let mut doc = document();
        let id = doc.add_region(rect(10.0, 10.0, 10.0, 10.0)).unwrap();
        let before = doc.clone();
        doc.move_annotation(id, 0.0, -0.0).unwrap();
        doc.move_annotation(id, f32::MIN_POSITIVE, 0.0).unwrap();
        assert_eq!(doc, before);
        assert!(matches!(
            doc.annotation(id).unwrap().kind,
            AnnotationKind::Region { .. }
        ));
    }

    #[test]
    fn first_real_move_converts_region_preserving_id_text_and_source() {
        let mut doc = document();
        let source = rect(10.0, 10.0, 20.0, 15.0);
        let id = doc.add_region(source).unwrap();
        doc.set_text(id, "move this".to_owned()).unwrap();
        doc.move_annotation(id, 5.0, -3.0).unwrap();
        assert_eq!(block(&doc, id), (source, rect(15.0, 7.0, 20.0, 15.0)));
        assert_eq!(doc.annotation(id).unwrap().text, "move this");
        assert_eq!(doc.annotation(id).unwrap().id, id);
        assert_eq!(doc.annotations.len(), 1);
        assert_eq!(doc.next_id, id + 1);
    }

    #[test]
    fn repeated_moves_only_translate_target_and_never_resize_or_drift_source() {
        let mut doc = document();
        let source = rect(10.0, 10.0, 20.0, 15.0);
        let id = doc.add_region(source).unwrap();
        for (dx, dy) in [(5.0, 1.0), (10.0, 20.0), (-25.0, -31.0)] {
            doc.move_annotation(id, dx, dy).unwrap();
            let (actual_source, target) = block(&doc, id);
            assert_eq!(actual_source, source);
            assert_eq!((target.width, target.height), (20.0, 15.0));
        }
        assert_eq!(block(&doc, id).1, rect(0.0, 0.0, 20.0, 15.0));
        doc.move_annotation(id, 80.0, 65.0).unwrap();
        assert_eq!(block(&doc, id).1, rect(80.0, 65.0, 20.0, 15.0));
        let before = doc.clone();
        assert!(doc.move_annotation(id, 0.01, 0.0).is_err());
        assert_eq!(doc, before);
    }

    #[test]
    fn invalid_first_move_leaves_region_unconverted() {
        let mut doc = document();
        let id = doc.add_region(rect(0.0, 0.0, 10.0, 10.0)).unwrap();
        let before = doc.clone();
        assert!(doc.move_annotation(id, -1.0, 0.0).is_err());
        assert!(doc.move_annotation(id, 0.0, 71.0).is_err());
        assert_eq!(doc, before);
    }

    #[test]
    fn overlapping_regions_allowed_but_overlapping_block_sources_rejected() {
        let mut doc = document();
        let a = doc.add_region(rect(10.0, 10.0, 20.0, 20.0)).unwrap();
        let b = doc.add_region(rect(15.0, 15.0, 20.0, 20.0)).unwrap();
        assert!(doc.validate().is_ok());
        doc.move_annotation(a, 30.0, 0.0).unwrap();
        // A zero move is not a conversion and does not reserve a source.
        doc.move_annotation(b, 0.0, 0.0).unwrap();
        let before = doc.clone();
        assert!(doc.move_annotation(b, 1.0, 1.0).is_err());
        assert_eq!(doc, before);
        // Moving the first target away does not free the first source.
        doc.move_annotation(a, 20.0, 20.0).unwrap();
        assert!(doc.move_annotation(b, 1.0, 1.0).is_err());
    }

    #[test]
    fn touching_source_edges_are_allowed_and_targets_may_overlap() {
        let mut doc = document();
        let a = doc.add_region(rect(0.0, 0.0, 10.0, 10.0)).unwrap();
        let b = doc.add_region(rect(10.0, 0.0, 10.0, 10.0)).unwrap();
        doc.move_annotation(a, 20.0, 20.0).unwrap();
        doc.move_annotation(b, 10.0, 20.0).unwrap();
        assert_eq!(block(&doc, a).1, block(&doc, b).1);
        assert!(doc.validate().is_ok());
    }

    #[test]
    fn removed_block_releases_source_reservation() {
        let mut doc = document();
        let a = doc.add_region(rect(10.0, 10.0, 20.0, 20.0)).unwrap();
        let b = doc.add_region(rect(15.0, 15.0, 20.0, 20.0)).unwrap();
        doc.move_annotation(a, 30.0, 0.0).unwrap();
        assert!(doc.move_annotation(b, 1.0, 1.0).is_err());
        doc.remove(a).unwrap();
        doc.move_annotation(b, 1.0, 1.0).unwrap();
        assert!(doc.validate().is_ok());
    }

    #[test]
    fn points_move_to_edges_but_never_outside() {
        let mut doc = document();
        let id = doc.add_point(Point { x: 50.0, y: 40.0 }).unwrap();
        doc.move_annotation(id, -50.0, -40.0).unwrap();
        doc.move_annotation(id, 100.0, 80.0).unwrap();
        assert_eq!(
            doc.annotation(id).unwrap().kind,
            AnnotationKind::Point {
                point: Point { x: 100.0, y: 80.0 }
            }
        );
        let before = doc.clone();
        assert!(doc.move_annotation(id, 0.01, 0.0).is_err());
        assert_eq!(doc, before);
    }

    #[test]
    fn global_cannot_move_even_with_zero_delta() {
        let mut doc = document();
        let id = doc.add_global().unwrap();
        let before = doc.clone();
        assert!(doc.move_annotation(id, 0.0, 0.0).is_err());
        assert!(doc.move_annotation(id, 1.0, 1.0).is_err());
        assert_eq!(doc, before);
    }

    #[test]
    fn missing_ids_do_not_change_document() {
        let mut doc = document();
        doc.add_global().unwrap();
        let before = doc.clone();
        assert!(doc.annotation(999).is_none());
        assert!(doc.set_text(999, "missing".to_owned()).is_err());
        assert!(doc.remove(999).is_err());
        assert!(doc.move_annotation(999, 1.0, 1.0).is_err());
        assert_eq!(doc, before);
    }

    #[test]
    fn ids_are_stable_after_delete_and_do_not_follow_display_numbers() {
        let mut doc = document();
        let a = doc.add_global().unwrap();
        let b = doc.add_global().unwrap();
        doc.remove(a).unwrap();
        let c = doc.add_global().unwrap();
        assert_eq!((a, b, c), (1, 2, 3));
        assert_eq!(
            doc.annotations.iter().map(|a| a.id).collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(doc.next_id, 4);
    }

    #[test]
    fn exhausted_id_space_is_an_atomic_error() {
        let mut doc = document();
        doc.next_id = u64::MAX;
        let before = doc.clone();
        assert!(doc.validate().is_ok());
        assert!(doc.add_global().is_err());
        assert_eq!(doc, before);
    }

    #[test]
    fn large_image_bounds_are_not_rounded_through_f32() {
        let mut doc = Document::new("large", 16_777_217, 1).unwrap();
        doc.add_point(Point {
            x: 16_777_216.0,
            y: 1.0,
        })
        .unwrap();
        assert!(
            doc.add_point(Point {
                x: 16_777_218.0,
                y: 1.0
            })
            .is_err()
        );
        doc.add_region(rect(16_777_216.0, 0.0, 1.0, 1.0)).unwrap();
        assert!(doc.add_region(rect(16_777_216.0, 0.0, 2.0, 1.0)).is_err());
        let mut max_doc = Document::new("max", u32::MAX, 1).unwrap();
        assert!(
            max_doc
                .add_point(Point {
                    x: u32::MAX as f32,
                    y: 0.0
                })
                .is_err()
        );
    }

    #[test]
    fn serialization_round_trip_keeps_schema_ids_and_tagged_kinds() {
        let mut doc = document();
        let point = doc.add_point(Point { x: 1.0, y: 2.0 }).unwrap();
        doc.set_text(point, "中文\nfeedback".to_owned()).unwrap();
        doc.add_region(rect(10.0, 10.0, 10.0, 10.0)).unwrap();
        let moved = doc.add_region(rect(30.0, 30.0, 10.0, 10.0)).unwrap();
        doc.move_annotation(moved, 5.0, 5.0).unwrap();
        doc.add_global().unwrap();
        let encoded = serde_json::to_string(&doc).unwrap();
        let decoded: Document = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, doc);
        decoded.validate().unwrap();
        let value: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(value["schema"], DOCUMENT_SCHEMA);
        assert_eq!(value["next_id"], 5);
        assert_eq!(value["annotations"][0]["id"], point);
        for (index, name) in ["point", "region", "block", "global"].iter().enumerate() {
            assert_eq!(value["annotations"][index]["kind"]["kind"], *name);
        }
    }

    #[test]
    fn deserialized_documents_need_semantic_validation() {
        let valid = serde_json::to_value(document()).unwrap();
        let mut invalid = valid.clone();
        invalid["schema"] = json!("future-unsupported-schema");
        assert!(
            serde_json::from_value::<Document>(invalid)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut invalid = valid.clone();
        invalid["image"]["width"] = json!(0);
        assert!(
            serde_json::from_value::<Document>(invalid)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut invalid = valid;
        invalid["next_id"] = json!(0);
        assert!(
            serde_json::from_value::<Document>(invalid)
                .unwrap()
                .validate()
                .is_err()
        );
        assert!(serde_json::from_value::<AnnotationKind>(json!({"kind": "unknown"})).is_err());
    }

    #[test]
    fn validation_rejects_duplicate_zero_and_stale_ids() {
        let mut doc = document();
        doc.add_global().unwrap();
        let mut invalid = doc.clone();
        invalid.annotations.push(invalid.annotations[0].clone());
        assert!(invalid.validate().is_err());
        let mut invalid = doc.clone();
        invalid.annotations[0].id = 0;
        assert!(invalid.validate().is_err());
        doc.next_id = doc.annotations[0].id;
        assert!(doc.validate().is_err());
    }

    #[test]
    fn validation_rejects_resizing_and_overlapping_imported_sources() {
        let mut doc = document();
        let a = doc.add_region(rect(10.0, 10.0, 10.0, 10.0)).unwrap();
        let b = doc.add_region(rect(30.0, 10.0, 10.0, 10.0)).unwrap();
        doc.move_annotation(a, 0.0, 20.0).unwrap();
        doc.move_annotation(b, 0.0, 20.0).unwrap();
        let mut resized = doc.clone();
        if let AnnotationKind::Block { target, .. } = &mut resized.annotations[0].kind {
            target.width += 1.0;
        }
        assert!(resized.validate().is_err());
        if let AnnotationKind::Block { source, .. } = &mut doc.annotations[1].kind {
            source.x = 15.0;
        }
        assert!(doc.validate().is_err());
    }

    #[test]
    fn history_returns_callback_result_and_undo_redo_restore_exact_snapshots() {
        let original = document();
        let mut history = History::new(original.clone());
        assert!(!history.can_undo());
        assert!(!history.can_redo());
        assert!(!history.undo());
        assert!(!history.redo());
        let id = history.apply(|doc| doc.add_global()).unwrap();
        let added = history.document().clone();
        history
            .apply(|doc| doc.set_text(id, "message".to_owned()))
            .unwrap();
        let edited = history.document().clone();
        assert!(history.undo());
        assert_eq!(history.document(), &added);
        assert!(history.undo());
        assert_eq!(history.document(), &original);
        assert!(!history.undo());
        assert!(history.redo());
        assert_eq!(history.document(), &added);
        assert!(history.redo());
        assert_eq!(history.document(), &edited);
        assert!(!history.redo());
    }

    #[test]
    fn undoing_conversion_restores_region_and_redo_restores_fixed_source() {
        let mut doc = document();
        let source = rect(10.0, 10.0, 10.0, 10.0);
        let id = doc.add_region(source).unwrap();
        let mut history = History::new(doc.clone());
        history
            .apply(|doc| doc.move_annotation(id, 5.0, 6.0))
            .unwrap();
        assert_eq!(
            block(history.document(), id),
            (source, rect(15.0, 16.0, 10.0, 10.0))
        );
        assert!(history.undo());
        assert_eq!(history.document(), &doc);
        assert!(history.redo());
        assert_eq!(block(history.document(), id).0, source);
    }

    #[test]
    fn failed_multistep_transaction_is_atomic_and_preserves_redo() {
        let mut history = History::new(document());
        let id = history.apply(|doc| doc.add_global()).unwrap();
        history
            .apply(|doc| doc.set_text(id, "redo me".to_owned()))
            .unwrap();
        history.undo();
        let before = history.document().clone();
        let result: Result<(), String> = history.apply(|doc| {
            doc.set_text(id, "must roll back".to_owned())?;
            doc.add_point(Point { x: -1.0, y: 0.0 })?;
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(history.document(), &before);
        assert!(history.can_undo());
        assert!(history.can_redo());
        history.redo();
        assert_eq!(history.document().annotation(id).unwrap().text, "redo me");
    }

    #[test]
    fn post_edit_validation_failure_is_atomic() {
        let mut history = History::new(document());
        let original = history.document().clone();
        let result = history.apply(|doc| {
            doc.add_global()?;
            doc.schema = "broken".to_owned();
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(history.document(), &original);
        assert!(!history.can_undo());
        assert!(!history.can_redo());
        assert_eq!(history.apply(|doc| doc.add_global()).unwrap(), 1);
    }

    #[test]
    fn no_op_transactions_do_not_push_or_clear_redo() {
        let mut history = History::new(document());
        let id = history.apply(|doc| doc.add_global()).unwrap();
        history
            .apply(|doc| doc.set_text(id, "later".to_owned()))
            .unwrap();
        history.undo();
        let original = history.document().clone();
        assert_eq!(history.apply(|_| Ok(42)).unwrap(), 42);
        history
            .apply(|doc| doc.set_text(id, String::new()))
            .unwrap();
        assert_eq!(history.document(), &original);
        assert!(history.can_redo());
        assert!(history.undo());
        assert!(history.document().annotations.is_empty());
        assert!(!history.can_undo());
    }

    #[test]
    fn new_edit_after_undo_discards_redo_without_reusing_abandoned_ids() {
        let mut history = History::new(document());
        let a = history.apply(|doc| doc.add_global()).unwrap();
        let b = history.apply(|doc| doc.add_global()).unwrap();
        assert!(history.undo());
        let c = history.apply(|doc| doc.add_global()).unwrap();
        assert_eq!((a, b, c), (1, 2, 3));
        assert!(!history.can_redo());
        assert!(history.document().annotation(b).is_none());
        assert!(history.document().annotation(a).is_some());
        assert!(history.document().annotation(c).is_some());
    }

    #[test]
    fn no_op_after_undo_does_not_commit_id_high_water_metadata() {
        let mut history = History::new(document());
        history.apply(|doc| doc.add_global()).unwrap();
        history.undo();
        let original = history.document().clone();
        history.apply(|_| Ok(())).unwrap();
        assert_eq!(history.document(), &original);
        assert!(!history.can_undo());
        assert!(history.can_redo());
    }

    #[test]
    fn history_does_not_allow_next_id_rewind_or_deleted_id_reuse() {
        let mut doc = document();
        let id = doc.add_global().unwrap();
        doc.remove(id).unwrap();
        let mut history = History::new(doc.clone());
        assert!(
            history
                .apply(|doc| {
                    doc.next_id = 1;
                    Ok(())
                })
                .is_err()
        );
        assert!(
            history
                .apply(|doc| {
                    doc.annotations.push(Annotation {
                        id,
                        text: String::new(),
                        kind: AnnotationKind::Global,
                    });
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(history.document(), &doc);
        assert!(!history.can_undo());
    }

    #[test]
    fn undo_history_retains_only_the_last_hundred_steps() {
        let mut doc = document();
        let id = doc.add_global().unwrap();
        let mut history = History::new(doc);
        for i in 1..=105 {
            history
                .apply(|doc| doc.set_text(id, i.to_string()))
                .unwrap();
        }
        let mut undone = 0;
        while history.undo() {
            undone += 1;
        }
        assert_eq!(undone, HISTORY_LIMIT);
        assert_eq!(history.document().annotation(id).unwrap().text, "5");
        let mut redone = 0;
        while history.redo() {
            redone += 1;
        }
        assert_eq!(redone, HISTORY_LIMIT);
        assert_eq!(history.document().annotation(id).unwrap().text, "105");
    }

    #[test]
    fn replace_resets_both_stacks_and_id_allocation_session() {
        let mut history = History::new(document());
        history.apply(|doc| doc.add_global()).unwrap();
        history.apply(|doc| doc.add_global()).unwrap();
        history.undo();
        let replacement = Document::new("new.png", 25, 20).unwrap();
        history.replace(replacement.clone());
        assert_eq!(history.document(), &replacement);
        assert!(!history.can_undo());
        assert!(!history.can_redo());
        assert_eq!(history.apply(|doc| doc.add_global()).unwrap(), 1);
    }

    #[test]
    fn annotation_limit_is_validated_and_add_failure_does_not_consume_an_id() {
        let mut doc = document();
        doc.annotations = (1..=MAX_ANNOTATIONS as u64)
            .map(|id| Annotation {
                id,
                text: String::new(),
                kind: AnnotationKind::Global,
            })
            .collect();
        doc.next_id = MAX_ANNOTATIONS as u64 + 1;
        doc.validate().unwrap();
        let before = doc.clone();
        assert!(doc.add_global().is_err());
        assert!(doc.add_point(Point { x: 1.0, y: 1.0 }).is_err());
        assert!(doc.add_region(rect(1.0, 1.0, 2.0, 2.0)).is_err());
        assert_eq!(doc, before);
        doc.annotations.push(Annotation {
            id: doc.next_id,
            text: String::new(),
            kind: AnnotationKind::Global,
        });
        doc.next_id += 1;
        assert!(doc.validate().is_err());
    }

    #[test]
    fn text_limit_counts_unicode_characters_and_is_atomic() {
        let mut doc = document();
        let id = doc.add_global().unwrap();
        doc.set_text(id, "文".repeat(MAX_TEXT_CHARS)).unwrap();
        assert!(doc.validate().is_ok());
        let before = doc.clone();
        assert!(doc.set_text(id, "文".repeat(MAX_TEXT_CHARS + 1)).is_err());
        assert_eq!(doc, before);
        doc.annotations[0].text.push('a');
        assert!(doc.validate().is_err());
    }

    #[test]
    fn invalid_initial_or_replacement_documents_never_panic() {
        let mut invalid = document();
        invalid.image.width = 0;
        let mut history = History::new(invalid.clone());
        assert!(history.apply(|doc| doc.add_global()).is_err());
        assert_eq!(history.document(), &invalid);
        assert!(!history.undo());
        assert!(!history.redo());
        invalid.schema = "unsupported".to_owned();
        history.replace(invalid.clone());
        assert!(history.apply(|_| Ok(())).is_err());
        assert_eq!(history.document(), &invalid);
        assert!(!history.can_undo());
        assert!(!history.can_redo());
    }

    #[test]
    fn a_panicking_callback_cannot_modify_live_document_or_stacks() {
        let mut history = History::new(document());
        let before = history.document().clone();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<(), String> = history.apply(|doc| {
                doc.add_global()?;
                panic!("simulated editor failure");
            });
        }));
        assert!(outcome.is_err());
        assert_eq!(history.document(), &before);
        assert!(!history.can_undo());
        assert!(!history.can_redo());
    }
}
