//! Loads material images (PNG/JPEG) once and keeps them cached by path + modification time.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

pub type Image = Arc<image::RgbaImage>;

const MAX_SIZE: u32 = 1024;

fn cache() -> &'static Mutex<HashMap<String, (Option<SystemTime>, Result<Image, String>)>> {
    static C: OnceLock<Mutex<HashMap<String, (Option<SystemTime>, Result<Image, String>)>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Load (or reuse) an image. Large images are scaled down to 1024 px for display and previews.
pub fn load(path: &str) -> Result<Image, String> {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Some((t, r)) = cache().lock().unwrap().get(path) {
        if *t == mtime {
            return r.clone();
        }
    }
    let r = (|| {
        if !Path::new(path).is_file() {
            return Err(format!("image not found: {path}"));
        }
        let img = image::open(path).map_err(|e| format!("cannot read image {path}: {e}"))?;
        let img = if img.width() > MAX_SIZE || img.height() > MAX_SIZE {
            img.resize(MAX_SIZE, MAX_SIZE, image::imageops::FilterType::Triangle)
        } else {
            img
        };
        Ok(Arc::new(img.to_rgba8()))
    })();
    cache().lock().unwrap().insert(path.to_string(), (mtime, r.clone()));
    r
}

/// Is this a readable PNG/JPEG? Returns the file's own size (exports embed or copy the file as
/// it is; only the viewport's and the billboard renderer's copy is scaled down to 1024 px).
pub fn check(path: &str) -> Result<(u32, u32), String> {
    let lower = path.to_ascii_lowercase();
    if !(lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg")) {
        return Err(format!("textures must be .png, .jpg or .jpeg files: {path}"));
    }
    load(path)?;
    image::image_dimensions(path).map_err(|e| format!("cannot read image {path}: {e}"))
}

/// The size of the copy the viewport and the billboard renderer use (at most 1024 px).
pub fn preview_size(path: &str) -> Option<(u32, u32)> {
    load(path).ok().map(|i| (i.width(), i.height()))
}

/// Nearest-neighbour sample with repeat wrapping; uv origin is the image's top-left.
pub fn sample(img: &image::RgbaImage, u: f32, v: f32) -> [u8; 4] {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let x = ((u.rem_euclid(1.0)) * w) as u32 % img.width();
    let y = ((v.rem_euclid(1.0)) * h) as u32 % img.height();
    img.get_pixel(x, y).0
}
