//! Bounded local images and durable snapshots. No network or host IDs.
use crate::document::{AnnotationKind, Document, Point, Rect};
use crate::transform;
use image::{ImageEncoder, Pixel, Rgba, RgbaImage};
use std::fs::{self, File};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 40_000_000;

pub struct Asset {
    pub pixels: RgbaImage,
    pub original: Vec<u8>,
    pub extension: String,
    pub name: String,
    /// Disk origin is session-only metadata.
    pub source_path: Option<PathBuf>,
}

pub fn png_bytes(pixels: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            pixels.as_raw(),
            pixels.width(),
            pixels.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    Ok(out)
}

impl Asset {
    pub fn from_bytes(bytes: Vec<u8>, name: String) -> Result<Self, String> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err("图片文件不能超过 64 MiB".into());
        }
        let format = image::guess_format(&bytes)
            .map_err(|_| "不支持的图片格式；请选择 PNG/JPEG/WebP/BMP")?;
        let extension = match format {
            image::ImageFormat::Png => "png",
            image::ImageFormat::Jpeg => "jpg",
            image::ImageFormat::WebP => "webp",
            image::ImageFormat::Bmp => "bmp",
            _ => return Err("请选择 PNG、JPEG、WebP 或 BMP 图片".into()),
        }
        .to_string();
        let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
        let (w, h) = reader
            .into_dimensions()
            .map_err(|e| format!("无法读取图片尺寸：{e}"))?;
        if w == 0 || h == 0 || w as u64 * h as u64 > MAX_PIXELS {
            return Err("图片超过 4000 万像素的原型安全上限".into());
        }
        reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(320 * 1024 * 1024);
        reader.limits(limits);
        let pixels = reader
            .decode()
            .map_err(|e| format!("图片解码失败：{e}"))?
            .to_rgba8();
        Ok(Self {
            pixels,
            original: bytes,
            extension,
            name,
            source_path: None,
        })
    }
    pub fn open(path: &Path) -> Result<Self, String> {
        let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
        if metadata.len() > MAX_FILE_BYTES {
            return Err("图片文件不能超过 64 MiB".into());
        }
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|e| e.to_string())?
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Self::from_bytes(
            bytes,
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        )
        .map(|mut asset| {
            asset.source_path = std::path::absolute(path).ok();
            asset
        })
    }
    pub fn from_rgba(pixels: RgbaImage, name: &str) -> Result<Self, String> {
        if pixels.width() as u64 * pixels.height() as u64 > MAX_PIXELS {
            return Err("图片超过 4000 万像素的安全上限".into());
        }
        let original = png_bytes(&pixels)?;
        Ok(Self {
            pixels,
            original,
            extension: "png".into(),
            name: name.into(),
            source_path: None,
        })
    }
}

