//! The main window.

use crate::export::{self, Format};
use crate::grow::{build_mesh, grow, Mesh, Skeleton, Stats};
use crate::mcp;
use crate::model::*;
use crate::render::{self, PickData, RenderMode};
use crate::state::{load_project, save_project, Shared, Source};
use egui::{Color32, RichText, Ui};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const ACCENT: Color32 = Color32::from_rgb(108, 178, 84);
const PANEL: Color32 = Color32::from_rgb(40, 42, 46);
const PANEL_DARK: Color32 = Color32::from_rgb(31, 33, 36);
const HEADER: Color32 = Color32::from_rgb(160, 166, 174);
/// Warnings, such as a level of detail over its triangle budget.
const WARN: Color32 = Color32::from_rgb(236, 178, 72);

enum Action {
    Open,
    /// Open a project from the recent list.
    OpenPath(std::path::PathBuf),
    SaveAs,
    Save,
    Export(Format, bool),
    /// Export several seeds of the current tree into a folder.
    ExportVariations(Format),
    /// Choose an image for a material: (material id, is normal map).
    PickTexture(u32, bool),
    /// Choose an OBJ model for a leaves generator.
    PickLeafMesh(u32),
    /// Open the help in the web browser (a page, or the start page).
    Help(Option<&'static str>),
}

#[derive(PartialEq, Clone, Copy)]
enum BottomTab {
    Activity,
    Stats,
}

pub struct ArborApp {
    shared: Arc<Mutex<Shared>>,
    mesh: Mesh,
    skeleton: Skeleton,
    built: Option<(Document, u32)>,
    stats: Stats,
    /// The shown mesh level against its triangle budget.
    fit: Option<crate::budget::LevelFit>,
    pick: PickData,
    time: f32,
    frames: u32,
    cli_shot: Option<PathBuf>,
    shot_in_flight: bool,
    show_connect: bool,
    show_about: bool,
    bottom_tab: BottomTab,
    port: u16,
    mcp_url: Option<String>,
    export_lods: bool,
    status: String,
    last_selected: Option<u32>,
    flash_until: f32,
    /// Loaded material images by file path (None = could not be read).
    tex_cache: HashMap<String, Option<egui::TextureHandle>>,
    /// Material id -> texture for this frame.
    mat_textures: HashMap<u32, egui::TextureId>,
    /// Material id -> (normal map texture, DirectX style, strength).
    mat_normals: HashMap<u32, (egui::TextureId, bool, f32)>,
    gpu: crate::gpu::GpuRenderer,
    has_gl: bool,
    editing_material: Option<u32>,
    /// Documentation pictures (--shot-script).
    pub script: Option<crate::docshots::ShotScript>,
    /// Scroll the properties panel to this offset once (documentation pictures).
    force_scroll: Option<f32>,
    export_billboard: bool,
    /// Bake material gradients into plain materials in GLB exports.
    export_bake_blend: bool,
    /// Bake ambient occlusion into the exported vertex colours (off for engines with their own AO).
    export_bake_ao: bool,
    /// macOS: the menus in the system menu bar.
    #[cfg(target_os = "macos")]
    native_menu: Option<crate::native_menu::NativeMenu>,
    variation_count: u32,
    /// Billboard atlas shown when the viewport is at LOD 4.
    bb_tex: Option<egui::TextureHandle>,
    pending_atlas: Option<std::sync::Arc<image::RgbaImage>>,
    /// Growth preview: moves the shown mesh's vertices (made when first needed).
    grower: Option<crate::growth::Grower>,
    /// The shown mesh at the previewed growth.
    grown: Option<(f32, Mesh)>,
}

pub fn setup_style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = Color32::from_rgb(46, 48, 53);
    v.extreme_bg_color = Color32::from_rgb(24, 25, 28);
    v.faint_bg_color = Color32::from_rgb(45, 47, 51);
    v.selection.bg_fill = Color32::from_rgb(64, 110, 52);
    v.selection.stroke = egui::Stroke::new(1.0, Color32::from_rgb(190, 235, 170));
    v.hyperlink_color = ACCENT;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, Color32::from_rgb(56, 59, 64));
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(55, 58, 63);
    v.widgets.inactive.bg_fill = Color32::from_rgb(55, 58, 63);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(68, 72, 78);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(78, 84, 90);
    v.slider_trailing_fill = true;
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.slider_width = 140.0;
        s.spacing.interact_size.y = 20.0;
    });
}

impl ArborApp {
    pub fn new(cc: &eframe::CreationContext<'_>, shared: Arc<Mutex<Shared>>, port: u16, cli_shot: Option<PathBuf>, mcp_enabled: bool) -> Self {
        setup_style(&cc.egui_ctx);
        let mut mcp_url = None;
        if mcp_enabled {
            let server = mcp::Server { shared: shared.clone(), ctx: cc.egui_ctx.clone(), headless: false };
            let mut res = mcp::start(server.clone(), port);
            if res.is_err() {
                // a check run must not clash with a window the person already has open
                res = mcp::start(server, 0);
            }
            let mut sh = shared.lock().unwrap();
            match res {
                Ok(url) => {
                    sh.log(Source::App, format!("MCP server listening at {url}"));
                    sh.mcp.url = Some(url.clone());
                    mcp_url = Some(url);
                }
                Err(e) => {
                    sh.log(Source::App, format!("MCP server not started: {e}"));
                    sh.mcp.error = Some(e);
                }
            }
        }
        ArborApp {
            shared,
            mesh: Mesh::default(),
            skeleton: Skeleton::default(),
            built: None,
            stats: Stats::default(),
            fit: None,
            pick: PickData::default(),
            time: 0.0,
            frames: 0,
            cli_shot,
            shot_in_flight: false,
            show_connect: false,
            show_about: false,
            bottom_tab: BottomTab::Activity,
            port,
            mcp_url,
            export_lods: true,
            status: "Ready".into(),
            last_selected: None,
            flash_until: 0.0,
            tex_cache: HashMap::new(),
            mat_textures: HashMap::new(),
            mat_normals: HashMap::new(),
            gpu: Default::default(),
            has_gl: false,
            editing_material: None,
            script: None,
            force_scroll: None,
            export_billboard: true,
            export_bake_blend: false,
            export_bake_ao: true,
            #[cfg(target_os = "macos")]
            native_menu: Some(crate::native_menu::NativeMenu::install(&cc.egui_ctx)),
            variation_count: 5,
            bb_tex: None,
            pending_atlas: None,
            grower: None,
            grown: None,
        }
    }

    fn rebuild(&mut self, sh: &mut Shared) {
        let lod = sh.view.lod;
        let stale = match &self.built {
            Some((d, l)) => *l != lod || *d != sh.doc,
            None => true,
        };
        if stale {
            let lm = crate::billboard::lod_mesh(&sh.doc, lod);
            self.skeleton = lm.skeleton;
            self.mesh = lm.mesh;
            self.bb_tex = None;
            if let Some(atlas) = lm.atlas {
                // stored for the viewport; the egui texture is made in `ui`
                self.pending_atlas = Some(atlas);
            }
            self.stats = self.mesh.stats(&self.skeleton);
            self.fit = lm.fit;
            self.built = Some((sh.doc.clone(), lod));
            self.grower = None;
            self.grown = None;
        }
        if sh.frame_camera {
            sh.frame_camera = false;
            sh.view.camera.frame(self.mesh.height, self.mesh.radius);
            // testing aid for --screenshot: ARBOR_VIEW="yaw,pitch,distance,x,y,z" sets the camera
            if let Some(l) = std::env::var("ARBOR_LIGHTING").ok().and_then(|l| render::Lighting::parse(&l)) {
                sh.view.lighting = l;
            }
            if let Ok(v) = std::env::var("ARBOR_VIEW") {
                let f: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                if f.len() == 6 {
                    let c = &mut sh.view.camera;
                    (c.yaw, c.pitch, c.distance, c.target) = (f[0], f[1], f[2], [f[3], f[4], f[5]]);
                }
            }
        }
    }
}

