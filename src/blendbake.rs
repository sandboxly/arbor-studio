//! Baking material gradients into plain materials, for engines without a custom shader and for
//! OBJ (which cannot carry blend weights).
//!
//! Triangles drawn with one layer's material keep it (with its tiling textures). The triangles
//! where two layers blend get a material of their own per transition: its texture is an atlas
//! with one cell per stem, covering the part of that stem's texture space the transition spans,
//! painted from the two materials' full-size images mixed with the same weight (noise mask
//! included) the viewport uses. Those triangles get their own copies of their vertices, with
//! texture coordinates into the atlas; positions, normals, colours and wind stay the same, so
//! nothing moves apart.

use crate::grow::Mesh;
use crate::model::Document;
use glam::Vec3;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Material ids from here on are baked transitions (never used by the library).
pub const BAKED_BASE: u32 = 0xB1E0_0000;

/// One baked transition material.
#[derive(Clone, Debug)]
pub struct BakedMaterial {
    pub id: u32,
    pub name: String,
    pub texture: PathBuf,
    pub normal_map: Option<PathBuf>,
    pub roughness: f32,
}

type Img = image::RgbaImage;

fn load_full(path: &str, cache: &mut HashMap<String, Option<std::sync::Arc<Img>>>) -> Option<std::sync::Arc<Img>> {
    cache.entry(path.to_string()).or_insert_with(|| image::open(path).ok().map(|i| std::sync::Arc::new(i.to_rgba8()))).clone()
}

