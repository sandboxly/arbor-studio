//! Recently opened and saved projects, remembered between sessions in a small file in the
//! user's settings folder (macOS: ~/Library/Application Support/Arbor Studio, Windows:
//! %APPDATA%\Arbor Studio, Linux: ~/.config/arbor-studio; ARBOR_CONFIG_DIR overrides it).

use std::path::{Path, PathBuf};

const MAX: usize = 10;

/// Bumped on every change, so menus know when to rebuild.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

fn config_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("ARBOR_CONFIG_DIR") {
        return Some(PathBuf::from(d));
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Arbor Studio"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|h| PathBuf::from(h).join("Arbor Studio"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|d| d.join("arbor-studio"))
    }
}

fn file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("recent.json"))
}

/// The list, most recent first.
pub fn list() -> Vec<PathBuf> {
    file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|s| serde_json::from_str::<Vec<PathBuf>>(&s).ok())
        .unwrap_or_default()
}

fn store(v: &[PathBuf]) {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let (Some(d), Some(f)) = (config_dir(), file()) {
        let _ = std::fs::create_dir_all(d);
        if let Ok(s) = serde_json::to_string_pretty(v) {
            let _ = std::fs::write(f, s);
        }
    }
}

/// Put a project at the top of the list (after opening or saving it).
pub fn add(path: &Path) {
    let p = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut v = list();
    v.retain(|q| q != &p);
    v.insert(0, p);
    v.truncate(MAX);
    store(&v);
}

pub fn remove(path: &Path) {
    let mut v = list();
    v.retain(|q| q != path);
    store(&v);
}

pub fn clear() {
    store(&[]);
}

/// The full path, with the home folder shortened to ~ on macOS and Linux.
pub fn short_path(path: &Path) -> String {
    #[cfg(not(target_os = "windows"))]
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if let Ok(rest) = path.strip_prefix(&home) {
            return if rest.as_os_str().is_empty() { "~".into() } else { format!("~/{}", rest.display()) };
        }
    }
    path.display().to_string()
}

/// Menu label: the project's name without the folder and the `.arbor.json` ending.
pub fn label(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string());
    name.strip_suffix(".arbor.json").or_else(|| name.strip_suffix(".json")).map(str::to_string).unwrap_or(name)
}
