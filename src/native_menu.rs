//! macOS: the app's menus live in the system menu bar at the top of the screen (instead of a
//! menu row inside the window, which other systems keep). Menu choices arrive as command ids
//! that the app handles once per frame; ticks and greyed-out items follow the app's state.

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{AboutMetadata, CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::model::PRESETS;
use crate::render::RenderMode;

pub const RENDER_MODES: [RenderMode; 3] = [RenderMode::Shaded, RenderMode::Wireframe, RenderMode::Skeleton];
/// View switches: (command id, label).
pub const VIEW_TOGGLES: [(&str, &str); 6] = [
    ("view.grid", "Ground grid"),
    ("view.leaves", "Leaves"),
    ("view.highlight", "Highlight selection"),
    ("view.wind", "Play wind"),
    ("view.helpers", "Forces and pruning helpers"),
    ("view.collision", "Collision objects"),
];

/// What the menus need to know to show ticks and greyed-out items.
#[derive(PartialEq, Clone, Default)]
pub struct MenuState {
    pub can_undo: bool,
    pub can_redo: bool,
    pub has_selection: bool,
    pub can_add_child: bool,
    pub can_add_roots: bool,
    pub export_lods: bool,
    pub export_billboard: bool,
    pub mode: usize,
    pub toggles: [bool; 6],
}

pub struct NativeMenu {
    _menu: Menu,
    events: Arc<Mutex<Vec<String>>>,
    recent_menu: Submenu,
    recent_items: Vec<Box<dyn muda::IsMenuItem>>,
    recent_gen: u64,
    /// Paths behind the "recent.N" commands.
    pub recent: Vec<PathBuf>,
    undo: MenuItem,
    redo: MenuItem,
    delete: MenuItem,
    delete_gen: MenuItem,
    add_branches: MenuItem,
    add_roots: MenuItem,
    add_leaves: MenuItem,
    lods: CheckMenuItem,
    billboard: CheckMenuItem,
    modes: Vec<CheckMenuItem>,
    toggles: Vec<CheckMenuItem>,
    last: Option<MenuState>,
}

fn acc(mods: Modifiers, code: Code) -> Option<Accelerator> {
    Some(Accelerator::new(Some(mods), code))
}

fn item(id: &str, text: &str, accel: Option<Accelerator>) -> MenuItem {
    MenuItem::with_id(id, text, true, accel)
}

impl NativeMenu {
    /// Builds the menu bar and puts it in place. Call once the app is running.
    pub fn install(ctx: &egui::Context) -> NativeMenu {
        let cmd = Modifiers::SUPER;
        let events: Arc<Mutex<Vec<String>>> = Arc::default();
        {
            let events = events.clone();
            let ctx = ctx.clone();
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                events.lock().unwrap().push(e.id.0.clone());
                ctx.request_repaint();
            }));
        }
        let menu = Menu::new();

        // the app menu (shown under the app's name)
        let app = Submenu::new("ArborStudio", true);
        let about = PredefinedMenuItem::about(
            Some("About ArborStudio"),
            Some(AboutMetadata { name: Some("ArborStudio".into()), version: Some(env!("CARGO_PKG_VERSION").into()), ..Default::default() }),
        );
        let _ = app.append_items(&[
            &about,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &item("app.quit", "Quit ArborStudio", acc(cmd, Code::KeyQ)),
        ]);

        // File
        let file = Submenu::new("File", true);
        let recent_menu = Submenu::new("Open Recent", true);
        let lods = CheckMenuItem::with_id("file.lods", "Include LOD0–3 in Exports", true, true, None);
        let billboard = CheckMenuItem::with_id("file.billboard", "Include Billboard LOD in Exports", true, true, None);
        let _ = file.append_items(&[
            &item("file.new", "New Tree", acc(cmd, Code::KeyN)),
            &item("file.open", "Open…", acc(cmd, Code::KeyO)),
            &recent_menu,
            &PredefinedMenuItem::separator(),
            &item("file.save", "Save", acc(cmd, Code::KeyS)),
            &item("file.save_as", "Save As…", acc(cmd | Modifiers::SHIFT, Code::KeyS)),
            &PredefinedMenuItem::separator(),
            &lods,
            &billboard,
            &item("file.export_glb", "Export GLB (glTF) for Unity / Unreal / Godot…", acc(cmd, Code::KeyE)),
            &item("file.export_obj", "Export OBJ for Unity / Unreal / Godot…", None),
            &item("file.export_json", "Export Tree Data (JSON) for Custom Engines…", None),
            &PredefinedMenuItem::separator(),
            &item("file.variations_glb", "Export Variations as GLB…", None),
            &item("file.variations_obj", "Export Variations as OBJ…", None),
        ]);

        // Edit
        let edit = Submenu::new("Edit", true);
        let undo = item("edit.undo", "Undo", acc(cmd, Code::KeyZ));
        let redo = item("edit.redo", "Redo", acc(cmd | Modifiers::SHIFT, Code::KeyZ));
        let delete = item("edit.delete", "Delete Generator", None);
        let _ = edit.append_items(&[&undo, &redo, &PredefinedMenuItem::separator(), &delete, &item("edit.seed", "Randomize Seed", None)]);

        // Generators
        let gens = Submenu::new("Generators", true);
        let add_branches = item("gen.branches", "Add Branches", None);
        let add_roots = item("gen.roots", "Add Roots", None);
        let add_leaves = item("gen.leaves", "Add Leaves", None);
        let delete_gen = item("gen.delete", "Delete Selected Generator", None);
        let _ = gens.append_items(&[&item("gen.trunk", "Add Trunk", None), &add_branches, &add_roots, &add_leaves, &PredefinedMenuItem::separator(), &delete_gen]);

        // Presets
        let presets = Submenu::new("Presets", true);
        for (i, (n, _)) in PRESETS.iter().enumerate() {
            let _ = presets.append(&item(&format!("preset.{i}"), n, None));
        }

        // View
        let view = Submenu::new("View", true);
        let modes: Vec<CheckMenuItem> = RENDER_MODES.iter().enumerate().map(|(i, m)| CheckMenuItem::with_id(format!("mode.{i}"), m.label(), true, i == 0, None)).collect();
        for m in &modes {
            let _ = view.append(m);
        }
        let _ = view.append(&PredefinedMenuItem::separator());
        let toggles: Vec<CheckMenuItem> = VIEW_TOGGLES.iter().map(|(id, label)| CheckMenuItem::with_id(*id, *label, true, false, None)).collect();
        for t in &toggles {
            let _ = view.append(t);
        }
        let _ = view.append_items(&[&PredefinedMenuItem::separator(), &item("view.frame", "Frame Tree (F)", None), &PredefinedMenuItem::fullscreen(None)]);

        // Agent
        let agent = Submenu::new("Agent", true);
        let _ = agent.append(&item("agent.connect", "Connect a Coding Agent…", None));

        // Window
        let window = Submenu::new("Window", true);
        let _ = window.append_items(&[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::maximize(None), &PredefinedMenuItem::separator(), &PredefinedMenuItem::bring_all_to_front(None)]);

        // Help
        let help = Submenu::new("Help", true);
        let _ = help.append_items(&[&item("help.docs", "ArborStudio Help", acc(Modifiers::empty(), Code::F1)), &item("help.shortcuts", "Keyboard and Mouse", None)]);

        let _ = menu.append_items(&[&app, &file, &edit, &gens, &presets, &view, &agent, &window, &help]);
        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();
        help.set_as_help_menu_for_nsapp();

        let mut m = NativeMenu {
            _menu: menu,
            events,
            recent_menu,
            recent_items: vec![],
            recent_gen: u64::MAX,
            recent: vec![],
            undo,
            redo,
            delete,
            delete_gen,
            add_branches,
            add_roots,
            add_leaves,
            lods,
            billboard,
            modes,
            toggles,
            last: None,
        };
        m.refresh_recent();
        m
    }

    /// Menu choices since the last frame.
    pub fn take_events(&mut self) -> Vec<String> {
        let ev = std::mem::take(&mut *self.events.lock().unwrap());
        if !ev.is_empty() {
            // clicking a tick item flips it by itself; redraw all ticks from the app's state
            self.last = None;
        }
        ev
    }

    /// Rebuild "Open Recent" when the list changed (opening or saving, also by an agent).
    pub fn refresh_recent(&mut self) {
        let gen = crate::recent::generation();
        if gen == self.recent_gen {
            return;
        }
        self.recent_gen = gen;
        for it in self.recent_items.drain(..) {
            let _ = self.recent_menu.remove(it.as_ref());
        }
        self.recent = crate::recent::list();
        let mut items: Vec<Box<dyn muda::IsMenuItem>> = vec![];
        if self.recent.is_empty() {
            items.push(Box::new(MenuItem::with_id("recent.none", "No Recent Projects", false, None)));
        }
        for (i, p) in self.recent.iter().enumerate() {
            let exists = p.exists();
            let label = if exists { crate::recent::label(p) } else { format!("{} (missing)", crate::recent::label(p)) };
            items.push(Box::new(MenuItem::with_id(format!("recent.{i}"), label, exists, None)));
        }
        if !self.recent.is_empty() {
            items.push(Box::new(PredefinedMenuItem::separator()));
            if self.recent.iter().any(|p| !p.exists()) {
                items.push(Box::new(MenuItem::with_id("recent.remove_missing", "Remove Missing", true, None)));
            }
            items.push(Box::new(MenuItem::with_id("recent.clear", "Clear Menu", true, None)));
        }
        for it in &items {
            let _ = self.recent_menu.append(it.as_ref());
        }
        self.recent_items = items;
        let paths: Vec<String> = self.recent.iter().map(|p| crate::recent::short_path(p)).collect();
        set_recent_subtitles(&paths);
    }

    /// Show the app's state in the menus (only touches the menus when something changed).
    pub fn sync(&mut self, s: MenuState) {
        self.refresh_recent();
        if self.last.as_ref() == Some(&s) {
            return;
        }
        self.undo.set_enabled(s.can_undo);
        self.redo.set_enabled(s.can_redo);
        self.delete.set_enabled(s.has_selection);
        self.delete_gen.set_enabled(s.has_selection);
        self.add_branches.set_enabled(s.can_add_child);
        self.add_leaves.set_enabled(s.can_add_child);
        self.add_roots.set_enabled(s.can_add_roots);
        self.lods.set_checked(s.export_lods);
        self.billboard.set_checked(s.export_billboard);
        for (i, m) in self.modes.iter().enumerate() {
            m.set_checked(i == s.mode);
        }
        for (t, on) in self.toggles.iter().zip(s.toggles) {
            t.set_checked(on);
        }
        self.last = Some(s);
    }
}

