//! Engine-neutral exports: Wavefront OBJ (+MTL), glTF 2.0 binary (GLB) and a JSON data file.
//! Units are metres, Y is up. Unity, Unreal and Godot import OBJ and GLB directly
//! (Unity via its glTF package for GLB). LOD meshes are named `<Tree>_LOD0..3`, the
//! naming Unity and Unreal use to build LOD groups.
//!
//! Wind data: GLB stores per-vertex sway weights in TEXCOORD_1 (UV2):
//! u = branch sway (0 at the ground .. 1 at crown tips), v = leaf flutter (0..1).
//!
//! Growth (when `growth.export` is on): GLB meshes get one morph target per keyframe of the
//! growth (POSITION offsets from the mature tree), a one-bone skin (so engines that only keep
//! morph targets on skinned meshes, such as Unreal, import them) and animations that drive the
//! morph weights: "Grow" from nothing to the mature tree, and one clip per stage. OBJ gets one
//! extra file per stage; JSON gets the growth timing of every stem and leaf.
//!
//! Material gradients (several materials on a generator's stems): GLB keeps every material and
//! stores the blend in TEXCOORD_2 (x = position on the gradient's axis, y = layer value); each
//! primitive where two layers meet names its second material and the blend formula in its
//! extras. With `bake_blend` (and always for OBJ, which cannot carry weights) the transitions are
//! baked into plain materials instead, see [`crate::blendbake`]. JSON has the gradient itself.

use crate::grow::{grow, Mesh, Stats};
use crate::model::Document;
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Obj,
    Glb,
    Json,
}

impl Format {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "obj" => Some(Format::Obj),
            "glb" | "gltf" => Some(Format::Glb),
            "json" => Some(Format::Json),
            _ => None,
        }
    }
    pub fn from_path(p: &Path) -> Option<Self> {
        p.extension().and_then(|e| e.to_str()).and_then(Format::parse)
    }
}

pub struct ExportResult {
    pub files: Vec<PathBuf>,
    pub lods: Vec<Stats>,
    /// Each exported mesh level against its triangle budget.
    pub budget: Vec<crate::budget::LevelFit>,
    /// The growth animation that went out (keyframes, clips), when it was included.
    pub growth: Option<serde_json::Value>,
}

impl ExportResult {
    /// Levels that went out over their triangle budget, as one sentence (None: all fit).
    pub fn budget_warning(&self, fitting: bool) -> Option<String> {
        let mut over: Vec<String> = self.budget.iter().filter(|f| f.over).map(|f| f.message(fitting)).collect();
        over.extend(self.budget.iter().filter(|f| f.area_over).map(|f| f.area_message()));
        (!over.is_empty()).then(|| format!("Over the triangle budget: {}", over.join(" ")))
    }
}

fn safe_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let n = n.trim_matches('_').to_string();
    if n.is_empty() { "Tree".into() } else { n }
}

/// Name of the mesh/object for a level: `<Tree>_LOD<n>` with LODs, `<Tree>_Billboard` for a lone billboard.
fn level_name(name: &str, level: u32, with_lods: bool) -> String {
    if with_lods {
        format!("{name}_LOD{level}")
    } else if level == crate::billboard::BILLBOARD_LOD {
        format!("{name}_Billboard")
    } else {
        name.to_string()
    }
}

/// The document to export with or without the ambient occlusion baked into the vertex colours
/// (COLOR_0). Without it, COLOR_0 is only the materials' colours, the branch shading and each
/// leaf's variation: what an engine with its own AO multiplies its base colour by.
pub fn with_baked_ao(mut doc: Document, bake_ao: bool) -> Document {
    if !bake_ao {
        doc.settings.ao_strength = 0.0;
    }
    doc
}

