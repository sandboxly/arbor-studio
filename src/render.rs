//! Viewport rendering. The tree is projected on the CPU and drawn as one depth-sorted
//! egui mesh; the same projection drives an offscreen z-buffer renderer for MCP previews.

use crate::grow::{wind_offset, Mesh, Skeleton};
use crate::model::Document;
use egui::{epaint::Vertex, Color32, Pos2, Rect, Shape};
use glam::{Mat4, Vec3, Vec4};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Camera {
    /// Degrees around the vertical axis.
    pub yaw: f32,
    /// Degrees above the horizon.
    pub pitch: f32,
    /// Distance from the target in metres.
    pub distance: f32,
    pub target: [f32; 3],
    pub fov: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { yaw: 35.0, pitch: 12.0, distance: 22.0, target: [0.0, 4.5, 0.0], fov: 40.0 }
    }
}

impl Camera {
    pub fn eye(&self) -> Vec3 {
        let (y, p) = (self.yaw.to_radians(), self.pitch.to_radians());
        Vec3::from(self.target) + Vec3::new(p.cos() * y.sin(), p.sin(), p.cos() * y.cos()) * self.distance
    }
    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), Vec3::from(self.target), Vec3::Y)
    }
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(self.fov.to_radians(), aspect.max(0.05), 0.05, 2000.0) * self.view()
    }
    /// Frame a tree of the given height / crown radius.
    pub fn frame(&mut self, height: f32, radius: f32) {
        let h = height.max(0.3);
        self.target = [0.0, h * 0.48, 0.0];
        let ext = (h * 0.62).max(radius * 1.15);
        self.distance = ext / (self.fov.to_radians() * 0.5).tan() * 1.12;
    }
    pub fn clamp(&mut self) {
        self.pitch = self.pitch.clamp(-89.0, 89.0);
        self.yaw = self.yaw.rem_euclid(360.0);
        self.distance = self.distance.clamp(0.2, 1500.0);
        self.fov = self.fov.clamp(10.0, 100.0);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum RenderMode {
    Shaded,
    Wireframe,
    Skeleton,
}

impl RenderMode {
    pub fn label(self) -> &'static str {
        match self {
            RenderMode::Shaded => "Shaded",
            RenderMode::Wireframe => "Wireframe",
            RenderMode::Skeleton => "Skeleton",
        }
    }
}

/// Viewport lighting presets.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Lighting {
    /// Colours and textures as they are, no shading.
    Unlit,
    Morning,
    #[default]
    Noon,
    Evening,
    Night,
}

/// What a lighting preset means for the shading (all in display/gamma space).
#[derive(Clone, Copy)]
pub struct LightParams {
    /// Direction towards the sun (normalised).
    pub dir: Vec3,
    pub sun: [f32; 3],
    pub ambient: [f32; 3],
    /// Extra light on surfaces facing the sky (× normal.y).
    pub sky: [f32; 3],
    /// Background gradient: top, horizon, bottom.
    pub bg: [[u8; 3]; 3],
    pub unlit: bool,
}

