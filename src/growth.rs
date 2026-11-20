//! Growth animation: the tree from nothing to the mature tree, on one mesh whose vertices move.
//!
//! The topology never changes, so an engine can play the growth as morph targets (blend
//! shapes) between a handful of sampled shapes. Growth `g` runs from 0 (nothing above the
//! ground) to 1 (the mature tree as designed). It follows how tree modelling tools commonly
//! animate growth:
//!
//! * every stem grows out along its final path, from its base to its tip, at a speed (its
//!   generator's speed scale times the trunk's pace), so a longer branch takes longer; the part
//!   not grown yet is gathered at the growing tip, which tapers to a point;
//! * its radius grows over the stem's own growth (a radius curve), and a branch is never
//!   thicker than its parent where it comes out;
//! * by default the whole tree grows at once: a branch starts where its parent has grown past
//!   it and finishes with the parent; leaves come out with the growing shoots and ride the
//!   nearest growing tip until their twig reaches them, so there are leaves at every stage;
//! * other syncs start a part Start % of the way through the parent's growth, together with
//!   its siblings, or exactly when the parent reaches its spot, but never before the parent has
//!   grown that far; an offset with a random variation spreads neighbours out;
//! * branches can fold out of their parent and leaves turn into place (start angle);
//! * parts stay where they are in the mature tree: nothing slides or scales around, so joints
//!   stay closed (the opening in a parent's bark is as wide as the branch is thick).
//!
//! The settings are per generator (`GenParams::growth`); the whole growth is normalised so it
//! ends at 1 when the last part is done.

use crate::grow::{Mesh, Skeleton, Stem};
use crate::model::{Document, GenGrowth, GenKind, GrowthCurve, GrowthSync};
use glam::{Quat, Vec3};
use serde::Serialize;
use std::collections::HashMap;

/// A leaf unfolds in this share of the trunk's growth time at speed 1.
const LEAF_TIME: f32 = 0.15;
/// A branch starts once its parent has grown this many times its base radius past where it
/// comes out, so its joint collar is never above the parent's growing tip.
const JOINT_REACH: f32 = 2.5;
/// A growing tip is a cone this many times as long as the stem is thick there.
const TIP_SLOPE: f32 = 2.5;