fn png_from_color_image(img: &egui::ColorImage) -> Result<Vec<u8>, String> {
    let [w, h] = img.size;
    let mut buf = image::RgbaImage::new(w as u32, h as u32);
    for (i, c) in img.pixels.iter().enumerate() {
        let [r, g, b, _] = c.to_array();
        buf.put_pixel((i % w) as u32, (i / w) as u32, image::Rgba([r, g, b, 255]));
    }
    let mut out = std::io::Cursor::new(Vec::new());
    buf.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

fn section_title(ui: &mut Ui, t: &str) {
    ui.add_space(2.0);
    ui.label(RichText::new(t).size(11.0).strong().color(HEADER));
}

fn help_for(field: &str) -> String {
    PARAM_REFERENCE
        .iter()
        .find(|r| r.0 == field)
        .map(|r| format!("{}\nRange: {}", r.3, r.2))
        .or_else(|| SETTINGS_REFERENCE.iter().find(|r| r.0 == field).map(|r| format!("{}\nRange: {}", r.2, r.1)))
        .unwrap_or_default()
}

fn prow(ui: &mut Ui, label: &str, field: &str, add: impl FnOnce(&mut Ui) -> egui::Response) {
    ui.label(label).on_hover_text(help_for(field));
    add(ui).on_hover_text(help_for(field));
    ui.end_row();
}

fn kind_color(k: GenKind) -> Color32 {
    match k {
        GenKind::Trunk => Color32::from_rgb(176, 128, 86),
        GenKind::Branches => Color32::from_rgb(214, 180, 120),
        GenKind::Leaves => Color32::from_rgb(110, 196, 84),
        GenKind::Roots => Color32::from_rgb(140, 100, 70),
    }
}

impl eframe::App for ArborApp {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        self.has_gl = frame.gl().is_some();
        let ctx = ui.ctx().clone();
        self.frames += 1;
        let dt = ctx.input(|i| i.stable_dt).min(0.1);

        // -------- screenshots (window capture for MCP and --screenshot)
        let shots: Vec<Arc<egui::ColorImage>> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None })
                .collect()
        });
        if let Some(img) = shots.last() {
            self.shot_in_flight = false;
            if let Some((path, rect)) = self.script.as_mut().and_then(|s| s.pending.take()) {
                crate::docshots::save(img, &path, rect, ctx.pixels_per_point());
            }
            let png = png_from_color_image(img);
            let waiters: Vec<_> = self.shared.lock().unwrap().screenshot_waiters.drain(..).collect();
            for w in waiters {
                let _ = w.send(png.clone());
            }
            if self.frames >= 120 {
                if let Some(path) = self.cli_shot.take() {
                    if let Ok(bytes) = &png {
                        if let Some(d) = path.parent() {
                            let _ = std::fs::create_dir_all(d);
                        }
                        let _ = std::fs::write(&path, bytes);
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        let want_shot = !self.shared.lock().unwrap().screenshot_waiters.is_empty() || (self.cli_shot.is_some() && self.frames >= 120);
        if want_shot && !self.shot_in_flight {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.shot_in_flight = true;
        }
        if self.cli_shot.is_some() || self.shot_in_flight {
            ctx.request_repaint();
        }

        self.run_script(&ctx);
        let shared = self.shared.clone();
        let mut sh = shared.lock().unwrap();
        self.rebuild(&mut sh);
        let mut actions: Vec<Action> = vec![];

        // -------- macOS menu bar
        #[cfg(target_os = "macos")]
        let native_menu = self.native_menu.is_some();
        #[cfg(not(target_os = "macos"))]
        let native_menu = false;
        #[cfg(target_os = "macos")]
        self.native_menu_frame(&mut sh, &mut actions, &ctx);

        // -------- keyboard shortcuts (with the macOS menu bar, its items carry them)
        let typing = ctx.egui_wants_keyboard_input();
        ctx.input_mut(|i| {
            use egui::{Key, KeyboardShortcut, Modifiers};
            if native_menu {
                // only the plain keys below
            } else if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)) {
                sh.redo();
            } else if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)) {
                sh.undo();
            } else if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S)) {
                actions.push(Action::Save);
            } else if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::O)) {
                actions.push(Action::Open);
            } else if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::E)) {
                actions.push(Action::Export(Format::Glb, self.export_lods));
            }
            if !typing && i.key_pressed(Key::F) {
                sh.frame_camera = true;
            }
            if !typing && i.key_pressed(Key::Escape) {
                sh.selected = None;
                sh.selected_node = None;
            }
            if !native_menu && i.key_pressed(Key::F1) {
                actions.push(Action::Help(None));
            }
            if !typing && (i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace)) {
                if let Some(id) = sh.selected {
                    sh.doc.remove(id);
                    sh.fix_selection();
                }
            }
        });

        // -------- menu bar (in the window, unless it is in the macOS menu bar)
        if !native_menu {
        egui::Panel::top("menu").frame(egui::Frame::new().fill(PANEL_DARK).inner_margin(egui::Margin::symmetric(6, 2))).show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New tree").clicked() {
                        let mut d = preset("Sapling").unwrap();
                        d.name = "New tree".into();
                        sh.replace_doc(d);
                        sh.file = None;
                    }
                    if ui.button("Open…").clicked() {
                        actions.push(Action::Open);
                    }
                    ui.menu_button("Open recent", |ui| {
                        let recent = crate::recent::list();
                        if recent.is_empty() {
                            ui.add_enabled(false, egui::Button::new("No recent projects"));
                        }
                        for p in &recent {
                            let exists = p.exists();
                            let name = if exists { crate::recent::label(p) } else { format!("{} (missing)", crate::recent::label(p)) };
                            // the name, with the file's path as a grey line under it
                            let mut job = egui::text::LayoutJob::default();
                            let body = ui.style().text_styles[&egui::TextStyle::Button].clone();
                            let small = egui::FontId::new(body.size * 0.85, body.family.clone());
                            job.append(&name, 0.0, egui::TextFormat { font_id: body, color: ui.visuals().text_color(), ..Default::default() });
                            job.append(&format!("\n{}", crate::recent::short_path(p)), 0.0, egui::TextFormat { font_id: small, color: Color32::from_gray(130), ..Default::default() });
                            let r = ui.add_enabled(exists, egui::Button::new(job)).on_hover_text(p.display().to_string()).on_disabled_hover_text(format!("{} no longer exists", p.display()));
                            if r.clicked() {
                                actions.push(Action::OpenPath(p.clone()));
                                ui.close();
                            }
                        }
                        if !recent.is_empty() {
                            ui.separator();
                            if recent.iter().any(|p| !p.exists()) && ui.button("Remove missing").clicked() {
                                for p in recent.iter().filter(|p| !p.exists()) {
                                    crate::recent::remove(p);
                                }
                            }
                            if ui.button("Clear list").clicked() {
                                crate::recent::clear();
                                ui.close();
                            }
                        }
                    });
                    if ui.button("Save").clicked() {
                        actions.push(Action::Save);
                    }
                    if ui.button("Save as…").clicked() {
                        actions.push(Action::SaveAs);
                    }
                    ui.separator();
                    ui.checkbox(&mut self.export_lods, "Include LOD0–3 in exports");
                    ui.checkbox(&mut self.export_billboard, "Include billboard LOD in exports");
                    ui.checkbox(&mut self.export_bake_blend, "Bake material gradients (GLB)").on_hover_text("Plain materials instead of blend weights, for engines without a custom shader. OBJ is always baked.");
                    ui.checkbox(&mut self.export_bake_ao, "Bake ambient occlusion into vertex colours").on_hover_text("Untick for an engine with its own ambient occlusion (SSAO, GTAO): the vertex colours then carry only the materials' colours, the branch shading and each leaf's colour variation.");
                    if ui.button("Export GLB (glTF) — Unity / Unreal / Godot…").clicked() {
                        actions.push(Action::Export(Format::Glb, self.export_lods));
                    }
                    if ui.button("Export OBJ — Unity / Unreal / Godot…").clicked() {
                        actions.push(Action::Export(Format::Obj, self.export_lods));
                    }
                    if ui.button("Export tree data (JSON) — custom engines…").clicked() {
                        actions.push(Action::Export(Format::Json, false));
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("Variations");
                        ui.add(egui::DragValue::new(&mut self.variation_count).range(1..=64));
                    });
                    if ui.button("Export variations as GLB…").on_hover_text("Same species, different seeds: one file per individual, for forests").clicked() {
                        actions.push(Action::ExportVariations(Format::Glb));
                    }
                    if ui.button("Export variations as OBJ…").clicked() {
                        actions.push(Action::ExportVariations(Format::Obj));
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui.add_enabled(!sh.undo.is_empty() || sh.doc != sh.committed, egui::Button::new("Undo")).clicked() {
                        sh.undo();
                    }
                    if ui.add_enabled(!sh.redo.is_empty(), egui::Button::new("Redo")).clicked() {
                        sh.redo();
                    }
                    ui.separator();
                    if ui.add_enabled(sh.selected.is_some(), egui::Button::new("Delete generator")).clicked() {
                        if let Some(id) = sh.selected {
                            sh.doc.remove(id);
                            sh.fix_selection();
                        }
                    }
                    if ui.button("Randomize seed").clicked() {
                        randomize(&mut sh);
                    }
                });
                ui.menu_button("Generators", |ui| {
                    add_buttons(ui, &mut sh);
                });
                ui.menu_button("Presets", |ui| {
                    for (n, d) in PRESETS {
                        if ui.button(n).on_hover_text(d).clicked() {
                            sh.replace_doc(preset(n).unwrap());
                            sh.log(Source::You, format!("loaded preset {n}"));
                        }
                    }
                });
                ui.menu_button("View", |ui| {
                    for m in [RenderMode::Shaded, RenderMode::Wireframe, RenderMode::Skeleton] {
                        ui.radio_value(&mut sh.view.mode, m, m.label());
                    }
                    ui.separator();
                    ui.checkbox(&mut sh.view.show_grid, "Ground grid");
                    ui.checkbox(&mut sh.view.show_leaves, "Leaves");
                    ui.checkbox(&mut sh.view.highlight_selection, "Highlight selection");
                    ui.checkbox(&mut sh.view.wind_playing, "Play wind");
                    ui.checkbox(&mut sh.view.show_helpers, "Forces and pruning helpers");
                    ui.checkbox(&mut sh.view.show_collision, "Collision objects");
                    if ui.button("Frame tree  (F)").clicked() {
                        sh.frame_camera = true;
                    }
                });
                ui.menu_button("Agent", |ui| {
                    if ui.button("Connect a coding agent…").clicked() {
                        self.show_connect = true;
                    }
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("ArborStudio Help  (F1)").clicked() {
                        actions.push(Action::Help(None));
                    }
                    if ui.button("Keyboard and mouse").clicked() {
                        actions.push(Action::Help(Some("shortcuts.html")));
                    }
                    ui.separator();
                    if ui.button("About ArborStudio").clicked() {
                        self.show_about = true;
                    }
                });
            });
        });
        }

        // -------- toolbar
        egui::Panel::top("toolbar").frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(8, 5))).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("🗋 New").on_hover_text("New tree").clicked() {
                    let mut d = preset("Sapling").unwrap();
                    d.name = "New tree".into();
                    sh.replace_doc(d);
                    sh.file = None;
                }
                if ui.button("📂 Open").clicked() {
                    actions.push(Action::Open);
                }
                if ui.button("💾 Save").clicked() {
                    actions.push(Action::Save);
                }
                ui.menu_button("⮩ Export", |ui| {
                    ui.checkbox(&mut self.export_lods, "Include LOD0–3");
                    ui.checkbox(&mut self.export_billboard, "Include billboard LOD");
                    ui.checkbox(&mut self.export_bake_blend, "Bake material gradients (GLB)").on_hover_text("Plain materials instead of blend weights, for engines without a custom shader. OBJ is always baked.");
                    ui.checkbox(&mut self.export_bake_ao, "Bake ambient occlusion into vertex colours").on_hover_text("Untick for an engine with its own ambient occlusion (SSAO, GTAO): the vertex colours then carry only the materials' colours, the branch shading and each leaf's colour variation.");
                    if ui.button("GLB (glTF 2.0)").clicked() {
                        actions.push(Action::Export(Format::Glb, self.export_lods));
                    }
                    if ui.button("OBJ + MTL").clicked() {
                        actions.push(Action::Export(Format::Obj, self.export_lods));
                    }
                    if ui.button("Tree data (JSON)").clicked() {
                        actions.push(Action::Export(Format::Json, false));
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("Variations");
                        ui.add(egui::DragValue::new(&mut self.variation_count).range(1..=64));
                    });
                    if ui.button("Variations as GLB…").on_hover_text("Same species, different seeds: one file per individual, for forests").clicked() {
                        actions.push(Action::ExportVariations(Format::Glb));
                    }
                    if ui.button("Variations as OBJ…").clicked() {
                        actions.push(Action::ExportVariations(Format::Obj));
                    }
                });
                ui.separator();
                if ui.add_enabled(!sh.undo.is_empty() || sh.doc != sh.committed, egui::Button::new("Undo")).on_hover_text("Undo (⌘Z)").clicked() {
                    sh.undo();
                }
                if ui.add_enabled(!sh.redo.is_empty(), egui::Button::new("Redo")).on_hover_text("Redo (⇧⌘Z)").clicked() {
                    sh.redo();
                }
                ui.separator();
                if ui.button("🎲 Randomize").on_hover_text("New seed: same species, different tree").clicked() {
                    randomize(&mut sh);
                }
                ui.separator();
                for m in [RenderMode::Shaded, RenderMode::Wireframe, RenderMode::Skeleton] {
                    ui.selectable_value(&mut sh.view.mode, m, m.label());
                }
                ui.separator();
                let wind_label = if sh.view.wind_playing { "⏸ Wind" } else { "▶ Wind" };
                ui.toggle_value(&mut sh.view.wind_playing, wind_label).on_hover_text("Preview wind animation");
                let grow_label = if sh.view.growth_playing { "⏸ Grow" } else { "▶ Grow" };
                if ui.selectable_label(sh.view.growth_playing, grow_label).on_hover_text("Play the growth from a sprout to the mature tree (Tree › Growth)").clicked() {
                    toggle_growth(&mut sh.view);
                }
                ui.separator();
                ui.label("LOD");
                for l in 0..4u32 {
                    ui.selectable_value(&mut sh.view.lod, l, format!("{l}"));
                }
                ui.selectable_value(&mut sh.view.lod, crate::billboard::BILLBOARD_LOD, "BB")
                    .on_hover_text("Billboard: the tree pictured from 8 sides on crossed cards, for far distances");
                ui.separator();
                ui.label("Pick");
                ui.selectable_value(&mut sh.view.node_mode, false, "Generators").on_hover_text("Clicking the tree selects a whole generator");
                ui.selectable_value(&mut sh.view.node_mode, true, "Branches").on_hover_text("Clicking the tree selects one single branch to hand-edit (node editing)");
                ui.separator();
                if ui.button("⛶ Frame").on_hover_text("Fit the tree in view (F)").clicked() {
                    sh.frame_camera = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (txt, col) = match (&sh.mcp.url, &sh.mcp.error) {
                        (Some(_), _) => (
                            match &sh.mcp.client {
                                Some(c) => format!("Agent: {c}"),
                                None => "Agent link ready".to_string(),
                            },
                            ACCENT,
                        ),
                        (None, Some(_)) => ("Agent link off".to_string(), Color32::from_rgb(220, 120, 90)),
                        _ => ("Agent link disabled".to_string(), Color32::GRAY),
                    };
                    if ui.add(egui::Button::new(RichText::new(txt).color(col)).frame(true)).on_hover_text("Connect your coding agent (MCP)").clicked() {
                        self.show_connect = true;
                    }
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, col);
                });
            });
        });

        // -------- status bar
        egui::Panel::bottom("status").frame(egui::Frame::new().fill(PANEL_DARK).inner_margin(egui::Margin::symmetric(8, 3))).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.status).size(11.5).color(HEADER));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let s = &self.stats;
                    ui.label(
                        RichText::new(format!(
                            "{} triangles · {} vertices · {} stems · {} leaves · {:.1} m tall · seed {}",
                            s.triangles, s.vertices, s.stems, s.leaves, s.height_m, sh.doc.settings.seed
                        ))
                        .size(11.5)
                        .color(HEADER),
                    );
                });
            });
        });

        // -------- left: generation hierarchy + presets
        // -------- material textures (loaded once per image file)
        self.mat_textures.clear();
        for m in &sh.doc.materials {
            let Some(path) = &m.texture else { continue };
            let entry = self.tex_cache.entry(path.clone()).or_insert_with(|| {
                crate::textures::load(path).ok().map(|img| {
                    let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                    // mipmaps: large (e.g. 4K) bark textures stay smooth instead of shimmering when small on screen
                    ctx.load_texture(path.clone(), ci, egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR_REPEAT })
                })
            });
            if let Some(h) = entry {
                self.mat_textures.insert(m.id, h.id());
            }
        }
        // normal maps (raw values: egui textures are not sRGB-decoded)
        self.mat_normals.clear();
        for m in &sh.doc.materials {
            let Some(path) = &m.normal_map else { continue };
            let entry = self.tex_cache.entry(format!("normal:{path}")).or_insert_with(|| {
                crate::textures::load(path).ok().map(|img| {
                    let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                    ctx.load_texture(format!("normal:{path}"), ci, egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR_REPEAT })
                })
            });
            if let Some(h) = entry {
                self.mat_normals.insert(m.id, (h.id(), m.normal_flip_green, m.normal_strength));
            }
        }
        if let Some(atlas) = self.pending_atlas.take() {
            let ci = egui::ColorImage::from_rgba_unmultiplied([atlas.width() as usize, atlas.height() as usize], atlas.as_raw());
            self.bb_tex = Some(ctx.load_texture("billboard-atlas", ci, egui::TextureOptions::LINEAR));
        }
        if let Some(t) = &self.bb_tex {
            self.mat_textures.insert(crate::billboard::BILLBOARD_MAT, t.id());
        }

        egui::Panel::left("generation").resizable(true).default_size(250.0).frame(egui::Frame::new().fill(PANEL).inner_margin(8)).show(ui, |ui| {
          egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            section_title(ui, "GENERATION");
            ui.add_space(2.0);
            egui::Frame::new().fill(PANEL_DARK).corner_radius(4).inner_margin(6).show(ui, |ui| {
                ui.set_min_height(150.0);
                ui.set_width(ui.available_width());
                let order = sh.doc.ordered();
                let mut remove = None;
                let mut add: Option<(GenKind, u32)> = None;
                for (id, depth) in order {
                    let Some(g) = sh.doc.get(id).cloned() else { continue };
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 14.0);
                        let mut en = g.enabled;
                        if ui.checkbox(&mut en, "").on_hover_text("Enable / disable").changed() {
                            if let Some(gm) = sh.doc.get_mut(id) {
                                gm.enabled = en;
                            }
                        }
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.5, kind_color(g.kind));
                        let label = RichText::new(format!("{}  ×{}", g.name, g.params.count)).color(if g.enabled { Color32::from_gray(225) } else { Color32::from_gray(120) });
                        let r = ui.selectable_label(sh.selected == Some(id), label);
                        if r.clicked() {
                            sh.selected = Some(id);
                        }
                        r.context_menu(|ui| {
                            if g.kind != GenKind::Leaves {
                                if ui.button("Add branches here").clicked() {
                                    add = Some((GenKind::Branches, id));
                                }
                                if ui.button("Add leaves here").clicked() {
                                    add = Some((GenKind::Leaves, id));
                                }
                            }
                            if ui.button("Delete").clicked() {
                                remove = Some(id);
                            }
                        });
                    });
                }
                if let Some(id) = remove {
                    sh.doc.remove(id);
                    sh.fix_selection();
                }
                if let Some((k, p)) = add {
                    if let Ok(id) = sh.doc.add(k, Some(p), None) {
                        sh.selected = Some(id);
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| add_buttons(ui, &mut sh));
            ui.add_space(10.0);
            section_title(ui, "MATERIALS");
            ui.add_space(2.0);
            materials_ui(ui, &mut sh, &self.mat_textures, &mut self.editing_material);
            ui.add_space(10.0);
            section_title(ui, "SPECIES PRESETS");
            ui.add_space(2.0);
            egui::Grid::new("presets").num_columns(2).spacing([6.0, 6.0]).show(ui, |ui| {
                for (i, (n, d)) in PRESETS.iter().enumerate() {
                    let current = sh.doc.name == *n;
                    let b = egui::Button::new(*n).selected(current).min_size(egui::vec2(108.0, 24.0));
                    if ui.add(b).on_hover_text(*d).clicked() {
                        sh.replace_doc(preset(n).unwrap());
                        sh.log(Source::You, format!("loaded preset {n}"));
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
          });
        });

        // -------- right: properties
        egui::Panel::right("properties").resizable(true).default_size(330.0).frame(egui::Frame::new().fill(PANEL).inner_margin(8)).show(ui, |ui| {
            let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
            if let Some(y) = self.force_scroll.take() {
                area = area.vertical_scroll_offset(y);
            }
            area.show(ui, |ui| {
                if sh.view.node_mode {
                    node_ui(ui, &mut sh, &self.skeleton);
                }
                properties_ui(ui, &mut sh, &self.stats, &mut self.editing_material);
                if let Some(g) = ui.data_mut(|d| d.remove_temp::<u32>(egui::Id::new("pick_leaf_mesh"))) {
                    actions.push(Action::PickLeafMesh(g));
                }
            });
        });

        // -------- bottom: agent activity / statistics
        egui::Panel::bottom("console").resizable(true).default_size(150.0).frame(egui::Frame::new().fill(PANEL_DARK).inner_margin(8)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.bottom_tab, BottomTab::Activity, "Agent activity");
                ui.selectable_value(&mut self.bottom_tab, BottomTab::Stats, "Statistics");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(url) = &sh.mcp.url {
                        ui.label(RichText::new(format!("MCP {url} · {} calls", sh.mcp.calls)).size(11.5).color(HEADER));
                    }
                });
            });
            ui.separator();
            match self.bottom_tab {
                BottomTab::Activity => {
                    egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                        for l in &sh.log {
                            let (tag, col) = match l.source {
                                Source::Agent => ("AGENT", Color32::from_rgb(130, 190, 250)),
                                Source::You => ("YOU  ", Color32::from_rgb(200, 200, 200)),
                                Source::App => ("APP  ", Color32::from_rgb(150, 200, 120)),
                            };
                            let m = (l.at / 60.0) as u32;
                            let s = l.at as u32 % 60;
                            ui.label(RichText::new(format!("{m:02}:{s:02}  {tag}  {}", l.text)).monospace().size(11.5).color(col));
                        }
                    });
                }
                BottomTab::Stats => {
                    let mut per_lod = vec![];
                    for l in 0..4 {
                        if l == sh.view.lod {
                            per_lod.push((l, self.stats.clone()));
                        }
                    }
                    egui::Grid::new("stats").striped(true).show(ui, |ui| {
                        let s = &self.stats;
                        for (k, v) in [
                            ("Triangles", s.triangles.to_string()),
                            ("Vertices", s.vertices.to_string()),
                            ("Stems", s.stems.to_string()),
                            ("Leaves", s.leaves.to_string()),
                            ("Height", format!("{:.2} m", s.height_m)),
                            ("Crown width", format!("{:.2} m", s.crown_width_m)),
                            ("Level of detail", format!("LOD{}", sh.view.lod)),
                        ] {
                            ui.label(k);
                            ui.label(RichText::new(v).strong());
                            ui.end_row();
                        }
                    });
                }
            }
        });

        // -------- centre: viewport
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::from_rgb(60, 66, 74))).show(ui, |ui| {
            let rect = ui.available_rect_before_wrap();
            let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
            let cam = &mut sh.view.camera;
            if resp.dragged_by(egui::PointerButton::Primary) {
                let d = resp.drag_delta();
                cam.yaw -= d.x * 0.4;
                cam.pitch += d.y * 0.3;
            }
            if resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle) {
                let d = resp.drag_delta();
                let yaw = cam.yaw.to_radians();
                let right = glam::Vec3::new(yaw.cos(), 0.0, -yaw.sin());
                let k = cam.distance * 0.0015;
                let t = glam::Vec3::from(cam.target) - right * d.x * k + glam::Vec3::Y * d.y * k;
                cam.target = t.into();
            }
            if resp.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y + (i.zoom_delta() - 1.0) * 200.0);
                if scroll != 0.0 {
                    cam.distance *= (-scroll * 0.0018).exp();
                }
            }
            cam.clamp();
            if resp.clicked() {
                if let Some(p) = resp.interact_pointer_pos() {
                    if sh.view.lod < crate::billboard::BILLBOARD_LOD {
                        if sh.view.node_mode {
                            let hit = self.pick.pick_vertex(&self.mesh, p).and_then(|v| self.mesh.stem.get(v).copied()).and_then(|si| self.skeleton.stems.get(si as usize));
                            sh.selected_node = hit.map(|s| s.key);
                            if let Some(s) = hit {
                                sh.selected = Some(s.gen);
                            }
                        } else {
                            sh.selected = self.pick.pick(&self.mesh, p);
                        }
                    }
                }
            }
            self.time += dt;
            if sh.view.wind_playing {
                ctx.request_repaint();
            }
            if sh.view.growth_playing {
                sh.view.growth = (sh.view.growth + dt / sh.doc.growth.duration.max(0.5)).min(1.0);
                if sh.view.growth >= 1.0 {
                    sh.view.growth_playing = false;
                }
                ctx.request_repaint();
            }
            // briefly flash a newly selected generator in the viewport
            if sh.selected != self.last_selected {
                if self.last_selected.is_some() || self.frames > 5 {
                    self.flash_until = self.time + 1.2;
                }
                self.last_selected = sh.selected;
            }
            let flashing = self.time < self.flash_until;
            if flashing {
                ctx.request_repaint();
            }
            let shown = if sh.view.highlight_selection || flashing { sh.selected } else { None };
            let mut view = sh.view.clone();
            view.highlight_selection = shown.is_some();
            let painter = ui.painter_at(rect);
            let growing = sh.view.growth < 1.0;
            if growing && self.grown.as_ref().map(|(g, _)| *g) != Some(sh.view.growth) {
                if self.grower.is_none() {
                    let timing = crate::growth::Timing::new(&sh.doc, &self.skeleton);
                    self.grower = Some(crate::growth::Grower::new(&sh.doc, &self.skeleton, &self.mesh, &timing));
                }
                let gm = self.grower.as_ref().unwrap().mesh_at(&self.mesh, sh.view.growth);
                self.grown = Some((sh.view.growth, gm));
            }
            let shown_mesh = match &self.grown {
                Some((_, gm)) if growing => gm,
                _ => &self.mesh,
            };
            let args = render::DrawArgs { doc: &sh.doc, mesh: shown_mesh, skeleton: &self.skeleton, view: &view, selected: shown, time: self.time, textures: &self.mat_textures, normals: &self.mat_normals, selected_stem: if sh.view.node_mode { sh.selected_node.and_then(|k| self.skeleton.stems.iter().position(|s| s.key == k)).map(|i| i as u32) } else { None }, gpu: if self.has_gl { Some(&self.gpu) } else { None } };
            self.pick = render::draw(&painter, rect, &args);
            let info = format!(
                "Perspective · {} · LOD{}{}",
                sh.view.mode.label(),
                sh.view.lod,
                if sh.view.wind_playing { " · Wind" } else { "" }
            ) + &growth_label(&sh.doc.growth, sh.view.growth);
            painter.text(rect.left_top() + egui::vec2(12.0, 10.0), egui::Align2::LEFT_TOP, info, egui::FontId::proportional(12.5), Color32::from_gray(230));
            // lighting switches, under the view label
            let light_rect = egui::Rect::from_min_size(rect.left_top() + egui::vec2(8.0, 30.0), egui::vec2(380.0, 22.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(light_rect), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for l in render::Lighting::ALL {
                        let on = sh.view.lighting == l;
                        let text = RichText::new(l.label()).size(11.5).color(if on { Color32::WHITE } else { Color32::from_gray(215) });
                        let b = egui::Button::new(text).fill(if on { Color32::from_rgb(68, 128, 52) } else { Color32::from_black_alpha(110) }).corner_radius(3).min_size(egui::vec2(0.0, 20.0));
                        if ui.add(b).on_hover_text("Viewport lighting").clicked() {
                            sh.view.lighting = l;
                        }
                    }
                });
            });
            let tris = crate::budget::group(self.stats.triangles as u32);
            let (label, colour) = match self.fit.as_ref().and_then(|f| f.budget.map(|b| (f, b))) {
                Some((f, b)) if f.over => (format!("{}  ·  ⚠ {tris} / {} tris, over budget", sh.doc.name, crate::budget::group(b)), WARN),
                Some((_, b)) => (format!("{}  ·  {tris} / {} tris", sh.doc.name, crate::budget::group(b)), Color32::from_gray(230)),
                None => (format!("{}  ·  {tris} tris", sh.doc.name), Color32::from_gray(230)),
            };
            let r = painter.text(rect.right_top() + egui::vec2(-12.0, 10.0), egui::Align2::RIGHT_TOP, label, egui::FontId::proportional(12.5), colour);
            if let Some(f) = self.fit.as_ref().filter(|f| f.budget.is_some()) {
                let hover = ui.interact(r.expand(3.0), ui.id().with("budget_hint"), egui::Sense::hover());
                hover.on_hover_text(format!("{}\nTriangle budgets: Tree › Level of detail.", f.message(sh.doc.budget.fit)));
            }
            painter.text(
                rect.right_bottom() + egui::vec2(-12.0, -10.0),
                egui::Align2::RIGHT_BOTTOM,
                "Drag: orbit · Right-drag: pan · Scroll: zoom · Click: select",
                egui::FontId::proportional(11.0),
                Color32::from_gray(200),
            );
        });

        // -------- windows
        if self.show_connect {
            let url = sh.mcp.url.clone().or(self.mcp_url.clone()).unwrap_or_else(|| format!("http://127.0.0.1:{}/mcp", self.port));
            let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "arbor_studio".into());
            let mut open = true;
            egui::Window::new("Connect a coding agent").open(&mut open).collapsible(false).resizable(false).default_width(560.0).show(&ctx, |ui| {
                ui.label("ArborStudio has a built-in MCP server. While this window is open, your coding agent can read and change the tree, render previews and export it.");
                ui.add_space(6.0);
                if let Some(e) = &sh.mcp.error {
                    ui.colored_label(Color32::from_rgb(230, 130, 100), format!("The server could not start: {e}. Close other ArborStudio windows and restart."));
                }
                connect_block(ui, "Claude Code (one command in a terminal):", &format!("claude mcp add --transport http arbor-studio {url}"));
                connect_block(ui, "Any MCP client that supports HTTP — server URL:", &url);
                connect_block(
                    ui,
                    "Clients that start servers themselves (stdio) — add to their MCP config:",
                    &format!("{{\n  \"mcpServers\": {{\n    \"arbor-studio\": {{\n      \"command\": \"{}\",\n      \"args\": [\"--mcp-stdio\"]\n    }}\n  }}\n}}", exe.replace('\\', "\\\\")),
                );
                ui.add_space(4.0);
                ui.label(RichText::new("Try asking your agent: “Load the pine preset, make it 20 m tall, render a preview and export it as GLB with LODs into my game's Assets folder.”").italics());
            });
            self.show_connect = open;
        }
        if let Some(mid) = self.editing_material {
            let open = material_editor(&ctx, &mut sh, mid, &self.mat_textures, &self.tex_cache, &mut actions);
            if !open {
                self.editing_material = None;
            }
        }
        if self.show_about {
            let mut open = true;
            egui::Window::new("About ArborStudio").open(&mut open).collapsible(false).resizable(false).show(&ctx, |ui| {
                ui.heading("ArborStudio");
                ui.label(format!("Procedural tree modeller · version {}", env!("CARGO_PKG_VERSION")));
                ui.label("Exports OBJ and glTF/GLB for Unity, Unreal, Godot and custom engines.");
                ui.label("Built-in MCP server for coding agents.");
            });
            self.show_about = open;
        }

        // -------- record undo history once the mouse is released
        if !ctx.input(|i| i.pointer.any_down()) {
            sh.commit();
        }
        let title = sh.title();
        drop(sh);
        if self.frames % 30 == 1 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        // -------- file dialogs run without holding the shared lock
        for a in actions {
            self.run_action(a);
        }

    }
}