impl Lighting {
    pub const ALL: [Lighting; 5] = [Lighting::Unlit, Lighting::Morning, Lighting::Noon, Lighting::Evening, Lighting::Night];
    pub fn label(self) -> &'static str {
        match self {
            Lighting::Unlit => "Unlit",
            Lighting::Morning => "Morning",
            Lighting::Noon => "Noon",
            Lighting::Evening => "Evening",
            Lighting::Night => "Night",
        }
    }
    pub fn parse(s: &str) -> Option<Lighting> {
        Lighting::ALL.into_iter().find(|l| l.label().eq_ignore_ascii_case(s.trim()))
    }
    pub fn params(self) -> LightParams {
        let noon_bg = [[58, 66, 78], [112, 122, 134], [70, 72, 74]];
        match self {
            Lighting::Unlit => LightParams { dir: LIGHT.normalize(), sun: [0.0; 3], ambient: [1.0; 3], sky: [0.0; 3], bg: noon_bg, unlit: true },
            Lighting::Noon => LightParams { dir: LIGHT.normalize(), sun: [0.78; 3], ambient: [0.36; 3], sky: [0.14; 3], bg: noon_bg, unlit: false },
            // low warm sun from the east, cool shadows
            Lighting::Morning => LightParams {
                dir: Vec3::new(0.82, 0.36, 0.44).normalize(),
                sun: [0.98, 0.80, 0.60],
                ambient: [0.30, 0.32, 0.38],
                sky: [0.12, 0.14, 0.19],
                bg: [[92, 110, 140], [214, 178, 150], [78, 74, 76]],
                unlit: false,
            },
            // low orange sun from the west, purple shadows
            Lighting::Evening => LightParams {
                dir: Vec3::new(-0.84, 0.26, 0.46).normalize(),
                sun: [1.0, 0.62, 0.34],
                ambient: [0.27, 0.23, 0.29],
                sky: [0.11, 0.09, 0.15],
                bg: [[52, 46, 82], [222, 132, 84], [62, 52, 56]],
                unlit: false,
            },
            // cold moonlight
            Lighting::Night => LightParams {
                dir: Vec3::new(0.3, 0.75, -0.55).normalize(),
                sun: [0.24, 0.30, 0.44],
                ambient: [0.10, 0.12, 0.18],
                sky: [0.05, 0.07, 0.11],
                bg: [[8, 11, 22], [30, 38, 60], [16, 18, 24]],
                unlit: false,
            },
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct ViewState {
    /// Lighting preset of the viewport and previews.
    pub lighting: Lighting,
    pub camera: Camera,
    pub mode: RenderMode,
    pub wind_playing: bool,
    pub lod: u32,
    pub show_grid: bool,
    pub show_leaves: bool,
    pub highlight_selection: bool,
    /// Draw force arrows / attractor points and the pruning volume.
    pub show_helpers: bool,
    /// Clicks in the viewport pick single branches (node editing) instead of generators.
    pub node_mode: bool,
    /// Draw the collision capsules over the tree.
    pub show_collision: bool,
    /// Growth preview: 0 = nothing above ground, 1 = the mature tree (the tree as designed).
    pub growth: f32,
    /// Play the growth in the viewport.
    pub growth_playing: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            camera: Camera::default(),
            lighting: Lighting::Noon,
            mode: RenderMode::Shaded,
            wind_playing: false,
            lod: 0,
            show_grid: true,
            show_leaves: true,
            highlight_selection: false,
            show_helpers: true,
            node_mode: false,
            show_collision: false,
            growth: 1.0,
            growth_playing: false,
        }
    }
}

const LIGHT: Vec3 = Vec3::new(0.48, 0.78, 0.40);
const HIGHLIGHT: [f32; 3] = [1.0, 0.62, 0.15];

pub fn vertex_color(m: &Mesh, i: usize, selected: Option<u32>, highlight: bool, lp: &LightParams) -> [f32; 3] {
    if m.vmat[i] == crate::billboard::BILLBOARD_MAT {
        // the billboard atlas already contains the lighting
        return m.col[i];
    }
    let base = m.col[i];
    let mut c = if lp.unlit {
        base
    } else {
        let l = lp.dir;
        let n = m.nrm[i];
        let mut c = [0.0; 3];
        for j in 0..3 {
            let k = if m.leaf[i] {
                lp.ambient[j] * 1.17 + lp.sky[j] * 0.7 * n.y.abs() + lp.sun[j] * 0.8 * n.dot(l).abs()
            } else {
                lp.ambient[j] + lp.sky[j] * n.y + lp.sun[j] * n.dot(l).max(0.0)
            };
            c[j] = (base[j] * k).min(1.0);
        }
        c
    };
    if highlight && selected == Some(m.gen[i]) {
        for j in 0..3 {
            c[j] = c[j] * 0.55 + HIGHLIGHT[j] * 0.45;
        }
    }
    c
}

fn c32(c: [f32; 3]) -> Color32 {
    Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8)
}

pub struct Projector {
    vp: Mat4,
    rect: Rect,
}

impl Projector {
    pub fn new(cam: &Camera, rect: Rect) -> Self {
        Projector { vp: cam.view_proj(rect.width() / rect.height().max(1.0)), rect }
    }
    /// Clip-space position (for the GPU path).
    pub fn clip(&self, p: Vec3) -> Vec4 {
        self.vp * Vec4::new(p.x, p.y, p.z, 1.0)
    }
    /// Screen position and view depth, or None when behind the camera.
    pub fn project(&self, p: Vec3) -> Option<(Pos2, f32)> {
        let c = self.vp * Vec4::new(p.x, p.y, p.z, 1.0);
        if c.w < 0.05 {
            return None;
        }
        let nx = c.x / c.w;
        let ny = c.y / c.w;
        Some((
            Pos2::new(self.rect.left() + (nx * 0.5 + 0.5) * self.rect.width(), self.rect.top() + (0.5 - ny * 0.5) * self.rect.height()),
            c.w,
        ))
    }
}

/// What the last frame drew, used for click-picking.
#[derive(Default)]
pub struct PickData {
    screen: Vec<Pos2>,
    order: Vec<u32>,
}

impl PickData {
    /// Generator under the pointer.
    pub fn pick(&self, m: &Mesh, p: Pos2) -> Option<u32> {
        self.pick_vertex(m, p).map(|v| m.gen[v])
    }

