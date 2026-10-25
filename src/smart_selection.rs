//! Pure, bounded screenshot geometry/layout heuristics, ported from the
//! independently authored MIT-compatible smart-selection.js at 6f256a6.
//! No OCR, semantic segmentation, UI tree, subprocess, network or pixel mutation.
//! Returned right/bottom edges are exclusive; a shape still describes a bbox crop.
//! Unstructured/ambiguous input deliberately abstains so manual selection can win.

use crate::document::{Point, Rect};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::f64::consts::PI;

const MAX_CANDIDATES: usize = 400;
const VISIBLE: u32 = 0x1000000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: String,
    pub rect: Rect,
    pub score: f32,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<Vec<[f32; 2]>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_height: Option<f32>,
}

// Keep intermediate arithmetic in JS's double precision; only the public
// boundary uses document f32 coordinates. All detected bbox edges are integers.
#[derive(Clone, Copy, Debug)]
struct B {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
impl B {
    fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    fn area(self) -> f64 {
        self.width * self.height
    }
    fn right(self) -> f64 {
        self.x + self.width
    }
    fn bottom(self) -> f64 {
        self.y + self.height
    }
    fn cy(self) -> f64 {
        self.y + self.height / 2.0
    }
    fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }
    fn encloses(self, other: Self) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }
    fn intersection(self, other: Self) -> f64 {
        (self.right().min(other.right()) - self.x.max(other.x)).max(0.0)
            * (self.bottom().min(other.bottom()) - self.y.max(other.y)).max(0.0)
    }
    fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }
}
impl From<Rect> for B {
    fn from(r: Rect) -> Self {
        Self::new(r.x as f64, r.y as f64, r.width as f64, r.height as f64)
    }
}
impl From<B> for Rect {
    fn from(r: B) -> Self {
        Self {
            x: r.x as f32,
            y: r.y as f32,
            width: r.width as f32,
            height: r.height as f32,
        }
    }
}
fn difference(a: u32, b: u32) -> u32 {
    (a & 255)
        .abs_diff(b & 255)
        .max(((a >> 8) & 255).abs_diff((b >> 8) & 255))
        .max(((a >> 16) & 255).abs_diff((b >> 16) & 255))
}
// JS Math.round uses ties toward +infinity, including negative refinement offsets.
fn round(n: f64) -> f64 {
    (n + 0.5).floor()
}
fn cmp(a: f64, b: f64) -> Ordering {
    a.total_cmp(&b)
}

#[derive(Clone, Debug)]
struct Template {
    shape: &'static str,
    name: Option<&'static str>,
    outline: Vec<[f64; 2]>,
    fill: f64,
}
impl Template {
    fn simple(shape: &'static str, fill: f64) -> Self {
        Self {
            shape,
            name: None,
            outline: vec![],
            fill,
        }
    }
    fn polygon(name: &'static str, outline: &[[f64; 2]], fill: f64) -> Self {
        Self {
            shape: "polygon",
            name: Some(name),
            outline: outline.to_vec(),
            fill,
        }
    }
}
fn polygons() -> Vec<Template> {
    let mut p = vec![
        Template::polygon(
            "diamond",
            &[[0.5, 0.0], [1.0, 0.5], [0.5, 1.0], [0.0, 0.5]],
            0.5,
        ),
        Template::polygon("triangle", &[[0.5, 0.0], [1.0, 1.0], [0.0, 1.0]], 0.5),
        Template::polygon("triangle", &[[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]], 0.5),
        Template::polygon("triangle", &[[0.0, 0.5], [1.0, 0.0], [1.0, 1.0]], 0.5),
        Template::polygon("triangle", &[[0.0, 0.0], [1.0, 0.5], [0.0, 1.0]], 0.5),
        Template::polygon(
            "hexagon",
            &[
                [0.25, 0.0],
                [0.75, 0.0],
                [1.0, 0.5],
                [0.75, 1.0],
                [0.25, 1.0],
                [0.0, 0.5],
            ],
            0.75,
        ),
        Template::polygon(
            "hexagon",
            &[
                [0.5, 0.0],
                [1.0, 0.25],
                [1.0, 0.75],
                [0.5, 1.0],
                [0.0, 0.75],
                [0.0, 0.25],
            ],
            0.75,
        ),
    ];
    for i in [0.15, 0.25] {
        p.extend([
            Template::polygon(
                "trapezoid",
                &[[i, 0.0], [1.0 - i, 0.0], [1.0, 1.0], [0.0, 1.0]],
                1.0 - i,
            ),
            Template::polygon(
                "trapezoid",
                &[[0.0, 0.0], [1.0, 0.0], [1.0 - i, 1.0], [i, 1.0]],
                1.0 - i,
            ),
            Template::polygon(
                "parallelogram",
                &[[i, 0.0], [1.0, 0.0], [1.0 - i, 1.0], [0.0, 1.0]],
                1.0 - i,
            ),
            Template::polygon(
                "parallelogram",
                &[[0.0, 0.0], [1.0 - i, 0.0], [1.0, 1.0], [i, 1.0]],
                1.0 - i,
            ),
        ]);
    }
    p
}
fn contour_contains(r: B, shape: Option<&str>, outline: &[[f64; 2]], x: f64, y: f64) -> bool {
    if !r.contains(x, y) {
        return false;
    }
    match shape {
        Some("ellipse") => {
            let u = (x - r.x) / r.width * 2.0 - 1.0;
            let v = (y - r.y) / r.height * 2.0 - 1.0;
            u * u + v * v <= 1.0
        }
        Some("capsule") => {
            let radius = r.width.min(r.height) / 2.0;
            let cx = x.clamp(r.x + radius, r.right() - radius);
            let cy = y.clamp(r.y + radius, r.bottom() - radius);
            (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius
        }
        Some("polygon") => {
            if outline.len() < 3 {
                return false;
            }
            let u = (x - r.x) / r.width;
            let v = (y - r.y) / r.height;
            let mut inside = false;
            for i in 0..outline.len() {
                let a = outline[(i + outline.len() - 1) % outline.len()];
                let b = outline[i];
                let cross = (u - a[0]) * (b[1] - a[1]) - (v - a[1]) * (b[0] - a[0]);
                if cross.abs() < 1e-10
                    && u >= a[0].min(b[0])
                    && u <= a[0].max(b[0])
                    && v >= a[1].min(b[1])
                    && v <= a[1].max(b[1])
                {
                    return true;
                }
                if (a[1] > v) != (b[1] > v) && u < (b[0] - a[0]) * (v - a[1]) / (b[1] - a[1]) + a[0]
                {
                    inside = !inside;
                }
            }
            inside
        }
        _ => true,
    }
}
#[derive(Clone, Debug)]
struct Hypothesis {
    id: String,
    rect: B,
    score: f64,
    kind: &'static str,
    template: Option<Template>,
    role: Option<&'static str>,
    line_count: Option<usize>,
    font_height: Option<f64>,
    parent: Option<String>,
}
impl Hypothesis {
    fn contains(&self, x: f64, y: f64) -> bool {
        contour_contains(
            self.rect,
            self.template.as_ref().map(|t| t.shape),
            self.template.as_ref().map_or(&[], |t| t.outline.as_slice()),
            x,
            y,
        )
    }
    fn candidate(self) -> Candidate {
        Candidate {
            id: self.id,
            rect: self.rect.into(),
            score: self.score as f32,
            kind: self.kind.into(),
            shape: self.template.as_ref().map(|t| t.shape.into()),
            shape_name: self
                .template
                .as_ref()
                .and_then(|t| t.name.map(String::from)),
            outline: self
                .template
                .as_ref()
                .filter(|t| !t.outline.is_empty())
                .map(|t| {
                    t.outline
                        .iter()
                        .map(|p| [p[0] as f32, p[1] as f32])
                        .collect()
                }),
            role: self.role.map(String::from),
            line_count: self.line_count,
            font_height: self.font_height.map(|h| h as f32),
        }
    }
}
fn geometric_order(a: B, b: B) -> Ordering {
    cmp(a.area(), b.area())
        .then(cmp(a.y, b.y))
        .then(cmp(a.x, b.x))
        .then(cmp(a.width, b.width))
        .then(cmp(a.height, b.height))
}
fn order(a: &Hypothesis, b: &Hypothesis) -> Ordering {
    cmp(b.score, a.score)
        .then(geometric_order(a.rect, b.rect))
        .then(a.kind.cmp(b.kind))
}
fn same_contour(a: &Hypothesis, b: &Hypothesis) -> bool {
    let (Some(t), Some(u)) = (&a.template, &b.template) else {
        return false;
    };
    if t.shape != u.shape || t.name != u.name {
        return false;
    }
    let p = a.rect;
    let q = b.rect;
    if (p.x + p.width / 2.0 - q.x - q.width / 2.0).abs() > 2.0
        || (p.cy() - q.cy()).abs() > 2.0
        || p.area().min(q.area()) / p.area().max(q.area()) < 0.65
    {
        return false;
    }
    if t.shape != "polygon" {
        return (p.x - q.x).abs() <= 4.0
            && (p.y - q.y).abs() <= 4.0
            && (p.right() - q.right()).abs() <= 4.0
            && (p.bottom() - q.bottom()).abs() <= 4.0;
    }
    if t.outline != u.outline {
        return false;
    }
    for i in 0..t.outline.len() {
        let j = (i + 1) % t.outline.len();
        let v = t.outline[i];
        let w = t.outline[j];
        let ax = p.x + v[0] * p.width;
        let ay = p.y + v[1] * p.height;
        let dx = (w[0] - v[0]) * p.width;
        let dy = (w[1] - v[1]) * p.height;
        let len = dx.hypot(dy);
        for k in [i, j] {
            let bx = q.x + u.outline[k][0] * q.width;
            let by = q.y + u.outline[k][1] * q.height;
            if ((bx - ax) * dy - (by - ay) * dx).abs() / len > 4.0 {
                return false;
            }
        }
    }
    true
}
#[derive(Clone)]
struct Piece {
    rect: B,
    color: u32,
    pixels: usize,
}
#[derive(Clone)]
struct Line {
    rect: B,
    ink: usize,
    background: u32,
    used: bool,
    closed: Option<Hypothesis>,
}
struct Evidence {
    coverage: f64,
    uniform: f64,
    fit: f64,
    rect: B,
}
struct Engine<'a> {
    cancelled: &'a dyn Fn() -> bool,
    was_cancelled: Cell<bool>,
    width: usize,
    height: usize,
    scale: f64,
    pixels: Vec<u32>,
    horizontal: Vec<u8>,
    vertical: Vec<u8>,
    labels: Vec<u32>,
    queue: Vec<usize>,
    raw: Vec<Hypothesis>,
    glyphs: Vec<Piece>,
    fragments: Vec<Piece>,
    template_checks: usize,
    serial: u32,
    shape_min: f64,
}

