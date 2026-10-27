#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use image::{RgbaImage, imageops::FilterType};
use image_feedback_slint::{
    analysis::AnalysisService,
    delivery::{self, PreparedFeedback},
    document::{AnnotationKind, Document, History, Point, Rect},
    smart_selection::{self, Candidate},
    storage::{self, Asset},
};
use slint::winit_030::{EventResult as NativeEventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, mpsc},
    time::Duration,
};
slint::include_modules!();
mod copy_control;
mod delivery_smoke;
mod dialog_smoke;
#[cfg(test)]
mod editor_tests;
mod platform;
mod resident;
mod smoke_ready;

#[derive(Clone, Copy, Default)]
struct View {
    scale: f32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    fit: bool,
}
#[derive(Clone)]
enum Gesture {
    Select {
        start: Point,
        candidate: Option<Rect>,
        manual: bool,
    },
    Move {
        id: u64,
        start: Point,
        dx: f32,
        dy: f32,
    },
    Pan {
        x: f32,
        y: f32,
        origin_x: f32,
        origin_y: f32,
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum CopyRequest {
    Feedback,
    OriginalPath,
    Base64,
}
#[derive(Clone, Debug)]
struct DeliveryCache {
    snapshot: Document,
    package: PreparedFeedback,
    source: Option<PathBuf>,
}
enum Work {
    Loaded(Result<(Document, Asset, Option<PathBuf>, RgbaImage), String>),
    Saved(Result<PathBuf, String>, Document),
    Exported(Result<DeliveryCache, String>),
    Delivered(
        Result<(DeliveryCache, Result<String, String>), String>,
        CopyRequest,
    ),
}
#[derive(Clone, Debug, PartialEq)]
enum PendingAction {
    New,
    Close,
    Load { path: PathBuf, project: bool },
    Demo { annotated: bool },
    Paste,
}
#[derive(Clone, Debug)]
struct SaveConfirmation {
    action: PendingAction,
    saving: bool,
    error: String,
}
#[derive(Clone, Debug)]
enum MessageAction {
    Dismiss,
    Remove(u64),
    ClearFeedback,
    CloseNotice,
}
#[derive(Clone, Debug)]
struct AppMessage {
    title: String,
    description: String,
    name: String,
    primary: String,
    destructive: bool,
    action: MessageAction,
}
struct Editor {
    history: Option<History>,
    asset: Option<Arc<Asset>>,
    project: Option<PathBuf>,
    saved: Option<Document>,
    selected: Option<u64>,
    pending: Option<(u64, String)>,
    gesture: Option<Gesture>,
    view: View,
    tool: i32,
    preview_scale: f32,
    preview_scale_y: f32,
    worker: Option<mpsc::Receiver<Work>>,
    confirmation: Option<SaveConfirmation>,
    message: Option<AppMessage>,
    smart_enabled: bool,
    analysis: Option<AnalysisService>,
    analysis_generation: u64,
    analysis_loading: bool,
    candidates: Vec<Candidate>,
    eligible: Vec<Candidate>,
    ownership: Vec<(Rect, Rect)>,
    hover: Option<String>,
    hover_point: Option<Point>,
    pointer_move_events: u64,
    close_requests: u64,
    clipboard_probe: bool,
    clipboard_written: Option<String>,
    clipboard_error: Option<String>,
    delivery: Option<DeliveryCache>,
    delivery_root: Option<PathBuf>,
    opened_delivery: Option<PathBuf>,
    revealed_selection: Option<u64>,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            history: None,
            asset: None,
            project: None,
            saved: None,
            selected: None,
            pending: None,
            gesture: None,
            view: View {
                scale: 1.,
                fit: true,
                ..Default::default()
            },
            tool: 0,
            preview_scale: 1.,
            preview_scale_y: 1.,
            worker: None,
            confirmation: None,
            message: None,
            smart_enabled: true,
            analysis: None,
            analysis_generation: 0,
            analysis_loading: false,
            candidates: Vec::new(),
            eligible: Vec::new(),
            ownership: Vec::new(),
            hover: None,
            hover_point: None,
            pointer_move_events: 0,
            close_requests: 0,
            clipboard_probe: false,
            clipboard_written: None,
            clipboard_error: None,
            delivery: None,
            delivery_root: None,
            opened_delivery: None,
            revealed_selection: None,
        }
    }
}
impl Editor {
    fn clear_feedback(&mut self) -> Result<(), String> {
        self.flush()?;
        if let Some(history) = self.history.as_mut() {
            history.apply(|doc| {
                doc.annotations.clear();
                Ok(())
            })?;
        }
        self.selected = None;
        self.revealed_selection = None;
        self.gesture = None;
        self.hover = None;
        self.hover_point = None;
        self.delivery = None;
        Ok(())
    }
    fn new_project(&mut self) {
        if let Some(analysis) = self.analysis.as_ref() {
            analysis.cancel();
        }
        self.history = None;
        self.asset = None;
        self.project = None;
        self.saved = None;
        self.selected = None;
        self.pending = None;
        self.gesture = None;
        self.view = View {
            scale: 1.,
            fit: true,
            ..Default::default()
        };
        self.tool = 0;
        self.preview_scale = 1.;
        self.preview_scale_y = 1.;
        self.analysis_loading = false;
        self.candidates.clear();
        self.eligible.clear();
        self.ownership.clear();
        self.hover = None;
        self.hover_point = None;
        self.delivery = None;
        self.revealed_selection = None;
    }
    fn doc(&self) -> Option<&Document> {
        self.history.as_ref().map(History::document)
    }
    fn dirty(&self) -> bool {
        self.pending.as_ref().is_some_and(|(id, text)| {
            self.doc()
                .and_then(|d| d.annotation(*id))
                .is_some_and(|n| n.text != *text)
        }) || self.doc().is_some_and(|d| self.saved.as_ref() != Some(d))
    }
    fn flush(&mut self) -> Result<(), String> {
        if let Some((id, text)) = self.pending.take() {
            if let Some(h) = self.history.as_mut() {
                if let Err(err) = h.apply(|d| d.set_text(id, text.clone())) {
                    self.pending = Some((id, text));
                    return Err(err);
                }
            }
        }
        Ok(())
    }
    fn request_action(&mut self, action: PendingAction) -> Option<PendingAction> {
        if self.worker.is_some() || self.confirmation.is_some() || self.message.is_some() {
            return None;
        }
        if self.dirty() {
            self.gesture = None;
            self.confirmation = Some(SaveConfirmation {
                action,
                saving: false,
                error: String::new(),
            });
            None
        } else {
            Some(action)
        }
    }
    fn cancel_confirmation(&mut self) {
        if self.confirmation.as_ref().is_some_and(|c| !c.saving) {
            self.confirmation = None;
        }
    }
    fn discard_confirmation(&mut self) -> Option<PendingAction> {
        if self.confirmation.as_ref().is_some_and(|c| !c.saving) {
            self.confirmation.take().map(|c| c.action)
        } else {
            None
        }
    }
    fn start_confirmed_save(&mut self) -> bool {
        let Some(confirmation) = self.confirmation.as_mut() else {
            return false;
        };
        if confirmation.saving || self.worker.is_some() {
            return false;
        }
        confirmation.saving = true;
        confirmation.error.clear();
        true
    }
    fn save_problem(&mut self, error: impl Into<String>) {
        if let Some(c) = self.confirmation.as_mut() {
            c.saving = false;
            c.error = error.into();
        }
    }
    fn saved_successfully(&mut self, path: PathBuf, snapshot: Document) -> Option<PendingAction> {
        self.project = Some(path);
        self.saved = Some(snapshot);
        if !self.confirmation.as_ref().is_some_and(|c| c.saving) {
            return None;
        }
        if self.dirty() {
            self.save_problem("保存期间出现了新的修改，请再次保存或取消返回编辑。");
            None
        } else {
            self.confirmation.take().map(|c| c.action)
        }
    }
    fn point(&self, x: f32, y: f32) -> Point {
        Point {
            x: (x - self.view.x) / self.view.scale,
            y: (y - self.view.y) / self.view.scale,
        }
    }
    fn inside(&self, p: Point) -> bool {
        self.doc().is_some_and(|d| {
            p.x >= 0. && p.y >= 0. && p.x <= d.image.width as f32 && p.y <= d.image.height as f32
        })
    }
    fn fit(&mut self, w: f32, h: f32) {
        if let Some(doc) = self.doc() {
            let scale = ((w - 56.).max(20.) / doc.image.width as f32)
                .min((h - 56.).max(20.) / doc.image.height as f32)
                .min(2.);
            self.view = View {
                scale,
                x: (w - doc.image.width as f32 * scale) / 2.,
                y: (h - doc.image.height as f32 * scale) / 2.,
                width: w,
                height: h,
                fit: true,
            };
        }
    }
    fn hit(&self, p: Point) -> Option<u64> {
        let doc = self.doc()?;
        let tolerance = 13. / self.view.scale;
        let anchor = |a: &image_feedback_slint::document::Annotation| match a.kind {
            AnnotationKind::Point { point } => Some(point),
            AnnotationKind::Region { rect } => Some(Point {
                x: rect.x,
                y: rect.y,
            }),
            AnnotationKind::Block { target, .. } => Some(Point {
                x: target.x,
                y: target.y,
            }),
            AnnotationKind::Global => None,
        };
        if let Some(a) = doc
            .annotations
            .iter()
            .rev()
            .find(|a| anchor(a).is_some_and(|q| (q.x - p.x).hypot(q.y - p.y) <= tolerance))
        {
            return Some(a.id);
        }
        // Preserve an explicitly selected region's drag intent; otherwise moved blocks win
        // over unrelated transparent annotations (the original plugin regression contract).
        if let Some(a) = self.selected.and_then(|id| doc.annotation(id)) {
            if matches!(a.kind,AnnotationKind::Region{rect} if contains(rect,p)) {
                return Some(a.id);
            }
        }
        if let Some(a) = doc
            .annotations
            .iter()
            .rev()
            .find(|a| matches!(a.kind,AnnotationKind::Block{target,..} if contains(target,p)))
        {
            return Some(a.id);
        }
        doc.annotations
            .iter()
            .rev()
            .find(|a| matches!(a.kind,AnnotationKind::Region{rect} if contains(rect,p)))
            .map(|a| a.id)
    }
    fn bounded_delta(&self, id: u64, dx: f32, dy: f32) -> (f32, f32) {
        let Some(d) = self.doc() else { return (0., 0.) };
        let Some(n) = d.annotation(id) else {
            return (0., 0.);
        };
        let r = match n.kind {
            AnnotationKind::Region { rect } => rect,
            AnnotationKind::Block { target, .. } => target,
            AnnotationKind::Point { point } => Rect {
                x: point.x,
                y: point.y,
                width: 0.,
                height: 0.,
            },
            AnnotationKind::Global => return (0., 0.),
        };
        (
            clamped_axis(r.x, r.width, d.image.width, dx),
            clamped_axis(r.y, r.height, d.image.height, dy),
        )
    }
    fn update_eligibility(&mut self, force: bool) -> bool {
        let blocks: Vec<_> = self
            .doc()
            .map(|d| {
                d.annotations
                    .iter()
                    .filter_map(|n| {
                        if let AnnotationKind::Block { source, target } = n.kind {
                            Some((source, target))
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !force && blocks == self.ownership {
            return false;
        }
        self.ownership = blocks;
        self.eligible =
            self.candidates
                .iter()
                .filter(|c| {
                    !self.ownership.iter().any(|(source, target)| {
                        overlap(c.rect, *source) || overlap(c.rect, *target)
                    })
                })
                .cloned()
                .collect();
        self.hover = None;
        true
    }
    fn candidate_at(&self, p: Point) -> Option<Candidate> {
        if !self.smart_enabled
            || self.tool != 0
            || self.worker.is_some()
            || self.gesture.is_some()
            || !self.inside(p)
            || self.hit(p).is_some()
        {
            return None;
        }
        smart_selection::pick(&self.eligible, p, self.hover.as_deref())
            .map(|i| self.eligible[i].clone())
    }
}
fn selection_result(
    start: Point,
    end: Point,
    scale: f32,
    candidate: Option<Rect>,
    manual: bool,
) -> Option<Rect> {
    let dx = (end.x - start.x).abs();
    let dy = (end.y - start.y).abs();
    if dx * scale >= 4. && dy * scale >= 4. {
        let x = start.x.min(end.x).round();
        let y = start.y.min(end.y).round();
        let rect = Rect {
            x,
            y,
            width: start.x.max(end.x).round() - x,
            height: start.y.max(end.y).round() - y,
        };
        if rect.width >= 2. && rect.height >= 2. {
            return Some(rect);
        }
    }
    if !manual && dx.hypot(dy) * scale < 4. {
        candidate
    } else {
        None
    }
}
fn overlap(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}
fn ui_note_id(e: &Editor, token: i32) -> Option<u64> {
    if token < 1 {
        return None;
    }
    e.doc()?.annotations.get((token - 1) as usize).map(|a| a.id)
}
fn ui_token(e: &Editor, id: u64) -> i32 {
    e.doc()
        .and_then(|d| d.annotations.iter().position(|a| a.id == id))
        .map(|i| i as i32 + 1)
        .unwrap_or(0)
}
fn clamped_axis(origin: f32, size: f32, bound: u32, delta: f32) -> f32 {
    if !delta.is_finite() {
        return 0.;
    }
    let upper = (bound as f64 - size as f64).max(0.);
    let mut target = (origin as f64 + delta as f64).clamp(0., upper) as f32;
    if target as f64 > upper {
        target = target.next_down();
    }
    let mut result = target - origin;
    // Subtraction followed by addition can round outward for imported fractional geometry.
    for _ in 0..8 {
        let actual = origin + result;
        if actual >= 0. && actual as f64 + size as f64 <= bound as f64 {
            return result;
        }
        result = if actual < 0. {
            result.next_up()
        } else {
            result.next_down()
        };
    }
    0.
}
fn contains(r: Rect, p: Point) -> bool {
    p.x >= r.x && p.y >= r.y && p.x <= r.x + r.width && p.y <= r.y + r.height
}
fn set_status(ui: &AppWindow, text: impl AsRef<str>) {
    ui.set_status_text(text.as_ref().into());
}
fn smart_status(e: &Editor) -> String {
    if !e.smart_enabled {
        "手动模式 · Shift 可重新框选，空格平移".into()
    } else if e.analysis_loading {
        "本地分析中 · 手动框选始终可用".into()
    } else if e.eligible.is_empty() {
        "无推荐范围 · 可直接手动框选".into()
    } else {
        format!(
            "{} 个推荐范围 · 悬停预览，单击采用，拖动手绘",
            e.eligible.len()
        )
    }
}
fn show_toast(ui: &AppWindow, timer: &slint::Timer, success: bool, title: &str, message: &str) {
    ui.set_toast_success(success);
    ui.set_toast_title(title.into());
    ui.set_toast_message(message.into());
    ui.set_toast_visible(true);
    ui.set_copy_complete(success);
    let weak = ui.as_weak();
    timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_secs(if success { 4 } else { 7 }),
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_visible(false);
                ui.set_copy_complete(false);
            }
        },
    );
}
fn feedback_payload(e: &mut Editor) -> Result<(String, usize, usize), String> {
    e.flush()?;
    let doc = e.doc().ok_or("请先导入图片并添加修改意见")?;
    if doc.annotations.is_empty() {
        return Err("还没有可复制的修改意见".into());
    }
    let moved = doc
        .annotations
        .iter()
        .filter(|n| matches!(n.kind, AnnotationKind::Block { .. }))
        .count();
    Ok((storage::feedback_text(doc), doc.annotations.len(), moved))
}
fn write_feedback_clipboard(e: &mut Editor, text: &str) -> Result<(), String> {
    // GUI regression tests must not overwrite the user's system clipboard.
    if e.clipboard_probe {
        if let Some(error) = e.clipboard_error.as_ref() {
            return Err(error.clone());
        }
        e.clipboard_written = Some(text.to_owned());
        return Ok(());
    }
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text))
        .map_err(|error| error.to_string())
}
fn clear_hover(ui: &AppWindow, e: &mut Editor) {
    e.hover = None;
    e.hover_point = None;
    ui.set_hover_visible(false);
    ui.set_smart_status(smart_status(e).into());
}
fn begin_analysis(e: &mut Editor) {
    if !e.smart_enabled {
        return;
    }
    let Some(asset) = e.asset.clone() else {
        return;
    };
    let service = e.analysis.get_or_insert_with(AnalysisService::new);
    e.analysis_generation = service.submit(asset);
    e.analysis_loading = true;
}
fn outline_svg(candidate: &Candidate, scale: f32) -> String {
    let r = candidate.rect;
    let pad = 1. / scale;
    let w = r.width;
    let h = r.height;
    let geometry = match candidate.shape.as_deref() {
        Some("ellipse") => format!(
            "<ellipse cx='{}' cy='{}' rx='{}' ry='{}'/>",
            w / 2.,
            h / 2.,
            (w / 2. - pad).max(0.1),
            (h / 2. - pad).max(0.1)
        ),
        Some("capsule") => format!(
            "<rect x='{pad}' y='{pad}' width='{}' height='{}' rx='{}'/>",
            (w - 2. * pad).max(0.1),
            (h - 2. * pad).max(0.1),
            w.min(h) / 2.
        ),
        Some("polygon") => format!(
            "<polygon points='{}'/>",
            candidate
                .outline
                .as_ref()
                .map(|points| points
                    .iter()
                    .map(|p| format!(
                        "{},{}",
                        pad + p[0] * (w - 2. * pad),
                        pad + p[1] * (h - 2. * pad)
                    ))
                    .collect::<Vec<_>>()
                    .join(" "))
                .unwrap_or_default()
        ),
        _ => format!(
            "<rect x='{pad}' y='{pad}' width='{}' height='{}'/>",
            (w - 2. * pad).max(0.1),
            (h - 2. * pad).max(0.1)
        ),
    };
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{w}' height='{h}' viewBox='0 0 {w} {h}'><g fill='#dc5147' fill-opacity='.055' stroke='#dc5147' stroke-width='{}' stroke-dasharray='{} {}'>{geometry}</g></svg>",
        1.5 / scale,
        5. / scale,
        3. / scale
    )
}
fn update_hover(ui: &AppWindow, e: &mut Editor, p: Point) {
    e.hover_point = Some(p);
    let candidate = if ui.get_shift_held() || ui.get_space_held() {
        None
    } else {
        e.candidate_at(p)
    };
    let Some(candidate) = candidate else {
        e.hover = None;
        ui.set_hover_visible(false);
        ui.set_smart_status(smart_status(e).into());
        return;
    };
    if e.hover.as_deref() != Some(candidate.id.as_str()) {
        let svg = outline_svg(&candidate, e.view.scale);
        let Ok(image) = slint::Image::load_from_svg_data(svg.as_bytes()) else {
            e.hover = None;
            ui.set_hover_visible(false);
            return;
        };
        ui.set_hover_image(image);
        e.hover = Some(candidate.id.clone());
    }
    ui.set_hover_x(e.view.x + candidate.rect.x * e.view.scale);
    ui.set_hover_y(e.view.y + candidate.rect.y * e.view.scale);
    ui.set_hover_w(candidate.rect.width * e.view.scale);
    ui.set_hover_h(candidate.rect.height * e.view.scale);
    ui.set_hover_visible(true);
    let label = if candidate.kind == "paragraph" {
        "整段文字"
    } else if candidate.kind == "row" || candidate.role.as_deref() == Some("row") {
        "完整列表行"
    } else if candidate.shape.is_some() {
        "形状轮廓（确认后取矩形范围）"
    } else {
        "图片区块"
    };
    ui.set_smart_status(format!("{label} · 单击采用，拖动自行框选").into());
}
fn preview(asset: &Asset) -> RgbaImage {
    let (w, h) = asset.pixels.dimensions();
    let s = (2048. / w as f32).min(1600. / h as f32).min(1.);
    if s < 1. {
        image::imageops::resize(
            &asset.pixels,
            (w as f32 * s).round() as u32,
            (h as f32 * s).round() as u32,
            FilterType::Triangle,
        )
    } else {
        asset.pixels.clone()
    }
}
fn sync_confirmation(ui: &AppWindow, e: &Editor) {
    ui.set_confirm_cancel_label("取消".into());
    ui.set_confirm_discard_label("不保存".into());
    if let Some(message) = e.message.as_ref() {
        ui.set_confirm_title(message.title.clone().into());
        ui.set_confirm_description(message.description.clone().into());
        ui.set_confirm_name(message.name.clone().into());
        ui.set_confirm_save_label(message.primary.clone().into());
        ui.set_confirm_show_cancel(matches!(
            message.action,
            MessageAction::Remove(_) | MessageAction::ClearFeedback
        ));
        ui.set_confirm_show_discard(false);
        ui.set_confirm_destructive(message.destructive);
        ui.set_confirm_footnote(
            if matches!(message.action, MessageAction::Remove(_)) {
                "移除后可通过撤销恢复。"
            } else if matches!(message.action, MessageAction::ClearFeedback) {
                "清空后可按 Ctrl/⌘+Z 一次恢复全部内容。"
            } else {
                ""
            }
            .into(),
        );
        ui.set_confirm_error("".into());
        ui.set_confirm_saving(false);
        ui.set_confirm_visible(true);
    } else if let Some(c) = e.confirmation.as_ref() {
        ui.set_confirm_show_cancel(true);
        ui.set_confirm_show_discard(true);
        ui.set_confirm_destructive(false);
        ui.set_confirm_footnote("保存为可继续编辑的项目，不会覆盖原图。".into());
        let closing = matches!(c.action, PendingAction::Close);
        ui.set_confirm_title(
            if closing {
                "保存修改后再关闭？"
            } else if matches!(c.action, PendingAction::New) {
                "新建前保存当前项目？"
            } else {
                "先保存当前修改？"
            }
            .into(),
        );
        ui.set_confirm_description(
            if closing {
                "未保存的批注和位置调整将丢失。保存后，可随时回来继续编辑。"
            } else if matches!(c.action, PendingAction::New) {
                "新建后将返回空白工作区。请先保存当前批注和位置调整，或选择不保存。"
            } else {
                "即将替换当前图片或项目。请先保存，或明确选择不保存。"
            }
            .into(),
        );
        ui.set_confirm_name(
            e.doc()
                .map(|d| d.image.file_name.as_str())
                .unwrap_or("当前项目")
                .into(),
        );
        ui.set_confirm_error(c.error.clone().into());
        ui.set_confirm_saving(c.saving);
        ui.set_confirm_save_label(
            if closing {
                "保存并关闭"
            } else if matches!(c.action, PendingAction::New) {
                "保存并新建"
            } else {
                "保存并继续"
            }
            .into(),
        );
        ui.set_confirm_visible(true);
    } else {
        ui.set_confirm_visible(false);
        ui.set_confirm_saving(false);
        ui.set_confirm_error("".into());
    }
}
fn sync(ui: &AppWindow, e: &mut Editor) {
    sync_confirmation(ui, e);
    ui.set_delivery_ready(e.delivery.is_some());
    ui.set_smart_enabled(e.smart_enabled);
    if e.update_eligibility(false) || e.gesture.is_some() {
        e.hover = None;
        ui.set_hover_visible(false);
    }
    ui.set_smart_status(smart_status(e).into());
    let width = ui.get_viewport_width();
    let height = ui.get_viewport_height();
    if e.view.fit && ((width - e.view.width).abs() > 0.5 || (height - e.view.height).abs() > 0.5) {
        e.fit(width, height);
    }
    let Some(doc) = e.doc() else {
        ui.set_loaded(false);
        ui.set_source_image(slint::Image::default());
        ui.set_image_name("尚未导入图片".into());
        ui.set_dirty(false);
        ui.set_can_undo(false);
        ui.set_can_redo(false);
        ui.set_note_count(0);
        ui.set_notes(ModelRc::default());
        ui.set_marks(ModelRc::default());
        ui.set_active_note_token(0);
        ui.set_smart_status("智能预选 · 载入图片后准备".into());
        ui.set_tool(e.tool);
        ui.set_zoom_label("100%".into());
        return;
    };
    ui.set_loaded(true);
    ui.set_image_name(doc.image.file_name.clone().into());
    ui.set_dirty(e.dirty());
    ui.set_can_undo(e.history.as_ref().is_some_and(History::can_undo));
    ui.set_can_redo(e.history.as_ref().is_some_and(History::can_redo));
    ui.set_tool(e.tool);
    ui.set_zoom_label(format!("{:.0}%", e.view.scale * 100.).into());
    ui.set_note_count(doc.annotations.len() as i32);
    ui.set_canvas_x(e.view.x);
    ui.set_canvas_y(e.view.y);
    ui.set_canvas_w(doc.image.width as f32 * e.view.scale);
    ui.set_canvas_h(doc.image.height as f32 * e.view.scale);
    let refresh_notes = e.gesture.is_none();
    let mut marks = Vec::new();
    let mut notes = Vec::new();
    for (index, a) in doc.annotations.iter().enumerate() {
        let selected = e.selected == Some(a.id);
        let (kind, detail, r, source, point, block) = match a.kind {
            AnnotationKind::Region { rect } => (
                "范围批注",
                format!(
                    "{:.0}, {:.0} · {:.0} × {:.0}",
                    rect.x, rect.y, rect.width, rect.height
                ),
                rect,
                rect,
                false,
                false,
            ),
            AnnotationKind::Point { point } => (
                "点批注",
                format!("位置 {:.0}, {:.0}", point.x, point.y),
                Rect {
                    x: point.x,
                    y: point.y,
                    width: 0.,
                    height: 0.,
                },
                Rect {
                    x: 0.,
                    y: 0.,
                    width: 0.,
                    height: 0.,
                },
                true,
                false,
            ),
            AnnotationKind::Block { source, target } => (
                "图块移动",
                format!(
                    "位移 x {:+.0} · y {:+.0} px",
                    target.x - source.x,
                    target.y - source.y
                ),
                target,
                source,
                false,
                true,
            ),
            AnnotationKind::Global => (
                "全局意见",
                String::new(),
                Rect {
                    x: 0.,
                    y: 0.,
                    width: 0.,
                    height: 0.,
                },
                Rect {
                    x: 0.,
                    y: 0.,
                    width: 0.,
                    height: 0.,
                },
                false,
                false,
            ),
        };
        if refresh_notes {
            let text = e
                .pending
                .as_ref()
                .filter(|(id, _)| *id == a.id)
                .map(|(_, text)| text.clone())
                .unwrap_or_else(|| a.text.clone());
            notes.push(NoteRow {
                id: index as i32 + 1,
                number: index as i32 + 1,
                kind_label: kind.into(),
                text: text.into(),
                selected,
                detail: detail.into(),
            });
        }
        if !matches!(a.kind, AnnotationKind::Global) {
            let mut r = r;
            let mut moving = false;
            if let Some(Gesture::Move { id, dx, dy, .. }) = e.gesture {
                if id == a.id {
                    r.x += dx;
                    r.y += dy;
                    moving = dx.hypot(dy) * e.view.scale >= 2.;
                }
            }
            marks.push(Mark {
                id: index as i32 + 1,
                number: index as i32 + 1,
                x: e.view.x + r.x * e.view.scale,
                y: e.view.y + r.y * e.view.scale,
                w: r.width * e.view.scale,
                h: r.height * e.view.scale,
                is_point: point,
                selected,
                is_block: block || moving,
                source_x: source.x * e.preview_scale,
                source_y: source.y * e.preview_scale_y,
                source_w: source.width * e.preview_scale,
                source_h: source.height * e.preview_scale_y,
            });
        }
    }
    // Preserve the text editor instance/caret when only canvas geometry changes.
    if refresh_notes && ui.get_notes().iter().collect::<Vec<_>>() != notes {
        ui.set_notes(ModelRc::new(VecModel::from(notes)));
    }
    ui.set_marks(ModelRc::new(VecModel::from(marks)));
    if refresh_notes {
        let token = e.selected.map(|id| ui_token(e, id)).unwrap_or(0);
        ui.set_active_note_token(token);
        if e.revealed_selection != e.selected {
            e.revealed_selection = e.selected;
            if token > 0 {
                let weak = ui.as_weak();
                slint::Timer::single_shot(Duration::ZERO, move || {
                    if let Some(ui) = weak.upgrade() {
                        if ui.get_active_note_token() == token {
                            ui.invoke_reveal_note(token);
                        }
                    }
                });
            }
        }
    }
}
fn perform_action(ui: &AppWindow, e: &mut Editor, timer: &Rc<slint::Timer>, action: PendingAction) {
    match action {
        PendingAction::New => {
            e.new_project();
            ui.set_preview_visible(false);
            ui.set_hover_visible(false);
            ui.set_toast_visible(false);
            ui.set_copy_complete(false);
            ui.set_space_held(false);
            ui.set_shift_held(false);
            sync(ui, e);
            ui.invoke_focus_canvas();
            set_status(ui, "新项目已就绪 · 导入或粘贴图片开始反馈");
        }
        PendingAction::Close => {
            let _ = ui.hide();
        }
        PendingAction::Load { path, project } => load_path_unchecked(ui, e, timer, path, project),
        PendingAction::Demo { annotated } => load_demo_unchecked(ui, e, timer, annotated),
        PendingAction::Paste => paste_image_unchecked(ui, e, timer),
    }
}
fn begin_action(ui: &AppWindow, e: &mut Editor, timer: &Rc<slint::Timer>, action: PendingAction) {
    if e.worker.is_some() {
        set_status(ui, "请等待当前任务完成后再继续");
        return;
    }
    if let Some(action) = e.request_action(action) {
        perform_action(ui, e, timer, action);
    } else {
        clear_hover(ui, e);
        ui.set_preview_visible(false);
        ui.set_toast_visible(false);
        sync_confirmation(ui, e);
    }
}
fn confirmation_response(ui: &AppWindow, e: &mut Editor, timer: &Rc<slint::Timer>, response: i32) {
    if let Some(message) = e.message.take() {
        match message.action {
            MessageAction::Remove(id) if response == 2 => remove_annotation(ui, e, id),
            MessageAction::ClearFeedback if response == 2 => {
                if let Err(error) = e.clear_feedback() {
                    show_message(ui, e, "无法清空当前反馈", error);
                    return;
                }
                ui.set_preview_visible(false);
                ui.set_hover_visible(false);
                ui.set_copy_complete(false);
                ui.invoke_focus_canvas();
                set_status(ui, "已清空当前反馈 · 图片保留，可撤销恢复");
            }
            MessageAction::CloseNotice => {
                let _ = ui.hide();
            }
            _ => {}
        }
        sync(ui, e);
        return;
    }
    match response {
        0 => e.cancel_confirmation(),
        1 => {
            if let Some(action) = e.discard_confirmation() {
                sync_confirmation(ui, e);
                perform_action(ui, e, timer, action);
                return;
            }
        }
        2 => {
            if e.start_confirmed_save() {
                sync_confirmation(ui, e);
                save(ui, e, timer);
                return;
            }
        }
        _ => {}
    }
    sync_confirmation(ui, e);
}
fn show_message(ui: &AppWindow, e: &mut Editor, title: &str, description: impl Into<String>) {
    e.message = Some(AppMessage {
        title: title.into(),
        description: description.into(),
        name: String::new(),
        primary: "知道了".into(),
        destructive: false,
        action: MessageAction::Dismiss,
    });
    clear_hover(ui, e);
    ui.set_preview_visible(false);
    ui.set_toast_visible(false);
    sync_confirmation(ui, e);
}
fn remove_annotation(ui: &AppWindow, e: &mut Editor, id: u64) {
    if let Some(history) = e.history.as_mut() {
        if let Err(error) = history.apply(|doc| doc.remove(id)) {
            show_message(ui, e, "无法移除批注", error);
            return;
        }
    }
    if e.selected == Some(id) {
        e.selected = None;
    }
    if e.pending
        .as_ref()
        .is_some_and(|(pending, _)| *pending == id)
    {
        e.pending = None;
    }
}
fn request_remove(ui: &AppWindow, e: &mut Editor, id: u64) {
    if e.worker.is_some() || e.confirmation.is_some() || e.message.is_some() {
        return;
    }
    if let Err(error) = e.flush() {
        show_message(ui, e, "请先处理当前文字", error);
        return;
    }
    let Some(annotation) = e.doc().and_then(|d| d.annotation(id)) else {
        return;
    };
    if annotation.text.is_empty() {
        remove_annotation(ui, e, id);
    } else {
        e.message = Some(AppMessage {
            title: "移除这条批注？".into(),
            description: "标记和对应的文字意见会一起移除。此操作可以撤销。".into(),
            name: format!(
                "批注 {} · {}",
                ui_token(e, id),
                e.doc().unwrap().image.file_name
            ),
            primary: "移除批注".into(),
            destructive: true,
            action: MessageAction::Remove(id),
        });
        clear_hover(ui, e);
        ui.set_toast_visible(false);
    }
    sync(ui, e);
}
fn install(
    ui: &AppWindow,
    e: &mut Editor,
    doc: Document,
    asset: Asset,
    path: Option<PathBuf>,
    thumb: RgbaImage,
) {
    let pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        thumb.as_raw(),
        thumb.width(),
        thumb.height(),
    );
    ui.set_source_image(slint::Image::from_rgba8(pixels));
    e.preview_scale = thumb.width() as f32 / asset.pixels.width() as f32;
    e.preview_scale_y = thumb.height() as f32 / asset.pixels.height() as f32;
    e.saved = Some(doc.clone());
    e.history = Some(History::new(doc));
    e.asset = Some(Arc::new(asset));
    e.project = path;
    e.delivery = None;
    e.revealed_selection = None;
    e.selected = None;
    e.pending = None;
    e.gesture = None;
    e.view.fit = true;
    e.fit(ui.get_viewport_width(), ui.get_viewport_height());
    e.candidates = Vec::new();
    e.eligible = Vec::new();
    e.ownership = Vec::new();
    e.analysis_loading = false;
    clear_hover(ui, e);
    begin_analysis(e);
    ui.set_preview_visible(false);
    sync(ui, e);
    set_status(ui, "图片已就绪 · 拖动框选，直接写下修改意见");
}
fn start_work(
    ui: &AppWindow,
    e: &mut Editor,
    timer: &Rc<slint::Timer>,
    task: impl FnOnce() -> Work + Send + 'static,
) {
    if e.worker.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    e.worker = Some(rx);
    ui.set_busy(true);
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
        if let Ok(result) = result {
            let _ = tx.send(result);
        }
    });
    // Timer is started by the caller after the callback has been installed at startup.
    timer.restart();
}
fn save_problem(ui: &AppWindow, e: &mut Editor, error: impl Into<String>) {
    let error = error.into();
    if e.confirmation.is_some() {
        e.save_problem(error.clone());
        sync_confirmation(ui, e);
    } else {
        show_message(ui, e, "保存未完成", error.clone());
    }
    set_status(ui, format!("保存未完成：{error}"));
}
fn save(ui: &AppWindow, e: &mut Editor, timer: &Rc<slint::Timer>) {
    if e.worker.is_some() {
        return;
    }
    if let Err(err) = e.flush() {
        save_problem(ui, e, err);
        return;
    }
    let (Some(doc), Some(asset)) = (e.doc().cloned(), e.asset.clone()) else {
        save_problem(ui, e, "没有可保存的图片数据");
        return;
    };
    let path = match e.project.clone() {
        Some(p) => p,
        None => {
            let Some(p) = rfd::FileDialog::new()
                .set_title("保存可恢复项目")
                .add_filter("图片反馈项目", &["ifp"])
                .set_file_name("图片反馈.ifp")
                .save_file()
            else {
                if e.confirmation.is_some() {
                    save_problem(
                        ui,
                        e,
                        "未选择保存位置。当前修改仍保留，可重试或取消返回编辑。",
                    );
                } else {
                    set_status(ui, "已取消保存，当前修改仍保留");
                }
                return;
            };
            p
        }
    };
    set_status(ui, "正在保存项目…");
    let snapshot = doc.clone();
    start_work(ui, e, timer, move || {
        Work::Saved(
            storage::save_project(&path, &doc, &asset).map(|_| path),
            snapshot,
        )
    });
}
fn load_path(
    ui: &AppWindow,
    e: &mut Editor,
    timer: &Rc<slint::Timer>,
    path: PathBuf,
    project: bool,
) {
    begin_action(ui, e, timer, PendingAction::Load { path, project });
}
fn load_path_unchecked(
    ui: &AppWindow,
    e: &mut Editor,
    timer: &Rc<slint::Timer>,
    path: PathBuf,
    project: bool,
) {
    if e.worker.is_some() {
        return;
    }
    set_status(ui, "正在读取图片与项目…");
    start_work(ui, e, timer, move || {
        let result = (|| {
            let (doc, asset, path) = if project {
                let (d, a) = storage::open_project(&path)?;
                (d, a, Some(path))
            } else {
                let a = Asset::open(&path)?;
                let d = Document::new(a.name.clone(), a.pixels.width(), a.pixels.height())?;
                (d, a, None)
            };
            let thumb = preview(&asset);
            Ok((doc, asset, path, thumb))
        })();
        Work::Loaded(result)
    });
}
fn load_demo(ui: &AppWindow, e: &mut Editor, timer: &Rc<slint::Timer>, annotated: bool) {
    begin_action(ui, e, timer, PendingAction::Demo { annotated });
}
fn load_demo_unchecked(ui: &AppWindow, e: &mut Editor, timer: &Rc<slint::Timer>, annotated: bool) {
    if e.worker.is_some() {
        return;
    }
    start_work(ui, e, timer, move || {
        Work::Loaded((|| {
            let a = Asset::from_bytes(
                include_bytes!("../assets/demo.png").to_vec(),
                "首页设计.png".into(),
            )?;
            let mut d = Document::new(a.name.clone(), a.pixels.width(), a.pixels.height())?;
            if annotated {
                let id = d.add_region(Rect {
                    x: 48.,
                    y: 199.,
                    width: 510.,
                    height: 158.,
                })?;
                d.set_text(id, "标题更突出，减少上方留白".into())?;
                let id = d.add_region(Rect {
                    x: 780.,
                    y: 363.,
                    width: 300.,
                    height: 286.,
                })?;
                d.move_annotation(id, 42., 0.)?;
                d.set_text(id, "把主视觉向右移，给标题留出更多呼吸空间。".into())?;
                let id = d.add_point(Point { x: 270., y: 493. })?;
                d.set_text(id, "按钮颜色再醒目一些".into())?;
            }
            let thumb = preview(&a);
            Ok((d, a, None, thumb))
        })())
    });
}
fn paste_image_unchecked(ui: &AppWindow, e: &mut Editor, timer: &Rc<slint::Timer>) {
    start_work(ui, e, timer, move || {
        Work::Loaded((|| {
            let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
            let img = clipboard
                .get_image()
                .map_err(|_| "剪贴板中没有图片，请复制图片后重试".to_string())?;
            if img.width as u64 * img.height as u64 > storage::MAX_PIXELS {
                return Err("剪贴板图片超过 4000 万像素上限".into());
            }
            let rgba =
                RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
                    .ok_or("剪贴板像素格式无效")?;
            let asset = Asset::from_rgba(rgba, "剪贴板图片.png")?;
            let doc = Document::new(
                asset.name.clone(),
                asset.pixels.width(),
                asset.pixels.height(),
            )?;
            let thumb = preview(&asset);
            Ok((doc, asset, None, thumb))
        })())
    });
}
fn zoom(ui: &AppWindow, e: &mut Editor, factor: f32, x: f32, y: f32) {
    if e.worker.is_some() || e.doc().is_none() {
        return;
    }
    if let Err(err) = e.flush() {
        show_message(ui, e, "无法完成操作", err);
        return;
    }
    e.gesture = None;
    ui.set_preview_visible(false);
    clear_hover(ui, e);
    let p = e.point(x, y);
    let scale = (e.view.scale * factor).clamp(0.03, 8.);
    e.view.scale = scale;
    e.view.x = x - p.x * scale;
    e.view.y = y - p.y * scale;
    e.view.fit = false;
    sync(ui, e);
}
fn app_dir() -> PathBuf {
    #[cfg(windows)]
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("Library/Application Support"))
        .unwrap_or_else(std::env::temp_dir);
    #[cfg(not(any(windows, target_os = "macos")))]
    let root = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".local/share"))
                .unwrap_or_else(std::env::temp_dir)
        });
    root.join("image-feedback-slint")
}