fn unique_suffix() -> String {
    unique_suffix_at(SystemTime::now())
}
fn unique_suffix_at(now: SystemTime) -> String {
    // Wall-clock resolution is not uniqueness, especially in parallel Windows tests.
    // PID separates concurrent processes; the counter separates calls within one process.
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        now.duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn verify_asset(doc: &Document, asset: &Asset) -> Result<(), String> {
    if !["png", "jpg", "webp", "bmp"].contains(&asset.extension.as_str()) {
        return Err("无效的原图扩展名".into());
    }
    if (doc.image.width, doc.image.height) != (asset.pixels.width(), asset.pixels.height()) {
        return Err("批注图片尺寸与原图不一致".into());
    }
    Ok(())
}

pub fn feedback_text(doc: &Document) -> String {
    let mut text = format!(
        "# 图片修改意见\n\n图片：{}（{} × {} 像素）\n坐标以原图像素为准；原图保持不变。\n\n",
        doc.image.file_name, doc.image.width, doc.image.height
    );
    if has_clipped_block(doc) {
        text.push_str("注意：图块超出原图边界的部分不会出现在参考图中。\n\n");
    }
    for (index, note) in doc.annotations.iter().enumerate() {
        let location = match &note.kind {
            AnnotationKind::Point { point } => format!("点位置 ({:.0}, {:.0})", point.x, point.y),
            AnnotationKind::Region { rect } => format!(
                "范围 ({:.0}, {:.0}, {:.0}, {:.0})",
                rect.x, rect.y, rect.width, rect.height
            ),
            AnnotationKind::Block {
                source,
                target,
                rotation,
            } => block_description(*source, *target, *rotation),
            AnnotationKind::Global => "全局意见".into(),
        };
        text.push_str(&format!(
            "{}. {}\n   {}\n\n",
            index + 1,
            location,
            if note.text.trim().is_empty() {
                "（按标记的位置／布局调整）"
            } else {
                note.text.trim()
            }
        ));
    }
    text
}

pub fn has_clipped_block(doc: &Document) -> bool {
    doc.annotations.iter().any(|note| match note.kind {
        AnnotationKind::Block {
            target, rotation, ..
        } => transform::corners(target, rotation).iter().any(|p| {
            p.x < -0.001
                || p.y < -0.001
                || p.x > doc.image.width as f32 + 0.001
                || p.y > doc.image.height as f32 + 0.001
        }),
        _ => false,
    })
}

/// Explicit machine-readable-enough geometry in copied feedback, without exposing
/// transform controls in the editing UI. Target is the rectangle before rotation.
pub fn block_description(source: Rect, target: Rect, rotation: f32) -> String {
    if rotation.abs() < 0.001 && (target.width / source.width - 1.).abs() < 0.0001 {
        return format!(
            "移动图块：来源 ({:.0}, {:.0}, {:.0}, {:.0}) → 目标 ({:.0}, {:.0}, {:.0}, {:.0})",
            source.x,
            source.y,
            source.width,
            source.height,
            target.x,
            target.y,
            target.width,
            target.height
        );
    }
    let base = format!(
        "移动图块：来源 ({:.2}, {:.2}, {:.2}, {:.2}) → 目标 ({:.2}, {:.2}, {:.2}, {:.2})",
        source.x,
        source.y,
        source.width,
        source.height,
        target.x,
        target.y,
        target.width,
        target.height
    );
    let c = transform::center(target);
    format!(
        "{base}；等比缩放 {:.4} 倍，以目标中心 ({:.2}, {:.2}) 为轴顺时针旋转 {:+.2}°；目标坐标为旋转前矩形，超出原图部分在参考图中裁剪",
        target.width / source.width,
        c.x,
        c.y,
        rotation
    )
}

/// Sample the original pixels using the same rounded source bounds used by the
/// source placeholder. Outside the rotated rectangle is transparent.
struct BlockSampler<'a> {
    pixels: &'a RgbaImage,
    target: Rect,
    center: Point,
    sin: f32,
    cos: f32,
    x0: u32,
    y0: u32,
    width: u32,
    height: u32,
}
impl<'a> BlockSampler<'a> {
    fn new(pixels: &'a RgbaImage, source: Rect, target: Rect, rotation: f32) -> Self {
        let x0 = source.x.round().max(0.) as u32;
        let y0 = source.y.round().max(0.) as u32;
        let (sin, cos) = rotation.to_radians().sin_cos();
        Self {
            pixels,
            target,
            center: transform::center(target),
            sin,
            cos,
            x0,
            y0,
            width: ((source.x + source.width).round() as u32).saturating_sub(x0),
            height: ((source.y + source.height).round() as u32).saturating_sub(y0),
        }
    }
    #[inline]
    fn sample(&self, p: Point, smooth: bool) -> Option<Rgba<u8>> {
        let x = p.x - self.center.x;
        let y = p.y - self.center.y;
        let u = (x * self.cos + y * self.sin) / self.target.width + 0.5;
        let v = (-x * self.sin + y * self.cos) / self.target.height + 0.5;
        if !(0.0..1.0).contains(&u)
            || !(0.0..1.0).contains(&v)
            || self.width == 0
            || self.height == 0
        {
            return None;
        }
        if !smooth {
            let x = self.x0 + ((u * self.width as f32) as u32).min(self.width - 1);
            let y = self.y0 + ((v * self.height as f32) as u32).min(self.height - 1);
            return self.pixels.get_pixel_checked(x, y).copied();
        }
        // Interpolate premultiplied colors so transparent sources do not develop dark fringes.
        let x = (u * self.width as f32 - 0.5).clamp(0., (self.width - 1) as f32);
        let y = (v * self.height as f32 - 0.5).clamp(0., (self.height - 1) as f32);
        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let fx = x - x0 as f32;
        let fy = y - y0 as f32;
        let samples = [
            (
                *self.pixels.get_pixel_checked(self.x0 + x0, self.y0 + y0)?,
                (1. - fx) * (1. - fy),
            ),
            (
                *self.pixels.get_pixel_checked(self.x0 + x1, self.y0 + y0)?,
                fx * (1. - fy),
            ),
            (
                *self.pixels.get_pixel_checked(self.x0 + x0, self.y0 + y1)?,
                (1. - fx) * fy,
            ),
            (
                *self.pixels.get_pixel_checked(self.x0 + x1, self.y0 + y1)?,
                fx * fy,
            ),
        ];
        let mut alpha = 0.;
        let mut rgb = [0.; 3];
        for (pixel, weight) in samples {
            let a = pixel[3] as f32 * weight;
            alpha += a;
            for i in 0..3 {
                rgb[i] += pixel[i] as f32 * a;
            }
        }
        if alpha <= 0. {
            return Some(Rgba([0, 0, 0, 0]));
        }
        Some(Rgba([
            (rgb[0] / alpha).round() as u8,
            (rgb[1] / alpha).round() as u8,
            (rgb[2] / alpha).round() as u8,
            alpha.round() as u8,
        ]))
    }
}