/// Analyze an immutable RGBA raster in memory, with a hard 400-candidate budget.
/// Scale is analysis pixels per UI unit; coordinates remain original input pixels.
pub fn detect(width: u32, height: u32, rgba: &[u8], scale: f32) -> Result<Vec<Candidate>, String> {
    detect_with_cancel(width, height, rgba, scale, || false)
}

/// Cooperatively cancellable detection. The callback is local to this call and
/// may read an AtomicBool/generation token; a true result is latched. Checks occur
/// at scan rows, bounded traversal chunks, template fits and layout groups. No
/// partial candidates escape: cancellation returns exactly Err("cancelled").
/// Validation errors still take precedence over cancellation for invalid input.
pub fn detect_with_cancel(
    width: u32,
    height: u32,
    rgba: &[u8],
    scale: f32,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<Candidate>, String> {
    if !(8..=1600).contains(&width)
        || !(8..=1600).contains(&height)
        || u64::from(width) * u64::from(height) > 2_000_000
    {
        return Err("Image exceeds the 8..1600 axis / 2,000,000 pixel budget".into());
    }
    let count = width as usize * height as usize;
    if rgba.len() != count * 4 {
        return Err("RGBA byte length does not match dimensions".into());
    }
    if !scale.is_finite() || !(0.25..=4.0).contains(&scale) {
        return Err("scale must be 0.25..4".into());
    }
    if cancelled() {
        return Err("cancelled".into());
    }
    let mut pixels = Vec::with_capacity(count);
    let mut visible = 0;
    for (i, p) in rgba.chunks_exact(4).enumerate() {
        if i & 4095 == 0 && cancelled() {
            return Err("cancelled".into());
        }
        let alpha = p[3] as u32;
        let composite = |v: u8| (v as u32 * alpha + 255 * (255 - alpha) + 127) / 255;
        pixels.push(
            composite(p[0])
                | composite(p[1]) << 8
                | composite(p[2]) << 16
                | if alpha >= 32 { VISIBLE } else { 0 },
        );
        if alpha >= 32 {
            visible += 1;
        }
    }
    if visible < 32 {
        return Ok(vec![]);
    }
    let mut e = Engine {
        cancelled: &cancelled,
        was_cancelled: Cell::new(false),
        width: width as usize,
        height: height as usize,
        scale: scale as f64,
        pixels,
        horizontal: vec![255; count],
        vertical: vec![255; count],
        labels: vec![0; count],
        queue: vec![0; count],
        raw: vec![],
        glyphs: vec![],
        fragments: vec![],
        template_checks: 0,
        serial: 0,
        shape_min: 18.0_f64.max(round(18.0 * scale as f64)),
    };
    for y in 0..e.height {
        if e.stopped() {
            return Err("cancelled".into());
        }
        for x in 0..e.width {
            let i = y * e.width + x;
            let p = e.pixels[i];
            if x + 1 < e.width {
                e.horizontal[i] = if (p ^ e.pixels[i + 1]) & VISIBLE != 0 {
                    255
                } else {
                    difference(p, e.pixels[i + 1]) as u8
                };
            }
            if y + 1 < e.height {
                e.vertical[i] = if (p ^ e.pixels[i + e.width]) & VISIBLE != 0 {
                    255
                } else {
                    difference(p, e.pixels[i + e.width]) as u8
                };
            }
        }
    }
    e.chromatic();
    if e.stopped() {
        return Err("cancelled".into());
    }
    e.edge_shapes();
    if e.stopped() {
        return Err("cancelled".into());
    }
    e.glyph_runs();
    if e.stopped() {
        return Err("cancelled".into());
    }
    let groups = e.layout();
    if e.stopped() {
        return Err("cancelled".into());
    }
    let result = e.finish(groups);
    if e.stopped() {
        return Err("cancelled".into());
    }
    Ok(result.into_iter().map(Hypothesis::candidate).collect())
}
impl Engine<'_> {
    fn stopped(&self) -> bool {
        if !self.was_cancelled.get() && (self.cancelled)() {
            self.was_cancelled.set(true);
        }
        self.was_cancelled.get()
    }
    fn add(
        &mut self,
        rect: B,
        score: f64,
        kind: &'static str,
        template: Option<Template>,
    ) -> Hypothesis {
        let shape = template
            .as_ref()
            .map_or(String::new(), |t| format!(":{}", t.name.unwrap_or(t.shape)));
        let c = Hypothesis {
            id: format!(
                "{kind}{shape}:{},{},{},{}",
                rect.x, rect.y, rect.width, rect.height
            ),
            rect,
            score: round(score.clamp(0.05, 1.0) * 10000.0) / 10000.0,
            kind,
            template,
            role: None,
            line_count: None,
            font_height: None,
            parent: None,
        };
        self.raw.push(c.clone());
        if self.raw.len() >= 1600 {
            self.raw.sort_by(order);
            self.raw.truncate(800);
        }
        c
    }
    fn perimeter(&self, x0: usize, y0: usize, x1: usize, y1: usize, label: u32) -> (f64, f64) {
        let w = x1 - x0 + 1;
        let h = y1 - y0 + 1;
        let ix = ((w as f64 * 0.22).floor() as usize).min(round(12.0 * self.scale) as usize);
        let iy = ((h as f64 * 0.22).floor() as usize).min(round(12.0 * self.scale) as usize);
        let mut n = [0; 4];
        let mut c = [0; 4];
        for x in x0 + ix..=x1 - ix {
            let t = y0 * self.width + x;
            let b = y1 * self.width + x;
            if self.labels[t] == label {
                n[0] += 1;
                if y0 > 0 && self.vertical[t - self.width] >= 4 {
                    c[0] += 1;
                }
            }
            if self.labels[b] == label {
                n[1] += 1;
                if y1 + 1 < self.height && self.vertical[b] >= 4 {
                    c[1] += 1;
                }
            }
        }
        for y in y0 + iy..=y1 - iy {
            let l = y * self.width + x0;
            let r = y * self.width + x1;
            if self.labels[l] == label {
                n[2] += 1;
                if x0 > 0 && self.horizontal[l - 1] >= 4 {
                    c[2] += 1;
                }
            }
            if self.labels[r] == label {
                n[3] += 1;
                if x1 + 1 < self.width && self.horizontal[r] >= 4 {
                    c[3] += 1;
                }
            }
        }
        let denominators = [w - 2 * ix, w - 2 * ix, h - 2 * iy, h - 2 * iy];
        let membership = (0..4)
            .map(|i| n[i] as f64 / denominators[i] as f64)
            .fold(1.0, f64::min);
        let contrast = (0..4)
            .map(|i| c[i] as f64 / denominators[i] as f64)
            .fold(1.0, f64::min);
        (membership, contrast)
    }
    fn chromatic(&mut self) {
        let count = self.pixels.len();
        let min_side = 8.0_f64.max(round(12.0 * self.scale));
        let min_area = 96.0_f64.max(round(180.0 * self.scale * self.scale));
        for threshold in [3, 18] {
            self.labels.fill(0);
            for seed in 0..count {
                if seed & 4095 == 0 && self.stopped() {
                    return;
                }
                if self.labels[seed] != 0 || self.pixels[seed] & VISIBLE == 0 {
                    continue;
                }
                self.serial += 1;
                let label = self.serial;
                let mut head = 0;
                let mut tail = 1;
                self.queue[0] = seed;
                self.labels[seed] = label;
                let (mut x0, mut y0, mut x1, mut y1, mut flat) =
                    (self.width, self.height, 0, 0, 0usize);
                let mut mins = [255u32; 3];
                let mut maxs = [0u32; 3];
                while head < tail {
                    if head & 4095 == 0 && self.stopped() {
                        return;
                    }
                    let at = self.queue[head];
                    head += 1;
                    let y = at / self.width;
                    let x = at - y * self.width;
                    let p = self.pixels[at];
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    y0 = y0.min(y);
                    y1 = y1.max(y);
                    for k in 0..3 {
                        let v = (p >> (k * 8)) & 255;
                        mins[k] = mins[k].min(v);
                        maxs[k] = maxs[k].max(v);
                    }
                    if self.horizontal[at] <= 4 && self.vertical[at] <= 4 {
                        flat += 1;
                    }
                    let mut enqueue = |next: usize| {
                        if self.labels[next] == 0 {
                            self.labels[next] = label;
                            self.queue[tail] = next;
                            tail += 1;
                        }
                    };
                    if x > 0 && self.horizontal[at - 1] <= threshold {
                        enqueue(at - 1);
                    }
                    if x + 1 < self.width && self.horizontal[at] <= threshold {
                        enqueue(at + 1);
                    }
                    if y > 0 && self.vertical[at - self.width] <= threshold {
                        enqueue(at - self.width);
                    }
                    if y + 1 < self.height && self.vertical[at] <= threshold {
                        enqueue(at + self.width);
                    }
                }
                let w = (x1 - x0 + 1) as f64;
                let h = (y1 - y0 + 1) as f64;
                let range = (0..3).map(|k| maxs[k] - mins[k]).max().unwrap_or(0);
                let frame = x0 == 0 || y0 == 0 || x1 == self.width - 1 || y1 == self.height - 1;
                let rect = B::new(x0 as f64, y0 as f64, w, h);
                if threshold == 3
                    && !frame
                    && range <= 80
                    && h <= 48.0 * self.scale
                    && w <= 64.0 * self.scale
                    && tail >= 2
                    && tail as f64 <= 1600.0 * self.scale * self.scale
                    && tail as f64 / (w * h) >= 0.10
                {
                    let piece = Piece {
                        rect,
                        color: self.pixels[seed],
                        pixels: tail,
                    };
                    if h >= 5.0_f64.max(5.0 * self.scale) && w <= h * 2.8 {
                        if self.glyphs.len() < 6000 {
                            self.glyphs.push(piece);
                        }
                    } else if self.fragments.len() < 6000 {
                        self.fragments.push(piece);
                    }
                }
                if frame
                    || w < min_side
                    || h < min_side
                    || w * h < min_area
                    || w > h * 24.0
                    || h > w * 12.0
                    || range > 80
                    || w * h > count as f64 * 0.88
                {
                    continue;
                }
                let (membership, contrast) = self.perimeter(x0, y0, x1, y1, label);
                let occupancy = tail as f64 / (w * h);
                let border = occupancy < 0.42;
                let rectangular = membership >= 0.66
                    && contrast >= 0.60
                    && if border {
                        membership >= 0.84 && tail as f64 >= (w + h) * 0.8
                    } else {
                        occupancy >= 0.50 && flat as f64 / tail as f64 >= 0.48
                    };
                let kind = if h <= 80.0 * self.scale && w <= 480.0 * self.scale {
                    "control"
                } else {
                    "panel"
                };
                if rectangular {
                    let score = 0.67
                        + 0.13 * membership
                        + 0.13 * contrast
                        + if border {
                            0.045
                        } else {
                            0.035 * (flat as f64 / tail as f64).min(1.0)
                        };
                    self.add(rect, score, kind, None);
                } else {
                    let Some((template, ev)) = self.match_shape(rect, label, occupancy, false)
                    else {
                        continue;
                    };
                    self.add(
                        rect,
                        0.83 + 0.10 * ev.coverage + 0.04 * ev.uniform + 0.015 * ev.fit,
                        kind,
                        Some(template),
                    );
                }
                if threshold == 3
                    && self.glyphs.last().is_some_and(|g| {
                        g.rect.x == rect.x
                            && g.rect.y == rect.y
                            && g.rect.width == w
                            && g.rect.height == h
                    })
                {
                    self.glyphs.pop();
                }
            }
        }
    }
}