fn connect_block(ui: &mut Ui, title: &str, code: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(title).strong());
    ui.horizontal(|ui| {
        egui::Frame::new().fill(PANEL_DARK).corner_radius(4).inner_margin(6).show(ui, |ui| {
            ui.set_width(440.0);
            ui.label(RichText::new(code).monospace().size(11.5));
        });
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(code.to_string());
        }
    });
}

fn randomize(sh: &mut Shared) {
    let t = sh.started.elapsed().as_nanos() as u64;
    sh.doc.settings.seed = (crate::grow::Rng::new(&[t, sh.doc.settings.seed as u64]).next_u64() % 100_000) as u32;
    sh.log(Source::You, format!("randomized seed → {}", sh.doc.settings.seed));
}

/// Add a generator of this kind: trunks grow from the ground, the others on the selected generator.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn add_generator(sh: &mut Shared, kind: GenKind) {
    let parent = if kind == GenKind::Trunk { None } else { sh.selected };
    if let Ok(id) = sh.doc.add(kind, parent, None) {
        sh.selected = Some(id);
    }
}

fn add_buttons(ui: &mut Ui, sh: &mut Shared) {
    let parent = sh.selected.and_then(|id| sh.doc.get(id)).map(|g| (g.id, g.kind));
    let can_child = matches!(parent, Some((_, k)) if k != GenKind::Leaves);
    if ui.button("+ Trunk").on_hover_text("A new trunk growing from the ground").clicked() {
        if let Ok(id) = sh.doc.add(GenKind::Trunk, None, None) {
            sh.selected = Some(id);
        }
    }
    if ui.add_enabled(can_child, egui::Button::new("+ Branches")).on_hover_text("Branches growing on the selected generator").clicked() {
        if let Ok(id) = sh.doc.add(GenKind::Branches, parent.map(|p| p.0), None) {
            sh.selected = Some(id);
        }
    }
    let on_trunk = matches!(parent, Some((_, GenKind::Trunk)));
    if ui.add_enabled(on_trunk, egui::Button::new("+ Roots")).on_hover_text("Roots spreading from the selected trunk along the ground").clicked() {
        if let Ok(id) = sh.doc.add(GenKind::Roots, parent.map(|p| p.0), None) {
            sh.selected = Some(id);
        }
    }
    if ui.add_enabled(can_child, egui::Button::new("+ Leaves")).on_hover_text("Leaves growing on the selected generator").clicked() {
        if let Ok(id) = sh.doc.add(GenKind::Leaves, parent.map(|p| p.0), None) {
            sh.selected = Some(id);
        }
    }
    if ui.add_enabled(sh.selected.is_some(), egui::Button::new("🗑")).on_hover_text("Delete the selected generator and everything on it").clicked() {
        if let Some(id) = sh.selected {
            sh.doc.remove(id);
            sh.fix_selection();
        }
    }
}