/// Bounded screen-resolution bitmap, so software and GPU renderers show the
/// same rotated pixels without relying on Slint's unsupported software rotation.
pub fn rotated_preview(
    pixels: &RgbaImage,
    source: Rect,
    target: Rect,
    rotation: f32,
    view_scale: f32,
    smooth: bool,
) -> RgbaImage {
    let b = transform::bounds(target, rotation);
    let screen_w = (b.width * view_scale).max(1.);
    let screen_h = (b.height * view_scale).max(1.);
    let cap = (1600. / screen_w)
        .min(1600. / screen_h)
        .min((2_000_000. / (screen_w * screen_h)).sqrt())
        .min(1.);
    let w = (screen_w * cap).ceil().max(1.) as u32;
    let h = (screen_h * cap).ceil().max(1.) as u32;
    let mut image = RgbaImage::new(w, h);
    let sampler = BlockSampler::new(pixels, source, target, rotation);
    for y in 0..h {
        for x in 0..w {
            let p = Point {
                x: b.x + (x as f32 + 0.5) / w as f32 * b.width,
                y: b.y + (y as f32 + 0.5) / h as f32 * b.height,
            };
            if let Some(color) = sampler.sample(p, smooth) {
                image.put_pixel(x, y, color);
            }
        }
    }
    image
}

