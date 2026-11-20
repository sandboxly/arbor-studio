//! Growth animation: the tree from a sprout to the mature tree, on one mesh whose vertices move.
//!
//! The topology never changes, so an engine can play the growth as morph targets (blend
//! shapes) between a handful of sampled shapes. Growth `g` runs from 0 (nothing above the
//! ground) to 1 (the mature tree as designed):
//!
//! * every stem is a scaled-down copy of its final shape that grows to full size: it lengthens
//!   (and thickens) from the point where it leaves its parent, carrying what grows on it;
//! * the trunk grows the whole time; a side branch, twig or root starts later the further out
//!   along its parent it sits, so lower branches are older and larger, and the crown fills in
//!   from the bottom up and from the inside out;
//! * leaves unfold from nothing at their stalk once their twig has started;
//! * optional seedling leaves sit on the young shoot until the first side branches carry
//!   their own, then shrink away where they are (a keyframe always sits where they are gone,
//!   so they never travel through the crown between two keyframes).

use crate::grow::{Mesh, Skeleton, Stem};
use crate::model::Growth;
use glam::Vec3;
use serde::Serialize;

/// A side stem starts this share of its parent's remaining growth after the parent...
const BIRTH_MIN: f32 = 0.02;
/// ...plus this share times how far out along the parent it sits (0 at the base, 1 at the tip).
const BIRTH_ALONG: f32 = 0.22;
/// Share of the growth a leaf takes to unfold.
const LEAF_TIME: f32 = 0.06;
/// Seedling leaves are gone at this point of the growth.
pub const SEEDLING_END: f32 = 0.4;
/// The trunk's length fraction is g^TRUNK_PACE: slow at first, like a seedling.
const TRUNK_PACE: f32 = 1.3;

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Closest point on a stem's axis to `q`: (arc-length fraction 0..1, the point).
pub fn locate(st: &Stem, q: Vec3) -> (f32, Vec3) {
    if st.points.len() < 2 {
        return (0.0, st.points.first().copied().unwrap_or(Vec3::ZERO));
    }
    let (mut best, mut best_arc, mut best_p) = (f32::MAX, 0.0f32, st.points[0]);
    let mut acc = 0.0f32;
    for w in st.points.windows(2) {
        let ab = w[1] - w[0];
        let len = ab.length();
        let t = ((q - w[0]).dot(ab) / (len * len).max(1e-12)).clamp(0.0, 1.0);
        let p = w[0] + ab * t;
        let d = p.distance_squared(q);
        if d < best {
            (best, best_arc, best_p) = (d, acc + len * t, p);
        }
        acc += len;
    }
    (best_arc / acc.max(1e-6), best_p)
}

/// When every stem and leaf grows, as fractions of the whole growth (0..1). Every stem
/// reaches its full size at 1.
#[derive(Clone, Serialize, Debug)]
pub struct Timing {
    /// Per stem: when it starts to grow (it grows until the end).
    pub stem_start: Vec<f32>,
    /// Per leaf: when it starts to unfold.
    pub leaf_start: Vec<f32>,
    /// How long a leaf takes to unfold.
    pub leaf_time: f32,
}

impl Timing {
    pub fn new(sk: &Skeleton) -> Timing {
        let n = sk.stems.len();
        let mut start = vec![f32::NAN; n];
        fn visit(i: usize, sk: &Skeleton, start: &mut [f32]) {
            if !start[i].is_nan() {
                return;
            }
            let st = &sk.stems[i];
            start[i] = match st.parent {
                None => 0.0,
                Some(p) => {
                    visit(p, sk, start);
                    let (a, _) = locate(&sk.stems[p], st.points[0]);
                    let b = start[p];
                    (b + (1.0 - b) * (BIRTH_MIN + BIRTH_ALONG * a)).min(0.95)
                }
            };
        }
        for i in 0..n {
            visit(i, sk, &mut start);
        }
        let leaf_start = Self::leaves(sk, &start);
        Timing { stem_start: start, leaf_start, leaf_time: LEAF_TIME }
    }

    /// Leaves start a little after their stem, later the further out they sit on it.
    fn leaves(sk: &Skeleton, stem_start: &[f32]) -> Vec<f32> {
        sk.leaves
            .iter()
            .map(|l| {
                let Some(&b) = stem_start.get(l.stem) else { return 0.0 };
                let (t, _) = locate(&sk.stems[l.stem], l.pos);
                (b + (1.0 - b) * (0.01 + 0.06 * t)).min(1.0 - LEAF_TIME)
            })
            .collect()
    }

    /// Size of stem `i` at growth `g`, as a fraction of its full size.
    pub fn scale(&self, i: usize, g: f32) -> f32 {
        let b = self.stem_start[i];
        if g <= b {
            return 0.0;
        }
        let x = ((g - b) / (1.0 - b).max(1e-4)).clamp(0.0, 1.0);
        if b <= 0.0 {
            x.powf(TRUNK_PACE)
        } else {
            x
        }
    }