pub fn export(doc: &Document, path: &Path, format: Format, with_lods: bool, with_billboard: bool, bake_blend: bool) -> Result<ExportResult, String> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create folder {}: {e}", dir.display()))?;
        }
    }
    let levels: Vec<u32> = if with_lods { vec![0, 1, 2, 3] } else { vec![0] };
    let mut meshes = Vec::new();
    let mut stats = Vec::new();
    let mut fits = Vec::new();
    // every level is reduced from the same full tree, and fitted to its triangle budget. With
    // the growth, a tree that sheds stems while young is the young tree (with them): the
    // grown tree's base positions have them withered away (below), so its triangles are
    // there, collapsed. Its billboard and collision are the grown tree's.
    let young = doc.growth.export && doc.has_shed();
    let full = grow(doc, 0);
    let young_full = young.then(|| crate::grow::grow_young(doc, 0));
    for &l in &levels {
        let (sk, m, fit) = match &young_full {
            Some(y) => crate::budget::young_level(doc, &full, y, l),
            None => crate::budget::fit_level(doc, &full, l),
        };
        stats.push(m.stats(&sk));
        fits.push(fit);
        meshes.push((l, m, sk));
    }
    let name = safe_name(&doc.name);
    let caps = if young { crate::collision::capsules(doc, &full) } else { crate::collision::capsules(doc, &meshes[0].2) };
    // billboard LOD: atlas picture saved next to the export, cards added as the last level
    let mut bb_texture: Option<String> = None;
    let mut extra_files = vec![];
    if with_billboard && format != Format::Json {
        let bb = crate::billboard::build(doc);
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| name.clone());
        let atlas_path = path.with_file_name(format!("{stem}_billboard.png"));
        bb.atlas.save(&atlas_path).map_err(|e| format!("cannot write {}: {e}", atlas_path.display()))?;
        bb_texture = Some(atlas_path.display().to_string());
        extra_files.push(atlas_path);
        let sk = crate::grow::Skeleton::default();
        stats.push(bb.mesh.stats(&sk));
        meshes.push((crate::billboard::BILLBOARD_LOD, bb.mesh, sk));
    }
    // growth animation: every level grows on the full tree's timing
    let mut growth = doc.growth.export.then(|| {
        let timing = crate::growth::Timing::new(doc, young_full.as_ref().unwrap_or(&full));
        let pairs: Vec<(&Mesh, &crate::grow::Skeleton)> = meshes.iter().map(|(_, m, sk)| (m, sk)).collect();
        (timing.clone(), crate::growth::animate(doc, &timing, &pairs))
    });
    // the grown tree is the growth at 1: what was shed lies collapsed where it came out
    if let (true, Some((_, anim))) = (young, growth.as_ref()) {
        for (i, (_, m, _)) in meshes.iter_mut().enumerate() {
            if let Some(last) = anim.shapes[i].last() {
                m.pos = last.clone();
                // the crown is the grown tree's, without the shed stems' leaves
                if !m.leaf_first.is_empty() {
                    crate::grow::crown_light(doc, m);
                }
            }
        }
    }
    // material gradients baked into plain materials (OBJ always: it cannot carry blend weights)
    let mut baked: Vec<crate::blendbake::BakedMaterial> = vec![];
    let has_blend = meshes.iter().any(|(_, m, _)| !m.axis.is_empty());
    let bake = has_blend && (format == Format::Obj || (format == Format::Glb && bake_blend));
    let mut bake_tmp: Option<PathBuf> = None;
    if bake {
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| name.clone());
        let dir = if format == Format::Obj {
            path.parent().map(Path::to_path_buf).unwrap_or_default()
        } else {
            let t = std::env::temp_dir().join(format!("arbor_bake_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
            std::fs::create_dir_all(&t).map_err(|e| format!("cannot create {}: {e}", t.display()))?;
            bake_tmp = Some(t.clone());
            t
        };
        for (i, (l, m, _)) in meshes.iter_mut().enumerate() {
            if m.axis.is_empty() {
                continue;
            }
            let max = match *l {
                0 => 2048,
                1 => 1024,
                _ => 512,
            };
            let tag = if with_lods { format!("{stem}_blend_LOD{l}") } else { format!("{stem}_blend") };
            let mut src = vec![];
            let first = crate::blendbake::BAKED_BASE + baked.len() as u32;
            let r = crate::blendbake::bake_mesh(doc, m, max, &dir, &tag, first, &mut src);
            let mut r = match r {
                Ok(r) => r,
                Err(e) => {
                    if let Some(t) = &bake_tmp {
                        let _ = std::fs::remove_dir_all(t);
                    }
                    return Err(e);
                }
            };
            if with_lods {
                for b in &mut r {
                    b.name = format!("{}_LOD{l}", b.name);
                }
            }
            if let Some((_, anim)) = growth.as_mut() {
                for shape in anim.shapes[i].iter_mut() {
                    let old = std::mem::take(shape);
                    *shape = src.iter().map(|&v| old[v as usize]).collect();
                }
            }
            if format == Format::Obj {
                for b in &r {
                    extra_files.push(b.texture.clone());
                    extra_files.extend(b.normal_map.clone());
                }
            }
            baked.extend(r);
        }
    }
    let growth_info = growth.as_ref().map(|_| crate::mcp::growth_info(&doc.growth));
    let files = match format {
        Format::Obj => {
            let mut f = write_obj(doc, &name, path, &meshes, with_lods, bb_texture.as_deref(), &caps, &baked)?;
            // one more file per stage before the mature tree: <file>_<Stage>.obj
            if let Some((_, anim)) = &growth {
                let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| name.clone());
                for (si, st) in doc.growth.stages.iter().enumerate().filter(|(_, s)| s.at < 1.0) {
                    let Some(k) = anim.keys.iter().position(|&g| (g - st.at).abs() < 1e-4) else { continue };
                    let staged: Vec<(u32, Mesh, crate::grow::Skeleton)> = meshes
                        .iter()
                        .enumerate()
                        .map(|(i, (l, m, sk))| {
                            let mut m = m.clone();
                            m.pos = anim.shapes[i][k].clone();
                            (*l, m, sk.clone())
                        })
                        .collect();
                    let p = path.with_file_name(format!("{stem}_{}.obj", safe_name(&doc.growth.label(si))));
                    f.extend(write_obj(doc, &name, &p, &staged, with_lods, bb_texture.as_deref(), &[], &baked)?);
                }
            }
            f.extend(extra_files);
            f
        }
        Format::Glb => {
            let refs: Vec<(u32, &Mesh)> = meshes.iter().map(|(l, m, _)| (*l, m)).collect();
            let r = write_glb(doc, &name, path, &refs, with_lods, bb_texture.as_deref(), &caps, growth.as_ref().map(|(_, a)| a), &baked);
            if let Some(t) = &bake_tmp {
                let _ = std::fs::remove_dir_all(t);
            }
            r?;
            let mut f = vec![path.to_path_buf()];
            f.extend(extra_files);
            f
        }
        Format::Json => {
            let (_, m, sk) = &meshes[0];
            let mut v = json!({
                "format": "arbor-tree",
                "version": 1,
                "units": "metres",
                "up_axis": "Y",
                "document": doc,
                "stats": m.stats(sk),
                "skeleton": sk,
                "collision": caps,
                "note": "stems: polylines with per-point radius; leaves: position, direction (base->tip), normal, size; collision: capsules (cap centres a, b and radius). 'document' holds the generator settings so the tree can be regrown.",
            });
            if let (Some((timing, _)), Some(info)) = (&growth, &growth_info) {
                let mut g = info.clone();
                g["timing"] = json!(timing);
                g["note"] = json!("Growth g runs from 0 (nothing above ground) to 1 (this tree). Stem i grows from timing.stem_start[i] for timing.stem_time[i]: it keeps its final shape and grows in size relative to its parent from where it comes out (what grows on it stays at the same place along it; the trunk grows from its base), so the tree's size is the product along the chain; its radius grows with size^1.3 times the generator's growth.radius_curve. Leaf i unfolds from timing.leaf_start[i] for timing.leaf_time[i]; until its stem is out, it sits where the stem will come out. Each generator's growth settings are in document.generators[].params.growth. Use the GLB morph targets to get the shapes exactly.");
                v["growth"] = g;
            }
            std::fs::write(path, serde_json::to_string_pretty(&v).unwrap()).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            vec![path.to_path_buf()]
        }
    };
    Ok(ExportResult { files, lods: stats, budget: fits, growth: growth_info })
}

