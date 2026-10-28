//! Bounded local images and durable snapshots. No network or host IDs.
use crate::document::{AnnotationKind, Document, Rect};
use image::{ImageEncoder, Rgba, RgbaImage};
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
    for (index, note) in doc.annotations.iter().enumerate() {
        let location = match &note.kind {
            AnnotationKind::Point { point } => format!("点位置 ({:.0}, {:.0})", point.x, point.y),
            AnnotationKind::Region { rect } => format!(
                "范围 ({:.0}, {:.0}, {:.0}, {:.0})",
                rect.x, rect.y, rect.width, rect.height
            ),
            AnnotationKind::Block { source, target } => format!(
                "移动图块：来源 ({:.0}, {:.0}, {:.0}, {:.0}) → 目标 ({:.0}, {:.0}, {:.0}, {:.0})",
                source.x,
                source.y,
                source.width,
                source.height,
                target.x,
                target.y,
                target.width,
                target.height
            ),
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

fn put(img: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
        img.put_pixel(x as u32, y as u32, color);
    }
}
fn line(img: &mut RgbaImage, x1: i32, y1: i32, x2: i32, y2: i32, color: Rgba<u8>, thickness: i32) {
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
        if let AnnotationKind::Block { source, target } = note.kind {
            let (sx0, sy0, w, h) = raster(source);
            for dy in 0..h {
                for dx in 0..w {
                    let sx = sx0 + dx;
                    let sy = sy0 + dy;
                    let tx = target.x.round() as u32 + dx;
                    let ty = target.y.round() as u32 + dy;
                    if sx < asset.pixels.width()
                        && sy < asset.pixels.height()
                        && tx < result.width()
                        && ty < result.height()
                    {
                        result.put_pixel(tx, ty, *asset.pixels.get_pixel(sx, sy));
                    }
                }
            }
        }
    }
    for (index, note) in doc.annotations.iter().enumerate() {
        let rect = match note.kind {
            AnnotationKind::Region { rect } => Some(rect),
            AnnotationKind::Block { target, .. } => Some(target),
            _ => None,
        };
        if let Some(Rect {
            x,
            y,
            width,
            height,
        }) = rect
        {
            let (x, y, x2, y2) = (
                x.round() as i32,
                y.round() as i32,
                (x + width).round() as i32,
                (y + height).round() as i32,
            );
            line(&mut result, x, y, x2, y, red, 2 * scale);
            line(&mut result, x2, y, x2, y2, red, 2 * scale);
            line(&mut result, x2, y2, x, y2, red, 2 * scale);
            line(&mut result, x, y2, x, y, red, 2 * scale);
            pin(&mut result, x as f32, y as f32, index + 1, scale);
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