    /// Size of leaf `i` at growth `g` (0 = not yet, 1 = unfolded).
    pub fn leaf(&self, i: usize, g: f32) -> f32 {
        smooth((g - self.leaf_start[i]) / self.leaf_time.max(1e-4))
    }
}

/// The growth points the animation is sampled at: an even spread plus every stage (and the end
/// of the seedling leaves), always including 0 and 1.
pub fn keyframes(growth: &Growth) -> Vec<f32> {
    let mut special: Vec<f32> = vec![0.0, 1.0];
    special.extend(growth.stages.iter().map(|s| s.at.clamp(0.0, 1.0)));
    if growth.seedling_leaves {
        special.push(SEEDLING_END);
    }
    let mut keys = special.clone();
    let k = growth.keyframes.max(2);
    let gap = 0.45 / k as f32;
    for i in 1..k {
        let g = i as f32 / k as f32;
        if special.iter().all(|s| (s - g).abs() > gap) {
            keys.push(g);
        }
    }
    keys.sort_by(|a, b| a.total_cmp(b));
    keys.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    keys
}

#[derive(Clone, Copy)]
enum Bind {
    /// Grows with the trunk's size only (billboard cards, anything without a stem).
    Fixed,
    /// On stem `s`, whose axis passes `c` there.
    Stem { s: u32, c: Vec3 },
    /// Part of leaf `l`.
    Leaf { l: u32 },
}

struct LeafInfo {
    /// The point of its stem's axis nearest to the stalk.
    axis: Vec3,
    anchor: Vec3,
    side: Vec3,
    dir: Vec3,
    normal: Vec3,
    /// Seedling slot: (index among seedling leaves, their count).
    seedling: Option<(u32, u32)>,
}

/// Where a stem is at one point of the growth.
#[derive(Clone, Copy, Default)]
struct Pose {
    /// Where its base is now, its base at full size, its size and thickness factors.
    at: Vec3,
    base: Vec3,
    scale: f32,
    thick: f32,
}

impl Pose {
    /// A point of the full-size stem with axis point `c`.
    fn place(&self, c: Vec3, q: Vec3) -> Vec3 {
        self.at + (c - self.base) * self.scale + (q - c) * self.thick
    }
}

/// Moves the vertices of one mesh (built from `sk`) to any point of the growth.
pub struct Grower {
    pub timing: Timing,
    base: Vec<Vec3>,
    bind: Vec<Bind>,
    /// Per stem: parent, and the parent's axis point where it starts.
    parents: Vec<Option<(usize, Vec3)>>,
    stem_base: Vec<Vec3>,
    leaves: Vec<LeafInfo>,
    /// Per leaf: the stem it grows on.
    leaf_stems: Vec<usize>,
    /// The stem seedling leaves sit on (the first trunk), with its axis.
    leader: Option<usize>,
    leader_axis: Vec<Vec3>,
    /// Size of the whole tree for vertices that belong to no stem.
    trunk_pose: Option<usize>,
}