fn put(img: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
        img.put_pixel(x as u32, y as u32, color);
    }
}
fn line(img: &mut RgbaImage, x1: i32, y1: i32, x2: i32, y2: i32, color: Rgba<u8>, thickness: i32) {
    // Clip before stepping: a valid block may extend far outside the image.
    let (x, y, dx, dy) = (
        x1 as f64,
        y1 as f64,
        x2 as f64 - x1 as f64,
        y2 as f64 - y1 as f64,
    );
    let (mut lo, mut hi): (f64, f64) = (0., 1.);
    let pad = thickness as f64;
    for (p, q) in [
        (-dx, x + pad),
        (dx, img.width() as f64 + pad - x),
        (-dy, y + pad),
        (dy, img.height() as f64 + pad - y),
    ] {
        if p == 0. {
            if q < 0. {
                return;
            }
        } else if p < 0. {
            lo = lo.max(q / p);
        } else {
            hi = hi.min(q / p);
        }
        if lo > hi {
            return;
        }
    }
    let (x1, y1, x2, y2) = (
        (x + dx * lo).round() as i32,
        (y + dy * lo).round() as i32,
        (x + dx * hi).round() as i32,
        (y + dy * hi).round() as i32,
    );
    let steps = (x2 - x1).abs().max((y2 - y1).abs()).max(1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = (x1 as f32 + (x2 - x1) as f32 * t).round() as i32;
        let y = (y1 as f32 + (y2 - y1) as f32 * t).round() as i32;
        for dy in -thickness / 2..=thickness / 2 {
            for dx in -thickness / 2..=thickness / 2 {
                put(img, x + dx, y + dy, color);
            }
        }
    }
}
const DIGITS: [[u8; 5]; 10] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
];
fn pin(img: &mut RgbaImage, x: f32, y: f32, number: usize, scale: i32) {
    let digits = number.to_string();
    let r = (10 * scale).max((digits.len() as i32 * 4 * scale + 4 * scale) / 2);
    let cx = (x.round() as i32).clamp(r, (img.width() as i32 - r - 1).max(r));
    let cy = (y.round() as i32).clamp(r, (img.height() as i32 - r - 1).max(r));
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r * r {
                put(img, cx + dx, cy + dy, Rgba([220, 43, 57, 255]));
            }
        }
    }
    let start = cx - (digits.len() as i32 * 4 - 1) * scale / 2;
    for (i, ch) in digits.bytes().enumerate() {
        for (row, bits) in DIGITS[(ch - b'0') as usize].iter().enumerate() {
            for col in 0..3 {
                if bits & (1 << (2 - col)) != 0 {
                    for py in 0..scale {
                        for px in 0..scale {
                            put(
                                img,
                                start + (i as i32 * 4 + col) * scale + px,
                                cy - 5 * scale / 2 + row as i32 * scale + py,
                                Rgba([255, 255, 255, 255]),
                            );
                        }
                    }
                }
            }
        }
    }
}

pub fn render_reference(doc: &Document, asset: &Asset) -> Result<RgbaImage, String> {
    doc.validate()?;
    verify_asset(doc, asset)?;
    let mut result = asset.pixels.clone();
    let scale = ((doc.image.width.min(doc.image.height) as f32 / 700.0).round() as i32).clamp(1, 4);
    let red = Rgba([220, 43, 57, 255]);
    let muted = Rgba([229, 230, 235, 255]);
    // One rasterization policy for clearing AND copying; fractional edges cannot lose columns.
    let raster = |r: Rect| {
        let x = r.x.round() as u32;
        let y = r.y.round() as u32;
        (
            x,
            y,
            ((r.x + r.width).round() as u32).saturating_sub(x),
            ((r.y + r.height).round() as u32).saturating_sub(y),
        )
    };
    // Cover every removed source first, then composite targets; never crop a previously moved layer.
    for note in &doc.annotations {
        if let AnnotationKind::Block { source, .. } = note.kind {
            let (x, y, w, h) = raster(source);
            for y in y..y + h {
                for x in x..x + w {
                    if x < result.width() && y < result.height() {
                        result.put_pixel(x, y, muted);
                    }
                }
            }
        }
    }
    for note in &doc.annotations {
        if let AnnotationKind::Block {
            source,
            target,
            rotation,
        } = note.kind
        {
            let b = transform::bounds(target, rotation);
            let x0 = (b.x.floor() as i64).clamp(0, i64::from(result.width())) as u32;
            let y0 = (b.y.floor() as i64).clamp(0, i64::from(result.height())) as u32;
            let x1 = ((b.x + b.width).ceil() as i64).clamp(0, i64::from(result.width())) as u32;
            let y1 = ((b.y + b.height).ceil() as i64).clamp(0, i64::from(result.height())) as u32;
            let sampler = BlockSampler::new(&asset.pixels, source, target, rotation);
            for y in y0..y1 {
                for x in x0..x1 {
                    if let Some(color) = sampler.sample(
                        Point {
                            x: x as f32 + 0.5,
                            y: y as f32 + 0.5,
                        },
                        true,
                    ) {
                        result.get_pixel_mut(x, y).blend(&color);
                    }
                }
            }
        }
    }
    for (index, note) in doc.annotations.iter().enumerate() {
        let geometry = match note.kind {
            AnnotationKind::Region { rect } => Some((rect, 0.)),
            AnnotationKind::Block {
                target, rotation, ..
            } => Some((target, rotation)),
            _ => None,
        };
        if let Some((rect, angle)) = geometry {
            let corners = transform::corners(rect, angle);
            for i in 0..4 {
                let a = corners[i];
                let b = corners[(i + 1) % 4];
                line(
                    &mut result,
                    a.x.round() as i32,
                    a.y.round() as i32,
                    b.x.round() as i32,
                    b.y.round() as i32,
                    red,
                    2 * scale,
                );
            }
            pin(&mut result, corners[0].x, corners[0].y, index + 1, scale);
        } else if let AnnotationKind::Point { point } = note.kind {
            pin(&mut result, point.x, point.y, index + 1, scale);
        }
    }
    Ok(result)
}