/// Bilinear sample with repeat wrapping, `x`, `y` in texels (top-left origin), as 0..1 floats.
fn bilinear(img: &Img, x: f32, y: f32) -> [f32; 4] {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let (x, y) = (x - 0.5, y - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let px = |ix: i64, iy: i64| img.get_pixel(ix.rem_euclid(w) as u32, iy.rem_euclid(h) as u32).0;
    let (a, b, c, d) = (px(x0 as i64, y0 as i64), px(x0 as i64 + 1, y0 as i64), px(x0 as i64, y0 as i64 + 1), px(x0 as i64 + 1, y0 as i64 + 1));
    let mut out = [0.0f32; 4];
    for k in 0..4 {
        let top = a[k] as f32 + (b[k] as f32 - a[k] as f32) * fx;
        let bot = c[k] as f32 + (d[k] as f32 - c[k] as f32) * fx;
        out[k] = (top + (bot - top) * fy) / 255.0;
    }
    out
}

/// A layer material's look for baking: colour image (or white), normal image (or flat) and
/// whether its normal map uses the DirectX convention.
struct Look {
    color: Option<std::sync::Arc<Img>>,
    normal: Option<std::sync::Arc<Img>>,
    flip: bool,
    strength: f32,
    roughness: f32,
    name: String,
}

fn look(doc: &Document, id: u32, cache: &mut HashMap<String, Option<std::sync::Arc<Img>>>) -> Look {
    match doc.material(id) {
        Some(m) => Look {
            color: m.texture.as_deref().and_then(|p| load_full(p, cache)),
            normal: m.normal_map.as_deref().and_then(|p| load_full(p, cache)),
            flip: m.normal_flip_green,
            strength: m.normal_strength,
            roughness: m.roughness,
            name: m.name.clone(),
        },
        None => Look { color: None, normal: None, flip: false, strength: 1.0, roughness: 0.95, name: "Bark".into() },
    }
}

/// Texels per texture repeat of a look (its image size, or a small default for plain colours).
fn density(l: &Look) -> (f32, f32) {
    l.color.as_ref().or(l.normal.as_ref()).map(|i| (i.width() as f32, i.height() as f32)).unwrap_or((64.0, 64.0))
}

/// Colour of a look at texture coordinate (u, v) (v up), 0..1.
fn sample_color(l: &Look, u: f32, v: f32) -> [f32; 3] {
    match &l.color {
        Some(img) => {
            let p = bilinear(img, u * img.width() as f32, (1.0 - v) * img.height() as f32);
            [p[0], p[1], p[2]]
        }
        None => [1.0; 3],
    }
}

/// Tangent-space normal of a look at (u, v), in the OpenGL convention, strength applied.
fn sample_normal(l: &Look, u: f32, v: f32) -> Vec3 {
    match &l.normal {
        Some(img) => {
            let p = bilinear(img, u * img.width() as f32, (1.0 - v) * img.height() as f32);
            let mut n = Vec3::new(p[0] * 2.0 - 1.0, p[1] * 2.0 - 1.0, p[2] * 2.0 - 1.0);
            if l.flip {
                n.y = -n.y;
            }
            n.x *= l.strength;
            n.y *= l.strength;
            n.normalize_or(Vec3::Z)
        }
        None => Vec3::Z,
    }
}

struct Cell {
    tris: Vec<usize>,
    umin: f32,
    vmax: f32,
    w: u32,
    h: u32,
    x: u32,
    y: u32,
}

const PAD: u32 = 3;

/// Shelf packing of cells into a square-ish atlas no wider than `max`; scales cells by `s`.
fn pack(cells: &mut [Cell], du: &[f32], dv: &[f32], s: f32, max: u32) -> Option<(u32, u32)> {
    let mut order: Vec<usize> = (0..cells.len()).collect();
    for (i, c) in cells.iter_mut().enumerate() {
        c.w = ((du[i] * s).ceil() as u32).max(1) + 2 * PAD;
        c.h = ((dv[i] * s).ceil() as u32).max(1) + 2 * PAD;
    }
    order.sort_by(|&a, &b| cells[b].h.cmp(&cells[a].h));
    let (mut x, mut y, mut shelf, mut width) = (0u32, 0u32, 0u32, 0u32);
    for &i in &order {
        let c = &mut cells[i];
        if c.w > max {
            return None;
        }
        if x + c.w > max {
            x = 0;
            y += shelf;
            shelf = 0;
        }
        c.x = x;
        c.y = y;
        x += c.w;
        shelf = shelf.max(c.h);
        width = width.max(x);
    }
    let height = y + shelf;
    (height <= max).then_some((width.max(4), height.max(4)))
}

/// Bake the transitions of one mesh. Returns the new materials (written as PNG files into `dir`
/// named `<stem>_<n>`); the mesh gets vertex copies for the transition triangles, and every bark
/// vertex of a gradient generator gets its layer's material, so writers need nothing else.
/// `src` receives, per new vertex, the vertex it was copied from (for morph targets).
pub fn bake_mesh(doc: &Document, m: &mut Mesh, max_size: u32, dir: &Path, file_stem: &str, first_id: u32, src: &mut Vec<u32>) -> Result<Vec<BakedMaterial>, String> {
    let n0 = m.pos.len();
    *src = (0..n0 as u32).collect();
    if m.axis.is_empty() {
        return Ok(vec![]);
    }
    let mut cache = HashMap::new();
    // which layer material every bark triangle uses, and the transition triangles per (gen, k)
    let mut groups: Vec<((u32, usize), Vec<usize>)> = vec![];
    let mut vmat_new = m.vmat.clone();
    for ti in 0..m.leaf_tri_start.min(m.tris.len()) {
        let (_, a, next) = m.tri_layers(doc, ti);
        if next.is_none() {
            for &v in &m.tris[ti] {
                vmat_new[v as usize] = a;
            }
            continue;
        }
        let Some((k, _)) = m.tri_blend(doc, ti) else { continue };
        let gen = m.gen[m.tris[ti][0] as usize];
        match groups.iter_mut().find(|(key, _)| *key == (gen, k)) {
            Some((_, v)) => v.push(ti),
            None => groups.push(((gen, k), vec![ti])),
        }
    }
    let mut out = vec![];
    for (gi, ((gen, k), tris)) in groups.into_iter().enumerate() {
        let Some(blend) = doc.get(gen).and_then(|g| g.blend.clone()) else { continue };
        let (ida, idb) = (doc.layer_material_id(gen, k as u32), doc.layer_material_id(gen, k as u32 + 1));
        let (la, lb) = (look(doc, ida, &mut cache), look(doc, idb, &mut cache));
        let (da, db) = (density(&la), density(&lb));
        let dens = (da.0.max(db.0), da.1.max(db.1));
        // one cell per stem: the stem's texture-space rectangle over its transition triangles;
        // a long, narrow one is cut into bands along the stem so the atlas stays near square
        let mut per_stem: Vec<(u32, Vec<usize>)> = vec![];
        for &ti in &tris {
            let st = m.stem[m.tris[ti][0] as usize];
            match per_stem.iter_mut().find(|c| c.0 == st) {
                Some(c) => c.1.push(ti),
                None => per_stem.push((st, vec![ti])),
            }
        }
        let mut by_stem: Vec<Cell> = vec![];
        for (_, ts) in per_stem {
            let span = |f: &dyn Fn([f32; 2]) -> f32| {
                ts.iter().flat_map(|&ti| m.tris[ti]).map(|v| f(m.uv[v as usize])).fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(x), b.max(x)))
            };
            let (u0, u1) = span(&|uv| uv[0]);
            let (v0, v1) = span(&|uv| uv[1]);
            let (tu, tv) = ((u1 - u0).max(1e-4) * dens.0, (v1 - v0).max(1e-4) * dens.1);
            let bands = ((tv / tu).sqrt().round() as usize).clamp(1, 32);
            let mut cells: Vec<Cell> = (0..bands).map(|_| Cell { tris: vec![], umin: f32::MAX, vmax: f32::MIN, w: 0, h: 0, x: 0, y: 0 }).collect();
            for &ti in &ts {
                let vc = m.tris[ti].iter().map(|&v| m.uv[v as usize][1]).sum::<f32>() / 3.0;
                let b = (((vc - v0) / (v1 - v0).max(1e-6)) * bands as f32).floor().clamp(0.0, bands as f32 - 1.0) as usize;
                cells[b].tris.push(ti);
            }
            by_stem.extend(cells.into_iter().filter(|c| !c.tris.is_empty()));
        }
        let mut du = vec![];
        let mut dv = vec![];
        for c in &mut by_stem {
            let (mut umax, mut vmin) = (f32::MIN, f32::MAX);
            for &ti in &c.tris {
                for &v in &m.tris[ti] {
                    let uv = m.uv[v as usize];
                    c.umin = c.umin.min(uv[0]);
                    umax = umax.max(uv[0]);
                    vmin = vmin.min(uv[1]);
                    c.vmax = c.vmax.max(uv[1]);
                }
            }
            du.push((umax - c.umin).max(1e-4) * dens.0);
            dv.push((c.vmax - vmin).max(1e-4) * dens.1);
        }
        // scale the cells down until they fit the atlas
        let area: f32 = du.iter().zip(&dv).map(|(a, b)| (a + 2.0 * PAD as f32) * (b + 2.0 * PAD as f32)).sum();
        let mut s = ((max_size as f32 * max_size as f32 * 0.7) / area.max(1.0)).sqrt().min(1.0);
        let (aw, ah) = loop {
            if let Some(sz) = pack(&mut by_stem, &du, &dv, s, max_size) {
                break sz;
            }
            s *= 0.85;
            if s < 1e-4 {
                return Err("material gradient: the transitions do not fit an atlas".into());
            }
        };
        let mut col = Img::new(aw, ah);
        let want_nrm = la.normal.is_some() || lb.normal.is_some();
        let mut nrm = if want_nrm { Some(Img::from_pixel(aw, ah, image::Rgba([128, 128, 255, 255]))) } else { None };
        let mut filled = vec![false; (aw * ah) as usize];
        let id = first_id + gi as u32;
        let mut remap: HashMap<u32, u32> = HashMap::new();
        for c in &by_stem {
            let to_px = |uv: [f32; 2]| -> (f32, f32) { (c.x as f32 + PAD as f32 + (uv[0] - c.umin) * dens.0 * s, c.y as f32 + PAD as f32 + (c.vmax - uv[1]) * dens.1 * s) };
            for &ti in &c.tris {
                let tri = m.tris[ti];
                let p = tri.map(|v| to_px(m.uv[v as usize]));
                let (minx, maxx) = (p.iter().map(|q| q.0).fold(f32::MAX, f32::min), p.iter().map(|q| q.0).fold(f32::MIN, f32::max));
                let (miny, maxy) = (p.iter().map(|q| q.1).fold(f32::MAX, f32::min), p.iter().map(|q| q.1).fold(f32::MIN, f32::max));
                let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
                if area.abs() > 1e-9 {
                    for y in (miny.floor().max(0.0) as u32)..=(maxy.ceil() as u32).min(ah - 1) {
                        for x in (minx.floor().max(0.0) as u32)..=(maxx.ceil() as u32).min(aw - 1) {
                            let (qx, qy) = (x as f32 + 0.5, y as f32 + 0.5);
                            let w1 = ((p[2].0 - p[0].0) * (qy - p[0].1) - (qx - p[0].0) * (p[2].1 - p[0].1)) / -area;
                            let w2 = ((p[1].0 - p[0].0) * (qy - p[0].1) - (qx - p[0].0) * (p[1].1 - p[0].1)) / area;
                            let w0 = 1.0 - w1 - w2;
                            // a texel's centre a little outside still gets painted (no dark seams)
                            if w0 < -0.02 || w1 < -0.02 || w2 < -0.02 {
                                continue;
                            }
                            let wsum = [w0.max(0.0), w1.max(0.0), w2.max(0.0)];
                            let t = wsum[0] + wsum[1] + wsum[2];
                            let bw = wsum.map(|x| x / t.max(1e-6));
                            let lerp = |f: &dyn Fn(usize) -> f32| bw[0] * f(0) + bw[1] * f(1) + bw[2] * f(2);
                            let u = lerp(&|i| m.uv[tri[i] as usize][0]);
                            let v = lerp(&|i| m.uv[tri[i] as usize][1]);
                            let x_axis = lerp(&|i| m.axis[tri[i] as usize]);
                            let pos = m.pos[tri[0] as usize] * bw[0] + m.pos[tri[1] as usize] * bw[1] + m.pos[tri[2] as usize] * bw[2];
                            let nz = if blend.noise > 0.0 { crate::grow::blend_noise(pos, blend.noise_scale) } else { 0.0 };
                            let wb = blend.weight(k, x_axis, nz);
                            let (ca, cb) = (sample_color(&la, u, v), sample_color(&lb, u, v));
                            let c3 = [0, 1, 2].map(|i| ((ca[i] + (cb[i] - ca[i]) * wb) * 255.0).round().clamp(0.0, 255.0) as u8);
                            col.put_pixel(x, y, image::Rgba([c3[0], c3[1], c3[2], 255]));
                            if let Some(nm) = nrm.as_mut() {
                                let n = (sample_normal(&la, u, v) * (1.0 - wb) + sample_normal(&lb, u, v) * wb).normalize_or(Vec3::Z);
                                let e = |x: f32| ((x * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
                                nm.put_pixel(x, y, image::Rgba([e(n.x), e(n.y), e(n.z), 255]));
                            }
                            filled[(y * aw + x) as usize] = true;
                        }
                    }
                }
                // the triangle's own vertex copies, with atlas coordinates
                for (i, &v) in tri.iter().enumerate() {
                    let key = v;
                    let nv = *remap.entry(key).or_insert_with(|| {
                        let s = v as usize;
                        m.pos.push(m.pos[s]);
                        m.nrm.push(m.nrm[s]);
                        m.col.push(m.col[s]);
                        m.uv.push([p[i].0 / aw as f32, 1.0 - p[i].1 / ah as f32]);
                        m.wind.push(m.wind[s]);
                        m.phase.push(m.phase[s]);
                        m.gen.push(m.gen[s]);
                        m.vmat.push(id);
                        vmat_new.push(id);
                        m.stem.push(m.stem[s]);
                        m.leaf.push(false);
                        if let Some(&x) = m.xuv.get(s) {
                            m.xuv.resize(m.pos.len() - 1, [0.0; 3]);
                            m.xuv.push(x);
                        }
                        src.push(v);
                        m.pos.len() as u32 - 1
                    });
                    m.tris[ti][i] = nv;
                }
            }
            // copies are shared within a cell; a vertex on two stems' cells (a joint) gets one
            // copy per cell, since it sits at a different place in each
            remap.clear();
        }
        // bleed painted texels outwards a few times so filtering never reaches unpainted ones
        for _ in 0..PAD + 1 {
            let snap = filled.clone();
            let cs = col.clone();
            let ns = nrm.clone();
            for y in 0..ah {
                for x in 0..aw {
                    if snap[(y * aw + x) as usize] {
                        continue;
                    }
                    let mut found = None;
                    for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                        if nx >= 0 && ny >= 0 && (nx as u32) < aw && (ny as u32) < ah && snap[(ny as u32 * aw + nx as u32) as usize] {
                            found = Some((nx as u32, ny as u32));
                            break;
                        }
                    }
                    if let Some((nx, ny)) = found {
                        col.put_pixel(x, y, *cs.get_pixel(nx, ny));
                        if let (Some(nm), Some(ns)) = (nrm.as_mut(), ns.as_ref()) {
                            nm.put_pixel(x, y, *ns.get_pixel(nx, ny));
                        }
                        filled[(y * aw + x) as usize] = true;
                    }
                }
            }
        }
        let name = format!("{}_to_{}", la.name, lb.name);
        let tex = dir.join(format!("{file_stem}_{}.png", gi + 1));
        col.save(&tex).map_err(|e| format!("cannot write {}: {e}", tex.display()))?;
        let normal_map = match nrm {
            Some(nm) => {
                let p = dir.join(format!("{file_stem}_{}_n.png", gi + 1));
                nm.save(&p).map_err(|e| format!("cannot write {}: {e}", p.display()))?;
                Some(p)
            }
            None => None,
        };
        out.push(BakedMaterial { id, name, texture: tex, normal_map, roughness: 0.5 * (la.roughness + lb.roughness) });
    }
    // remaining copies keep the vmat they were given; originals take their layer's material
    m.vmat = vmat_new;
    m.axis.clear();
    m.layer.clear();
    Ok(out)
}