impl Grower {
    /// `timing` should come from the full tree so every level of detail grows alike; the
    /// levels share its stems (reduced meshes keep every stem, but only some leaves).
    pub fn new(settings: &Growth, sk: &Skeleton, m: &Mesh, timing: &Timing) -> Grower {
        let mut timing = timing.clone();
        if timing.stem_start.len() != sk.stems.len() {
            timing = Timing::new(sk);
        } else if timing.leaf_start.len() != sk.leaves.len() {
            timing.leaf_start = Timing::leaves(sk, &timing.stem_start);
        }
        let mut bind = vec![Bind::Fixed; m.pos.len()];
        for (li, &first) in m.leaf_first.iter().enumerate() {
            let end = m.leaf_first.get(li + 1).copied().unwrap_or(m.pos.len() as u32);
            for v in first..end {
                bind[v as usize] = Bind::Leaf { l: li as u32 };
            }
        }
        for v in 0..m.pos.len() {
            if matches!(bind[v], Bind::Leaf { .. }) || m.leaf.get(v).copied().unwrap_or(false) {
                continue;
            }
            let s = m.stem.get(v).copied().unwrap_or(u32::MAX);
            if (s as usize) < sk.stems.len() {
                let (_, c) = locate(&sk.stems[s as usize], m.pos[v]);
                bind[v] = Bind::Stem { s, c };
            }
        }
        let parents = sk.stems.iter().map(|st| st.parent.map(|p| (p, locate(&sk.stems[p], st.points[0]).1))).collect();
        let stem_base = sk.stems.iter().map(|st| st.points[0]).collect();
        let leader = sk.stems.iter().position(|s| s.parent.is_none() && !s.root);
        // seedling leaves: some of the leaves that come late, spread over the whole set
        let mut seedling = vec![None; sk.leaves.len()];
        if settings.seedling_leaves && leader.is_some() {
            let late: Vec<usize> = (0..sk.leaves.len()).filter(|&i| timing.leaf_start[i] >= SEEDLING_END + 0.02).collect();
            let want = (sk.leaves.len() / 40).clamp(4, 14).min(late.len());
            for k in 0..want {
                let i = late[k * late.len() / want.max(1)];
                seedling[i] = Some((k as u32, want as u32));
            }
        }
        let leaves = sk
            .leaves
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let dir = l.dir.normalize_or(Vec3::Y);
                let normal = l.normal.normalize_or(Vec3::Z);
                let axis = locate(&sk.stems[l.stem], l.pos).1;
                LeafInfo { axis, anchor: l.pos, side: normal.cross(dir).normalize_or(Vec3::X), dir, normal, seedling: seedling[i] }
            })
            .collect();
        let leader_axis = leader.map(|i| sk.stems[i].points.clone()).unwrap_or_default();
        let leaf_stems = sk.leaves.iter().map(|l| l.stem).collect();
        Grower { timing, base: m.pos.clone(), bind, parents, stem_base, leaves, leaf_stems, leader, leader_axis, trunk_pose: leader }
    }

    /// Every stem's pose at growth `g` (parents come before their children in a skeleton,
    /// but this does not rely on it).
    fn poses(&self, g: f32) -> Vec<Pose> {
        let n = self.stem_base.len();
        let mut out: Vec<Option<Pose>> = vec![None; n];
        fn pose(i: usize, g: f32, gr: &Grower, out: &mut [Option<Pose>]) -> Pose {
            if let Some(p) = out[i] {
                return p;
            }
            let scale = gr.timing.scale(i, g);
            let base = gr.stem_base[i];
            let at = match gr.parents[i] {
                None => base,
                Some((p, c)) => pose(p, g, gr, out).place(c, base),
            };
            let p = Pose { at, base, scale, thick: scale.powf(0.75) };
            out[i] = Some(p);
            p
        }
        (0..n).map(|i| pose(i, g, self, &mut out)).collect()
    }

    /// Vertex positions at growth `g` (1 = the mature mesh).
    pub fn positions(&self, g: f32) -> Vec<Vec3> {
        let g = g.clamp(0.0, 1.0);
        if g >= 1.0 {
            return self.base.clone();
        }
        let poses = self.poses(g);
        let tree = self.trunk_pose.map(|i| poses[i].scale).unwrap_or_else(|| g.powf(TRUNK_PACE));
        // seedling leaves: present from the start, gone at SEEDLING_END
        let seed_k = if g < SEEDLING_END { smooth(g / 0.03) * (1.0 - smooth((g - (SEEDLING_END - 0.12)) / 0.12)) } else { 0.0 };
        let mut out = Vec::with_capacity(self.base.len());
        for (v, b) in self.bind.iter().enumerate() {
            let p0 = self.base[v];
            let p = match *b {
                Bind::Fixed => p0 * tree,
                Bind::Stem { s, c } => poses[s as usize].place(c, p0),
                Bind::Leaf { l } => {
                    let li = &self.leaves[l as usize];
                    let d = p0 - li.anchor;
                    match (li.seedling, self.leader) {
                        (Some((k, n)), Some(ls)) if g < SEEDLING_END => {
                            // on the young shoot, spiralling up below its tip
                            let f = 1.0 - 0.35 * (k as f32 + 0.5) / n as f32;
                            let ax = &self.leader_axis;
                            let c = ax[0].lerp(*ax.last().unwrap(), f);
                            let at = poses[ls].place(c, c);
                            let phi = k as f32 * 2.399_963;
                            let dir = Vec3::new(phi.cos(), 0.9, phi.sin()).normalize();
                            let normal = (Vec3::Y - dir * dir.y).normalize_or(Vec3::X);
                            let side = normal.cross(dir);
                            let local = side * d.dot(li.side) + dir * d.dot(li.dir) + normal * d.dot(li.normal);
                            at + local * (seed_k * 0.6)
                        }
                        _ => {
                            let st = self.leaf_stem(l as usize);
                            let pose = poses[st];
                            let k = self.timing.leaf(l as usize, g) * (0.45 + 0.55 * pose.scale);
                            pose.place(li.axis, li.anchor) + d * k
                        }
                    }
                }
            };
            out.push(p);
        }
        out
    }

    fn leaf_stem(&self, l: usize) -> usize {
        self.leaf_stems[l]
    }

    /// The mesh at growth `g`: positions moved, everything else as in `m`.
    pub fn mesh_at(&self, m: &Mesh, g: f32) -> Mesh {
        let mut out = m.clone();
        out.pos = self.positions(g);
        out
    }
}