/// The path of each recent project as a grey second line under its name (macOS 14.4 and later;
/// before that it follows the name). muda has no subtitles, so this reaches the menu bar's own
/// "File > Open Recent" menu through AppKit.
fn set_recent_subtitles(paths: &[String]) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send, sel};
    fn ns(s: &str) -> *mut AnyObject {
        let c = std::ffi::CString::new(s.replace('\0', "")).unwrap_or_default();
        unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
    }
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let main: *mut AnyObject = msg_send![app, mainMenu];
        if main.is_null() {
            return;
        }
        let file: *mut AnyObject = msg_send![main, itemWithTitle: ns("File")];
        if file.is_null() {
            return;
        }
        let file_menu: *mut AnyObject = msg_send![file, submenu];
        if file_menu.is_null() {
            return;
        }
        let recent: *mut AnyObject = msg_send![file_menu, itemWithTitle: ns("Open Recent")];
        if recent.is_null() {
            return;
        }
        let menu: *mut AnyObject = msg_send![recent, submenu];
        if menu.is_null() {
            return;
        }
        let count: isize = msg_send![menu, numberOfItems];
        for (i, p) in paths.iter().enumerate() {
            if i as isize >= count {
                break;
            }
            let it: *mut AnyObject = msg_send![menu, itemAtIndex: i as isize];
            if it.is_null() {
                continue;
            }
            let has_subtitle: bool = msg_send![it, respondsToSelector: sel!(setSubtitle:)];
            if has_subtitle {
                let _: () = msg_send![it, setSubtitle: ns(p)];
            } else {
                let title: *mut AnyObject = msg_send![it, title];
                let utf8: *const std::ffi::c_char = msg_send![title, UTF8String];
                let name = if utf8.is_null() { String::new() } else { std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned() };
                let _: () = msg_send![it, setTitle: ns(&format!("{name}   {p}"))];
            }
        }
    }
}
