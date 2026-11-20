//! Billboard LOD: the tree rendered from 8 sides into one atlas image, shown on four
//! crossed two-faced cards. This is the far-distance level of detail game engines use
//! when a full tree is too expensive (a few triangles instead of thousands).

use crate::grow::{build_mesh, grow, Mesh, Skeleton};
use crate::model::Document;
use crate::render::{offscreen_textures, render_transparent, Camera};
use glam::Vec3;
use std::sync::Arc;

/// Material id marking billboard vertices (never a real library material).
pub const BILLBOARD_MAT: u32 = u32::MAX;
/// LOD level number used for the billboard.
pub const BILLBOARD_LOD: u32 = 4;
pub const VIEWS: u32 = 8;
const COLS: u32 = 4;
const ROWS: u32 = 2;
const CELL_H: u32 = 512;

pub struct Billboard {
    pub mesh: Mesh,
    pub atlas: Arc<image::RgbaImage>,
}

/// The mesh (and billboard atlas, for LOD 4) for any level of detail.
pub struct LodMesh {
    pub skeleton: Skeleton,
    pub mesh: Mesh,
    pub atlas: Option<Arc<image::RgbaImage>>,
}

pub fn lod_mesh(doc: &Document, lod: u32) -> LodMesh {
    if lod >= BILLBOARD_LOD {
        let bb = build(doc);
        LodMesh { skeleton: Skeleton::default(), mesh: bb.mesh, atlas: Some(bb.atlas) }
    } else {
        let skeleton = grow(doc, lod);
        let mesh = build_mesh(doc, &skeleton, lod);
        LodMesh { skeleton, mesh, atlas: None }
    }
}

pub fn build(doc: &Document) -> Billboard {
    let sk = grow(doc, 0);
    let full = build_mesh(doc, &sk, 0);
    let tex = offscreen_textures(doc);
    let height = full.height * 1.03;
    let width = full.radius * 2.0 * 1.05;
    let cell_w = (((CELL_H as f32 * width / height).round() as u32).clamp(64, 1024) + 3) / 4 * 4;
    let mut atlas = image::RgbaImage::new(cell_w * COLS, CELL_H * ROWS);
    // near-orthographic camera: tiny field of view from far away
    let fov: f32 = 2.0;
    let dist = (height * 0.5) / (fov.to_radians() * 0.5).tan();
    // the projection is fitted to the height; widen the cell to the real aspect
    for k in 0..VIEWS {
        let cam = Camera { yaw: k as f32 * 360.0 / VIEWS as f32, pitch: 0.0, distance: dist, target: [0.0, height * 0.5, 0.0], fov };
        let img = render_transparent(doc, &full, &cam, cell_w, CELL_H, &tex);
        let (cx, cy) = ((k % COLS) * cell_w, (k / COLS) * CELL_H);
        image::imageops::replace(&mut atlas, &img, cx as i64, cy as i64);
    }
    bleed_colors(&mut atlas);
    let mesh = cards(width, height, cell_w as f32 / atlas.width() as f32, CELL_H as f32 / atlas.height() as f32, atlas.width(), atlas.height());
    Billboard { mesh, atlas: Arc::new(atlas) }
}

/// Give transparent pixels the average foliage colour so texture filtering doesn't create dark fringes.
fn bleed_colors(img: &mut image::RgbaImage) {
    let (mut sum, mut n) = ([0u64; 3], 0u64);
    for p in img.pixels() {
        if p.0[3] > 0 {
            for k in 0..3 {
                sum[k] += p.0[k] as u64;
            }
            n += 1;
        }
    }
    if n == 0 {
        return;
    }
    let avg = sum.map(|s| (s / n) as u8);
    for p in img.pixels_mut() {
        if p.0[3] == 0 {
            p.0 = [avg[0], avg[1], avg[2], 0];
        }
    }
}

/// Four crossed cards through the trunk; each card has a front face (view k) and a back
/// face (view k+4), so both sides show the matching picture.
fn cards(width: f32, height: f32, cu: f32, cv: f32, aw: u32, ah: u32) -> Mesh {
    let mut m = Mesh::default();
    let (iu, iv) = (0.5 / aw as f32, 0.5 / ah as f32);
    let face = |m: &mut Mesh, view: u32, yaw: f32| {
        let (s, c) = yaw.to_radians().sin_cos();
        let right = Vec3::new(c, 0.0, -s);
        let normal = Vec3::new(s, 0.0, c);
        let col = (view % COLS) as f32;
        let row = (view / COLS) as f32;
        let (u0, u1) = (col * cu + iu, (col + 1.0) * cu - iu);
        // internal UVs have a bottom-left origin: v = 1 - image v
        let (v_bottom, v_top) = (1.0 - ((row + 1.0) * cv - iv), 1.0 - (row * cv + iv));
        let corners = [
            (-right * width * 0.5, [u0, v_bottom]),
            (right * width * 0.5, [u1, v_bottom]),
            (right * width * 0.5 + Vec3::Y * height, [u1, v_top]),
            (-right * width * 0.5 + Vec3::Y * height, [u0, v_top]),
        ];
        let base = m.pos.len() as u32;
        for (p, uv) in corners {
            m.pos.push(p);
            m.nrm.push(normal);
            m.col.push([1.0, 1.0, 1.0]);
            m.uv.push(uv);
            let sway = (p.y / height).clamp(0.0, 1.0).powf(2.0) * 0.6;
            m.wind.push([sway, 0.0]);
            m.phase.push(0.0);
            m.gen.push(0);
            m.vmat.push(BILLBOARD_MAT);
            m.stem.push(u32::MAX);
            m.leaf.push(false);
        }
        m.tris.push([base, base + 1, base + 2]);
        m.tris.push([base, base + 2, base + 3]);
    };
    for k in 0..VIEWS / 2 {
        let yaw = k as f32 * 360.0 / VIEWS as f32;
        face(&mut m, k, yaw);
        face(&mut m, k + VIEWS / 2, yaw + 180.0);
    }
    m.leaf_tri_start = m.tris.len();
    m.height = height;
    m.radius = width * 0.5;
    m
}
