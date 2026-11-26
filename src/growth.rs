//! Growth animation: the tree from nothing to the mature tree, on one mesh whose vertices move.
//!
//! The topology never changes, so an engine can play the growth as morph targets (blend
//! shapes) between a handful of sampled shapes. Growth `g` runs from 0 (nothing above the
//! ground) to 1 (the mature tree as designed). It follows how trees develop:
//!
//! * a tree grows as a whole: every season each bud on the tree makes a new shoot, so all
//!   orders (trunk, branches, twigs) grow at the same time, and a young tree looks like a
//!   small tree, not like a bare pole that sprouts branches later (Palubicki et al., "Self-
//!   organizing tree models", SIGGRAPH 2009). Its lower branches are the ones a real tree later
//!   sheds as its bole lengthens; with a fixed set of branches, the crown keeps its share of the
//!   tree's height instead: every stem has its final shape at a size that grows, and what
//!   grows on it stays at the same place along it;
//! * a stem's size relative to its parent follows a growth function from its bud break
//!   (Prusinkiewicz et al., "Animation of plant development", SIGGRAPH 1993); buds low on a
//!   parent break first, twigs a little after their branch;
//! * a young stem has its mature proportions: with the default curves its radius grows as its
//!   length does, so a 0.8 m birch sapling of a 17 m birch with a 34 cm foot is 16 mm thick at
//!   the foot, as real saplings are. The radius curve is the radius's own growth (against the
//!   speed curve, the length's); a branch is never thicker than its parent where it comes out.
//!   Until 2026-11-26 the radius also grew with size^1.3 and the radius curve came on top of the
//!   size, so a 1.5 m birch sapling was 3 mm thick (474 times as tall as thick, against the
//!   mature tree's 50): a hair. Elastic similarity is a law of grown trees, not of saplings;
//! * leaves come out with the young shoots and keep coming over the whole growth, so even a
//!   seedling has leaves and the crown gets denser as it grows;
//! * other syncs start a part Start % of the way through the parent's growth, together with
//!   its siblings, or when the parent has grown to its spot; an offset with a random variation
//!   spreads neighbours out;
//! * branches can fold out of their parent and leaves turn into place (start angle);
//! * where a branch comes out, the opening in the parent's bark is only as wide as the branch
//!   is thick, so joints stay closed.
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
/// With Auto, a branch's bud breaks this long (share of the trunk's growth time) after its
/// parent's, less for buds low on the parent; plus up to STAGGER at random.
const BUD_DELAY: f32 = 0.06;
const STAGGER: f32 = 0.1;
/// With Auto, a branch reaches its full size relative to its parent in this share of the
/// trunk's growth time at speed 1 (from then on it grows with the parent).
const REL_TIME: f32 = 0.25;
/// A young stem is at most this many times as thick, against its length, as when it is grown
/// (a radius curve ahead of the speed curve: ease-out radius on linear length is 2x at the start).
const STOUT_MAX: f32 = 2.0;
/// With Auto, new leaves keep coming out until this point of the growth.
const LEAF_END: f32 = 0.85;
/// A branch starts once its parent has grown this many times its base radius past where it
/// comes out, so its joint collar is never above the parent's growing tip.
const JOINT_REACH: f32 = 2.5;
/// A growing tip is a cone this many times as long as the stem is thick there.
const TIP_SLOPE: f32 = 2.5;