fn window_pointer_move(ui: &AppWindow, x: f32, y: f32) {
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(x, y),
        });
}
fn window_pointer_click(ui: &AppWindow, x: f32, y: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(x, y);
    window_pointer_move(ui, x, y);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}
fn canvas_move(ui: &AppWindow, x: f32, y: f32) {
    window_pointer_move(
        ui,
        ui.get_canvas_window_x() + x,
        ui.get_canvas_window_y() + y,
    );
}
fn canvas_press(ui: &AppWindow, x: f32, y: f32, button: i32) {
    canvas_move(ui, x, y);
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerPressed {
            position: slint::LogicalPosition::new(
                ui.get_canvas_window_x() + x,
                ui.get_canvas_window_y() + y,
            ),
            button: if button == 0 {
                slint::platform::PointerEventButton::Left
            } else if button == 2 {
                slint::platform::PointerEventButton::Middle
            } else {
                slint::platform::PointerEventButton::Right
            },
        });
}
fn canvas_release(ui: &AppWindow, x: f32, y: f32) {
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerReleased {
            position: slint::LogicalPosition::new(
                ui.get_canvas_window_x() + x,
                ui.get_canvas_window_y() + y,
            ),
            button: slint::platform::PointerEventButton::Left,
        });
}
fn smoke_request_close(ui: &AppWindow) -> Result<(), String> {
    let native = cfg!(target_os = "macos");
    let decorated = ui
        .window()
        .with_winit_window(|window| window.is_decorated())
        .ok_or("Native window was not created")?;
    if ui.get_native_chrome() != native || decorated != native {
        return Err("Platform window decorations do not match the platform".into());
    }
    if native {
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::CloseRequested);
    } else {
        window_pointer_click(
            ui,
            ui.get_close_button_center_x(),
            ui.get_close_button_center_y(),
        );
    }
    Ok(())
}
fn save_window_snapshot(ui: &AppWindow, path: &Path) -> Result<(), String> {
    let snapshot = ui.window().take_snapshot().map_err(|e| e.to_string())?;
    let pixels = RgbaImage::from_raw(
        snapshot.width(),
        snapshot.height(),
        snapshot.as_bytes().to_vec(),
    )
    .ok_or("invalid screenshot buffer")?;
    std::fs::write(path, storage::png_bytes(&pixels)?).map_err(|e| e.to_string())
}
fn smoke_interactions(
    ui: &AppWindow,
    state: &Rc<RefCell<Editor>>,
    folder: &Path,
) -> Result<(), String> {
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    if !ui.get_loaded() || ui.get_busy() {
        return Err("Demo was not ready within the smoke-test startup budget".into());
    }
    let screen = |x: f32, y: f32| {
        let e = state.borrow();
        (e.view.x + x * e.view.scale, e.view.y + y * e.view.scale)
    };
    // Dispatch unpressed window pointer events through scene hit testing and TouchArea.
    // Calling the Rust callback directly would miss a broken Slint event bridge.
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerExited);
    if state.borrow().analysis_loading {
        return Err("native smart selection did not finish within the smoke budget".into());
    }
    let candidate = state
        .borrow()
        .eligible
        .first()
        .cloned()
        .ok_or("demo produced no native selection candidates")?;
    let before_hover = state.borrow().doc().unwrap().clone();
    let (x, y) = screen(
        candidate.rect.x + candidate.rect.width / 2.,
        candidate.rect.y + candidate.rect.height / 2.,
    );
    canvas_move(ui, x, y);
    if !ui.get_hover_visible() {
        return Err("native hover preview was not displayed".into());
    }
    if state.borrow().doc() != Some(&before_hover) {
        return Err("hover changed the document".into());
    }
    let selected_rect = {
        let e = state.borrow();
        e.eligible
            .iter()
            .find(|c| Some(c.id.as_str()) == e.hover.as_deref())
            .ok_or("hover candidate missing")?
            .rect
    };
    let snap = ui.window().take_snapshot().map_err(|e| e.to_string())?;
    let shot = RgbaImage::from_raw(snap.width(), snap.height(), snap.as_bytes().to_vec())
        .ok_or("invalid snapshot")?;
    std::fs::write(folder.join("smart-hover.png"), storage::png_bytes(&shot)?)
        .map_err(|e| e.to_string())?;
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerExited);
    if ui.get_hover_visible() {
        return Err("hover preview did not disappear on pointer exit".into());
    }
    canvas_move(ui, x, y);
    if !ui.get_hover_visible() {
        return Err("unpressed hover did not return on pointer re-entry".into());
    }
    canvas_press(ui, x, y, 0);
    canvas_release(ui, x, y);
    if !matches!(state.borrow().doc().unwrap().annotations[0].kind,AnnotationKind::Region{rect}if rect==selected_rect)
    {
        return Err("click did not adopt the pressed recommendation".into());
    }
    ui.invoke_undo();
    ui.invoke_toggle_smart();
    if state.borrow().smart_enabled {
        return Err("smart selection toggle did not disable analysis".into());
    }
    let (x, y) = screen(52., 200.);
    let (x2, y2) = screen(555., 355.);
    canvas_press(ui, x, y, 0);
    canvas_move(ui, x2, y2);
    canvas_release(ui, x2, y2);
    let id = state
        .borrow()
        .selected
        .ok_or("box selection did not create a note")?;
    let token = ui_token(&state.borrow(), id);
    ui.invoke_note_edited(token, "标题更突出，减少上方留白".into());
    ui.invoke_note_edit_finished(token);
    if state.borrow().doc().unwrap().annotations.len() != 1 {
        return Err("region note count mismatch".into());
    }
    let (x, y) = screen(240., 270.);
    let (x2, y2) = screen(270., 285.);
    canvas_press(ui, x, y, 0);
    let move_count = state.borrow().pointer_move_events;
    canvas_move(ui, x2, y2);
    if state.borrow().pointer_move_events != move_count + 1 {
        return Err("one drag pointer event was forwarded zero or multiple times".into());
    }
    canvas_release(ui, x2, y2);
    if !matches!(
        state.borrow().doc().unwrap().annotation(id).unwrap().kind,
        AnnotationKind::Block { .. }
    ) {
        return Err("dragging region did not create a block".into());
    }
    // Exercise the ready-cache path here; the delivery smoke separately tests asynchronous preparation.
    let (copy_doc, copy_asset) = {
        let e = state.borrow();
        (e.doc().unwrap().clone(), e.asset.as_ref().unwrap().clone())
    };
    let cache = copy_control::prepare(&folder.join("copy-cache"), copy_doc, &copy_asset, None)?;
    state.borrow_mut().delivery = Some(cache);
    // Click the real copy button after a committed move; use an isolated clipboard sink.
    window_pointer_click(
        ui,
        ui.get_copy_button_center_x(),
        ui.get_copy_button_center_y(),
    );
    if !ui.get_toast_visible()
        || ui.get_toast_title().as_str() != "复制成功"
        || !ui.get_copy_complete()
    {
        return Err("copy button did not show an explicit success notification".into());
    }
    if !state
        .borrow()
        .clipboard_written
        .as_ref()
        .is_some_and(|text| text.contains("移动图块") && text.contains("目标"))
    {
        return Err("copy after move did not include the movement instructions".into());
    }
    save_window_snapshot(ui, &folder.join("copy-success.png"))?;
    state.borrow_mut().clipboard_error = Some("测试：剪贴板正被其他程序占用".into());
    window_pointer_click(
        ui,
        ui.get_copy_button_center_x(),
        ui.get_copy_button_center_y(),
    );
    if ui.get_toast_title().as_str() != "复制失败"
        || ui.get_toast_success()
        || ui.get_copy_complete()
    {
        return Err("clipboard error was not shown, or falsely kept the success state".into());
    }
    save_window_snapshot(ui, &folder.join("copy-failure.png"))?;
    state.borrow_mut().clipboard_error = None;
    window_pointer_click(
        ui,
        ui.get_copy_button_center_x(),
        ui.get_copy_button_center_y(),
    );
    if !ui.get_copy_complete() {
        return Err("copy did not recover after a clipboard error".into());
    }
    // Custom close must use the same data-loss guard as native close / Alt+F4.
    let (sender, receiver) = mpsc::channel();
    let close_count = state.borrow().close_requests;
    state.borrow_mut().worker = Some(receiver);
    smoke_request_close(ui)?;
    if state.borrow().close_requests != close_count + 1 {
        return Err("platform close bypassed the close-request guard".into());
    }
    #[cfg(target_os = "macos")]
    {
        let before = state.borrow().close_requests;
        platform::request_native_quit();
        if state.borrow().close_requests != before + 1 || !ui.window().is_visible() {
            return Err("AppKit Quit bypassed the unsaved-work guard".into());
        }
    }
    state.borrow_mut().worker = None;
    drop(sender);
    ui.invoke_choose_tool(1);
    let (x, y) = screen(272., 495.);
    canvas_press(ui, x, y, 0);
    canvas_release(ui, x, y);
    let point = state.borrow().selected.ok_or("point selection failed")?;
    let token = ui_token(&state.borrow(), point);
    ui.invoke_note_edited(token, "按钮颜色再醒目一些".into());
    ui.invoke_note_edit_finished(token);
    ui.invoke_add_global();
    let global = state.borrow().selected.ok_or("global note failed")?;
    let token = ui_token(&state.borrow(), global);
    ui.invoke_note_edited(token, "保持整体简洁，突出主要操作。".into());
    ui.invoke_note_edit_finished(token);
    let before = state.borrow().doc().unwrap().clone();
    ui.invoke_undo();
    ui.invoke_redo();
    if state.borrow().doc() != Some(&before) {
        return Err("UI undo/redo failed to preserve document".into());
    }
    let anchor = state.borrow().point(350., 300.);
    ui.invoke_canvas_wheel(55., 350., 300.);
    let after = state.borrow().point(350., 300.);
    if (anchor.x - after.x).abs() > 0.01 || (anchor.y - after.y).abs() > 0.01 {
        return Err("zoom did not preserve pointer anchor".into());
    }
    ui.invoke_fit_view();
    ui.invoke_choose_tool(0);
    let token = ui_token(&state.borrow(), id);
    ui.invoke_select_note(token);
    ui.invoke_toggle_smart();
    let e = state.borrow();
    let doc = e.doc().unwrap();
    let asset = e.asset.as_ref().unwrap();
    let path = folder.join("smoke-project.ifp");
    storage::save_project(&path, doc, asset)?;
    let (restored, original) = storage::open_project(&path)?;
    if &restored != doc || original.original != asset.original {
        return Err("portable project roundtrip mismatch".into());
    }
    let package = storage::export_package(folder, doc, asset)?;
    let report = serde_json::json!({"result":"interactions_passed","platform":std::env::consts::OS,"native_chrome":ui.get_native_chrome(),"appkit_quit_guard_checked":cfg!(target_os="macos"),"slint":"1.18.1","method":"real Slint window; scene-dispatched pointer events for hover/selection/drag/copy/close, callbacks for editing commands; isolated clipboard sink, not OS mouse automation","checks":["unpressed hover through the real TouchArea event bridge","hover exit and re-entry","single forwarding for each drag pointer event","copy after movement includes coordinates and success toast","clipboard failure notification and retry","platform close uses the existing save guard; native AppKit Quit is also checked on macOS","native hover without document mutation","click adopts pressed recommendation","smart toggle with manual fallback","region selection","Chinese note commit","region to block drag","point annotation","global annotation","undo and redo callbacks","pointer-anchored zoom","portable project roundtrip","unchanged original bytes","four-file feedback export"],"annotations":doc.annotations.len(),"package":package,"screenshot":"editor.png"});
    std::fs::write(
        folder.join("smoke-report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    set_status(ui, "交互与保存／导出自检通过 · 这是原生 Slint 窗口实机截图");
    Ok(())
}

fn create_app_window() -> Result<AppWindow, Box<dyn std::error::Error>> {
    platform::configure_editor_backend()?;
    let ui = AppWindow::new()?;
    ui.set_native_chrome(cfg!(target_os = "macos"));
    platform::install_quit_handler(ui.as_weak())?;
    Ok(ui)
}
fn run_editor(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(app_dir())?;
    let lock = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(app_dir().join("editor.lock"))?;
    if lock.try_lock().is_err() {
        if args.iter().any(|arg| {
            arg == "--smoke-test" || arg == "--dialog-smoke" || arg == "--delivery-smoke"
        }) {
            return Err("请关闭现有编辑器后再运行窗口自检（未修改现有进程）".into());
        }
        #[cfg(target_os = "macos")]
        if platform::activate_editor() {
            return Ok(());
        }
        return show_notice_window(
            "编辑器已在运行",
            "请切换至已有窗口，避免重复加载图片。当前已打开的项目不会受到影响。",
        );
    }
    let ui = create_app_window()?;
    #[cfg(target_os = "macos")]
    let activation_server = platform::start_activation_server(ui.as_weak())?;
    let editor = Rc::new(RefCell::new(Editor {
        clipboard_probe: args.iter().any(|arg| {
            arg == "--smoke-test" || arg == "--dialog-smoke" || arg == "--delivery-smoke"
        }),
        ..Editor::default()
    }));
    let timer = Rc::new(slint::Timer::default());
    let toast_timer = Rc::new(slint::Timer::default());
    {
        let weak = ui.as_weak();
        let t = toast_timer.clone();
        ui.on_dismiss_toast(move || {
            t.stop();
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_visible(false);
                ui.set_copy_complete(false);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        let t = timer.clone();
        ui.on_confirm_response(move |response| {
            if let Some(ui) = weak.upgrade() {
                confirmation_response(&ui, &mut state.borrow_mut(), &t, response);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        let t = timer.clone();
        ui.window().on_winit_window_event(move |_, event| {
            use slint::winit_030::winit::event::WindowEvent;
            match event {
                WindowEvent::DroppedFile(path) => {
                    if let Some(ui) = weak.upgrade() {
                        let path = path.clone();
                        let project = path
                            .extension()
                            .is_some_and(|s| s.eq_ignore_ascii_case("ifp"));
                        load_path(&ui, &mut state.borrow_mut(), &t, path, project);
                    }
                    return NativeEventResult::PreventDefault;
                }
                WindowEvent::ModifiersChanged(mods) => {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_shift_held(mods.state().shift_key());
                    }
                }
                WindowEvent::Focused(false) => {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_space_held(false);
                        ui.set_shift_held(false);
                        if let Ok(mut e) = state.try_borrow_mut() {
                            e.gesture = None;
                            ui.set_preview_visible(false);
                        }
                    }
                }
                _ => {}
            }
            NativeEventResult::Propagate
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_canvas_command(move |command| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let command = command.as_str();
            let Ok(mut e) = state.try_borrow_mut() else {
                return;
            };
            if command == "leave" {
                clear_hover(&ui, &mut e);
                return;
            }
            if e.worker.is_some() {
                return;
            }
            if command == "cancel-text" {
                e.pending = None;
                // Reset the TextEdit's local value too, not just the Rust transaction.
                ui.set_notes(ModelRc::default());
                sync(&ui, &mut e);
                return;
            }
            if command == "finish-text" {
                if let Err(err) = e.flush() {
                    show_message(&ui, &mut e, "无法完成操作", err);
                    return;
                }
                e.selected = None;
                sync(&ui, &mut e);
                return;
            }
            if command == "cancel" {
                e.gesture = None;
                ui.set_preview_visible(false);
                e.selected = None;
                sync(&ui, &mut e);
                return;
            }
            if let Err(err) = e.flush() {
                show_message(&ui, &mut e, "无法完成操作", err);
                return;
            }
            e.gesture = None;
            ui.set_preview_visible(false);
            let Some(id) = e.selected else {
                return;
            };
            if command == "delete" {
                request_remove(&ui, &mut e, id);
            } else {
                let step = if command.ends_with("-fast") { 10. } else { 1. };
                let (dx, dy) = match command.split('-').next().unwrap_or("") {
                    "left" => (-step, 0.),
                    "right" => (step, 0.),
                    "up" => (0., -step),
                    "down" => (0., step),
                    _ => (0., 0.),
                };
                let (dx, dy) = e.bounded_delta(id, dx, dy);
                if let Some(h) = e.history.as_mut() {
                    if let Err(err) = h.apply(|d| d.move_annotation(id, dx, dy)) {
                        show_message(&ui, &mut e, "无法完成操作", err);
                    }
                }
            }
            sync(&ui, &mut e);
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_viewport_resized(move |w, h| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut e) = state.try_borrow_mut() {
                    if e.view.fit && w > 0. && h > 0. {
                        e.fit(w, h);
                        sync(&ui, &mut e);
                    }
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        let timer_weak = Rc::downgrade(&timer);
        let notifications = toast_timer.clone();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(35),
            move || {
                let Some(t) = timer_weak.upgrade() else {
                    return;
                };
                let Some(ui) = weak.upgrade() else {
                    t.stop();
                    return;
                };
                let mut e = state.borrow_mut();
                let result = e.worker.as_ref().map(|rx| rx.try_recv());
                match result {
                    Some(Ok(work)) => {
                        e.worker = None;
                        t.stop();
                        ui.set_busy(false);
                        match work {
                            Work::Loaded(Ok((d, a, p, thumb))) => {
                                install(&ui, &mut e, d, a, p, thumb)
                            }
                            Work::Loaded(Err(err)) => {
                                set_status(&ui, format!("读取失败：{err}"));
                                show_message(&ui, &mut e, "无法读取图片或项目", err);
                            }
                            Work::Saved(result, snapshot) => match result {
                                Ok(path) => {
                                    let action = e.saved_successfully(path, snapshot);
                                    set_status(&ui, "项目已保存 · 原图与修改意见均可恢复");
                                    sync(&ui, &mut e);
                                    if let Some(action) = action {
                                        perform_action(&ui, &mut e, &t, action);
                                    } else if e.confirmation.is_none() {
                                        show_toast(&ui,&notifications,true,"项目已保存","原图与修改意见均可恢复。");ui.set_copy_complete(false);
                                    }
                                }
                                Err(err) => save_problem(&ui, &mut e, err),
                            },
                            Work::Delivered(result,request)=>copy_control::finish(&ui,&mut e,&notifications,result,request),
                            Work::Exported(result) => match result {
                                Ok(cache) => {
                                    set_status(&ui, format!("已导出完整反馈包：{}", cache.package.folder.display()));
                                    e.delivery=Some(cache);sync(&ui,&mut e);
                                    show_toast(&ui,&notifications,true,"反馈包已导出","包含原图、编号参考图和可发送的 ZIP。\n可从更多菜单打开最近反馈包。");ui.set_copy_complete(false);
                                }
                                Err(err) => {
                                    set_status(&ui, format!("导出失败：{err}"));
                                    show_message(
                                        &ui,
                                        &mut e,
                                        "导出未完成",
                                        format!(
                                            "{err}\n目标目录可能保留部分文件，当前编辑数据未丢失。"
                                        ),
                                    );
                                }
                            },
                        }
                    }
                    Some(Err(mpsc::TryRecvError::Disconnected)) => {
                        e.worker = None;
                        t.stop();
                        ui.set_busy(false);
                        if e.confirmation.is_some() {
                            e.save_problem(
                                "后台任务异常结束，当前编辑数据仍保留。可重试或取消返回编辑。",
                            );
                            sync_confirmation(&ui, &e);
                        } else {
                            show_message(
                                &ui,
                                &mut e,
                                "后台任务未完成",
                                "任务异常结束，当前编辑数据仍保留。请重试。",
                            );
                        }
                        set_status(&ui, "后台任务异常结束，当前编辑数据仍保留");
                    }
                    _ => {}
                }
                if e.analysis_loading {
                    let analysis = e.analysis.as_ref().and_then(AnalysisService::take_result);
                    if let Some(result) = analysis {
                        if result.generation == e.analysis_generation
                            && e.doc().is_some_and(|d| {
                                (d.image.width, d.image.height) == (result.width, result.height)
                            })
                        {
                            e.analysis_loading = false;
                            match result.candidates {
                                Ok(found) => {
                                    e.candidates = found;
                                    e.update_eligibility(true);
                                }
                                Err(err) => {
                                    e.candidates.clear();
                                    e.eligible.clear();
                                    set_status(
                                        &ui,
                                        format!("智能预选不可用（{err}），可继续手动框选"),
                                    );
                                }
                            }
                            let point = e.hover_point;
                            sync(&ui, &mut e);
                            if let Some(p) = point {
                                update_hover(&ui, &mut e, p);
                            }
                        }
                    }
                }
                if e.worker.is_some() || e.analysis_loading {
                    t.restart();
                } else {
                    t.stop();
                }
            },
        );
        timer.stop();
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        let t = timer.clone();
        ui.on_toggle_smart(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut e = state.borrow_mut();
            e.smart_enabled = !e.smart_enabled;
            clear_hover(&ui, &mut e);
            if !e.smart_enabled {
                e.analysis.take();
                e.analysis_loading = false;
                e.candidates = Vec::new();
                e.eligible = Vec::new();
            } else {
                begin_analysis(&mut e);
                if e.analysis_loading {
                    t.restart();
                }
            }
            sync(&ui, &mut e);
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        let t = timer.clone();
        ui.on_new_project(move || {
            if let Some(ui) = weak.upgrade() {
                begin_action(&ui, &mut state.borrow_mut(), &t, PendingAction::New);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_clear_feedback(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut e = state.borrow_mut();
            if e.worker.is_some()
                || e.confirmation.is_some()
                || e.message.is_some()
                || !e.doc().is_some_and(|d| !d.annotations.is_empty())
            {
                return;
            }
            e.message = Some(AppMessage {
                title: "清空当前反馈？".into(),
                description: "保留当前图片，清除全部批注、文字意见并复原图块位置。".into(),
                name: e.doc().unwrap().image.file_name.clone(),
                primary: "清空当前".into(),
                destructive: true,
                action: MessageAction::ClearFeedback,
            });
            e.gesture = None;
            clear_hover(&ui, &mut e);
            ui.set_preview_visible(false);
            ui.set_toast_visible(false);
            sync_confirmation(&ui, &e);
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        let t = timer.clone();
        ui.on_import_image(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some(path) = rfd::FileDialog::new()
                .set_title("导入图片")
                .add_filter("图片", &["png", "jpg", "jpeg", "webp", "bmp"])
                .pick_file()
            else {
                return;
            };
            load_path(&ui, &mut state.borrow_mut(), &t, path, false);
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        let t = timer.clone();
        ui.on_open_project(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some(path) = rfd::FileDialog::new()
                .set_title("打开项目")
                .add_filter("图片反馈项目", &["ifp"])
                .pick_file()
            else {
                return;
            };
            load_path(&ui, &mut state.borrow_mut(), &t, path, true);
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        let t = timer.clone();
        ui.on_load_example(move || {
            if let Some(ui) = weak.upgrade() {
                load_demo(&ui, &mut state.borrow_mut(), &t, false);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        let t = timer.clone();
        ui.on_save_project(move || {
            if let Some(ui) = weak.upgrade() {
                save(&ui, &mut state.borrow_mut(), &t);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        let t = timer.clone();
        ui.on_paste_image(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            begin_action(&ui, &mut state.borrow_mut(), &t, PendingAction::Paste);
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        let t = timer.clone();
        ui.on_export_package(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut e = state.borrow_mut();
            if e.worker.is_some() {
                return;
            }
            if let Err(err) = e.flush() {
                show_message(&ui, &mut e, "无法完成操作", err);
                return;
            }
            let (Some(doc), Some(asset)) = (e.doc().cloned(), e.asset.clone()) else {
                return;
            };
            let Some(folder) = rfd::FileDialog::new()
                .set_title("选择反馈包保存目录（将在其中新建文件夹）")
                .pick_folder()
            else {
                return;
            };
            sync(&ui, &mut e);
            set_status(&ui, "正在生成原图、编号参考图、说明与 JSON…");
            start_work(&ui, &mut e, &t, move || {
                Work::Exported(copy_control::prepare(&folder, doc, &asset, None))
            });
        });
    }
    for request in [
        CopyRequest::Feedback,
        CopyRequest::OriginalPath,
        CopyRequest::Base64,
    ] {
        let weak = ui.as_weak();
        let state = editor.clone();
        let work = timer.clone();
        let notice = toast_timer.clone();
        let callback = move || {
            if let Some(ui) = weak.upgrade() {
                copy_control::begin(&ui, &mut state.borrow_mut(), &work, &notice, request);
            }
        };
        match request {
            CopyRequest::Feedback => ui.on_copy_feedback(callback),
            CopyRequest::OriginalPath => ui.on_copy_original_path(callback),
            CopyRequest::Base64 => ui.on_copy_portable_data(callback),
        }
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_open_feedback_folder(move || {
            if let Some(ui) = weak.upgrade() {
                copy_control::open_folder(&ui, &mut state.borrow_mut());
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_add_global(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut e = state.borrow_mut();
            if e.worker.is_some() {
                return;
            }
            if let Err(err) = e.flush() {
                show_message(&ui, &mut e, "无法完成操作", err);
                return;
            }
            if let Some(h) = e.history.as_mut() {
                match h.apply(Document::add_global) {
                    Ok(id) => e.selected = Some(id),
                    Err(err) => show_message(&ui, &mut e, "无法完成操作", err),
                }
            }
            sync(&ui, &mut e);
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_note_edited(move |id, text| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut e = state.borrow_mut();
            let Some(actual) = ui_note_id(&e, id) else {
                return;
            };
            e.pending = Some((actual, text.into()));
            ui.set_dirty(e.dirty());
            let draft_changed = e.pending.as_ref().is_some_and(|(id, text)| {
                e.doc()
                    .and_then(|d| d.annotation(*id))
                    .is_some_and(|n| n.text != *text)
            });
            ui.set_can_undo(draft_changed || e.history.as_ref().is_some_and(History::can_undo));
            ui.set_can_redo(!draft_changed && e.history.as_ref().is_some_and(History::can_redo));
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_note_edit_finished(move |_| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Ok(mut e) = state.try_borrow_mut() else {
                return;
            };
            if let Err(err) = e.flush() {
                show_message(&ui, &mut e, "无法完成操作", err);
            }
            sync(&ui, &mut e);
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_select_note(move |id| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut e = state.borrow_mut();
            if let Err(err) = e.flush() {
                show_message(&ui, &mut e, "无法完成操作", err);
                return;
            }
            e.selected = ui_note_id(&e, id);
            e.revealed_selection = None;
            sync(&ui, &mut e);
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_remove_note(move |id| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut e = state.borrow_mut();
            if let Some(actual) = ui_note_id(&e, id) {
                request_remove(&ui, &mut e, actual);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_undo(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut e = state.borrow_mut();
            if e.worker.is_some() {
                return;
            }
            if let Err(err) = e.flush() {
                show_message(&ui, &mut e, "无法完成操作", err);
                return;
            }
            e.gesture = None;
            clear_hover(&ui, &mut e);
            ui.set_preview_visible(false);
            if let Some(h) = e.history.as_mut() {
                h.undo();
            }
            e.selected = e
                .selected
                .filter(|id| e.doc().is_some_and(|d| d.annotation(*id).is_some()));
            sync(&ui, &mut e);
            if e.selected.is_none() {
                ui.invoke_focus_canvas();
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_redo(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut e = state.borrow_mut();
            if e.worker.is_some() {
                return;
            }
            if let Err(err) = e.flush() {
                show_message(&ui, &mut e, "无法完成操作", err);
                return;
            }
            e.gesture = None;
            clear_hover(&ui, &mut e);
            ui.set_preview_visible(false);
            if let Some(h) = e.history.as_mut() {
                h.redo();
            }
            e.selected = e
                .selected
                .filter(|id| e.doc().is_some_and(|d| d.annotation(*id).is_some()));
            sync(&ui, &mut e);
            if e.selected.is_none() {
                ui.invoke_focus_canvas();
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_choose_tool(move |tool| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut e = state.borrow_mut();
            if let Err(err) = e.flush() {
                show_message(&ui, &mut e, "无法完成操作", err);
                return;
            }
            e.tool = tool;
            e.gesture = None;
            clear_hover(&ui, &mut e);
            ui.set_preview_visible(false);
            sync(&ui, &mut e);
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_fit_view(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut e = state.borrow_mut();
            if let Err(err) = e.flush() {
                show_message(&ui, &mut e, "无法完成操作", err);
                return;
            }
            e.fit(ui.get_viewport_width(), ui.get_viewport_height());
            sync(&ui, &mut e);
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_zoom_by(move |factor| {
            if let Some(ui) = weak.upgrade() {
                zoom(
                    &ui,
                    &mut state.borrow_mut(),
                    factor,
                    ui.get_viewport_width() / 2.,
                    ui.get_viewport_height() / 2.,
                );
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_canvas_wheel(move |dy, x, y| {
            if let Some(ui) = weak.upgrade() {
                zoom(
                    &ui,
                    &mut state.borrow_mut(),
                    (dy * 0.002).exp().clamp(0.7, 1.4),
                    x,
                    y,
                );
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_canvas_press(move |x, y, button| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut e = state.borrow_mut();
            if e.worker.is_some() || e.doc().is_none() {
                return;
            }
            if let Err(err) = e.flush() {
                show_message(&ui, &mut e, "无法完成操作", err);
                return;
            }
            let p = e.point(x, y);
            let candidate = if !ui.get_shift_held() && !ui.get_space_held() {
                e.candidate_at(p).map(|c| c.rect)
            } else {
                None
            };
            clear_hover(&ui, &mut e);
            if button == 2 || e.tool == 2 || ui.get_space_held() {
                e.gesture = Some(Gesture::Pan {
                    x,
                    y,
                    origin_x: e.view.x,
                    origin_y: e.view.y,
                });
                return;
            }
            if button != 0 || !e.inside(p) {
                return;
            }
            if e.tool == 1 {
                if let Some(h) = e.history.as_mut() {
                    match h.apply(|d| d.add_point(p)) {
                        Ok(id) => e.selected = Some(id),
                        Err(err) => show_message(&ui, &mut e, "无法完成操作", err),
                    }
                }
                sync(&ui, &mut e);
                return;
            }
            let hit = if ui.get_shift_held() { None } else { e.hit(p) };
            if let Some(id) = hit {
                e.selected = Some(id);
                e.gesture = Some(Gesture::Move {
                    id,
                    start: p,
                    dx: 0.,
                    dy: 0.,
                });
            } else {
                e.selected = None;
                e.gesture = Some(Gesture::Select {
                    start: p,
                    candidate,
                    manual: false,
                });
                ui.set_preview_visible(true);
                ui.set_preview_x(x);
                ui.set_preview_y(y);
                ui.set_preview_w(0.);
                ui.set_preview_h(0.);
            }
            sync(&ui, &mut e);
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_canvas_move(move |x, y| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut e = state.borrow_mut();
            e.pointer_move_events = e.pointer_move_events.wrapping_add(1);
            let Some(g) = e.gesture.clone() else {
                let p = e.point(x, y);
                update_hover(&ui, &mut e, p);
                return;
            };
            match g {
                Gesture::Pan {
                    x: origin_x,
                    y: origin_y,
                    origin_x: vx,
                    origin_y: vy,
                } => {
                    e.view.x = vx + x - origin_x;
                    e.view.y = vy + y - origin_y;
                    e.view.fit = false;
                }
                Gesture::Select {
                    start,
                    candidate,
                    manual,
                } => {
                    let p = e.point(x, y);
                    let Some(doc) = e.doc() else {
                        return;
                    };
                    let px = p.x.clamp(0., doc.image.width as f32);
                    let py = p.y.clamp(0., doc.image.height as f32);
                    let manual =
                        manual || (p.x - start.x).hypot(p.y - start.y) * e.view.scale >= 4.;
                    e.gesture = Some(Gesture::Select {
                        start,
                        candidate,
                        manual,
                    });
                    ui.set_preview_x(e.view.x + start.x.min(px) * e.view.scale);
                    ui.set_preview_y(e.view.y + start.y.min(py) * e.view.scale);
                    ui.set_preview_w((px - start.x).abs() * e.view.scale);
                    ui.set_preview_h((py - start.y).abs() * e.view.scale);
                    return;
                }
                Gesture::Move { id, start, .. } => {
                    let p = e.point(x, y);
                    let (dx, dy) = e.bounded_delta(id, p.x - start.x, p.y - start.y);
                    e.gesture = Some(Gesture::Move { id, start, dx, dy });
                }
            }
            sync(&ui, &mut e);
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        ui.on_canvas_release(move |x, y| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut e = state.borrow_mut();
            ui.set_preview_visible(false);
            let Some(g) = e.gesture.take() else {
                return;
            };
            match g {
                Gesture::Select {
                    start,
                    candidate,
                    manual,
                } => {
                    let p = e.point(x, y);
                    if let Some(d) = e.doc() {
                        let end = Point {
                            x: p.x.clamp(0., d.image.width as f32),
                            y: p.y.clamp(0., d.image.height as f32),
                        };
                        let candidate = if ui.get_shift_held() { None } else { candidate };
                        if let Some(rect) =
                            selection_result(start, end, e.view.scale, candidate, manual)
                        {
                            if let Some(h) = e.history.as_mut() {
                                match h.apply(|d| d.add_region(rect)) {
                                    Ok(id) => {
                                        e.selected = Some(id);
                                        set_status(
                                            &ui,
                                            "选区已创建 · 直接输入意见，或拖动范围移动图块",
                                        );
                                    }
                                    Err(err) => show_message(&ui, &mut e, "无法完成操作", err),
                                }
                            }
                        }
                    }
                }
                Gesture::Move { id, dx, dy, .. } => {
                    if dx.hypot(dy) * e.view.scale >= 2. {
                        if let Some(h) = e.history.as_mut() {
                            if let Err(err) = h.apply(|d| d.move_annotation(id, dx, dy)) {
                                show_message(&ui, &mut e, "无法完成操作", err);
                            } else {
                                set_status(&ui, "位置已更新 · 原图像素保持不变");
                            }
                        }
                    }
                }
                Gesture::Pan { .. } => {}
            }
            sync(&ui, &mut e);
        });
    }
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        let t = timer.clone();
        ui.window().on_close_requested(move || {
            let Some(ui) = weak.upgrade() else {
                return slint::CloseRequestResponse::HideWindow;
            };
            let Ok(mut e) = state.try_borrow_mut() else {
                set_status(&ui, "请先完成当前文件选择或编辑操作");
                return slint::CloseRequestResponse::KeepWindowShown;
            };
            e.close_requests = e.close_requests.wrapping_add(1);
            if e.worker.is_some() {
                set_status(&ui, "请等待当前保存／导出完成后再关闭");
                return slint::CloseRequestResponse::KeepWindowShown;
            }
            if e.message.is_some() {
                e.message = None;
                sync_confirmation(&ui, &e);
                return slint::CloseRequestResponse::KeepWindowShown;
            }
            if e.confirmation.is_some() {
                e.cancel_confirmation();
                sync_confirmation(&ui, &e);
                return slint::CloseRequestResponse::KeepWindowShown;
            }
            if !e.dirty() {
                return slint::CloseRequestResponse::HideWindow;
            }
            begin_action(&ui, &mut e, &t, PendingAction::Close);
            slint::CloseRequestResponse::KeepWindowShown
        });
    }
    set_status(&ui, "本地处理 · 不联网 · 原图保持不变");
    ui.show()?;
    // Startup loading happens once, after the real viewport has been laid out.
    let smoke = args
        .iter()
        .any(|a| a == "--smoke-test" || a == "--dialog-smoke" || a == "--delivery-smoke");
    let demo = smoke || args.iter().any(|a| a == "--demo");
    let input = args
        .iter()
        .position(|a| a == "--open")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    {
        let weak = ui.as_weak();
        let state = editor.clone();
        let t = timer.clone();
        slint::Timer::single_shot(Duration::from_millis(100), move || {
            if let Some(ui) = weak.upgrade() {
                if let Some(path) = input {
                    let project = path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("ifp"));
                    load_path(&ui, &mut state.borrow_mut(), &t, path, project);
                } else if demo {
                    load_demo(&ui, &mut state.borrow_mut(), &t, !smoke);
                }
            }
        });
    }
    let smoke_result = Rc::new(RefCell::new(None::<String>));
    if let Some(index) = args.iter().position(|arg| arg == "--smoke-test") {
        let folder = args
            .get(index + 1)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("artifacts/smoke"));
        let weak = ui.as_weak();
        let state = editor.clone();
        let failure = smoke_result.clone();
        let wait_ms = args
            .iter()
            .position(|a| a == "--smoke-wait-ms")
            .and_then(|i| args.get(i + 1))
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|ms| (500..=30_000).contains(ms))
            .unwrap_or(2_000);
        smoke_ready::start(
            weak,
            state.clone(),
            folder.clone(),
            failure.clone(),
            Duration::from_millis(wait_ms),
            move |ui| {
                let result = smoke_interactions(ui, &state, &folder);
                if let Err(err) = result {
                    *failure.borrow_mut() = Some(err.clone());
                    let _ = std::fs::create_dir_all(&folder);
                    let _ = std::fs::write(folder.join("failed.txt"), err);
                    let _ = ui.hide();
                    return;
                }
                let weak = ui.as_weak();
                let failure = failure.clone();
                slint::Timer::single_shot(Duration::from_millis(300), move || {
                    if let Some(ui) = weak.upgrade() {
                        let result = (|| -> Result<(), String> {
                            let snap = ui.window().take_snapshot().map_err(|e| e.to_string())?;
                            let image = RgbaImage::from_raw(
                                snap.width(),
                                snap.height(),
                                snap.as_bytes().to_vec(),
                            )
                            .ok_or("snapshot buffer size mismatch")?;
                            std::fs::write(folder.join("editor.png"), storage::png_bytes(&image)?)
                                .map_err(|e| e.to_string())?;
                            let report_path = folder.join("smoke-report.json");
                            let mut report: serde_json::Value = serde_json::from_slice(
                                &std::fs::read(&report_path).map_err(|e| e.to_string())?,
                            )
                            .map_err(|e| e.to_string())?;
                            report["result"] = serde_json::json!("passed");
                            report["snapshot_size"] =
                                serde_json::json!([image.width(), image.height()]);
                            std::fs::write(
                                report_path,
                                serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
                            )
                            .map_err(|e| e.to_string())?;
                            Ok(())
                        })();
                        if let Err(err) = result {
                            *failure.borrow_mut() = Some(err);
                        }
                        let _ = ui.hide();
                    }
                });
            },
        );
    }
    if let Some(index) = args.iter().position(|arg| arg == "--dialog-smoke") {
        let folder = args
            .get(index + 1)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("artifacts/dialog-smoke"));
        let weak = ui.as_weak();
        let state = editor.clone();
        let t = timer.clone();
        let failure = smoke_result.clone();
        smoke_ready::start(
            weak,
            state.clone(),
            folder.clone(),
            failure.clone(),
            Duration::from_secs(2),
            move |ui| {
                dialog_smoke::start(ui, &state, &t, folder, failure);
            },
        );
    }
    if let Some(index) = args.iter().position(|arg| arg == "--delivery-smoke") {
        let folder = args
            .get(index + 1)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("artifacts/delivery-smoke"));
        let weak = ui.as_weak();
        let state = editor.clone();
        let t = timer.clone();
        let failure = smoke_result.clone();
        smoke_ready::start(
            weak,
            state.clone(),
            folder.clone(),
            failure.clone(),
            Duration::from_secs(2),
            move |ui| {
                delivery_smoke::start(ui, &state, &t, folder, failure);
            },
        );
    }
    slint::run_event_loop()?;
    timer.stop();
    #[cfg(target_os = "macos")]
    drop(activation_server);
    drop(lock);
    if let Some(err) = smoke_result.borrow_mut().take() {
        return Err(err.into());
    }
    if let Some(index) = args.iter().position(|arg| arg == "--dialog-smoke") {
        let folder = args
            .get(index + 1)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("artifacts/dialog-smoke"));
        if !folder.join("dialog-report.json").is_file() {
            return Err("窗口在弹窗回归完成前退出".into());
        }
    }
    if let Some(index) = args.iter().position(|arg| arg == "--delivery-smoke") {
        let folder = args
            .get(index + 1)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("artifacts/delivery-smoke"));
        if !folder.join("delivery-report.json").is_file() {
            return Err("窗口在图片交付回归完成前退出".into());
        }
    }
    Ok(())
}
fn show_notice_window(title: &str, description: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = create_app_window()?;
    let state = Editor {
        message: Some(AppMessage {
            title: title.into(),
            description: description.into(),
            name: String::new(),
            primary: "知道了".into(),
            destructive: false,
            action: MessageAction::CloseNotice,
        }),
        ..Editor::default()
    };
    sync_confirmation(&ui, &state);
    let weak = ui.as_weak();
    ui.on_confirm_response(move |_| {
        if let Some(ui) = weak.upgrade() {
            let _ = ui.hide();
        }
    });
    ui.show()?;
    ui.set_confirm_visible(false);
    ui.set_confirm_visible(true);
    slint::run_event_loop()?;
    Ok(())
}
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version") {
        println!("Image Feedback {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    // Before any worker/event-loop threads exist; inherited by the editor child.
    if args.iter().any(|arg| arg == "--software-rendering") {
        unsafe {
            std::env::set_var("SLINT_BACKEND", "winit-software");
        }
    }
    let _installation_guard = platform::installation_guard();
    let smoke = args
        .iter()
        .any(|a| a == "--smoke-test" || a == "--dialog-smoke" || a == "--delivery-smoke");
    let result = if smoke
        || args
            .iter()
            .any(|a| a == "--editor" || a == "--demo" || a == "--open")
    {
        run_editor(&args)
    } else {
        resident::run(!args.iter().any(|a| a == "--tray-only"))
    };
    if let Err(error) = result {
        eprintln!("{error}");
        if !smoke && !args.iter().any(|a| a == "--tray-smoke") {
            let description = error.to_string();
            let _ = std::fs::create_dir_all(app_dir());
            let _ = std::fs::write(app_dir().join("last-startup-error.txt"), &description);
            // If a renderer cannot initialize, a self-drawn window may be impossible.
            // Keep the log rather than falling back to a stock OS message box.
            let _ = show_notice_window("图片反馈启动失败", &description);
        }
        std::process::exit(1);
    }
}