fn boundary(t: &Template, w: f64, h: f64) -> Vec<[f64; 4]> {
    let mut points = Vec::with_capacity(72);
    if t.shape == "ellipse" {
        for i in 0..72 {
            let a = (i as f64 + 0.5) * PI * 2.0 / 72.0;
            let c = a.cos();
            let s = a.sin();
            let nx = c / w;
            let ny = s / h;
            let len = nx.hypot(ny);
            points.push([
                w / 2.0 + w / 2.0 * c,
                h / 2.0 + h / 2.0 * s,
                nx / len,
                ny / len,
            ]);
        }
    } else if t.shape == "capsule" {
        let horizontal = w >= h;
        let long = w.max(h);
        let short = w.min(h);
        let r = short / 2.0;
        let mut put = |x, y, nx, ny| {
            points.push(if horizontal {
                [x, y, nx, ny]
            } else {
                [y, x, ny, nx]
            })
        };
        for i in 0..18 {
            let t = (i as f64 + 0.5) / 18.0;
            put(r + (long - short) * t, 0.0, 0.0, -1.0);
        }
        for i in 0..18 {
            let a = -PI / 2.0 + (i as f64 + 0.5) * PI / 18.0;
            put(long - r + r * a.cos(), r + r * a.sin(), a.cos(), a.sin());
        }
        for i in 0..18 {
            let t = (i as f64 + 0.5) / 18.0;
            put(long - r - (long - short) * t, short, 0.0, 1.0);
        }
        for i in 0..18 {
            let a = PI / 2.0 + (i as f64 + 0.5) * PI / 18.0;
            put(r + r * a.cos(), r + r * a.sin(), a.cos(), a.sin());
        }
    } else {
        let per_side = 72.0 / t.outline.len() as f64;
        for i in 0..t.outline.len() {
            let a = t.outline[i];
            let b = t.outline[(i + 1) % t.outline.len()];
            let dx = (b[0] - a[0]) * w;
            let dy = (b[1] - a[1]) * h;
            let len = dx.hypot(dy);
            for j in 0..per_side.ceil() as usize {
                let t = (j as f64 + 0.5) / per_side;
                points.push([a[0] * w + dx * t, a[1] * h + dy * t, dy / len, -dx / len]);
            }
        }
    }
    points
}
impl Engine<'_> {
    fn refine_polygon(&self, r: B, observations: &[[f64; 5]]) -> Option<B> {
        let mut matrix = [[0.0f64; 5]; 4];
        for o in observations {
            let row = [o[2], o[3], o[2] * o[0], o[3] * o[1]];
            for i in 0..4 {
                for j in 0..4 {
                    matrix[i][j] += row[i] * row[j];
                }
                matrix[i][4] += row[i] * o[4];
            }
        }
        for (i, row) in matrix.iter_mut().enumerate() {
            row[i] += 0.0001;
        }
        for col in 0..4 {
            let mut pivot = col;
            for row in col + 1..4 {
                if matrix[row][col].abs() > matrix[pivot][col].abs() {
                    pivot = row;
                }
            }
            matrix.swap(col, pivot);
            let divisor = matrix[col][col];
            if divisor.abs() < 1e-8 {
                return None;
            }
            for j in col..5 {
                matrix[col][j] /= divisor;
            }
            for row in 0..4 {
                if row != col {
                    let factor = matrix[row][col];
                    for j in col..5 {
                        matrix[row][j] -= factor * matrix[col][j];
                    }
                }
            }
        }
        let [dx, dy, dw, dh] = matrix.map(|row| row[4]);
        if [dx, dy, dx + dw, dy + dh]
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 3.0)
        {
            return None;
        }
        let x = round(r.x + dx);
        let y = round(r.y + dy);
        let right = round(r.right() + dx + dw);
        let bottom = round(r.bottom() + dy + dh);
        if x == r.x && y == r.y && right == r.right() && bottom == r.bottom() {
            return None;
        }
        if x < 1.0
            || y < 1.0
            || right >= self.width as f64
            || bottom >= self.height as f64
            || right - x < self.shape_min
            || bottom - y < self.shape_min
        {
            return None;
        }
        Some(B::new(x, y, right - x, bottom - y))
    }
    fn closed_template(
        &mut self,
        r: B,
        label: u32,
        t: &Template,
        refine: bool,
    ) -> Option<Evidence> {
        if self.stopped() {
            return None;
        }
        self.template_checks += 1;
        if self.template_checks > 6000 {
            return None;
        }
        let points = boundary(t, r.width, r.height);
        let reference_x = r.x - 3.0;
        let reference_y = r.y - 3.0;
        if reference_x < 0.0 || reference_y < 0.0 {
            return None;
        }
        let background = self.pixels[reference_y as usize * self.width + reference_x as usize];
        let mut sectors = [[0.0; 4]; 8];
        let mut offsets = vec![];
        let mut observations = vec![];
        let mut supported = 0;
        let mut exterior = 0;
        for (i, [px, py, nx, ny]) in points.iter().copied().enumerate() {
            let x = r.x + px;
            let y = r.y + py;
            let sector = &mut sectors[i * 8 / points.len()];
            sector[0] += 1.0;
            let (ox, oy, fx, fy) = (
                (x + nx * 4.0).floor(),
                (y + ny * 4.0).floor(),
                (x + nx * 7.0).floor(),
                (y + ny * 7.0).floor(),
            );
            if ox < 0.0
                || ox >= self.width as f64
                || oy < 0.0
                || oy >= self.height as f64
                || fx < 0.0
                || fx >= self.width as f64
                || fy < 0.0
                || fy >= self.height as f64
            {
                continue;
            }
            let at = oy as usize * self.width + ox as usize;
            let outside = self.pixels[at];
            if difference(outside, background) <= 16
                && difference(outside, self.pixels[fy as usize * self.width + fx as usize]) <= 12
                && self.horizontal[at] <= 12
                && self.vertical[at] <= 12
            {
                exterior += 1;
                sector[3] += 1.0;
            }
            let mut member = false;
            let mut first = None;
            for step in 0..=8 {
                let d = 2.0 - step as f64 * 0.5;
                let xx = (x + nx * d).floor();
                let yy = (y + ny * d).floor();
                if xx < 0.0 || xx >= self.width as f64 || yy < 0.0 || yy >= self.height as f64 {
                    continue;
                }
                let p = yy as usize * self.width + xx as usize;
                if self.labels[p] == label {
                    member = true;
                }
                if first.is_none() && difference(self.pixels[p], outside) >= 4 {
                    first = Some(d);
                }
            }
            if member {
                if let Some(first) = first {
                    supported += 1;
                    sector[1] += 1.0;
                    sector[2] += first;
                    offsets.push(first);
                    if refine {
                        observations.push([px / r.width, py / r.height, nx, ny, first]);
                    }
                }
            }
        }
        let coverage = supported as f64 / points.len() as f64;
        let uniform = exterior as f64 / points.len() as f64;
        if coverage < 0.94
            || uniform < 0.90
            || sectors
                .iter()
                .any(|s| s[1] / s[0] < 0.85 || s[3] / s[0] < 0.75)
        {
            return None;
        }
        let mean = offsets.iter().sum::<f64>() / offsets.len() as f64;
        let deviation =
            offsets.iter().map(|v| (v - mean).abs()).sum::<f64>() / offsets.len() as f64;
        let sector_offsets = sectors.map(|s| s[2] / s[1]);
        if deviation > 0.80
            || sector_offsets
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max)
                - sector_offsets.iter().copied().fold(f64::INFINITY, f64::min)
                > 1.8
        {
            if refine && t.shape == "polygon" {
                if let Some(rect) = self.refine_polygon(r, &observations) {
                    return self.closed_template(rect, label, t, false);
                }
            }
            return None;
        }
        Some(Evidence {
            coverage,
            uniform,
            fit: 1.0 - deviation.min(1.0),
            rect: r,
        })
    }
    fn match_shape(
        &mut self,
        r: B,
        label: u32,
        occupancy: f64,
        edge_only: bool,
    ) -> Option<(Template, Evidence)> {
        let w = r.width;
        let h = r.height;
        if w < self.shape_min
            || h < self.shape_min
            || w > h * 4.0
            || h > w * 4.0
            || w * h > self.pixels.len() as f64 * 0.70
            || self.template_checks >= 6000
        {
            return None;
        }
        let mut templates = vec![Template::simple("ellipse", PI / 4.0)];
        if w.max(h) / w.min(h) >= 1.35 {
            templates.push(Template::simple(
                "capsule",
                1.0 - (1.0 - PI / 4.0) * w.min(h) / w.max(h),
            ));
        }
        templates.extend(polygons());
        let mut best = None;
        let mut best_quality = f64::NEG_INFINITY;
        for template in templates {
            if !edge_only
                && occupancy >= 0.35
                && (occupancy > template.fill + 0.07 || occupancy < template.fill * 0.55)
            {
                continue;
            }
            let Some(ev) = self.closed_template(r, label, &template, edge_only) else {
                continue;
            };
            let quality = ev.coverage + ev.uniform + ev.fit * 0.15;
            if quality > best_quality {
                best_quality = quality;
                best = Some((template, ev));
            }
        }
        best
    }
    fn edge_mask(&self) -> Vec<u8> {
        let mut mask = vec![0; self.pixels.len()];
        for y in 0..self.height {
            if self.stopped() {
                return mask;
            }
            for x in 0..self.width {
                let i = y * self.width + x;
                mask[i] = u8::from(
                    self.horizontal[i] >= 12
                        || self.vertical[i] >= 12
                        || (x > 0 && self.horizontal[i - 1] >= 12)
                        || (y > 0 && self.vertical[i - self.width] >= 12),
                );
            }
        }
        mask
    }
    // Iterative 8-connected traversal, shared by edge and dilated-ink passes.
    fn flood_mask(&mut self, seed: usize, mask: &[u8], label: u32) -> usize {
        self.queue[0] = seed;
        self.labels[seed] = label;
        let mut head = 0;
        let mut tail = 1;
        while head < tail {
            if head & 4095 == 0 && self.stopped() {
                return 0;
            }
            let at = self.queue[head];
            head += 1;
            let y = at / self.width;
            let x = at % self.width;
            for yy in y.saturating_sub(1)..=(y + 1).min(self.height - 1) {
                for xx in x.saturating_sub(1)..=(x + 1).min(self.width - 1) {
                    let next = yy * self.width + xx;
                    if mask[next] != 0 && self.labels[next] == 0 {
                        self.labels[next] = label;
                        self.queue[tail] = next;
                        tail += 1;
                    }
                }
            }
        }
        tail
    }
    fn edge_shapes(&mut self) {
        let mask = self.edge_mask();
        self.labels.fill(0);
        for seed in 0..self.pixels.len() {
            if seed & 4095 == 0 && self.stopped() {
                return;
            }
            if mask[seed] == 0 || self.labels[seed] != 0 {
                continue;
            }
            self.serial += 1;
            let label = self.serial;
            let tail = self.flood_mask(seed, &mask, label);
            if self.stopped() {
                return;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (self.width, self.height, 0, 0);
            for &at in &self.queue[..tail] {
                let x = at % self.width;
                let y = at / self.width;
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
            let w = (x1 - x0 + 1) as f64;
            let h = (y1 - y0 + 1) as f64;
            if x0 == 0
                || y0 == 0
                || x1 == self.width - 1
                || y1 == self.height - 1
                || (tail as f64) < (w + h) * 0.8
            {
                continue;
            }
            let rect = B::new((x0 + 1) as f64, (y0 + 1) as f64, w - 2.0, h - 2.0);
            if let Some((t, ev)) = self.match_shape(rect, label, tail as f64 / (w * h), true) {
                let kind = if h <= 80.0 * self.scale && w <= 480.0 * self.scale {
                    "control"
                } else {
                    "panel"
                };
                self.add(
                    ev.rect,
                    0.836 + 0.10 * ev.coverage + 0.04 * ev.uniform + 0.015 * ev.fit,
                    kind,
                    Some(t),
                );
            }
        }
    }
    fn emit_run(&mut self, run: &[Piece], fragments: &[Piece], color: u32) {
        if run.len() < 3 {
            return;
        }
        let mut r = run[0].rect;
        let mut ink = 0;
        let mut total_h = 0.0;
        for g in run {
            r = r.union(g.rect);
            ink += g.pixels;
            total_h += g.rect.height;
        }
        let mean_h = total_h / run.len() as f64;
        for f in fragments {
            if f.rect.right() < r.x - mean_h * 0.18 || f.rect.x > r.right() + mean_h * 0.18 {
                continue;
            }
            r = r.union(f.rect);
            ink += f.pixels;
        }
        if r.width < r.height * 1.5
            || r.height > mean_h * 1.7
            || (ink as f64) / r.area() < 0.08
            || (ink as f64) / r.area() > 0.85
        {
            return;
        }
        let background = self.pixels[(r.y - 2.0).max(0.0) as usize * self.width + r.x as usize];
        let mut samples = 0;
        let mut good = 0;
        let mut x = r.x as usize;
        while (x as f64) < r.right() {
            for y in [r.y - 2.0, r.bottom() + 1.0] {
                if y < 0.0 || y >= self.height as f64 {
                    continue;
                }
                samples += 1;
                if difference(self.pixels[y as usize * self.width + x], background) <= 12 {
                    good += 1;
                }
            }
            x += 1usize.max((r.width / 48.0).floor() as usize);
        }
        if samples == 0
            || good as f64 / (samples as f64) < 0.88
            || difference(background, color) < 24
        {
            return;
        }
        let pad = 1.0_f64.max(round(2.0 * self.scale));
        let rect = self.padded(r, pad, pad);
        self.add(
            rect,
            0.65 + 0.07 * (run.len() as f64 / 8.0).min(1.0),
            "text",
            None,
        );
    }
    fn glyph_runs(&mut self) {
        struct Row {
            cy: f64,
            height: f64,
            color: u32,
            items: Vec<Piece>,
            fragments: Vec<Piece>,
        }
        self.glyphs
            .sort_by(|a, b| cmp(a.rect.cy(), b.rect.cy()).then(cmp(a.rect.x, b.rect.x)));
        let mut rows: Vec<Row> = vec![];
        for g in &self.glyphs {
            if self.stopped() {
                return;
            }
            let cy = g.rect.cy();
            let mut matched = None;
            let mut best = f64::INFINITY;
            for j in (0..rows.len()).rev().take(32) {
                let r = &rows[j];
                let delta = (cy - r.cy).abs();
                let ratio = g.rect.height / r.height;
                if delta <= g.rect.height.min(r.height) * 0.32
                    && ratio > 0.45
                    && ratio < 2.2
                    && difference(g.color, r.color) < 64
                    && delta < best
                {
                    matched = Some(j);
                    best = delta;
                }
            }
            if let Some(j) = matched {
                let r = &mut rows[j];
                r.items.push(g.clone());
                r.cy = (r.cy * (r.items.len() - 1) as f64 + cy) / r.items.len() as f64;
            } else {
                rows.push(Row {
                    cy,
                    height: g.rect.height,
                    color: g.color,
                    items: vec![g.clone()],
                    fragments: vec![],
                });
            }
        }
        let cell = 12.0_f64.max(round(24.0 * self.scale));
        let mut bins: HashMap<(i32, i32), Vec<(usize, Piece)>> = HashMap::new();
        for (j, row) in rows.iter().enumerate() {
            for g in &row.items {
                let key = (
                    ((g.rect.x + g.rect.width / 2.0) / cell).floor() as i32,
                    (g.rect.cy() / cell).floor() as i32,
                );
                let bin = bins.entry(key).or_default();
                if bin.len() < 32 {
                    bin.push((j, g.clone()));
                }
            }
        }
        for f in &self.fragments {
            if self.stopped() {
                return;
            }
            let mut matched = None;
            let mut best = f64::INFINITY;
            let cy = f.rect.cy();
            let bx = ((f.rect.x + f.rect.width / 2.0) / cell).floor() as i32;
            let by = (cy / cell).floor() as i32;
            for yy in by - 2..=by + 2 {
                for xx in bx - 2..=bx + 2 {
                    let Some(bin) = bins.get(&(xx, yy)) else {
                        continue;
                    };
                    for (j, g) in bin {
                        let row = &rows[*j];
                        if row.items.len() < 3
                            || (cy - row.cy).abs() > row.height * 0.65
                            || difference(f.color, row.color) > 32
                        {
                            continue;
                        }
                        let dx = 0.0_f64
                            .max(g.rect.x - f.rect.right())
                            .max(f.rect.x - g.rect.right());
                        let dy = 0.0_f64
                            .max(g.rect.y - f.rect.bottom())
                            .max(f.rect.y - g.rect.bottom());
                        let distance = dx + dy;
                        if distance <= 1.0_f64.max(round(g.rect.height * 0.18)) && distance < best {
                            matched = Some(*j);
                            best = distance;
                        }
                    }
                }
            }
            if let Some(j) = matched {
                if rows[j].fragments.len() < 128 {
                    rows[j].fragments.push(f.clone());
                }
            }
        }
        for row in &mut rows {
            if self.stopped() {
                return;
            }
            row.items.sort_by(|a, b| cmp(a.rect.x, b.rect.x));
            let mut run: Vec<Piece> = vec![];
            for g in &row.items {
                if let Some(previous) = run.last() {
                    if g.rect.x - previous.rect.right()
                        > (4.0 * self.scale).max(g.rect.height.max(previous.rect.height) * 1.15)
                        || g.rect.x
                            < previous.rect.right() - g.rect.width.min(previous.rect.width) * 0.25
                    {
                        self.emit_run(&run, &row.fragments, row.color);
                        run.clear();
                    }
                }
                run.push(g.clone());
            }
            self.emit_run(&run, &row.fragments, row.color);
        }
    }
    fn padded(&self, r: B, px: f64, py: f64) -> B {
        let x = (r.x - px).floor().max(0.0);
        let y = (r.y - py).floor().max(0.0);
        B::new(
            x,
            y,
            (r.right() + px).ceil().min(self.width as f64) - x,
            (r.bottom() + py).ceil().min(self.height as f64) - y,
        )
    }
    fn closed_label(&self, line: &Line) -> Option<Hypothesis> {
        self.raw
            .iter()
            .find(|c| {
                let r = c.rect;
                let l = line.rect;
                c.kind == "control"
                    && l.width <= r.width * 1.25
                    && l.height <= r.height * 1.25
                    && c.contains(l.x + l.width / 2.0, l.cy())
                    && r.intersection(l) / r.area().min(l.area()) >= 0.80
            })
            .cloned()
    }
    fn finish_line(&self, line: Option<Line>, lines: &mut Vec<Line>) {
        let Some(mut line) = line else {
            return;
        };
        if line.rect.width < line.rect.height * 1.8
            || (line.ink as f64) < 8.0_f64.max(line.rect.height * 1.5)
        {
            return;
        }
        if let Some(control) = self.closed_label(&line) {
            if control.template.is_none()
                && control.rect.width / control.rect.height >= 4.5
                && control.rect.width >= line.rect.height * 10.0
            {
                line.closed = Some(control);
                lines.push(line);
            }
        } else {
            lines.push(line);
        }
    }
    fn surface_for(&self, line: &Line, max_height: f64, ink: &[u8]) -> Option<B> {
        let r = line.rect;
        let sx = r.x.floor().max(0.0) as usize;
        let sy = (r.y - 2.0).max(0.0) as usize;
        let paint = self.pixels[sy * self.width + sx];
        let mut left = sx;
        let mut end = sx;
        while left > 0 && difference(self.pixels[sy * self.width + left - 1], paint) <= 3 {
            left -= 1;
        }
        while end + 1 < self.width && difference(self.pixels[sy * self.width + end + 1], paint) <= 3
        {
            end += 1;
        }
        if left == 0
            || end + 1 == self.width
            || ((end - left) as f64) < r.height * 5.0
            || (end - left) as f64 > self.width as f64 * 0.8
        {
            return None;
        }
        let painted = |y: usize| {
            let mut good = 0;
            for k in 0..24 {
                let x =
                    (left as f64 + (end - left) as f64 * (k as f64 + 0.5) / 24.0).floor() as usize;
                let at = y * self.width + x;
                if difference(self.pixels[at], paint) <= 3 || ink[at] != 0 {
                    good += 1;
                }
            }
            good >= 16
        };
        let mut top = sy;
        let mut bottom = sy;
        while top > 0 && ((sy - top) as f64) < max_height && painted(top - 1) {
            top -= 1;
        }
        while bottom + 1 < self.height && ((bottom - sy) as f64) < max_height && painted(bottom + 1)
        {
            bottom += 1;
        }
        let h = (bottom - top + 1) as f64;
        if h < r.height * 1.3
            || h > max_height
            || top as f64 > r.y
            || (bottom as f64) < r.bottom() - 1.0
        {
            return None;
        }
        if left < 2 || end + 2 >= self.width || top < 2 || bottom + 2 >= self.height {
            return None;
        }
        let mut contrast = [0; 4];
        for k in 0..16 {
            let x = (left as f64 + (end - left) as f64 * (0.12 + 0.76 * (k as f64 + 0.5) / 16.0))
                .floor() as usize;
            let y = (top as f64 + (bottom - top) as f64 * (0.15 + 0.70 * (k as f64 + 0.5) / 16.0))
                .floor() as usize;
            if difference(self.pixels[(top - 2) * self.width + x], paint) >= 5 {
                contrast[0] += 1;
            }
            if difference(self.pixels[(bottom + 2) * self.width + x], paint) >= 5 {
                contrast[1] += 1;
            }
            if difference(self.pixels[y * self.width + left - 2], paint) >= 5 {
                contrast[2] += 1;
            }
            if difference(self.pixels[y * self.width + end + 2], paint) >= 5 {
                contrast[3] += 1;
            }
        }
        if contrast.iter().any(|&n| (n as f64) < 16.0 * 0.70) {
            return None;
        }
        Some(B::new(left as f64, top as f64, (end - left + 1) as f64, h))
    }
    fn group(
        &mut self,
        rect: B,
        score: f64,
        kind: &'static str,
        font: f64,
        children: &[usize],
        role: &'static str,
        lines: &mut [Line],
        groups: &mut Vec<Hypothesis>,
    ) {
        if groups.len() >= 160 || rect.area() > self.pixels.len() as f64 * 0.65 {
            return;
        }
        let mut c = self.add(rect, score, kind, None);
        c.role = Some(role);
        c.font_height = Some(round(font * 100.0) / 100.0);
        c.line_count = Some(children.len());
        // add() can truncate/reorder raw, so update by stable ID, not stale index.
        if let Some(raw) = self.raw.iter_mut().rev().find(|raw| raw.id == c.id) {
            *raw = c.clone();
        }
        groups.push(c);
        for &child in children {
            lines[child].used = true;
        }
    }
    fn layout(&mut self) -> Vec<Hypothesis> {
        let tile = 12usize.max(round(24.0 * self.scale) as usize);
        let columns = self.width.div_ceil(tile);
        let tile_rows = self.height.div_ceil(tile);
        let mut backgrounds = vec![0u32; columns * tile_rows];
        let mut confident = vec![false; backgrounds.len()];
        let mut histogram = vec![0u16; 32768];
        let key = |p: u32| {
            ((p & 255) >> 3) | ((((p >> 8) & 255) >> 3) << 5) | ((((p >> 16) & 255) >> 3) << 10)
        };
        for ty in 0..tile_rows {
            if self.stopped() {
                return vec![];
            }
            for tx in 0..columns {
                let left = tx * tile;
                let top = ty * tile;
                let right = self.width.min(left + tile);
                let bottom = self.height.min(top + tile);
                let (mut samples, mut best_key, mut best_count) = (0usize, 0usize, 0u16);
                let mut touched = Vec::with_capacity(tile * tile / 4);
                for y in (top..bottom).step_by(2) {
                    for x in (left..right).step_by(2) {
                        let p = self.pixels[y * self.width + x];
                        if p & VISIBLE == 0 {
                            continue;
                        }
                        samples += 1;
                        let k = key(p) as usize;
                        if histogram[k] == 0 {
                            touched.push(k);
                        }
                        histogram[k] += 1;
                        if histogram[k] > best_count {
                            best_count = histogram[k];
                            best_key = k;
                        }
                    }
                }
                if best_count >= 4 && best_count as f64 / samples.max(1) as f64 >= 0.28 {
                    let (mut red, mut green, mut blue, mut n) = (0u32, 0u32, 0u32, 0u32);
                    for y in (top..bottom).step_by(2) {
                        for x in (left..right).step_by(2) {
                            let p = self.pixels[y * self.width + x];
                            if p & VISIBLE != 0 && key(p) as usize == best_key {
                                red += p & 255;
                                green += (p >> 8) & 255;
                                blue += (p >> 16) & 255;
                                n += 1;
                            }
                        }
                    }
                    let at = ty * columns + tx;
                    backgrounds[at] = round(red as f64 / n as f64) as u32
                        | (round(green as f64 / n as f64) as u32) << 8
                        | (round(blue as f64 / n as f64) as u32) << 16
                        | VISIBLE;
                    confident[at] = true;
                }
                for k in touched {
                    histogram[k] = 0;
                }
            }
        }
        let mut cleaned = backgrounds.clone();
        let mut clean_confidence = confident.clone();
        for ty in 0..tile_rows {
            if self.stopped() {
                return vec![];
            }
            for tx in 0..columns {
                let mut neighbors = vec![];
                for yy in ty.saturating_sub(1)..=(ty + 1).min(tile_rows - 1) {
                    for xx in tx.saturating_sub(1)..=(tx + 1).min(columns - 1) {
                        let at = yy * columns + xx;
                        if confident[at] {
                            neighbors.push(backgrounds[at]);
                        }
                    }
                }
                let mut votes = 0;
                let mut paint = 0;
                for &candidate in &neighbors {
                    let n = neighbors
                        .iter()
                        .filter(|&&p| difference(candidate, p) <= 12)
                        .count();
                    if n > votes {
                        votes = n;
                        paint = candidate;
                    }
                }
                if votes >= 3 {
                    cleaned[ty * columns + tx] = paint;
                    clean_confidence[ty * columns + tx] = true;
                }
            }
        }
        backgrounds = cleaned;
        confident = clean_confidence;
        let mut ink = vec![0u8; self.pixels.len()];
        let mut horizontal = vec![0u8; ink.len()];
        let mut expanded = vec![0u8; ink.len()];
        for y in 0..self.height {
            if self.stopped() {
                return vec![];
            }
            for x in 0..self.width {
                let i = y * self.width + x;
                let t = y / tile * columns + x / tile;
                if confident[t]
                    && self.pixels[i] & VISIBLE != 0
                    && difference(self.pixels[i], backgrounds[t]) >= 20
                {
                    ink[i] = 1;
                }
            }
        }
        let rx = 2usize.max(round(3.0 * self.scale) as usize);
        let ry = 1usize.max(round(1.5 * self.scale) as usize);
        for y in 0..self.height {
            if self.stopped() {
                return vec![];
            }
            let start = y * self.width;
            let mut sum = 0u32;
            for x in 0..=rx.min(self.width - 1) {
                sum += ink[start + x] as u32;
            }
            for x in 0..self.width {
                horizontal[start + x] = u8::from(sum != 0);
                if x >= rx {
                    sum -= ink[start + x - rx] as u32;
                }
                if x + rx + 1 < self.width {
                    sum += ink[start + x + rx + 1] as u32;
                }
            }
        }
        for x in 0..self.width {
            if self.stopped() {
                return vec![];
            }
            let mut sum = 0u32;
            for y in 0..=ry.min(self.height - 1) {
                sum += horizontal[y * self.width + x] as u32;
            }
            for y in 0..self.height {
                expanded[y * self.width + x] = u8::from(sum != 0);
                if y >= ry {
                    sum -= horizontal[(y - ry) * self.width + x] as u32;
                }
                if y + ry + 1 < self.height {
                    sum += horizontal[(y + ry + 1) * self.width + x] as u32;
                }
            }
        }
        self.labels.fill(0);
        let mut pieces = vec![];
        for seed in 0..self.pixels.len() {
            if seed & 4095 == 0 && self.stopped() {
                return vec![];
            }
            if expanded[seed] == 0 || self.labels[seed] != 0 {
                continue;
            }
            let tail = self.flood_mask(seed, &expanded, 1);
            if self.stopped() {
                return vec![];
            }
            let (mut n, mut x0, mut y0, mut x1, mut y1, mut first) =
                (0usize, self.width, self.height, 0, 0, seed);
            for &at in &self.queue[..tail] {
                if ink[at] != 0 {
                    if n == 0 {
                        first = at;
                    }
                    n += 1;
                    let x = at % self.width;
                    let y = at / self.width;
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    y0 = y0.min(y);
                    y1 = y1.max(y);
                }
            }
            // Expanded components always include original ink. Check explicitly
            // before bbox subtraction to keep that invariant safe on any input.
            if n == 0 {
                continue;
            }
            let w = (x1 - x0 + 1) as f64;
            let h = (y1 - y0 + 1) as f64;
            if pieces.len() >= 1600
                || n < 4
                || h < 3.0_f64.max(4.0 * self.scale)
                || h > 14.0_f64.max(60.0 * self.scale)
                || w < h * 0.55
                || w * h > self.pixels.len() as f64 * 0.25
                || (n as f64) / (w * h) < 0.035
                || (n as f64) / (w * h) > 0.82
            {
                continue;
            }
            let base =
                backgrounds[(first / self.width) / tile * columns + (first % self.width) / tile];
            let mut margin = 0;
            let mut good = 0;
            for x in (x0..=x1).step_by(1usize.max((w / 48.0).floor() as usize)) {
                for y in [y0 as isize - 2, y1 as isize + 2] {
                    if y < 0 || y >= self.height as isize {
                        continue;
                    }
                    margin += 1;
                    if difference(self.pixels[y as usize * self.width + x], base) <= 16 {
                        good += 1;
                    }
                }
            }
            if margin == 0 || good as f64 / (margin as f64) < 0.78 {
                continue;
            }
            pieces.push(Line {
                rect: B::new(x0 as f64, y0 as f64, w, h),
                ink: n,
                background: base,
                used: false,
                closed: None,
            });
        }
        struct Band {
            cy: f64,
            height: f64,
            items: Vec<Line>,
        }
        pieces.sort_by(|a, b| cmp(a.rect.cy(), b.rect.cy()).then(cmp(a.rect.x, b.rect.x)));
        let mut bands: Vec<Band> = vec![];
        for piece in pieces {
            if self.stopped() {
                return vec![];
            }
            let mut best = None;
            let mut distance = f64::INFINITY;
            let cy = piece.rect.cy();
            for i in (0..bands.len()).rev().take(48) {
                let band = &bands[i];
                let delta = (cy - band.cy).abs();
                let ratio = piece.rect.height / band.height;
                if delta <= piece.rect.height.max(band.height) * 0.36
                    && (0.4..=2.5).contains(&ratio)
                    && delta < distance
                {
                    best = Some(i);
                    distance = delta;
                }
            }
            if let Some(i) = best {
                let band = &mut bands[i];
                band.height = band.height.max(piece.rect.height);
                band.items.push(piece);
                band.cy = (band.cy * (band.items.len() - 1) as f64 + cy) / band.items.len() as f64;
            } else {
                bands.push(Band {
                    cy,
                    height: piece.rect.height,
                    items: vec![piece],
                });
            }
        }
        let mut lines: Vec<Line> = vec![];
        for band in &mut bands {
            if self.stopped() {
                return vec![];
            }
            band.items.sort_by(|a, b| cmp(a.rect.x, b.rect.x));
            let mut line: Option<Line> = None;
            for p in &band.items {
                if line.as_ref().is_some_and(|l| {
                    p.rect.x - l.rect.right() > 4.0_f64.max(l.rect.height.max(p.rect.height) * 1.8)
                        || difference(p.background, l.background) > 20
                }) {
                    self.finish_line(line.take(), &mut lines);
                }
                if let Some(l) = &mut line {
                    l.rect = l.rect.union(p.rect);
                    l.ink += p.ink;
                } else {
                    line = Some(p.clone());
                }
            }
            self.finish_line(line, &mut lines);
        }
        lines.sort_by(|a, b| cmp(a.rect.y, b.rect.y).then(cmp(a.rect.x, b.rect.x)));
        let mut groups = vec![];
        for i in 0..lines.len() {
            if self.stopped() {
                return vec![];
            }
            if lines[i].closed.is_some() {
                continue;
            }
            if let Some(surface) = self.surface_for(&lines[i], lines[i].rect.height * 4.0, &ink) {
                self.group(
                    surface,
                    0.97,
                    "row",
                    lines[i].rect.height,
                    &[i],
                    "surface-row",
                    &mut lines,
                    &mut groups,
                );
            }
        }
        // Paragraph spacing is tighter than list rhythm. Short final lines are
        // included in the full-width hypothesis rather than their own targets.
        for i in 0..lines.len() {
            if self.stopped() {
                return vec![];
            }
            if lines[i].used || lines[i].closed.is_some() {
                continue;
            }
            let first = &lines[i];
            let mut children = vec![i];
            let mut last = i;
            for _ in 1..12 {
                let mut next = None;
                let mut cost = f64::INFINITY;
                for j in i + 1..lines.len() {
                    let l = &lines[j];
                    let prev = &lines[last];
                    let gap = l.rect.y - prev.rect.bottom();
                    let size = prev.rect.height.max(l.rect.height);
                    if l.rect.y > prev.rect.bottom() + size * 1.3 {
                        break;
                    }
                    let ratio = l.rect.height / first.rect.height;
                    if l.used
                        || l.closed.is_some()
                        || children.contains(&j)
                        || gap < 0.0
                        || gap > size * 1.20
                        || gap > prev.rect.height.min(l.rect.height) * 1.10
                        || (l.rect.x - first.rect.x).abs() > size * 0.65
                        || !(0.55..=1.8).contains(&ratio)
                        || difference(l.background, first.background) > 16
                    {
                        continue;
                    }
                    let value = gap + (l.rect.x - first.rect.x).abs();
                    if value < cost {
                        next = Some(j);
                        cost = value;
                    }
                }
                let Some(next) = next else {
                    break;
                };
                children.push(next);
                last = next;
            }
            if children.len() < 2 {
                continue;
            }
            let mut r = first.rect;
            let mut font = 0.0;
            for &child in &children {
                r = r.union(lines[child].rect);
                font += lines[child].rect.height;
            }
            font /= children.len() as f64;
            self.group(
                self.padded(r, font * 0.52, font * 0.55),
                0.94,
                "paragraph",
                font,
                &children,
                "paragraph",
                &mut lines,
                &mut groups,
            );
        }
        struct Cluster {
            x: f64,
            height: f64,
            background: u32,
            lines: Vec<usize>,
        }
        let mut clusters: Vec<Cluster> = vec![];
        for (i, line) in lines.iter().enumerate() {
            if line.used || line.rect.width < line.rect.height * 3.0 {
                continue;
            }
            let cluster = clusters.iter_mut().find(|c| {
                (c.x - line.rect.x).abs() <= 4.0_f64.max(line.rect.height * 0.65)
                    && (0.55..=1.8).contains(&(line.rect.height / c.height))
                    && difference(line.background, c.background) <= 24
            });
            if let Some(c) = cluster {
                c.lines.push(i);
            } else if clusters.len() < 240 {
                clusters.push(Cluster {
                    x: line.rect.x,
                    height: line.rect.height,
                    background: line.background,
                    lines: vec![i],
                });
            }
        }
        for mut cluster in clusters {
            if self.stopped() {
                return vec![];
            }
            if cluster.lines.len() < 3 {
                continue;
            }
            cluster
                .lines
                .sort_by(|&a, &b| cmp(lines[a].rect.y, lines[b].rect.y));
            let mut heights: Vec<f64> = cluster
                .lines
                .iter()
                .map(|&i| lines[i].rect.height)
                .collect();
            heights.sort_by(|a, b| cmp(*a, *b));
            let font = heights[heights.len() / 2];
            let mut gaps = vec![];
            for pair in cluster.lines.windows(2) {
                let d = lines[pair[1]].rect.cy() - lines[pair[0]].rect.cy();
                if d >= font * 1.75 && d <= font * 4.0 {
                    gaps.push(d);
                }
            }
            if gaps.len() < 2 {
                continue;
            }
            gaps.sort_by(|a, b| cmp(*a, *b));
            let stride = gaps[gaps.len() / 2];
            let regular: Vec<usize> = cluster
                .lines
                .iter()
                .enumerate()
                .filter_map(|(i, &l)| {
                    let cy = lines[l].rect.cy();
                    if (i > 0
                        && (cy - lines[cluster.lines[i - 1]].rect.cy() - stride).abs()
                            <= stride * 0.23)
                        || (i + 1 < cluster.lines.len()
                            && (lines[cluster.lines[i + 1]].rect.cy() - cy - stride).abs()
                                <= stride * 0.23)
                    {
                        Some(l)
                    } else {
                        None
                    }
                })
                .collect();
            if regular.len() < 3 {
                continue;
            }
            let members: Vec<(usize, f64, Vec<usize>, f64)> = regular
                .iter()
                .map(|&title| {
                    let t = &lines[title];
                    let cy = t.rect.cy();
                    let mut attached = vec![title];
                    for (i, l) in lines.iter().enumerate() {
                        if !l.used
                            && i != title
                            && l.rect.x >= t.rect.right()
                            && l.rect.x - t.rect.right() <= font * 6.0
                            && (l.rect.cy() - cy).abs() <= font * 0.38
                            && difference(l.background, t.background) <= 24
                        {
                            attached.push(i);
                        }
                    }
                    let right = attached
                        .iter()
                        .map(|&i| lines[i].rect.right())
                        .fold(f64::NEG_INFINITY, f64::max);
                    (title, cy, attached, right)
                })
                .collect();
            let right = members
                .iter()
                .map(|m| m.3)
                .fold(f64::NEG_INFINITY, f64::max)
                + font * 0.25;
            for (title, cy, attached, _) in members {
                if lines[title].used {
                    continue;
                }
                let mut rect = self.padded(
                    B::new(
                        cluster.x,
                        cy - stride * 0.48,
                        right - cluster.x,
                        stride * 0.96,
                    ),
                    0.0,
                    0.0,
                );
                rect.x = (cluster.x - font * 0.8).floor().max(0.0);
                rect.width = right.ceil().min(self.width as f64) - rect.x;
                let surface = lines[title]
                    .closed
                    .as_ref()
                    .map(|c| c.rect)
                    .or_else(|| self.surface_for(&lines[title], stride * 1.35, &ink));
                let (role, score) = if let Some(surface) = surface {
                    rect = surface;
                    ("surface-row", 0.97)
                } else {
                    ("repeated-row", 0.91)
                };
                self.group(
                    rect,
                    score,
                    "row",
                    font,
                    &attached,
                    role,
                    &mut lines,
                    &mut groups,
                );
            }
        }
        for line in &lines {
            if line.used || line.closed.is_some() {
                continue;
            }
            let pad = 1.0_f64.max(round(2.0 * self.scale));
            let c = self.add(self.padded(line.rect, pad, pad), 0.76, "text", None);
            if let Some(raw) = self.raw.iter_mut().rev().find(|r| r.id == c.id) {
                raw.role = Some("line");
            }
        }
        for c in &mut self.raw {
            for parent in &groups {
                let font = parent.font_height.unwrap_or(0.0);
                if c.id != parent.id
                    && parent.rect.encloses(c.rect)
                    && (c.kind == "text"
                        || (c.kind == "control"
                            && c.template.is_none()
                            && c.rect.height <= font * 1.25
                            && c.rect.width <= font * 1.4))
                {
                    c.parent = Some(parent.id.clone());
                    break;
                }
            }
        }
        groups
    }
    fn finish(&mut self, mut groups: Vec<Hypothesis>) -> Vec<Hypothesis> {
        self.raw.sort_by(order);
        groups.sort_by(order);
        let prioritized = groups.iter().take(120).chain(self.raw.iter());
        let mut result: Vec<Hypothesis> = vec![];
        for c in prioritized {
            if self.stopped() {
                return vec![];
            }
            if c.parent
                .as_ref()
                .is_some_and(|id| groups.iter().any(|g| &g.id == id))
            {
                continue;
            }
            if c.kind == "text"
                && self.raw.iter().any(|r| {
                    r.kind == "control"
                        && r.rect.encloses(c.rect)
                        && r.rect.area() < c.rect.area() * 30.0
                        && r.contains(c.rect.x + c.rect.width / 2.0, c.rect.cy())
                })
            {
                continue;
            }
            if result.iter().any(|r| {
                let a = c.rect;
                let b = r.rect;
                let intersection = a.intersection(b);
                same_contour(c, r)
                    || intersection / (a.area() + b.area() - intersection) >= 0.86
                    || ((a.x - b.x).abs() <= 2.0
                        && (a.y - b.y).abs() <= 2.0
                        && (a.right() - b.right()).abs() <= 2.0
                        && (a.bottom() - b.bottom()).abs() <= 2.0)
            }) {
                continue;
            }
            result.push(c.clone());
            if result.len() == MAX_CANDIDATES {
                break;
            }
        }
        result
    }
}

fn valid_shape(c: &Candidate) -> bool {
    match c.shape.as_deref() {
        None | Some("ellipse" | "capsule") => true,
        Some("polygon") => {
            let Some(points) = &c.outline else {
                return false;
            };
            if !(3..=8).contains(&points.len())
                || points
                    .iter()
                    .flatten()
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            {
                return false;
            }
            let area: f64 = (0..points.len())
                .map(|i| {
                    let p = points[i];
                    let q = points[(i + 1) % points.len()];
                    p[0] as f64 * q[1] as f64 - q[0] as f64 * p[1] as f64
                })
                .sum();
            area.abs() >= 0.02
                && c.shape_name.as_deref().is_none_or(|n| {
                    matches!(
                        n,
                        "triangle" | "diamond" | "hexagon" | "trapezoid" | "parallelogram"
                    )
                })
        }
        _ => false,
    }
}
fn candidate_contains(c: &Candidate, p: Point) -> bool {
    // The public picker needs at most eight normalized vertices. Do not copy
    // arbitrary optional metadata (or allocate) on the hot hover path.
    let mut outline = [[0.0; 2]; 8];
    let len = if c.shape.as_deref() == Some("polygon") {
        let points = c.outline.as_deref().unwrap_or_default();
        if points.len() > outline.len() {
            return false;
        }
        for (to, from) in outline.iter_mut().zip(points) {
            *to = [from[0] as f64, from[1] as f64];
        }
        points.len()
    } else {
        0
    };
    contour_contains(
        c.rect.into(),
        c.shape.as_deref(),
        &outline[..len],
        p.x as f64,
        p.y as f64,
    )
}
fn candidate_order(a: &Candidate, b: &Candidate) -> Ordering {
    cmp(b.score as f64, a.score as f64)
        .then(geometric_order(a.rect.into(), b.rect.into()))
        .then(a.kind.cmp(&b.kind))
        .then(a.id.cmp(&b.id))
}

/// Pick a true-contour target without touching pixels or mutating candidates.
/// Previous is a current-cache ID, not an expanded sticky rectangle. Clear it
/// when replacing the cache: unlike JS's object API, an ID alone cannot verify
/// the geometry of a stale external object. Retention is bounded to 0.07 utility.
pub fn pick(candidates: &[Candidate], point: Point, previous: Option<&str>) -> Option<usize> {
    if !point.x.is_finite() || !point.y.is_finite() || point.x < 0.0 || point.y < 0.0 {
        return None;
    }
    let eligible: Vec<usize> = candidates
        .iter()
        .take(MAX_CANDIDATES)
        .enumerate()
        .filter_map(|(i, c)| {
            let r = B::from(c.rect);
            if !c.score.is_finite()
                || [r.x, r.y, r.width, r.height].iter().any(|v| !v.is_finite())
                || r.x < 0.0
                || r.y < 0.0
                || r.width < 4.0
                || r.height < 4.0
                || !(c.rect.width * c.rect.height).is_finite()
                || !valid_shape(c)
                || !candidate_contains(c, point)
            {
                None
            } else {
                Some(i)
            }
        })
        .collect();
    let grouped = |c: &Candidate| c.kind == "paragraph" || c.kind == "row";
    let mut best = None;
    let mut best_utility = f64::NEG_INFINITY;
    let mut retained = None;
    let mut retained_utility = f64::NEG_INFINITY;
    for &i in &eligible {
        let c = &candidates[i];
        let r = B::from(c.rect);
        let group = grouped(c);
        let mut bonus = if group {
            0.16
        } else if c.kind == "control" {
            0.10
        } else if c.kind == "text" {
            -0.015
        } else {
            0.0
        };
        if c.kind == "control"
            && c.score >= 0.8
            && eligible.iter().any(|&j| {
                let g = &candidates[j];
                grouped(g)
                    && g.font_height
                        .is_some_and(|font| font.is_finite() && r.height >= font as f64 * 1.35)
                    && r.area() < B::from(g.rect).area() * 0.65
                    && B::from(g.rect).encloses(r)
            })
        {
            bonus += 0.22;
        }
        let utility = (c.score as f64).clamp(0.0, 1.0) + bonus
            - if group { 0.035 } else { 0.085 } * (r.area() / 768.0).max(1.0).log2();
        if utility > best_utility
            || (utility == best_utility
                && best.is_none_or(|j| candidate_order(c, &candidates[j]) == Ordering::Less))
        {
            best = Some(i);
            best_utility = utility;
        }
        if previous == Some(c.id.as_str()) {
            retained = Some(i);
            retained_utility = utility;
        }
    }
    if retained.is_some() && retained_utility >= best_utility - 0.07 {
        retained
    } else {
        best
    }
}
