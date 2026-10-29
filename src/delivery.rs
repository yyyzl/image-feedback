//! Persistent, local-only delivery snapshots. No upload and no system clipboard access.
use crate::document::{AnnotationKind, Document};
use crate::storage::{self, Asset};
use base64::Engine;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MAX_EMBEDDED_BYTES: u64 = 4 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct PreparedFeedback {
    pub folder: PathBuf,
    pub original: PathBuf,
    pub reference: PathBuf,
}
impl PreparedFeedback {
    pub fn clipboard_text(&self, doc: &Document) -> String {
        self.clipboard_text_with_source(doc, None)
    }
    pub fn clipboard_text_with_source(&self, doc: &Document, source: Option<&Path>) -> String {
        let mut text = format!("原图：{}\n", source.unwrap_or(&self.original).display());
        if doc
            .annotations
            .iter()
            .any(|note| !matches!(note.kind, AnnotationKind::Global))
        {
            text.push_str("坐标：原图像素；范围=(x, y, 宽, 高)。\n");
        }
        if storage::has_clipped_block(doc) {
            text.push_str("注意：图块超出原图边界的部分不会出现在参考图中。\n");
        }
        text.push('\n');
        append_annotations(&mut text, doc);
        text
    }
    pub fn is_complete(&self) -> bool {
        self.folder.is_dir()
            && [&self.original, &self.reference]
                .iter()
                .all(|p| p.is_file())
    }
    /// Optional machine-readable format. Receivers must decode image data, not just read it as prose.
    pub fn embedded_text(&self, doc: &Document) -> Result<String, String> {
        let original_size = fs::metadata(&self.original)
            .map_err(|e| e.to_string())?
            .len();
        let reference_size = fs::metadata(&self.reference)
            .map_err(|e| e.to_string())?
            .len();
        if original_size.saturating_add(reference_size) > MAX_EMBEDDED_BYTES {
            return Err("两张图片合计超过 4 MiB，Base64 会过长。请另附原图和修改意见。".into());
        }
        let original = bounded_read(&self.original, MAX_EMBEDDED_BYTES)?;
        let reference = bounded_read(
            &self.reference,
            MAX_EMBEDDED_BYTES.saturating_sub(original.len() as u64),
        )?;
        let mime = match self.original.extension().and_then(|s| s.to_str()) {
            Some("jpg") => "image/jpeg",
            Some("webp") => "image/webp",
            Some("bmp") => "image/bmp",
            _ => "image/png",
        };
        let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let data = serde_json::json!({"schema":"image-feedback-embedded-1","instructions":"接收工具需先把 data_url 解码为图片附件或图像输入；Base64 文本本身不等于图片输入。","original":{"name":self.original.file_name().unwrap_or_default().to_string_lossy(),"mime":mime,"data_url":format!("data:{mime};base64,{}",encode(&original))},"reference":{"name":"reference.png","mime":"image/png","data_url":format!("data:image/png;base64,{}",encode(&reference))},"coordinate_space":"original-image-pixels","document":doc});
        serde_json::to_string_pretty(&data).map_err(|e| e.to_string())
    }
}
fn bounded_read(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err("图片数据超过便携复制上限，请另附原图和修改意见。".into());
    }
    Ok(bytes)
}
/// Prefer a disk origin only while it is byte-identical to the image actually being annotated.
pub fn verified_source_path(asset: &Asset) -> Option<PathBuf> {
    let path = asset.source_path.as_ref()?;
    let text = path.to_str()?;
    if text.contains(['\r', '\n']) {
        return None;
    }
    let bytes = bounded_read(path, storage::MAX_FILE_BYTES).ok()?;
    if bytes == asset.original {
        Some(path.clone())
    } else {
        None
    }
}
pub fn prepare(root: &Path, doc: &Document, asset: &Asset) -> Result<PreparedFeedback, String> {
    let root = std::path::absolute(root).map_err(|e| e.to_string())?;
    fs::create_dir_all(&root).map_err(|e| format!("无法创建本机图片快照目录：{e}"))?;
    let folder = storage::prepare_snapshot(&root, doc, asset)?;
    let output = PreparedFeedback {
        original: folder.join(format!("original.{}", asset.extension)),
        reference: folder.join("reference.png"),
        folder,
    };
    if !output.is_complete() {
        return Err("图片快照文件缺失".into());
    }
    Ok(output)
}
fn append_annotations(text: &mut String, doc: &Document) {
    for (index, note) in doc.annotations.iter().enumerate() {
        let location = match note.kind {
            AnnotationKind::Point { point } => format!("点位置 ({:.0}, {:.0})", point.x, point.y),
            AnnotationKind::Region { rect } => format!(
                "范围 ({:.0}, {:.0}, {:.0}, {:.0})",
                rect.x, rect.y, rect.width, rect.height
            ),
            AnnotationKind::Block {
                source,
                target,
                rotation,
            } => storage::block_description(source, target, rotation),
            AnnotationKind::Global => "全局意见".into(),
        };
        let comment = if note.text.trim().is_empty() {
            "（按标记的位置／布局调整）"
        } else {
            note.text.trim()
        };
        text.push_str(&format!(
            "{}. {}：{}\n",
            index + 1,
            location,
            comment.replace('\n', "\n   ")
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            // 系统时钟精度有限；并行测试不能只凭时间戳共用目录。
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let p = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/delivery-tests")
                .join(format!(
                    "{}-{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos(),
                    SEQUENCE.fetch_add(1, Ordering::Relaxed),
                ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn sample() -> (Document, Asset) {
        let a = Asset::from_rgba(
            RgbaImage::from_pixel(80, 60, Rgba([70, 120, 200, 180])),
            "剪贴板图片.png",
        )
        .unwrap();
        let mut d = Document::new(a.name.clone(), 80, 60).unwrap();
        let id = d.add_global().unwrap();
        d.set_text(id, "保留这条意见".into()).unwrap();
        (d, a)
    }
    #[test]
    fn missing_delivery_root_is_created_without_overwriting_files() {
        let t = Temp::new();
        let (d, a) = sample();
        let root = t.0.join("new-root/nested");
        let p = prepare(&root, &d, &a).unwrap();
        assert!(p.is_complete());
        assert!(p.folder.starts_with(&root));
    }
    #[test]
    fn pasted_image_is_persistent_and_unchanged() {
        let t = Temp::new();
        let (d, a) = sample();
        let p = prepare(&t.0, &d, &a).unwrap();
        assert_eq!(fs::read(&p.original).unwrap(), a.original);
        assert!(p.is_complete());
        assert_eq!(fs::read_dir(&p.folder).unwrap().count(), 2);
        let text = p.clipboard_text(&d);
        assert!(p.original.is_absolute());
        assert!(text.starts_with(&format!("原图：{}\n", p.original.display())));
        assert_eq!(text.matches(p.original.to_str().unwrap()).count(), 1);
        for extra in [
            "reference.png",
            "feedback-package.zip",
            "交付文件夹",
            "原图快照",
            "本机路径不是公开链接",
        ] {
            assert!(!text.contains(extra));
        }
        assert!(text.contains("1. 全局意见"));
        assert!(text.contains("保留这条意见"));
    }
    #[test]
    fn compact_clipboard_prefers_one_source_path_without_delivery_metadata() {
        let t = Temp::new();
        let (d, a) = sample();
        let p = prepare(&t.0, &d, &a).unwrap();
        let source = t.0.join("来源 图片.png");
        fs::write(&source, &a.original).unwrap();
        assert_eq!(
            p.clipboard_text_with_source(&d, Some(&source)),
            format!("原图：{}\n\n1. 全局意见：保留这条意见\n", source.display())
        );
    }
    #[test]
    fn compact_spatial_comments_keep_coordinates_multiline_text_and_moves() {
        let t = Temp::new();
        let (mut d, a) = sample();
        d.annotations.clear();
        let region = d
            .add_region(crate::document::Rect {
                x: 10.,
                y: 10.,
                width: 20.,
                height: 15.,
            })
            .unwrap();
        d.set_text(region, "第一行\n第二行".into()).unwrap();
        let block = d
            .add_region(crate::document::Rect {
                x: 30.,
                y: 30.,
                width: 10.,
                height: 10.,
            })
            .unwrap();
        d.move_annotation(block, 15., 5.).unwrap();
        let point = d
            .add_point(crate::document::Point { x: 70., y: 50. })
            .unwrap();
        d.set_text(point, "检查这个点".into()).unwrap();
        let p = prepare(&t.0, &d, &a).unwrap();
        let text = p.clipboard_text(&d);
        assert!(text.contains("坐标：原图像素；范围=(x, y, 宽, 高)。"));
        assert!(text.contains("1. 范围 (10, 10, 20, 15)：第一行\n   第二行\n"));
        assert!(text.contains("2. 移动图块：来源 (30, 30, 10, 10) → 目标 (45, 35, 10, 10)："));
        assert!(text.contains("3. 点位置 (70, 50)：检查这个点"));
        assert!(!text.contains("reference.png"));
    }
    #[test]
    fn repeated_snapshot_never_overwrites_previous_image() {
        let t = Temp::new();
        let (mut d, a) = sample();
        let first = prepare(&t.0, &d, &a).unwrap();
        let old = fs::read(&first.original).unwrap();
        d.set_text(1, "新版意见".into()).unwrap();
        let second = prepare(&t.0, &d, &a).unwrap();
        assert_ne!(first.folder, second.folder);
        assert_eq!(fs::read(first.original).unwrap(), old);
        fs::remove_file(&second.reference).unwrap();
        assert!(!second.is_complete());
    }
    #[test]
    fn original_disk_path_only_used_while_bytes_match() {
        let t = Temp::new();
        let (_, a) = sample();
        let source = t.0.join("来源 图片.png");
        fs::write(&source, &a.original).unwrap();
        let opened = Asset::open(&source).unwrap();
        assert_eq!(
            verified_source_path(&opened),
            Some(std::path::absolute(&source).unwrap())
        );
        fs::write(&source, b"changed").unwrap();
        assert!(verified_source_path(&opened).is_none());
        fs::remove_file(&source).unwrap();
        assert!(verified_source_path(&opened).is_none());
    }
    #[test]
    fn base64_roundtrip_contains_both_images_and_metadata() {
        let t = Temp::new();
        let (d, a) = sample();
        let p = prepare(&t.0, &d, &a).unwrap();
        let value: serde_json::Value = serde_json::from_str(&p.embedded_text(&d).unwrap()).unwrap();
        assert_eq!(value["schema"], "image-feedback-embedded-1");
        for (field, path) in [("original", &p.original), ("reference", &p.reference)] {
            let url = value[field]["data_url"].as_str().unwrap();
            let data = url.split_once(",").unwrap().1;
            assert_eq!(
                base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .unwrap(),
                fs::read(path).unwrap()
            );
        }
        assert_eq!(
            value["document"]["annotations"].as_array().unwrap().len(),
            1
        );
    }
    #[test]
    fn base64_and_extensions_are_bounded() {
        let t = Temp::new();
        let (d, mut a) = sample();
        let p = prepare(&t.0, &d, &a).unwrap();
        let f = File::options().write(true).open(&p.reference).unwrap();
        f.set_len(MAX_EMBEDDED_BYTES + 1).unwrap();
        assert!(p.embedded_text(&d).is_err());
        a.extension = "../../bad".into();
        assert!(prepare(&t.0, &d, &a).is_err());
    }
}
