//! Engine-neutral exports: Wavefront OBJ (+MTL), glTF 2.0 binary (GLB) and a JSON data file.
//! Units are metres, Y is up. Unity, Unreal and Godot import OBJ and GLB directly
//! (Unity via its glTF package for GLB). LOD meshes are named `<Tree>_LOD0..3`, the
//! naming Unity and Unreal use to build LOD groups.
//!
//! Wind data: GLB stores per-vertex sway weights in TEXCOORD_1 (UV2):
//! u = branch sway (0 at the ground .. 1 at crown tips), v = leaf flutter (0..1).

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
}

impl ExportResult {
    /// Levels that went out over their triangle budget, as one sentence (None: all fit).
    pub fn budget_warning(&self, fitting: bool) -> Option<String> {
        let over: Vec<String> = self.budget.iter().filter(|f| f.over).map(|f| f.message(fitting)).collect();
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

pub fn export(doc: &Document, path: &Path, format: Format, with_lods: bool, with_billboard: bool) -> Result<ExportResult, String> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create folder {}: {e}", dir.display()))?;
        }
    }
    let levels: Vec<u32> = if with_lods { vec![0, 1, 2, 3] } else { vec![0] };
    let mut meshes = Vec::new();
    let mut stats = Vec::new();
    let mut fits = Vec::new();
    // every level is reduced from the same full tree, and fitted to its triangle budget
    let full = grow(doc, 0);
    for &l in &levels {
        let (sk, m, fit) = crate::budget::fit_level(doc, &full, l);
        stats.push(m.stats(&sk));
        fits.push(fit);
        meshes.push((l, m, sk));
    }
    let name = safe_name(&doc.name);
    let caps = crate::collision::capsules(doc, &meshes[0].2);
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
    let files = match format {
        Format::Obj => {
            let mut f = write_obj(doc, &name, path, &meshes, with_lods, bb_texture.as_deref(), &caps)?;
            f.extend(extra_files);
            f
        }
        Format::Glb => {
            let refs: Vec<(u32, &Mesh)> = meshes.iter().map(|(l, m, _)| (*l, m)).collect();
            write_glb(doc, &name, path, &refs, with_lods, bb_texture.as_deref(), &caps)?;
            let mut f = vec![path.to_path_buf()];
            f.extend(extra_files);
            f
        }
        Format::Json => {
            let (_, m, sk) = &meshes[0];
            let v = json!({
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
            std::fs::write(path, serde_json::to_string_pretty(&v).unwrap()).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            vec![path.to_path_buf()]
        }
    };
    Ok(ExportResult { files, lods: stats, budget: fits })
}

/// Export `count` individuals of the same species (seeds `first_seed`, `first_seed + 1`, ...)
/// into `dir` as `<tree>_v01.<ext>`, `<tree>_v02.<ext>`, ... Mesh names carry the same suffix.
pub fn export_variations(doc: &Document, dir: &Path, count: u32, first_seed: u32, format: Format, with_lods: bool, with_billboard: bool) -> Result<Vec<(u32, Vec<PathBuf>, Option<String>)>, String> {
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
        let r = export(&d, &path, format, with_lods, with_billboard)?;
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
    roughness: f32,
    cutout: bool,
    double_sided: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotKey {
    DefaultBark,
    DefaultLeaves,
    Mat(u32),
    Billboard,
}

fn tri_key(m: &Mesh, ti: usize) -> SlotKey {
    let v = m.tris[ti][0] as usize;
    match m.vmat[v] {
        crate::billboard::BILLBOARD_MAT => SlotKey::Billboard,
        0 if ti >= m.leaf_tri_start => SlotKey::DefaultLeaves,
        0 => SlotKey::DefaultBark,
        id => SlotKey::Mat(id),
    }
}

/// The materials the exported meshes actually use, in a stable order.
fn slots(doc: &Document, meshes: &[&Mesh], bb_texture: Option<&str>) -> Vec<Slot> {
    let mut used: Vec<SlotKey> = vec![];
    let mut on_leaves: Vec<u32> = vec![];
    for m in meshes {
        for ti in 0..m.tris.len() {
            let k = tri_key(m, ti);
            if !used.contains(&k) {
                used.push(k);
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
                    roughness: m.roughness,
                    cutout: m.alpha_cutout,
                    double_sided: m.double_sided || on_leaves.contains(&id),
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

fn write_obj(doc: &Document, name: &str, path: &Path, meshes: &[(u32, Mesh, crate::grow::Skeleton)], with_lods: bool, bb_texture: Option<&str>, caps: &[crate::collision::Capsule]) -> Result<Vec<PathBuf>, String> {
    let mesh_refs: Vec<&Mesh> = meshes.iter().map(|(_, m, _)| m).collect();
    let slots = slots(doc, &mesh_refs, bb_texture);
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
            let tris: Vec<&[u32; 3]> = (0..m.tris.len()).filter(|&ti| tri_key(m, ti) == slot.key).map(|ti| &m.tris[ti]).collect();
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
            mtl += &format!("norm {f}\nmap_Bump {f}\n");
        }
    }
    std::fs::write(&mtl_path, mtl).map_err(|e| format!("cannot write {}: {e}", mtl_path.display()))?;
    Ok(files)
}

fn write_glb(doc: &Document, name: &str, path: &Path, meshes: &[(u32, &Mesh)], with_lods: bool, bb_texture: Option<&str>, caps: &[crate::collision::Capsule]) -> Result<(), String> {
    let mesh_refs: Vec<&Mesh> = meshes.iter().map(|(_, m)| *m).collect();
    let slots = slots(doc, &mesh_refs, bb_texture);
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
    let mut texture_for = |p: &str, bin: &mut Vec<u8>, views: &mut Vec<serde_json::Value>| -> Result<usize, String> {
        if let Some((_, i)) = tex_index.iter().find(|(q, _)| q == p) {
            return Ok(*i);
        }
        let lower = p.to_ascii_lowercase();
        let (bytes, mime) = if lower.ends_with(".png") {
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
        tex_index.push((p.to_string(), textures.len() - 1));
        Ok(textures.len() - 1)
    };
    let mut materials = vec![];
    for s in &slots {
        let mut pbr = json!({"baseColorFactor": [1.0, 1.0, 1.0, 1.0], "metallicFactor": 0.0, "roughnessFactor": s.roughness});
        let mut mat = json!({"name": s.name, "doubleSided": s.double_sided, "extras": {"color": s.color}});
        if let Some(t) = &s.texture {
            pbr["baseColorTexture"] = json!({"index": texture_for(t, &mut bin, &mut views)?});
        }
        if let Some(n) = &s.normal_map {
            mat["normalTexture"] = json!({"index": texture_for(n, &mut bin, &mut views)?, "scale": s.normal_scale});
        }
        if s.cutout {
            mat["alphaMode"] = json!("MASK");
            mat["alphaCutoff"] = json!(0.5);
        }
        mat["pbrMetallicRoughness"] = pbr;
        materials.push(mat);
    }

    for (l, m) in meshes {
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

        let mut prims = vec![];
        for (mi, slot) in slots.iter().enumerate() {
            let idx: Vec<u8> = (0..m.tris.len())
                .filter(|&ti| tri_key(m, ti) == slot.key)
                .flat_map(|ti| m.tris[ti].iter().flat_map(|i| i.to_le_bytes()).collect::<Vec<u8>>())
                .collect();
            if idx.is_empty() {
                continue;
            }
            let count = idx.len() / 4;
            let iv = push_view(&mut bin, &mut views, &idx, Some(34963));
            accessors.push(json!({"bufferView": iv, "componentType": 5125, "count": count, "type": "SCALAR"}));
            prims.push(json!({
                "attributes": {"POSITION": pos_acc, "NORMAL": nrm_acc, "TEXCOORD_0": uv_acc, "TEXCOORD_1": wind_acc, "COLOR_0": col_acc},
                "indices": accessors.len() - 1,
                "material": mi,
            }));
        }
        let mname = level_name(name, *l, with_lods);
        gl_meshes.push(json!({"name": mname, "primitives": prims}));
        nodes.push(json!({"name": mname, "mesh": gl_meshes.len() - 1}));
        child_nodes.push(nodes.len() - 1);
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
            "color_attribute": "COLOR_0: material colour/tint with per-leaf variation and baked ambient occlusion; multiplies the base colour texture",
        },
    });
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
