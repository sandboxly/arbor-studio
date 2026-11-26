//! Procedural growth: document -> skeleton (stems + leaves) -> triangle mesh.

use crate::model::*;
use glam::Vec3;
use std::collections::HashMap;
use serde::Serialize;
use std::f32::consts::{PI, TAU};

/// Small deterministic random generator (splitmix64).
pub struct Rng(u64);

impl Rng {
    pub fn new(parts: &[u64]) -> Self {
        let mut h = 0x9E37_79B9_7F4A_7C15u64;
        for &p in parts {
            h ^= p.wrapping_add(0x9E37_79B9_7F4A_7C15).wrapping_add(h << 6).wrapping_add(h >> 2);
            h = splitmix(h);
        }
        Rng(h)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        splitmix(self.0)
    }
    /// 0..1
    pub fn f(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    /// -1..1
    pub fn s(&mut self) -> f32 {
        self.f() * 2.0 - 1.0
    }
    pub fn unit(&mut self) -> Vec3 {
        loop {
            let v = Vec3::new(self.s(), self.s(), self.s());
            let l = v.length_squared();
            if l > 1e-4 && l <= 1.0 {
                return v / l.sqrt();
            }
        }
    }
}

fn splitmix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[derive(Clone, Serialize)]
pub struct Stem {
    pub gen: u32,
    pub depth: u32,
    pub parent: Option<usize>,
    pub points: Vec<Vec3>,
    pub radii: Vec<f32>,
    pub length: f32,
    #[serde(skip)]
    pub phase: f32,
    /// Roots (and sub-roots) follow the ground instead of growing freely.
    pub root: bool,
    /// Trunks: world azimuths (radians) of the buttress ridges at the base.
    #[serde(skip)]
    pub lobes: Vec<f32>,
    /// Stable id of this branch instance, used for hand edits (node editing).
    pub key: u32,
    /// Broken-off stub: blunt end, nothing grows on it.
    pub broken: bool,
    /// A leader from a split of its parent (the same generator's stem, split in two or more).
    #[serde(rename = "split")]
    pub fork: bool,
    /// The leaders this stem forks into (indices), and the fraction of its length where it
    /// forks (1 = no fork). What grows above the fork spreads over the stem and its leaders.
    #[serde(skip)]
    pub forks: Vec<usize>,
    #[serde(skip)]
    pub fork_t: f32,
    /// Length (metres) of the crotch below and above the fork where nothing grows.
    #[serde(skip)]
    pub fork_gap: f32,
}

#[derive(Clone, Serialize)]
pub struct Leaf {
    pub gen: u32,
    pub stem: usize,
    pub pos: Vec3,
    pub dir: Vec3,
    pub normal: Vec3,
    pub size: f32,
    pub aspect: f32,
    pub shape: LeafShape,
    pub color: [f32; 3],
    /// Material id (0 = default leaf look).
    pub mat: u32,
    /// Fold angle of the halves (radians) and tip curl.
    pub fold: f32,
    pub bend: f32,
}

#[derive(Default, Clone, Serialize)]
pub struct Skeleton {
    pub stems: Vec<Stem>,
    pub leaves: Vec<Leaf>,
}

const MAX_STEMS: usize = 30_000;
const MAX_LEAVES: usize = 60_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lod {
    pub sides: f32,
    pub segments: f32,
    pub leaves: f32,
    pub leaf_size: f32,
    /// Share of the finest stems (twigs first) left out, with what grows on them.
    pub prune: f32,
}

pub fn lod(level: u32) -> Lod {
    match level {
        0 => Lod { sides: 1.0, segments: 1.0, leaves: 1.0, leaf_size: 1.0, prune: 0.0 },
        1 => Lod { sides: 0.65, segments: 0.7, leaves: 0.6, leaf_size: 1.29, prune: 0.0 },
        2 => Lod { sides: 0.45, segments: 0.5, leaves: 0.35, leaf_size: 1.69, prune: 0.0 },
        // leaves grow by 1 / sqrt(share kept), so the crown keeps its leaf area
        _ => Lod { sides: 0.25, segments: 0.3, leaves: 0.15, leaf_size: 2.58, prune: 0.0 },
    }
}

/// The least stem detail a triangle budget may reduce a level to (reduction 1).
const FLOOR: Lod = Lod { sides: 0.12, segments: 0.15, leaves: 0.04, leaf_size: 5.0, prune: 0.0 };
/// At reduction 2 (the most a budget may take away) only this share of the leaves is left and
/// this share of the stems, the finest ones, is left out.
const FLOOR_LEAVES: f32 = 0.006;
const MAX_PRUNE: f32 = 0.9;

/// Detail between a level's usual amount (`s` = 0) and the least detail allowed, for fitting a
/// level into a triangle budget. From 0 to 1 the stems get fewer sides and rings and the leaves
/// thin out; from 1 to 2 the leaves thin out further and the finest twigs (with their leaves)
/// are left out, smallest first, which is what a distant tree loses first. Kept leaves grow by
/// 1 / sqrt(share kept), so the crown keeps its leaf area.
pub fn lod_blend(level: u32, s: f32) -> Lod {
    let a = lod(level.min(3));
    let s = s.clamp(0.0, 2.0);
    if s > 1.0 {
        let u = s - 1.0;
        let leaves = FLOOR.leaves * (FLOOR_LEAVES / FLOOR.leaves).powf(u);
        return Lod { sides: FLOOR.sides, segments: FLOOR.segments, leaves, leaf_size: 1.0 / leaves.sqrt(), prune: MAX_PRUNE * u };
    }
    let mix = |x: f32, y: f32| x + (y - x) * s;
    let leaves = mix(a.leaves, FLOOR.leaves);
    Lod { sides: mix(a.sides, FLOOR.sides), segments: mix(a.segments, FLOOR.segments), leaves, leaf_size: 1.0 / leaves.sqrt(), prune: 0.0 }
}

/// Leave out the finest stems: a share `prune` of the stems that are not trunks or a trunk's
/// split leaders, smallest (base radius × length) first, each with everything growing on it.
/// Returns the share of the leaves kept.
fn prune_finest(doc: &Document, sk: &mut Skeleton, prune: f32) -> f32 {
    let n = sk.stems.len();
    let structural = |i: usize| {
        let st = &sk.stems[i];
        st.parent.is_none() || (st.fork && st.parent.is_some_and(|p| sk.stems[p].parent.is_none()))
    };
    // SHED stems (`growth.shed`, `grow_young`) are neither ranked nor cut: the grown tree's
    // levels leave out exactly the twigs they leave out without them (parents come first).
    let mut shed = vec![false; n];
    for i in 0..n {
        shed[i] = doc.get(sk.stems[i].gen).is_some_and(|g| doc.is_shed(g)) || sk.stems[i].parent.is_some_and(|p| shed[p]);
    }
    let mut cand: Vec<usize> = (0..n).filter(|&i| !structural(i) && !shed[i]).collect();
    if cand.is_empty() || prune <= 0.0 {
        return 1.0;
    }
    cand.sort_by(|&a, &b| {
        let sa = sk.stems[a].radii[0] * sk.stems[a].length;
        let sb = sk.stems[b].radii[0] * sk.stems[b].length;
        sa.total_cmp(&sb)
    });
    let cut = ((cand.len() as f32) * prune.clamp(0.0, 1.0)).round() as usize;
    let mut gone = vec![false; n];
    for &i in &cand[..cut] {
        gone[i] = true;
    }
    // parents come before their children, so one pass carries removal down the hierarchy
    for i in 0..n {
        if let Some(p) = sk.stems[i].parent {
            if gone[p] {
                gone[i] = true;
            }
        }
    }
    let mut map = vec![usize::MAX; n];
    let mut stems = Vec::with_capacity(n);
    for (i, st) in sk.stems.drain(..).enumerate() {
        if !gone[i] {
            map[i] = stems.len();
            stems.push(st);
        }
    }
    for st in &mut stems {
        st.parent = st.parent.map(|p| map[p]);
        st.forks = st.forks.iter().filter(|&&f| map[f] != usize::MAX).map(|&f| map[f]).collect();
    }
    sk.stems = stems;
    let kept = |l: &Leaf| !shed[l.stem] && !doc.get(l.gen).is_some_and(|g| doc.is_shed(g));
    let before = sk.leaves.iter().filter(|l| kept(l)).count().max(1);
    let after = sk.leaves.iter().filter(|l| kept(l) && !gone[l.stem]).count();
    sk.leaves.retain(|l| !gone[l.stem]);
    for l in &mut sk.leaves {
        l.stem = map[l.stem];
    }
    after as f32 / before as f32
}

/// Sides around a stem: (full detail, at this LOD). Trunks keep at least five sides at low
/// detail so they stay round enough from every direction.
fn ring_sides(gpar: &GenParams, root_lobes: Option<usize>, trunk: bool, lodf: &Lod) -> (usize, usize) {
    let gp = if gpar.flutes > 0 { gpar.sides.max(gpar.flutes * 4).min(64) } else { gpar.sides };
    // root-driven flutes need a few sides per gap between roots
    let gp = if let Some(nl) = root_lobes { gp.max(nl as u32 * 4).min(64) } else { gp } as usize;
    let gp = gp.max(3);
    let min = if trunk && lodf.sides < 1.0 { 5 } else { 3 };
    let sides = ((gp as f32 * lodf.sides).round() as usize).max(min).min(gp);
    (gp, sides)
}

/// Average width of a regular polygon with `n` corners on a unit circle, relative to the
/// circle's (2): fewer sides look thinner unless the radius grows by its inverse.
fn polygon_width(n: usize) -> f32 {
    let n = n as f32;
    n * (PI / n).sin() / PI
}

/// Lower levels of detail are the full tree with less detail, never a different tree: the
/// same skeleton (every branch where it was), with fewer rings along each stem (the rings that
/// shape it least go first), fewer sides around it (radius widened so it keeps its apparent
/// thickness) and fewer, larger leaves picked evenly from the full set (each grows around its
/// own centre, so the crown keeps its outline and its coverage).
pub fn reduce_detail(doc: &Document, sk: &mut Skeleton, lodf: &Lod) {
    let defaults = GenParams::default();
    let mut leaf_grow = 1.0;
    if lodf.prune > 0.0 {
        // the leaves of left-out twigs: the others grow a little more to keep the crown's area
        leaf_grow = 1.0 / prune_finest(doc, sk, lodf.prune).max(0.05).sqrt();
    }
    let keep_rings = joint_rings(doc, sk);
    for si in 0..sk.stems.len() {
        let st = &sk.stems[si];
        let trunk = st.depth == 0 && !st.root;
        let root_lobes = (trunk && sk.stems.iter().any(|c| c.parent == Some(si) && c.root)).then(|| st.lobes.len());
        let gpar = doc.get(st.gen).map(|g| &g.params).unwrap_or(&defaults);
        let (gp, sides) = ring_sides(gpar, root_lobes, trunk, lodf);
        let widen = polygon_width(gp) / polygon_width(sides);
        let st = &mut sk.stems[si];
        let n = st.points.len();
        let keep = ((((n - 1) as f32) * lodf.segments).round() as usize + 1).max(if trunk { 4 } else { 2 });
        // the rings that shape a joint (a branch's base, the parent's bark around it) stay, or
        // the joint can't close and leaves stray faces
        let mut fixed = keep_rings[si].clone();
        while st.points.len() > keep.max(2) {
            let mut best = (f32::MAX, 0usize);
            for i in 1..st.points.len() - 1 {
                if fixed[i] {
                    continue;
                }
                let (a, b, p) = (st.points[i - 1], st.points[i + 1], st.points[i]);
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
                let rr = st.radii[i - 1] + (st.radii[i + 1] - st.radii[i - 1]) * t;
                let e = p.distance(a + ab * t) + (st.radii[i] - rr).abs();
                if e < best.0 {
                    best = (e, i);
                }
            }
            if best.0 == f32::MAX {
                break;
            }
            st.points.remove(best.1);
            st.radii.remove(best.1);
            fixed.remove(best.1);
        }
        if widen != 1.0 {
            for r in &mut st.radii {
                *r *= widen;
            }
        }
    }
    if lodf.leaves < 1.0 || leaf_grow != 1.0 {
        let mut acc = 0.5f32;
        sk.leaves.retain(|_| {
            acc += lodf.leaves;
            if acc >= 1.0 {
                acc -= 1.0;
                true
            } else {
                false
            }
        });
        for l in &mut sk.leaves {
            let old = l.size;
            l.size *= lodf.leaf_size * leaf_grow;
            // grow around the leaf's centre, not outwards from its stalk
            l.pos -= l.dir * ((l.size - old) * 0.5);
        }
    }
}

/// Per stem and point: whether the point is a ring a joint needs when the tree is joined into
/// one surface: the densified rings at a branch's base (up to where it leaves the parent's bark,
/// with its weld zone) and the parent's rings across each branch's footprint.
fn joint_rings(doc: &Document, sk: &Skeleton) -> Vec<Vec<bool>> {
    let mut out: Vec<Vec<bool>> = sk.stems.iter().map(|s| vec![false; s.points.len()]).collect();
    if !doc.settings.join_branches {
        return out;
    }
    let arcs: Vec<Vec<f32>> = sk
        .stems
        .iter()
        .map(|s| {
            let mut a = vec![0.0f32];
            for w in s.points.windows(2) {
                a.push(a.last().unwrap() + w[0].distance(w[1]));
            }
            a
        })
        .collect();
    for (ci, c) in sk.stems.iter().enumerate() {
        let Some(pi) = c.parent else { continue };
        if c.points.len() < 2 || c.fork {
            continue;
        }
        let ps = &sk.stems[pi];
        if ps.points.len() < 2 {
            continue;
        }
        let weld = doc.get(c.gen).map(|g| g.params.weld).unwrap_or(0.0);
        let (_, pr, _) = axis_info(ps, c.points[0]);
        // the branch's own base: everything inside the parent plus the weld zone
        let reach = c.radii[0] * 2.0 + pr * (0.4 + 1.4 * weld) + pr;
        // and up to the first two rings fully outside the parent's bark (a steep branch runs
        // inside its parent for a while)
        let first_out = (1..c.points.len())
            .find(|&i| {
                let (d, r, _) = axis_info(ps, c.points[i]);
                d - c.radii[i] > r * 1.15
            })
            .unwrap_or(c.points.len() - 1);
        for (i, &d) in arcs[ci].iter().enumerate() {
            if d <= reach || i <= first_out + 1 {
                out[ci][i] = true;
            }
        }
        // the parent's rings across the footprint
        let p0 = c.points[0];
        let (mut best, mut at, mut tan) = (f32::MAX, 0.0f32, Vec3::Y);
        for i in 0..ps.points.len() - 1 {
            let ab = ps.points[i + 1] - ps.points[i];
            let l = ab.length().max(1e-6);
            let t = ((p0 - ps.points[i]).dot(ab) / (l * l)).clamp(0.0, 1.0);
            let d = p0.distance(ps.points[i] + ab * t);
            if d < best {
                best = d;
                at = arcs[pi][i] + t * l;
                tan = ab / l;
            }
        }
        let cdir = (c.points[1] - c.points[0]).normalize_or(tan);
        let sin = (cdir - tan * cdir.dot(tan)).length().max(0.3);
        let h = c.radii[0] * (1.08 + 0.8 * weld) * 1.2 / sin + c.radii[0] * 0.15;
        let h = h * 1.7;
        for (i, &d) in arcs[pi].iter().enumerate() {
            if (d - at).abs() <= h {
                out[pi][i] = true;
            }
        }
    }
    out
}

impl Stem {
    /// Position, tangent and radius at fraction `t` of the stem.
    pub fn sample(&self, t: f32) -> (Vec3, Vec3, f32) {
        // by arc length, so extra rings (welding) do not shift what grows on the stem
        let n = self.points.len() - 1;
        let total: f32 = self.points.windows(2).map(|w| w[0].distance(w[1])).sum();
        let mut want = t.clamp(0.0, 1.0) * total;
        let (mut i, mut f) = (n - 1, 1.0);
        for k in 0..n {
            let seg = self.points[k].distance(self.points[k + 1]);
            if want <= seg || k == n - 1 {
                i = k;
                f = if seg > 0.0 { (want / seg).clamp(0.0, 1.0) } else { 0.0 };
                break;
            }
            want -= seg;
        }
        let a = self.points[i];
        let b = self.points[i + 1];
        let tan = (b - a).normalize_or(Vec3::Y);
        (a.lerp(b, f), tan, self.radii[i] + (self.radii[i + 1] - self.radii[i]) * f)
    }
}

fn perp_basis(t: Vec3) -> (Vec3, Vec3) {
    let r = if t.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let b1 = t.cross(r).normalize();
    let b2 = t.cross(b1).normalize();
    (b1, b2)
}

/// Roots thin out quickly near the trunk, then stay slender: a curved taper.
fn root_taper(tip: f32, t: f32) -> f32 {
    tip + (1.0 - tip) * (1.0 - t).powf(1.5)
}

/// Stable 32-bit key for a branch instance: survives regrowing as long as the
/// hierarchy and counts above it stay the same.
pub fn node_key(parent: u32, gen: u32, k: u32) -> u32 {
    (Rng::new(&[parent as u64, gen as u64, k as u64, 0x5EED]).next_u64() >> 32) as u32
}

/// Everything growth needs that doesn't change while growing.
struct Ctx<'a> {
    doc: &'a Document,
    lodf: Lod,
    seed: u64,
    edits: HashMap<u32, &'a NodeEdit>,
    /// Expected tree height (metres), for curves measured along the tree's height.
    tree_h: f32,
}