    /// A vertex of the front-most triangle under the pointer.
    pub fn pick_vertex(&self, m: &Mesh, p: Pos2) -> Option<usize> {
        if self.screen.len() != m.pos.len() {
            return None;
        }
        for &ti in self.order.iter().rev() {
            let [a, b, c] = m.tris[ti as usize];
            let (a, b, c) = (self.screen[a as usize], self.screen[b as usize], self.screen[c as usize]);
            let d1 = edge(a, b, p);
            let d2 = edge(b, c, p);
            let d3 = edge(c, a, p);
            let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
            let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
            if !(neg && pos) {
                return Some(m.tris[ti as usize][0] as usize);
            }
        }
        None
    }
}

fn edge(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

fn line(mesh: &mut egui::Mesh, a: Pos2, b: Pos2, w: f32, col: Color32) {
    let d = b - a;
    let len = d.length();
    if len < 0.01 {
        return;
    }
    let n = egui::vec2(-d.y, d.x) / len * (w * 0.5);
    let i = mesh.vertices.len() as u32;
    for p in [a + n, a - n, b - n, b + n] {
        mesh.vertices.push(Vertex { pos: p, uv: egui::epaint::WHITE_UV, color: col });
    }
    mesh.indices.extend_from_slice(&[i, i + 1, i + 2, i, i + 2, i + 3]);
}

pub fn background(painter: &egui::Painter, rect: Rect, lp: &LightParams) {
    let mut m = egui::Mesh::default();
    let [top, mid, bot] = lp.bg.map(|c| Color32::from_rgb(c[0], c[1], c[2]));
    let h1 = rect.top() + rect.height() * 0.58;
    let rows = [(rect.top(), top), (h1, mid), (rect.bottom(), bot)];
    for (i, (y, c)) in rows.iter().enumerate() {
        m.vertices.push(Vertex { pos: Pos2::new(rect.left(), *y), uv: egui::epaint::WHITE_UV, color: *c });
        m.vertices.push(Vertex { pos: Pos2::new(rect.right(), *y), uv: egui::epaint::WHITE_UV, color: *c });
        if i > 0 {
            let b = (i as u32 - 1) * 2;
            m.indices.extend_from_slice(&[b, b + 1, b + 2, b + 1, b + 3, b + 2]);
        }
    }
    painter.add(Shape::mesh(m));
}

pub struct DrawArgs<'a> {
    pub doc: &'a Document,
    pub mesh: &'a Mesh,
    pub skeleton: &'a Skeleton,
    pub view: &'a ViewState,
    pub selected: Option<u32>,
    pub time: f32,
    /// Material id -> loaded texture, for materials that have one.
    pub textures: &'a HashMap<u32, egui::TextureId>,
    /// Material id -> (normal map, DirectX style, strength), for the GPU viewport.
    pub normals: &'a HashMap<u32, (egui::TextureId, bool, f32)>,
    /// Skeleton stem index of the hand-selected branch (node editing), highlighted.
    pub selected_stem: Option<u32>,
    /// Depth-buffered GPU drawing for the shaded mode, when OpenGL is available.
    pub gpu: Option<&'a crate::gpu::GpuRenderer>,
}

const NODE_HIGHLIGHT: [f32; 3] = [0.35, 0.8, 1.0];

/// The selection highlights of `draw_color` as one overlay: final = lit × (1 − amount) + colour × amount.
fn highlight_overlay(a: &DrawArgs, i: usize) -> ([f32; 3], f32) {
    let m = a.mesh;
    let mut col = [0.0f32; 3];
    let mut amt = 0.0f32;
    if m.vmat[i] != crate::billboard::BILLBOARD_MAT && a.view.highlight_selection && a.selected == Some(m.gen[i]) {
        col = HIGHLIGHT.map(|x| x * 0.45);
        amt = 0.45;
    }
    if a.selected_stem.is_some() && m.stem.get(i).copied() == a.selected_stem && !m.leaf[i] {
        for k in 0..3 {
            col[k] = col[k] * 0.4 + NODE_HIGHLIGHT[k] * 0.6;
        }
        amt = 1.0 - (1.0 - amt) * 0.4;
    }
    if amt > 0.0 {
        (col.map(|x| x / amt), amt)
    } else {
        ([0.0; 3], 0.0)
    }
}

fn draw_color(a: &DrawArgs, i: usize) -> Color32 {
    let m = a.mesh;
    let mut c = vertex_color(m, i, a.selected, a.view.highlight_selection, &a.view.lighting.params());
    if a.selected_stem.is_some() && m.stem.get(i).copied() == a.selected_stem && !m.leaf[i] {
        for k in 0..3 {
            c[k] = c[k] * 0.4 + NODE_HIGHLIGHT[k] * 0.6;
        }
    }
    c32(c)
}

/// Material id -> (colour image, alpha cut-out) for the offscreen renderer.
pub type OffscreenTextures = HashMap<u32, (std::sync::Arc<image::RgbaImage>, bool)>;