fn properties_ui(ui: &mut Ui, sh: &mut Shared, stats: &Stats, editing_material: &mut Option<u32>) {
    section_title(ui, "PROPERTIES");
    let mats: Vec<(u32, String)> = sh.doc.materials.iter().map(|m| (m.id, m.name.clone())).collect();
    if let Some(id) = sh.selected {
        let has_roots = sh.doc.children(Some(id)).any(|c| c.kind == GenKind::Roots && c.enabled);
        // roots, or branches somewhere under roots (side roots)
        let rootlike = {
            let mut cur = sh.doc.get(id);
            let mut found = false;
            let mut guard = 0;
            while let Some(c) = cur {
                if c.kind == GenKind::Roots {
                    found = true;
                    break;
                }
                guard += 1;
                if guard > 64 {
                    break;
                }
                cur = c.parent.and_then(|pid| sh.doc.get(pid));
            }
            found
        };
        if let Some(g) = sh.doc.get_mut(id) {
            let kind = g.kind;
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 5.0, kind_color(kind));
                ui.add(egui::TextEdit::singleline(&mut g.name).desired_width(170.0));
                ui.label(RichText::new(kind.label()).color(HEADER));
                ui.checkbox(&mut g.enabled, "On");
            });
            ui.horizontal(|ui| {
                ui.label("Material");
                let default_name = if kind == GenKind::Leaves { "Default leaves" } else { "Default bark" };
                let current = g.material.and_then(|m| mats.iter().find(|x| x.0 == m)).map(|x| x.1.clone()).unwrap_or_else(|| default_name.to_string());
                egui::ComboBox::from_id_salt("gen_material").width(160.0).selected_text(current).show_ui(ui, |ui| {
                    ui.selectable_value(&mut g.material, None, default_name);
                    for (mid, name) in &mats {
                        ui.selectable_value(&mut g.material, Some(*mid), name);
                    }
                });
                if let Some(m) = g.material {
                    if ui.small_button("Edit…").clicked() {
                        *editing_material = Some(m);
                    }
                }
            });
            if kind != GenKind::Leaves {
                material_blend_ui(ui, g, &mats);
            }
            let p = &mut g.params;
            let trunk = kind == GenKind::Trunk;
            let ccx = CurveCx { gid: id, kind, taper: p.taper, profile: p.profile };
            let leaves = kind == GenKind::Leaves;
            egui::CollapsingHeader::new("Generation").default_open(true).show(ui, |ui| {
                egui::Grid::new("gen").num_columns(3).show(ui, |ui| {
                    let max = if trunk { 24 } else { 120 };
                    crow(ui, if trunk { "Trunks" } else { "Count per parent" }, "count", &["density"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.count, if trunk { 1 } else { 0 }..=max)));
                    if !trunk {
                        prow(ui, "Start", "start", |ui| ui.add(egui::Slider::new(&mut p.start, 0.0..=1.0)));
                        prow(ui, "End", "end", |ui| ui.add(egui::Slider::new(&mut p.end, 0.0..=1.0)));
                        prow(ui, "Phyllotaxis", "phyllotaxis", |ui| ui.add(egui::Slider::new(&mut p.phyllotaxis, 0.0..=360.0).suffix("°")));
                        prow(ui, "Per node (whorl)", "whorl", |ui| ui.add(egui::Slider::new(&mut p.whorl, 1..=8)));
                    }
                    prow(ui, "Randomness", "random", |ui| ui.add(egui::Slider::new(&mut p.random, 0.0..=1.0)));
                    if !trunk {
                        prow(ui, "Spacing (0 = count)", "interval", |ui| ui.add(egui::Slider::new(&mut p.interval, 0.0..=5.0).suffix(" m")));
                    }
                });
            });
            egui::CollapsingHeader::new("Spine").default_open(true).show(ui, |ui| {
                egui::Grid::new("spine").num_columns(3).show(ui, |ui| {
                    if !leaves {
                        if trunk {
                            crow(ui, "Height", "length", &["length"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.length, 0.2..=40.0).suffix(" m").logarithmic(true)));
                        } else {
                            crow(ui, "Length (× parent)", "length", &["length"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.length, 0.02..=1.5)));
                        }
                        crow(ui, "Length variation", "length_variation", &["length_variation"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.length_variation, 0.0..=1.0)));
                        if !trunk {
                            prow(ui, "Crown profile", "profile", |ui| {
                                egui::ComboBox::from_id_salt("profile")
                                    .selected_text(p.profile.label())
                                    .show_ui(ui, |ui| {
                                        for pr in Profile::ALL {
                                            ui.selectable_value(&mut p.profile, pr, pr.label());
                                        }
                                    })
                                    .response
                            });
                        }
                    }
                    crow(ui, if trunk { "Lean" } else { "Angle" }, "angle", &["angle"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.angle, 0.0..=if trunk { 60.0 } else { 120.0 }).suffix("°")));
                    crow(ui, "Angle variation", "angle_variation", &["angle_variation"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.angle_variation, 0.0..=60.0).suffix("°")));
                    if !leaves {
                        if rootlike {
                            if p.gravity < 0.0 {
                                p.gravity = 0.0;
                            }
                            crow(ui, "Gravity (dive)", "gravity", &["gravity"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.gravity, 0.0..=1.0)).on_hover_text("0 = roots run along the ground, 1 = they dive straight down"));
                        } else {
                            crow(ui, "Gravity", "gravity", &["gravity"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.gravity, -1.0..=1.0)));
                        }
                        crow(ui, "Wiggle", "wiggle", &["wiggle"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.wiggle, 0.0..=1.0)));
                        prow(ui, "Segments", "segments", |ui| ui.add(egui::Slider::new(&mut p.segments, 1..=32)));
                        crow(ui, "Force response", "force_response", &["force_response"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.force_response, 0.0..=3.0)));
                        crow(ui, "Kink", "kink", &["kink"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.kink, 0.0..=120.0).suffix("°")));
                        crow(ui, "Jitter", "jitter", &["jitter"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.jitter, 0.0..=1.0)));
                        let split_label = |n: u32| if n < 2 { "No split".to_string() } else { format!("{n} leaders") };
                        prow(ui, "Split", "splits", |ui| {
                            egui::ComboBox::from_id_salt("splits")
                                .selected_text(split_label(p.splits))
                                .show_ui(ui, |ui| {
                                    for n in [0u32, 2, 3, 4, 5] {
                                        ui.selectable_value(&mut p.splits, n, split_label(n));
                                    }
                                })
                                .response
                        });
                        if p.splits >= 2 {
                            prow(ui, "Split height", "split_at", |ui| ui.add(egui::Slider::new(&mut p.split_at, 0.05..=0.95)));
                            prow(ui, "Split angle", "split_angle", |ui| ui.add(egui::Slider::new(&mut p.split_angle, 0.0..=80.0).suffix("°")));
                        }
                    }
                });
            });
            if !leaves {
                egui::CollapsingHeader::new("Skin").default_open(true).show(ui, |ui| {
                    egui::Grid::new("skin").num_columns(3).show(ui, |ui| {
                        if trunk {
                            crow(ui, "Radius", "radius", &["radius"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.radius, 0.01..=2.0).suffix(" m").logarithmic(true)));
                        } else {
                            crow(ui, "Radius (× parent)", "radius", &["radius"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.radius, 0.05..=1.0)));
                        }
                        crow(ui, "Taper", "taper", &["taper", "radius_profile"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.taper, 0.0..=1.0)));
                        if trunk {
                            prow(ui, "Buttress flare", "flare", |ui| ui.add(egui::Slider::new(&mut p.flare, 0.0..=2.0)));
                        } else if kind == GenKind::Branches {
                            prow(ui, "Branch welding", "weld", |ui| ui.add(egui::Slider::new(&mut p.weld, 0.0..=1.0)));
                            crow(ui, "Broken branches", "break_chance", &["break_chance"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.break_chance, 0.0..=1.0)));
                            if p.break_chance > 0.0 {
                                prow(ui, "Stub length", "break_length", |ui| ui.add(egui::Slider::new(&mut p.break_length, 0.03..=0.9)));
                            }
                        }
                        prow(ui, "Sides", "sides", |ui| ui.add(egui::Slider::new(&mut p.sides, 3..=24)));
                        if trunk && has_roots {
                            ui.label("Flutes");
                            ui.label(RichText::new("follow the roots").color(HEADER));
                            ui.end_row();
                        } else {
                            prow(ui, "Flutes", "flutes", |ui| ui.add(egui::Slider::new(&mut p.flutes, 0..=16)));
                        }
                        if p.flutes > 0 || (trunk && has_roots) {
                            crow(ui, "Flute depth", "flute_depth", &["flute_depth"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.flute_depth, 0.0..=0.8)));
                        }
                        crow(ui, "Flatten", "flatten", &["flatten"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.flatten, 0.0..=0.85)));
                        crow(ui, "Bark lumps", "bumps", &["bumps"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.bumps, 0.0..=0.6)));
                        if p.bumps > 0.0 {
                            prow(ui, "Lump size", "bump_size", |ui| ui.add(egui::Slider::new(&mut p.bump_size, 0.05..=3.0).suffix(" m").logarithmic(true)));
                        }
                    });
                });
                if kind == GenKind::Roots {
                    egui::CollapsingHeader::new("Ground").default_open(true).show(ui, |ui| {
                        egui::Grid::new("ground").num_columns(2).show(ui, |ui| {
                            prow(ui, "Sunk into ground", "buried", |ui| ui.add(egui::Slider::new(&mut p.buried, 0.0..=1.0)));
                        });
                        ui.label(RichText::new("Roots follow the ground; the trunk's buttress ridges line up with them.").size(11.0).color(HEADER));
                    });
                }
            } else {
                egui::CollapsingHeader::new("Leaves").default_open(true).show(ui, |ui| {
                    egui::Grid::new("leaves").num_columns(3).show(ui, |ui| {
                        prow(ui, "Shape", "leaf_shape", |ui| {
                            egui::ComboBox::from_id_salt("leafshape")
                                .selected_text(p.leaf_shape.label())
                                .show_ui(ui, |ui| {
                                    for s in LeafShape::ALL {
                                        ui.selectable_value(&mut p.leaf_shape, s, s.label());
                                    }
                                })
                                .response
                        });
                        if p.leaf_shape == LeafShape::Mesh {
                            let shown = p.leaf_mesh.as_deref().and_then(|s| std::path::Path::new(s).file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "none".into());
                            prow(ui, "Model", "leaf_mesh", |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(shown);
                                    if ui.small_button("Choose…").clicked() {
                                        ui.data_mut(|d| d.insert_temp(egui::Id::new("pick_leaf_mesh"), id));
                                    }
                                    if p.leaf_mesh.is_some() && ui.small_button("✕").clicked() {
                                        p.leaf_mesh = None;
                                    }
                                })
                                .response
                            });
                            if let Some(err) = p.leaf_mesh.as_deref().and_then(|s| crate::leafmesh::load(s).err()) {
                                ui.label(RichText::new(err).size(11.0).color(Color32::from_rgb(240, 120, 100)));
                                ui.end_row();
                            } else if p.leaf_mesh.is_none() {
                                ui.label(RichText::new("No model chosen: plain cards are used.").size(11.0).color(HEADER));
                                ui.end_row();
                            }
                        }
                        crow(ui, "Size", "leaf_size", &["leaf_size"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.leaf_size, 0.02..=6.0).suffix(" m").logarithmic(true)));
                        prow(ui, "Size variation", "leaf_size_variation", |ui| ui.add(egui::Slider::new(&mut p.leaf_size_variation, 0.0..=1.0)));
                        prow(ui, "Width", "leaf_aspect", |ui| ui.add(egui::Slider::new(&mut p.leaf_aspect, 0.1..=1.5)));
                        crow(ui, "Fold", "leaf_fold", &["leaf_fold"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.leaf_fold, 0.0..=89.0).suffix("°")));
                        crow(ui, "Curl", "leaf_bend", &["leaf_bend"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.leaf_bend, -1.0..=1.0)));
                        crow(ui, "Face the sky", "leaf_face_up", &["leaf_face_up"], &mut p.curves, ccx, |ui| ui.add(egui::Slider::new(&mut p.leaf_face_up, 0.0..=1.0)));
                    });
                });
            }
            gen_growth_ui(ui, &mut p.growth, kind);
            p.clamp(kind);
        }
    } else {
        ui.label(RichText::new("Select a generator on the left or click the tree.").italics().color(HEADER));
    }

    ui.add_space(8.0);
    section_title(ui, "TREE");
    let mut reseed = false;
    egui::CollapsingHeader::new("Appearance").default_open(true).show(ui, |ui| {
        egui::Grid::new("tree").num_columns(2).show(ui, |ui| {
            let d = &mut sh.doc;
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(170.0));
            ui.end_row();
            prow(ui, "Seed", "seed", |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut d.settings.seed).range(0..=99_999));
                    if ui.small_button("🎲").on_hover_text("Random seed").clicked() {
                        reseed = true;
                    }
                })
                .response
            });
            prow(ui, "Scale", "scale", |ui| ui.add(egui::Slider::new(&mut d.settings.scale, 0.1..=5.0).logarithmic(true)));
            prow(ui, "Age", "age", |ui| ui.add(egui::Slider::new(&mut d.settings.age, 0.05..=1.0)));
            prow(ui, "Join branches to parent", "join_branches", |ui| ui.checkbox(&mut d.settings.join_branches, "on"));
            if d.settings.join_branches {
                prow(ui, "Joint method", "join_method", |ui| {
                    egui::ComboBox::from_id_salt("join_method")
                        .selected_text(match d.settings.join_method {
                            JoinMethod::Stitch => "Cut & stitch (one mesh)",
                            JoinMethod::Weld => "Weld (wrap onto parent)",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut d.settings.join_method, JoinMethod::Stitch, "Cut & stitch (one mesh)").on_hover_text("An opening is cut into the parent and stitched to the branch: one watertight mesh");
                            ui.selectable_value(&mut d.settings.join_method, JoinMethod::Weld, "Weld (wrap onto parent)").on_hover_text("The parent stays whole; the branch base wraps onto its surface and blends its shading, colour and wind into the parent");
                        })
                        .response
                });
            }
            // one slider for the welding of every branch level (each level also has its own)
            let welds: Vec<f32> = d.generators.iter().filter(|g| g.kind == GenKind::Branches).map(|g| g.params.weld).collect();
            if !welds.is_empty() {
                let mut w = welds.iter().sum::<f32>() / welds.len() as f32;
                let mixed = welds.iter().any(|x| (x - w).abs() > 0.005);
                let mut changed = false;
                prow(ui, if mixed { "Branch welding (mixed)" } else { "Branch welding" }, "weld", |ui| {
                    let r = ui.add(egui::Slider::new(&mut w, 0.0..=1.0));
                    changed = r.changed();
                    r
                });
                if changed {
                    for g in d.generators.iter_mut().filter(|g| g.kind == GenKind::Branches) {
                        g.params.weld = w;
                    }
                }
            }
            prow(ui, "Default bark colour", "bark_color", |ui| ui.color_edit_button_srgb(&mut d.settings.bark_color));
            prow(ui, "Default leaf colour", "leaf_color", |ui| ui.color_edit_button_srgb(&mut d.settings.leaf_color));
            prow(ui, "Colour variation", "leaf_color_variation", |ui| ui.add(egui::Slider::new(&mut d.settings.leaf_color_variation, 0.0..=1.0)));
            prow(ui, "Autumn", "autumn", |ui| ui.add(egui::Slider::new(&mut d.settings.autumn, 0.0..=1.0)));
            prow(ui, "Fallen leaves", "leaf_drop", |ui| ui.add(egui::Slider::new(&mut d.settings.leaf_drop, 0.0..=1.0)));
            prow(ui, "Ambient occlusion", "ao_strength", |ui| ui.add(egui::Slider::new(&mut d.settings.ao_strength, 0.0..=1.0)));
        });
    });
    if reseed {
        randomize(sh);
    }
    egui::CollapsingHeader::new("Wind").default_open(true).show(ui, |ui| {
        egui::Grid::new("wind").num_columns(2).show(ui, |ui| {
            let w = &mut sh.doc.settings.wind;
            prow(ui, "Strength", "wind.strength", |ui| ui.add(egui::Slider::new(&mut w.strength, 0.0..=2.0)));
            prow(ui, "Direction", "wind.direction", |ui| ui.add(egui::Slider::new(&mut w.direction, 0.0..=360.0).suffix("°")));
            prow(ui, "Gustiness", "wind.gustiness", |ui| ui.add(egui::Slider::new(&mut w.gustiness, 0.0..=1.0)));
            prow(ui, "Speed", "wind.speed", |ui| ui.add(egui::Slider::new(&mut w.speed, 0.1..=4.0)));
            ui.label("Preview");
            ui.checkbox(&mut sh.view.wind_playing, "Play wind in viewport");
            ui.end_row();
        });
    });
    forces_ui(ui, sh);
    pruning_ui(ui, sh);
    collision_ui(ui, sh);
    egui::CollapsingHeader::new("Level of detail").default_open(false).open(force_open("Level of detail")).show(ui, |ui| {
        ui.horizontal(|ui| {
            for l in 0..4u32 {
                ui.radio_value(&mut sh.view.lod, l, format!("LOD{l}"));
            }
            ui.radio_value(&mut sh.view.lod, crate::billboard::BILLBOARD_LOD, "Billboard");
        });
        ui.label(RichText::new(format!("{} triangles at LOD{}", crate::budget::group(stats.triangles as u32), sh.view.lod)).color(HEADER));
        ui.label(RichText::new("Exports can include all levels, named _LOD0…_LOD4 (the billboard is LOD4) for engine LOD groups.").size(11.0).color(HEADER));
        ui.add_space(6.0);
        budget_ui(ui, sh);
    });
    growth_ui(ui, sh);
    // room below the last section, so its last rows scroll fully into view
    ui.add_space(16.0);
    sh.doc.settings.clamp();
}

/// Start the growth preview (from the start when it is at the mature tree), or pause it.
fn toggle_growth(v: &mut render::ViewState) {
    if v.growth_playing {
        v.growth_playing = false;
    } else {
        if v.growth >= 0.999 {
            v.growth = 0.0;
        }
        v.growth_playing = true;
    }
}

/// " · Growth 38% (Sapling)" for the viewport label while the growth is previewed.
fn growth_label(g: &crate::model::Growth, at: f32) -> String {
    if at >= 1.0 {
        return String::new();
    }
    let stage = g.stage_at(at).map(|i| format!(" ({})", g.label(i))).unwrap_or_default();
    format!(" · Growth {:.0}%{stage}", at * 100.0)
}

/// How the selected generator's stems or leaves grow in the growth animation.
fn gen_growth_ui(ui: &mut Ui, g: &mut crate::model::GenGrowth, kind: GenKind) {
    use crate::model::{GrowthCurve, GrowthSync};
    let leaves = kind == GenKind::Leaves;
    let trunk = kind == GenKind::Trunk;
    egui::CollapsingHeader::new("Growth").id_salt("gen_growth").default_open(false).open(force_open("Generator growth")).show(ui, |ui| {
        egui::Grid::new("gen_growth_grid").num_columns(2).show(ui, |ui| {
            prow(ui, "Speed", "growth.speed", |ui| ui.add(egui::Slider::new(&mut g.speed, 0.05..=10.0).logarithmic(true).custom_formatter(|x, _| format!("×{x:.2}"))));
            let curve = |ui: &mut Ui, id: &str, v: &mut GrowthCurve| {
                egui::ComboBox::from_id_salt(id.to_string())
                    .selected_text(v.label())
                    .show_ui(ui, |ui| {
                        for c in GrowthCurve::ALL {
                            ui.selectable_value(v, c, c.label());
                        }
                    })
                    .response
            };
            prow(ui, "Size curve", "growth.speed_curve", |ui| curve(ui, "gg_speed_curve", &mut g.speed_curve));
            if !leaves {
                prow(ui, "Radius curve", "growth.radius_curve", |ui| curve(ui, "gg_radius_curve", &mut g.radius_curve));
            }
            prow(ui, "Start size", "growth.start_size", |ui| ui.add(egui::Slider::new(&mut g.start_size, 0.0..=1.0).custom_formatter(|x, _| format!("{:.0}%", x * 100.0))));
            if !trunk {
                prow(ui, if leaves { "Start orientation" } else { "Start angle" }, "growth.start_angle", |ui| ui.add(egui::Slider::new(&mut g.start_angle, 0.0..=1.0).custom_formatter(|x, _| format!("{:.0}%", x * 100.0))));
                if leaves {
                    prow(ui, "Orientation time", "growth.orientation_time", |ui| ui.add(egui::Slider::new(&mut g.orientation_time, 0.1..=5.0).logarithmic(true).custom_formatter(|x, _| format!("×{x:.2}"))));
                }
                prow(ui, "Sync with", "growth.sync", |ui| {
                    egui::ComboBox::from_id_salt("gg_sync")
                        .selected_text(g.sync.label())
                        .show_ui(ui, |ui| {
                            for c in GrowthSync::ALL {
                                ui.selectable_value(&mut g.sync, c, c.label());
                            }
                        })
                        .response
                });
                let uses_pct = match g.sync {
                    GrowthSync::Parent | GrowthSync::Siblings => true,
                    _ => false,
                };
                if uses_pct {
                    prow(ui, "Start %", "growth.start_pct", |ui| ui.add(egui::Slider::new(&mut g.start_pct, 0.0..=1.0).custom_formatter(|x, _| format!("{:.0}%", x * 100.0))));
                }
                prow(ui, "Offset", "growth.offset", |ui| ui.add(egui::Slider::new(&mut g.offset, 0.0..=3.0)));
                prow(ui, "Offset variation", "growth.offset_variation", |ui| ui.add(egui::Slider::new(&mut g.offset_variation, 0.0..=3.0)));
            }
        });
        ui.label(RichText::new("Preview with ▶ Grow or the Growth slider under TREE.").size(11.0).color(HEADER));
    });
}