/// A fixed pseudo-random number in 0..1 for `i`.
fn hash01(i: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// Closest point on a stem's axis to `q`: (arc-length fraction 0..1, the point).
pub fn locate(st: &Stem, q: Vec3) -> (f32, Vec3) {
    locate_on(&st.points, q)
}

fn locate_on(points: &[Vec3], q: Vec3) -> (f32, Vec3) {
    if points.len() < 2 {
        return (0.0, points.first().copied().unwrap_or(Vec3::ZERO));
    }
    let (mut best, mut best_arc, mut best_p) = (f32::MAX, 0.0f32, points[0]);
    let mut acc = 0.0f32;
    for w in points.windows(2) {
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

fn polyline_length(st: &Stem) -> f32 {
    st.points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// For a stem growing out of a parent: the arc fraction of its first axis point outside the
/// parent's bark. The part before it is the joint (inside the parent, welded to it): it is
/// there as soon as the stem starts, and the stem grows out from there.
fn joint_fraction(sk: &Skeleton, i: usize) -> f32 {
    let st = &sk.stems[i];
    let Some(p) = st.parent else { return 0.0 };
    let par = &sk.stems[p];
    let (a, _) = locate(par, st.points[0]);
    let rp = radius_at(par, &Axis::new(par), a);
    let total = polyline_length(st).max(1e-6);
    let mut acc = 0.0f32;
    for (k, q) in st.points.iter().enumerate() {
        if k > 0 {
            acc += st.points[k - 1].distance(*q);
        }
        let (_, on) = locate(par, *q);
        if on.distance(*q) >= rp {
            return (acc / total).min(0.5);
        }
    }
    0.5
}

/// The growth settings of generator `gen` (defaults when it is gone).
fn settings(doc: &Document, gen: u32) -> (GenGrowth, GenKind) {
    doc.get(gen).map(|g| (g.params.growth.clone(), g.kind)).unwrap_or_else(|| (GenGrowth::default(), GenKind::Branches))
}

/// When every stem and leaf grows, as fractions of the whole growth (0..1): each starts at
/// its start and is done after its time. The last part is done at 1.
#[derive(Clone, Serialize, Debug, Default)]
pub struct Timing {
    pub stem_start: Vec<f32>,
    pub stem_time: Vec<f32>,
    pub leaf_start: Vec<f32>,
    pub leaf_time: Vec<f32>,
}

/// One part's growth: (start, time), and when its parent reaches a point.
struct Clock<'a> {
    start: &'a [f32],
    time: &'a [f32],
    curve: &'a [GrowthCurve],
    size0: &'a [f32],
    joint: &'a [f32],
}

impl Clock<'_> {
    /// When stem `p` has grown out to arc fraction `a`.
    fn reaches(&self, p: usize, a: f32) -> f32 {
        let (ss, j) = (self.size0[p], self.joint[p]);
        let a = ((a - j) / (1.0 - j).max(1e-6)).clamp(0.0, 1.0);
        if a <= ss {
            return self.start[p];
        }
        let y = ((a - ss) / (1.0 - ss).max(1e-6)).clamp(0.0, 1.0);
        self.start[p] + self.time[p] * self.curve[p].inverse(y)
    }
}

impl Timing {
    pub fn new(doc: &Document, sk: &Skeleton) -> Timing {
        let n = sk.stems.len();
        let len: Vec<f32> = sk.stems.iter().map(polyline_length).collect();
        // the trunk's pace: the first trunk grows in 1 at speed 1
        let leader = sk.stems.iter().position(|s| s.parent.is_none() && !s.root);
        let pace = leader.map(|i| len[i]).or_else(|| len.iter().copied().reduce(f32::max)).unwrap_or(1.0).max(0.01);
        let gs: Vec<(GenGrowth, GenKind)> = sk.stems.iter().map(|s| settings(doc, s.gen)).collect();
        let mut start = vec![0.0f32; n];
        let mut time: Vec<f32> = (0..n).map(|i| (len[i] / (pace * gs[i].0.speed)).max(1e-3)).collect();
        let curve: Vec<GrowthCurve> = gs.iter().map(|g| g.0.speed_curve).collect();
        let size0: Vec<f32> = gs.iter().map(|g| g.0.start_size).collect();
        let joint: Vec<f32> = (0..n).map(|i| joint_fraction(sk, i)).collect();
        // a stem starts once its parent has grown past its joint: where it comes out plus
        // the collar around it (JOINT_REACH times its base radius along the parent)
        let attach: Vec<f32> = sk
            .stems
            .iter()
            .map(|st| match st.parent {
                Some(p) => {
                    let a = locate(&sk.stems[p], st.points[0]).0;
                    let rc = st.radii.first().copied().unwrap_or(0.0);
                    (a + JOINT_REACH * rc / len[p].max(1e-6)).min(1.0)
                }
                None => 0.0,
            })
            .collect();
        // generators in tree order, so parents are timed before their children
        let mut order: Vec<u32> = doc.ordered().iter().map(|&(id, _)| id).collect();
        for st in &sk.stems {
            if !order.contains(&st.gen) {
                order.push(st.gen);
            }
        }
        let mut done = vec![false; n];
        for gen in &order {
            let mine: Vec<usize> = (0..n).filter(|&i| sk.stems[i].gen == *gen).collect();
            // stems of one generator can grow on each other (rarely); time parents first
            let mut pending = mine.clone();
            let mut rounds = 0;
            while !pending.is_empty() && rounds < 64 {
                rounds += 1;
                let ready: Vec<usize> = pending.iter().copied().filter(|&i| sk.stems[i].parent.map_or(true, |p| done[p] || sk.stems[p].gen != *gen)).collect();
                let ready = if ready.is_empty() { pending.clone() } else { ready };
                pending.retain(|i| !ready.contains(i));
                let clock = Clock { start: &start, time: &time, curve: &curve, size0: &size0, joint: &joint };
                let mut plan: Vec<(usize, f32, f32)> = vec![];
                // siblings start together: as early and as late as all of them can
                let g0 = &gs[ready[0]].0;
                let place = |i: usize| sk.stems[i].parent.map(|p| clock.reaches(p, attach[i])).unwrap_or(0.0);
                let parent_end = |i: usize| sk.stems[i].parent.map(|p| start[p] + time[p]).unwrap_or(0.0);
                let sync = g0.sync;
                let (early, late) = (ready.iter().map(|&i| place(i)).fold(0.0f32, f32::max), ready.iter().map(|&i| parent_end(i)).fold(0.0f32, f32::max));
                for &i in &ready {
                    let g = &gs[i].0;
                    let pl = place(i);
                    let (mut s, mut d) = (pl, time[i]);
                    if let Some(p) = sk.stems[i].parent {
                        match sync {
                            // grow with the parent: start where it reaches them and take the
                            // parent's remaining time (divided by the speed), so the whole tree
                            // grows at once, the way every shoot of a real tree grows each year
                            GrowthSync::Auto => {
                                s = pl.max(start[p]);
                                d = ((start[p] + time[p] - s) / g.speed).max(0.25 * time[i]);
                            }
                            GrowthSync::Parent => s = pl.max(start[p] + g.start_pct * time[p]),
                            GrowthSync::InPlace => s = pl,
                            GrowthSync::Siblings => s = (early + g.start_pct * (late - early)).max(pl),
                            GrowthSync::ParentStartEnd => {
                                s = pl.max(start[p]);
                                d = (start[p] + time[p] - s).max(0.25 * time[i]);
                            }
                        }
                        s += g.offset + g.offset_variation * hash01(2 * i as u32);
                    }
                    plan.push((i, s, d));
                }
                for (i, s, d) in plan {
                    start[i] = s;
                    time[i] = d;
                    done[i] = true;
                }
            }
        }
        // leaves
        let clock = Clock { start: &start, time: &time, curve: &curve, size0: &size0, joint: &joint };
        let nl = sk.leaves.len();
        let lg: Vec<(GenGrowth, GenKind)> = sk.leaves.iter().map(|l| settings(doc, l.gen)).collect();
        let lplace: Vec<f32> = sk.leaves.iter().map(|l| if l.stem < n { clock.reaches(l.stem, locate(&sk.stems[l.stem], l.pos).0) } else { 0.0 }).collect();
        let mut by_gen: HashMap<u32, (f32, f32)> = HashMap::new();
        for (i, l) in sk.leaves.iter().enumerate() {
            let e = by_gen.entry(l.gen).or_insert((0.0, 0.0));
            e.0 = e.0.max(lplace[i]);
            if l.stem < n {
                e.1 = e.1.max(start[l.stem] + time[l.stem]);
            }
        }
        let mut leaf_start = vec![0.0f32; nl];
        let mut leaf_time = vec![0.0f32; nl];
        for (i, l) in sk.leaves.iter().enumerate() {
            let g = &lg[i].0;
            let pl = lplace[i];
            let mut d = LEAF_TIME / g.speed;
            let mut s = pl;
            if l.stem < n {
                let p = l.stem;
                match g.sync {
                    // come out with the growing shoots: most when their twig starts, some with
                    // the branch below it and a few with the young trunk; until their twig has
                    // grown to their place they ride the nearest growing tip
                    GrowthSync::Auto => {
                        let h = hash01(7919 * i as u32 + 3);
                        let mut chain = vec![p];
                        while let Some(q) = sk.stems[*chain.last().unwrap()].parent {
                            chain.push(q);
                        }
                        let up = if h < SEEDLING_LEAVES / nl.max(1) as f32 { chain.len() - 1 } else if h < EARLY_LEAVES { 1.min(chain.len() - 1) } else { 0 };
                        s = start[chain[up]];
                    }
                    GrowthSync::Parent => s = pl.max(start[p] + g.start_pct * time[p]),
                    GrowthSync::InPlace => s = pl,
                    GrowthSync::Siblings => {
                        let (early, late) = by_gen[&l.gen];
                        s = (early + g.start_pct * (late - early)).max(pl);
                    }
                    GrowthSync::ParentStartEnd => {
                        s = pl.max(start[p]);
                        d = (start[p] + time[p] - s).max(0.25 * d);
                    }
                }
            }
            s += g.offset + g.offset_variation * hash01(2 * i as u32 + 1);
            leaf_start[i] = s;
            leaf_time[i] = d.max(1e-3);
        }
        // normalise: the last part is done at 1
        let end = (0..n).map(|i| start[i] + time[i]).chain((0..nl).map(|i| leaf_start[i] + leaf_time[i])).fold(1e-3f32, f32::max);
        for v in start.iter_mut().chain(time.iter_mut()).chain(leaf_start.iter_mut()).chain(leaf_time.iter_mut()) {
            *v /= end;
        }
        Timing { stem_start: start, stem_time: time, leaf_start, leaf_time }
    }

    /// How far stem or leaf with (`start`, `time`) is through its own growth at `g` (None
    /// before it starts).
    fn progress(start: f32, time: f32, g: f32) -> Option<f32> {
        if g < start {
            return None;
        }
        Some(((g - start) / time.max(1e-6)).clamp(0.0, 1.0))
    }
}

/// With Auto, this share of the leaves comes out with the branch their twig grows on (riding
/// its growing tip), rather than with their own twig.
const EARLY_LEAVES: f32 = 0.15;
/// With Auto, about this many leaves come out with the young trunk, so even a seedling has
/// leaves.
const SEEDLING_LEAVES: f32 = 6.0;

/// The growth points the animation is sampled at: an even spread plus every stage, always
/// including 0 and 1.
pub fn keyframes(growth: &crate::model::Growth) -> Vec<f32> {
    let mut special: Vec<f32> = vec![0.0, 1.0];
    special.extend(growth.stages.iter().map(|s| s.at.clamp(0.0, 1.0)));
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
    /// Grows with the first trunk only (billboard cards, anything without a stem).
    Fixed,
    /// On stem `s` at arc fraction `t`, whose axis passes `c` there.
    Stem { s: u32, t: f32, c: Vec3 },
    /// Part of the joint where branch `child` comes out of stem `s` (the opening's rim in the
    /// parent's bark and the branch's rings inside the parent), at arc fraction `t` of the
    /// parent whose axis passes `c` there.
    Joint { s: u32, t: f32, c: Vec3, child: u32 },
    /// Part of leaf `l`.
    Leaf { l: u32 },
}

/// A stem's final axis, for finding points along it.
struct Axis {
    points: Vec<Vec3>,
    /// Arc-length fraction at each point (0..1).
    arc: Vec<f32>,
}

impl Axis {
    fn new(st: &Stem) -> Axis {
        let mut arc = vec![0.0f32];
        for w in st.points.windows(2) {
            arc.push(arc.last().unwrap() + w[0].distance(w[1]));
        }
        let total = arc.last().copied().unwrap_or(0.0).max(1e-6);
        for a in &mut arc {
            *a /= total;
        }
        Axis { points: st.points.clone(), arc }
    }

    fn seg(&self, t: f32) -> (usize, f32) {
        let n = self.points.len();
        if n < 2 {
            return (0, 0.0);
        }
        let i = self.arc.partition_point(|&a| a <= t).clamp(1, n - 1);
        let span = self.arc[i] - self.arc[i - 1];
        (i, if span > 0.0 { ((t - self.arc[i - 1]) / span).clamp(0.0, 1.0) } else { 0.0 })
    }

    fn at(&self, t: f32) -> Vec3 {
        if self.points.len() < 2 {
            return self.points.first().copied().unwrap_or(Vec3::ZERO);
        }
        let (i, f) = self.seg(t);
        self.points[i - 1].lerp(self.points[i], f)
    }

    fn tangent(&self, t: f32) -> Vec3 {
        if self.points.len() < 2 {
            return Vec3::Y;
        }
        let (i, _) = self.seg(t);
        (self.points[i] - self.points[i - 1]).normalize_or(Vec3::Y)
    }
}

/// A stem's radius at arc fraction `t`.
fn radius_at(st: &Stem, ax: &Axis, t: f32) -> f32 {
    if st.radii.len() < 2 || ax.points.len() < 2 {
        return st.radii.first().copied().unwrap_or(0.0);
    }
    let (i, f) = ax.seg(t);
    let i = i.min(st.radii.len() - 1);
    st.radii[i - 1] + (st.radii[i] - st.radii[i - 1]) * f
}

/// What stays fixed about one stem while it grows.
struct StemInfo {
    axis: Axis,
    base: Vec3,
    /// Parent, arc fraction and axis point there.
    parent: Option<(usize, f32, Vec3)>,
    /// Folding out of the parent: axis and full angle (radians).
    fold: (Vec3, f32),
    /// Final radius at each axis point, and the length in metres.
    radii: Vec<f32>,
    length: f32,
    /// Arc fraction where it comes out of its parent's bark (see `joint_fraction`).
    joint: f32,
    /// A root: grows with its parent, so its joint keeps the parent's proportions.
    root: bool,
    g: GenGrowth,
}

impl StemInfo {
    /// The final radius at arc fraction `t`.
    fn final_radius(&self, t: f32) -> f32 {
        if self.radii.len() < 2 || self.axis.points.len() < 2 {
            return self.radii.first().copied().unwrap_or(0.0);
        }
        let (i, f) = self.axis.seg(t);
        let i = i.min(self.radii.len() - 1);
        self.radii[i - 1] + (self.radii[i] - self.radii[i - 1]) * f
    }

    /// Where the final stem's arc fraction `t` is now and how much of its final radius it has:
    /// the stem's own taper, from base to tip, spread over the length grown so far, times the
    /// radius growth.
    fn factor(&self, st: &State, t: f32) -> (f32, f32) {
        let at = st.map(t);
        if st.len <= 0.0 || t > st.len {
            return (at, 0.0);
        }
        let want = self.final_radius((at / st.len).min(1.0));
        let have = self.final_radius(t);
        let shape = if have > 1e-6 { (want / have).clamp(0.0, 1.0) } else { 0.0 };
        let r = st.radius * shape;
        // the growing tip is a short cone (TIP_SLOPE times as long as the stem is thick there),
        // which turns into the stem's own tip shape as it finishes
        let radius_m = have * r;
        let behind = (st.len - at) * self.length;
        let cone = if radius_m > 1e-6 { (behind / (TIP_SLOPE * radius_m)).clamp(0.0, 1.0) } else { 1.0 };
        let cone = cone * (2.0 - cone);
        let done = st.x.powi(4);
        (at, r * (cone + (1.0 - cone) * done))
    }

    /// Point `q` of the final stem (axis point `c` at arc fraction `t`) where it is now, before
    /// the stem's own origin and turn.
    fn moved(&self, st: &State, t: f32, c: Vec3, q: Vec3) -> Vec3 {
        let (at, r) = self.factor(st, t);
        if (at - t).abs() < 1e-6 {
            return c + (q - c) * r;
        }
        self.axis.at(at)
    }
}

/// One stem at one point of the growth.
#[derive(Clone, Copy)]
struct State {
    /// Grown length (arc fraction), radius factor, and how far through its own growth.
    len: f32,
    radius: f32,
    x: f32,
    /// Where its base is, and how it is turned.
    origin: Vec3,
    rot: Quat,
}

impl State {
    /// Where the part of the final stem at arc fraction `t` is now, as an arc fraction along
    /// the final path: the grown part keeps its place, the rest waits at the growing tip.
    fn map(&self, t: f32) -> f32 {
        t.clamp(0.0, self.len.max(0.0))
    }
}

struct LeafInfo {
    stem: usize,
    t: f32,
    axis: Vec3,
    anchor: Vec3,
    /// Turning the leaf from lying along its stem into place.
    fold: Quat,
    g: GenGrowth,
}

/// Where a branch comes out of its parent. The opening's rim, the strip stitching it to the
/// branch and the branch's rings up to that strip all close towards the opening's centre
/// together (by the same factor, in the parent's frame), so none of them turn inside out.
#[derive(Clone, Copy)]
struct Opening {
    /// The opening's centre on the parent's bark (the middle of its rim), and the parent's
    /// arc fraction and axis point there.
    centre: Vec3,
    t: f32,
    c: Vec3,
    /// The rim's mean distance from the parent's axis.
    d: f32,
    /// The branch's axis at its last ring that closes with the opening, and the parent's arc
    /// fraction and axis point there.
    anchor: Vec3,
    at: f32,
    ac: Vec3,
}

/// Moves the vertices of one mesh (built from `sk`) to any point of the growth.
pub struct Grower {
    pub timing: Timing,
    base: Vec<Vec3>,
    bind: Vec<Bind>,
    stems: Vec<StemInfo>,
    /// Per stem: the opening it comes out of in its parent's bark.
    openings: Vec<Option<Opening>>,
    leaves: Vec<LeafInfo>,
    leader: Option<usize>,
}

impl Grower {
    /// `timing` should come from the full tree so every level of detail grows alike; the
    /// levels share its stems (reduced meshes keep every stem, but only some leaves).
    pub fn new(doc: &Document, sk: &Skeleton, m: &Mesh, timing: &Timing) -> Grower {
        let mut timing = timing.clone();
        if timing.stem_start.len() != sk.stems.len() || timing.leaf_start.len() != sk.leaves.len() {
            let own = Timing::new(doc, sk);
            if timing.stem_start.len() != sk.stems.len() {
                timing = own;
            } else {
                // same stems, fewer leaves: keep the stems' timing, time these leaves alone
                timing.leaf_start = own.leaf_start;
                timing.leaf_time = own.leaf_time;
            }
        }
        let mut bind = vec![Bind::Fixed; m.pos.len()];
        for (li, &first) in m.leaf_first.iter().enumerate() {
            let end = m.leaf_first.get(li + 1).copied().unwrap_or(m.pos.len() as u32);
            for v in first..end {
                bind[v as usize] = Bind::Leaf { l: li as u32 };
            }
        }
        let axes: Vec<Axis> = sk.stems.iter().map(Axis::new).collect();
        // tube rings sit at the axis points: a vertex belongs to the ring nearest to where it
        // projects, so a whole ring appears at once even where the stem curves
        let ring = |s: usize, q: Vec3| -> (f32, Vec3) {
            let (t, _) = locate(&sk.stems[s], q);
            let ax = &axes[s];
            match ax.arc.iter().enumerate().min_by(|a, b| (a.1 - t).abs().total_cmp(&(b.1 - t).abs())) {
                Some((i, &a)) => (a, ax.points[i]),
                None => locate(&sk.stems[s], q),
            }
        };
        for v in 0..m.pos.len() {
            if matches!(bind[v], Bind::Leaf { .. }) || m.leaf.get(v).copied().unwrap_or(false) {
                continue;
            }
            let s = m.stem.get(v).copied().unwrap_or(u32::MAX);
            if (s as usize) < sk.stems.len() {
                let (t, c) = ring(s as usize, m.pos[v]);
                bind[v] = Bind::Stem { s, t, c };
            }
        }
        // joints: the opening's rim in the parent's bark and the branch's rings inside the
        // parent stay on the parent's bark and close towards the opening's centre while the
        // branch is thinner (relative to its final size) than the parent around it
        let mut joints: Vec<f32> = (0..sk.stems.len()).map(|i| joint_fraction(sk, i)).collect();
        // the branch ring each opening is stitched to closes with the opening
        let mut rim_of: HashMap<u32, u32> = HashMap::new();
        let mut rim_sum: Vec<(Vec3, f32, u32)> = vec![(Vec3::ZERO, 0.0, 0); sk.stems.len()];
        for &(v, child) in &m.joint_rim {
            if (child as usize) < sk.stems.len() && (v as usize) < m.pos.len() {
                rim_of.insert(v, child);
                let q = m.pos[v as usize];
                let d = sk.stems[child as usize].parent.map_or(0.0, |p| locate(&sk.stems[p], q).1.distance(q));
                let e = &mut rim_sum[child as usize];
                (e.0, e.1, e.2) = (e.0 + q, e.1 + d, e.2 + 1);
            }
        }
        for t in &m.tris[..m.leaf_tri_start.min(m.tris.len())] {
            let Some(child) = t.iter().find_map(|v| rim_of.get(v)).copied() else { continue };
            for &v in t {
                if let (false, Bind::Stem { s, t, .. }) = (rim_of.contains_key(&v), bind[v as usize]) {
                    if s == child && t < 0.95 {
                        joints[s as usize] = joints[s as usize].max(t);
                    }
                }
            }
        }
        // a branch whose opening touches the first ring past its parent's joint (a twig right at
        // the base of a branch): the parent's joint takes in that opening, so both move alike
        let mut span: Vec<(f32, f32)> = vec![(f32::MAX, f32::MIN); sk.stems.len()];
        for (&v, &child) in &rim_of {
            if let Some(p) = sk.stems[child as usize].parent {
                let t = locate(&sk.stems[p], m.pos[v as usize]).0;
                let e = &mut span[child as usize];
                (e.0, e.1) = (e.0.min(t), e.1.max(t));
            }
        }
        for y in 0..sk.stems.len() {
            let (Some(x), (lo, hi)) = (sk.stems[y].parent, span[y]) else { continue };
            if sk.stems[x].parent.is_none() || lo > hi {
                continue;
            }
            let arc = &axes[x].arc;
            let next = arc.iter().copied().find(|&a| a > joints[x] + 1e-6).unwrap_or(1.0);
            if lo <= next + 1e-6 {
                let cover = arc.iter().copied().find(|&a| a >= hi).unwrap_or(1.0);
                if cover < 0.6 {
                    joints[x] = joints[x].max(cover);
                }
            }
        }
        let mut claimed = vec![false; m.pos.len()];
        let on_parent = |child: usize, q: Vec3| -> Option<Bind> {
            let p = sk.stems[child].parent?;
            let (t, c) = locate(&sk.stems[p], q);
            Some(Bind::Joint { s: p as u32, t, c, child: child as u32 })
        };
        for &(v, child) in &m.joint_rim {
            let (v, child) = (v as usize, child as usize);
            if child >= sk.stems.len() || v >= m.pos.len() || claimed[v] {
                continue;
            }
            if let (Bind::Stem { s, .. }, Some(p)) = (bind[v], sk.stems[child].parent) {
                if s as usize == p {
                    if let Some(b) = on_parent(child, m.pos[v]) {
                        bind[v] = b;
                        claimed[v] = true;
                    }
                }
            }
        }
        for v in 0..m.pos.len() {
            if let Bind::Stem { s, t, .. } = bind[v] {
                let s = s as usize;
                if sk.stems[s].parent.is_some() && t < joints[s] + 1e-6 {
                    if let Some(b) = on_parent(s, m.pos[v]) {
                        bind[v] = b;
                    }
                }
            }
        }
        let openings: Vec<Option<Opening>> = (0..sk.stems.len())
            .map(|i| {
                let p = sk.stems[i].parent?;
                let anchor = axes[i].at(joints[i]);
                let (sum, dsum, n) = rim_sum[i];
                let mid = if n > 0 { sum / n as f32 } else { anchor };
                let (t, c) = locate(&sk.stems[p], mid);
                // the rim's middle lies inside the parent where the rim wraps around it: put the
                // centre out on the bark
                let d = if n > 0 { dsum / n as f32 } else { mid.distance(c) };
                let out = (mid - c).try_normalize().unwrap_or_else(|| axes[p].tangent(t).any_orthonormal_vector());
                let centre = c + out * d;
                let (at, ac) = locate(&sk.stems[p], anchor);
                Some(Opening { centre, t, c, d, anchor, at, ac })
            })
            .collect();
        let stems: Vec<StemInfo> = sk
            .stems
            .iter()
            .enumerate()
            .map(|(i, st)| {
                let base = st.points.first().copied().unwrap_or(Vec3::ZERO);
                let (parent, fold) = match st.parent {
                    Some(p) => {
                        let (a, c) = locate(&sk.stems[p], base);
                        let tp = axes[p].tangent(a);
                        let d = axes[i].tangent(0.0);
                        let angle = tp.angle_between(d);
                        let axis = tp.cross(d).try_normalize().unwrap_or_else(|| tp.any_orthonormal_vector());
                        (Some((p, a, c)), (axis, if angle.is_finite() { angle } else { 0.0 }))
                    }
                    None => (None, (Vec3::X, 0.0)),
                };
                StemInfo { axis: Axis::new(st), base, parent, fold, radii: st.radii.clone(), length: polyline_length(st), joint: joints[i], root: st.root, g: settings(doc, st.gen).0 }
            })
            .collect();
        let leaves = sk
            .leaves
            .iter()
            .map(|l| {
                let stem = l.stem.min(sk.stems.len().saturating_sub(1));
                let (t, axis) = if sk.stems.is_empty() { (0.0, l.pos) } else { locate(&sk.stems[stem], l.pos) };
                let along = if sk.stems.is_empty() { Vec3::Y } else { axes[stem].tangent(t) };
                let dir = l.dir.normalize_or(Vec3::Y);
                LeafInfo { stem, t, axis, anchor: l.pos, fold: Quat::from_rotation_arc(dir, along), g: settings(doc, l.gen).0 }
            })
            .collect();
        let leader = sk.stems.iter().position(|s| s.parent.is_none() && !s.root);
        Grower { timing, base: m.pos.clone(), bind, stems, openings, leaves, leader }
    }

    /// Every stem's state at growth `g`.
    fn states(&self, g: f32) -> Vec<State> {
        let n = self.stems.len();
        let mut out: Vec<Option<State>> = vec![None; n];
        fn state(i: usize, g: f32, gr: &Grower, out: &mut [Option<State>]) -> State {
            if let Some(s) = out[i] {
                return s;
            }
            let si = &gr.stems[i];
            let gs = &si.g;
            let x = Timing::progress(gr.timing.stem_start[i], gr.timing.stem_time[i], g);
            let (len, mut radius, fold) = match x {
                // not started: everything past the joint waits where it comes out
                None => (if si.parent.is_some() { si.joint } else { 0.0 }, 0.0, gs.start_angle),
                Some(x) => {
                    let ss = gs.start_size;
                    let grown = ss + (1.0 - ss) * gs.speed_curve.at(x);
                    (si.joint + (1.0 - si.joint) * grown, ss + (1.0 - ss) * gs.radius_curve.at(x), gs.start_angle + (1.0 - gs.start_angle) * gs.speed_curve.at(x))
                }
            };
            let st = match si.parent {
                None => State { len, radius, x: x.unwrap_or(0.0), origin: si.base, rot: Quat::IDENTITY },
                Some((p, a, c)) => {
                    let ps = state(p, g, gr, out);
                    let pr = gr.stems[p].factor(&ps, gr.openings[i].map_or(a, |o| o.t)).1;
                    // never further along in thickness than the parent where it comes out, so its
                    // joint stays inside the parent's bark; a root's flare keeps the parent's
                    // proportions (it is part of the trunk's foot)
                    radius = if si.root { pr } else { radius.min(pr) };
                    let rot = ps.rot * Quat::from_axis_angle(si.fold.0, -(1.0 - fold) * si.fold.1);
                    // where it comes out of the parent's bark is the centre of the opening there as
                    // the parent is now, so the opening and the branch close around the same point
                    // (at the parent's growing tip while the parent has not reached it yet)
                    let origin = match gr.openings[i] {
                        Some(o) => {
                            let done: &[Option<State>] = out;
                            let on = gr.close(&|j| done[j].unwrap_or(ps), p, &o, radius, o.at, o.ac, o.anchor);
                            on - rot * (o.anchor - si.base)
                        }
                        None => {
                            let on = gr.stems[p].moved(&ps, a, c, si.base);
                            ps.origin + ps.rot * (on - gr.stems[p].base)
                        }
                    };
                    State { len, radius, x: x.unwrap_or(0.0), origin, rot }
                }
            };
            out[i] = Some(st);
            st
        }
        (0..n).map(|i| state(i, g, self, &mut out)).collect()
    }

    /// Point `q` of the joint where a branch of radius factor `cr` comes out of opening `o` in
    /// stem `p` (axis point `c` at arc fraction `t` of the parent), in the parent's frame: on
    /// the parent's bark as it is now, closed towards the opening's centre while the branch is
    /// thinner (relative to its final size) than the parent there.
    /// Returns where it is now.
    fn close(&self, states: &dyn Fn(usize) -> State, p: usize, o: &Opening, cr: f32, t: f32, c: Vec3, q: Vec3) -> Vec3 {
        let par = &self.stems[p];
        let ps = states(p);
        let pr = par.factor(&ps, o.t).1;
        let k = if pr > 1e-6 { (cr / pr).clamp(0.0, 1.0) } else { 0.0 };
        if k >= 1.0 {
            return self.on_stem(states, p, t, c, q);
        }
        // close along the parent, around it and in depth by k (so it slides over the bark
        // instead of cutting through the wood), then take the parent's growth
        let out = (o.centre - o.c).normalize_or(Vec3::X);
        let d = q.distance(c);
        let dir = if d > 1e-6 { (q - c) / d } else { out };
        let t2 = o.t + (t - o.t) * k;
        let c2 = par.axis.at(t2);
        let dir2 = Quat::IDENTITY.slerp(Quat::from_rotation_arc(out, dir), k) * out;
        let q2 = c2 + dir2 * (o.d + (d - o.d) * k);
        self.on_stem(states, p, t2, c2, q2)
    }

    /// Where point `q` of stem `s`'s final bark (axis point `c` at arc fraction `t`) is now. Its
    /// joint (inside its own parent) moves with the opening it comes out of, so a branch that
    /// comes out of another branch's joint follows that joint.
    fn on_stem(&self, states: &dyn Fn(usize) -> State, s: usize, t: f32, c: Vec3, q: Vec3) -> Vec3 {
        let info = &self.stems[s];
        if let (Some((p, ..)), Some(o)) = (info.parent, self.openings[s]) {
            if t <= info.joint + 1e-6 {
                let (tp, cp) = locate_on(&self.stems[p].axis.points, q);
                return self.close(states, p, &o, states(s).radius, tp, cp, q);
            }
        }
        self.place(s, &states(s), t, c, q)
    }

    /// Point `q` of stem `s` (axis point `c` at arc fraction `t`) at its state.
    fn place(&self, s: usize, st: &State, t: f32, c: Vec3, q: Vec3) -> Vec3 {
        let info = &self.stems[s];
        st.origin + st.rot * (info.moved(st, t, c, q) - info.base)
    }

    /// Vertex positions at growth `g` (1 = the mature mesh).
    pub fn positions(&self, g: f32) -> Vec<Vec3> {
        let g = g.clamp(0.0, 1.0);
        if g >= 1.0 {
            return self.base.clone();
        }
        let states = self.states(g);
        let tree = self.leader.map(|i| states[i].len).unwrap_or(g);
        let mut out = Vec::with_capacity(self.base.len());
        for (v, b) in self.bind.iter().enumerate() {
            let p0 = self.base[v];
            let p = match *b {
                Bind::Fixed => p0 * tree,
                Bind::Stem { s, t, c } => self.place(s as usize, &states[s as usize], t, c, p0),
                Bind::Joint { s, t, c, child } => {
                    let s = s as usize;
                    match self.openings[child as usize] {
                        Some(o) => self.close(&|j| states[j], s, &o, states[child as usize].radius, t, c, p0),
                        None => self.place(s, &states[s], t, c, p0),
                    }
                }
                Bind::Leaf { l } => {
                    let li = &self.leaves[l as usize];
                    if self.stems.is_empty() {
                        p0 * tree
                    } else {
                        let st = &states[li.stem];
                        let anchor = self.place(li.stem, st, li.t, li.axis, li.anchor);
                        let gs = &li.g;
                        match Timing::progress(self.timing.leaf_start[l as usize], self.timing.leaf_time[l as usize], g) {
                            None => anchor,
                            Some(x) => {
                                let size = gs.start_size + (1.0 - gs.start_size) * gs.speed_curve.at(x);
                                let xo = (x / gs.orientation_time).min(1.0);
                                let turned = gs.start_angle + (1.0 - gs.start_angle) * gs.speed_curve.at(xo);
                                let r = Quat::IDENTITY.slerp(li.fold, 1.0 - turned);
                                anchor + st.rot * (r * ((p0 - li.anchor) * size))
                            }
                        }
                    }
                }
            };
            out.push(p);
        }
        out
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
pub fn animate(doc: &Document, timing: &Timing, meshes: &[(&Mesh, &Skeleton)]) -> Animation {
    let keys = keyframes(&doc.growth);
    let shapes = meshes
        .iter()
        .map(|(m, sk)| {
            let gr = Grower::new(doc, sk, m, timing);
            keys.iter().map(|&g| gr.positions(g)).collect()
        })
        .collect();
    Animation { keys, shapes }
}

/// Stage clips: (name, from, to). "Grow" runs from nothing to the mature tree; the first
/// stage's clip grows from nothing to it, the others from the stage before.
pub fn clips(growth: &crate::model::Growth) -> Vec<(String, f32, f32)> {
    let mut order: Vec<usize> = (0..growth.stages.len()).collect();
    order.sort_by(|&a, &b| growth.stages[a].at.total_cmp(&growth.stages[b].at));
    let mut out = vec![("Grow".to_string(), 0.0, 1.0)];
    let mut prev: Option<(String, f32)> = None;
    for i in order {
        let (name, at) = (growth.label(i), growth.stages[i].at.clamp(0.0, 1.0));
        match &prev {
            None if at > 0.0 => out.push((name.clone(), 0.0, at)),
            Some((pn, pa)) if at > *pa => out.push((format!("{pn} to {name}"), *pa, at)),
            _ => {}
        }
        prev = Some((name, at));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grow::{build_mesh, grow};
    use crate::model::preset;

    fn shown_leaves(m: &Mesh, p: &[Vec3]) -> usize {
        (0..m.leaf_first.len())
            .filter(|&i| {
                let a = m.leaf_first[i] as usize;
                p[a].distance(p[a + 2]) > 1e-4
            })
            .count()
    }

    #[test]
    fn grows_from_nothing_to_the_mature_tree() {
        for name in ["Oak", "Pine", "Birch"] {
            let doc = preset(name).unwrap();
            let sk = grow(&doc, 0);
            let m = build_mesh(&doc, &sk, 0);
            let timing = Timing::new(&doc, &sk);
            let last = timing.stem_start.iter().zip(&timing.stem_time).map(|(s, t)| s + t).chain(timing.leaf_start.iter().zip(&timing.leaf_time).map(|(s, t)| s + t)).fold(0.0f32, f32::max);
            assert!((last - 1.0).abs() < 1e-4, "{name}: {last}");
            let gr = Grower::new(&doc, &sk, &m, &timing);
            // the mature tree is the mesh as designed, and very close to it just before
            assert_eq!(gr.positions(1.0), m.pos);
            let late = gr.positions(0.9999);
            assert!(late.iter().zip(&m.pos).all(|(a, b)| a.distance(*b) < 0.02), "{name}");
            // nothing pokes out of the mature tree's bounds
            let (lo, hi) = m.pos.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
            let pad = (hi - lo) * 0.03 + Vec3::splat(0.05);
            for g in [0.1, 0.3, 0.5, 0.7, 0.9] {
                assert!(gr.positions(g).iter().all(|p| p.cmpge(lo - pad).all() && p.cmple(hi + pad).all()), "{name} at {g}");
            }
            // grows taller all the way, and leaves keep coming
            let h = |p: &[Vec3]| m.tris.iter().filter(|t| (p[t[1] as usize] - p[t[0] as usize]).cross(p[t[2] as usize] - p[t[0] as usize]).length() > 1e-7).flat_map(|t| t.iter()).fold(0.0f32, |a, &i| a.max(p[i as usize].y));
            let gs = [0.0, 0.1, 0.3, 0.5, 0.7, 0.999];
            let hs: Vec<f32> = gs.iter().map(|&g| h(&gr.positions(g))).collect();
            assert!(hs.windows(2).all(|w| w[0] <= w[1] + 1e-3), "{name} {hs:?}");
            assert!(hs[0] < 0.01 * m.height, "{name} {hs:?}");
            let ls: Vec<usize> = gs.iter().map(|&g| shown_leaves(&m, &gr.positions(g))).collect();
            assert!(ls.windows(2).all(|w| w[0] <= w[1]), "{name} {ls:?}");
            assert!(ls[0] == 0 && ls[3] > 0 && ls[3] < m.leaf_first.len(), "{name} {ls:?}");
        }
    }

    #[test]
    fn nothing_starts_before_its_parent_reaches_it() {
        let mut doc = preset("Oak").unwrap();
        for g in &mut doc.generators {
            g.params.growth.offset_variation = 0.0;
        }
        let sk = grow(&doc, 0);
        let t = Timing::new(&doc, &sk);
        for (i, st) in sk.stems.iter().enumerate() {
            if let Some(p) = st.parent {
                assert!(t.stem_start[i] >= t.stem_start[p] - 1e-5);
                assert!(t.stem_start[i] <= t.stem_start[p] + t.stem_time[p] + 1e-5);
            }
        }
        // later Start % on the twigs: they start later
        let twigs = doc.generators.iter().find(|g| g.name == "Twigs").unwrap().id;
        let mean = |t: &Timing| {
            let v: Vec<f32> = (0..sk.stems.len()).filter(|&i| sk.stems[i].gen == twigs).map(|i| t.stem_start[i]).collect();
            v.iter().sum::<f32>() / v.len() as f32
        };
        let before = mean(&t);
        let tw = &mut doc.generators.iter_mut().find(|g| g.id == twigs).unwrap().params.growth;
        tw.sync = GrowthSync::Parent;
        tw.start_pct = 0.8;
        let later = Timing::new(&doc, &sk);
        assert!(mean(&later) > before, "{} vs {before}", mean(&later));
        for (i, st) in sk.stems.iter().enumerate() {
            if let (Some(p), true) = (st.parent, st.gen == twigs) {
                assert!(later.stem_start[i] >= later.stem_start[p] + 0.8 * later.stem_time[p] - 1e-4);
            }
        }
    }

    /// Bark seen from behind is not drawn, so a triangle that turns inside out while the tree
    /// grows shows as a hole. Joints must stay closed: at most small slivers.
    #[test]
    fn joints_stay_closed_while_growing() {
        for name in ["Oak", "Pine", "Birch"] {
            let doc = preset(name).unwrap();
            let sk = grow(&doc, 0);
            let m = build_mesh(&doc, &sk, 0);
            let gr = Grower::new(&doc, &sk, &m, &Timing::new(&doc, &sk));
            let (mut count, mut worst) = (0usize, 0.0f32);
            for gi in 1..20 {
                let p = gr.positions(gi as f32 * 0.05);
                for t in &m.tris[..m.leaf_tri_start] {
                    let [a, b, c] = t.map(|i| i as usize);
                    let n = (p[b] - p[a]).cross(p[c] - p[a]);
                    let n1 = (m.pos[b] - m.pos[a]).cross(m.pos[c] - m.pos[a]);
                    if n.length() * 0.5 > 1e-6 && n.dot(n1) < 0.0 {
                        count += 1;
                        worst = worst.max(n.length() * 0.5);
                    }
                }
            }
            assert!(worst < 0.006 && count < 250, "{name}: {count} triangles inside out, the largest {worst} m²");
        }
    }

    #[test]
    fn keyframes_hold_the_stages() {
        let g = crate::model::Growth::default();
        let k = keyframes(&g);
        assert_eq!(*k.first().unwrap(), 0.0);
        assert_eq!(*k.last().unwrap(), 1.0);
        for st in &g.stages {
            assert!(k.iter().any(|x| (x - st.at).abs() < 1e-4));
        }
        let c = clips(&g);
        assert_eq!(c[0].0, "Grow");
        assert_eq!(c[1].0, "Sprout");
        assert_eq!(c[2].0, "Sprout to Sapling");
    }
}