/// A stem's radius factor against its present size: its radius growth `rc` over its length
/// growth `f` (1 when they agree: the mature proportions).
fn stout(rc: f32, f: f32) -> f32 {
    if f > 1e-6 { (rc / f).min(STOUT_MAX) } else { 0.0 }
}

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
        // where each stem comes out along its parent (0 = base, 1 = tip)
        let apos: Vec<f32> = sk.stems.iter().map(|st| st.parent.map_or(0.0, |p| locate(&sk.stems[p], st.points[0]).0)).collect();
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
                    if let (true, Some(p)) = (sk.stems[i].fork, sk.stems[i].parent) {
                        // a split's leader is part of the stem it splits from: it grows with it
                        // (always its full share of it, see `states`)
                        s = start[p];
                        d = time[p];
                    } else if let Some(p) = sk.stems[i].parent {
                        match sync {
                            // the whole tree grows at once: a bud breaks a little after its
                            // parent's (low buds first, roots with the trunk) and the shoot
                            // reaches its share of the parent's size in REL_TIME / speed
                            GrowthSync::Auto => {
                                s = if sk.stems[i].root { start[p] } else { start[p] + BUD_DELAY * (0.35 + 0.65 * apos[i]) + STAGGER * hash01(3 * i as u32 + 7) };
                                d = REL_TIME / g.speed;
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
                    // come out with the young shoots and keep coming over the whole growth, so
                    // the crown gets denser: a few with the young trunk, some with the branch
                    // below their twig, most once their twig has broken bud; until their twig
                    // is out they sit where it will come out
                    GrowthSync::Auto => {
                        let h = hash01(7919 * i as u32 + 3);
                        let mut chain = vec![p];
                        while let Some(q) = sk.stems[*chain.last().unwrap()].parent {
                            chain.push(q);
                        }
                        let up = if h < SEEDLING_LEAVES / nl.max(1) as f32 { chain.len() - 1 } else if h < EARLY_LEAVES { 1.min(chain.len() - 1) } else { 0 };
                        let s0 = start[chain[up]] + 0.02;
                        let h2 = hash01(7919 * i as u32 + 11);
                        s = if up + 1 == chain.len() && chain.len() > 1 { s0 + 0.04 * h2 } else { s0 + h2 * h2 * (LEAF_END - s0).max(0.0) };
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
const SEEDLING_LEAVES: f32 = 14.0;

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
    /// `ct`, `cc`: for the branch's own rings, their arc fraction and axis point on the branch
    /// (its growing tip shapes them too); -1 for the rim.
    Joint { s: u32, t: f32, c: Vec3, child: u32, ct: f32, cc: Vec3 },
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
    /// A leader of a split: part of its parent, always its full share of it.
    fork: bool,
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
        // only once it has (nearly) all its length; blending earlier leaves a blunt, sawn-off end
        let left = ((1.0 - st.len) * self.length / (TIP_SLOPE * self.final_radius(1.0).max(radius_m))).clamp(0.0, 1.0);
        let done = (1.0 - left).powi(2);
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
    /// Grown length (arc fraction) and radius factor (of the stem at its present size).
    len: f32,
    radius: f32,
    /// Where its base is, how it is turned and how large it is (1 = its final size).
    origin: Vec3,
    rot: Quat,
    scale: f32,
    /// How thick it is relative to its final thickness, compared with its parent's frame
    /// (opens the parent's bark for it).
    open: f32,
}

impl State {
    /// Where the part of the final stem at arc fraction `t` is now, as an arc fraction along
    /// the stem (before scaling): a grown stem is whole, one that has not started is closed.
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
    /// How far the rim may close at most (0 = to a point): where openings sit side by side,
    /// closing both further would pull the bark between them inside out; the branch's own base
    /// covers the rest.
    k_min: f32,
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
        let on_parent = |child: usize, q: Vec3, own: Option<(f32, Vec3)>| -> Option<Bind> {
            let p = sk.stems[child].parent?;
            let (t, c) = locate(&sk.stems[p], q);
            let (ct, cc) = own.unwrap_or((-1.0, Vec3::ZERO));
            Some(Bind::Joint { s: p as u32, t, c, child: child as u32, ct, cc })
        };
        for &(v, child) in &m.joint_rim {
            let (v, child) = (v as usize, child as usize);
            if child >= sk.stems.len() || v >= m.pos.len() || claimed[v] {
                continue;
            }
            if let (Bind::Stem { s, .. }, Some(p)) = (bind[v], sk.stems[child].parent) {
                if s as usize == p {
                    if let Some(b) = on_parent(child, m.pos[v], None) {
                        bind[v] = b;
                        claimed[v] = true;
                    }
                }
            }
        }
        for v in 0..m.pos.len() {
            if let Bind::Stem { s, t, c } = bind[v] {
                let s = s as usize;
                if sk.stems[s].parent.is_some() && t < joints[s] + 1e-6 {
                    if let Some(b) = on_parent(s, m.pos[v], Some((t, c))) {
                        bind[v] = b;
                    }
                }
            }
        }
        // copies of one point (texture seams, the stitch strip's own copy of the rim) must move
        // as one, or the bark cracks open between them: a joint binding wins (the lowest
        // branch's, so it is the same for every copy)
        {
            let mut groups: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
            for (v, b) in bind.iter().enumerate() {
                if matches!(b, Bind::Stem { .. } | Bind::Joint { .. }) {
                    let q = m.pos[v] * 2e4;
                    groups.entry((q.x.round() as i64, q.y.round() as i64, q.z.round() as i64)).or_default().push(v);
                }
            }
            for vs in groups.values().filter(|vs| vs.len() > 1) {
                let rank = |b: &Bind| match *b {
                    Bind::Joint { child, .. } => (0, child),
                    Bind::Stem { s, .. } => (1, s),
                    _ => (2, 0),
                };
                let best = vs.iter().map(|&v| bind[v]).min_by_key(rank).unwrap();
                for &v in vs {
                    bind[v] = best;
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
                Some(Opening { centre, t, c, d, anchor, at, ac, k_min: 0.0 })
            })
            .collect();
        // how far each rim may close: try closing every opening alike and see which bark
        // triangles around rims turn over
        let mut openings = openings;
        {
            let mut k_min = vec![0.0f32; sk.stems.len()];
            for tri in &m.tris[..m.leaf_tri_start.min(m.tris.len())] {
                let mut parent = None;
                let mut kids: Vec<usize> = vec![];
                let mut ok = true;
                for &v in tri {
                    let (s, child) = match bind[v as usize] {
                        Bind::Stem { s, .. } => (s, None),
                        Bind::Joint { s, child, ct, .. } if ct < 0.0 => (s, Some(child as usize)),
                        _ => {
                            ok = false;
                            break;
                        }
                    };
                    if *parent.get_or_insert(s) != s {
                        ok = false;
                    }
                    if let Some(ch) = child {
                        kids.push(ch);
                    }
                }
                if !ok || kids.is_empty() {
                    continue;
                }
                let p = parent.unwrap() as usize;
                let at = |v: u32, k: f32| -> Vec3 {
                    match bind[v as usize] {
                        Bind::Joint { t, c, child, .. } => match &openings[child as usize] {
                            Some(o) => Grower::bark_shrink(&axes[p], o, k, t, c, m.pos[v as usize]).2,
                            None => m.pos[v as usize],
                        },
                        _ => m.pos[v as usize],
                    }
                };
                let [a, b, c] = *tri;
                let n1 = (m.pos[b as usize] - m.pos[a as usize]).cross(m.pos[c as usize] - m.pos[a as usize]);
                for step in 0..10 {
                    let k = step as f32 * 0.1;
                    let (pa, pb, pc) = (at(a, k), at(b, k), at(c, k));
                    let n = (pb - pa).cross(pc - pa);
                    if n.length() > 2e-3 && n.dot(n1) < 0.0 {
                        for &ch in &kids {
                            k_min[ch] = k_min[ch].max(k + 0.1);
                        }
                    }
                }
            }
            for (i, o) in openings.iter_mut().enumerate() {
                if let Some(o) = o {
                    o.k_min = k_min[i].min(1.0);
                }
            }
        }
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
                StemInfo { axis: Axis::new(st), base, parent, fold, radii: st.radii.clone(), length: polyline_length(st), joint: joints[i], root: st.root, fork: st.fork, g: settings(doc, st.gen).0 }
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
            // its size relative to its parent, its radius curve and how far it has folded out
            let (f, rc, fold) = match x {
                // not out yet: gathered where it will come out
                None => (0.0, 0.0, gs.start_angle),
                Some(_) if si.fork => (1.0, 1.0, 1.0),
                Some(x) => {
                    let ss = gs.start_size;
                    (ss + (1.0 - ss) * gs.speed_curve.at(x), ss + (1.0 - ss) * gs.radius_curve.at(x), gs.start_angle + (1.0 - gs.start_angle) * gs.speed_curve.at(x))
                }
            };
            let len = if f > 0.0 { 1.0 } else if si.parent.is_some() { si.joint } else { 0.0 };
            let st = match si.parent {
                // the radius curve is the radius's own growth; the state's radius is against the
                // stem's present size (the length's growth), so equal curves keep the proportions
                None => State { len, radius: stout(rc, f), origin: si.base, rot: Quat::IDENTITY, scale: f, open: 1.0 },
                Some((p, a, c)) => {
                    let ps = state(p, g, gr, out);
                    let scale = ps.scale * f;
                    let pr = gr.stems[p].factor(&ps, gr.openings[i].map_or(a, |o| o.t)).1;
                    // never thicker, against its final size, than the parent where it comes out,
                    // so its joint stays inside the parent's bark; a root's flare keeps the
                    // parent's proportions (it is part of the trunk's foot)
                    let radius = if si.root {
                        pr
                    } else if f > 1e-6 {
                        stout(rc, f).min(pr / f)
                    } else {
                        0.0
                    };
                    let open = f * radius;
                    let rot = ps.rot * Quat::from_axis_angle(si.fold.0, -(1.0 - fold) * si.fold.1);
                    // where it comes out of the parent's bark is the centre of the opening there as
                    // the parent is now, so the opening and the branch close around the same point
                    let origin = match gr.openings[i] {
                        Some(o) => {
                            let done: &[Option<State>] = out;
                            let on = gr.close(&|j| done[j].unwrap_or(ps), p, &o, open, true, o.at, o.ac, o.anchor);
                            on - rot * ((o.anchor - si.base) * scale)
                        }
                        None => gr.place(p, &ps, a, c, si.base),
                    };
                    State { len, radius, origin, rot, scale, open }
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
    #[allow(clippy::too_many_arguments)]
    fn close(&self, states: &dyn Fn(usize) -> State, p: usize, o: &Opening, cr: f32, own: bool, t: f32, c: Vec3, q: Vec3) -> Vec3 {
        let par = &self.stems[p];
        let ps = states(p);
        let pr = par.factor(&ps, o.t).1;
        let k = if pr > 1e-6 { (cr / pr).clamp(0.0, 1.0) } else { 0.0 };
        let k = if own { k } else { k.max(o.k_min) };
        if k >= 1.0 {
            return self.on_stem(states, p, t, c, q);
        }
        // the branch's own joint shrinks evenly towards the opening (a smaller copy of its
        // base) ...
        if own {
            let q2 = o.centre + (q - o.centre) * k;
            let (t2, c2) = locate_on(&par.axis.points, q2);
            return self.on_stem(states, p, t2, c2, q2);
        }
        // ... while the parent's bark closes over it; then the parent's growth
        let (t2, c2, q2) = Grower::bark_shrink(&par.axis, o, k, t, c, q);
        self.on_stem(states, p, t2, c2, q2)
    }

    /// Point `q` of the parent's final bark (axis point `c` at arc fraction `t`) with opening
    /// `o` closed to `k`: along the parent, around it and in depth by k, so it slides over the
    /// bark instead of cutting through the wood. Returns (arc fraction, axis point, point).
    fn bark_shrink(axis: &Axis, o: &Opening, k: f32, t: f32, c: Vec3, q: Vec3) -> (f32, Vec3, Vec3) {
        let out = (o.centre - o.c).normalize_or(Vec3::X);
        let d = q.distance(c);
        let dir = if d > 1e-6 { (q - c) / d } else { out };
        let t2 = o.t + (t - o.t) * k;
        let c2 = axis.at(t2);
        let dir2 = Quat::IDENTITY.slerp(Quat::from_rotation_arc(out, dir), k) * out;
        (t2, c2, c2 + dir2 * (o.d + (d - o.d) * k))
    }

    /// Where point `q` of stem `s`'s final bark (axis point `c` at arc fraction `t`) is now. Its
    /// joint (inside its own parent) moves with the opening it comes out of, so a branch that
    /// comes out of another branch's joint follows that joint.
    fn on_stem(&self, states: &dyn Fn(usize) -> State, s: usize, t: f32, c: Vec3, q: Vec3) -> Vec3 {
        let info = &self.stems[s];
        if let (Some((p, ..)), Some(o)) = (info.parent, self.openings[s]) {
            if t <= info.joint + 1e-6 {
                let (tp, cp) = locate_on(&self.stems[p].axis.points, q);
                return self.close(states, p, &o, states(s).open, true, tp, cp, q);
            }
        }
        self.place(s, &states(s), t, c, q)
    }

    /// How much of its own (shrunken) thickness branch `ch`'s joint ring at arc fraction `ct`
    /// keeps: its growing tip is a short cone, measured in the joint's shrunken size up to the
    /// joint's end and in full size beyond.
    fn joint_tip(&self, states: &[State], p: usize, o: &Opening, ch: usize, ct: f32) -> f32 {
        let info = &self.stems[ch];
        let cs = &states[ch];
        let pr = self.stems[p].factor(&states[p], o.t).1;
        let k = if pr > 1e-6 { (cs.open / pr).clamp(0.0, 1.0) } else { 0.0 };
        let behind = (k * (info.joint - ct).max(0.0) + (cs.len - info.joint).max(0.0)) * info.length;
        let rad = info.final_radius(ct) * k;
        if rad <= 1e-6 {
            return 1.0;
        }
        let cone = (behind / (TIP_SLOPE * rad)).clamp(0.0, 1.0);
        cone * (2.0 - cone)
    }

    /// Point `q` of stem `s` (axis point `c` at arc fraction `t`) at its state.
    fn place(&self, s: usize, st: &State, t: f32, c: Vec3, q: Vec3) -> Vec3 {
        let info = &self.stems[s];
        st.origin + st.rot * ((info.moved(st, t, c, q) - info.base) * st.scale)
    }

    /// Vertex positions at growth `g` (1 = the mature mesh).
    pub fn positions(&self, g: f32) -> Vec<Vec3> {
        let g = g.clamp(0.0, 1.0);
        if g >= 1.0 {
            return self.base.clone();
        }
        let states = self.states(g);
        let tree = self.leader.map(|i| states[i].scale).unwrap_or(g);
        let mut out = Vec::with_capacity(self.base.len());
        for (v, b) in self.bind.iter().enumerate() {
            let p0 = self.base[v];
            let p = match *b {
                Bind::Fixed => p0 * tree,
                Bind::Stem { s, t, c } => self.place(s as usize, &states[s as usize], t, c, p0),
                Bind::Joint { s, t, c, child, ct, cc } => {
                    let (s, ch) = (s as usize, child as usize);
                    match self.openings[ch] {
                        Some(o) => {
                            // the branch's own joint rings end in its growing tip too, so a long
                            // welded base does not show as a stump with a flat end
                            let q = if ct >= 0.0 { cc + (p0 - cc) * self.joint_tip(&states, s, &o, ch, ct) } else { p0 };
                            self.close(&|j| states[j], s, &o, states[ch].open, ct >= 0.0, t, c, q)
                        }
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
                                // a young tree's leaves are a little smaller
                                let size = (gs.start_size + (1.0 - gs.start_size) * gs.speed_curve.at(x)) * (0.45 + 0.55 * tree.clamp(0.0, 1.0));
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
    /// grows shows as a hole, and so does a crack where copies of one point move apart.
    /// (`flips`: per growth step, inside-out triangles and the largest one's area in m².)
    fn holes(doc: &Document) -> (Vec<(usize, f32)>, f32) {
        let sk = grow(doc, 0);
        let m = build_mesh(doc, &sk, 0);
        let gr = Grower::new(doc, &sk, &m, &Timing::new(doc, &sk));
        let mut same: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
        for t in &m.tris[..m.leaf_tri_start] {
            for &v in t {
                let q = m.pos[v as usize] * 2e4;
                let e = same.entry((q.x.round() as i64, q.y.round() as i64, q.z.round() as i64)).or_default();
                if !e.contains(&(v as usize)) {
                    e.push(v as usize);
                }
            }
        }
        let (mut flips, mut crack) = (vec![], 0.0f32);
        for gi in 1..20 {
            let p = gr.positions(gi as f32 * 0.05);
            let (mut count, mut worst) = (0usize, 0.0f32);
            for t in &m.tris[..m.leaf_tri_start] {
                let [a, b, c] = t.map(|i| i as usize);
                let n = (p[b] - p[a]).cross(p[c] - p[a]);
                let n1 = (m.pos[b] - m.pos[a]).cross(m.pos[c] - m.pos[a]);
                if n.length() * 0.5 > 1e-6 && n.dot(n1) < 0.0 {
                    count += 1;
                    worst = worst.max(n.length() * 0.5);
                }
            }
            flips.push((count, worst));
            for vs in same.values() {
                for &v in &vs[1..] {
                    crack = crack.max(p[v].distance(p[vs[0]]));
                }
            }
        }
        (flips, crack)
    }

    fn tot_area(doc: &Document) -> f32 {
        let sk = grow(doc, 0);
        let m = build_mesh(doc, &sk, 0);
        let gr = Grower::new(doc, &sk, &m, &Timing::new(doc, &sk));
        let mut tot = 0.0;
        for gi in 1..20 {
            let p = gr.positions(gi as f32 * 0.05);
            for t in &m.tris[..m.leaf_tri_start] {
                let [a, b, c] = t.map(|i| i as usize);
                let n = (p[b] - p[a]).cross(p[c] - p[a]);
                let n1 = (m.pos[b] - m.pos[a]).cross(m.pos[c] - m.pos[a]);
                if n.dot(n1) < 0.0 {
                    tot += n.length() * 0.5;
                }
            }
        }
        tot
    }

    #[test]
    fn bark_stays_closed_while_growing() {
        for name in ["Oak", "Pine", "Birch"] {
            let (flips, crack) = holes(&preset(name).unwrap());
            let count: usize = flips.iter().map(|f| f.0).sum();
            let worst = flips.iter().map(|f| f.1).fold(0.0f32, f32::max);
            assert!(crack < 1e-4, "{name}: copies of a point {crack} m apart");
            assert!(worst < 0.006 && count < 250, "{name}: {count} triangles inside out, the largest {worst} m²");
        }
    }

    /// `ARBOR_DOC=path cargo test holes_in_a_document -- --ignored --nocapture`
    /// Where the largest turned-over bark triangles are, per growth step (for debugging).
    #[test]
    #[ignore]
    fn worst_flips_in_a_document() {
        let Ok(name) = std::env::var("ARBOR_DOC") else { return };
        let doc = preset(&name).unwrap_or_else(|| crate::state::load_project(std::path::Path::new(&name)).unwrap());
        let sk = grow(&doc, 0);
        let m = build_mesh(&doc, &sk, 0);
        let gr = Grower::new(&doc, &sk, &m, &Timing::new(&doc, &sk));
        for gi in 1..20 {
            let p = gr.positions(gi as f32 * 0.05);
            let mut worst: Vec<(f32, usize)> = vec![];
            for (ti, t) in m.tris[..m.leaf_tri_start].iter().enumerate() {
                let [a, b, c] = t.map(|i| i as usize);
                let n = (p[b] - p[a]).cross(p[c] - p[a]);
                let n1 = (m.pos[b] - m.pos[a]).cross(m.pos[c] - m.pos[a]);
                if n.length() * 0.5 > 1e-4 && n.dot(n1) < 0.0 {
                    worst.push((n.length() * 0.5, ti));
                }
            }
            worst.sort_by(|a, b| b.0.total_cmp(&a.0));
            for &(area, ti) in worst.iter().take(3) {
                let [a, b, c] = m.tris[ti].map(|i| i as usize);
                let stems: Vec<(u32, bool, Option<usize>)> = [a, b, c].iter().map(|&v| m.stem[v]).map(|s| (s, sk.stems.get(s as usize).map(|x| x.fork).unwrap_or(false), sk.stems.get(s as usize).and_then(|x| x.parent))).collect();
                println!("g {:.2} area {:.4} at {:?} stems {:?}", gi as f32 * 0.05, area, m.pos[a], stems);
            }
        }
    }

    #[test]
    #[ignore]
    fn holes_in_a_document() {
        let names: Vec<String> = match std::env::var("ARBOR_DOC") {
            Ok(f) => vec![f],
            Err(_) => vec!["Oak".into(), "Pine".into(), "Birch".into()],
        };
        for name in names {
            let doc = preset(&name).unwrap_or_else(|| crate::state::load_project(std::path::Path::new(&name)).unwrap());
            let (flips, crack) = holes(&doc);
            let flips: Vec<(usize, f32)> = flips.iter().map(|f| (f.0, (f.1 * 1e4).round() / 1e4)).collect();
            println!("{name}: crack {crack:.5} flips {flips:?}");
            println!("   total flipped area over all steps {:.4} m²", tot_area(&doc));
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