/// Growth animation: preview, stages and export settings.
fn growth_ui(ui: &mut Ui, sh: &mut Shared) {
    egui::CollapsingHeader::new("Growth").default_open(false).open(force_open("Growth")).show(ui, |ui| {
        ui.horizontal(|ui| {
            let label = if sh.view.growth_playing { "⏸ Pause" } else { "▶ Grow" };
            if ui.button(label).on_hover_text("Play the growth in the viewport, from nothing to the mature tree").clicked() {
                toggle_growth(&mut sh.view);
            }
            if ui.add(egui::Slider::new(&mut sh.view.growth, 0.0..=1.0).show_value(false)).on_hover_text("Scrub through the growth: 0 = nothing above ground, 1 = the mature tree").changed() {
                sh.view.growth_playing = false;
            }
            ui.label(RichText::new(format!("{:.0}%", sh.view.growth * 100.0)).color(HEADER));
        });
        ui.label(RichText::new("How each part grows (speed, radius, when it starts) is set per generator, in its Growth group.").size(11.0).color(HEADER));
        ui.add_space(4.0);
        section_title(ui, "Stages");
        let mut jump = None;
        let mut add = false;
        {
            let g = &sh.doc.growth;
            let sel = g.stage_at(sh.view.growth);
            ui.horizontal_wrapped(|ui| {
                for i in 0..g.stages.len() {
                    let st = &g.stages[i];
                    let label = g.label(i);
                    if ui.selectable_label(sel == Some(i), &label).on_hover_text(format!("Show and edit the tree at {label} ({:.0}% of the growth)", st.at * 100.0)).clicked() {
                        jump = Some(st.at);
                    }
                }
                if g.stages.len() < crate::model::Growth::MAX_STAGES && ui.small_button("+ Stage").on_hover_text("Add a stage at the growth shown in the viewport (or in the largest gap between stages)").clicked() {
                    add = true;
                }
            });
        }
        if add {
            let g = &mut sh.doc.growth;
            let here = sh.view.growth;
            let at = if g.stage_at(here).is_none() && here >= 0.01 {
                here
            } else {
                let mut pts: Vec<f32> = g.stages.iter().map(|s| s.at).collect();
                pts.push(0.0);
                pts.push(1.0);
                pts.sort_by(f32::total_cmp);
                let w = pts.windows(2).max_by(|a, b| (a[1] - a[0]).total_cmp(&(b[1] - b[0]))).unwrap();
                ((w[0] + w[1]) * 0.5 * 100.0).round() / 100.0
            };
            g.stages.push(crate::model::GrowthStage { name: String::new(), at });
            g.stages.sort_by(|a, b| a.at.total_cmp(&b.at));
            jump = Some(at);
        }
        if let Some(at) = jump {
            sh.view.growth = at;
            sh.view.growth_playing = false;
        }
        match sh.doc.growth.stage_at(sh.view.growth) {
            None => {
                ui.label(RichText::new("Pick a stage to edit it, or move the preview and click + Stage to add one there.").size(11.0).color(HEADER));
            }
            Some(i) => stage_ui(ui, sh, i),
        }
        ui.add_space(6.0);
        let g = &mut sh.doc.growth;
        egui::Grid::new("growth_settings").num_columns(2).show(ui, |ui| {
            prow(ui, "Duration", "growth.duration", |ui| ui.add(egui::Slider::new(&mut g.duration, 0.5..=120.0).suffix(" s").logarithmic(true)));
            prow(ui, "Keyframes", "growth.keyframes", |ui| ui.add(egui::Slider::new(&mut g.keyframes, 4..=48)));
            prow(ui, "Export", "growth.export", |ui| ui.checkbox(&mut g.export, "Include the growth in exports"));
        });
        let keys = crate::growth::keyframes(g).len();
        ui.label(
            RichText::new(format!(
                "GLB: {} morph targets, a Grow clip and one clip per stage. OBJ: one file per stage. JSON: the growth timing of every stem and leaf.",
                keys - 1
            ))
            .size(11.0)
            .color(HEADER),
        );
    });
}

/// One growth stage: its optional name and its place in the growth.
fn stage_ui(ui: &mut Ui, sh: &mut Shared, i: usize) {
    let g = &mut sh.doc.growth;
    let n = g.stages.len();
    let hint = format!("Stage {}", i + 1);
    let mut remove = false;
    let mut moved = None;
    egui::Frame::group(ui.style()).inner_margin(6.0).show(ui, |ui| {
        egui::Grid::new("growth_stage").num_columns(2).show(ui, |ui| {
            ui.label("Name").on_hover_text(help_for("growth.stages"));
            ui.horizontal(|ui| {
                ui.add_sized([120.0, 20.0], egui::TextEdit::singleline(&mut g.stages[i].name).hint_text(hint)).on_hover_text("Optional; unnamed stages are called by their place");
                if n > 1 && ui.small_button("🗑 Remove").on_hover_text("Remove this stage").clicked() {
                    remove = true;
                }
            });
            ui.end_row();
            ui.label("At").on_hover_text(help_for("growth.stages"));
            let st = &mut g.stages[i];
            if ui.add(egui::Slider::new(&mut st.at, 0.01..=1.0).fixed_decimals(2)).on_hover_text("Where in the growth this stage is: 0 = nothing above ground, 1 = the mature tree").changed() {
                moved = Some(st.at);
            }
            ui.end_row();
        });
    });
    if remove {
        g.stages.remove(i);
    } else if let Some(at) = moved {
        g.stages.sort_by(|a, b| a.at.total_cmp(&b.at));
        sh.view.growth = at;
        sh.view.growth_playing = false;
    }
}

/// Triangle budget controls and the check of all four mesh levels.
fn budget_ui(ui: &mut Ui, sh: &mut Shared) {
    let b = &mut sh.doc.budget;
    ui.checkbox(&mut b.enabled, "Triangle budget").on_hover_text(help_for("budget.enabled"));
    if !b.enabled {
        return;
    }
    egui::Grid::new("budget_grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("Budgets").on_hover_text(help_for("budget.auto"));
        ui.horizontal(|ui| {
            ui.radio_value(&mut b.auto, true, "From a maximum");
            ui.radio_value(&mut b.auto, false, "Per level");
        });
        ui.end_row();
        if b.auto {
            ui.label("Maximum (LOD0)").on_hover_text(help_for("budget.max"));
            ui.add(egui::DragValue::new(&mut b.max).range(16..=crate::model::TriangleBudget::LIMIT).speed(100.0).suffix(" tris"));
            ui.end_row();
            let d = b.derived();
            ui.label("");
            ui.label(RichText::new(format!("LOD1 {}  ·  LOD2 {}  ·  LOD3 {}", crate::budget::group(d[1]), crate::budget::group(d[2]), crate::budget::group(d[3]))).size(11.0).color(HEADER));
            ui.end_row();
        } else {
            for l in 0..4 {
                ui.label(format!("LOD{l}")).on_hover_text(help_for("budget.levels"));
                ui.add(egui::DragValue::new(&mut b.levels[l]).range(0..=crate::model::TriangleBudget::LIMIT).speed(100.0).custom_formatter(|v, _| if v == 0.0 { "no limit".into() } else { format!("{} tris", v as u32) }));
                ui.end_row();
            }
        }
        ui.label("Fit");
        ui.checkbox(&mut b.fit, "Reduce detail to fit").on_hover_text(help_for("budget.fit"));
        ui.end_row();
        ui.label("Leaf area").on_hover_text(help_for("budget.leaf_area"));
        ui.horizontal(|ui| {
            for l in 0..4 {
                ui.add(egui::DragValue::new(&mut b.leaf_area[l]).range(0.0..=100_000.0).speed(0.5).max_decimals(1).custom_formatter(|v, _| if v == 0.0 { "–".into() } else { format!("{v:.1}") }))
                    .on_hover_text(format!("Most leaf card area for LOD{l} in m² (– = no limit)"));
            }
            ui.label(RichText::new("m²").size(11.0).color(HEADER));
        });
        ui.end_row();
    });
    let doc = sh.doc.clone();
    let (report, fresh) = budget_report(ui.ctx(), &doc);
    ui.add_space(4.0);
    if report.is_empty() {
        ui.label(RichText::new("Checking the levels…").size(11.0).color(HEADER));
        return;
    }
    for f in &report {
        let colour = if f.over { WARN } else if !fresh { Color32::from_gray(120) } else { HEADER };
        let icon = if f.over { "⚠ " } else { "" };
        ui.label(RichText::new(format!("{icon}{}", f.message(doc.budget.fit))).size(11.0).color(colour));
        let acol = if f.area_over { WARN } else if !fresh { Color32::from_gray(120) } else { HEADER };
        let aicon = if f.area_over { "⚠ " } else { "   " };
        ui.label(RichText::new(format!("{aicon}{}", f.area_message())).size(10.5).color(acol));
    }
    if !fresh {
        ui.label(RichText::new("Checking the levels…").size(11.0).color(HEADER));
    }
}

/// The budget check of all four levels, run off the interface thread: growing and meshing
/// every level of a large tree takes a moment. Returns the latest result and whether it is for
/// the current tree; one check runs at a time and the next starts when the tree changed.
fn budget_report(ctx: &egui::Context, doc: &Document) -> (Vec<crate::budget::LevelFit>, bool) {
    struct State {
        done_for: Option<Document>,
        result: Vec<crate::budget::LevelFit>,
        running: bool,
    }
    static STATE: Mutex<State> = Mutex::new(State { done_for: None, result: Vec::new(), running: false });
    let mut st = STATE.lock().unwrap();
    if st.done_for.as_ref() == Some(doc) {
        return (st.result.clone(), true);
    }
    if !st.running {
        st.running = true;
        let (doc, ctx) = (doc.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = crate::budget::report(&doc);
            let mut st = STATE.lock().unwrap();
            st.done_for = Some(doc);
            st.result = r;
            st.running = false;
            ctx.request_repaint();
        });
    }
    (st.result.clone(), false)
}

/// Sections the documentation script asked to open (each opens once).
static FORCE_OPEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn force_open(name: &str) -> Option<bool> {
    let mut v = FORCE_OPEN.lock().unwrap();
    let i = v.iter().position(|n| n == name)?;
    v.remove(i);
    Some(true)
}