pub fn offscreen_textures(doc: &Document) -> OffscreenTextures {
    doc.materials
        .iter()
        .filter_map(|mat| {
            let p = mat.texture.as_deref()?;
            crate::textures::load(p).ok().map(|img| (mat.id, (img, mat.alpha_cutout)))
        })
        .collect()
}

pub fn draw(painter: &egui::Painter, rect: Rect, a: &DrawArgs) -> PickData {
    let lp = a.view.lighting.params();
    background(painter, rect, &lp);
    let cam = &a.view.camera;
    let pr = Projector::new(cam, rect);
    let m = a.mesh;

    // ground grid
    if a.view.show_grid {
        let mut gm = egui::Mesh::default();
        let spacing = nice_step(m.height.max(1.0) / 6.0);
        let n = 10;
        let ext = spacing * n as f32;
        for i in -n..=n {
            let f = i as f32 * spacing;
            let col = if i == 0 { Color32::from_rgba_unmultiplied(220, 220, 220, 70) } else { Color32::from_rgba_unmultiplied(200, 200, 200, 32) };
            for (p0, p1) in [(Vec3::new(f, 0.0, -ext), Vec3::new(f, 0.0, ext)), (Vec3::new(-ext, 0.0, f), Vec3::new(ext, 0.0, f))] {
                let steps = 16;
                for s in 0..steps {
                    let q0 = p0.lerp(p1, s as f32 / steps as f32);
                    let q1 = p0.lerp(p1, (s + 1) as f32 / steps as f32);
                    if let (Some((s0, _)), Some((s1, _))) = (pr.project(q0), pr.project(q1)) {
                        line(&mut gm, s0, s1, 1.0, col);
                    }
                }
            }
        }
        painter.add(Shape::mesh(gm));
    }

    // soft contact shadow
    {
        let l = lp.dir;
        let off = -Vec3::new(l.x, 0.0, l.z) / l.y.max(0.45) * m.height * 0.25;
        let r = m.radius.max(0.3) * 0.95;
        let mut sm = egui::Mesh::default();
        let c = off;
        if let Some((cp, _)) = pr.project(c) {
            sm.vertices.push(Vertex { pos: cp, uv: egui::epaint::WHITE_UV, color: Color32::from_black_alpha(95) });
            let k = 40;
            let mut ok = true;
            for i in 0..k {
                let ang = i as f32 / k as f32 * std::f32::consts::TAU;
                match pr.project(c + Vec3::new(ang.cos() * r * 1.15, 0.0, ang.sin() * r)) {
                    Some((p, _)) => sm.vertices.push(Vertex { pos: p, uv: egui::epaint::WHITE_UV, color: Color32::TRANSPARENT }),
                    None => ok = false,
                }
            }
            if ok {
                for i in 0..k as u32 {
                    sm.indices.extend_from_slice(&[0, 1 + i, 1 + (i + 1) % k as u32]);
                }
                painter.add(Shape::mesh(sm));
            }
        }
    }

    // project vertices (with wind)
    let nv = m.pos.len();
    let mut screen = Vec::with_capacity(nv);
    let mut depth = Vec::with_capacity(nv);
    let mut valid = Vec::with_capacity(nv);
    let wind = a.view.wind_playing;
    let use_gpu = a.gpu.is_some() && a.view.mode == RenderMode::Shaded;
    let mut clips: Vec<Vec4> = if use_gpu { Vec::with_capacity(nv) } else { vec![] };
    let mut wpos: Vec<Vec3> = if use_gpu { Vec::with_capacity(nv) } else { vec![] };
    for i in 0..nv {
        let p = if wind { m.pos[i] + wind_offset(a.doc, m, i, a.time) } else { m.pos[i] };
        if use_gpu {
            clips.push(pr.clip(p));
            wpos.push(p);
        }
        match pr.project(p) {
            Some((s, d)) => {
                screen.push(s);
                depth.push(d);
                valid.push(true);
            }
            None => {
                screen.push(Pos2::ZERO);
                depth.push(0.0);
                valid.push(false);
            }
        }
    }

    let mut order: Vec<u32> = Vec::new();
    match a.view.mode {
        RenderMode::Shaded | RenderMode::Wireframe => {
            let tri_count = if a.view.show_leaves { m.tris.len() } else { m.leaf_tri_start };
            let mut keyed: Vec<(f32, u32)> = Vec::with_capacity(tri_count);
            for ti in 0..tri_count {
                let [i0, i1, i2] = m.tris[ti];
                let (i0, i1, i2) = (i0 as usize, i1 as usize, i2 as usize);
                if !(valid[i0] && valid[i1] && valid[i2]) {
                    continue;
                }
                let (p0, p1, p2) = (screen[i0], screen[i1], screen[i2]);
                let minx = p0.x.min(p1.x).min(p2.x);
                let maxx = p0.x.max(p1.x).max(p2.x);
                let miny = p0.y.min(p1.y).min(p2.y);
                let maxy = p0.y.max(p1.y).max(p2.y);
                if maxx < rect.left() || minx > rect.right() || maxy < rect.top() || miny > rect.bottom() {
                    continue;
                }
                if ti < m.leaf_tri_start && a.view.mode == RenderMode::Shaded {
                    // back-face cull bark (screen y points down)
                    if edge(p0, p1, p2) > 0.0 {
                        continue;
                    }
                }
                keyed.push((depth[i0] + depth[i1] + depth[i2], ti as u32));
            }
            keyed.sort_unstable_by(|x, y| y.0.total_cmp(&x.0));
            order = keyed.iter().map(|k| k.1).collect();
            if let (true, Some(gpu)) = (use_gpu, a.gpu) {
                // real depth test on the GPU: no ordering problems where surfaces meet
                // lighting happens per pixel on the GPU (so normal maps can bend it): send the
                // unlit colour, the highlight overlay, the normal and the world position
                let mut gm = crate::gpu::GpuMesh { verts: Vec::with_capacity(nv * crate::gpu::STRIDE), ..Default::default() };
                for i in 0..nv {
                    let c = clips[i];
                    let (base, kind) = if m.vmat[i] == crate::billboard::BILLBOARD_MAT { (m.col[i], 2.0) } else { (m.col[i], if m.leaf[i] { 1.0 } else { 0.0 }) };
                    let (hl, amt) = highlight_overlay(a, i);
                    let uv = m.uv[i];
                    let n = m.nrm[i];
                    let w = wpos[i];
                    gm.verts.extend_from_slice(&[c.x, c.y, c.z, c.w, base[0], base[1], base[2], hl[0], hl[1], hl[2], amt, uv[0], 1.0 - uv[1], n.x, n.y, n.z, w.x, w.y, w.z, kind, m.axis.get(i).copied().unwrap_or(0.0)]);
                }
                for &ti in &order {
                    let tri = m.tris[ti as usize];
                    let (_, mat, next) = m.tri_layers(a.doc, ti as usize);
                    let ramp = m.tri_blend(a.doc, ti as usize).map(|(k, b)| [b.layers[k].at, b.half_ramp(), b.noise_reach(), b.noise_scale]);
                    let key = crate::gpu::GroupKey {
                        color: a.textures.get(&mat).copied(),
                        normal: a.normals.get(&mat).map(|n| n.0),
                        flip_green: a.normals.get(&mat).map(|n| n.1).unwrap_or(false),
                        strength_milli: a.normals.get(&mat).map(|n| (n.2 * 1000.0) as u32).unwrap_or(1000),
                        blend: next.map(|m2| crate::gpu::BlendKey {
                            at: ramp.map(|r| r[0]).unwrap_or(0.0).to_bits(),
                            half: ramp.map(|r| r[1]).unwrap_or(0.0).to_bits(),
                            reach: ramp.map(|r| r[2]).unwrap_or(0.0).to_bits(),
                            scale: ramp.map(|r| r[3]).unwrap_or(1.0).to_bits(),
                            color: a.textures.get(&m2).copied(),
                            normal: a.normals.get(&m2).map(|n| n.0),
                            flip_green: a.normals.get(&m2).map(|n| n.1).unwrap_or(false),
                            strength_milli: a.normals.get(&m2).map(|n| (n.2 * 1000.0) as u32).unwrap_or(1000),
                        }),
                    };
                    gm.groups.entry(key).or_default().extend_from_slice(&tri);
                }
                gm.light = lp.dir.to_array();
                gm.sun = lp.sun;
                gm.ambient = lp.ambient;
                gm.sky = lp.sky;
                gm.unlit = lp.unlit;
                painter.add(gpu.callback(rect, std::sync::Arc::new(gm)));
            } else if a.view.mode == RenderMode::Shaded && !a.textures.is_empty() {
                // textured materials: one mesh per run of triangles sharing a texture,
                // so the back-to-front order is kept across textures
                let cols: Vec<Color32> = (0..nv).map(|i| draw_color(a, i)).collect();
                let mut run = egui::Mesh::default();
                let mut run_tex = egui::TextureId::default();
                for &ti in &order {
                    let tri = m.tris[ti as usize];
                    let tex = a.textures.get(&m.tri_layers(a.doc, ti as usize).1).copied();
                    let want = tex.unwrap_or_default();
                    if want != run_tex && !run.indices.is_empty() {
                        let full = std::mem::replace(&mut run, egui::Mesh::default());
                        painter.add(Shape::mesh(full));
                    }
                    run.texture_id = want;
                    run_tex = want;
                    for &vi in &tri {
                        let vi = vi as usize;
                        let uv = if tex.is_some() { Pos2::new(m.uv[vi][0], 1.0 - m.uv[vi][1]) } else { egui::epaint::WHITE_UV };
                        run.indices.push(run.vertices.len() as u32);
                        run.vertices.push(Vertex { pos: screen[vi], uv, color: cols[vi] });
                    }
                }
                if !run.indices.is_empty() {
                    painter.add(Shape::mesh(run));
                }
            } else if a.view.mode == RenderMode::Shaded {
                let mut em = egui::Mesh::default();
                em.vertices.reserve(nv);
                for i in 0..nv {
                    em.vertices.push(Vertex { pos: screen[i], uv: egui::epaint::WHITE_UV, color: draw_color(a, i) });
                }
                em.indices.reserve(order.len() * 3);
                for &ti in &order {
                    em.indices.extend_from_slice(&m.tris[ti as usize]);
                }
                painter.add(Shape::mesh(em));
            } else {
                let mut wm = egui::Mesh::default();
                for &ti in &order {
                    let [i0, i1, i2] = m.tris[ti as usize];
                    let sel = a.selected == Some(m.gen[i0 as usize]);
                    let col = if sel {
                        Color32::from_rgb(255, 170, 60)
                    } else if m.leaf[i0 as usize] {
                        Color32::from_rgba_unmultiplied(120, 220, 110, 150)
                    } else {
                        Color32::from_rgba_unmultiplied(230, 210, 190, 150)
                    };
                    let (p0, p1, p2) = (screen[i0 as usize], screen[i1 as usize], screen[i2 as usize]);
                    line(&mut wm, p0, p1, 0.8, col);
                    line(&mut wm, p1, p2, 0.8, col);
                    line(&mut wm, p2, p0, 0.8, col);
                }
                painter.add(Shape::mesh(wm));
            }
        }
        RenderMode::Skeleton => {
            let mut lm = egui::Mesh::default();
            for st in &a.skeleton.stems {
                let col = gen_color(st.gen, a.selected == Some(st.gen));
                let w = (3.0 - st.depth as f32).max(1.0);
                for seg in st.points.windows(2) {
                    if let (Some((s0, _)), Some((s1, _))) = (pr.project(seg[0]), pr.project(seg[1])) {
                        line(&mut lm, s0, s1, w, col);
                    }
                }
            }
            if a.view.show_leaves {
                for l in &a.skeleton.leaves {
                    let col = if a.selected == Some(l.gen) { Color32::from_rgb(255, 170, 60) } else { Color32::from_rgb(110, 210, 100) };
                    if let (Some((s0, _)), Some((s1, _))) = (pr.project(l.pos), pr.project(l.pos + l.dir * l.size)) {
                        line(&mut lm, s0, s1, 1.0, col);
                    }
                }
            }
            painter.add(Shape::mesh(lm));
        }
    }

    if a.view.show_helpers {
        draw_helpers(painter, &pr, a.doc, m.height.max(1.0));
    }
    if a.view.show_collision && a.doc.collision.enabled {
        draw_collision(painter, &pr, &crate::collision::capsules(a.doc, a.skeleton));
    }

    // axis gizmo
    {
        let v = cam.view();
        let origin = Pos2::new(rect.left() + 38.0, rect.bottom() - 38.0);
        for (axis, col, name) in [
            (Vec3::X, Color32::from_rgb(230, 80, 70), "X"),
            (Vec3::Y, Color32::from_rgb(110, 210, 90), "Y"),
            (Vec3::Z, Color32::from_rgb(80, 140, 240), "Z"),
        ] {
            let d = v.transform_vector3(axis);
            let end = origin + egui::vec2(d.x, -d.y) * 24.0;
            painter.line_segment([origin, end], egui::Stroke::new(2.0, col));
            painter.text(end + egui::vec2(d.x, -d.y) * 7.0, egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(11.0), col);
        }
    }
    PickData { screen, order }
}

