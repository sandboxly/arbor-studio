//! State shared between the window and the MCP server thread.

use crate::model::Document;
use crate::render::ViewState;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    You,
    Agent,
    App,
}

pub struct LogLine {
    pub at: f32,
    pub source: Source,
    pub text: String,
}

#[derive(Default, Clone)]
pub struct McpStatus {
    pub url: Option<String>,
    pub error: Option<String>,
    pub calls: u64,
    pub client: Option<String>,
}

pub type ShotReply = Sender<Result<Vec<u8>, String>>;

pub struct Shared {
    pub doc: Document,
    /// Last state recorded in the undo history.
    pub committed: Document,
    pub undo: Vec<Document>,
    pub redo: Vec<Document>,
    pub selected: Option<u32>,
    pub view: ViewState,
    pub file: Option<PathBuf>,
    pub dirty: bool,
    pub log: VecDeque<LogLine>,
    pub started: Instant,
    pub screenshot_waiters: Vec<ShotReply>,
    pub mcp: McpStatus,
    /// Ask the viewport to re-frame the camera on the next frame.
    pub frame_camera: bool,
    /// Hand-selected branch instance (node editing), by its stable key.
    pub selected_node: Option<u32>,
}

impl Shared {
    pub fn new(doc: Document) -> Self {
        let selected = doc.generators.first().map(|g| g.id);
        Shared {
            committed: doc.clone(),
            doc,
            undo: vec![],
            redo: vec![],
            selected,
            view: ViewState::default(),
            file: None,
            dirty: false,
            log: VecDeque::new(),
            started: Instant::now(),
            screenshot_waiters: vec![],
            mcp: McpStatus::default(),
            frame_camera: true,
            selected_node: None,
        }
    }

    pub fn log(&mut self, source: Source, text: impl Into<String>) {
        let at = self.started.elapsed().as_secs_f32();
        self.log.push_back(LogLine { at, source, text: text.into() });
        while self.log.len() > 400 {
            self.log.pop_front();
        }
    }

    /// Record the current document in the undo history if it changed.
    pub fn commit(&mut self) {
        if self.doc != self.committed {
            let prev = std::mem::replace(&mut self.committed, self.doc.clone());
            self.undo.push(prev);
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.dirty = true;
        }
    }

    pub fn undo(&mut self) -> bool {
        self.commit();
        if let Some(prev) = self.undo.pop() {
            let cur = std::mem::replace(&mut self.doc, prev);
            self.redo.push(cur);
            self.committed = self.doc.clone();
            self.fix_selection();
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        self.commit();
        if let Some(next) = self.redo.pop() {
            let cur = std::mem::replace(&mut self.doc, next);
            self.undo.push(cur);
            self.committed = self.doc.clone();
            self.fix_selection();
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Replace the whole document (preset, new, open) as one undoable step.
    pub fn replace_doc(&mut self, doc: Document) {
        self.commit();
        self.doc = doc;
        self.commit();
        self.selected = self.doc.generators.first().map(|g| g.id);
        self.frame_camera = true;
    }

    pub fn fix_selection(&mut self) {
        if let Some(s) = self.selected {
            if self.doc.get(s).is_none() {
                self.selected = self.doc.generators.first().map(|g| g.id);
            }
        }
    }

    pub fn title(&self) -> String {
        let f = self.file.as_ref().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled".into());
        format!("{}{} — {} — ArborStudio", f, if self.dirty { " •" } else { "" }, self.doc.name)
    }
}

pub fn save_project(doc: &Document, path: &std::path::Path) -> Result<(), String> {
    let s = serde_json::to_string_pretty(doc).map_err(|e| e.to_string())?;
    if let Some(d) = path.parent() {
        if !d.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(d);
        }
    }
    std::fs::write(path, s).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

pub fn load_project(path: &std::path::Path) -> Result<Document, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut d: Document = serde_json::from_str(&s).map_err(|e| format!("not a tree project: {e}"))?;
    d.clamp_all();
    Ok(d)
}