/// Export `count` individuals of the same species (seeds `first_seed`, `first_seed + 1`, ...)
/// into `dir` as `<tree>_v01.<ext>`, `<tree>_v02.<ext>`, ... Mesh names carry the same suffix.
pub fn export_variations(doc: &Document, dir: &Path, count: u32, first_seed: u32, format: Format, with_lods: bool, with_billboard: bool, bake_blend: bool) -> Result<Vec<(u32, Vec<PathBuf>, Option<String>)>, String> {
    let count = count.clamp(1, 64);
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create folder {}: {e}", dir.display()))?;
    let ext = match format {
        Format::Obj => "obj",
        Format::Glb => "glb",
        Format::Json => "json",
    };
    let base = safe_name(&doc.name);
    let mut out = vec![];
    for i in 0..count {
        let mut d = doc.clone();
        d.settings.seed = first_seed.wrapping_add(i);
        // node edits belong to one individual; variations are free-grown
        d.nodes.clear();
        d.name = format!("{base}_v{:02}", i + 1);
        let path = dir.join(format!("{}.{ext}", d.name));
        let r = export(&d, &path, format, with_lods, with_billboard, bake_blend)?;
        let warning = if d.budget.enabled { r.budget_warning(d.budget.fit) } else { None };
        out.push((d.settings.seed, r.files, warning));
    }
    Ok(out)
}

