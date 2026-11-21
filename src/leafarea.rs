//! Leaf card area of a mesh: the overdraw cost of the foliage, which the triangle count does
//! not show (a few big cards can cost more than many small ones). Reported per level of detail.

use crate::grow::Mesh;
use crate::model::Document;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct LeafArea {
    /// Area of all leaf geometry in m² (one side of each card).
    pub card_m2: f32,
    /// The part of it the alpha cut-out keeps (opaque texels), in m². None when no leaf material
    /// cuts out transparent pixels (then all of the card area is drawn as leaf).
    pub opaque_m2: Option<f32>,
}

/// Share of the texels inside a texture-space triangle that the cut-out keeps (alpha >= 0.5).
fn coverage(img: &image::RgbaImage, uv: [[f32; 2]; 3]) -> f32 {
    let (w, h) = (img.width() as f32, img.height() as f32);
    // pixel space, top-left origin (texture coordinates have v up)
    let p = uv.map(|t| (t[0] * w, (1.0 - t[1]) * h));
    let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
    let alpha = |x: f32, y: f32| img.get_pixel((x.floor() as i64).rem_euclid(img.width() as i64) as u32, (y.floor() as i64).rem_euclid(img.height() as i64) as u32).0[3];
    if area.abs() < 4.0 {
        // tiny in the texture: its centre decides
        let (cx, cy) = ((p[0].0 + p[1].0 + p[2].0) / 3.0, (p[0].1 + p[1].1 + p[2].1) / 3.0);
        return if alpha(cx, cy) >= 128 { 1.0 } else { 0.0 };
    }
    let minx = p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor();
    let maxx = p.iter().map(|q| q.0).fold(f32::MIN, f32::max).ceil();
    let miny = p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor();
    let maxy = p.iter().map(|q| q.1).fold(f32::MIN, f32::max).ceil();
    // at most about 64k samples per triangle
    let step = ((maxx - minx) * (maxy - miny) / 65536.0).sqrt().max(1.0);
    let (mut inside, mut kept) = (0u32, 0u32);
    let mut y = miny + 0.5 * step;
    while y < maxy {
        let mut x = minx + 0.5 * step;
        while x < maxx {
            let w1 = ((p[2].0 - p[0].0) * (y - p[0].1) - (x - p[0].0) * (p[2].1 - p[0].1)) / -area;
            let w2 = ((p[1].0 - p[0].0) * (y - p[0].1) - (x - p[0].0) * (p[1].1 - p[0].1)) / area;
            if w1 >= 0.0 && w2 >= 0.0 && w1 + w2 <= 1.0 {
                inside += 1;
                if alpha(x, y) >= 128 {
                    kept += 1;
                }
            }
            x += step;
        }
        y += step;
    }
    if inside == 0 {
        0.0
    } else {
        kept as f32 / inside as f32
    }
}

/// Leaf card area of a mesh (leaf triangles only; the billboard has none).
pub fn measure(doc: &Document, m: &Mesh) -> LeafArea {
    let mut out = LeafArea::default();
    let mut opaque = 0.0f32;
    let mut any_cutout = false;
    let mut images: HashMap<u32, Option<crate::textures::Image>> = HashMap::new();
    let mut cache: HashMap<(u32, [i32; 6]), f32> = HashMap::new();
    for ti in m.leaf_tri_start..m.tris.len() {
        let t = m.tris[ti];
        let [a, b, c] = t.map(|i| m.pos[i as usize]);
        let area = 0.5 * (b - a).cross(c - a).length();
        out.card_m2 += area;
        let mat = m.vmat[t[0] as usize];
        let img = images
            .entry(mat)
            .or_insert_with(|| doc.material(mat).filter(|mm| mm.alpha_cutout).and_then(|mm| mm.texture.as_deref()).and_then(|p| crate::textures::load(p).ok()))
            .clone();
        let Some(img) = img else {
            opaque += area;
            continue;
        };
        any_cutout = true;
        let uv = t.map(|i| m.uv[i as usize]);
        let key = (mat, [uv[0][0], uv[0][1], uv[1][0], uv[1][1], uv[2][0], uv[2][1]].map(|x| (x * 4096.0).round() as i32));
        let share = *cache.entry(key).or_insert_with(|| coverage(&img, uv));
        opaque += area * share;
    }
    let r = |x: f32| (x * 100.0).round() / 100.0;
    out.card_m2 = r(out.card_m2);
    out.opaque_m2 = any_cutout.then(|| r(opaque));
    out
}