impl Ctx<'_> {
    /// Height of a point as a fraction of the tree's height (0 at the ground).
    fn hf(&self, p: Vec3) -> f32 {
        (p.y / self.tree_h.max(0.01)).clamp(0.0, 1.0)
    }
}

/// Vertex layout of one stem's tube: `n` rings of `sides + 1` vertices (seam duplicated).
#[derive(Clone, Copy)]
struct Grid {
    base: u32,
    ring: u32,
    sides: u32,
    n: u32,
    /// First ring that is drawn (rings before it are inside the parent).
    start: u32,
    tip: u32,
    /// Base wrapped onto the parent (weld join): no base cap.
    welded: bool,
}

impl Grid {
    fn v(&self, j: u32, k: u32) -> u32 {
        self.base + j * self.ring + (k % self.sides)
    }
}

/// Distance from `q` to a stem's axis and the stem radius at the closest point.
fn axis_info(st: &Stem, q: Vec3) -> (f32, f32, Vec3) {
    let mut best = (f32::MAX, st.radii[0], st.points[0]);
    for i in 0..st.points.len() - 1 {
        let (a, b) = (st.points[i], st.points[i + 1]);
        let ab = b - a;
        let t = ((q - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
        let c = a + ab * t;
        let d = q.distance(c);
        if d < best.0 {
            best = (d, st.radii[i] + (st.radii[i + 1] - st.radii[i]) * t, c);
        }
    }
    best
}

/// Radius of a stem's actual bark surface (buttress flare, flutes, lumps included) in the
/// direction of `q`, read from its built ring vertices.
fn surface_radius(m: &Mesh, ps: &Stem, gp: &Grid, q: Vec3) -> f32 {
    let n = ps.points.len().min(gp.n as usize);
    if n < 2 {
        return ps.radii[0];
    }
    let mut best = (f32::MAX, 0usize, 0.0f32);
    for i in 0..n - 1 {
        let (a, b) = (ps.points[i], ps.points[i + 1]);
        let ab = b - a;
        let t = ((q - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
        let d = q.distance_squared(a + ab * t);
        if d < best.0 {
            best = (d, i, t);
        }
    }
    let (_, i, t) = best;
    let mut r = 0.0;
    for (j, w) in [(i, 1.0 - t), (i + 1, t)] {
        let c = ps.points[j];
        let dq = (q - c).normalize_or(Vec3::X);
        let mut pick = (-2.0f32, ps.radii[j]);
        for k in 0..gp.sides {
            let v = m.pos[gp.v(j as u32, k) as usize] - c;
            let dd = v.normalize_or(Vec3::ZERO).dot(dq);
            if dd > pick.0 {
                pick = (dd, v.length());
            }
        }
        r += pick.1 * w;
    }
    r
}

/// 0 at a root's azimuth, 1 midway between two neighbouring roots.
fn root_groove(sorted: &[f32], az: f32) -> f32 {
    if sorted.len() < 2 {
        return 0.0;
    }
    let a = az.rem_euclid(TAU);
    let n = sorted.len();
    let next = sorted.iter().position(|&x| x >= a).unwrap_or(n);
    let (prev, nxt) = if next == 0 || next == n { (sorted[n - 1] - if next == 0 { TAU } else { 0.0 }, sorted[0] + if next == n { TAU } else { 0.0 }) } else { (sorted[next - 1], sorted[next]) };
    let gap = (nxt - prev).max(1e-4);
    (PI * ((a - prev) / gap).clamp(0.0, 1.0)).sin()
}

/// Copy of the skeleton where trunks with roots reach below their base, so roots can join
/// the trunk's side even when they leave its bottom pointing down. Returns the extension
/// length per stem (0 for the others).
fn extend_trunk_bases(sk: &Skeleton) -> (Skeleton, Vec<f32>) {
    let mut out = sk.clone();
    let mut below = vec![0.0f32; sk.stems.len()];
    for (i, st) in sk.stems.iter().enumerate() {
        if st.depth != 0 || st.root || st.points.len() < 2 {
            continue;
        }
        let root_r = sk.stems.iter().filter(|c| c.parent == Some(i) && c.root).map(|c| c.radii[0]).fold(0.0f32, f32::max);
        if root_r <= 0.0 {
            continue;
        }
        // just deep enough for the roots' undersides, and rounded off (no stump below ground)
        let ext = (root_r * 1.4).min(st.radii[0] * 1.6);
        let tan = (st.points[1] - st.points[0]).normalize_or(Vec3::Y);
        let steps = 5;
        let mut pts: Vec<Vec3> = vec![];
        let mut rad: Vec<f32> = vec![];
        for k in (1..=steps).rev() {
            let d = k as f32 / steps as f32;
            pts.push(st.points[0] - tan * (ext * d));
            // carries on below the base and rounds off like the end of an ellipsoid, so the
            // roots grow out of its side (no lip where the flared base used to tuck in)
            rad.push(st.radii[0] * (0.04 + 0.96 * (1.0 - d * d).max(0.0).sqrt()));
        }
        pts.extend_from_slice(&st.points);
        rad.extend_from_slice(&st.radii);
        out.stems[i].points = pts;
        out.stems[i].radii = rad;
        below[i] = ext;
    }
    (out, below)
}

/// Closest point on a stem's axis to `q`: (point, tangent, radius).
fn axis_frame(st: &Stem, q: Vec3) -> (Vec3, Vec3, f32) {
    let mut best = (f32::MAX, st.points[0], Vec3::Y, st.radii[0]);
    for i in 0..st.points.len() - 1 {
        let (a, b) = (st.points[i], st.points[i + 1]);
        let ab = b - a;
        let t = ((q - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
        let c = a + ab * t;
        let d = q.distance_squared(c);
        if d < best.0 {
            best = (d, c, ab.normalize_or(Vec3::Y), st.radii[i] + (st.radii[i + 1] - st.radii[i]) * t);
        }
    }
    (best.1, best.2, best.3)
}

/// Weld join: keep the parent whole and wrap the branch's base onto the parent's surface.
/// The rings inside the parent become a curved fillet (quadratic curve from a footprint on the
/// bark, through the point where the branch surface meets the bark, to the first ring outside).
/// The footprint takes the parent's normal, colour and wind, and the fillet blends them into
/// the branch's, so the seam is invisible in shading and the joint never separates in wind.
#[allow(clippy::too_many_arguments)]
fn weld_onto_parent(m: &mut Mesh, sk: &Skeleton, grids: &[Grid], margin: f32, weld: f32, si: usize, pi: usize) -> bool {
    let st = &sk.stems[si];
    let ps = &sk.stems[pi];
    let (gc, gp) = (grids[si], grids[pi]);
    if gc.n < 3 {
        return false;
    }
    let Some(s) = (1..gc.n - 1).find(|&i| {
        (0..gc.sides).all(|k| {
            let q = m.pos[gc.v(i, k) as usize];
            let (d, _, _) = axis_info(ps, q);
            d > surface_radius(m, ps, &gp, q) * 1.03
        })
    }) else {
        return false;
    };
    let p0 = st.points[0];
    let ring_c = st.points[s as usize];
    let dir = (ring_c - p0).normalize_or(Vec3::Y);
    let tc = (st.points[(s as usize + 1).min(st.points.len() - 1)] - ring_c).normalize_or(dir);
    let rc = st.radii[s as usize];
    let spread = rc * (0.2 + 1.3 * weld);
    let lift = margin - 1.035;
    let perp = |w: Vec3, t: Vec3| w - t * w.dot(t);
    // parent attributes near a point: nearest parent tube vertex
    let nearest_parent = |m: &Mesh, q: Vec3| -> usize {
        let mut best = (f32::MAX, gp.base as usize);
        for j in 0..gp.n {
            for k in 0..gp.sides {
                let i = gp.v(j, k) as usize;
                let d = m.pos[i].distance_squared(q);
                if d < best.0 {
                    best = (d, i);
                }
            }
        }
        best.1
    };
    for k in 0..=gc.sides {
        let si_v = (gc.base + s * gc.ring + k) as usize;
        let sv = m.pos[si_v];
        // where this line of the branch surface, followed back, meets the parent's bark
        let (a, tp, rp) = axis_frame(ps, sv);
        let (u, v) = (perp(sv - a, tp), perp(tc, tp));
        let (qa, qb, qc) = (v.dot(v), -2.0 * u.dot(v), u.dot(u) - rp * rp);
        let disc = qb * qb - 4.0 * qa * qc;
        let x = if qa > 1e-6 && disc >= 0.0 {
            let t = (-qb - disc.sqrt()) / (2.0 * qa);
            let t = if t > 0.0 { t } else { (-qb + disc.sqrt()) / (2.0 * qa) };
            if t > 0.0 { sv - tc * t } else { a + u.normalize_or(Vec3::X) * rp }
        } else {
            a + u.normalize_or(Vec3::X) * rp
        };
        // onto the real bark surface (buttress flare, flutes and lumps included)
        let (a2, tp2, _) = axis_frame(ps, x);
        let radial = perp(x - a2, tp2).normalize_or(Vec3::X);
        let x = a2 + radial * surface_radius(m, ps, &gp, x).max(rp);
        // spread the footprint outwards over the bark, then put it back on the surface
        let w = x - p0;
        let away = perp(w - dir * w.dot(dir), radial).normalize_or(Vec3::ZERO);
        let xs = x + away * spread;
        let (a3, tp3, rp3) = axis_frame(ps, xs);
        let n_parent = perp(xs - a3, tp3).normalize_or(radial);
        let foot = a3 + n_parent * surface_radius(m, ps, &gp, xs).max(rp3) * 1.006;
        let pv = nearest_parent(m, foot);
        let (p_col, p_wind, p_phase) = (m.col[pv], m.wind[pv], m.phase[pv]);
        let (c_nrm, c_col, c_wind) = (m.nrm[si_v], m.col[si_v], m.wind[si_v]);
        let corner = x + n_parent * rp * lift.min(0.02);
        for i in 0..s {
            let t = i as f32 / s as f32;
            let vi = (gc.base + i * gc.ring + k) as usize;
            // quadratic curve: footprint -> corner on the bark -> first ring outside
            m.pos[vi] = foot * (1.0 - t) * (1.0 - t) + corner * (2.0 * t * (1.0 - t)) + sv * (t * t);
            let e = t * t * (3.0 - 2.0 * t);
            m.nrm[vi] = (n_parent * (1.0 - e) + c_nrm * e).normalize_or(c_nrm);
            for c in 0..3 {
                m.col[vi][c] = p_col[c] * (1.0 - e) + c_col[c] * e;
            }
            for c in 0..2 {
                m.wind[vi][c] = p_wind[c] * (1.0 - e) + c_wind[c] * e;
            }
            if i == 0 {
                m.phase[vi] = p_phase;
            }
        }
    }
    true
}

/// A new vertex blended from existing ones (used for openings cut into a parent's bark).
fn push_blend(m: &mut Mesh, idx: [u32; 4], w: [f32; 4]) -> u32 {
    let mut pos = Vec3::ZERO;
    let mut nrm = Vec3::ZERO;
    let (mut col, mut uv, mut wind) = ([0.0f32; 3], [0.0f32; 2], [0.0f32; 2]);
    for (i, &k) in idx.iter().enumerate() {
        let k = k as usize;
        pos += m.pos[k] * w[i];
        nrm += m.nrm[k] * w[i];
        for c in 0..3 {
            col[c] += m.col[k][c] * w[i];
        }
        for c in 0..2 {
            uv[c] += m.uv[k][c] * w[i];
            wind[c] += m.wind[k][c] * w[i];
        }
    }
    let s = idx[0] as usize;
    m.pos.push(pos);
    m.nrm.push(nrm.normalize_or(m.nrm[s]));
    m.col.push(col);
    m.uv.push(uv);
    m.wind.push(wind);
    m.phase.push(m.phase[s]);
    m.gen.push(m.gen[s]);
    m.vmat.push(m.vmat[s]);
    m.stem.push(m.stem[s]);
    m.leaf.push(false);
    m.pos.len() as u32 - 1
}

/// Join a branch to its parent as one surface: cut an opening into the parent's bark where the
/// branch comes out, drop the branch rings that are inside the parent, and stitch the opening's
/// rim to the first branch ring outside. Returns that ring, or None (kept as a separate tube).
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn join_branch(m: &mut Mesh, sk: &Skeleton, grids: &[Grid], removed: &mut [Vec<bool>], _margin: f32, weld: f32, si: usize, pi: usize, xfade: bool) -> Option<u32> {
    let st = &sk.stems[si];
    let ps = &sk.stems[pi];
    let (gc, gp) = (grids[si], grids[pi]);
    if gc.n < 3 || gp.n < 3 {
        return None;
    }
    // first ring of the branch that lies completely outside the parent
    let s = (1..gc.n - 1).find(|&i| {
        (0..gc.sides).all(|k| {
            let q = m.pos[gc.v(i, k) as usize];
            let (d, _, _) = axis_info(ps, q);
            d > surface_radius(m, ps, &gp, q) * 1.03
        })
    })?;
    let p0 = st.points[0];
    let ring_c = st.points[s as usize];
    let dir = (ring_c - p0).normalize_or(Vec3::Y);
    let rc = st.radii[s as usize];
    // the opening encloses the whole branch base, plus a margin that welding widens into a fillet
    let rc_open = rc * (1.08 + 0.8 * weld);
    let line_dist = |q: Vec3| {
        let w = q - p0;
        (w - dir * w.dot(dir)).length()
    };
    // quad rows of the parent that may be opened (not its base row, not rows already skipped)
    let (lo, hi) = (gp.start, gp.n - 2);
    if lo > hi {
        return None;
    }
    let corner = |m: &Mesh, j: u32, k: u32| m.pos[gp.v(j, k) as usize];
    let centre = |m: &Mesh, j: u32, k: u32| (corner(m, j, k) + corner(m, j, k + 1) + corner(m, j + 1, k) + corner(m, j + 1, k + 1)) * 0.25;
    let qi = |j: u32, k: u32| (j * gp.sides + k % gp.sides) as usize;
    // Siblings growing close together (roots around a trunk base, whorls) would claim the same
    // bark. Each bark quad belongs to the sibling whose axis it is relatively closest to, so
    // neighbouring openings meet along a shared edge instead of overlapping.
    let approx = |c: &Stem| {
        let far = c.points[(3).min(c.points.len() - 1)];
        let d = (far - c.points[0]).normalize_or(Vec3::Y);
        (c.points[0], d, c.radii[0] * (1.08 + 0.8 * weld))
    };
    let score = |(o, d, r): (Vec3, Vec3, f32), q: Vec3| {
        let w = q - o;
        if w.dot(d) <= 0.0 {
            f32::MAX
        } else {
            (w - d * w.dot(d)).length() / r.max(1e-4)
        }
    };
    let me = approx(st);
    let others: Vec<(Vec3, Vec3, f32)> = sk.stems.iter().enumerate().filter(|(j, c)| *j != si && c.parent == Some(pi) && c.points.len() >= 2).map(|(_, c)| approx(c)).collect();
    let own = |q: Vec3| {
        let mine = score(me, q);
        others.iter().all(|&o| score(o, q) >= mine)
    };
    // seed: the parent quad the branch axis comes out through
    let mut seed = None;
    let mut best = f32::MAX;
    for j in lo..=hi {
        for k in 0..gp.sides {
            let c = centre(m, j, k);
            if (c - p0).dot(dir) <= 0.0 || removed[pi][qi(j, k)] || !own(c) {
                continue;
            }
            let d = line_dist(c);
            if d < best {
                best = d;
                seed = Some((j, k));
            }
        }
    }
    let (sj, sk0) = seed?;
    if best > rc_open * 2.0 + corner(m, sj, sk0).distance(corner(m, sj + 1, sk0 + 1)) {
        return None;
    }
    // The footprint follows the branch's real (possibly curving) base up to the first ring
    // outside the parent, not a straight line: roots bend sharply as they leave the trunk, and a
    // straight-line footprint left their upper side outside the opening (folded joints).
    let open_scale = rc_open / rc.max(1e-5);
    let tube_n = (s as usize + 1).min(st.points.len() - 1);
    // how far `q` lies inside the branch base tube, as a fraction of its local radius (<1 = inside)
    let tube_frac = |q: Vec3| {
        let mut best = f32::MAX;
        for i in 0..tube_n {
            let (a, b) = (st.points[i], st.points[i + 1]);
            let ab = b - a;
            let t = ((q - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
            let r = st.radii[i] + (st.radii[i + 1] - st.radii[i]) * t;
            best = best.min(q.distance(a + ab * t) / r.max(1e-5));
        }
        best
    };
    let in_footprint = |q: Vec3| (q - p0).dot(dir) > 0.0 && (line_dist(q) < rc_open || tube_frac(q) < open_scale);
    let mut tris: Vec<[u32; 3]> = vec![];
    // the opening's rim as one closed loop, counter-clockwise seen from outside (the direction
    // the removed quads ran along it); None if the region pinches or has several loops
    let boundary_loop = |region: &[(u32, u32)], inreg: &[bool]| -> Option<Vec<u32>> {
        let sides = gp.sides as i64;
        let mut next_of: std::collections::HashMap<(u32, u32), (u32, u32)> = std::collections::HashMap::new();
        for &(j, k) in region {
            let k1 = (k + 1) % gp.sides;
            let (ji, ki) = (j as i64, k as i64);
            let edges = [((j, k), (j, k1), (ji - 1, ki)), ((j, k1), (j + 1, k1), (ji, ki + 1)), ((j + 1, k1), (j + 1, k), (ji + 1, ki)), ((j + 1, k), (j, k), (ji, ki - 1))];
            for (a, b, nb) in edges {
                let inside = nb.0 >= 0 && nb.0 <= (gp.n - 2) as i64 && inreg[qi(nb.0 as u32, nb.1.rem_euclid(sides) as u32)];
                if !inside && next_of.insert(a, b).is_some() {
                    return None;
                }
            }
        }
        let start = *next_of.keys().next()?;
        let mut out = vec![gp.v(start.0, start.1)];
        let mut cur = *next_of.get(&start)?;
        while cur != start {
            out.push(gp.v(cur.0, cur.1));
            cur = *next_of.get(&cur)?;
            if out.len() > next_of.len() {
                return None;
            }
        }
        (out.len() == next_of.len()).then_some(out)
    };

    // 1) the whole branch cross-section fits inside one bark quad: cut an inset opening that
    //    encloses the footprint (the ellipse where the branch cylinder meets the bark)
    let (j, k) = (sj, sk0);
    let c = [gp.v(j, k), gp.v(j, k + 1), gp.v(j + 1, k + 1), gp.v(j + 1, k)];
    let c00 = m.pos[c[0] as usize];
    let eu = m.pos[c[1] as usize] - c00;
    let ev = m.pos[c[3] as usize] - c00;
    let nq = eu.cross(ev).normalize_or(dir);
    let cosn = dir.dot(nq).abs().max(0.2);
    let dn = dir.dot(nq);
    let tq = if dn.abs() > 1e-3 { (c00 - p0).dot(nq) / dn } else { 0.0 };
    let entry = p0 + dir * tq.max(0.0);
    let d_t = (dir - nq * dir.dot(nq)).normalize_or(eu.normalize_or(Vec3::X));
    let d_p = nq.cross(d_t);
    // half-extent of the footprint ellipse along a direction in the bark plane
    let extent = |e: Vec3| {
        let e = e.normalize_or(Vec3::X);
        ((rc_open / cosn * e.dot(d_t)).powi(2) + (rc_open * e.dot(d_p)).powi(2)).sqrt()
    };
    let du = extent(eu) / eu.length().max(1e-5);
    let dv = extent(ev) / ev.length().max(1e-5);
    let u = (entry - c00).dot(eu) / eu.length_squared().max(1e-10);
    let v = (entry - c00).dot(ev) / ev.length_squared().max(1e-10);
    let oct = 1.1 / (PI / 8.0).cos();
    let fits = u - du * oct > 0.03 && u + du * oct < 0.97 && v - dv * oct > 0.03 && v + dv * oct < 0.97;
    let mut region = vec![(sj, sk0)];
    let rim = if fits {
        let bil = |u: f32, v: f32| [(1.0 - u) * (1.0 - v), u * (1.0 - v), u * v, (1.0 - u) * v];
        // 8-sided opening, so the stitched collar follows the round branch closely
        let inset: Vec<u32> = (0..8)
            .map(|i| {
                let a = (i as f32 + 0.5) / 8.0 * TAU - PI;
                // corners of an octagon around the ellipse's bounding box, in quad (u,v)
                let (uu, vv) = (u + du * oct * a.cos(), v + dv * oct * a.sin());
                push_blend(m, c, bil(uu.clamp(0.0, 1.0), vv.clamp(0.0, 1.0)))
            })
            .collect();
        // fan from the quad's corners to the octagon; each corner serves the octagon edges
        // facing it, all with the bark's winding (corners run counter-clockwise like the inset)
        let corner_of = |i: usize| -> usize { i / 2 };
        for i in 0..8 {
            let (ia, ib) = (inset[i], inset[(i + 1) % 8]);
            let ca = c[corner_of(i)];
            let cb = c[corner_of((i + 1) % 8)];
            tris.push([ca, ib, ia]);
            if ca != cb {
                tris.push([ca, cb, ib]);
            }
        }
        inset
    } else {
        // 2) bigger branch: open every quad the branch cylinder touches, so every rim vertex
        //    lies outside the branch's base and the collar can only widen towards the bark
        let mut inreg = vec![false; removed[pi].len()];
        inreg[qi(sj, sk0)] = true;
        // sample the quad densely: bark quads can be tall near the base, and a root's upper
        // side passing through the lower part of one must still open it
        let touches = |m: &Mesh, j: u32, k: u32| {
            let (a, b, c, d) = (corner(m, j, k), corner(m, j, k + 1), corner(m, j + 1, k + 1), corner(m, j + 1, k));
            (0..=4).any(|x| {
                (0..=4).any(|y| {
                    let (u, v) = (x as f32 / 4.0, y as f32 / 4.0);
                    in_footprint(a * ((1.0 - u) * (1.0 - v)) + b * (u * (1.0 - v)) + c * (u * v) + d * ((1.0 - u) * v))
                })
            })
        };
        let mut limited = false;
        let mut q = 0;
        while q < region.len() {
            let (j, k) = region[q];
            q += 1;
            let nbs = [(j as i64 - 1, k as i64), (j as i64 + 1, k as i64), (j as i64, k as i64 - 1), (j as i64, k as i64 + 1)];
            for (nj, nk) in nbs {
                if nj < lo as i64 || nj > hi as i64 {
                    limited = true;
                    continue;
                }
                let (nj, nk) = (nj as u32, nk.rem_euclid(gp.sides as i64) as u32);
                let id = qi(nj, nk);
                if inreg[id] {
                    continue;
                }
                if removed[pi][id] {
                    // already opened for a sibling: the two openings share this edge
                    limited |= touches(m, nj, nk);
                    continue;
                }
                if touches(m, nj, nk) {
                    if !own(centre(m, nj, nk)) {
                        // a sibling's bark: the opening stops here and the two meet on this edge
                        limited = true;
                        continue;
                    }
                    inreg[id] = true;
                    region.push((nj, nk));
                }
            }
        }
        // a vertex inside the footprint must not end up on the rim: if the opening could not
        // grow around it (row limits or a neighbouring opening), keep this branch separate
        let rim = boundary_loop(&region, &inreg)?;
        if !limited && rim.iter().any(|&i| tube_frac(m.pos[i as usize]) < 0.98 && (m.pos[i as usize] - p0).dot(dir) > 0.0) {
            return None;
        }
        rim
    };
    for &(j, k) in &region {
        removed[pi][qi(j, k)] = true;
    }
    // stitch the rim loop to the branch ring. Both loops are walked in their own order (rim
    // counter-clockwise, ring with increasing k), so every triangle has the winding of the bark
    // around it and the strip closes without gaps; angles only pick the starting pair and pace.
    let next = st.points[(s as usize + 1).min(st.points.len() - 1)];
    let tc = (next - ring_c).normalize_or(dir);
    let b1 = tc.any_orthonormal_vector();
    let b2 = tc.cross(b1);
    let ang = |p: Vec3| {
        let w = p - ring_c;
        w.dot(b2).atan2(w.dot(b1))
    };
    let ring: Vec<u32> = (0..gc.sides).map(|k| gc.v(s, k)).collect();
    let a0 = ang(m.pos[ring[0] as usize]);
    let wrap = |x: f32| (x + PI).rem_euclid(TAU) - PI;
    let r0i = (0..rim.len()).min_by(|&x, &y| wrap(ang(m.pos[rim[x] as usize]) - a0).abs().total_cmp(&wrap(ang(m.pos[rim[y] as usize]) - a0).abs())).unwrap_or(0);
    let rim: Vec<u32> = (0..rim.len()).map(|i| rim[(r0i + i) % rim.len()]).collect();
    // pace both loops by the angle around the branch axis, so every rim vertex is joined to
    // the part of the ring right in front of it (no twisted, folded strips where the opening
    // is irregular, e.g. where two root openings meet). Falls back to arc length / even steps
    // if a loop does not wind once around the axis.
    let paced = |pts: &[u32], m: &Mesh| -> Vec<f32> {
        let n = pts.len();
        let mut acc = vec![0.0f32];
        let mut prev = ang(m.pos[pts[0] as usize]);
        let mut th = 0.0f32;
        for i in 1..=n {
            let a = ang(m.pos[pts[i % n] as usize]);
            th += wrap(a - prev);
            prev = a;
            acc.push(th);
        }
        let total = acc[n];
        if (total.abs() - TAU).abs() < 0.6 {
            // monotone envelope, normalised to 0..1
            let mut out = Vec::with_capacity(n + 1);
            let mut hi = 0.0f32;
            for &x in &acc {
                hi = hi.max(x / total);
                out.push(hi.min(1.0));
            }
            out
        } else {
            let mut acc = vec![0.0f32];
            for i in 0..n {
                let d = m.pos[pts[i] as usize].distance(m.pos[pts[(i + 1) % n] as usize]);
                acc.push(acc[i] + d);
            }
            let t = acc[n].max(1e-6);
            acc.iter().map(|x| x / t).collect()
        }
    };
    let fb = paced(&rim, m);
    let fr = paced(&ring, m);
    let (nb, nc) = (rim.len(), ring.len());
    // The collar strip between the opening and the branch is textured like the branch itself:
    // its rim gets its own copies of the bark vertices (same position, normal, colour and wind,
    // so nothing moves apart) carrying the branch's texture coordinates, continued from the
    // branch ring. Without this the strip would blend the parent's and the branch's unrelated
    // coordinates and smear the bark into zigzags. The ring's seam vertex (u = 1) closes it.
    // (gc.v wraps k around; the seam vertex k = sides is addressed directly)
    let ring_ext: Vec<u32> = (0..=gc.sides).map(|k| gc.base + s * gc.ring + k).collect();
    let child_mat = m.vmat[ring[0] as usize];
    // texture v per metre along the branch at the ring
    let kv = {
        let s1 = (s + 1).min(gc.n - 1);
        let d = st.points[s1 as usize].distance(st.points[s as usize]);
        let dv = m.uv[gc.v(s1, 0) as usize][1] - m.uv[gc.v(s, 0) as usize][1];
        if s1 != s && d > 1e-5 && dv > 0.0 { dv / d } else { 1.0 / (TAU * rc.max(0.004)) }
    };
    // texture position around the branch: the rim vertex's own angle (unwrapped, not the monotone
    // pacing, which would give runs of rim vertices the same u and stretch the bark between them)
    let raw: Vec<f32> = {
        let mut acc = vec![0.0f32];
        let mut prev = ang(m.pos[rim[0] as usize]);
        let mut th = 0.0f32;
        for i in 1..=nb {
            let a = ang(m.pos[rim[i % nb] as usize]);
            th += wrap(a - prev);
            prev = a;
            acc.push(th);
        }
        let total = acc[nb];
        if (total.abs() - TAU).abs() < 0.6 {
            acc.iter().map(|x| x / total).collect()
        } else {
            fb.clone()
        }
    };
    let rim_ext: Vec<u32> = (0..=nb)
        .map(|i| {
            let src = rim[i % nb] as usize;
            let f = raw[i].clamp(0.0, 1.0);
            // the point on the ring at the same place around the branch
            let j = (0..nc).find(|&j| fr[j + 1] >= f).unwrap_or(nc - 1);
            let t = ((f - fr[j]) / (fr[j + 1] - fr[j]).max(1e-6)).clamp(0.0, 1.0);
            let (qa, qb) = (ring_ext[j] as usize, ring_ext[j + 1] as usize);
            let q = m.pos[qa].lerp(m.pos[qb], t);
            let u = m.uv[qa][0] + (m.uv[qb][0] - m.uv[qa][0]) * t;
            let v = m.uv[qa][1] - m.pos[src].distance(q) * kv;
            // colour (material tint and baked occlusion) of the branch at that spot: the strip
            // belongs to the branch's material
            let (ca, cb) = (m.col[qa], m.col[qb]);
            let col = [0, 1, 2].map(|c| ca[c] + (cb[c] - ca[c]) * t);
            m.pos.push(m.pos[src]);
            m.nrm.push(m.nrm[src]);
            m.col.push(col);
            m.uv.push([u, v]);
            m.wind.push(m.wind[src]);
            m.phase.push(m.phase[src]);
            m.gen.push(m.gen[src]);
            m.vmat.push(child_mat);
            m.stem.push(m.stem[src]);
            m.leaf.push(false);
            m.pos.len() as u32 - 1
        })
        .collect();
    for &v in rim.iter().chain(rim_ext.iter()) {
        m.joint_rim.push((v, si as u32));
    }
    let (mut i, mut j) = (0usize, 0usize);
    while i < nb || j < nc {
        let tb = if i < nb { fb[i + 1] } else { f32::MAX };
        let tr = if j < nc { fr[j + 1] } else { f32::MAX };
        if tb <= tr {
            tris.push([rim_ext[i], rim_ext[i + 1], ring_ext[j]]);
            i += 1;
        } else {
            tris.push([rim_ext[i], ring_ext[j + 1], ring_ext[j]]);
            j += 1;
        }
    }
    m.tris.extend(tris);
    if xfade {
        joint_crossfade(m, sk, &gp, &gc, si, pi, s, &rim, &rim_ext, &ang);
    }
    Some(s)
}

/// Cross-fade the bark where a stitched branch meets its parent: the collar starts with the
/// parent's texture mapping (exactly the parent's coordinates at the opening, so there is no
/// seam) and fades into the branch's own mapping over a few branch radii. The parent's mapping
/// is extended off its surface by projecting onto its axis (angle around it, position along
/// it), corrected to match the opening's real coordinates.
#[allow(clippy::too_many_arguments)]
fn joint_crossfade(m: &mut Mesh, sk: &Skeleton, gp: &Grid, gc: &Grid, si: usize, pi: usize, s: u32, rim: &[u32], rim_ext: &[u32], ang: &dyn Fn(Vec3) -> f32) {
    let (st, ps) = (&sk.stems[si], &sk.stems[pi]);
    let n = ps.points.len().min(gp.n as usize);
    if n < 2 || rim.is_empty() {
        return;
    }
    let ring0 = gp.base as usize;
    let tiling_u = (m.uv[ring0 + 1][0] - m.uv[ring0][0]) * gp.sides as f32;
    if tiling_u.abs() < 1e-5 {
        return;
    }
    let wrap_u = |x: f32| (x + 0.5 * tiling_u).rem_euclid(tiling_u) - 0.5 * tiling_u;
    // the parent's mapping, projected: (u, v) of the point of its tube in line with `p`
    let pmap = |m: &Mesh, p: Vec3| -> [f32; 2] {
        let mut best = (f32::MAX, 0usize, 0.0f32);
        for i in 0..n - 1 {
            let (a, b) = (ps.points[i], ps.points[i + 1]);
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
            let d = p.distance_squared(a + ab * t);
            if d < best.0 {
                best = (d, i, t);
            }
        }
        let (_, i, t) = best;
        let (a, b) = (ps.points[i], ps.points[i + 1]);
        let ta = (b - a).normalize_or(Vec3::Y);
        let c = a.lerp(b, t);
        let flat = |q: Vec3| (q - a) - ta * (q - a).dot(ta);
        let b1 = flat(m.pos[gp.v(i as u32, 0) as usize]).normalize_or(ta.any_orthonormal_vector());
        let mut b2 = ta.cross(b1);
        if flat(m.pos[gp.v(i as u32, 1) as usize]).dot(b2) < 0.0 {
            b2 = -b2;
        }
        let w = p - c;
        let th = w.dot(b2).atan2(w.dot(b1)).rem_euclid(TAU);
        let v0 = m.uv[gp.v(i as u32, 0) as usize][1];
        let v1 = m.uv[gp.v(i as u32 + 1, 0) as usize][1];
        [th / TAU * tiling_u, v0 + (v1 - v0) * t]
    };
    // one reference so the projected u never jumps across the parent's seam within this joint
    let u_ref = pmap(m, st.points[s as usize])[0];
    let unwrapped = |m: &Mesh, p: Vec3| {
        let q = pmap(m, p);
        [u_ref + wrap_u(q[0] - u_ref), q[1]]
    };
    // correction at every rim vertex: its real parent coordinates minus the projection
    let rim_fix: Vec<(f32, [f32; 2])> = rim
        .iter()
        .map(|&r| {
            let p = m.pos[r as usize];
            let q = unwrapped(m, p);
            let real = m.uv[r as usize];
            (ang(p), [wrap_u(real[0] - q[0]), real[1] - q[1]])
        })
        .collect();
    let fix_at = |p: Vec3| -> [f32; 2] {
        // the two rim vertices around the branch on either side of `p`, by angle
        let a = ang(p);
        let d = |x: f32| (x - a + PI).rem_euclid(TAU) - PI;
        let (mut lo, mut hi) = ((f32::MIN, [0.0f32; 2]), (f32::MAX, [0.0f32; 2]));
        for &(ra, f) in &rim_fix {
            let x = d(ra);
            if x <= 0.0 && x > lo.0 {
                lo = (x, f);
            }
            if x >= 0.0 && x < hi.0 {
                hi = (x, f);
            }
        }
        match (lo.0 > f32::MIN, hi.0 < f32::MAX) {
            (true, true) if hi.0 - lo.0 > 1e-6 => {
                let t = -lo.0 / (hi.0 - lo.0);
                [lo.1[0] + (hi.1[0] - lo.1[0]) * t, lo.1[1] + (hi.1[1] - lo.1[1]) * t]
            }
            (true, _) => lo.1,
            (_, true) => hi.1,
            _ => [0.0; 2],
        }
    };
    let set = |m: &mut Mesh, i: u32, w: f32| {
        let p = m.pos[i as usize];
        let q = unwrapped(m, p);
        let f = fix_at(p);
        if m.xuv.len() < m.pos.len() {
            m.xuv.resize(m.pos.len(), [0.0; 3]);
        }
        m.xuv[i as usize] = [q[0] + f[0], q[1] + f[1], w];
    };
    // the collar's opening side shows the parent's bark exactly
    for (i, &e) in rim_ext.iter().enumerate() {
        let r = rim[i % rim.len()] as usize;
        let real = m.uv[r];
        let q = unwrapped(m, m.pos[r]);
        if m.xuv.len() < m.pos.len() {
            m.xuv.resize(m.pos.len(), [0.0; 3]);
        }
        m.xuv[e as usize] = [q[0] + wrap_u(real[0] - q[0]), real[1], 1.0];
    }
    // then the branch's rings fade into its own bark over a few of its radii
    let rc = st.radii[s as usize].max(0.002);
    let len = rc * 3.0;
    let mut arc = 0.0f32;
    for j in s..gc.n {
        if j > s {
            arc += st.points[j as usize].distance(st.points[j as usize - 1]);
        }
        let t = (arc / len).min(1.0);
        let w = 0.7 * (1.0 - t * t * (3.0 - 2.0 * t));
        if w <= 0.0 {
            break;
        }
        for k in 0..=gc.sides {
            set(m, gc.base + j * gc.ring + k, w);
        }
    }
}

/// Extra rings along the first part of a stem: `zones` are (arc length from the base, longest
/// spacing there), in increasing order. The rings lie on the polyline, so the shape is unchanged.
fn densify_base(points: &mut Vec<Vec3>, radii: &mut Vec<f32>, zones: &[(f32, f32)]) {
    let mut np = vec![points[0]];
    let mut nr = vec![radii[0]];
    let mut d = 0.0f32;
    for i in 1..points.len() {
        let seg = points[i].distance(points[i - 1]);
        if let Some(&(_, step)) = zones.iter().find(|z| d < z.0) {
            let k = ((seg / step.max(1e-3)).ceil() as usize).clamp(1, 24);
            for s in 1..k {
                let t = s as f32 / k as f32;
                np.push(points[i - 1].lerp(points[i], t));
                nr.push(radii[i - 1] + (radii[i] - radii[i - 1]) * t);
            }
        }
        d += seg;
        np.push(points[i]);
        nr.push(radii[i]);
    }
    *points = np;
    *radii = nr;
}

/// Welding: swell the base of a branch into a collar that blends into its parent.
/// Extra rings are inserted near the base so the blend is smooth.
/// `exit` is how far along the branch its underside leaves the parent's bark (a branch at a
/// steep angle to its parent runs inside it for a while); the rings reach past it, so the joint
/// closes right where the branch comes out instead of stretching to a ring far up the branch.
#[allow(clippy::too_many_arguments)]
fn weld_base(points: &mut Vec<Vec3>, radii: &mut Vec<f32>, r0: f32, parent_r: f32, weld: f32, swell: bool, exit: f32) {
    let reach = r0 * 2.0 + parent_r * (0.4 + 1.4 * weld);
    let zone = reach.max(parent_r).max(exit * 1.15);
    // densify the polyline inside the blend zone
    let mut np = vec![points[0]];
    let mut nr = vec![radii[0]];
    let mut d = 0.0;
    for i in 1..points.len() {
        let seg = points[i].distance(points[i - 1]);
        let step = (reach * 0.34).min(parent_r * 0.6).max(exit * 0.4).max(1e-3);
        if d < zone && seg > step {
            let k = ((seg.min(zone + step - d) / step).ceil() as usize).clamp(1, 6);
            for s in 1..k {
                let t = s as f32 / k as f32;
                np.push(points[i - 1].lerp(points[i], t));
                nr.push(radii[i - 1] + (radii[i] - radii[i - 1]) * t);
            }
        }
        d += seg;
        np.push(points[i]);
        nr.push(radii[i]);
    }
    *points = np;
    *radii = nr;
    if !swell {
        // joined to the parent's mesh: the opening in the bark forms the fillet, the branch
        // itself keeps its natural thickness (no narrowing inside, no bulge outside)
        return;
    }
    let (np, mut nr) = (points.clone(), radii.clone());
    let base = (r0 * (1.0 + 1.1 * weld) + weld * 0.8 * (parent_r * 0.9 - r0).max(0.0)).min(parent_r.max(r0) * 1.15);
    // the part inside the parent narrows, so the base ring never pokes out of the bark
    let inner = parent_r * 0.75;
    let mut d = 0.0;
    for i in 0..np.len() {
        if i > 0 {
            d += np[i].distance(np[i - 1]);
        }
        let f = (1.0 - d / reach).max(0.0);
        nr[i] += (base - r0) * f * f;
        if d < inner {
            let t = d / inner;
            nr[i] *= 0.45 + 0.55 * t * (2.0 - t);
        }
    }
    *points = np;
    *radii = nr;
}

/// Radii of a stem's points: `ts` = fraction along the stem, `hs` = height fraction of each
/// point. A taper curve sets where the stem narrows: the narrowing is accumulated along the
/// stem, so a low curve value keeps the thickness there and a high one narrows it there. The
/// tip still ends at the taper setting.
fn stem_radii(p: &GenParams, r0: f32, ts: &[f32], hs: &[f32], ground: bool) -> Vec<f32> {
    let prof = p.curves.get("radius_profile");
    let tc = p.curves.get("taper");
    // accumulated curve along the stem (a few sub-steps per segment)
    let mut acc = vec![0.0f32; ts.len()];
    for i in 1..ts.len() {
        let mut d = 0.0;
        let sub = 4;
        for k in 0..sub {
            let f = (k as f32 + 0.5) / sub as f32;
            let t = ts[i - 1] + (ts[i] - ts[i - 1]) * f;
            let h = hs[i - 1] + (hs[i] - hs[i - 1]) * f;
            d += (ts[i] - ts[i - 1]) / sub as f32 * tc.map(|c| c.at(t, h)).unwrap_or(1.0);
        }
        acc[i] = acc[i - 1] + d;
    }
    // normalised so the stem's end gets the same narrowing as without a curve
    let span = ts.last().copied().unwrap_or(0.0) - ts.first().copied().unwrap_or(0.0);
    let total = acc.last().copied().unwrap_or(0.0);
    let norm = if tc.is_some() { if total > 1e-5 { span / total } else { 0.0 } } else { 1.0 };
    let mut out = Vec::with_capacity(ts.len());
    for i in 0..ts.len() {
        let u = ts[0] + acc[i] * norm;
        let (t, h) = (ts[i], hs[i]);
        let r = if let Some(c) = prof {
            r0 * c.at(t, h)
        } else if ground {
            r0 * root_taper(p.taper, u)
        } else {
            r0 * (1.0 + (p.taper - 1.0) * u)
        };
        out.push(r.max(0.002));
    }
    out
}

/// Stem radius at fraction `t` (before bark detail).
fn stem_radius(p: &GenParams, r0: f32, t: f32, ground: bool) -> f32 {
    let r = if let Some(c) = p.curves.get("radius_profile") {
        r0 * c.eval(t)
    } else if ground {
        r0 * root_taper(p.taper, t)
    } else {
        r0 * (1.0 + (p.taper - 1.0) * t)
    };
    r.max(0.002)
}

#[allow(clippy::too_many_arguments)]
fn build_stem(cx: &Ctx, origin: Vec3, dir: Vec3, length: f32, r0: f32, p: &GenParams, kink: f32, trunk: bool, ground: bool, rng: &mut Rng) -> (Vec<Vec3>, Vec<f32>) {
    let doc = cx.doc;
    let n = ((p.segments as f32 * cx.lodf.segments).round() as usize).max(if trunk { 3 } else { 1 });
    let step = length / n as f32;
    let mut pos = origin;
    let mut d = dir.normalize_or(Vec3::Y);
    if ground {
        // roots never start upwards: they spread sideways and down into the soil
        d = Vec3::new(d.x, d.y.min(-0.05), d.z).normalize_or(Vec3::X);
    }
    // a root's compass heading, kept when it dives steeply
    let mut heading = Vec3::new(d.x, 0.0, d.z).normalize_or(Vec3::X);
    // roots turn towards their gravity pitch at this rate per segment (about 90% of the way
    // over ten segments, independent of the segment count)
    let steer = 1.0 - 0.8f32.powf(10.0 / n as f32);
    let mut pts = vec![pos];
    // roots meander more than branches as they find their way through the soil
    let wig = p.wiggle * if ground { 1.1 } else { 0.6 } / (n as f32).sqrt();
    let forces: Vec<&Force> = doc.forces.iter().filter(|f| f.enabled && f.strength != 0.0 && p.force_response > 0.0).collect();
    let prune = doc.pruning.enabled.then_some(&doc.pruning).filter(|_| !ground);
    // one kink somewhere in the middle
    let kink_t = 0.3 + 0.4 * rng.f();
    let kink_axis = rng.unit();
    let mut kinked = kink <= 0.0;
    let mut cut_short = false;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let h = cx.hf(pos);
        if ground {
            // Root gravity is how steeply roots head into the soil: 0 runs along the ground,
            // 1 dives straight down. Roots turn gradually towards that pitch, so they leave the
            // trunk near the surface and sink further away from it.
            let g = (p.gravity * p.cv("gravity", t, h)).clamp(0.0, 1.0);
            let hz = Vec3::new(d.x, 0.0, d.z);
            if hz.length() > 0.05 {
                heading = hz.normalize();
            }
            let a = g * 88f32.to_radians();
            let target = heading * a.cos() - Vec3::Y * a.sin();
            d = d.lerp(target, steer).normalize_or(target);
            // meander mostly sideways; up and down is the gravity's job
            let mut w = rng.unit() * (wig * p.cv("wiggle", t, h));
            w.y *= 0.35;
            d = (d + w).normalize_or(d);
        } else {
            d = (d + Vec3::NEG_Y * (p.gravity * p.cv("gravity", t, h) * 2.2 / n as f32)).normalize_or(d);
            d = (d + rng.unit() * (wig * p.cv("wiggle", t, h))).normalize_or(d);
        }
        let fr = p.force_response * p.cv("force_response", t, h);
        for f in &forces {
            d = (d + f.vector(pos) * (f.strength * fr * 2.0 / n as f32)).normalize_or(d);
        }
        if !kinked && t >= kink_t {
            kinked = true;
            let axis = d.cross(kink_axis).normalize_or(perp_basis(d).0);
            d = glam::Quat::from_axis_angle(axis, kink.to_radians()) * d;
        }
        let mut next = pos + d * step;
        if ground {
            // Roots never come up out of the soil: their top stays sunk in by `buried` of
            // their thickness (0: resting on the ground; 1: just below the surface). With no
            // gravity they run along at that depth; with gravity they head below it.
            let r = stem_radius(p, r0, (i + 1) as f32 / n as f32, true);
            let surface = r * (1.0 - 2.0 * p.buried);
            let g = (p.gravity * p.cv("gravity", (i + 1) as f32 / n as f32, h)).clamp(0.0, 1.0);
            if next.y > surface || (g <= 0.0 && next.y < surface) {
                next.y = surface;
                d.y = if g <= 0.0 { 0.0 } else { d.y.min(0.0) };
                d = d.normalize_or(heading);
            }
        }
        if let Some(pr) = prune {
            // branches stop at the volume's surface; trunks may grow up into it but are cut at its top
            let cut = if trunk { !pr.contains(next) && next.y > pr.center[1] } else { !pr.contains(next) };
            if cut {
                cut_short = true;
                break;
            }
        }
        pos = next;
        pts.push(pos);
    }
    // jitter: small sharp offsets that don't accumulate
    if p.jitter > 0.0 {
        let last = pts.len() - 1;
        for (i, q) in pts.iter_mut().enumerate().skip(1) {
            if i == last {
                break;
            }
            let o = rng.unit();
            let k = p.cv("jitter", i as f32 / n as f32, cx.hf(*q));
            *q += o * (p.jitter * k * step * 0.35);
        }
    }
    // a stem stopped by the crown volume is as long as fits: it tapers to its tip over the
    // length it has, instead of ending in a sawn-off stump
    let span = if cut_short { (pts.len() - 1).max(1) } else { n };
    let ts: Vec<f32> = (0..pts.len()).map(|i| i as f32 / span as f32).collect();
    let hs: Vec<f32> = pts.iter().map(|q| cx.hf(*q)).collect();
    let radii = stem_radii(p, r0, &ts, &hs, ground);
    (pts, radii)
}

/// One leader of a fork: its axis and radii.
type Leader = (Vec<Vec3>, Vec<f32>);

/// Split a stem into `p.splits` leaders at `p.split_at` of its length, as decurrent crowns do
/// (oaks, maples): the stem itself carries on as the first leader, bent away from the others,
/// and the rest grow from the fork point. The leaders share the stem's wood (their cross-sections
/// add up to a bit more than the stem's below the fork), so each is thinner, and every one
/// tapers to a thin tip. Returns the arc fraction of the fork on the stem (1 = no fork) and the
/// other leaders.
fn fork_stem(cx: &Ctx, p: &GenParams, points: &mut Vec<Vec3>, radii: &mut Vec<f32>, trunk: bool, ground: bool, key: u32) -> (f32, f32, Vec<Leader>) {
    let nf = p.splits as usize;
    if nf < 2 || points.len() < 3 {
        return (1.0, 0.0, vec![]);
    }
    // its own random numbers, so forking does not reshuffle the rest of the tree
    let mut rng = Rng::new(&[cx.seed, key as u64, 0xF0_4C]);
    let mut acc = vec![0.0f32];
    for w in points.windows(2) {
        acc.push(acc.last().unwrap() + w[0].distance(w[1]));
    }
    let total = *acc.last().unwrap();
    if total < 1e-4 {
        return (1.0, 0.0, vec![]);
    }
    let at = (p.split_at + rng.s() * 0.04).clamp(0.05, 0.95) * total;
    // the fork sits on a ring: the nearest one if close, else a new one
    let i = (0..points.len() - 1).find(|&i| acc[i + 1] >= at).unwrap_or(points.len() - 2);
    let seg = (acc[i + 1] - acc[i]).max(1e-6);
    let f = (at - acc[i]) / seg;
    let fi = if f < 0.2 {
        i
    } else if f > 0.8 {
        i + 1
    } else {
        points.insert(i + 1, points[i].lerp(points[i + 1], f));
        radii.insert(i + 1, radii[i] + (radii[i + 1] - radii[i]) * f);
        i + 1
    };
    if fi < 1 || fi + 1 >= points.len() {
        return (1.0, 0.0, vec![]);
    }
    let c = points[fi];
    let fork_arc: f32 = points[..=fi].windows(2).map(|w| w[0].distance(w[1])).sum();
    let ft = fork_arc / total;
    let d0 = (points[fi] - points[fi - 1]).normalize_or(Vec3::Y);
    let rf = radii[fi];
    // the leaders' wood adds up to a little more than the stem's (da Vinci's rule, plus a bit)
    let rl = (rf * 1.15 / (nf as f32).sqrt()).min(rf * 0.9);
    let tip_l = radii.last().copied().unwrap_or(rl) * (rl / rf.max(1e-6));
    let (b1, b2) = perp_basis(d0);
    let az0 = rng.f() * TAU;
    let dirs: Vec<Vec3> = (0..nf)
        .map(|j| {
            let az = az0 + j as f32 * TAU / nf as f32 + rng.s() * 0.35;
            let tilt = (p.split_angle * (1.0 + rng.s() * 0.3)).max(0.0).to_radians();
            (d0 * tilt.cos() + (b1 * az.cos() + b2 * az.sin()) * tilt.sin()).normalize_or(d0)
        })
        .collect();
    let rest = total - fork_arc;
    let lens: Vec<f32> = (0..nf).map(|j| if j == 0 { rest } else { rest * (1.0 + rng.s() * 0.12) }).collect();
    // every leader grows afresh from the fork; a trunk's leaders curve back up towards the
    // light, as the limbs of a forked oak do
    let mut pj = p.clone();
    pj.segments = ((p.segments as f32 * (1.0 - ft)).round() as u32).max(2);
    pj.kink = 0.0;
    if trunk {
        pj.gravity = p.gravity.min(0.0) - 0.3;
    }
    let mut built: Vec<Vec<Vec3>> = (0..nf)
        .map(|j| {
            let mut lr = Rng::new(&[cx.seed, key as u64, 0xF0_4C, j as u64]);
            build_stem(cx, c, dirs[j], lens[j], rl, &pj, 0.0, trunk, ground, &mut lr).0
        })
        .collect();
    // the stem carries on as the first leader
    let first = std::mem::take(&mut built[0]);
    if first.len() < 2 {
        return (1.0, 0.0, vec![]);
    }
    points.truncate(fi + 1);
    radii.truncate(fi + 1);
    points.extend_from_slice(&first[1..]);
    radii.resize(points.len(), 0.0);
    // radii past the fork: from the leader's base to its tip, and right at the fork a short
    // crotch where the stem narrows into the leader
    let leader_radii = |pts: &[Vec3], crotch: bool| -> Vec<f32> {
        let mut a = vec![0.0f32];
        for w in pts.windows(2) {
            a.push(a.last().unwrap() + w[0].distance(w[1]));
        }
        let len = a.last().copied().unwrap_or(0.0).max(1e-6);
        a.iter()
            .map(|&x| {
                let r = rl + (tip_l - rl) * (x / len);
                if crotch {
                    let s = (x / (1.2 * rf)).clamp(0.0, 1.0);
                    r + (rf - rl) * (1.0 - s * s * (3.0 - 2.0 * s))
                } else {
                    r
                }
            })
            .map(|r| r.max(0.002))
            .collect()
    };
    let tail = leader_radii(&points[fi..], true);
    for (k, r) in tail.into_iter().enumerate() {
        radii[fi + k] = r;
    }
    let mut out = vec![];
    for lp in built.into_iter().skip(1) {
        if lp.len() < 2 {
            continue;
        }
        let rr = leader_radii(&lp, false);
        out.push((lp, rr));
    }
    // the crotch: about as far as a leader's base reaches along the stem
    let gap = (rl / p.split_angle.max(12.0).to_radians().sin() + 0.3 * rf).min(0.3 * total);
    (ft, gap, out)
}

/// Add a stem's fork leaders (from [`fork_stem`]) to the skeleton as children of stem `idx`.
fn push_leaders(cx: &Ctx, sk: &mut Skeleton, idx: usize, leaders: Vec<Leader>, p: &GenParams) {
    let main = &sk.stems[idx];
    let (gen, depth, key, phase, ft, root) = (main.gen, main.depth, main.key, main.phase, main.fork_t, main.root);
    let rf = main.sample(ft).2;
    for (j, (mut points, mut radii)) in leaders.into_iter().enumerate() {
        let lkey = node_key(key, gen, 0xF000 + j as u32);
        if cx.edits.get(&lkey).map(|e| e.hidden).unwrap_or(false) {
            continue;
        }
        let r0 = radii[0];
        if !root || cx.doc.settings.join_branches {
            weld_base(&mut points, &mut radii, r0, rf, p.weld, !root && !cx.doc.settings.join_branches, 0.0);
        }
        let length = points.windows(2).map(|w| w[0].distance(w[1])).sum();
        sk.stems.push(Stem { gen, depth: depth + 1, parent: Some(idx), points, radii, length, phase: phase + 0.3 * (j as f32 + 1.0), root, lobes: vec![], key: lkey, broken: false, fork: true, forks: vec![], fork_t: 1.0, fork_gap: 0.0 });
        let li = sk.stems.len() - 1;
        sk.stems[idx].forks.push(li);
    }
}

/// The grown tree's skeleton: without the stems it shed while young (`GenGrowth::shed`).
pub fn grow(doc: &Document, level: u32) -> Skeleton {
    if doc.has_shed() {
        grow_doc(&doc.without_shed(), level)
    } else {
        grow_doc(doc, level)
    }
}

/// The skeleton the growth animates: with the stems the tree sheds while young. Its other
/// stems are the grown tree's, one for one (every generator rolls its own random numbers).
pub fn grow_young(doc: &Document, level: u32) -> Skeleton {
    grow_doc(doc, level)
}

fn grow_doc(doc: &Document, level: u32) -> Skeleton {
    // expected height of the tree (its tallest trunk), for curves along the tree's height
    let tree_h = doc
        .generators
        .iter()
        .filter(|g| g.parent.is_none() && g.enabled && g.kind == GenKind::Trunk)
        .map(|g| g.params.length * g.params.angle.to_radians().cos().max(0.2))
        .fold(0.0f32, f32::max)
        * doc.settings.scale
        * doc.settings.age.powf(0.85);
    let cx = Ctx {
        doc,
        // every level grows the full tree; lower levels are reduced from it afterwards
        lodf: lod(0),
        seed: doc.settings.seed as u64,
        edits: doc.nodes.iter().map(|n| (n.key, n)).collect(),
        tree_h: tree_h.max(0.1),
    };
    let s = &doc.settings;
    let seed = cx.seed;
    let mut sk = Skeleton::default();
    // trunks
    for g in doc.generators.iter().filter(|g| g.parent.is_none() && g.enabled && g.kind == GenKind::Trunk) {
        let p = &g.params;
        for k in 0..p.count {
            let mut rng = Rng::new(&[seed, g.id as u64, 0xA, k as u64]);
            let key = node_key(0, g.id, k);
            let edit = cx.edits.get(&key).copied();
            if edit.map(|e| e.hidden).unwrap_or(false) {
                continue;
            }
            let az = if p.count > 1 { k as f32 / p.count as f32 * TAU + rng.s() * 0.4 } else { rng.f() * TAU } + edit.map(|e| e.rotate.to_radians()).unwrap_or(0.0);
            let spread = if p.count > 1 { p.radius * 1.5 + 0.05 * p.count as f32 * p.radius * 10.0 } else { 0.0 };
            let origin = Vec3::new(az.cos() * spread, 0.0, az.sin() * spread);
            let lean = (p.angle + rng.s() * p.angle_variation + edit.map(|e| e.angle).unwrap_or(0.0)).to_radians();
            let dir = Vec3::new(az.cos() * lean.sin(), lean.cos(), az.sin() * lean.sin());
            let len = p.length * (1.0 + rng.s() * p.length_variation) * s.scale * edit.map(|e| e.length).unwrap_or(1.0) * s.age.powf(0.85);
            let r0 = p.radius * s.scale * edit.map(|e| e.radius).unwrap_or(1.0) * s.age.powf(1.2);
            let (mut points, mut radii) = build_stem(&cx, origin, dir, len, r0, p, p.kink, true, false, &mut rng);
            if points.len() < 2 {
                continue;
            }
            let has_roots = doc.generators.iter().any(|c| c.parent == Some(g.id) && c.enabled && c.kind == GenKind::Roots);
            if p.flare > 0.0 || has_roots {
                // the flare and the root ridges are shaped ring by ring: enough rings near the
                // base that they curve smoothly into the roots instead of creasing at a ring
                densify_base(&mut points, &mut radii, &[(len / 6.0, len / 36.0), (len * 0.35, len * 0.35 / 6.0)]);
            }
            let (fork_t, fork_gap, leaders) = fork_stem(&cx, p, &mut points, &mut radii, true, false, key);
            sk.stems.push(Stem { gen: g.id, depth: 0, parent: None, points, radii, length: len, phase: rng.f() * TAU, root: false, lobes: vec![], key, broken: false, fork: false, forks: vec![], fork_t, fork_gap });
            let idx = sk.stems.len() - 1;
            push_leaders(&cx, &mut sk, idx, leaders, p);
            grow_children(&cx, g.id, idx, &mut sk);
            // buttress ridges line up with the roots; without roots, a few irregular ridges
            let mut lobes: Vec<f32> = sk
                .stems
                .iter()
                .filter(|st| st.parent == Some(idx) && st.root && st.points.len() > 1)
                .map(|st| {
                    let d = st.points[1] - st.points[0];
                    d.z.atan2(d.x)
                })
                .collect();
            if lobes.is_empty() {
                let mut lr = Rng::new(&[seed, g.id as u64, 0x10BE, k as u64]);
                let count = 4 + (lr.f() * 3.0) as usize;
                let off = lr.f() * TAU;
                lobes = (0..count).map(|i| off + i as f32 / count as f32 * TAU + lr.s() * 0.4).collect();
            }
            sk.stems[idx].lobes = lobes;
        }
    }
    if level > 0 {
        reduce_detail(doc, &mut sk, &lod(level));
    }
    sk
}

fn grow_children(cx: &Ctx, gen_id: u32, stem_idx: usize, sk: &mut Skeleton) {
    let doc = cx.doc;
    let children: Vec<&Generator> = doc.children(Some(gen_id)).filter(|g| g.enabled).collect();
    for g in children {
        let p = &g.params;
        let mut rng = Rng::new(&[cx.seed, g.id as u64, stem_idx as u64]);
        let whorl = p.whorl.max(1) as usize;
        let span = p.end - p.start;
        let mut n = if p.interval > 0.0 {
            // spacing in metres: longer parents get more children
            let nodes = ((sk.stems[stem_idx].length * span) / (p.interval * doc.settings.scale)).round().max(1.0) as usize;
            (nodes * whorl).min(400)
        } else {
            p.count as usize
        };
        if g.kind == GenKind::Leaves {
            n = ((n as f32 * cx.lodf.leaves).round() as usize).max(if n > 0 { 1 } else { 0 });
        } else if doc.settings.age < 1.0 {
            // younger trees: fewer children, deeper levels appear last
            let d = sk.stems[stem_idx].depth as f32;
            let lag = 0.12 * d;
            let f = ((doc.settings.age - lag) / (1.0 - lag).max(0.05)).clamp(0.0, 1.0).powf(0.7);
            n = (n as f32 * f).round() as usize;
        }
        let nodes = n.div_ceil(whorl).max(1);
        let parent_key = sk.stems[stem_idx].key;
        let main_gap = sk.stems[stem_idx].fork_gap;
        let main_arc: f32 = sk.stems[stem_idx].points.windows(2).map(|w| w[0].distance(w[1])).sum();
        for k in 0..n {
            if (g.kind == GenKind::Leaves && sk.leaves.len() >= MAX_LEAVES) || sk.stems.len() >= MAX_STEMS {
                return;
            }
            let node = k / whorl;
            let j = k % whorl;
            let tn = (node as f32 + 0.5) / nodes as f32;
            let rel = (tn + rng.s() * p.random * 0.5 / nodes as f32).clamp(0.0, 1.0);
            let t = p.start + span * rel;
            let key = node_key(parent_key, g.id, k as u32);
            // above a fork, children spread over the stem and its leaders
            let main = &sk.stems[stem_idx];
            let (host, ht) = if main.forks.is_empty() {
                (stem_idx, t)
            } else {
                // nothing grows in the crotch: below the fork the children move down out of
                // it, above it they spread over the stem and its leaders, past their bases
                let ft = main.fork_t;
                let g = main_gap / main_arc.max(1e-3);
                if t <= ft {
                    (stem_idx, t * ((ft - g).max(0.5 * ft) / ft.max(1e-4)))
                } else {
                    let u = ((t - ft) / (1.0 - ft).max(1e-4)).clamp(0.0, 1.0);
                    let m = main.forks.len() + 1;
                    match (splitmix(key as u64 ^ 0xF0_4C) % m as u64) as usize {
                        0 => {
                            let a = (ft + g).min(0.5 * (1.0 + ft));
                            (stem_idx, a + (1.0 - a) * u)
                        }
                        pick => {
                            let li = main.forks[pick - 1];
                            let u0 = (main_gap / sk.stems[li].length.max(1e-3)).min(0.4);
                            (li, u0 + (1.0 - u0) * u)
                        }
                    }
                }
            };
            let parent = &sk.stems[host];
            let (pos, tan, pr) = parent.sample(ht);
            let parent_len = main.length;
            let parent_depth = main.depth;
            let parent_phase = parent.phase;
            let ground = g.kind == GenKind::Roots || main.root;
            // height of the spot on the tree, for curves measured along the tree's height
            let ha = cx.hf(pos);
            // draw every random number up front so edits and curves don't reshuffle the tree
            let jitter_az = rng.s();
            let jitter_ang = rng.s();
            let density_roll = rng.f();
            if density_roll > p.cv("density", rel, ha) {
                continue;
            }
            let edit = if g.kind == GenKind::Leaves { None } else { cx.edits.get(&key).copied() };
            if edit.map(|e| e.hidden).unwrap_or(false) {
                continue;
            }
            let az = (node as f32 * p.phyllotaxis + j as f32 * 360.0 / whorl as f32 + edit.map(|e| e.rotate).unwrap_or(0.0)).to_radians() + jitter_az * p.random * 0.6;
            let (b1, b2) = perp_basis(tan);
            let radial = b1 * az.cos() + b2 * az.sin();
            let ang = (p.angle * p.cv("angle", rel, ha) + jitter_ang * p.angle_variation * p.cv("angle_variation", rel, ha) + edit.map(|e| e.angle).unwrap_or(0.0)).to_radians();
            let dir = (tan * ang.cos() + radial * ang.sin()).normalize_or(radial);
            match g.kind {
                GenKind::Leaves => {
                    let size = p.leaf_size * p.cv("leaf_size", rel, ha) * (1.0 + rng.s() * p.leaf_size_variation) * cx.lodf.leaf_size * doc.settings.scale * (0.7 + 0.3 * doc.settings.age);
                    let mut side = dir.cross(Vec3::Y).normalize_or(Vec3::X);
                    let roll = rng.s() * 0.6;
                    let up = side.cross(dir).normalize_or(Vec3::Y);
                    side = (side * roll.cos() + up * roll.sin()).normalize();
                    let mut normal = side.cross(dir).normalize_or(Vec3::Y);
                    let mut ldir = dir;
                    let face_up = p.leaf_face_up * p.cv("leaf_face_up", rel, ha);
                    if face_up > 0.0 {
                        // turn the leaf blade towards the sky
                        if normal.y < 0.0 {
                            normal = -normal;
                        }
                        let n2 = (normal + Vec3::Y * (face_up * 3.0)).normalize();
                        ldir = (ldir - n2 * ldir.dot(n2)).normalize_or(ldir);
                        normal = n2;
                    }
                    let base = doc.gen_material(g.id).map(|m| m.color).unwrap_or(doc.settings.leaf_color);
                    let color = leaf_color(&doc.settings, base, &mut rng);
                    let pos = pos + radial * pr * 0.8;
                    if doc.pruning.enabled && !doc.pruning.contains(pos + ldir * size * 0.5) {
                        continue;
                    }
                    if doc.settings.leaf_drop > 0.0 {
                        // season: a stable per-leaf draw decides whether it has fallen yet
                        let h = splitmix((sk.stems[stem_idx].key as u64) << 20 ^ (node as u64) << 6 ^ j as u64 ^ 0x5eed);
                        if ((h >> 40) as f32 / (1u64 << 24) as f32) < doc.settings.leaf_drop {
                            continue;
                        }
                    }
                    sk.leaves.push(Leaf {
                        gen: g.id,
                        stem: host,
                        pos,
                        dir: ldir,
                        normal,
                        size: size.max(0.005),
                        aspect: p.leaf_aspect,
                        shape: p.leaf_shape,
                        color,
                        mat: g.material.filter(|m| doc.material(*m).is_some()).unwrap_or(0),
                        fold: (p.leaf_fold * p.cv("leaf_fold", rel, ha)).to_radians(),
                        bend: p.leaf_bend * p.cv("leaf_bend", rel, ha),
                    });
                }
                _ => {
                    let shape = match p.curves.get("length") {
                        Some(c) => c.at(rel, ha),
                        None => p.profile.eval(rel),
                    };
                    let mut len = (parent_len * p.length * shape * (1.0 + rng.s() * p.length_variation * p.cv("length_variation", rel, ha)) * edit.map(|e| e.length).unwrap_or(1.0)).max(0.02);
                    // broken stubs: decided by a stable per-branch draw so the rest of the tree stays put
                    let hb = splitmix(key as u64 ^ 0xb4ea_c0de);
                    let broken = !ground && p.break_chance > 0.0 && ((hb >> 40) as f32 / (1u64 << 24) as f32) < p.break_chance * p.cv("break_chance", rel, ha);
                    let full_len = len;
                    if broken {
                        let h2 = (hb & 0xffff) as f32 / 65535.0;
                        len = (len * p.break_length * (0.6 + 0.8 * h2)).max(0.02);
                    }
                    let r0 = ((pr * p.radius * p.cv("radius", rel, ha)).min(pr * 0.95) * edit.map(|e| e.radius).unwrap_or(1.0)).max(0.002);
                    if doc.pruning.enabled && !ground && !doc.pruning.contains(pos) {
                        continue;
                    }
                    let (mut points, mut radii) = build_stem(cx, pos, dir, len, r0, p, p.kink * p.cv("kink", rel, ha), false, ground, &mut rng);
                    if points.len() < 2 {
                        continue;
                    }
                    if broken {
                        // keep the thickness the full branch would have had at the break
                        let frac = len / full_len;
                        let n = radii.len().max(2) - 1;
                        let ts: Vec<f32> = (0..radii.len()).map(|i| i as f32 / n as f32 * frac).collect();
                        let hs: Vec<f32> = points.iter().map(|q| cx.hf(*q)).collect();
                        radii = stem_radii(p, r0, &ts, &hs, false);
                    }
                    if !ground || doc.settings.join_branches {
                        // roots only get the extra rings near the base (for the joint), never a swell
                        let exit = {
                            let d = (points[1.min(points.len() - 1)] - points[0]).normalize_or(tan);
                            let sin = (d - tan * d.dot(tan)).length().max(0.15);
                            (pr + r0) / sin
                        };
                        weld_base(&mut points, &mut radii, r0, pr, p.weld, !ground && !doc.settings.join_branches, exit);
                    }
                    let (fork_t, fork_gap, leaders) = if broken { (1.0, 0.0, vec![]) } else { fork_stem(cx, p, &mut points, &mut radii, false, ground, key) };
                    sk.stems.push(Stem {
                        gen: g.id,
                        depth: parent_depth + 1,
                        parent: Some(host),
                        points,
                        radii,
                        length: len,
                        phase: parent_phase + rng.s() * 1.5,
                        root: ground,
                        lobes: vec![],
                        key,
                        broken,
                        fork: false,
                        forks: vec![],
                        fork_t,
                        fork_gap,
                    });
                    let idx = sk.stems.len() - 1;
                    push_leaders(cx, sk, idx, leaders, p);
                    if !broken {
                        grow_children(cx, g.id, idx, sk);
                    }
                }
            }
        }
    }
    if doc.settings.join_branches && doc.settings.join_method == JoinMethod::Stitch {
        refine_for_children(doc, sk, stem_idx);
        for li in sk.stems[stem_idx].forks.clone() {
            refine_for_children(doc, sk, li);
        }
    }
}

/// Add rings to a stem just above and below every branch growing from it, so the opening cut
/// for each branch (mesh joining) fits the branch base snugly instead of spanning long bark
/// quads. The new points lie on the existing polyline, so the shape does not change.
fn refine_for_children(doc: &Document, sk: &mut Skeleton, pi: usize) {
    let pts = sk.stems[pi].points.clone();
    let radii = sk.stems[pi].radii.clone();
    if pts.len() < 2 {
        return;
    }
    let mut acc = vec![0.0f32];
    for w in pts.windows(2) {
        acc.push(acc.last().unwrap() + w[0].distance(w[1]));
    }
    let total = *acc.last().unwrap();
    let mut cuts: Vec<(f32, f32)> = vec![];
    for c in sk.stems.iter().filter(|c| c.parent == Some(pi) && c.points.len() >= 2) {
        let p0 = c.points[0];
        let (mut best, mut at, mut tan) = (f32::MAX, 0.0f32, Vec3::Y);
        for i in 0..pts.len() - 1 {
            let ab = pts[i + 1] - pts[i];
            let l = ab.length().max(1e-6);
            let t = ((p0 - pts[i]).dot(ab) / (l * l)).clamp(0.0, 1.0);
            let d = p0.distance(pts[i] + ab * t);
            if d < best {
                best = d;
                at = acc[i] + t * l;
                tan = ab / l;
            }
        }
        let cdir = (c.points[1] - c.points[0]).normalize_or(tan);
        let sin = (cdir - tan * cdir.dot(tan)).length().max(0.3);
        let weld = doc.get(c.gen).map(|g| g.params.weld).unwrap_or(0.0);
        // half-length of the branch's footprint along this stem, with the opening's margin
        let h = c.radii[0] * (1.08 + 0.8 * weld) * 1.2 / sin + c.radii[0] * 0.15;
        cuts.push((at - h, h));
        cuts.push((at + h, h));
        // thick children (roots at the trunk base, big limbs) also get rings across their
        // footprint, so the opening can follow their round base closely
        let seg_r = {
            let i = acc.iter().position(|&a| a >= at).unwrap_or(radii.len() - 1).min(radii.len() - 1);
            radii[i]
        };
        if c.radii[0] > seg_r * 0.3 {
            cuts.push((at - h * 0.5, h));
            cuts.push((at + h * 0.5, h));
            cuts.push((at + h * 1.6, h));
        }
    }
    if cuts.is_empty() {
        return;
    }
    cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut stations: Vec<f32> = acc.clone();
    for (x, h) in cuts {
        if x <= 0.0 || x >= total {
            continue;
        }
        if stations.iter().any(|&s| (s - x).abs() < h * 0.3) {
            continue;
        }
        stations.push(x);
    }
    stations.sort_by(|a, b| a.total_cmp(b));
    if stations.len() == pts.len() {
        return;
    }
    let mut np = Vec::with_capacity(stations.len());
    let mut nr = Vec::with_capacity(stations.len());
    let mut seg = 0;
    for x in stations {
        while seg + 1 < acc.len() - 1 && acc[seg + 1] < x {
            seg += 1;
        }
        let l = (acc[seg + 1] - acc[seg]).max(1e-9);
        let t = ((x - acc[seg]) / l).clamp(0.0, 1.0);
        np.push(pts[seg].lerp(pts[seg + 1], t));
        nr.push(radii[seg] + (radii[seg + 1] - radii[seg]) * t);
    }
    sk.stems[pi].points = np;
    sk.stems[pi].radii = nr;
}

fn leaf_color(s: &TreeSettings, base: [u8; 3], rng: &mut Rng) -> [f32; 3] {
    let base = [base[0] as f32 / 255.0, base[1] as f32 / 255.0, base[2] as f32 / 255.0];
    let v = 1.0 + rng.s() * s.leaf_color_variation * 0.5;
    let hue = rng.s() * s.leaf_color_variation * 0.15;
    let mut c = [base[0] * v * (1.0 + hue), base[1] * v, base[2] * v * (1.0 - hue)];
    if s.autumn > 0.0 {
        let palette = [[0.85, 0.62, 0.12], [0.86, 0.38, 0.08], [0.72, 0.16, 0.08], [0.78, 0.5, 0.1]];
        let a = palette[(rng.f() * 4.0) as usize % 4];
        let amt = (s.autumn * (0.7 + 0.5 * rng.f())).min(1.0);
        for i in 0..3 {
            c[i] = c[i] * (1.0 - amt) + a[i] * amt;
        }
    }
    c.map(|x| x.clamp(0.0, 1.0))
}

// ---------------------------------------------------------------- mesh

#[derive(Default, Clone)]
pub struct Mesh {
    pub pos: Vec<Vec3>,
    pub nrm: Vec<Vec3>,
    pub col: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    /// x = branch sway weight (0 at ground .. 1 at crown tips), y = leaf flutter weight.
    pub wind: Vec<[f32; 2]>,
    pub phase: Vec<f32>,
    pub gen: Vec<u32>,
    /// Material id per vertex (0 = default bark / leaf look).
    pub vmat: Vec<u32>,
    /// Skeleton stem index per vertex (leaves: the stem they grow on; u32::MAX = none).
    pub stem: Vec<u32>,
    pub leaf: Vec<bool>,
    /// First vertex of each skeleton leaf (leaf `i` owns vertices `leaf_first[i]..leaf_first[i + 1]`).
    pub leaf_first: Vec<u32>,
    /// Rim of the opening cut into a parent's bark for a joined branch: (vertex, branch stem).
    /// The growth animation closes these openings while the branch is still small.
    pub joint_rim: Vec<(u32, u32)>,
    /// Material layer value per vertex for stems with a material gradient: the layer it is in
    /// plus how far it has blended into the next (empty or 0 = the generator's own material).
    pub layer: Vec<f32>,
    /// Position on its generator's gradient axis per vertex (see [`crate::model::BlendAxis`]),
    /// before noise; the blend weight of a pixel is computed from it and [`blend_noise`].
    pub axis: Vec<f32>,
    /// Where a joined branch or root meets its parent, its bark cross-fades from the parent's
    /// texture mapping into its own: per vertex the parent's texture coordinates (u, v) and the
    /// weight of that mapping (0 = the vertex's own `uv` only). Shorter than `pos` (or empty):
    /// the missing vertices have weight 0.
    pub xuv: Vec<[f32; 3]>,
    /// Bark triangles first, then leaf triangles from `leaf_tri_start`.
    pub tris: Vec<[u32; 3]>,
    pub leaf_tri_start: usize,
    pub height: f32,
    pub radius: f32,
}

#[derive(Default, Clone, Serialize, PartialEq)]
pub struct Stats {
    pub stems: usize,
    pub leaves: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub height_m: f32,
    pub crown_width_m: f32,
}

impl Mesh {
    /// The materials a triangle is drawn with: (layer `k`, material of layer k, and the material
    /// of layer k + 1 when the triangle blends into it). The blend weight at a vertex is
    /// `clamp(layer - k, 0, 1)`.
    pub fn tri_layers(&self, doc: &Document, ti: usize) -> (u32, u32, Option<u32>) {
        let tri = self.tris[ti];
        let v0 = tri[0] as usize;
        if self.axis.is_empty() || ti >= self.leaf_tri_start || self.vmat[v0] == crate::billboard::BILLBOARD_MAT {
            return (0, self.vmat[v0], None);
        }
        let gen = self.gen[v0];
        let Some(b) = doc.get(gen).and_then(|g| g.blend.as_ref()) else { return (0, self.vmat[v0], None) };
        let x = tri.map(|i| self.axis[i as usize]);
        let (k, blends) = b.span_layers(x[0].min(x[1]).min(x[2]), x[0].max(x[1]).max(x[2]));
        let a = doc.layer_material_id(gen, k);
        (k, a, blends.then(|| doc.layer_material_id(gen, k + 1)))
    }

    /// The blend of a triangle that blends into its next layer: (layer index in the gradient's
    /// `layers`, the gradient). None for triangles drawn with one material.
    pub fn tri_blend<'a>(&self, doc: &'a Document, ti: usize) -> Option<(usize, &'a crate::model::MaterialBlend)> {
        let (k, _, next) = self.tri_layers(doc, ti);
        next?;
        let gen = self.gen[self.tris[ti][0] as usize];
        doc.get(gen).and_then(|g| g.blend.as_ref()).map(|b| (k as usize, b))
    }

    pub fn stats(&self, sk: &Skeleton) -> Stats {
        Stats {
            stems: sk.stems.len(),
            leaves: sk.leaves.len(),
            vertices: self.pos.len(),
            triangles: self.tris.len(),
            height_m: (self.height * 100.0).round() / 100.0,
            crown_width_m: (self.radius * 200.0).round() / 100.0,
        }
    }
}

pub fn build_mesh(doc: &Document, sk: &Skeleton, level: u32) -> Mesh {
    build_mesh_with(doc, sk, &lod(level), level)
}

/// The mesh of a skeleton reduced with `lodf`; `level` sets the ambient occlusion resolution.
pub fn build_mesh_with(doc: &Document, sk: &Skeleton, lodf: &Lod, level: u32) -> Mesh {
    // Roots leave the trunk at its very base, often pointing down. So that every root has bark to
    // grow out of, trunks with roots get a short extension below their base (underground). The
    // skeleton itself is unchanged, so nothing growing on the trunk moves.
    let extended;
    let (sk, below): (&Skeleton, Vec<f32>) = if doc.settings.join_branches {
        let (e, b) = extend_trunk_bases(sk);
        extended = e;
        (&extended, b)
    } else {
        (sk, vec![0.0; sk.stems.len()])
    };
    let lodf = *lodf;
    let mut m = Mesh::default();
    let height = sk.stems.iter().flat_map(|s| s.points.iter()).chain(sk.leaves.iter().map(|l| &l.pos)).fold(0.1f32, |a, p| a.max(p.y));
    let radius = sk
        .stems
        .iter()
        .flat_map(|s| s.points.iter())
        .chain(sk.leaves.iter().map(|l| &l.pos))
        .fold(0.1f32, |a, p| a.max((p.x * p.x + p.z * p.z).sqrt()));
    m.height = height;
    m.radius = radius;
    let bark = doc.settings.bark_color.map(|c| c as f32 / 255.0);

    // depth along the hierarchy -> sway weight at the stem's start
    let mut base_sway = vec![0.0f32; sk.stems.len()];
    for (i, st) in sk.stems.iter().enumerate() {
        let w = |p: Vec3| (p.y / height).clamp(0.0, 1.0).powf(2.0) * 0.6;
        base_sway[i] = match st.parent {
            None => 0.0,
            Some(_) if st.root => 0.0,
            // a trunk's fork leaders sway like the trunk they continue
            Some(pi) if st.fork && sk.stems[pi].depth == 0 => base_sway[pi].max(w(st.points[0])),
            Some(pi) => base_sway[pi].max(w(st.points[0])) + 0.1,
        };
    }

    // trunks with roots: flutes follow the roots (sorted root azimuths per stem)
    let root_flutes: Vec<bool> = (0..sk.stems.len()).map(|i| sk.stems[i].depth == 0 && !sk.stems[i].root && sk.stems.iter().any(|c| c.parent == Some(i) && c.root)).collect();
    let lobes_sorted: Vec<Vec<f32>> = sk
        .stems
        .iter()
        .enumerate()
        .map(|(i, st)| {
            if !root_flutes[i] {
                return vec![];
            }
            let mut v: Vec<f32> = st.lobes.iter().map(|a| a.rem_euclid(TAU)).collect();
            v.sort_by(|a, b| a.total_cmp(b));
            v
        })
        .collect();
    let mut grids: Vec<Grid> = Vec::with_capacity(sk.stems.len());
    let mut removed: Vec<Vec<bool>> = Vec::with_capacity(sk.stems.len());
    for (si, st) in sk.stems.iter().enumerate() {
        let defaults = GenParams::default();
        let gpar = doc.get(st.gen).map(|g| &g.params).unwrap_or(&defaults);
        let flare = if st.depth == 0 { gpar.flare } else { 0.0 };
        let (_, sides) = ring_sides(gpar, root_flutes[si].then(|| lobes_sorted[si].len()), st.depth == 0 && !st.root, &lodf);
        let (flutes, flute_depth0, flatten0, bumps0, bump_size) = (gpar.flutes as f32, gpar.flute_depth, gpar.flatten, gpar.bumps, gpar.bump_size.max(0.02));
        let shade = 1.0 - (st.depth as f32 * 0.06).min(0.2);
        let mat = doc.gen_material(st.gen);
        let mat_id = mat.map(|m| m.id).unwrap_or(0);
        let tiling = mat.map(|m| m.effective_tiling()).unwrap_or([1.0, 1.0]);
        let col = mat.map(|m| m.color.map(|c| c as f32 / 255.0)).unwrap_or(bark);
        let c = [col[0] * shade, col[1] * shade, col[2] * shade];
        // parallel transport frame
        let n = st.points.len();
        let mut tan0 = (st.points[1] - st.points[0]).normalize_or(Vec3::Y);
        let (mut b1, _) = perp_basis(tan0);
        let base = m.pos.len() as u32;
        let mut vcoord = 0.0;
        // arc-length fraction per ring (rings can be unevenly spaced around joined branches)
        let mut arc = vec![0.0f32];
        for w in st.points.windows(2) {
            arc.push(arc.last().unwrap() + w[0].distance(w[1]));
        }
        let arc_total = arc.last().copied().unwrap_or(1.0).max(1e-6);
        for i in 0..n {
            let tan = if i + 1 < n { (st.points[i + 1] - st.points[i]).normalize_or(tan0) } else { tan0 };
            // rotate b1 onto plane perpendicular to tan
            b1 = (b1 - tan * b1.dot(tan)).normalize_or(perp_basis(tan).0);
            let b2 = tan.cross(b1);
            tan0 = tan;
            if i > 0 {
                // along the stem the texture repeats once per local circumference, matching how it
                // wraps around: texels stay square on thick and thin parts alike (as with tapering)
                let r = 0.5 * (st.radii[i] + st.radii[i - 1]);
                vcoord += st.points[i].distance(st.points[i - 1]) / (TAU * r.max(0.004));
            }
            // fraction along the stem measured from its real base (extensions are below 0)
            let t = ((arc[i] - below[si]) / (arc_total - below[si]).max(1e-6)).max(0.0);
            // buttress flare near the trunk base: ridges towards the roots, hollows between them
            // below the real base (underground extension) flare and flutes fade out with the rounding
            let under = if below[si] > 0.0 && arc[i] < below[si] { (arc[i] / below[si]).powf(0.6) } else { 1.0 };
            let flare_k = if flare > 0.0 { (1.0 - t * 6.0).max(0.0).powf(1.6) * under } else { 0.0 };
            let sway = if st.root {
                0.0
            } else if st.depth == 0 || (st.fork && st.parent.is_some_and(|pi| sk.stems[pi].depth == 0)) {
                (st.points[i].y / height).clamp(0.0, 1.0).powf(2.0) * 0.6
            } else {
                (base_sway[si] + t * 0.25 * (st.depth as f32).min(2.0)).min(1.0)
            };
            // bark detail curves (along the stem or the tree's height)
            let hm = (st.points[i].y / height).clamp(0.0, 1.0);
            let flute_depth = flute_depth0 * gpar.cv("flute_depth", t, hm);
            let flatten = flatten0 * gpar.cv("flatten", t, hm);
            let bumps = bumps0 * gpar.cv("bumps", t, hm);
            for k in 0..=sides {
                // the seam vertex (k = sides) sits exactly on k = 0, so stitched joins stay closed
                let a = (k % sides) as f32 / sides as f32 * TAU;
                let (ca, sa) = (a.cos(), a.sin());
                let dirv = b1 * ca + b2 * sa;
                let mut r = st.radii[i];
                if root_flutes[si] {
                    // the trunk's flutes come from its roots: ridges run up from each root,
                    // grooves lie between neighbouring roots, fading out up the trunk
                    let fade = (1.0 - t / 0.35).max(0.0).powf(1.3) * under;
                    if fade > 0.0 && flute_depth > 0.0 {
                        let az = dirv.z.atan2(dirv.x);
                        r *= 1.0 - flute_depth * 0.6 * root_groove(&lobes_sorted[si], az).powf(1.5) * fade;
                    }
                } else if flutes > 0.0 {
                    r *= 1.0 - flute_depth * 0.5 * (1.0 - (flutes * a).cos());
                }
                if bumps > 0.0 {
                    let q = (st.points[i] + dirv * st.radii[i]) / bump_size;
                    r *= 1.0 + bumps * (noise3(q) * 0.7 + noise3(q * 2.3 + Vec3::splat(17.0)) * 0.3);
                }
                if flare_k > 0.0 {
                    let a = dirv.z.atan2(dirv.x);
                    let ridge = st.lobes.iter().map(|&az| (a - az).cos().max(0.0).powf(5.0)).fold(0.0f32, f32::max);
                    r *= 1.0 + flare * flare_k * (0.2 + 1.3 * ridge);
                }
                // flatten squashes the cross-section into an ellipse
                let offset = b1 * ca + b2 * (sa * (1.0 - flatten));
                let normal = (b1 * (ca * (1.0 - flatten)) + b2 * sa).normalize_or(dirv);
                m.pos.push(st.points[i] + offset * r);
                m.nrm.push(normal);
                m.stem.push(si as u32);
                m.col.push(c);
                m.uv.push([k as f32 / sides as f32 * tiling[0], vcoord * tiling[1]]);
                m.wind.push([sway, 0.0]);
                m.phase.push(st.phase);
                m.gen.push(st.gen);
        m.vmat.push(mat_id);
                m.leaf.push(false);
            }
        }
        // Along the stem the texture repeats once per circumference of the real bark (buttress
        // flare and flutes included), measured along the surface, so it keeps its proportions on
        // the flared trunk base too.
        {
            let rl = sides + 1;
            let at = |i: usize, k: usize| m.pos[base as usize + i * rl + k];
            let mean_r: Vec<f32> = (0..n).map(|i| (0..sides).map(|k| at(i, k).distance(st.points[i])).sum::<f32>() / sides as f32).collect();
            let mut v = 0.0f32;
            for i in 0..n {
                if i > 0 {
                    let d = (0..sides).map(|k| at(i, k).distance(at(i - 1, k))).sum::<f32>() / sides as f32;
                    let r = 0.5 * (mean_r[i] + mean_r[i - 1]);
                    v += d / (TAU * r.max(0.004));
                }
                for k in 0..rl {
                    m.uv[base as usize + i * rl + k][1] = v * tiling[1];
                }
            }
            vcoord = v;
        }
        let ring = (sides + 1) as u32;
        // tip cap
        let last = *st.points.last().unwrap();
        let tip = m.pos.len() as u32;
        m.pos.push(last + tan0 * st.radii[n - 1] * if st.broken { 0.12 } else { 1.0 });
        m.nrm.push(tan0);
        m.stem.push(si as u32);
        m.col.push(c);
        m.uv.push([0.5 * tiling[0], vcoord * tiling[1]]);
        let last_wind = *m.wind.last().unwrap();
        m.wind.push(last_wind);
        m.phase.push(st.phase);
        m.gen.push(st.gen);
        m.vmat.push(mat_id);
        m.leaf.push(false);
        grids.push(Grid { base, ring, sides: sides as u32, n: n as u32, start: 0, tip, welded: false });
        removed.push(vec![false; (n - 1) * sides]);
        if doc.settings.join_branches {
            if let Some(pi) = st.parent {
                let margin = 1.04 + doc.get(sk.stems[pi].gen).map(|g| g.params.bumps).unwrap_or(0.0);
                let weld = doc.get(st.gen).map(|g| g.params.weld).unwrap_or(0.0);
                match doc.settings.join_method {
                    JoinMethod::Stitch => {
                        // cross-fade the bark into the parent's where both wear the same material
                        let same = match (doc.get(st.gen), doc.get(sk.stems[pi].gen)) {
                            (Some(a), Some(b)) => a.material == b.material && a.blend == b.blend,
                            _ => false,
                        };
                        if let Some(s) = join_branch(&mut m, sk, &grids, &mut removed, margin, weld, si, pi, same) {
                            grids[si].start = s;
                        }
                    }
                    JoinMethod::Weld => {
                        grids[si].welded = weld_onto_parent(&mut m, sk, &grids, margin, weld, si, pi);
                    }
                }
            }
        }
    }
    // tube triangles, leaving out the openings where branches were joined
    for (si, g) in grids.iter().enumerate() {
        for i in g.start..g.n - 1 {
            for k in 0..g.sides {
                if removed[si][(i * g.sides + k) as usize] {
                    continue;
                }
                let a = g.base + i * g.ring + k;
                let b = a + 1;
                let c2 = a + g.ring;
                let d = c2 + 1;
                m.tris.push([a, b, c2]);
                m.tris.push([b, d, c2]);
            }
        }
        let lr = g.base + (g.n - 1) * g.ring;
        for k in 0..g.sides {
            m.tris.push([lr + k, lr + k + 1, g.tip]);
        }
        if g.start == 0 && !g.welded {
            // not stitched to a parent: close the base so the tube is never see-through
            let q = g.sides / 4;
            let c = push_blend(&mut m, [g.v(0, 0), g.v(0, q), g.v(0, 2 * q), g.v(0, 3 * q)], [0.25; 4]);
            let st = &sk.stems[si];
            m.pos[c as usize] = st.points[0];
            m.nrm[c as usize] = (st.points[0] - st.points[1]).normalize_or(Vec3::NEG_Y);
            for k in 0..g.sides {
                m.tris.push([c, g.base + k + 1, g.base + k]);
            }
        }
    }

    m.leaf_tri_start = m.tris.len();
    let mut rng = Rng::new(&[doc.settings.seed as u64, 77]);
    // leaf atlases: width / height of each atlas material's image
    let mut atlas_aspect: std::collections::HashMap<u32, f32> = std::collections::HashMap::new();
    for l in &sk.leaves {
        let first = m.pos.len();
        m.leaf_first.push(first as u32);
        // the picture of an atlas this leaf shows (picked by its place, so it stays the same
        // when other leaves come and go), and the picture's width / height
        let cell = doc.material(l.mat).filter(|mm| !mm.atlas.is_empty()).and_then(|mm| {
            let img = *atlas_aspect.entry(l.mat).or_insert_with(|| mm.texture.as_deref().and_then(|p| image::image_dimensions(p).ok()).map(|(w, h)| w as f32 / h.max(1) as f32).unwrap_or(1.0));
            let r = Rng::new(&[doc.settings.seed as u64, 313, l.pos.x.to_bits() as u64, l.pos.y.to_bits() as u64, l.pos.z.to_bits() as u64]).f();
            mm.atlas_pick(r).map(|c| (c.clone(), img * c.rect[2] / c.rect[3].max(1e-4)))
        });
        let to_atlas = |m: &mut Mesh| {
            if let Some((c, _)) = &cell {
                for t in &mut m.uv[first..] {
                    *t = c.uv(t[0], t[1]);
                }
            }
        };
        let st = &sk.stems[l.stem];
        let sway = (base_sway[l.stem] + 0.3).min(1.0);
        let c = l.color;
        let phase = st.phase + rng.f() * TAU;
        let side = l.normal.cross(l.dir).normalize_or(Vec3::X);
        // fold: the leaf halves rise along the midrib
        let (fc, fs) = (l.fold.cos(), l.fold.sin());
        let half = |sgn: f32| side * (sgn * fc) + l.normal * fs;
        let s = l.size;
        let w = s * l.aspect;
        let add = |m: &mut Mesh, p: Vec3, uv: [f32; 2], flutter: f32, nrm: Vec3| -> u32 {
            m.pos.push(p);
            m.nrm.push(nrm);
            m.col.push(c);
            m.uv.push(uv);
            m.wind.push([sway, flutter]);
            m.phase.push(phase);
            m.gen.push(l.gen);
            m.vmat.push(l.mat);
            m.stem.push(l.stem as u32);
            m.leaf.push(true);
            m.pos.len() as u32 - 1
        };
        let custom = if l.shape == LeafShape::Mesh {
            doc.get(l.gen).and_then(|g| g.params.leaf_mesh.as_deref()).and_then(|p| crate::leafmesh::load(p).ok())
        } else {
            None
        };
        if let Some(lm) = custom {
            let xs = s * l.aspect / 0.55;
            let base = m.pos.len() as u32;
            for k in 0..lm.pos.len() {
                let q = lm.pos[k];
                let n = lm.nrm[k];
                let p = l.pos + side * (q.x * xs) + l.dir * (q.y * s) + l.normal * (q.z * s);
                let nn = (side * (n.x / xs.max(1e-4)) + l.dir * (n.y / s.max(1e-4)) + l.normal * (n.z / s.max(1e-4))).normalize_or(l.normal);
                add(&mut m, p, lm.uv[k], q.y.clamp(0.0, 1.0), nn);
            }
            for t in &lm.tris {
                m.tris.push([base + t[0], base + t[1], base + t[2]]);
            }
            to_atlas(&mut m);
            continue;
        }
        match l.shape {
            LeafShape::Card | LeafShape::Mesh => {
                // plain rectangle: the leaf texture (with transparency) draws the leaf. With an
                // atlas the card has the picture's proportions and hangs from its pivot.
                let (wl, wr, below, above, um) = match &cell {
                    Some((c, asp)) => {
                        let wid = s * asp;
                        (c.pivot[0] * wid, (1.0 - c.pivot[0]) * wid, (1.0 - c.pivot[1]) * s, c.pivot[1] * s, c.pivot[0])
                    }
                    None => (s * l.aspect * 0.5, s * l.aspect * 0.5, 0.0, s, 0.5),
                };
                let tipdir = (l.dir + Vec3::NEG_Y * (l.bend * 0.5)).normalize_or(l.dir);
                let foot = l.pos - l.dir * below;
                let top = l.pos + tipdir * above;
                // midrib vertices so the card can fold
                let a = add(&mut m, foot + half(-1.0) * wl, [0.0, 0.0], 0.0, l.normal);
                let mb = add(&mut m, foot, [um, 0.0], 0.0, l.normal);
                let b = add(&mut m, foot + half(1.0) * wr, [1.0, 0.0], 0.0, l.normal);
                let c3 = add(&mut m, top + half(1.0) * wr, [1.0, 1.0], 1.0, l.normal);
                let mt = add(&mut m, top, [um, 1.0], 1.0, l.normal);
                let d2 = add(&mut m, top + half(-1.0) * wl, [0.0, 1.0], 1.0, l.normal);
                m.tris.push([a, mb, mt]);
                m.tris.push([a, mt, d2]);
                m.tris.push([mb, b, c3]);
                m.tris.push([mb, c3, mt]);
            }
            LeafShape::Needle => {
                // a tuft of thin needles fanning out from the twig
                let blades = 6;
                let wv = s * 0.035 * l.aspect / 0.55;
                for b in 0..blades {
                    let f = b as f32 / (blades - 1) as f32 * 2.0 - 1.0;
                    let lift = ((b * 7919) % 5) as f32 / 5.0 - 0.4;
                    let bd = (l.dir + side * f * 0.75 + l.normal * lift * 0.5).normalize_or(l.dir);
                    let bs = (bd.cross(l.normal)).normalize_or(side);
                    let len = s * (0.75 + 0.25 * (1.0 - f.abs()));
                    let a = add(&mut m, l.pos - bs * wv, [0.0, 0.0], 0.0, l.normal);
                    let b2 = add(&mut m, l.pos + bs * wv, [1.0, 0.0], 0.0, l.normal);
                    let tip = add(&mut m, l.pos + bd * len, [0.5, 1.0], 1.0, l.normal);
                    m.tris.push([a, b2, tip]);
                }
            }
            LeafShape::Broad | LeafShape::Narrow => {
                let wf = match l.shape {
                    LeafShape::Broad => 0.5,
                    _ => 0.22,
                } * l.aspect
                    / 0.55;
                let wv = s * wf.max(0.02);
                // curl the tip (leaf_bend) and fold the halves (leaf_fold)
                let tipdir = (l.dir + Vec3::NEG_Y * (l.bend * 0.8)).normalize_or(l.dir);
                let a = add(&mut m, l.pos, [0.5, 0.0], 0.0, l.normal);
                let b = add(&mut m, l.pos + l.dir * s * 0.4 + half(1.0) * wv * 0.5, [1.0, 0.4], 0.6, l.normal);
                let tip = add(&mut m, l.pos + l.dir * s * 0.45 + tipdir * s * 0.55, [0.5, 1.0], 1.0, l.normal);
                let d2 = add(&mut m, l.pos + l.dir * s * 0.4 + half(-1.0) * wv * 0.5, [0.0, 0.4], 0.6, l.normal);
                m.tris.push([a, b, tip]);
                m.tris.push([a, tip, d2]);
            }
            LeafShape::Round => {
                let ctr = l.pos + l.dir * s * 0.5;
                let cv = add(&mut m, ctr, [0.5, 0.5], 0.6, l.normal);
                let k = 7;
                let first = m.pos.len() as u32;
                for i in 0..k {
                    let a = i as f32 / k as f32 * TAU;
                    let across = a.sin() * w * 0.5 / 0.55 * 0.5;
                    let p = ctr + l.dir * (a.cos() * s * 0.5) + half(across.signum()) * across.abs();
                    add(&mut m, p, [0.5 + a.sin() * 0.5, 0.5 + a.cos() * 0.5], 0.8, l.normal);
                }
                for i in 0..k as u32 {
                    m.tris.push([cv, first + i, first + (i + 1) % k as u32]);
                }
            }
            LeafShape::Frond => {
                // arching rachis with pairs of leaflets
                let segs = 10;
                let mut p = l.pos;
                let mut d = l.dir;
                let flat_side = d.cross(Vec3::Y).normalize_or(Vec3::X);
                let step = s / segs as f32;
                let up = flat_side.cross(d).normalize_or(Vec3::Y);
                for i in 0..segs {
                    let t = i as f32 / segs as f32;
                    d = (d + Vec3::NEG_Y * (0.04 + 0.3 * l.bend)).normalize_or(d);
                    let np = p + d * step;
                    let lw = s * 0.28 * (PI * (0.1 + 0.9 * t)).sin().max(0.15) * l.aspect / 0.55;
                    for sgn in [-1.0f32, 1.0] {
                        let out = (flat_side * (sgn * fc) + up * fs + d * 0.6 + Vec3::NEG_Y * 0.35).normalize_or(flat_side);
                        let a = add(&mut m, p, [0.5, t], t, up);
                        let b = add(&mut m, np, [0.5, t + 0.1], t + 0.1, up);
                        let c3 = add(&mut m, np + out * lw, [0.5 + sgn * 0.5, t + 0.1], (t + 0.3).min(1.0), up);
                        m.tris.push([a, b, c3]);
                    }
                    p = np;
                }
            }
        }
        to_atlas(&mut m);
    }
    // final bounds from the real geometry (leaf tips and fronds included)
    m.height = m.pos.iter().fold(0.1f32, |a, p| a.max(p.y));
    m.radius = m.pos.iter().fold(0.1f32, |a, p| a.max((p.x * p.x + p.z * p.z).sqrt()));
    assign_layers(doc, sk, &below, &mut m);
    bake_ao(&mut m, doc.settings.ao_strength, level);
    m
}

/// Material gradients: the layer value of every stem vertex of a generator with a gradient
/// (see [`crate::model::MaterialBlend::layer_value`]), and its colour tint blended between the
/// layers' materials. Fillets and collars belong to the branch they join, so they take its
/// value.
fn assign_layers(doc: &Document, sk: &Skeleton, below: &[f32], m: &mut Mesh) {
    if !doc.generators.iter().any(|g| g.blend.is_some()) {
        return;
    }
    let n = m.pos.len();
    m.layer = vec![0.0; n];
    m.axis = vec![0.0; n];
    let bark = doc.settings.bark_color.map(|c| c as f32 / 255.0);
    let tint = |id: u32| doc.material(id).map(|mt| mt.color.map(|c| c as f32 / 255.0)).unwrap_or(bark);
    let tree_h = m.height.max(0.1);
    // arc length along each stem's polyline, for the Stem axis
    let arcs: Vec<Vec<f32>> = sk
        .stems
        .iter()
        .map(|s| {
            let mut a = vec![0.0f32];
            for w in s.points.windows(2) {
                a.push(a.last().unwrap() + w[0].distance(w[1]));
            }
            a
        })
        .collect();
    for i in 0..n {
        if m.leaf[i] || m.vmat[i] == crate::billboard::BILLBOARD_MAT {
            continue;
        }
        let gen = m.gen[i];
        let Some(b) = doc.get(gen).and_then(|g| g.blend.as_ref()) else { continue };
        if b.layers.is_empty() {
            continue;
        }
        let p = m.pos[i];
        let x = match b.axis {
            BlendAxis::Height => p.y,
            BlendAxis::HeightShare => p.y / tree_h,
            BlendAxis::Stem => {
                let si = m.stem[i] as usize;
                let Some(st) = sk.stems.get(si) else { continue };
                let arc = &arcs[si];
                let mut best = (f32::MAX, 0.0f32);
                for k in 0..st.points.len().saturating_sub(1) {
                    let (a, c) = (st.points[k], st.points[k + 1]);
                    let ac = c - a;
                    let t = ((p - a).dot(ac) / ac.length_squared().max(1e-12)).clamp(0.0, 1.0);
                    let d = p.distance_squared(a + ac * t);
                    if d < best.0 {
                        best = (d, arc[k] + t * (arc[k + 1] - arc[k]));
                    }
                }
                let total = arc.last().copied().unwrap_or(1.0);
                let b0 = below.get(si).copied().unwrap_or(0.0);
                ((best.1 - b0) / (total - b0).max(1e-6)).clamp(0.0, 1.0)
            }
        };
        let nz = if b.noise > 0.0 { blend_noise(p, b.noise_scale) } else { 0.0 };
        let l = b.layer_value(x, nz);
        m.layer[i] = l;
        m.axis[i] = x;
        if l > 0.0 {
            let k = l.floor() as u32;
            let f = l - k as f32;
            let t0 = tint(doc.layer_material_id(gen, 0));
            let (ta, tb) = (tint(doc.layer_material_id(gen, k)), tint(doc.layer_material_id(gen, k + 1)));
            for c in 0..3 {
                let want = ta[c] + (tb[c] - ta[c]) * f;
                if t0[c] > 1e-4 {
                    m.col[i][c] *= want / t0[c];
                } else {
                    m.col[i][c] = want;
                }
            }
        }
    }
}

/// Ambient occlusion from the tree's own geometry: triangles are splatted into a density
/// grid, then each vertex marches a few rays towards the sky and the sides. Dense crowns
/// and inner branches get darker; the result is multiplied into the vertex colours.
pub fn bake_ao(m: &mut Mesh, strength: f32, level: u32) {
    if strength <= 0.0 || m.pos.is_empty() {
        return;
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in &m.pos {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let res = if level >= 2 { 24.0 } else { 40.0 };
    let cell = ((hi - lo).max_element() / res).max(0.05);
    lo -= Vec3::splat(cell);
    let dims = (((hi - lo) / cell).ceil() + Vec3::ONE).as_uvec3();
    let (nx, ny, nz) = (dims.x as usize, dims.y as usize, dims.z as usize);
    let mut grid = vec![0.0f32; nx * ny * nz];
    let idx = |q: Vec3| -> Option<usize> {
        let g = (q - lo) / cell;
        if g.x < 0.0 || g.y < 0.0 || g.z < 0.0 {
            return None;
        }
        let (x, y, z) = (g.x as usize, g.y as usize, g.z as usize);
        (x < nx && y < ny && z < nz).then(|| (z * ny + y) * nx + x)
    };
    let vol = cell * cell * cell;
    for (ti, t) in m.tris.iter().enumerate() {
        let (a, b, c) = (m.pos[t[0] as usize], m.pos[t[1] as usize], m.pos[t[2] as usize]);
        // leaves let some light through; bark is opaque
        let opacity = if ti >= m.leaf_tri_start { 0.55 } else { 1.0 };
        let area = (b - a).cross(c - a).length() * 0.5 * opacity;
        if let Some(i) = idx((a + b + c) / 3.0) {
            grid[i] += area / vol;
        }
    }
    // a little blur so single triangles do not cast blocky shadows
    let mut blurred = grid.clone();
    for z in 1..nz.saturating_sub(1) {
        for y in 1..ny.saturating_sub(1) {
            for x in 1..nx.saturating_sub(1) {
                let i = (z * ny + y) * nx + x;
                let s = grid[i - 1] + grid[i + 1] + grid[i - nx] + grid[i + nx] + grid[i - nx * ny] + grid[i + nx * ny];
                blurred[i] = grid[i] * 0.4 + s * 0.1;
            }
        }
    }
    let grid = blurred;
    // sky-weighted directions (up, ring of 6 tilted up, ring of 4 sideways)
    let mut dirs: Vec<(Vec3, f32)> = vec![(Vec3::Y, 1.6)];
    for k in 0..6 {
        let a = k as f32 / 6.0 * TAU;
        dirs.push((Vec3::new(a.cos() * 0.6, 0.8, a.sin() * 0.6).normalize(), 1.2));
    }
    for k in 0..4 {
        let a = (k as f32 + 0.5) / 4.0 * TAU;
        dirs.push((Vec3::new(a.cos(), 0.15, a.sin()).normalize(), 0.7));
    }
    let steps = if level >= 2 { 10 } else { 16 };
    let absorb = 0.35 * cell;
    let mut ao = vec![1.0f32; m.pos.len()];
    for (i, out) in ao.iter_mut().enumerate() {
        let p = m.pos[i];
        let n = m.nrm[i];
        let leaf = m.leaf[i];
        let (mut sum, mut wsum) = (0.0f32, 0.0f32);
        for &(d, w) in &dirs {
            let facing = if leaf { 0.5 + 0.5 * d.dot(n).abs() } else { (0.25 + 0.75 * d.dot(n)).max(0.0) };
            let w = w * facing;
            if w <= 0.0 {
                continue;
            }
            let mut q = p + d * (cell * 1.1) + n * (cell * 0.3);
            let mut trans = 1.0f32;
            for _ in 0..steps {
                match idx(q) {
                    Some(gi) => trans *= (-grid[gi] * absorb).exp(),
                    None => break,
                }
                if trans < 0.02 {
                    break;
                }
                q += d * cell;
            }
            sum += trans * w;
            wsum += w;
        }
        let vis = if wsum > 0.0 { sum / wsum } else { 1.0 };
        // ground darkens the very bottom a little
        let ground = 1.0 - 0.25 * (1.0 - (p.y / (cell * 4.0)).clamp(0.0, 1.0));
        *out = (1.0 - strength * 0.8 * (1.0 - vis)).max(0.2) * ground;
    }
    for (c, a) in m.col.iter_mut().zip(ao) {
        *c = c.map(|x| x * a);
    }
}


/// Wind deformation shared by the viewport and the offscreen preview.
pub fn wind_offset(doc: &Document, m: &Mesh, i: usize, time: f32) -> Vec3 {
    let w = &doc.settings.wind;
    if w.strength <= 0.0 {
        return Vec3::ZERO;
    }
    let [sway, flutter] = m.wind[i];
    let ph = m.phase[i];
    let t = time * w.speed;
    let dir = Vec3::new(w.direction.to_radians().cos(), 0.0, w.direction.to_radians().sin());
    let gust = 1.0 + w.gustiness * (0.6 * (t * 0.7 + ph * 0.2).sin() + 0.4 * (t * 2.3 + ph).sin());
    let bend = sway * w.strength * m.height * 0.035 * (0.7 + 0.3 * (t * 1.6 + ph * 0.5).sin()) * gust;
    let mut off = dir * bend + Vec3::NEG_Y * bend.abs() * 0.15 * sway;
    if flutter > 0.0 {
        off += m.nrm[i] * (flutter * w.strength * 0.06 * (t * 9.0 + ph * 3.0).sin());
    }
    off
}

/// The noise mask of material gradients at world position `p` (-1..1), patches about `scale`
/// metres across. The viewport shader computes the same function per pixel (see gpu.rs), and
/// exported files describe it, so engines can reproduce it.
pub fn blend_noise(p: Vec3, scale: f32) -> f32 {
    let q = p / scale.max(0.01);
    let a = value_noise(q);
    let b = value_noise(q * 2.7 + Vec3::splat(31.0));
    (((a * 0.65 + b * 0.35) * 2.0 - 1.0) * 1.6).clamp(-1.0, 1.0)
}

/// Hash of a lattice point to 0..1 (float arithmetic only, the same in the shader).
fn hash13(p: Vec3) -> f32 {
    let fr = |x: f32| x - x.floor();
    let mut q = Vec3::new(fr(p.x * 0.1031), fr(p.y * 0.1031), fr(p.z * 0.1031));
    let d = q.dot(Vec3::new(q.y, q.z, q.x) + Vec3::splat(31.32));
    q += Vec3::splat(d);
    fr((q.x + q.y) * q.z)
}

/// Smooth value noise in 0..1.
fn value_noise(p: Vec3) -> f32 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (Vec3::splat(3.0) - f * 2.0);
    let h = |x: f32, y: f32, z: f32| hash13(i + Vec3::new(x, y, z));
    let x00 = h(0.0, 0.0, 0.0) + (h(1.0, 0.0, 0.0) - h(0.0, 0.0, 0.0)) * u.x;
    let x10 = h(0.0, 1.0, 0.0) + (h(1.0, 1.0, 0.0) - h(0.0, 1.0, 0.0)) * u.x;
    let x01 = h(0.0, 0.0, 1.0) + (h(1.0, 0.0, 1.0) - h(0.0, 0.0, 1.0)) * u.x;
    let x11 = h(0.0, 1.0, 1.0) + (h(1.0, 1.0, 1.0) - h(0.0, 1.0, 1.0)) * u.x;
    let y0 = x00 + (x10 - x00) * u.y;
    let y1 = x01 + (x11 - x01) * u.y;
    y0 + (y1 - y0) * u.z
}

fn hash3(x: i32, y: i32, z: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ (z as u32).wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    h as f32 / u32::MAX as f32 * 2.0 - 1.0
}

/// Smooth 3D value noise in -1..1 (for bark lumps).
fn noise3(p: Vec3) -> f32 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (Vec3::splat(3.0) - f * 2.0);
    let (x, y, z) = (i.x as i32, i.y as i32, i.z as i32);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let c = |dx: i32, dy: i32, dz: i32| hash3(x + dx, y + dy, z + dz);
    lerp(
        lerp(lerp(c(0, 0, 0), c(1, 0, 0), u.x), lerp(c(0, 1, 0), c(1, 1, 0), u.x), u.y),
        lerp(lerp(c(0, 0, 1), c(1, 0, 1), u.x), lerp(c(0, 1, 1), c(1, 1, 1), u.x), u.y),
        u.z,
    )
}