/// One exported material: the default bark, the default leaves, or a library material.
struct Slot {
    key: SlotKey,
    name: String,
    color: [f32; 3],
    texture: Option<String>,
    normal_map: Option<String>,
    normal_scale: f32,
    /// The normal map is in the DirectX convention (green = down); GLB flips it on export.
    normal_flip_green: bool,
    roughness: f32,
    cutout: bool,
    double_sided: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotKey {
    DefaultBark,
    DefaultLeaves,
    Mat(u32),
    Baked(u32),
    Billboard,
}

fn key_of(id: u32, leaf: bool) -> SlotKey {
    match id {
        crate::billboard::BILLBOARD_MAT => SlotKey::Billboard,
        0 if leaf => SlotKey::DefaultLeaves,
        0 => SlotKey::DefaultBark,
        id if id >= crate::blendbake::BAKED_BASE => SlotKey::Baked(id),
        id => SlotKey::Mat(id),
    }
}

/// The material a triangle is drawn with (the first of the two where a material gradient blends).
fn tri_key(doc: &Document, m: &Mesh, ti: usize) -> SlotKey {
    let (_, a, _) = m.tri_layers(doc, ti);
    key_of(a, ti >= m.leaf_tri_start)
}

/// Where a gradient blends two materials on a triangle: (second material, layer index, generator).
fn tri_blend_key(doc: &Document, m: &Mesh, ti: usize) -> Option<(SlotKey, usize, u32)> {
    let (_, _, b) = m.tri_layers(doc, ti);
    let b = b?;
    let (k, _) = m.tri_blend(doc, ti)?;
    Some((key_of(b, false), k, m.gen[m.tris[ti][0] as usize]))
}

/// The materials the exported meshes actually use, in a stable order.
fn slots(doc: &Document, meshes: &[&Mesh], bb_texture: Option<&str>, baked: &[crate::blendbake::BakedMaterial]) -> Vec<Slot> {
    let mut used: Vec<SlotKey> = vec![];
    let mut on_leaves: Vec<u32> = vec![];
    for m in meshes {
        for ti in 0..m.tris.len() {
            let k = tri_key(doc, m, ti);
            if !used.contains(&k) {
                used.push(k);
            }
            if let Some((b, _, _)) = tri_blend_key(doc, m, ti) {
                if !used.contains(&b) {
                    used.push(b);
                }
            }
            if let SlotKey::Mat(id) = k {
                if ti >= m.leaf_tri_start && !on_leaves.contains(&id) {
                    on_leaves.push(id);
                }
            }
        }
    }
    let mut order = vec![SlotKey::DefaultBark, SlotKey::DefaultLeaves];
    order.extend(doc.materials.iter().map(|m| SlotKey::Mat(m.id)));
    order.extend(baked.iter().map(|b| SlotKey::Baked(b.id)));
    order.push(SlotKey::Billboard);
    let mut names: Vec<String> = vec![];
    let mut out = vec![];
    for key in order.into_iter().filter(|k| used.contains(k)) {
        let s = &doc.settings;
        let mut slot = match key {
            SlotKey::DefaultBark => Slot {
                key,
                name: "Bark".into(),
                color: s.bark_color.map(|c| c as f32 / 255.0),
                texture: None,
                normal_map: None,
                normal_scale: 1.0,
                normal_flip_green: false,
                roughness: 0.95,
                cutout: false,
                double_sided: false,
            },
            SlotKey::DefaultLeaves => Slot {
                key,
                name: "Leaves".into(),
                color: s.leaf_color.map(|c| c as f32 / 255.0),
                texture: None,
                normal_map: None,
                normal_scale: 1.0,
                normal_flip_green: false,
                roughness: 0.8,
                cutout: false,
                double_sided: true,
            },
            SlotKey::Billboard => Slot {
                key,
                name: "Billboard".into(),
                color: [1.0, 1.0, 1.0],
                texture: bb_texture.map(String::from),
                normal_map: None,
                normal_scale: 1.0,
                normal_flip_green: false,
                roughness: 0.9,
                cutout: true,
                // each card has separate front and back faces, so it must stay single-sided
                double_sided: false,
            },
            SlotKey::Mat(id) => {
                let m = doc.material(id).unwrap();
                Slot {
                    key,
                    name: safe_name(&m.name),
                    color: m.color.map(|c| c as f32 / 255.0),
                    texture: m.texture.clone().filter(|p| Path::new(p).is_file()),
                    normal_map: m.normal_map.clone().filter(|p| Path::new(p).is_file()),
                    normal_scale: m.normal_strength,
                    normal_flip_green: m.normal_flip_green,
                    roughness: m.roughness,
                    cutout: m.alpha_cutout,
                    double_sided: m.double_sided || on_leaves.contains(&id),
                }
            }
            SlotKey::Baked(id) => {
                let b = baked.iter().find(|b| b.id == id).unwrap();
                Slot {
                    key,
                    name: safe_name(&b.name),
                    color: [1.0, 1.0, 1.0],
                    texture: Some(b.texture.display().to_string()),
                    normal_map: b.normal_map.as_ref().map(|p| p.display().to_string()),
                    // the baked normal map already has each layer's strength in it
                    normal_scale: 1.0,
                    normal_flip_green: false,
                    roughness: b.roughness,
                    cutout: false,
                    double_sided: false,
                }
            }
        };
        if names.contains(&slot.name) {
            slot.name = format!("{}_{}", slot.name, out.len() + 1);
        }
        names.push(slot.name.clone());
        out.push(slot);
    }
    out
}

fn write_obj(doc: &Document, name: &str, path: &Path, meshes: &[(u32, Mesh, crate::grow::Skeleton)], with_lods: bool, bb_texture: Option<&str>, caps: &[crate::collision::Capsule], baked: &[crate::blendbake::BakedMaterial]) -> Result<Vec<PathBuf>, String> {
    let mesh_refs: Vec<&Mesh> = meshes.iter().map(|(_, m, _)| m).collect();
    let slots = slots(doc, &mesh_refs, bb_texture, baked);
    let mtl_path = path.with_extension("mtl");
    let mtl_name = mtl_path.file_name().unwrap().to_string_lossy().to_string();
    let err = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    let mut out = std::io::BufWriter::new(std::fs::File::create(path).map_err(err)?);
    writeln!(out, "# Tree exported from ArborStudio (metres, Y up)\nmtllib {mtl_name}").map_err(err)?;
    let mut offset = 1usize;
    for (l, m, _) in meshes {
        let obj = level_name(name, *l, with_lods);
        writeln!(out, "o {obj}").map_err(err)?;
        for p in &m.pos {
            writeln!(out, "v {:.5} {:.5} {:.5}", p.x, p.y, p.z).map_err(err)?;
        }
        for t in &m.uv {
            writeln!(out, "vt {:.5} {:.5}", t[0], t[1]).map_err(err)?;
        }
        for n in &m.nrm {
            writeln!(out, "vn {:.4} {:.4} {:.4}", n.x, n.y, n.z).map_err(err)?;
        }
        for slot in &slots {
            let tris: Vec<&[u32; 3]> = (0..m.tris.len()).filter(|&ti| tri_key(doc, m, ti) == slot.key).map(|ti| &m.tris[ti]).collect();
            if tris.is_empty() {
                continue;
            }
            writeln!(out, "g {obj}_{}\nusemtl {}", slot.name, slot.name).map_err(err)?;
            for t in tris {
                let [a, b, c] = t.map(|i| i as usize + offset);
                writeln!(out, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}").map_err(err)?;
            }
        }
        offset += m.pos.len();
    }
    // collision hulls: positions only, relative (negative) indices
    let render0 = level_name(name, 0, with_lods);
    for (i, c) in caps.iter().enumerate() {
        let (pos, tris) = crate::collision::hull(c);
        writeln!(out, "o {}", crate::collision::object_name(doc.collision.naming, &render0, name, i)).map_err(err)?;
        for p in &pos {
            writeln!(out, "v {:.5} {:.5} {:.5}", p.x, p.y, p.z).map_err(err)?;
        }
        let nv = pos.len() as i64;
        for t in &tris {
            let r = |k: u32| k as i64 - nv;
            writeln!(out, "f {} {} {}", r(t[0]), r(t[1]), r(t[2])).map_err(err)?;
        }
    }
    out.flush().map_err(err)?;

    // materials, with texture files copied next to the OBJ so the folder is self-contained
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut files = vec![path.to_path_buf(), mtl_path.clone()];
    let mut copy = |src: &str| -> Result<String, String> {
        let src = Path::new(src);
        let fname = src.file_name().ok_or("bad texture path")?.to_string_lossy().to_string();
        let dest = dir.join(&fname);
        if std::fs::canonicalize(src).ok() != std::fs::canonicalize(&dest).ok() {
            std::fs::copy(src, &dest).map_err(|e| format!("cannot copy texture {}: {e}", src.display()))?;
            files.push(dest);
        }
        Ok(fname)
    };
    let mut mtl = String::from("# ArborStudio materials\n");
    for s in &slots {
        let ns = ((1.0 - s.roughness) * 200.0).max(1.0);
        mtl += &format!("\nnewmtl {}\nKd {:.3} {:.3} {:.3}\nKa 0 0 0\nKs 0.04 0.04 0.04\nNs {:.0}\nillum 2\n", s.name, s.color[0], s.color[1], s.color[2], ns);
        if let Some(t) = &s.texture {
            let f = copy(t)?;
            mtl += &format!("map_Kd {f}\n");
            if s.cutout {
                mtl += &format!("map_d {f}\n");
            }
        }
        if let Some(n) = &s.normal_map {
            let f = copy(n)?;
            if s.normal_flip_green {
                mtl += "# normal map in the DirectX convention (green = down), copied as is\n";
            }
            mtl += &format!("norm {f}\nmap_Bump {f}\n");
        }
    }
    std::fs::write(&mtl_path, mtl).map_err(|e| format!("cannot write {}: {e}", mtl_path.display()))?;
    Ok(files)
}

#[allow(clippy::too_many_arguments)]
fn write_glb(doc: &Document, name: &str, path: &Path, meshes: &[(u32, &Mesh)], with_lods: bool, bb_texture: Option<&str>, caps: &[crate::collision::Capsule], growth: Option<&crate::growth::Animation>, baked: &[crate::blendbake::BakedMaterial]) -> Result<(), String> {
    let mesh_refs: Vec<&Mesh> = meshes.iter().map(|(_, m)| *m).collect();
    let slots = slots(doc, &mesh_refs, bb_texture, baked);
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut gl_meshes = Vec::new();
    let mut nodes = vec![];
    let mut child_nodes = vec![];

    fn push_view(bin: &mut Vec<u8>, views: &mut Vec<serde_json::Value>, data: &[u8], target: Option<u32>) -> usize {
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let off = bin.len();
        bin.extend_from_slice(data);
        let mut v = json!({"buffer": 0, "byteOffset": off, "byteLength": data.len()});
        if let Some(t) = target {
            v["target"] = json!(t);
        }
        views.push(v);
        views.len() - 1
    }
    fn f32s(v: impl Iterator<Item = f32>) -> Vec<u8> {
        v.flat_map(|x| x.to_le_bytes()).collect()
    }

    // embedded images
    let mut images = vec![];
    let mut textures = vec![];
    let mut tex_index: Vec<(String, usize)> = vec![];
    let mut texture_for = |p: &str, flip_green: bool, bin: &mut Vec<u8>, views: &mut Vec<serde_json::Value>| -> Result<usize, String> {
        let key = if flip_green { format!("{p}#flip") } else { p.to_string() };
        if let Some((_, i)) = tex_index.iter().find(|(q, _)| *q == key) {
            return Ok(*i);
        }
        let lower = p.to_ascii_lowercase();
        let (bytes, mime) = if flip_green {
            // glTF normal maps are OpenGL style (green = up): flip DirectX ones on the way out
            let mut img = image::open(p).map_err(|e| format!("cannot read texture {p}: {e}"))?.to_rgba8();
            for px in img.pixels_mut() {
                px.0[1] = 255 - px.0[1];
            }
            let mut buf = std::io::Cursor::new(Vec::new());
            img.write_to(&mut buf, image::ImageFormat::Png).map_err(|e| format!("cannot encode texture {p}: {e}"))?;
            (buf.into_inner(), "image/png")
        } else if lower.ends_with(".png") {
            (std::fs::read(p).map_err(|e| format!("cannot read texture {p}: {e}"))?, "image/png")
        } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
            (std::fs::read(p).map_err(|e| format!("cannot read texture {p}: {e}"))?, "image/jpeg")
        } else {
            return Err(format!("textures must be PNG or JPEG: {p}"));
        };
        let bv = push_view(bin, views, &bytes, None);
        let iname = Path::new(p).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        images.push(json!({"name": iname, "bufferView": bv, "mimeType": mime}));
        textures.push(json!({"sampler": 0, "source": images.len() - 1}));
        tex_index.push((key, textures.len() - 1));
        Ok(textures.len() - 1)
    };
    let mut materials = vec![];
    for s in &slots {
        let mut pbr = json!({"baseColorFactor": [1.0, 1.0, 1.0, 1.0], "metallicFactor": 0.0, "roughnessFactor": s.roughness});
        let mut mat = json!({"name": s.name, "doubleSided": s.double_sided, "extras": {"color": s.color}});
        if let Some(t) = &s.texture {
            pbr["baseColorTexture"] = json!({"index": texture_for(t, false, &mut bin, &mut views)?});
        }
        if let Some(n) = &s.normal_map {
            mat["normalTexture"] = json!({"index": texture_for(n, s.normal_flip_green, &mut bin, &mut views)?, "scale": s.normal_scale});
            mat["extras"]["normal_flip_green"] = json!(s.normal_flip_green);
            mat["extras"]["normal_map_convention"] = json!(if s.normal_flip_green {
                "the source map is DirectX style (green = down); its green channel was flipped on export, so the embedded map is glTF/OpenGL style (green = up)"
            } else {
                "the source map is OpenGL style (green = up), embedded as is"
            });
        }
        if let SlotKey::Baked(_) = s.key {
            mat["extras"]["baked_material_blend"] = json!(true);
        }
        if s.cutout {
            mat["alphaMode"] = json!("MASK");
            mat["alphaCutoff"] = json!(0.5);
        }
        mat["pbrMetallicRoughness"] = pbr;
        materials.push(mat);
    }

