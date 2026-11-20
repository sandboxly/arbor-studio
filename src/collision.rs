//! Collision objects: simple capsules fitted to the trunk and the thicker branches, so a
//! game engine can give the tree physics without using the render mesh. Exported as
//! low-poly convex hulls (named per engine convention) plus capsule data in node extras.

use crate::grow::Skeleton;
use crate::model::{CollisionNaming, Document};
use glam::Vec3;
use serde::Serialize;
use std::f32::consts::TAU;

#[derive(Clone, Debug, Serialize)]
pub struct Capsule {
    /// Centre of the start cap.
    pub a: [f32; 3],
    /// Centre of the end cap.
    pub b: [f32; 3],
    pub radius: f32,
    /// Key of the branch it was fitted to (see list_nodes).
    pub stem_key: u32,
}

/// Capsules for every stem up to `max_depth` (0 = trunks only) thicker than `min_radius`.
pub fn capsules(doc: &Document, sk: &Skeleton) -> Vec<Capsule> {
    let c = &doc.collision;
    let mut out = vec![];
    if !c.enabled {
        return out;
    }
    for st in &sk.stems {
        if st.root || st.depth > c.max_depth || st.points.len() < 2 {
            continue;
        }
        let n = st.points.len();
        let segs = (c.segments as usize).clamp(1, n - 1);
        // split the polyline into `segs` runs with about the same number of points
        for s in 0..segs {
            let i0 = s * (n - 1) / segs;
            let i1 = (s + 1) * (n - 1) / segs;
            if i1 <= i0 {
                continue;
            }
            let r = st.radii[i0..=i1].iter().sum::<f32>() / (i1 - i0 + 1) as f32 * c.radius_scale;
            if r < c.min_radius {
                continue;
            }
            let (a, b) = (st.points[i0], st.points[i1]);
            out.push(Capsule { a: a.to_array(), b: b.to_array(), radius: r, stem_key: st.key });
        }
    }
    out
}

/// A closed convex hull approximating the capsule (8-sided prism with pointed caps).
pub fn hull(c: &Capsule) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let (a, b) = (Vec3::from(c.a), Vec3::from(c.b));
    let axis = (b - a).normalize_or(Vec3::Y);
    let b1 = axis.any_orthonormal_vector();
    let b2 = axis.cross(b1);
    let sides = 8u32;
    let mut pos = vec![a - axis * c.radius, b + axis * c.radius];
    for end in [a, b] {
        for k in 0..sides {
            let t = k as f32 / sides as f32 * TAU;
            pos.push(end + (b1 * t.cos() + b2 * t.sin()) * c.radius);
        }
    }
    let mut tris = vec![];
    for k in 0..sides {
        let k1 = (k + 1) % sides;
        let (r0, r1) = (2 + k, 2 + k1);
        let (s0, s1) = (2 + sides + k, 2 + sides + k1);
        tris.push([0, r1, r0]);
        tris.push([r0, r1, s1]);
        tris.push([r0, s1, s0]);
        tris.push([1, s0, s1]);
    }
    (pos, tris)
}

/// Name of collision object `i` for the engine convention chosen in the document.
/// `render_mesh` is the name of the LOD0 render mesh.
pub fn object_name(naming: CollisionNaming, render_mesh: &str, tree: &str, i: usize) -> String {
    match naming {
        CollisionNaming::Neutral => format!("{tree}_Collision_{i:02}"),
        CollisionNaming::Unreal => format!("UCX_{render_mesh}_{i:02}"),
        CollisionNaming::Godot => format!("{tree}_Collision_{i:02}-convcolonly"),
    }
}