impl ArborApp {
    /// Documentation pictures: carry out the script's steps until the next wait or capture.
    fn run_script(&mut self, ctx: &egui::Context) {
        let Some(sc) = self.script.as_mut() else { return };
        ctx.request_repaint();
        if sc.pending.is_some() {
            return;
        }
        if sc.wait > 0 {
            sc.wait -= 1;
            return;
        }
        loop {
            let Some(step) = self.script.as_mut().and_then(|s| s.steps.pop_front()) else {
                self.script = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            };
            if let Some(name) = step.get("tool").and_then(|v| v.as_str()) {
                let server = mcp::Server { shared: self.shared.clone(), ctx: ctx.clone(), headless: false };
                let args = step.get("args").cloned().unwrap_or(serde_json::json!({}));
                if let Err(e) = mcp::call_tool(&server, name, &args) {
                    eprintln!("shot script: {name}: {e}");
                }
            } else if let Some(u) = step.get("ui") {
                if let Some(m) = u.get("material") {
                    self.editing_material = m.as_u64().map(|v| v as u32);
                }
                if let Some(v) = u.get("connect").and_then(|v| v.as_bool()) {
                    self.show_connect = v;
                }
                if let Some(v) = u.get("about").and_then(|v| v.as_bool()) {
                    self.show_about = v;
                }
                if let Some(t) = u.get("tab").and_then(|v| v.as_str()) {
                    self.bottom_tab = if t == "stats" { BottomTab::Stats } else { BottomTab::Activity };
                }
                if let Some(c) = u.get("curve").and_then(|v| v.as_array()) {
                    if let (Some(gid), Some(field)) = (c.first().and_then(|v| v.as_u64()), c.get(1).and_then(|v| v.as_str())) {
                        let id = egui::Id::new(("curve_popup", gid as u32, field));
                        egui::Popup::open_id(ctx, id);
                        if let Some(s) = self.script.as_mut() {
                            s.popup = Some(id);
                        }
                    }
                }
                if let Some(names) = u.get("expand").and_then(|v| v.as_array()) {
                    FORCE_OPEN.lock().unwrap().extend(names.iter().filter_map(|n| n.as_str().map(str::to_string)));
                }
                if let Some(y) = u.get("scroll").and_then(|v| v.as_f64()) {
                    self.force_scroll = Some(y as f32);
                }
                if let Some(v) = u.get("node_mode").and_then(|v| v.as_bool()) {
                    self.shared.lock().unwrap().view.node_mode = v;
                }
                if let Some(k) = u.get("node") {
                    self.shared.lock().unwrap().selected_node = k.as_u64().map(|v| v as u32);
                }
                if u.get("close").and_then(|v| v.as_bool()) == Some(true) {
                    self.editing_material = None;
                    self.show_connect = false;
                    self.show_about = false;
                    if let Some(id) = self.script.as_mut().and_then(|s| s.popup.take()) {
                        egui::Popup::close_id(ctx, id);
                    }
                }
            } else if let Some(n) = step.get("wait").and_then(|v| v.as_u64()) {
                if let Some(s) = self.script.as_mut() {
                    s.wait = n as u32;
                }
                return;
            } else if let Some(file) = step.get("capture").and_then(|v| v.as_str()) {
                let rect = step.get("rect").and_then(|v| v.as_array()).and_then(|r| {
                    let f: Vec<f32> = r.iter().filter_map(|x| x.as_f64().map(|x| x as f32)).collect();
                    (f.len() == 4).then(|| egui::Rect::from_min_size(egui::pos2(f[0], f[1]), egui::vec2(f[2], f[3])))
                });
                if let Some(s) = self.script.as_mut() {
                    s.pending = Some((s.resolve(file), rect));
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                self.shot_in_flight = true;
                return;
            }
        }
    }

    /// macOS: carry out menu-bar choices and show the app's state in the menus.
    #[cfg(target_os = "macos")]
    fn native_menu_frame(&mut self, sh: &mut Shared, actions: &mut Vec<Action>, ctx: &egui::Context) {
        use crate::native_menu::{MenuState, RENDER_MODES};
        let Some(menu) = self.native_menu.as_mut() else { return };
        for id in menu.take_events() {
            let (head, n) = match id.split_once('.') {
                Some((h, rest)) => (h, rest.parse::<usize>().ok()),
                None => (id.as_str(), None),
            };
            match (head, n, id.as_str()) {
                ("recent", Some(i), _) => {
                    if let Some(p) = menu.recent.get(i) {
                        actions.push(Action::OpenPath(p.clone()));
                    }
                }
                ("preset", Some(i), _) => {
                    if let Some((name, _)) = PRESETS.get(i) {
                        sh.replace_doc(preset(name).unwrap());
                        sh.log(Source::You, format!("loaded preset {name}"));
                    }
                }
                ("mode", Some(i), _) => {
                    if let Some(m) = RENDER_MODES.get(i) {
                        sh.view.mode = *m;
                    }
                }
                (_, _, "recent.clear") => crate::recent::clear(),
                (_, _, "recent.remove_missing") => {
                    for p in menu.recent.iter().filter(|p| !p.exists()) {
                        crate::recent::remove(p);
                    }
                }
                (_, _, "app.quit") => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                (_, _, "file.new") => {
                    let mut d = preset("Sapling").unwrap();
                    d.name = "New tree".into();
                    sh.replace_doc(d);
                    sh.file = None;
                }
                (_, _, "file.open") => actions.push(Action::Open),
                (_, _, "file.save") => actions.push(Action::Save),
                (_, _, "file.save_as") => actions.push(Action::SaveAs),
                (_, _, "file.lods") => self.export_lods = !self.export_lods,
                (_, _, "file.billboard") => self.export_billboard = !self.export_billboard,
                (_, _, "file.export_glb") => actions.push(Action::Export(Format::Glb, self.export_lods)),
                (_, _, "file.export_obj") => actions.push(Action::Export(Format::Obj, self.export_lods)),
                (_, _, "file.export_json") => actions.push(Action::Export(Format::Json, false)),
                (_, _, "file.variations_glb") => actions.push(Action::ExportVariations(Format::Glb)),
                (_, _, "file.variations_obj") => actions.push(Action::ExportVariations(Format::Obj)),
                (_, _, "edit.undo") => {
                    sh.undo();
                }
                (_, _, "edit.redo") => {
                    sh.redo();
                }
                (_, _, "edit.delete") | (_, _, "gen.delete") => {
                    if let Some(id) = sh.selected {
                        sh.doc.remove(id);
                        sh.fix_selection();
                    }
                }
                (_, _, "edit.seed") => randomize(sh),
                (_, _, "gen.trunk") => add_generator(sh, GenKind::Trunk),
                (_, _, "gen.branches") => add_generator(sh, GenKind::Branches),
                (_, _, "gen.roots") => add_generator(sh, GenKind::Roots),
                (_, _, "gen.leaves") => add_generator(sh, GenKind::Leaves),
                (_, _, "view.grid") => sh.view.show_grid = !sh.view.show_grid,
                (_, _, "view.leaves") => sh.view.show_leaves = !sh.view.show_leaves,
                (_, _, "view.highlight") => sh.view.highlight_selection = !sh.view.highlight_selection,
                (_, _, "view.wind") => sh.view.wind_playing = !sh.view.wind_playing,
                (_, _, "view.helpers") => sh.view.show_helpers = !sh.view.show_helpers,
                (_, _, "view.collision") => sh.view.show_collision = !sh.view.show_collision,
                (_, _, "view.frame") => sh.frame_camera = true,
                (_, _, "agent.connect") => self.show_connect = true,
                (_, _, "help.docs") => actions.push(Action::Help(None)),
                (_, _, "help.shortcuts") => actions.push(Action::Help(Some("shortcuts.html"))),
                _ => {}
            }
        }
        let parent = sh.selected.and_then(|id| sh.doc.get(id)).map(|g| g.kind);
        let v = &sh.view;
        menu.sync(MenuState {
            can_undo: !sh.undo.is_empty() || sh.doc != sh.committed,
            can_redo: !sh.redo.is_empty(),
            has_selection: sh.selected.is_some(),
            can_add_child: matches!(parent, Some(k) if k != GenKind::Leaves),
            can_add_roots: parent == Some(GenKind::Trunk),
            export_lods: self.export_lods,
            export_billboard: self.export_billboard,
            mode: RENDER_MODES.iter().position(|m| *m == v.mode).unwrap_or(0),
            toggles: [v.show_grid, v.show_leaves, v.highlight_selection, v.wind_playing, v.show_helpers, v.show_collision],
        });
    }

    fn open_path(&mut self, p: std::path::PathBuf) {
        match load_project(&p) {
            Ok(d) => {
                crate::recent::add(&p);
                let mut sh = self.shared.lock().unwrap();
                sh.replace_doc(d);
                sh.file = Some(p.clone());
                sh.dirty = false;
                sh.log(Source::You, format!("opened {}", p.display()));
                self.status = format!("Opened {}", p.display());
            }
            Err(e) => self.status = e,
        }
    }

    fn run_action(&mut self, a: Action) {
        match a {
            Action::Open => {
                if let Some(p) = rfd::FileDialog::new().add_filter("Tree project", &["json"]).pick_file() {
                    self.open_path(p);
                }
            }
            Action::OpenPath(p) => self.open_path(p),
            Action::Save | Action::SaveAs => {
                let current = self.shared.lock().unwrap().file.clone();
                let path = match (a, current) {
                    (Action::Save, Some(p)) => Some(p),
                    _ => {
                        let name = self.shared.lock().unwrap().doc.name.clone();
                        rfd::FileDialog::new().add_filter("Tree project", &["json"]).set_file_name(format!("{name}.arbor.json")).save_file()
                    }
                };
                if let Some(p) = path {
                    let mut sh = self.shared.lock().unwrap();
                    sh.commit();
                    match save_project(&sh.doc, &p) {
                        Ok(()) => {
                            crate::recent::add(&p);
                            sh.file = Some(p.clone());
                            sh.dirty = false;
                            self.status = format!("Saved {}", p.display());
                        }
                        Err(e) => self.status = e,
                    }
                }
            }
            Action::PickLeafMesh(gid) => {
                if let Some(p) = rfd::FileDialog::new().add_filter("OBJ model", &["obj"]).pick_file() {
                    let ps = p.display().to_string();
                    match crate::leafmesh::load(&ps) {
                        Ok(lm) => {
                            let mut sh = self.shared.lock().unwrap();
                            if let Some(g) = sh.doc.get_mut(gid) {
                                g.params.leaf_mesh = Some(ps.clone());
                                g.params.leaf_shape = LeafShape::Mesh;
                            }
                            sh.commit();
                            self.status = format!("Leaf model {} ({} triangles)", ps, lm.tris.len());
                        }
                        Err(e) => self.status = e,
                    }
                }
            }
            Action::PickTexture(mid, normal) => {
                if let Some(p) = rfd::FileDialog::new().add_filter("Image", &["png", "jpg", "jpeg"]).pick_file() {
                    let ps = p.display().to_string();
                    match crate::textures::check(&ps) {
                        Ok(_) => {
                            let has_alpha = crate::textures::load(&ps).map(|img| img.pixels().any(|px| px.0[3] < 250)).unwrap_or(false);
                            let mut sh = self.shared.lock().unwrap();
                            if let Some(m) = sh.doc.material_mut(mid) {
                                if normal {
                                    m.normal_map = Some(ps.clone());
                                } else {
                                    m.texture = Some(ps.clone());
                                    // images with transparency are almost always leaf cards
                                    if has_alpha {
                                        m.alpha_cutout = true;
                                    }
                                }
                            }
                            self.tex_cache.remove(&ps);
                            sh.log(Source::You, format!("material texture set to {ps}"));
                            self.status = format!("Texture loaded: {ps}");
                        }
                        Err(e) => self.status = e,
                    }
                }
            }
            Action::Export(fmt, lods) => {
                let bb = self.export_billboard;
                let (doc, name) = {
                    let sh = self.shared.lock().unwrap();
                    (sh.doc.clone(), sh.doc.name.clone())
                };
                let ext = match fmt {
                    Format::Obj => "obj",
                    Format::Glb => "glb",
                    Format::Json => "json",
                };
                if let Some(p) = rfd::FileDialog::new().add_filter(ext.to_uppercase(), &[ext]).set_file_name(format!("{name}.{ext}")).save_file() {
                    self.export_to(&doc, &p, fmt, lods, bb);
                }
            }
            Action::Help(page) => {
                match crate::help::open(page.unwrap_or("index.html")) {
                    Ok(address) => self.status = format!("Opened the help in the web browser: {address}"),
                    Err(e) => self.status = e,
                }
            }
            Action::ExportVariations(fmt) => {
                let doc = export::with_baked_ao(self.shared.lock().unwrap().doc.clone(), self.export_bake_ao);
                if let Some(dir) = rfd::FileDialog::new().set_title("Folder for the variations").pick_folder() {
                    match export::export_variations(&doc, &dir, self.variation_count, doc.settings.seed, fmt, self.export_lods, self.export_billboard, self.export_bake_blend) {
                        Ok(v) => {
                            self.status = format!("Exported {} variations to {}", v.len(), dir.display());
                            let over = v.iter().filter(|(_, _, w)| w.is_some()).count();
                            if over > 0 {
                                self.status = format!("{}. {over} of them are over the triangle budget (Tree › Level of detail shows the levels).", self.status);
                            }
                            self.shared.lock().unwrap().log(Source::You, self.status.clone());
                        }
                        Err(e) => self.status = e,
                    }
                }
            }
        }
    }

    fn export_to(&mut self, doc: &Document, p: &std::path::Path, fmt: Format, lods: bool, bb: bool) {
        let without_ao;
        let doc = if self.export_bake_ao { doc } else { without_ao = export::with_baked_ao(doc.clone(), false); &without_ao };
        match export::export(doc, p, fmt, lods, bb, self.export_bake_blend) {
            Ok(r) => {
                self.status = format!("Exported {}", r.files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>().join(", "));
                if let Some(w) = r.budget_warning(doc.budget.fit) {
                    self.status = format!("{}. {w}", self.status);
                }
                self.shared.lock().unwrap().log(Source::You, self.status.clone());
            }
            Err(e) => self.status = e,
        }
    }
}

fn swatch(ui: &mut Ui, color: [u8; 3], tex: Option<egui::TextureId>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    let c = Color32::from_rgb(color[0], color[1], color[2]);
    match tex {
        Some(t) => {
            ui.painter().image(t, rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), c);
        }
        None => {
            ui.painter().rect_filled(rect, 3, c);
        }
    }
    ui.painter().rect_stroke(rect, 3, egui::Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Inside);
}

/// Leaf pictures in a texture atlas: rectangles in pixels of the texture, each with a pivot
/// (where the stalk meets the twig) and a weight, drawn over a preview of the image.
fn atlas_ui(ui: &mut Ui, m: &mut Material, tex: Option<egui::TextureId>) {
    let Some((w, h)) = m.texture.as_deref().and_then(|p| image::image_dimensions(p).ok()) else { return };
    let (w, h) = (w as f32, h as f32);
    let title = if m.atlas.is_empty() { "Leaf atlas".to_string() } else { format!("Leaf atlas ({} pictures)", m.atlas.len()) };
    egui::CollapsingHeader::new(title).id_salt("leaf_atlas").default_open(!m.atlas.is_empty()).show(ui, |ui| {
        ui.label(RichText::new("For a texture with several leaves: one rectangle per leaf picture. Each leaf card shows one of them, picked at random by weight, shaped like the picture and hanging from its pivot (the dot). Pixels of the image, origin top left.").small().color(HEADER));
        if let Some(t) = tex {
            let side = 200.0;
            let size = if w >= h { egui::vec2(side, side * h / w) } else { egui::vec2(side * w / h, side) };
            let r = ui.add(egui::Image::new((t, size))).rect;
            let p = ui.painter();
            for (i, c) in m.atlas.iter().enumerate() {
                let rr = egui::Rect::from_min_size(r.min + egui::vec2(c.rect[0] * size.x, c.rect[1] * size.y), egui::vec2(c.rect[2] * size.x, c.rect[3] * size.y));
                let col = Color32::from_rgb(255, 200, 60);
                p.rect_stroke(rr, 0.0, egui::Stroke::new(1.0, col), egui::StrokeKind::Inside);
                let pv = rr.min + egui::vec2(c.pivot[0] * rr.width(), c.pivot[1] * rr.height());
                p.circle_filled(pv, 2.5, Color32::from_rgb(255, 80, 60));
                p.text(rr.min + egui::vec2(2.0, 1.0), egui::Align2::LEFT_TOP, format!("{}", i + 1), egui::FontId::proportional(10.0), col);
            }
        }
        let mut remove = None;
        egui::Grid::new("atlas_grid").num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
            for (i, c) in m.atlas.iter_mut().enumerate() {
                ui.label(format!("{}", i + 1));
                ui.vertical(|ui| {
                    let mut px = [c.rect[0] * w, c.rect[1] * h, c.rect[2] * w, c.rect[3] * h];
                    let mut pv = [c.pivot[0] * px[2], c.pivot[1] * px[3]];
                    ui.horizontal(|ui| {
                        for (k, (lab, max)) in [("x", w), ("y", h), ("w", w), ("h", h)].into_iter().enumerate() {
                            ui.label(lab);
                            ui.add(egui::DragValue::new(&mut px[k]).range(0.0..=max).speed(1.0).max_decimals(0));
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("pivot");
                        ui.add(egui::DragValue::new(&mut pv[0]).range(0.0..=px[2]).speed(1.0).max_decimals(0)).on_hover_text("From the rectangle's left edge, in pixels");
                        ui.add(egui::DragValue::new(&mut pv[1]).range(0.0..=px[3]).speed(1.0).max_decimals(0)).on_hover_text("From the rectangle's top edge, in pixels");
                        ui.label("weight");
                        ui.add(egui::DragValue::new(&mut c.weight).range(0.0..=100.0).speed(0.05).max_decimals(2));
                        if ui.small_button("🗑").on_hover_text("Remove this picture").clicked() {
                            remove = Some(i);
                        }
                    });
                    c.rect = [px[0] / w, px[1] / h, px[2].max(1.0) / w, px[3].max(1.0) / h];
                    c.pivot = [pv[0] / px[2].max(1.0), pv[1] / px[3].max(1.0)];
                });
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            m.atlas.remove(i);
        }
        ui.horizontal(|ui| {
            if m.atlas.len() < crate::model::MAX_ATLAS_CELLS && ui.button("+ Picture").clicked() {
                m.atlas.push(crate::model::AtlasCell::default());
            }
            if m.atlas.is_empty() && ui.button("Split into a grid…").on_hover_text("Rectangles for a 2 × 2 grid of pictures; adjust them after").clicked() {
                for k in 0..4 {
                    let (cx, cy) = ((k % 2) as f32 * 0.5, (k / 2) as f32 * 0.5);
                    m.atlas.push(crate::model::AtlasCell { rect: [cx, cy, 0.5, 0.5], ..Default::default() });
                }
            }
        });
    });
}

/// More materials on a generator's stems, blended along a gradient.
fn material_blend_ui(ui: &mut Ui, g: &mut Generator, mats: &[(u32, String)]) {
    let n = g.blend.as_ref().map(|b| b.layers.len()).unwrap_or(0);
    let title = if n == 0 { "Material gradient".to_string() } else { format!("Material gradient ({} layers)", n + 1) };
    egui::CollapsingHeader::new(title).id_salt("material_gradient").default_open(n > 0).show(ui, |ui| {
        let Some(b) = g.blend.as_mut() else {
            ui.label(RichText::new("Blend more materials along the stems, e.g. dark cracked bark at the foot and smooth bark higher up.").small().color(Color32::GRAY));
            if ui.button("+ Add a layer").clicked() {
                g.blend = Some(crate::model::MaterialBlend { layers: vec![crate::model::MaterialLayer { material: None, at: 0.3 }], ..Default::default() });
            }
            return;
        };
        let name_of = |m: Option<u32>| m.and_then(|m| mats.iter().find(|x| x.0 == m)).map(|x| x.1.clone()).unwrap_or_else(|| "Default bark".to_string());
        egui::Grid::new("blend_grid").num_columns(2).show(ui, |ui| {
            ui.label("Along");
            let before = b.axis;
            egui::ComboBox::from_id_salt("blend_axis").selected_text(b.axis.label()).show_ui(ui, |ui| {
                for ax in [crate::model::BlendAxis::Stem, crate::model::BlendAxis::Height, crate::model::BlendAxis::HeightShare] {
                    ui.selectable_value(&mut b.axis, ax, ax.label());
                }
            })
            .response
            .on_hover_text("Along each stem (0 = base, 1 = tip), by height above the ground in metres (the same on every generator, so trunk and roots can share a band) or by share of the tree's height");
            if b.axis != before {
                // keep the layers where they were, roughly: metres <-> shares of 10 m
                let f = match (before, b.axis) {
                    (crate::model::BlendAxis::Height, _) => 0.1,
                    (_, crate::model::BlendAxis::Height) => 10.0,
                    _ => 1.0,
                };
                for l in &mut b.layers {
                    l.at *= f;
                }
                b.width *= f;
            }
            ui.end_row();
            ui.label("Layer 1");
            ui.label(RichText::new("the generator's material, from the start").small().color(Color32::GRAY));
            ui.end_row();
            let max = b.axis.max();
            let unit = if b.axis == crate::model::BlendAxis::Height { " m" } else { "" };
            let mut remove = None;
            for (i, l) in b.layers.iter_mut().enumerate() {
                ui.label(format!("Layer {}", i + 2));
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("blend_mat", i)).width(110.0).selected_text(name_of(l.material)).show_ui(ui, |ui| {
                        ui.selectable_value(&mut l.material, None, "Default bark");
                        for (mid, name) in mats {
                            ui.selectable_value(&mut l.material, Some(*mid), name);
                        }
                    });
                    ui.label("from");
                    let range = if b.axis == crate::model::BlendAxis::Height { 0.0..=30.0 } else { 0.0..=max };
                    ui.add(egui::DragValue::new(&mut l.at).range(range).speed(0.01).suffix(unit));
                    if ui.small_button("🗑").on_hover_text("Remove this layer").clicked() {
                        remove = Some(i);
                    }
                });
                ui.end_row();
            }
            if let Some(i) = remove {
                b.layers.remove(i);
            }
            let wmax = if b.axis == crate::model::BlendAxis::Height { 5.0 } else { 1.0 };
            ui.label("Blend width");
            ui.add(egui::Slider::new(&mut b.width, 0.0..=wmax).suffix(unit)).on_hover_text("Width of each transition");
            ui.end_row();
            ui.label("Noise");
            ui.add(egui::Slider::new(&mut b.noise, 0.0..=1.0)).on_hover_text("Breaks the transition up: how far the noise moves it, as a share of the width");
            ui.end_row();
            ui.label("Noise size");
            ui.add(egui::Slider::new(&mut b.noise_scale, 0.02..=3.0).suffix(" m").logarithmic(true)).on_hover_text("Size of the noise patches");
            ui.end_row();
            ui.label("Contrast");
            ui.add(egui::Slider::new(&mut b.contrast, 0.0..=1.0)).on_hover_text("0 = a soft blend over the whole width, 1 = a sharp, ragged edge");
            ui.end_row();
        });
        ui.horizontal(|ui| {
            if b.layers.len() < crate::model::MAX_LAYERS && ui.button("+ Layer").clicked() {
                let at = b.layers.last().map(|l| (l.at + 0.2 * b.axis.max().min(5.0)).min(b.axis.max())).unwrap_or(0.3);
                b.layers.push(crate::model::MaterialLayer { material: None, at });
            }
        });
        b.layers.sort_by(|x, y| x.at.total_cmp(&y.at));
        if b.layers.is_empty() {
            g.blend = None;
        }
    });
}