    // growth: one morph target per keyframe before the mature tree (the base mesh)
    let n_targets = growth.map(|a| a.keys.iter().filter(|&&g| g < 1.0).count()).unwrap_or(0);
    let mut growth_nodes = vec![];
    for (mesh_i, (l, m)) in meshes.iter().enumerate() {
        // shared vertex attributes for all primitives
        let n = m.pos.len();
        let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &m.pos {
            for k in 0..3 {
                mn[k] = mn[k].min(p[k]);
                mx[k] = mx[k].max(p[k]);
            }
        }
        let pv = push_view(&mut bin, &mut views, &f32s(m.pos.iter().flat_map(|p| [p.x, p.y, p.z])), Some(34962));
        accessors.push(json!({"bufferView": pv, "componentType": 5126, "count": n, "type": "VEC3", "min": mn, "max": mx}));
        let pos_acc = accessors.len() - 1;
        let nv = push_view(&mut bin, &mut views, &f32s(m.nrm.iter().flat_map(|p| {
            let q = p.normalize_or(glam::Vec3::Y);
            [q.x, q.y, q.z]
        })), Some(34962));
        accessors.push(json!({"bufferView": nv, "componentType": 5126, "count": n, "type": "VEC3"}));
        let nrm_acc = accessors.len() - 1;
        let uv = push_view(&mut bin, &mut views, &f32s(m.uv.iter().flat_map(|t| [t[0], 1.0 - t[1]])), Some(34962));
        accessors.push(json!({"bufferView": uv, "componentType": 5126, "count": n, "type": "VEC2"}));
        let uv_acc = accessors.len() - 1;
        let wv = push_view(&mut bin, &mut views, &f32s(m.wind.iter().flat_map(|t| [t[0], t[1]])), Some(34962));
        accessors.push(json!({"bufferView": wv, "componentType": 5126, "count": n, "type": "VEC2"}));
        let wind_acc = accessors.len() - 1;
        let cv = push_view(&mut bin, &mut views, &f32s(m.col.iter().flat_map(|c| [c[0], c[1], c[2]])), Some(34962));
        accessors.push(json!({"bufferView": cv, "componentType": 5126, "count": n, "type": "VEC3"}));
        let col_acc = accessors.len() - 1;
        // the crown's depth (crate::crown): 0 on the outer shell .. 1 at the core
        let crown_acc = (!m.crown.is_empty()).then(|| {
            let v = push_view(&mut bin, &mut views, &f32s((0..n).map(|i| m.crown.get(i).copied().unwrap_or(0.0))), Some(34962));
            accessors.push(json!({"bufferView": v, "componentType": 5126, "count": n, "type": "SCALAR"}));
            accessors.len() - 1
        });
        // material gradients: x = position on the gradient's axis, y = layer value
        let blend_acc = if m.axis.len() == n {
            let bv = push_view(&mut bin, &mut views, &f32s(m.axis.iter().zip(&m.layer).flat_map(|(x, l)| [*x, *l])), Some(34962));
            accessors.push(json!({"bufferView": bv, "componentType": 5126, "count": n, "type": "VEC2"}));
            Some(accessors.len() - 1)
        } else {
            None
        };
        // bark cross-fade at joints: the parent's texture coordinates and their weight
        let xfade_acc = if m.xuv.iter().any(|x| x[2] > 0.0) {
            let x = |i: usize| m.xuv.get(i).copied().unwrap_or([0.0; 3]);
            let uvv = push_view(&mut bin, &mut views, &f32s((0..n).flat_map(|i| [x(i)[0], 1.0 - x(i)[1]])), Some(34962));
            accessors.push(json!({"bufferView": uvv, "componentType": 5126, "count": n, "type": "VEC2"}));
            let wv = push_view(&mut bin, &mut views, &f32s((0..n).flat_map(|i| [x(i)[2], 0.0])), Some(34962));
            accessors.push(json!({"bufferView": wv, "componentType": 5126, "count": n, "type": "VEC2"}));
            Some((accessors.len() - 2, accessors.len() - 1))
        } else {
            None
        };
        // growth: morph targets (position offsets) and the one-bone skin
        let mut targets = vec![];
        let mut skin_attrs = None;
        if let Some(anim) = growth {
            for (k, &g) in anim.keys.iter().enumerate() {
                if g >= 1.0 {
                    continue;
                }
                let shape = &anim.shapes[mesh_i][k];
                let d: Vec<glam::Vec3> = shape.iter().zip(&m.pos).map(|(s, b)| *s - *b).collect();
                let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
                for p in &d {
                    for c in 0..3 {
                        mn[c] = mn[c].min(p[c]);
                        mx[c] = mx[c].max(p[c]);
                    }
                }
                let tv = push_view(&mut bin, &mut views, &f32s(d.iter().flat_map(|p| [p.x, p.y, p.z])), Some(34962));
                accessors.push(json!({"bufferView": tv, "componentType": 5126, "count": n, "type": "VEC3", "min": mn, "max": mx}));
                let mut target = json!({"POSITION": accessors.len() - 1});
                // the crown of the tree at this keyframe: its leaves turned out of THAT crown
                // (a sapling's out of its own, not the grown tree's) and their depth in it
                if crown_acc.is_some() {
                    let flat: Vec<glam::Vec3> = (0..n).map(|i| m.flat_nrm.get(i).copied().unwrap_or(m.nrm[i])).collect();
                    let (kn, kd) = crate::crown::shape(shape, &flat, &m.leaf, &m.leaf_first, &m.tris, m.leaf_tri_start, doc.settings.crown_normals, doc.settings.crown_lobes).unwrap_or_else(|| (m.nrm.clone(), vec![0.0; n]));
                    let nd: Vec<f32> = (0..n).flat_map(|i| {
                        let d = kn[i].normalize_or(glam::Vec3::Y) - m.nrm[i].normalize_or(glam::Vec3::Y);
                        [d.x, d.y, d.z]
                    }).collect();
                    let nv = push_view(&mut bin, &mut views, &f32s(nd.into_iter()), Some(34962));
                    accessors.push(json!({"bufferView": nv, "componentType": 5126, "count": n, "type": "VEC3"}));
                    target["NORMAL"] = json!(accessors.len() - 1);
                    let dv = push_view(&mut bin, &mut views, &f32s((0..n).map(|i| kd[i] - m.crown.get(i).copied().unwrap_or(0.0))), Some(34962));
                    accessors.push(json!({"bufferView": dv, "componentType": 5126, "count": n, "type": "SCALAR"}));
                    target["_CROWN_DEPTH"] = json!(accessors.len() - 1);
                }
                targets.push(target);
            }
            let jv = push_view(&mut bin, &mut views, &vec![0u8; n * 4], Some(34962));
            accessors.push(json!({"bufferView": jv, "componentType": 5121, "count": n, "type": "VEC4"}));
            let joints = accessors.len() - 1;
            let wv = push_view(&mut bin, &mut views, &f32s((0..n).flat_map(|_| [1.0f32, 0.0, 0.0, 0.0])), Some(34962));
            accessors.push(json!({"bufferView": wv, "componentType": 5126, "count": n, "type": "VEC4"}));
            skin_attrs = Some((joints, accessors.len() - 1));
        }

        let mut prims = vec![];
        // one primitive per material, and one more per gradient transition into a second one
        // a primitive per material, transition and texture coordinate scale (a gradient layer
        // shows its own tiling: extras.uv_scale)
        let mut groups: Vec<(usize, Option<(SlotKey, usize, u32)>, [u32; 2], Vec<usize>)> = vec![];
        for ti in 0..m.tris.len() {
            let a = tri_key(doc, m, ti);
            let Some(mi) = slots.iter().position(|s| s.key == a) else { continue };
            let b = tri_blend_key(doc, m, ti);
            let gen = m.gen[m.tris[ti][0] as usize];
            // (a baked mesh has its gradient axis cleared and its layers' coordinates scaled)
            let sc = if ti < m.leaf_tri_start && !m.axis.is_empty() { doc.layer_uv_scale(gen, m.tri_layers(doc, ti).1) } else { [1.0, 1.0] };
            let sc = sc.map(f32::to_bits);
            match groups.iter_mut().find(|g| g.0 == mi && g.1 == b && g.2 == sc) {
                Some(g) => g.3.push(ti),
                None => groups.push((mi, b, sc, vec![ti])),
            }
        }
        groups.sort_by_key(|g| (g.0, g.1.is_some()));
        for (mi, b, sc, tris) in groups {
            let sc = sc.map(f32::from_bits);
            let idx: Vec<u8> = tris.iter().flat_map(|&ti| m.tris[ti].iter().flat_map(|i| i.to_le_bytes()).collect::<Vec<u8>>()).collect();
            let count = idx.len() / 4;
            let iv = push_view(&mut bin, &mut views, &idx, Some(34963));
            accessors.push(json!({"bufferView": iv, "componentType": 5125, "count": count, "type": "SCALAR"}));
            let mut prim = json!({
                "attributes": {"POSITION": pos_acc, "NORMAL": nrm_acc, "TEXCOORD_0": uv_acc, "TEXCOORD_1": wind_acc, "COLOR_0": col_acc},
                "indices": accessors.len() - 1,
                "material": mi,
            });
            if let Some(ca) = crown_acc {
                prim["attributes"]["_CROWN_DEPTH"] = json!(ca);
            }
            if let Some(ba) = blend_acc {
                prim["attributes"]["TEXCOORD_2"] = json!(ba);
            }
            if let Some((xu, xw)) = xfade_acc {
                // texture coordinate sets must be numbered without gaps
                let first = if blend_acc.is_some() { 3 } else { 2 };
                prim["attributes"][format!("TEXCOORD_{first}")] = json!(xu);
                prim["attributes"][format!("TEXCOORD_{}", first + 1)] = json!(xw);
            }
            if let Some((bk, k, gen)) = b {
                if let Some(bl) = doc.get(gen).and_then(|g| g.blend.as_ref()) {
                    let bi = slots.iter().position(|s| s.key == bk);
                    prim["extras"] = json!({"material_blend": {
                        "material_b": bi,
                        "layer": k,
                        "axis": bl.axis,
                        "at": bl.layers[k].at,
                        "half_ramp": bl.half_ramp(),
                        "noise_reach": bl.noise_reach(),
                        "noise_scale": bl.noise_scale,
                        "uv_scale_b": doc.layer_uv_scale(gen, match bk { SlotKey::Mat(id) => id, _ => 0 }),
                    }});
                }
            }
            if sc != [1.0, 1.0] {
                prim["extras"]["uv_scale"] = json!(sc);
            }
            if let Some((j, w)) = skin_attrs {
                prim["attributes"]["JOINTS_0"] = json!(j);
                prim["attributes"]["WEIGHTS_0"] = json!(w);
                prim["targets"] = json!(targets);
            }
            prims.push(prim);
        }
        let mname = level_name(name, *l, with_lods);
        let mut gm = json!({"name": mname, "primitives": prims});
        let mut node = json!({"name": mname, "mesh": gl_meshes.len()});
        if let Some(anim) = growth {
            let names: Vec<String> = anim.keys.iter().filter(|&&g| g < 1.0).map(|g| format!("growth_{:03.0}", g * 100.0)).collect();
            gm["weights"] = json!(vec![0.0f32; n_targets]);
            gm["extras"] = json!({"targetNames": names});
            node["skin"] = json!(0);
        }
        gl_meshes.push(gm);
        nodes.push(node);
        child_nodes.push(nodes.len() - 1);
        growth_nodes.push(nodes.len() - 1);
    }
    // growth: the bone of the skin, and one animation per clip driving the morph weights
    let mut skins = vec![];
    let mut animations = vec![];
    if let Some(anim) = growth {
        nodes.push(json!({"name": format!("{name}_Root")}));
        let joint = nodes.len() - 1;
        child_nodes.push(joint);
        skins.push(json!({"name": format!("{name}_Growth"), "joints": [joint], "skeleton": joint}));
        let g = &doc.growth;
        for (clip, from, to) in crate::growth::clips(g) {
            let ks: Vec<usize> = (0..anim.keys.len()).filter(|&k| anim.keys[k] >= from - 1e-4 && anim.keys[k] <= to + 1e-4).collect();
            if ks.len() < 2 {
                continue;
            }
            let times: Vec<f32> = ks.iter().map(|&k| (anim.keys[k] - from).max(0.0) * g.duration).collect();
            let tv = push_view(&mut bin, &mut views, &f32s(times.iter().copied()), None);
            accessors.push(json!({"bufferView": tv, "componentType": 5126, "count": times.len(), "type": "SCALAR", "min": [times[0]], "max": [*times.last().unwrap()]}));
            let input = accessors.len() - 1;
            // keyframe k shows morph target k at full weight (the mature tree: all weights 0)
            let mut w = vec![];
            for &k in &ks {
                for t in 0..n_targets {
                    w.push(if t == k && anim.keys[k] < 1.0 { 1.0f32 } else { 0.0 });
                }
            }
            let ov = push_view(&mut bin, &mut views, &f32s(w.iter().copied()), None);
            accessors.push(json!({"bufferView": ov, "componentType": 5126, "count": w.len(), "type": "SCALAR"}));
            let output = accessors.len() - 1;
            let channels: Vec<serde_json::Value> = growth_nodes.iter().map(|&nd| json!({"sampler": 0, "target": {"node": nd, "path": "weights"}})).collect();
            animations.push(json!({"name": clip, "samplers": [{"input": input, "output": output, "interpolation": "LINEAR"}], "channels": channels}));
        }
    }
    // collision hulls (no material) grouped under one node, capsule data in extras
    if !caps.is_empty() {
        let render0 = level_name(name, 0, with_lods);
        let mut coll_nodes = vec![];
        for (i, c) in caps.iter().enumerate() {
            let (pos, tris) = crate::collision::hull(c);
            let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
            for p in &pos {
                for k in 0..3 {
                    mn[k] = mn[k].min(p[k]);
                    mx[k] = mx[k].max(p[k]);
                }
            }
            let pv = push_view(&mut bin, &mut views, &f32s(pos.iter().flat_map(|p| [p.x, p.y, p.z])), Some(34962));
            accessors.push(json!({"bufferView": pv, "componentType": 5126, "count": pos.len(), "type": "VEC3", "min": mn, "max": mx}));
            let pa = accessors.len() - 1;
            let idx: Vec<u8> = tris.iter().flat_map(|t| t.iter().flat_map(|i| i.to_le_bytes())).collect();
            let iv = push_view(&mut bin, &mut views, &idx, Some(34963));
            accessors.push(json!({"bufferView": iv, "componentType": 5125, "count": tris.len() * 3, "type": "SCALAR"}));
            let cname = crate::collision::object_name(doc.collision.naming, &render0, name, i);
            gl_meshes.push(json!({"name": cname, "primitives": [{"attributes": {"POSITION": pa}, "indices": accessors.len() - 1}]}));
            nodes.push(json!({"name": cname, "mesh": gl_meshes.len() - 1, "extras": {"collision": "capsule", "a": c.a, "b": c.b, "radius": c.radius}}));
            coll_nodes.push(nodes.len() - 1);
        }
        nodes.push(json!({"name": format!("{name}_Collision"), "children": coll_nodes, "extras": {"collision_group": true}}));
        child_nodes.push(nodes.len() - 1);
    }
    nodes.push(json!({"name": name, "children": child_nodes}));
    let root = nodes.len() - 1;
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let mut gltf = json!({
        "asset": {"version": "2.0", "generator": "ArborStudio"},
        "scene": 0,
        "scenes": [{"name": name, "nodes": [root]}],
        "nodes": nodes,
        "meshes": gl_meshes,
        "materials": materials,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{"byteLength": bin.len()}],
        "extras": {
            "wind": doc.settings.wind,
            "wind_attribute": "TEXCOORD_1: x = branch sway weight, y = leaf flutter weight",
            "color_attribute": if doc.settings.ao_strength > 0.0 { "COLOR_0: material colour/tint with branch shading, per-leaf variation and baked ambient occlusion; multiplies the base colour texture" } else { "COLOR_0: material colour/tint with branch shading and per-leaf variation (no ambient occlusion baked); multiplies the base colour texture" },
            "crown": {"normals": doc.settings.crown_normals, "lobes": doc.settings.crown_lobes},
            "crown_attribute": "_CROWN_DEPTH (float, every vertex): how deep the vertex sits in the crown's leaf mass, 0 on the outer shell .. 1 at the core (never 1 in a crown thinner than 2 m). Not in COLOR_0: darken the sky's (ambient) light on the leaves by it, not the albedo or the direct sun. With crown normals (extras.crown.normals > 0) the leaf normals are bent out of the crown by that share; morph targets carry NORMAL and _CROWN_DEPTH deltas so each growth keyframe is lit by its own crown.",
            "normal_maps": "glTF normal maps are OpenGL style (green = up). Materials whose source map was DirectX style have normal_flip_green: true in their extras; those maps were flipped on export.",
        },
    });
    if meshes.iter().any(|(_, m)| !m.axis.is_empty()) {
        gltf["extras"]["material_blend"] = json!({
            "attribute": "TEXCOORD_2: x = position on the gradient's axis (stem: 0 = base .. 1 = tip; height: metres above the ground; height_share: 0 = ground .. 1 = top), y = layer value (the layer the vertex is in plus how far it has blended into the next)",
            "primitives": "Each material is its own primitive. Where a gradient blends into a second material, the primitive draws the first and its extras.material_blend names the second (material_b, a material index) and the transition: layer, axis, at, half_ramp, noise_reach, noise_scale.",
            "weight": "w = smoothstep(at - half_ramp, at + half_ramp, TEXCOORD_2.x + noise_reach * noise(world_position / noise_scale)); final = mix(material, material_b, w) for colour, normal and roughness. noise is smooth value noise in -1..1; without it (or per vertex) use w = clamp(TEXCOORD_2.y - layer, 0, 1).",
            "materials": doc.generators.iter().filter_map(|g| g.blend.as_ref().map(|b| json!({"generator": g.name, "blend": b}))).collect::<Vec<_>>(),
            "bake": "Export with bake_material_blend to get plain materials instead (each transition baked into a texture of its own).",
            "uv_scale": "Every layer shows its own material's tiling: with a primitive's extras.uv_scale [su, sv] (absent: [1, 1]) its material samples at (TEXCOORD_0.x * su, 1 - (1 - TEXCOORD_0.y) * sv), the same for the joint cross-fade's coordinates, and material_blend.uv_scale_b does the same for material_b.",
        });
    }
    if meshes.iter().any(|(_, m)| m.xuv.iter().any(|x| x[2] > 0.0)) {
        gltf["extras"]["joint_crossfade"] = {
            let (a, b) = if meshes.iter().any(|(_, m)| !m.axis.is_empty()) { (3, 4) } else { (2, 3) };
            json!(format!("Where a branch or root joins its parent, its bark cross-fades from the parent's texture mapping into its own: TEXCOORD_{a} holds the parent's texture coordinates and TEXCOORD_{b}.x the weight of that mapping (1 at the opening in the parent, 0 a few branch radii out). colour = mix(texture(TEXCOORD_0), texture(TEXCOORD_{a}), TEXCOORD_{b}.x), the same for the normal map (each in its own tangent frame). Without it the texture changes mapping at the opening."))
        };
    }
    if !baked.is_empty() {
        gltf["extras"]["material_blend_baked"] = json!("Material gradients were baked: materials with extras.baked_material_blend hold the transitions (their own textures, the blend and noise painted in); everything else uses each layer's own material.");
    }
    if growth.is_some() {
        gltf["skins"] = json!(skins);
        gltf["animations"] = json!(animations);
        gltf["extras"]["growth"] = crate::mcp::growth_info(&doc.growth);
        gltf["extras"]["growth_note"] = json!("Morph target k is the tree at growth keyframes[k] (0 = nothing above ground); weights all 0 = the mature tree. Play the 'Grow' animation, or one clip per stage. The one-bone skin only lets engines that keep morph targets on skinned meshes import them; it does not move anything.");
    }
    if !images.is_empty() {
        gltf["images"] = json!(images);
        gltf["textures"] = json!(textures);
        gltf["samplers"] = json!([{"magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497}]);
    }
    let mut js = serde_json::to_vec(&gltf).unwrap();
    while js.len() % 4 != 0 {
        js.push(b' ');
    }
    let total = 12 + 8 + js.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&js);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    std::fs::write(path, out).map_err(|e| format!("cannot write {}: {e}", path.display()))
}
