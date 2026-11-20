//! Scripted window pictures for the documentation: `arbor_studio --shot-script steps.json`
//! opens the window, carries out the steps one after another and exits when they are done.
//!
//! The script is a JSON list of steps:
//!   {"tool": "load_preset", "args": {"name": "Oak"}}   any MCP tool except capture_window / render_preview
//!   {"ui": {"material": 1}}                            open a material editor (null closes it)
//!   {"ui": {"curve": [generator_id, "taper"]}}         open the curve popup of a setting row
//!   {"ui": {"expand": ["Forces", "Pruning"]}}          open collapsed sections of the properties panel
//!   {"ui": {"scroll": 400}}                            scroll the properties panel (points)
//!   {"ui": {"tab": "stats"}}                           bottom panel tab: "stats" or "activity"
//!   {"ui": {"connect": true}} / {"ui": {"about": true}} dialogs
//!   {"ui": {"node_mode": true, "node": 12345}}         branch picking mode and the picked branch key
//!   {"ui": {"close": true}}                            close popups, dialogs and the material editor
//!   {"wait": 10}                                       let a few frames pass
//!   {"capture": "images/window.png", "rect": [x, y, w, h]}   save the window (or a part, in points)
//! Relative capture paths are relative to the script's folder.

use serde_json::Value;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

pub struct ShotScript {
    pub steps: VecDeque<Value>,
    pub base: PathBuf,
    pub wait: u32,
    /// A capture waiting for its picture: (file, part of the window in points).
    pub pending: Option<(PathBuf, Option<egui::Rect>)>,
    /// The curve popup the script opened, to close it again.
    pub popup: Option<egui::Id>,
}

impl ShotScript {
    pub fn load(path: &Path) -> Result<ShotScript, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let steps: Vec<Value> = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(ShotScript {
            steps: steps.into(),
            base: path.parent().map(Path::to_path_buf).unwrap_or_default(),
            // let the window settle first
            wait: 30,
            pending: None,
            popup: None,
        })
    }

    pub fn resolve(&self, p: &str) -> PathBuf {
        let p = PathBuf::from(p);
        if p.is_absolute() {
            p
        } else {
            self.base.join(p)
        }
    }
}

/// Save a captured window picture, cut to `rect` (points) when given.
pub fn save(img: &egui::ColorImage, path: &Path, rect: Option<egui::Rect>, pixels_per_point: f32) {
    let img = match rect {
        Some(r) => img.region(&r, Some(pixels_per_point)),
        None => img.clone(),
    };
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    // RGB, so pictures can be saved as .png or .jpg
    let rgb = image::RgbImage::from_raw(img.size[0] as u32, img.size[1] as u32, img.pixels.iter().flat_map(|c| { let [r, g, b, _] = c.to_array(); [r, g, b] }).collect());
    match rgb {
        Some(rgb) => {
            if let Err(e) = rgb.save(path) {
                eprintln!("could not save {}: {e}", path.display());
            }
        }
        None => eprintln!("could not save {}: empty picture", path.display()),
    }
}