/// The material library list in the left panel.
fn materials_ui(ui: &mut Ui, sh: &mut Shared, textures: &HashMap<u32, egui::TextureId>, editing: &mut Option<u32>) {
    egui::Frame::new().fill(PANEL_DARK).corner_radius(4).inner_margin(6).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.color_edit_button_srgb(&mut sh.doc.settings.bark_color).on_hover_text("Colour of stems without a material");
            ui.label("Default bark");
        });
        ui.horizontal(|ui| {
            ui.color_edit_button_srgb(&mut sh.doc.settings.leaf_color).on_hover_text("Colour of leaves without a material");
            ui.label("Default leaves");
        });
        let mats: Vec<(u32, String, [u8; 3], usize)> = sh.doc.materials.iter().map(|m| (m.id, m.name.clone(), m.color, sh.doc.material_users(m.id).len())).collect();
        for (id, name, color, users) in mats {
            ui.horizontal(|ui| {
                swatch(ui, color, textures.get(&id).copied());
                let r = ui.selectable_label(*editing == Some(id), format!("{name}  ({users})"));
                if r.clicked() {
                    *editing = Some(id);
                }
                r.on_hover_text("Click to edit. The number is how many generators use it.");
            });
        }
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let sel = sh.selected.and_then(|id| sh.doc.get(id)).map(|g| (g.id, g.kind, g.name.clone()));
        let tip = match &sel {
            Some((_, _, n)) => format!("Create a material and use it on \"{n}\""),
            None => "Create a material".to_string(),
        };
        if ui.button("+ Material").on_hover_text(tip).clicked() {
            let leafy = matches!(sel, Some((_, GenKind::Leaves, _)));
            let s = &sh.doc.settings;
            let m = Material {
                name: if leafy { "Leaf material".into() } else { "Bark material".into() },
                color: if leafy { s.leaf_color } else { s.bark_color },
                roughness: if leafy { 0.75 } else { 0.95 },
                double_sided: leafy,
                ..Default::default()
            };
            let id = sh.doc.add_material(m);
            if let Some((gid, _, _)) = sel {
                if let Some(g) = sh.doc.get_mut(gid) {
                    g.material = Some(id);
                }
            }
            *editing = Some(id);
        }
    });
}

/// Floating editor for one material. Returns false when it should close.
fn material_editor(
    ctx: &egui::Context,
    sh: &mut Shared,
    mid: u32,
    textures: &HashMap<u32, egui::TextureId>,
    cache: &HashMap<String, Option<egui::TextureHandle>>,
    actions: &mut Vec<Action>,
) -> bool {
    let Some(orig) = sh.doc.material(mid).cloned() else { return false };
    let mut m = orig.clone();
    let mut open = true;
    let mut delete = false;
    let mut assign: Option<u32> = None;
    let mut to_cards = false;
    let users: Vec<(u32, String, GenKind, LeafShape)> = sh.doc.material_users(mid).iter().map(|g| (g.id, g.name.clone(), g.kind, g.params.leaf_shape)).collect();
    let selected = sh.selected.and_then(|id| sh.doc.get(id)).map(|g| (g.id, g.name.clone(), g.material));
    egui::Window::new("Material").id(egui::Id::new("material_editor")).open(&mut open).collapsible(false).resizable(false).default_width(360.0).show(ctx, |ui| {
        egui::Grid::new("mat_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(200.0));
            ui.end_row();
            ui.label("Colour / tint").on_hover_text("With a texture this tints it: white shows the texture unchanged.");
            ui.horizontal(|ui| {
                ui.color_edit_button_srgb(&mut m.color);
                if m.texture.is_some() && ui.small_button("White").on_hover_text("Show the texture unchanged").clicked() {
                    m.color = [255, 255, 255];
                }
            });
            ui.end_row();
            ui.label("Texture");
            ui.vertical(|ui| {
                match &m.texture {
                    Some(p) => {
                        let fname = std::path::Path::new(p).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                        ui.label(RichText::new(fname).monospace()).on_hover_text(p);
                        match textures.get(&mid) {
                            Some(t) => {
                                ui.add(egui::Image::new((*t, egui::vec2(96.0, 96.0))));
                            }
                            None if matches!(cache.get(p), Some(None)) => {
                                ui.colored_label(Color32::from_rgb(230, 130, 100), "Cannot read this image");
                            }
                            None => {}
                        }
                    }
                    None => {
                        ui.label(RichText::new("none (plain colour)").color(HEADER));
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Choose…").clicked() {
                        actions.push(Action::PickTexture(mid, false));
                    }
                    if m.texture.is_some() && ui.button("Clear").clicked() {
                        m.texture = None;
                    }
                });
            });
            ui.end_row();
            ui.label("Normal map").on_hover_text("Surface detail for the lighting: shown in the viewport and exported to engines.");
            ui.horizontal(|ui| {
                let label = m.normal_map.as_ref().and_then(|p| std::path::Path::new(p).file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "none".into());
                ui.label(RichText::new(label).monospace());
                if ui.button("Choose…").clicked() {
                    actions.push(Action::PickTexture(mid, true));
                }
                if m.normal_map.is_some() && ui.button("Clear").clicked() {
                    m.normal_map = None;
                }
            });
            ui.end_row();
            if m.normal_map.is_some() {
                ui.label("Normal strength");
                ui.add(egui::Slider::new(&mut m.normal_strength, 0.0..=3.0));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut m.normal_flip_green, "DirectX style (flip green)")
                    .on_hover_text("Tick if the bumps look lit from the wrong side: the map stores green pointing down (DirectX) instead of up (OpenGL, glTF).");
                ui.end_row();
            }
            ui.label("Roughness");
            ui.add(egui::Slider::new(&mut m.roughness, 0.0..=1.0));
            ui.end_row();
            ui.label("Tiling").on_hover_text("How often a bark texture repeats around each stem, and along it per circumference: equal values keep a square image square. A gradient layer shows its own tiling.");
            ui.horizontal(|ui| {
                ui.label("around");
                ui.add(egui::DragValue::new(&mut m.tiling[0]).speed(0.05).range(0.05..=50.0));
                ui.label("along");
                if m.keep_proportions && m.effective_tiling() != m.tiling {
                    // worked out from the image's shape
                    let along = m.effective_tiling()[1];
                    ui.add_enabled(false, egui::DragValue::new(&mut along.clone()).speed(0.05));
                } else {
                    ui.add(egui::DragValue::new(&mut m.tiling[1]).speed(0.05).range(0.05..=50.0));
                }
            });
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut m.keep_proportions, "Keep the image's proportions")
                .on_hover_text("Work out the along repeats from the image's shape, so a tall bark photo is not squashed.");
            ui.end_row();
            ui.label("Cut-out");
            ui.checkbox(&mut m.alpha_cutout, "Transparent pixels are holes (leaf cards)");
            ui.end_row();
            ui.label("Two-sided");
            ui.checkbox(&mut m.double_sided, "Render both faces in engines");
            ui.end_row();
        });
        if m.texture.is_some() {
            atlas_ui(ui, &mut m, textures.get(&mid).copied());
        }
        ui.separator();
        if users.is_empty() {
            ui.label(RichText::new("Not used by any generator yet.").color(HEADER));
        } else {
            ui.label(format!("Used by: {}", users.iter().map(|u| u.1.as_str()).collect::<Vec<_>>().join(", ")));
        }
        let non_card_leaves = users.iter().any(|u| u.2 == GenKind::Leaves && u.3 != LeafShape::Card);
        if m.texture.is_some() && m.alpha_cutout && non_card_leaves {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Leaf textures look best on rectangular cards.").color(HEADER));
                if ui.small_button("Switch these leaves to Card").clicked() {
                    to_cards = true;
                }
            });
        }
        ui.horizontal(|ui| {
            if let Some((gid, gname, gmat)) = &selected {
                if *gmat != Some(mid) && ui.button(format!("Use on \"{gname}\"")).clicked() {
                    assign = Some(*gid);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete material").clicked() {
                    delete = true;
                }
            });
        });
    });
    m.clamp();
    if m != orig {
        if let Some(t) = sh.doc.material_mut(mid) {
            *t = m;
        }
    }
    if let Some(gid) = assign {
        if let Some(g) = sh.doc.get_mut(gid) {
            g.material = Some(mid);
        }
    }
    if to_cards {
        for (gid, _, kind, _) in &users {
            if *kind == GenKind::Leaves {
                if let Some(g) = sh.doc.get_mut(*gid) {
                    g.params.leaf_shape = LeafShape::Card;
                    g.params.leaf_aspect = g.params.leaf_aspect.max(0.6);
                }
            }
        }
    }
    if delete {
        sh.doc.remove_material(mid);
        return false;
    }
    open
}

fn xyz(ui: &mut Ui, v: &mut [f32; 3], speed: f64) {
    ui.horizontal(|ui| {
        for (k, label) in ["x", "y", "z"].iter().enumerate() {
            ui.add(egui::DragValue::new(&mut v[k]).speed(speed).prefix(format!("{label} ")).max_decimals(2));
        }
    });
}

/// Forces bend growth: direction pushes and attractor points.
fn forces_ui(ui: &mut Ui, sh: &mut Shared) {
    egui::CollapsingHeader::new(format!("Forces ({})", sh.doc.forces.len())).default_open(false).open(force_open("Forces")).show(ui, |ui| {
        let mut remove = None;
        for f in &mut sh.doc.forces {
            egui::Frame::new().fill(PANEL_DARK).corner_radius(4).inner_margin(6).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.checkbox(&mut f.enabled, "");
                    ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(120.0));
                    egui::ComboBox::from_id_salt(("force_kind", f.id)).width(90.0).selected_text(f.kind.label()).show_ui(ui, |ui| {
                        ui.selectable_value(&mut f.kind, ForceKind::Direction, "Direction");
                        ui.selectable_value(&mut f.kind, ForceKind::Attractor, "Attractor");
                    });
                    if ui.small_button("🗑").on_hover_text("Delete force").clicked() {
                        remove = Some(f.id);
                    }
                });
                egui::Grid::new(("force", f.id)).num_columns(2).show(ui, |ui| {
                    ui.label("Strength").on_hover_text("Negative values reverse the force (attractors then push away).");
                    ui.add(egui::Slider::new(&mut f.strength, -2.0..=2.0));
                    ui.end_row();
                    match f.kind {
                        ForceKind::Direction => {
                            ui.label("Direction");
                            xyz(ui, &mut f.direction, 0.02);
                        }
                        ForceKind::Attractor => {
                            ui.label("Position (m)");
                            xyz(ui, &mut f.position, 0.1);
                        }
                    }
                    ui.end_row();
                });
            });
            f.clamp();
        }
        if let Some(id) = remove {
            sh.doc.remove_force(id);
        }
        ui.horizontal(|ui| {
            if ui.button("+ Direction").on_hover_text("Push all growth one way (wind-swept, leaning towards light)").clicked() {
                sh.doc.add_force(Force { kind: ForceKind::Direction, direction: [1.0, 0.2, 0.0], strength: 0.5, ..Default::default() });
            }
            if ui.button("+ Attractor").on_hover_text("Pull growth towards a point; negative strength pushes away").clicked() {
                let h = sh.doc.generators.iter().find(|g| g.kind == GenKind::Trunk).map(|g| g.params.length * sh.doc.settings.scale).unwrap_or(8.0);
                sh.doc.add_force(Force { kind: ForceKind::Attractor, position: [h * 0.5, h * 0.9, 0.0], strength: 0.6, ..Default::default() });
            }
        });
        ui.label(RichText::new("Each generator's Force response scales how much it bends; roots ignore forces by default.").size(11.0).color(HEADER));
    });
}