/// Create only the durable images needed for copying; never overwrite an earlier snapshot.
pub fn prepare_snapshot(parent: &Path, doc: &Document, asset: &Asset) -> Result<PathBuf, String> {
    doc.validate()?;
    verify_asset(doc, asset)?;
    let folder = parent.join(format!("image-feedback-{}", unique_suffix()));
    fs::create_dir(&folder).map_err(|e| format!("无法创建图片快照目录：{e}"))?;
    fs::write(
        folder.join(format!("original.{}", asset.extension)),
        &asset.original,
    )
    .map_err(|e| e.to_string())?;
    let reference = render_reference(doc, asset)?;
    fs::write(folder.join("reference.png"), png_bytes(&reference)?).map_err(|e| e.to_string())?;
    Ok(folder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Point;

    #[test]
    fn suffixes_remain_unique_across_threads_when_the_clock_does_not_advance() {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    (0..1000)
                        .map(|_| unique_suffix_at(UNIX_EPOCH))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let suffixes: std::collections::HashSet<_> = workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(suffixes.len(), 8000);
    }

    fn asset() -> Asset {
        Asset::from_rgba(
            RgbaImage::from_pixel(80, 60, Rgba([30, 80, 140, 255])),
            "test.png",
        )
        .unwrap()
    }
    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("image-feedback-test-{}", unique_suffix()));
        fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn rejects_mismatched_dimensions() {
        let a = asset();
        let d = Document::new("x.png", 81, 60).unwrap();
        assert!(render_reference(&d, &a).is_err());
    }
    #[test]
    fn reference_never_mutates_original() {
        let a = asset();
        let before = a.pixels.clone();
        let mut d = Document::new("x.png", 80, 60).unwrap();
        let id = d
            .add_region(Rect {
                x: 5.,
                y: 5.,
                width: 20.,
                height: 20.,
            })
            .unwrap();
        d.move_annotation(id, 30., 10.).unwrap();
        let r = render_reference(&d, &a).unwrap();
        assert_eq!(a.pixels, before);
        assert_ne!(r, before);
    }
    #[test]
    fn snapshots_keep_only_images_and_never_overwrite_previous_results() {
        let a = asset();
        let mut d = Document::new("x.png", 80, 60).unwrap();
        let id = d.add_point(Point { x: 40., y: 40. }).unwrap();
        d.set_text(id, "移动按钮".into()).unwrap();
        let dir = temp();
        let first = prepare_snapshot(&dir, &d, &a).unwrap();
        let second = prepare_snapshot(&dir, &d, &a).unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read_dir(&first).unwrap().count(), 2);
        assert_eq!(fs::read(first.join("original.png")).unwrap(), a.original);
        assert!(first.join("reference.png").is_file());
        assert!(!first.join("feedback-package.zip").exists());
        assert!(!first.join("feedback.json").exists());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_invalid_image() {
        assert!(Asset::from_bytes(b"not an image".to_vec(), "x.png".into()).is_err());
    }
    #[test]
    fn reference_and_preview_rotate_scale_and_clip_original_pixels() {
        let white = Rgba([255, 255, 255, 255]);
        let colors = [
            Rgba([220, 20, 20, 255]),
            Rgba([20, 180, 20, 255]),
            Rgba([20, 20, 220, 255]),
            Rgba([200, 180, 20, 255]),
        ];
        let mut pixels = RgbaImage::from_pixel(160, 120, white);
        for y in 0..20 {
            for x in 0..20 {
                pixels.put_pixel(10 + x, 10 + y, colors[(y / 10 * 2 + x / 10) as usize]);
            }
        }
        let a = Asset::from_rgba(pixels, "x.png").unwrap();
        let original = a.pixels.clone();
        let mut d = Document::new("x.png", 160, 120).unwrap();
        let source = Rect {
            x: 10.,
            y: 10.,
            width: 20.,
            height: 20.,
        };
        let target = Rect {
            x: 70.,
            y: 40.,
            width: 60.,
            height: 60.,
        };
        let id = d.add_region(source).unwrap();
        d.transform_annotation(id, target, 90.).unwrap();
        let r = render_reference(&d, &a).unwrap();
        for (x, y, color) in [
            (115, 55, colors[0]),
            (115, 85, colors[1]),
            (85, 55, colors[2]),
            (85, 85, colors[3]),
        ] {
            assert_eq!(*r.get_pixel(x, y), color);
        }
        assert_eq!(*r.get_pixel(20, 20), Rgba([229, 230, 235, 255]));
        assert_eq!(a.pixels, original);
        let preview = rotated_preview(&a.pixels, source, target, 90., 1., true);
        assert_eq!(*preview.get_pixel(45, 15), colors[0]);
        assert_eq!(*preview.get_pixel(15, 45), colors[3]);
        let text = feedback_text(&d);
        assert!(text.contains("等比缩放 3.0000 倍") && text.contains("顺时针旋转 +90.00°"));
        d.transform_annotation(
            id,
            Rect {
                x: -40.,
                y: -40.,
                ..target
            },
            45.,
        )
        .unwrap();
        assert!(has_clipped_block(&d));
        assert!(feedback_text(&d).contains("不会出现在参考图中"));
        assert_eq!(render_reference(&d, &a).unwrap().dimensions(), (160, 120));
    }
    #[test]
    fn fractional_source_clearing_uses_the_same_columns_as_copying() {
        let a = Asset::from_rgba(
            RgbaImage::from_pixel(220, 150, Rgba([30, 80, 140, 255])),
            "x.png",
        )
        .unwrap();
        let mut d = Document::new("x.png", 220, 150).unwrap();
        let id = d
            .add_region(Rect {
                x: 10.4,
                y: 30.4,
                width: 40.4,
                height: 40.4,
            })
            .unwrap();
        d.move_annotation(id, 100., 0.).unwrap();
        let r = render_reference(&d, &a).unwrap();
        assert_eq!(*r.get_pixel(50, 55), Rgba([229, 230, 235, 255]));
        assert_eq!(*r.get_pixel(149, 55), Rgba([30, 80, 140, 255]));
    }
    #[test]
    fn untrusted_snapshot_extension_is_rejected() {
        let mut a = asset();
        a.extension = "../../other".into();
        let d = Document::new("x.png", 80, 60).unwrap();
        let dir = temp();
        assert!(prepare_snapshot(&dir, &d, &a).is_err());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        fs::remove_dir_all(dir).unwrap();
    }
}
