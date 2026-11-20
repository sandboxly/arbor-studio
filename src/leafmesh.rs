//! Custom leaf geometry loaded from an OBJ file (leaf clusters, needles sprigs, flowers,
//! fruit...). The model is normalised so it can be placed like a leaf:
//! +Y runs from the leaf's base to its tip (length 1), +X is across the blade, +Z is the
//! face normal; the base sits at the origin.

use glam::Vec3;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

pub struct LeafMesh {
    pub pos: Vec<Vec3>,
    pub nrm: Vec<Vec3>,
    pub uv: Vec<[f32; 2]>,
    pub tris: Vec<[u32; 3]>,
}

type Entry = (Option<SystemTime>, Result<Arc<LeafMesh>, String>);

fn cache() -> &'static Mutex<HashMap<String, Entry>> {
    static C: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Load (cached, reloaded when the file changes) and normalise an OBJ leaf model.
pub fn load(path: &str) -> Result<Arc<LeafMesh>, String> {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Some((t, r)) = cache().lock().unwrap().get(path) {
        if *t == mtime {
            return r.clone();
        }
    }
    let r = parse(path).map(Arc::new);
    cache().lock().unwrap().insert(path.to_string(), (mtime, r.clone()));
    r
}

fn parse(path: &str) -> Result<LeafMesh, String> {
    if !path.to_ascii_lowercase().ends_with(".obj") {
        return Err(format!("leaf meshes must be .obj files: {path}"));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let (mut v, mut vt, mut vn) = (vec![], vec![], vec![]);
    let mut out = LeafMesh { pos: vec![], nrm: vec![], uv: vec![], tris: vec![] };
    let mut has_normals = true;
    let mut seen: HashMap<(usize, Option<usize>, Option<usize>), u32> = HashMap::new();
    let num = |s: Option<&str>| s.and_then(|x| x.parse::<f32>().ok()).unwrap_or(0.0);
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => v.push(Vec3::new(num(it.next()), num(it.next()), num(it.next()))),
            Some("vt") => vt.push([num(it.next()), num(it.next())]),
            Some("vn") => vn.push(Vec3::new(num(it.next()), num(it.next()), num(it.next()))),
            Some("f") => {
                let mut poly = vec![];
                for c in it {
                    let mut parts = c.split('/');
                    let resolve = |s: Option<&str>, n: usize| -> Option<usize> {
                        let i: i64 = s.filter(|x| !x.is_empty())?.parse().ok()?;
                        let k = if i < 0 { n as i64 + i } else { i - 1 };
                        (k >= 0 && (k as usize) < n).then_some(k as usize)
                    };
                    let pi = resolve(parts.next(), v.len()).ok_or_else(|| format!("bad face in {path}: {line}"))?;
                    let ti = resolve(parts.next(), vt.len());
                    let ni = resolve(parts.next(), vn.len());
                    if ni.is_none() {
                        has_normals = false;
                    }
                    let key = (pi, ti, ni);
                    let idx = *seen.entry(key).or_insert_with(|| {
                        out.pos.push(v[pi]);
                        out.uv.push(ti.map(|t| vt[t]).unwrap_or([0.5, 0.5]));
                        out.nrm.push(ni.map(|n| vn[n]).unwrap_or(Vec3::ZERO));
                        out.pos.len() as u32 - 1
                    });
                    poly.push(idx);
                }
                for k in 1..poly.len().saturating_sub(1) {
                    out.tris.push([poly[0], poly[k], poly[k + 1]]);
                }
            }
            _ => {}
        }
    }
    if out.tris.is_empty() {
        return Err(format!("{path} has no faces"));
    }
    if out.tris.len() > 5000 {
        return Err(format!("{path} has {} triangles; leaf meshes are repeated many times, keep them under 5000", out.tris.len()));
    }
    if !has_normals {
        out.nrm.iter_mut().for_each(|n| *n = Vec3::ZERO);
        for t in &out.tris {
            let (a, b, c) = (out.pos[t[0] as usize], out.pos[t[1] as usize], out.pos[t[2] as usize]);
            let n = (b - a).cross(c - a);
            for &i in t {
                out.nrm[i as usize] += n;
            }
        }
    }
    for n in &mut out.nrm {
        *n = n.normalize_or(Vec3::Z);
    }
    // normalise: base at the origin, length 1 along +Y
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in &out.pos {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let len = (hi.y - lo.y).max((hi - lo).max_element() * 0.05).max(1e-6);
    let base = Vec3::new((lo.x + hi.x) * 0.5, lo.y, (lo.z + hi.z) * 0.5);
    for p in &mut out.pos {
        *p = (*p - base) / len;
    }
    Ok(out)
}
