//! One cancellable native analyzer, one latest queued image, one latest result.
//! No polling thread, subprocess, browser, network or pixel work on pointer motion.
use crate::{
    document::Rect,
    smart_selection::{Candidate, detect_with_cancel},
    storage::Asset,
};
use image::imageops::FilterType;
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
};

pub struct AnalysisResult {
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub candidates: Result<Vec<Candidate>, String>,
}
struct Request {
    generation: u64,
    asset: Arc<Asset>,
}
#[derive(Default)]
struct Queue {
    latest: Option<Request>,
    result: Option<AnalysisResult>,
    shutdown: bool,
}
pub struct AnalysisService {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    epoch: Arc<AtomicU64>,
    worker: Option<JoinHandle<()>>,
}
impl AnalysisService {
    pub fn new() -> Self {
        let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let epoch = Arc::new(AtomicU64::new(0));
        let shared = queue.clone();
        let current = epoch.clone();
        let worker = thread::Builder::new()
            .name("image-feedback-analysis".into())
            .spawn(move || {
                loop {
                    let request = {
                        let (state, signal) = (&shared.0, &shared.1);
                        let mut guard = state.lock().unwrap_or_else(|p| p.into_inner());
                        while !guard.shutdown && guard.latest.is_none() {
                            guard = signal.wait(guard).unwrap_or_else(|p| p.into_inner());
                        }
                        if guard.shutdown {
                            break;
                        }
                        guard.latest.take().unwrap()
                    };
                    let cancelled = || current.load(Ordering::Relaxed) != request.generation;
                    if cancelled() {
                        continue;
                    }
                    let (width, height) = request.asset.pixels.dimensions();
                    let ratio = (1600. / width.max(height) as f32)
                        .min((1_200_000. / (width as f64 * height as f64)).sqrt() as f32)
                        .min(1.);
                    let (w, h) = (
                        (width as f32 * ratio).round().max(1.) as u32,
                        (height as f32 * ratio).round().max(1.) as u32,
                    );
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if w < 8 || h < 8 {
                            return Ok(Vec::new());
                        }
                        let sample = image::imageops::resize(
                            &request.asset.pixels,
                            w,
                            h,
                            FilterType::Triangle,
                        );
                        if cancelled() {
                            return Err("cancelled".into());
                        }
                        let mut found =
                            detect_with_cancel(w, h, sample.as_raw(), ratio.max(0.25), cancelled)?;
                        let sx = width as f32 / w as f32;
                        let sy = height as f32 / h as f32;
                        for item in &mut found {
                            let left = (item.rect.x * sx).floor().max(0.);
                            let top = (item.rect.y * sy).floor().max(0.);
                            let right = ((item.rect.x + item.rect.width) * sx)
                                .ceil()
                                .min(width as f32);
                            let bottom = ((item.rect.y + item.rect.height) * sy)
                                .ceil()
                                .min(height as f32);
                            item.rect = Rect {
                                x: left,
                                y: top,
                                width: right - left,
                                height: bottom - top,
                            };
                            if let Some(height) = item.font_height.as_mut() {
                                *height *= sy;
                            }
                        }
                        found.retain(|c| c.rect.width >= 2. && c.rect.height >= 2.);
                        Ok(found)
                    }))
                    .unwrap_or_else(|_| Err("图片分析异常，仍可手动框选".into()));
                    if cancelled() {
                        continue;
                    }
                    let mut guard = shared.0.lock().unwrap_or_else(|p| p.into_inner());
                    if !guard.shutdown && current.load(Ordering::Relaxed) == request.generation {
                        guard.result = Some(AnalysisResult {
                            generation: request.generation,
                            width,
                            height,
                            candidates: result,
                        });
                    }
                }
            })
            .expect("could not start native image analysis worker");
        Self {
            queue,
            epoch,
            worker: Some(worker),
        }
    }
    pub fn submit(&self, asset: Arc<Asset>) -> u64 {
        let generation = self.epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let mut state = self.queue.0.lock().unwrap_or_else(|p| p.into_inner());
        state.result = None;
        state.latest = Some(Request { generation, asset });
        drop(state);
        self.queue.1.notify_one();
        generation
    }
    pub fn cancel(&self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
        let mut state = self.queue.0.lock().unwrap_or_else(|p| p.into_inner());
        state.latest = None;
        state.result = None;
    }
    pub fn take_result(&self) -> Option<AnalysisResult> {
        self.queue
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .result
            .take()
    }
}
impl Default for AnalysisService {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for AnalysisService {
    fn drop(&mut self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
        {
            let mut state = self.queue.0.lock().unwrap_or_else(|p| p.into_inner());
            state.shutdown = true;
            state.latest = None;
            state.result = None;
        }
        self.queue.1.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    fn asset(w: u32, h: u32) -> Arc<Asset> {
        Arc::new(
            Asset::from_rgba(
                RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255])),
                "x.png",
            )
            .unwrap(),
        )
    }
    fn wait(service: &AnalysisService) -> AnalysisResult {
        let start = std::time::Instant::now();
        loop {
            if let Some(r) = service.take_result() {
                return r;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(10));
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
    }
    #[test]
    fn tiny_images_fall_back_without_detection_error() {
        let s = AnalysisService::new();
        let generation = s.submit(asset(1, 1));
        let r = wait(&s);
        assert_eq!(generation, r.generation);
        assert!(r.candidates.unwrap().is_empty());
    }
    #[test]
    fn only_latest_generation_can_publish_results() {
        let s = AnalysisService::new();
        s.submit(asset(400, 300));
        s.cancel();
        let last = s.submit(asset(80, 60));
        let r = wait(&s);
        assert_eq!(r.generation, last);
        assert_eq!((r.width, r.height), (80, 60));
        assert!(r.candidates.is_ok());
    }
    #[test]
    fn queued_requests_are_replaced_not_accumulated() {
        let s = AnalysisService::new();
        let a = asset(100, 100);
        let mut last = 0;
        for _ in 0..50 {
            last = s.submit(a.clone());
        }
        assert_eq!(wait(&s).generation, last);
    }
    #[test]
    fn drop_releases_queued_images_and_stops_worker() {
        let a = asset(120, 80);
        let weak = Arc::downgrade(&a);
        let s = AnalysisService::new();
        s.submit(a.clone());
        drop(a);
        drop(s);
        assert!(weak.upgrade().is_none());
    }
}