/// Pruning: trim branches and leaves to a box or ellipsoid.
fn pruning_ui(ui: &mut Ui, sh: &mut Shared) {
    egui::CollapsingHeader::new("Pruning").default_open(false).open(force_open("Pruning")).show(ui, |ui| {
        let mut fit = false;
        {
            let pr = &mut sh.doc.pruning;
            ui.checkbox(&mut pr.enabled, "Trim the crown to a volume");
            egui::Grid::new("pruning").num_columns(2).show(ui, |ui| {
                ui.label("Shape");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut pr.shape, PruneShape::Ellipsoid, "Ellipsoid");
                    ui.radio_value(&mut pr.shape, PruneShape::Box, "Box");
                });
                ui.end_row();
                ui.label("Centre (m)");
                xyz(ui, &mut pr.center, 0.05);
                ui.end_row();
                ui.label("Half-size (m)");
                xyz(ui, &mut pr.size, 0.05);
                ui.end_row();
            });
            if ui.button("Fit to tree").on_hover_text("Place the volume just inside the current crown").clicked() {
                fit = true;
            }
            pr.clamp();
        }
        if fit {
            let mut free = sh.doc.clone();
            free.pruning.enabled = false;
            let m = build_mesh(&free, &grow(&free, 0), 0);
            sh.doc.pruning.fit(m.height, m.radius);
            sh.doc.pruning.enabled = true;
        }
        ui.label(RichText::new("Branches stop at the surface; trunks are only cut at the top; roots are never pruned.").size(11.0).color(HEADER));
    });
}

fn collision_ui(ui: &mut Ui, sh: &mut Shared) {
    egui::CollapsingHeader::new("Collision").default_open(false).open(force_open("Collision")).show(ui, |ui| {
        let c = &mut sh.doc.collision;
        ui.checkbox(&mut c.enabled, "Export collision objects");
        egui::Grid::new("collision").num_columns(2).show(ui, |ui| {
            ui.label("Levels");
            egui::ComboBox::from_id_salt("coll_depth")
                .selected_text(match c.max_depth {
                    0 => "Trunk only".to_string(),
                    1 => "Trunk + main branches".to_string(),
                    d => format!("{} levels", d + 1),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut c.max_depth, 0, "Trunk only");
                    ui.selectable_value(&mut c.max_depth, 1, "Trunk + main branches");
                    ui.selectable_value(&mut c.max_depth, 2, "3 levels");
                });
            ui.end_row();
            ui.label("Thinnest branch");
            ui.add(egui::Slider::new(&mut c.min_radius, 0.0..=0.5).suffix(" m"));
            ui.end_row();
            ui.label("Capsules per branch");
            ui.add(egui::Slider::new(&mut c.segments, 1..=12));
            ui.end_row();
            ui.label("Size");
            ui.add(egui::Slider::new(&mut c.radius_scale, 0.2..=3.0).suffix("×"));
            ui.end_row();
            ui.label("Naming");
            egui::ComboBox::from_id_salt("coll_naming")
                .selected_text(match c.naming {
                    CollisionNaming::Neutral => "Neutral",
                    CollisionNaming::Unreal => "Unreal (UCX_)",
                    CollisionNaming::Godot => "Godot (-convcolonly)",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut c.naming, CollisionNaming::Neutral, "Neutral");
                    ui.selectable_value(&mut c.naming, CollisionNaming::Unreal, "Unreal (UCX_)");
                    ui.selectable_value(&mut c.naming, CollisionNaming::Godot, "Godot (-convcolonly)");
                });
            ui.end_row();
        });
        c.clamp();
        ui.checkbox(&mut sh.view.show_collision, "Show in the view");
    });
}

fn curve_label(key: &str) -> &'static str {
    match key {
        "length" => "Length along parent",
        "angle" => "Angle along parent",
        "radius" => "Thickness along parent",
        "density" => "Density along parent",
        "leaf_size" => "Leaf size along parent",
        "radius_profile" => "Thickness along the stem",
        "taper" => "Taper strength",
        "gravity" => "Gravity along the stem",
        "wiggle" => "Wiggle along the stem",
        "jitter" => "Jitter along the stem",
        "force_response" => "Force response",
        "flute_depth" => "Flute depth",
        "flatten" => "Flatten",
        "bumps" => "Bark lumps",
        "angle_variation" => "Angle variation",
        "length_variation" => "Length variation",
        "kink" => "Kink",
        "break_chance" => "Broken branches",
        "leaf_fold" => "Leaf fold",
        "leaf_bend" => "Leaf bend",
        "leaf_face_up" => "Face the sky",
        _ => "Curve",
    }
}

/// What a curve button needs to know about its generator.
#[derive(Clone, Copy)]
struct CurveCx {
    gid: u32,
    kind: GenKind,
    taper: f32,
    profile: Profile,
}

/// A property row with a curve button after the widget (for settings that can follow a curve).
fn crow(ui: &mut Ui, label: &str, field: &str, keys: &[&str], curves: &mut std::collections::BTreeMap<String, Curve>, cx: CurveCx, add: impl FnOnce(&mut Ui) -> egui::Response) {
    ui.label(label).on_hover_text(help_for(field));
    add(ui).on_hover_text(help_for(field));
    let keys: Vec<&str> = keys.iter().copied().filter(|k| curve_applies(k, cx.kind)).collect();
    if !keys.is_empty() {
        curve_button(ui, field, &keys, curves, cx);
    }
    ui.end_row();
}

/// Small curve icon; highlighted when a curve is on. Clicking opens the curve editor popup.
fn curve_button(ui: &mut Ui, field: &str, keys: &[&str], curves: &mut std::collections::BTreeMap<String, Curve>, cx: CurveCx) {
    let on = keys.iter().any(|k| curves.contains_key(*k));
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 18.0), egui::Sense::click());
    let popup_id = egui::Id::new(("curve_popup", cx.gid, field));
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let painter = ui.painter();
    let bg = if on {
        ACCENT.gamma_multiply(0.28)
    } else if resp.hovered() || open {
        Color32::from_gray(66)
    } else {
        Color32::from_gray(46)
    };
    painter.rect_filled(rect, 3, bg);
    if open {
        painter.rect_stroke(rect, 3, egui::Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside);
    }
    // the glyph: an eased S-curve from bottom-left to top-right
    let col = if on { ACCENT } else if resp.hovered() { Color32::from_gray(225) } else { Color32::from_gray(165) };
    let r = rect.shrink2(egui::vec2(5.0, 4.0));
    let pts: Vec<egui::Pos2> = (0..=14)
        .map(|i| {
            let f = i as f32 / 14.0;
            let y = f * f * (3.0 - 2.0 * f);
            egui::pos2(r.left() + f * r.width(), r.bottom() - y * r.height())
        })
        .collect();
    painter.add(egui::Shape::line(pts, egui::Stroke::new(1.6, col)));
    if on {
        painter.circle_filled(egui::pos2(rect.right() - 3.0, rect.top() + 3.0), 2.0, ACCENT);
    }
    let names: Vec<&str> = keys.iter().map(|k| curve_label(k)).collect();
    let resp = resp.on_hover_text(if on { format!("Curve on: {}. Click to edit.", names.join(", ")) } else { format!("Shape this setting with a curve ({}). Click to open.", names.join(", ")) });
    egui::Popup::from_toggle_button_response(&resp)
        .id(popup_id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(300.0)
        .show(|ui| {
            ui.set_min_width(300.0);
            curve_popup(ui, keys, curves, cx);
        });
}

/// Contents of a curve popup: per curve, switch it on, choose what x measures, edit the points.
fn curve_popup(ui: &mut Ui, keys: &[&str], curves: &mut std::collections::BTreeMap<String, Curve>, cx: CurveCx) {
    for (i, key) in keys.iter().enumerate() {
        let Some((_, along, _, meaning)) = CURVE_REFERENCE.iter().find(|c| c.0 == *key) else {
            continue;
        };
        if i > 0 {
            ui.separator();
        }
        let mut on = curves.contains_key(*key);
        ui.horizontal(|ui| {
            if ui.checkbox(&mut on, RichText::new(curve_label(key)).strong()).changed() {
                if on {
                    let init = match *key {
                        "length" => {
                            let prof = cx.profile;
                            Curve::from_fn(|x| prof.eval(x), 6)
                        }
                        "radius_profile" => {
                            let tip = cx.taper;
                            if cx.kind == GenKind::Roots {
                                Curve::from_fn(|x| tip + (1.0 - tip) * (1.0 - x).powf(1.5), 5)
                            } else {
                                Curve { points: vec![[0.0, 1.0], [1.0, tip]], along: CurveAlong::Natural }
                            }
                        }
                        _ => Curve::flat(),
                    };
                    curves.insert(key.to_string(), init);
                } else {
                    curves.remove(*key);
                }
            }
        });
        ui.label(RichText::new(*meaning).size(11.0).color(HEADER));
        if let Some(c) = curves.get_mut(*key) {
            ui.horizontal(|ui| {
                ui.label(RichText::new("x:").size(11.0).color(HEADER));
                let natural = if *along == "self" { "Base to tip of each stem" } else { "Along the parent" };
                ui.selectable_value(&mut c.along, CurveAlong::Natural, RichText::new(natural).size(11.0));
                ui.selectable_value(&mut c.along, CurveAlong::TreeHeight, RichText::new("Tree height").size(11.0))
                    .on_hover_text("x = height above the ground: 0 at the ground, 1 at the top of the tree, the same for every branch level");
            });
            curve_editor(ui, egui::Id::new(("curve", cx.gid, *key)), c);
            ui.horizontal_wrapped(|ui| {
                for (pr, label) in [("flat", "Flat"), ("ramp_up", "Up"), ("ramp_down", "Down"), ("bell", "Bell"), ("valley", "Valley"), ("ease_out", "Ease")] {
                    if ui.small_button(label).clicked() {
                        let along = c.along;
                        *c = Curve::preset(pr).unwrap();
                        c.along = along;
                    }
                }
            });
            ui.label(RichText::new("The setting is multiplied by the curve (0–1). Drag points · double-click to add · right-click to remove.").size(10.5).color(HEADER));
        }
    }
}

/// A small editable curve: drag points, double-click to add, right-click to remove.
fn curve_editor(ui: &mut Ui, id: egui::Id, c: &mut Curve) -> bool {
    let w = ui.available_width().clamp(160.0, 280.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 92.0), egui::Sense::click_and_drag());
    let rect = rect.shrink(4.0);
    let painter = ui.painter_at(rect.expand(4.0));
    painter.rect_filled(rect.expand(4.0), 4, PANEL_DARK);
    for i in 1..4 {
        let f = i as f32 / 4.0;
        let x = rect.left() + rect.width() * f;
        let y = rect.bottom() - rect.height() * f;
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], egui::Stroke::new(1.0, Color32::from_gray(52)));
        painter.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], egui::Stroke::new(1.0, Color32::from_gray(52)));
    }
    let to_screen = |p: [f32; 2]| egui::pos2(rect.left() + p[0] * rect.width(), rect.bottom() - p[1] * rect.height());
    let from_screen = |q: egui::Pos2| [((q.x - rect.left()) / rect.width()).clamp(0.0, 1.0), ((rect.bottom() - q.y) / rect.height()).clamp(0.0, 1.0)];
    let nearest = |c: &Curve, q: egui::Pos2| -> Option<usize> {
        c.points.iter().enumerate().map(|(i, p)| (i, to_screen(*p).distance(q))).filter(|(_, d)| *d < 10.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
    };
    let drag_key = id.with("drag");
    let mut changed = false;
    if resp.drag_started() {
        if let Some(q) = resp.interact_pointer_pos() {
            match nearest(c, q) {
                Some(i) => {
                    ui.data_mut(|d| d.insert_temp(drag_key, i));
                }
                None => {
                    ui.data_mut(|d| d.remove::<usize>(drag_key));
                }
            }
        }
    }
    if resp.dragged() {
        if let (Some(i), Some(q)) = (ui.data(|d| d.get_temp::<usize>(drag_key)), resp.interact_pointer_pos()) {
            if i < c.points.len() {
                let mut np = from_screen(q);
                let last = c.points.len() - 1;
                if i == 0 {
                    np[0] = 0.0;
                } else if i == last {
                    np[0] = 1.0;
                } else {
                    np[0] = np[0].clamp(c.points[i - 1][0] + 0.01, c.points[i + 1][0] - 0.01);
                }
                c.points[i] = np;
                changed = true;
            }
        }
    }
    if resp.drag_stopped() {
        ui.data_mut(|d| d.remove::<usize>(drag_key));
    }
    if resp.double_clicked() {
        if let Some(q) = resp.interact_pointer_pos() {
            if nearest(c, q).is_none() && c.points.len() < 16 {
                c.points.push(from_screen(q));
                c.clamp();
                changed = true;
            }
        }
    }
    if resp.secondary_clicked() {
        if let Some(q) = resp.interact_pointer_pos() {
            if let Some(i) = nearest(c, q) {
                if i != 0 && i != c.points.len() - 1 {
                    c.points.remove(i);
                    changed = true;
                }
            }
        }
    }
    let line: Vec<egui::Pos2> = (0..=64).map(|i| {
        let x = i as f32 / 64.0;
        to_screen([x, c.eval(x)])
    }).collect();
    painter.add(egui::Shape::line(line, egui::Stroke::new(2.0, ACCENT)));
    for p in &c.points {
        painter.circle_filled(to_screen(*p), 4.0, Color32::from_gray(235));
        painter.circle_stroke(to_screen(*p), 4.0, egui::Stroke::new(1.0, Color32::from_gray(40)));
    }
    resp.on_hover_text("Drag points · double-click to add a point · right-click a point to remove it");
    changed
}

/// Hand-editing of one selected branch instance (node editing).
fn node_ui(ui: &mut Ui, sh: &mut Shared, sk: &Skeleton) {
    section_title(ui, "BRANCH (NODE EDITING)");
    let edited = sh.doc.nodes.len();
    let deleted = sh.doc.nodes.iter().filter(|n| n.hidden).count();
    egui::Frame::new().fill(PANEL_DARK).corner_radius(4).inner_margin(6).show(ui, |ui| {
        ui.set_width(ui.available_width());
        match sh.selected_node {
            None => {
                ui.label(RichText::new("Click a branch in the view to hand-edit just that one.").italics().color(HEADER));
            }
            Some(key) => {
                let info = sk.stems.iter().find(|s| s.key == key).map(|s| (s.gen, s.length, s.depth));
                let gen_name = info.and_then(|(g, _, _)| sh.doc.get(g)).map(|g| g.name.clone()).unwrap_or_else(|| "deleted branch".into());
                ui.label(match info {
                    Some((_, len, depth)) => format!("{gen_name} · {len:.2} m long · level {depth} · #{key}"),
                    None => format!("{gen_name} · #{key}"),
                });
                let mut e = sh.doc.node(key).cloned().unwrap_or(NodeEdit { key, ..Default::default() });
                let before = e.clone();
                egui::Grid::new("node_edit").num_columns(2).show(ui, |ui| {
                    ui.label("Length");
                    ui.add(egui::Slider::new(&mut e.length, 0.05..=3.0).suffix("×"));
                    ui.end_row();
                    ui.label("Thickness");
                    ui.add(egui::Slider::new(&mut e.radius, 0.1..=3.0).suffix("×"));
                    ui.end_row();
                    ui.label("Angle");
                    ui.add(egui::Slider::new(&mut e.angle, -120.0..=120.0).suffix("°"));
                    ui.end_row();
                    ui.label("Rotate around parent");
                    ui.add(egui::Slider::new(&mut e.rotate, -180.0..=180.0).suffix("°"));
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    if ui.button(if e.hidden { "Restore branch" } else { "Delete branch" }).on_hover_text("Removes this branch and everything growing on it").clicked() {
                        e.hidden = !e.hidden;
                    }
                    if ui.button("Reset").clicked() {
                        e = NodeEdit { key, ..Default::default() };
                    }
                });
                if e != before {
                    *sh.doc.node_mut(key) = e;
                    sh.doc.tidy_nodes();
                }
            }
        }
        if edited > 0 {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{edited} edited branch(es), {deleted} deleted")).size(11.0).color(HEADER));
                if ui.small_button("Clear all").clicked() {
                    sh.doc.nodes.clear();
                }
            });
        }
        ui.label(RichText::new("Edits stay with the branch while the counts above it stay the same.").size(11.0).color(HEADER));
    });
    ui.add_space(6.0);
}
