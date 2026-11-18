//! 사진 미리보기·썸네일 — 백그라운드에서 줄여 읽고, 화면에서는 텍스처로 보관합니다.
//!
//! 같은 파일·크기는 한 번만 읽고(수정 시간이 바뀌면 다시), 너무 큰 파일(80 MB 넘음)은 건너뜁니다.

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

const MAX_FILE: u64 = 80 * 1024 * 1024;
const MAX_CACHED: usize = 600;

type Key = (PathBuf, u32, Option<SystemTime>);

enum Slot {
    Loading,
    Decoded(ColorImage),
    Ready(TextureHandle),
    Failed,
}

struct Cache {
    slots: HashMap<Key, Slot>,
    queue: Sender<(Key, egui::Context)>,
}

fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<(Key, egui::Context)>();
        // 한 줄로 차례차례 (디스크·CPU를 너무 쓰지 않도록)
        std::thread::spawn(move || {
            while let Ok((key, ctx)) = rx.recv() {
                let img = decode(&key.0, key.1);
                if let Ok(mut c) = cache().lock() {
                    c.slots.insert(key, img.map(Slot::Decoded).unwrap_or(Slot::Failed));
                }
                ctx.request_repaint();
            }
        });
        Mutex::new(Cache { slots: HashMap::new(), queue: tx })
    })
}

/// 사진 파일인지 (확장자로)
pub fn is_image(path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico")
}

fn decode(path: &Path, max: u32) -> Option<ColorImage> {
    if std::fs::metadata(path).ok()?.len() > MAX_FILE {
        return None;
    }
    let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
    let rgba = img.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

/// `path`를 최대 `max`픽셀로 줄인 텍스처. 아직 읽는 중이거나 사진이 아니면 `None`.
pub fn texture(ctx: &egui::Context, path: &Path, max: u32) -> Option<TextureHandle> {
    if !is_image(path) {
        return None;
    }
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let key: Key = (path.to_path_buf(), max, mtime);
    let mut c = cache().lock().ok()?;
    match c.slots.get(&key) {
        Some(Slot::Ready(t)) => return Some(t.clone()),
        Some(Slot::Loading | Slot::Failed) => return None,
        Some(Slot::Decoded(_)) => {}
        None => {
            if c.slots.len() > MAX_CACHED {
                c.slots.retain(|_, s| matches!(s, Slot::Loading));
            }
            c.slots.insert(key.clone(), Slot::Loading);
            let _ = c.queue.send((key, ctx.clone()));
            return None;
        }
    }
    let Some(Slot::Decoded(img)) = c.slots.remove(&key) else { return None };
    let tex = ctx.load_texture(format!("thumb:{}:{max}", path.display()), img, TextureOptions::LINEAR);
    c.slots.insert(key, Slot::Ready(tex.clone()));
    Some(tex)
}

/// 텍스처를 `rect` 안에 비율을 지켜 가운데 그리기
pub fn paint_fit(ui: &egui::Ui, rect: egui::Rect, tex: &TextureHandle) {
    let size = tex.size_vec2();
    let scale = (rect.width() / size.x).min(rect.height() / size.y).min(1.0);
    let r = egui::Rect::from_center_size(rect.center(), size * scale);
    ui.painter().image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
}