pub fn gen_color(id: u32, selected: bool) -> Color32 {
    if selected {
        return Color32::from_rgb(255, 170, 60);
    }
    let palette = [
        Color32::from_rgb(214, 180, 140),
        Color32::from_rgb(140, 190, 230),
        Color32::from_rgb(220, 140, 200),
        Color32::from_rgb(230, 220, 120),
        Color32::from_rgb(150, 230, 200),
        Color32::from_rgb(240, 150, 120),
    ];
    palette[id as usize % palette.len()]
}

fn nice_step(x: f32) -> f32 {
    let p = 10f32.powf(x.log10().floor());
    let f = x / p;
    let n = if f < 1.5 { 1.0 } else if f < 3.5 { 2.0 } else if f < 7.5 { 5.0 } else { 10.0 };
    n * p
}

// ---------------------------------------------------------------- offscreen

/// Render the tree to an RGBA image with a z-buffer (no window needed).
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
pub fn render_offscreen(doc: &Document, m: &Mesh, cam: &Camera, w: u32, h: u32, time: f32, wind: bool, selected: Option<u32>, tex: &OffscreenTextures, lighting: Lighting) -> image::RgbaImage {
    render_core(doc, m, cam, w, h, time, wind, selected, tex, false, lighting)
}

/// Render with a transparent background (used for billboard atlases).
pub fn render_transparent(doc: &Document, m: &Mesh, cam: &Camera, w: u32, h: u32, tex: &OffscreenTextures) -> image::RgbaImage {
    render_core(doc, m, cam, w, h, 0.0, false, None, tex, true, Lighting::Noon)
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn render_core(doc: &Document, m: &Mesh, cam: &Camera, w: u32, h: u32, time: f32, wind: bool, selected: Option<u32>, tex: &OffscreenTextures, transparent: bool, lighting: Lighting) -> image::RgbaImage {
    let lp = lighting.params();
    let bgc = lp.bg.map(|c| c.map(|x| x as f32));
    let (w, h) = (w.clamp(16, 4096), h.clamp(16, 4096));
    let mut img = image::RgbaImage::new(w, h);
    for y in 0..h {
        if transparent {
            break;
        }
        let t = y as f32 / h as f32;
        let c = if t < 0.58 {
            lerp3(bgc[0], bgc[1], t / 0.58)
        } else {
            lerp3(bgc[1], bgc[2], (t - 0.58) / 0.42)
        };
        for x in 0..w {
            img.put_pixel(x, y, image::Rgba([c[0] as u8, c[1] as u8, c[2] as u8, 255]));
        }
    }
    let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(w as f32, h as f32));
    let pr = Projector::new(cam, rect);
    let n = m.pos.len();
    let mut sp = Vec::with_capacity(n);
    for i in 0..n {
        let p = if wind { m.pos[i] + wind_offset(doc, m, i, time) } else { m.pos[i] };
        sp.push(pr.project(p));
    }
    let cols: Vec<[f32; 3]> = (0..n).map(|i| vertex_color(m, i, selected, selected.is_some(), &lp)).collect();
    let mut zb = vec![f32::INFINITY; (w * h) as usize];
    for (ti, tri) in m.tris.iter().enumerate() {
        let [Some(a), Some(b), Some(c)] = tri.map(|i| sp[i as usize]) else { continue };
        let area = edge(a.0, b.0, c.0);
        if area.abs() < 1e-6 {
            continue;
        }
        if ti < m.leaf_tri_start && area > 0.0 {
            continue;
        }
        let minx = a.0.x.min(b.0.x).min(c.0.x).floor().max(0.0) as i32;
        let maxx = a.0.x.max(b.0.x).max(c.0.x).ceil().min(w as f32 - 1.0) as i32;
        let miny = a.0.y.min(b.0.y).min(c.0.y).floor().max(0.0) as i32;
        let maxy = a.0.y.max(b.0.y).max(c.0.y).ceil().min(h as f32 - 1.0) as i32;
        if minx > maxx || miny > maxy {
            continue;
        }
        let (ca, cb, cc) = (cols[tri[0] as usize], cols[tri[1] as usize], cols[tri[2] as usize]);
        let (_, mat, next) = m.tri_layers(doc, ti);
        let texture = tex.get(&mat);
        let texture2 = next.map(|m2| tex.get(&m2));
        let blend = m.tri_blend(doc, ti);
        let [la, lb, lc] = tri.map(|i| m.axis.get(i as usize).copied().unwrap_or(0.0));
        let [pa, pb, pc] = tri.map(|i| m.pos[i as usize]);
        let [ua, ub, uc] = tri.map(|i| m.uv[i as usize]);
        for y in miny..=maxy {
            for x in minx..=maxx {
                let p = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
                let w0 = edge(b.0, c.0, p) / area;
                let w1 = edge(c.0, a.0, p) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * a.1 + w1 * b.1 + w2 * c.1;
                let idx = (y as u32 * w + x as u32) as usize;
                if z < zb[idx] {
                    let mut t = [1.0f32; 3];
                    let u = ua[0] * w0 + ub[0] * w1 + uc[0] * w2;
                    let v = ua[1] * w0 + ub[1] * w1 + uc[1] * w2;
                    if let Some((timg, cutout)) = texture {
                        let px = crate::textures::sample(timg, u, 1.0 - v);
                        if *cutout && px[3] < 128 {
                            continue;
                        }
                        t = [px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0];
                    }
                    if let (Some(t2), Some((k, bl))) = (texture2, blend) {
                        let x = la * w0 + lb * w1 + lc * w2;
                        let nz = if bl.noise > 0.0 { crate::grow::blend_noise(pa * w0 + pb * w1 + pc * w2, bl.noise_scale) } else { 0.0 };
                        let bw = bl.weight(k, x, nz);
                        if bw > 0.0 {
                            let c2 = match t2 {
                                Some((img2, _)) => {
                                    let px = crate::textures::sample(img2, u, 1.0 - v);
                                    [px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0]
                                }
                                None => [1.0; 3],
                            };
                            t = [0, 1, 2].map(|k| t[k] + (c2[k] - t[k]) * bw);
                        }
                    }
                    zb[idx] = z;
                    let col = [0, 1, 2].map(|k| ((ca[k] * w0 + cb[k] * w1 + cc[k] * w2) * t[k] * 255.0).clamp(0.0, 255.0) as u8);
                    img.put_pixel(x as u32, y as u32, image::Rgba([col[0], col[1], col[2], 255]));
                }
            }
        }
    }
    img
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Force arrows, attractor markers and the pruning volume, drawn over the tree.
fn draw_collision(painter: &egui::Painter, pr: &Projector, caps: &[crate::collision::Capsule]) {
    let col = Color32::from_rgba_unmultiplied(120, 255, 170, 190);
    for c in caps {
        let (pos, tris) = crate::collision::hull(c);
        for t in tris {
            // draw each edge once-ish: only edges between consecutive indices of the triangle
            for (i, j) in [(t[0], t[1]), (t[1], t[2])] {
                if let (Some((p, _)), Some((q, _))) = (pr.project(pos[i as usize]), pr.project(pos[j as usize])) {
                    painter.line_segment([p, q], egui::Stroke::new(0.8, col));
                }
            }
        }
    }
}

fn draw_helpers(painter: &egui::Painter, pr: &Projector, doc: &Document, height: f32) {
    let seg = |a: Vec3, b: Vec3, col: Color32, w: f32| {
        if let (Some((p, _)), Some((q, _))) = (pr.project(a), pr.project(b)) {
            painter.line_segment([p, q], egui::Stroke::new(w, col));
        }
    };
    let p = &doc.pruning;
    if p.enabled {
        let col = Color32::from_rgba_unmultiplied(255, 170, 60, 170);
        let c = Vec3::from(p.center);
        let s = Vec3::from(p.size);
        match p.shape {
            crate::model::PruneShape::Box => {
                let corner = |i: u32| c + s * Vec3::new(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 });
                for i in 0..8u32 {
                    for bit in [1u32, 2, 4] {
                        if i & bit == 0 {
                            seg(corner(i), corner(i | bit), col, 1.2);
                        }
                    }
                }
            }
            crate::model::PruneShape::Ellipsoid => {
                let k = 48;
                for axis in 0..3 {
                    for i in 0..k {
                        let a0 = i as f32 / k as f32 * std::f32::consts::TAU;
                        let a1 = (i + 1) as f32 / k as f32 * std::f32::consts::TAU;
                        let pt = |a: f32| {
                            let (x, y) = (a.cos(), a.sin());
                            let v = match axis {
                                0 => Vec3::new(x, y, 0.0),
                                1 => Vec3::new(x, 0.0, y),
                                _ => Vec3::new(0.0, x, y),
                            };
                            c + v * s
                        };
                        seg(pt(a0), pt(a1), col, 1.2);
                    }
                }
            }
        }
    }
    for f in doc.forces.iter().filter(|f| f.enabled) {
        let col = if f.strength >= 0.0 { Color32::from_rgb(110, 200, 255) } else { Color32::from_rgb(255, 110, 140) };
        match f.kind {
            crate::model::ForceKind::Direction => {
                let d = Vec3::from(f.direction).normalize_or(Vec3::X) * f.strength.signum();
                let base = Vec3::new(0.0, height * 1.08, 0.0);
                let a = base - d * height * 0.18;
                let b = base + d * height * 0.18;
                seg(a, b, col, 2.5);
                if let (Some((pa, _)), Some((pb, _))) = (pr.project(a), pr.project(b)) {
                    let dir = (pb - pa).normalized();
                    let n = egui::vec2(-dir.y, dir.x);
                    painter.add(Shape::convex_polygon(vec![pb + dir * 4.0, pb - dir * 9.0 + n * 6.0, pb - dir * 9.0 - n * 6.0], col, egui::Stroke::NONE));
                }
            }
            crate::model::ForceKind::Attractor => {
                let c = Vec3::from(f.position);
                if let Some((p, _)) = pr.project(c) {
                    painter.circle_stroke(p, 7.0, egui::Stroke::new(2.0, col));
                    painter.circle_filled(p, 2.5, col);
                    painter.text(p + egui::vec2(10.0, -10.0), egui::Align2::LEFT_BOTTOM, &f.name, egui::FontId::proportional(11.0), col);
                }
            }
        }
    }
}