/// Everything an export needs: the keyframes and, per exported mesh, the positions there.
pub struct Animation {
    pub keys: Vec<f32>,
    /// Per mesh, per key: vertex positions.
    pub shapes: Vec<Vec<Vec<Vec3>>>,
}

/// Samples the growth of every mesh at the keyframes. `meshes` pairs each mesh with the
/// skeleton it was built from (an empty skeleton for the billboard).
pub fn animate(growth: &Growth, timing: &Timing, meshes: &[(&Mesh, &Skeleton)]) -> Animation {
    let keys = keyframes(growth);
    let shapes = meshes
        .iter()
        .map(|(m, sk)| {
            let gr = Grower::new(growth, sk, m, timing);
            keys.iter().map(|&g| gr.positions(g)).collect()
        })
        .collect();
    Animation { keys, shapes }
}

/// Stage clips: (name, from, to). "Grow" runs from nothing to the mature tree; the first
/// stage's clip grows from nothing to it, the others from the stage before.
pub fn clips(growth: &Growth) -> Vec<(String, f32, f32)> {
    let mut stages = growth.stages.clone();
    stages.sort_by(|a, b| a.at.total_cmp(&b.at));
    let mut out = vec![("Grow".to_string(), 0.0, 1.0)];
    let mut prev: Option<(String, f32)> = None;
    for st in &stages {
        let at = st.at.clamp(0.0, 1.0);
        match &prev {
            None if at > 0.0 => out.push((st.name.clone(), 0.0, at)),
            Some((pn, pa)) if at > *pa => out.push((format!("{pn} to {}", st.name), *pa, at)),
            _ => {}
        }
        prev = Some((st.name.clone(), at));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grow::{build_mesh, grow};
    use crate::model::preset;

    #[test]
    fn grows_from_nothing_to_the_mature_tree() {
        let doc = preset("Oak").unwrap();
        let sk = grow(&doc, 0);
        let m = build_mesh(&doc, &sk, 0);
        let timing = Timing::new(&sk);
        assert!(timing.leaf_start.iter().all(|s| s + timing.leaf_time <= 1.0 + 1e-4));
        let gr = Grower::new(&doc.growth, &sk, &m, &timing);
        // the mature tree is the mesh as designed, and very close to it just before
        assert_eq!(gr.positions(1.0), m.pos);
        let late = gr.positions(0.9999);
        assert!(late.iter().zip(&m.pos).all(|(a, b)| a.distance(*b) < 0.01));
        // height of what is visible (parts not grown yet are collapsed to points)
        let h = |p: &[Vec3]| {
            m.tris.iter().filter(|t| (p[t[1] as usize] - p[t[0] as usize]).cross(p[t[2] as usize] - p[t[0] as usize]).length() > 1e-7).flat_map(|t| t.iter()).fold(0.0f32, |a, &i| a.max(p[i as usize].y))
        };
        let hs: Vec<f32> = [0.0, 0.12, 0.38, 0.68, 0.999].iter().map(|&g| h(&gr.positions(g))).collect();
        // grows taller all the way, from nothing to a sprout to the tree
        assert!(hs.windows(2).all(|w| w[0] <= w[1] + 1e-4), "{hs:?}");
        assert!(hs[0] < 0.01 * m.height, "{hs:?}");
        assert!(hs[1] < 0.15 * m.height, "{hs:?}");
        // leaves: a few at the sprout (seedling leaves), many on the sapling, all on the mature tree
        let shown = |p: &[Vec3]| (0..m.leaf_first.len()).filter(|&i| {
            let a = m.leaf_first[i] as usize;
            p[a].distance(p[a + 2]) > 1e-4
        }).count();
        let n = m.leaf_first.len();
        let (sprout, sapling) = (shown(&gr.positions(0.12)), shown(&gr.positions(0.38)));
        assert!(sprout > 0 && sprout < n / 10, "{sprout} of {n}");
        assert!(sapling > n / 4, "{sapling} of {n}");
    }

    #[test]
    fn keyframes_hold_the_stages() {
        let g = Growth::default();
        let k = keyframes(&g);
        assert_eq!(*k.first().unwrap(), 0.0);
        assert_eq!(*k.last().unwrap(), 1.0);
        for st in &g.stages {
            assert!(k.contains(&st.at));
        }
        assert!(k.contains(&SEEDLING_END));
        assert!(k.windows(2).all(|w| w[0] < w[1]));
        let c = clips(&g);
        assert_eq!(c[0].0, "Grow");
        assert_eq!(c[1], ("Sprout".to_string(), 0.0, 0.12));
        assert_eq!(c[2].0, "Sprout to Sapling");
        assert_eq!(c.len(), 5);
    }
}
