//! The tree document: a hierarchy of generators plus global tree settings.
//! Everything here is plain data (serde) so it can be saved, undone and edited over MCP.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GenKind {
    Trunk,
    Branches,
    Leaves,
    /// Roots spreading from the trunk base along and into the ground.
    Roots,
}

impl GenKind {
    pub fn label(self) -> &'static str {
        match self {
            GenKind::Trunk => "Trunk",
            GenKind::Branches => "Branches",
            GenKind::Leaves => "Leaves",
            GenKind::Roots => "Roots",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "trunk" => Some(GenKind::Trunk),
            "branches" | "branch" | "twigs" => Some(GenKind::Branches),
            "leaves" | "leaf" | "fronds" | "needles" => Some(GenKind::Leaves),
            "roots" | "root" => Some(GenKind::Roots),
            _ => None,
        }
    }
}

/// How child length changes along the parent (the crown silhouette).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    Conical,
    Spherical,
    Hemispherical,
    Cylindrical,
    Flame,
    InverseConical,
}

impl Profile {
    pub const ALL: [Profile; 6] = [
        Profile::Conical,
        Profile::Spherical,
        Profile::Hemispherical,
        Profile::Cylindrical,
        Profile::Flame,
        Profile::InverseConical,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Profile::Conical => "Conical",
            Profile::Spherical => "Spherical",
            Profile::Hemispherical => "Hemispherical",
            Profile::Cylindrical => "Cylindrical",
            Profile::Flame => "Flame",
            Profile::InverseConical => "Inverse conical",
        }
    }
    /// Length factor for a child placed at `t` (0 = start of range, 1 = end).
    pub fn eval(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Profile::Conical => 1.0 - 0.85 * t,
            Profile::Spherical => 0.25 + 0.75 * (std::f32::consts::PI * (0.15 + 0.7 * t)).sin(),
            Profile::Hemispherical => 0.3 + 0.7 * (std::f32::consts::FRAC_PI_2 * (1.0 - t * t)).sin().max(0.0),
            Profile::Cylindrical => 1.0,
            Profile::Flame => {
                if t < 0.7 {
                    0.25 + 0.75 * t / 0.7
                } else {
                    0.25 + 0.75 * (1.0 - t) / 0.3
                }
            }
            Profile::InverseConical => 0.3 + 0.7 * t,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum LeafShape {
    Broad,
    Narrow,
    Needle,
    Round,
    Frond,
    /// A plain rectangle with full 0..1 UVs: the texture draws the leaf (cut-out).
    Card,
    /// Your own model from an OBJ file (leaf clusters, flowers, fruit...), see `leaf_mesh`.
    Mesh,
}

impl LeafShape {
    pub const ALL: [LeafShape; 7] = [
        LeafShape::Broad,
        LeafShape::Narrow,
        LeafShape::Needle,
        LeafShape::Round,
        LeafShape::Frond,
        LeafShape::Card,
        LeafShape::Mesh,
    ];
    pub fn label(self) -> &'static str {
        match self {
            LeafShape::Broad => "Broad",
            LeafShape::Narrow => "Narrow",
            LeafShape::Needle => "Needle",
            LeafShape::Round => "Round",
            LeafShape::Frond => "Frond (palm)",
            LeafShape::Card => "Card (for leaf textures)",
            LeafShape::Mesh => "Custom mesh (OBJ)",
        }
    }
}

/// All generator parameters. Which ones matter depends on the generator kind;
/// see [`param_reference`] for ranges and meaning.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct GenParams {
    // Generation
    pub count: u32,
    pub start: f32,
    pub end: f32,
    pub phyllotaxis: f32,
    pub whorl: u32,
    pub random: f32,
    // Spine
    pub length: f32,
    pub length_variation: f32,
    pub profile: Profile,
    pub angle: f32,
    pub angle_variation: f32,
    pub gravity: f32,
    pub wiggle: f32,
    pub segments: u32,
    // Skin
    pub radius: f32,
    pub taper: f32,
    pub flare: f32,
    pub sides: u32,
    // Leaves
    pub leaf_size: f32,
    pub leaf_size_variation: f32,
    pub leaf_shape: LeafShape,
    pub leaf_aspect: f32,
    /// OBJ file used when leaf_shape is `mesh` (+Y = base to tip, +Z = face normal).
    pub leaf_mesh: Option<String>,
    // Shaping
    /// How strongly the tree's forces bend this generator (0 = ignore forces).
    pub force_response: f32,
    // Ground (roots)
    /// How deep roots sink into the ground: 0 = lying on top, 1 = just under the surface.
    pub buried: f32,
    // Extra spine control
    /// Spacing between children in metres along the parent; 0 = use `count` instead.
    pub interval: f32,
    /// One sudden bend (degrees) somewhere in the middle of each stem.
    pub kink: f32,
    /// Small sharp zig-zags along the stem (unlike wiggle, they don't accumulate).
    pub jitter: f32,
    // Extra skin control
    /// Number of lengthwise grooves in the bark cross-section (0 = round).
    pub flutes: u32,
    pub flute_depth: f32,
    /// Squash the cross-section into an ellipse (0 = round).
    pub flatten: f32,
    /// Lumpy bark: displacement as a fraction of the radius.
    pub bumps: f32,
    /// Size of the lumps in metres.
    pub bump_size: f32,
    // Extra leaf control
    /// Fold the leaf halves up along the midrib (degrees).
    pub leaf_fold: f32,
    /// Curl the leaf tip down (positive) or up (negative).
    pub leaf_bend: f32,
    /// Turn leaves to face the sky (0 = random orientation, 1 = flat towards the sky).
    pub leaf_face_up: f32,
    /// Blend a branch's base smoothly into its parent (0 = plain joint, 1 = strong collar).
    pub weld: f32,
    /// Chance (0..1) that a branch is broken off: a short blunt stub with nothing growing on it.
    pub break_chance: f32,
    /// Length of broken stubs relative to the full branch.
    pub break_length: f32,
    /// Split each stem into this many leaders (0 = no split): decurrent crowns such as oaks and
    /// maples, whose trunk divides into several big limbs instead of running to the top.
    pub splits: u32,
    /// Where along the stem it splits (fraction of its length).
    pub split_at: f32,
    /// How far each leader leans away from the stem's direction (degrees).
    pub split_angle: f32,
    /// Curves that shape a value along the parent or along the stem itself; see CURVE_REFERENCE.
    pub curves: BTreeMap<String, Curve>,
    /// How this generator's parts grow in the growth animation.
    pub growth: GenGrowth,
}

/// The shape of a growth curve over one stem's or leaf's own growth (0 = starts, 1 = done).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum GrowthCurve {
    /// Steady.
    #[default]
    Linear,
    /// Slow at first, then faster.
    EaseIn,
    /// Fast at first, then slower.
    EaseOut,
    /// Slow, fast, slow.
    SCurve,
}

impl GrowthCurve {
    pub const ALL: [GrowthCurve; 4] = [GrowthCurve::Linear, GrowthCurve::EaseIn, GrowthCurve::EaseOut, GrowthCurve::SCurve];

    pub fn label(self) -> &'static str {
        match self {
            GrowthCurve::Linear => "Linear",
            GrowthCurve::EaseIn => "Ease in (slow start)",
            GrowthCurve::EaseOut => "Ease out (slow end)",
            GrowthCurve::SCurve => "S-curve",
        }
    }

    /// The curve's value at `x` (0..1 -> 0..1).
    pub fn at(self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match self {
            GrowthCurve::Linear => x,
            GrowthCurve::EaseIn => x * x,
            GrowthCurve::EaseOut => 1.0 - (1.0 - x) * (1.0 - x),
            GrowthCurve::SCurve => x * x * (3.0 - 2.0 * x),
        }
    }

    /// Where the curve reaches `y` (the inverse of `at`).
    pub fn inverse(self, y: f32) -> f32 {
        let y = y.clamp(0.0, 1.0);
        match self {
            GrowthCurve::Linear => y,
            GrowthCurve::EaseIn => y.sqrt(),
            GrowthCurve::EaseOut => 1.0 - (1.0 - y).sqrt(),
            GrowthCurve::SCurve => {
                let (mut lo, mut hi) = (0.0f32, 1.0f32);
                for _ in 0..24 {
                    let m = 0.5 * (lo + hi);
                    if self.at(m) < y {
                        lo = m;
                    } else {
                        hi = m;
                    }
                }
                0.5 * (lo + hi)
            }
        }
    }
}

/// When a stem or leaf starts to grow.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum GrowthSync {
    /// The whole tree grows at once, as real trees do: a branch's bud breaks a little after its
    /// parent's (low buds first) and the branch grows to its share of the parent's size while
    /// the parent keeps growing, so a young tree is a small tree in its mature proportions. Leaves come out with
    /// the young shoots and keep coming over the whole growth, so the crown gets denser.
    #[default]
    Auto,
    /// Start % of the way through the parent's growth (never before the parent has grown to
    /// its spot).
    Parent,
    /// All of this generator's stems or leaves start together: Start % between as early and as
    /// late as all of them can.
    Siblings,
    /// Start and finish together with the parent (good for roots).
    ParentStartEnd,
    /// Exactly when the parent has grown to the point it grows from.
    InPlace,
}

impl GrowthSync {
    pub const ALL: [GrowthSync; 5] = [GrowthSync::Auto, GrowthSync::Parent, GrowthSync::Siblings, GrowthSync::ParentStartEnd, GrowthSync::InPlace];

    pub fn label(self) -> &'static str {
        match self {
            GrowthSync::Auto => "Auto",
            GrowthSync::Parent => "Parent",
            GrowthSync::Siblings => "Siblings",
            GrowthSync::ParentStartEnd => "Parent's start and end",
            GrowthSync::InPlace => "In place",
        }
    }
}

/// How one generator's stems or leaves grow in the growth animation. Each stem keeps its final
/// shape and grows in size relative to its parent (what grows on it stays at the same place
/// along it), while it thickens; it starts when its parent allows (see `sync`).
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct GenGrowth {
    /// Speed scale: stems grow `speed` times as fast as the trunk's pace (a stem as long as
    /// the first trunk takes 1 / speed of the trunk's time); leaves unfold in 0.04 / speed of it.
    pub speed: f32,
    /// How the length (leaves: the size) grows over the part's own growth.
    pub speed_curve: GrowthCurve,
    /// Stems: how the radius grows over the stem's own growth.
    pub radius_curve: GrowthCurve,
    /// Share of its final size a part has as soon as it appears (0 = grows from nothing).
    pub start_size: f32,
    /// Stems: share of the final angle to the parent when the stem starts (below 1 it folds
    /// out of the parent). Leaves: share of the final orientation (below 1 they turn into place
    /// from lying along the twig).
    pub start_angle: f32,
    /// Leaves: how long turning into place takes, relative to unfolding (1 = together).
    pub orientation_time: f32,
    pub sync: GrowthSync,
    /// With Parent or Siblings: 0 = as early as possible, 1 = as late.
    pub start_pct: f32,
    /// Delay added to every start, in shares of the trunk's growth time.
    pub offset: f32,
    /// Random extra delay of up to this much per stem or leaf, so neighbours do not start
    /// together.
    pub offset_variation: f32,
    /// Shed while young. 0 = kept for life; otherwise the growth point by which this
    /// generator's stems (with everything on them) or leaves have withered away and gone, as a
    /// pine sheds its lower whorls while its bole lengthens, or a birch sapling's single leaves
    /// give way to the grown crown's sprays. A shed generator is in the growth only: the grown
    /// tree, its exports without growth, its billboard and its collision never have it.
    pub shed: f32,
    /// Leaves: their size on a seedling, as a share of their grown size; they reach it as the
    /// tree grows. A single leaf is nearly its full size on a sapling (0.85); a card that is a
    /// spray of many leaves is better kept off the young tree (a later `offset`) than shrunk.
    pub young_size: f32,
}

impl Default for GenGrowth {
    fn default() -> Self {
        GenGrowth {
            speed: 1.0,
            speed_curve: GrowthCurve::Linear,
            radius_curve: GrowthCurve::Linear,
            start_size: 0.0,
            start_angle: 1.0,
            orientation_time: 1.0,
            sync: GrowthSync::Auto,
            start_pct: 0.0,
            offset: 0.0,
            offset_variation: 0.02,
            shed: 0.0,
            young_size: 0.45,
        }
    }
}

impl GenGrowth {
    pub fn clamp(&mut self) {
        let f = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.speed = f(self.speed, 0.05, 10.0, 1.0);
        self.start_size = f(self.start_size, 0.0, 1.0, 0.0);
        self.start_angle = f(self.start_angle, 0.0, 1.0, 1.0);
        self.orientation_time = f(self.orientation_time, 0.1, 5.0, 1.0);
        self.start_pct = f(self.start_pct, 0.0, 1.0, 0.0);
        self.offset = f(self.offset, 0.0, 3.0, 0.0);
        self.offset_variation = f(self.offset_variation, 0.0, 3.0, 0.0);
        self.shed = f(self.shed, 0.0, 1.0, 0.0);
        self.young_size = f(self.young_size, 0.05, 1.0, 0.45);
    }

    /// This generator's stems or leaves are shed while the tree is young.
    pub fn sheds(&self) -> bool {
        self.shed > 0.0
    }
}

/// A shaping curve: control points (x 0..1 along the parent, the stem or the tree's height,
/// y 0..1 multiplier), smoothly interpolated. A value with a curve becomes `value × curve(x)`.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Curve {
    pub points: Vec<[f32; 2]>,
    /// What x measures: the curve's natural axis (along the stem or along the parent, see
    /// CURVE_REFERENCE) or the height above the ground as a fraction of the whole tree.
    #[serde(default, skip_serializing_if = "CurveAlong::is_natural")]
    pub along: CurveAlong,
}

/// What a curve's x axis measures.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum CurveAlong {
    /// Along the stem itself (base to tip) or along the parent, depending on the curve.
    #[default]
    Natural,
    /// Height above the ground, 0 = ground, 1 = top of the tree, the same for every level.
    TreeHeight,
}

impl CurveAlong {
    pub fn is_natural(&self) -> bool {
        *self == CurveAlong::Natural
    }
}

impl Default for Curve {
    fn default() -> Self {
        Curve::flat()
    }
}

impl Curve {
    pub fn flat() -> Self {
        Curve { points: vec![[0.0, 1.0], [1.0, 1.0]], along: CurveAlong::Natural }
    }
    pub fn preset(name: &str) -> Option<Self> {
        let pts: Vec<[f32; 2]> = match name {
            "flat" => vec![[0.0, 1.0], [1.0, 1.0]],
            "ramp_up" | "up" => vec![[0.0, 0.1], [1.0, 1.0]],
            "ramp_down" | "down" => vec![[0.0, 1.0], [1.0, 0.1]],
            "bell" => vec![[0.0, 0.2], [0.5, 1.0], [1.0, 0.2]],
            "valley" => vec![[0.0, 1.0], [0.5, 0.25], [1.0, 1.0]],
            "ease_out" => vec![[0.0, 1.0], [0.6, 0.85], [1.0, 0.1]],
            _ => return None,
        };
        Some(Curve { points: pts, along: CurveAlong::Natural })
    }
    pub const PRESETS: [&'static str; 6] = ["flat", "ramp_up", "ramp_down", "bell", "valley", "ease_out"];
    /// Sample a profile function into a curve.
    pub fn from_fn(f: impl Fn(f32) -> f32, n: usize) -> Self {
        let mut c = Curve { points: (0..n).map(|i| {
            let x = i as f32 / (n - 1) as f32;
            [x, f(x)]
        }).collect(), along: CurveAlong::Natural };
        c.clamp();
        c
    }
    /// Sort points, pin the ends to x = 0 and x = 1, keep values in 0..1, at most 16 points.
    pub fn clamp(&mut self) {
        self.points.retain(|p| p[0].is_finite() && p[1].is_finite());
        for p in &mut self.points {
            p[0] = p[0].clamp(0.0, 1.0);
            p[1] = p[1].clamp(0.0, 1.0);
        }
        self.points.sort_by(|a, b| a[0].total_cmp(&b[0]));
        self.points.truncate(16);
        if self.points.is_empty() {
            *self = Curve::flat();
            return;
        }
        if self.points.len() == 1 {
            let y = self.points[0][1];
            self.points = vec![[0.0, y], [1.0, y]];
        }
        self.points[0][0] = 0.0;
        let last = self.points.len() - 1;
        self.points[last][0] = 1.0;
    }
    /// Value at a point whose natural position is `x` and whose height (fraction of the
    /// tree's) is `h`.
    pub fn at(&self, x: f32, h: f32) -> f32 {
        self.eval(if self.along == CurveAlong::TreeHeight { h } else { x })
    }
    /// Smooth (Catmull-Rom) interpolation through the points, clamped to 0..1.
    pub fn eval(&self, x: f32) -> f32 {
        let p = &self.points;
        if p.is_empty() {
            return 1.0;
        }
        let x = x.clamp(0.0, 1.0);
        if p.len() == 1 || x <= p[0][0] {
            return p[0][1];
        }
        let last = p.len() - 1;
        if x >= p[last][0] {
            return p[last][1];
        }
        let i = p.windows(2).position(|w| x >= w[0][0] && x <= w[1][0]).unwrap_or(0);
        let (p1, p2) = (p[i], p[i + 1]);
        let p0 = if i > 0 { p[i - 1] } else { [2.0 * p1[0] - p2[0], p1[1]] };
        let p3 = if i + 2 <= last { p[i + 2] } else { [2.0 * p2[0] - p1[0], p2[1]] };
        let span = (p2[0] - p1[0]).max(1e-5);
        let t = (x - p1[0]) / span;
        // tangents scaled to the segment so uneven spacing doesn't overshoot much
        let m1 = (p2[1] - p0[1]) / (p2[0] - p0[0]).max(1e-5) * span;
        let m2 = (p3[1] - p1[1]) / (p3[0] - p1[0]).max(1e-5) * span;
        let (t2, t3) = (t * t, t * t * t);
        let y = (2.0 * t3 - 3.0 * t2 + 1.0) * p1[1] + (t3 - 2.0 * t2 + t) * m1 + (-2.0 * t3 + 3.0 * t2) * p2[1] + (t3 - t2) * m2;
        y.clamp(0.0, 1.0)
    }
}

/// (key, measured along, applies to, meaning)
pub const CURVE_REFERENCE: &[(&str, &str, &str, &str)] = &[
    ("length", "parent", "branches, roots", "Child length along the parent (x = position within start..end). Replaces the crown profile."),
    ("angle", "parent", "branches, leaves, roots", "Angle from the parent, along the parent."),
    ("radius", "parent", "branches, roots", "Child thickness, along the parent."),
    ("density", "parent", "branches, leaves, roots", "Chance that a child grows at each spot along the parent (0 = none there)."),
    ("leaf_size", "parent", "leaves", "Leaf size along the parent."),
    ("radius_profile", "self", "trunk, branches, roots", "Thickness along the stem itself, base to tip. Replaces taper."),
    ("taper", "self", "trunk, branches, roots", "How strongly the stem narrows at each point (low = keeps its thickness there, high = narrows there). E.g. ramp_up: weak taper at the base, strong towards the tip."),
    ("gravity", "self", "trunk, branches, roots", "How strongly gravity bends each part of the stem (roots: how steeply each part heads down)."),
    ("wiggle", "self", "trunk, branches, roots", "How much each part of the stem meanders."),
    ("jitter", "self", "trunk, branches, roots", "Size of the small zig-zags along the stem."),
    ("force_response", "self", "trunk, branches, roots", "How strongly forces (wind direction, attractors) bend each part of the stem."),
    ("flute_depth", "self", "trunk, branches, roots", "Depth of the bark grooves along the stem."),
    ("flatten", "self", "trunk, branches, roots", "How flat the cross-section is along the stem."),
    ("bumps", "self", "trunk, branches, roots", "Bark lumpiness along the stem."),
    ("angle_variation", "parent", "branches, leaves, roots", "Random angle variation, along the parent."),
    ("length_variation", "parent", "branches, roots", "Random length variation, along the parent."),
    ("kink", "parent", "branches, roots", "Kink strength of the children, along the parent."),
    ("break_chance", "parent", "branches", "Chance of broken stubs, along the parent."),
    ("leaf_fold", "parent", "leaves", "Leaf fold, along the parent."),
    ("leaf_bend", "parent", "leaves", "Leaf tip curl, along the parent."),
    ("leaf_face_up", "parent", "leaves", "How much leaves turn to the sky, along the parent."),
];

pub fn curve_applies(key: &str, kind: GenKind) -> bool {
    let k = match kind {
        GenKind::Trunk => "trunk",
        GenKind::Branches => "branches",
        GenKind::Leaves => "leaves",
        GenKind::Roots => "roots",
    };
    CURVE_REFERENCE.iter().any(|c| c.0 == key && c.2.split(", ").any(|a| a == k))
}

impl GenParams {
    /// Curve multiplier for `key` at a point with natural position `x` (along the stem or the
    /// parent) and height `h` (fraction of the tree's height); 1 without a curve.
    pub fn cv(&self, key: &str, x: f32, h: f32) -> f32 {
        self.curves.get(key).map(|c| c.at(x, h)).unwrap_or(1.0)
    }
}

/// A hand edit of one particular branch (node), identified by its stable key.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct NodeEdit {
    pub key: u32,
    /// Remove this branch and everything on it.
    pub hidden: bool,
    /// Length multiplier.
    pub length: f32,
    /// Thickness multiplier.
    pub radius: f32,
    /// Degrees added to the angle away from the parent.
    pub angle: f32,
    /// Degrees added around the parent.
    pub rotate: f32,
}

impl Default for NodeEdit {
    fn default() -> Self {
        NodeEdit { key: 0, hidden: false, length: 1.0, radius: 1.0, angle: 0.0, rotate: 0.0 }
    }
}

impl NodeEdit {
    pub fn clamp(&mut self) {
        self.length = self.length.clamp(0.05, 3.0);
        self.radius = self.radius.clamp(0.1, 3.0);
        self.angle = self.angle.clamp(-120.0, 120.0);
        self.rotate = self.rotate.clamp(-180.0, 180.0);
    }
    pub fn is_noop(&self) -> bool {
        let d = NodeEdit { key: self.key, ..Default::default() };
        *self == d
    }
}

impl Default for GenParams {
    fn default() -> Self {
        GenParams::for_kind(GenKind::Branches)
    }
}

impl GenParams {
    pub fn for_kind(kind: GenKind) -> Self {
        let base = GenParams {
            count: 10,
            start: 0.25,
            end: 1.0,
            phyllotaxis: 137.5,
            whorl: 1,
            random: 0.3,
            length: 0.5,
            length_variation: 0.2,
            profile: Profile::Conical,
            angle: 50.0,
            angle_variation: 10.0,
            gravity: 0.1,
            wiggle: 0.3,
            segments: 6,
            radius: 0.5,
            taper: 0.3,
            flare: 0.0,
            sides: 6,
            leaf_size: 0.3,
            leaf_size_variation: 0.25,
            leaf_shape: LeafShape::Broad,
            leaf_aspect: 0.55,
            leaf_mesh: None,
            force_response: 1.0,
            buried: 0.5,
            interval: 0.0,
            kink: 0.0,
            jitter: 0.0,
            flutes: 0,
            flute_depth: 0.25,
            flatten: 0.0,
            bumps: 0.0,
            bump_size: 0.4,
            leaf_fold: 0.0,
            leaf_bend: 0.3,
            leaf_face_up: 0.0,
            weld: 0.35,
            break_chance: 0.0,
            break_length: 0.2,
            splits: 0,
            split_at: 0.5,
            split_angle: 30.0,
            curves: BTreeMap::new(),
            growth: GenGrowth::default(),
        };
        match kind {
            GenKind::Trunk => GenParams {
                count: 1,
                start: 0.0,
                end: 1.0,
                length: 8.0,
                angle: 0.0,
                angle_variation: 3.0,
                gravity: 0.0,
                wiggle: 0.2,
                segments: 12,
                radius: 0.3,
                taper: 0.25,
                flare: 0.5,
                sides: 12,
                ..base
            },
            GenKind::Branches => base,
            GenKind::Roots => GenParams {
                count: 6,
                start: 0.0,
                end: 0.03,
                random: 0.4,
                length: 0.3,
                length_variation: 0.3,
                profile: Profile::Cylindrical,
                angle: 112.0,
                angle_variation: 10.0,
                gravity: 0.2,
                wiggle: 0.45,
                segments: 10,
                radius: 0.75,
                taper: 0.08,
                sides: 8,
                force_response: 0.0,
                buried: 0.55,
                ..base
            },
            GenKind::Leaves => GenParams {
                count: 12,
                start: 0.3,
                end: 1.0,
                angle: 50.0,
                angle_variation: 20.0,
                ..base
            },
        }
    }

    /// Keep every value inside a sensible range (also protects against wild MCP input).
    pub fn clamp(&mut self, kind: GenKind) {
        let maxc = if kind == GenKind::Trunk { 24 } else { 200 };
        self.count = self.count.clamp(if kind == GenKind::Trunk { 1 } else { 0 }, maxc);
        self.start = self.start.clamp(0.0, 1.0);
        self.end = self.end.clamp(self.start, 1.0);
        self.phyllotaxis = self.phyllotaxis.clamp(0.0, 360.0);
        self.whorl = self.whorl.clamp(1, 12);
        self.random = self.random.clamp(0.0, 1.0);
        self.length = if kind == GenKind::Trunk {
            self.length.clamp(0.1, 100.0)
        } else {
            self.length.clamp(0.01, 3.0)
        };
        self.length_variation = self.length_variation.clamp(0.0, 1.0);
        self.angle = self.angle.clamp(0.0, 180.0);
        self.angle_variation = self.angle_variation.clamp(0.0, 90.0);
        self.gravity = self.gravity.clamp(-1.0, 1.0);
        self.wiggle = self.wiggle.clamp(0.0, 1.0);
        self.segments = self.segments.clamp(1, 64);
        self.radius = if kind == GenKind::Trunk {
            self.radius.clamp(0.005, 5.0)
        } else {
            self.radius.clamp(0.01, 1.0)
        };
        self.taper = self.taper.clamp(0.0, 1.0);
        self.flare = self.flare.clamp(0.0, 3.0);
        self.sides = self.sides.clamp(3, 32);
        self.leaf_size = self.leaf_size.clamp(0.01, 10.0);
        self.leaf_size_variation = self.leaf_size_variation.clamp(0.0, 1.0);
        self.leaf_aspect = self.leaf_aspect.clamp(0.05, 2.0);
        self.force_response = self.force_response.clamp(0.0, 3.0);
        self.buried = self.buried.clamp(0.0, 1.0);
        self.interval = self.interval.clamp(0.0, 20.0);
        self.kink = self.kink.clamp(0.0, 120.0);
        self.jitter = self.jitter.clamp(0.0, 1.0);
        self.flutes = self.flutes.min(24);
        self.flute_depth = self.flute_depth.clamp(0.0, 0.8);
        self.flatten = self.flatten.clamp(0.0, 0.85);
        self.bumps = self.bumps.clamp(0.0, 0.6);
        self.bump_size = self.bump_size.clamp(0.02, 10.0);
        self.leaf_fold = self.leaf_fold.clamp(0.0, 89.0);
        self.leaf_bend = self.leaf_bend.clamp(-1.0, 1.0);
        self.leaf_face_up = self.leaf_face_up.clamp(0.0, 1.0);
        self.weld = self.weld.clamp(0.0, 1.0);
        self.break_chance = self.break_chance.clamp(0.0, 1.0);
        self.break_length = self.break_length.clamp(0.03, 0.9);
        self.splits = if self.splits < 2 || kind == GenKind::Leaves { 0 } else { self.splits.min(5) };
        self.split_at = self.split_at.clamp(0.05, 0.95);
        self.split_angle = self.split_angle.clamp(0.0, 80.0);
        self.growth.clamp();
        self.curves.retain(|k, _| curve_applies(k, kind));
        for c in self.curves.values_mut() {
            c.clamp();
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Generator {
    pub id: u32,
    /// `None` only for trunks, which grow from the ground.
    pub parent: Option<u32>,
    pub name: String,
    pub kind: GenKind,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub params: GenParams,
    /// Material id from the document's material library; `None` = the default bark / leaf look.
    #[serde(default)]
    pub material: Option<u32>,
    /// More materials on the stems, blended along a gradient (trunks, branches and roots).
    /// `material` is the first layer, from the start of the gradient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<MaterialBlend>,
}

/// What a material gradient runs along.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlendAxis {
    /// Along each stem's own length: 0 = its base, 1 = its tip.
    #[default]
    Stem,
    /// Height above the ground in metres, the same on every stem (trunk and roots alike).
    Height,
    /// Height as a share of the tree's height: 0 = the ground, 1 = the top.
    HeightShare,
}

impl BlendAxis {
    pub fn label(self) -> &'static str {
        match self {
            BlendAxis::Stem => "Along the stem",
            BlendAxis::Height => "Height (m)",
            BlendAxis::HeightShare => "Height (share)",
        }
    }
    pub fn parse(s: &str) -> Option<BlendAxis> {
        match s {
            "stem" => Some(BlendAxis::Stem),
            "height" => Some(BlendAxis::Height),
            "height_share" => Some(BlendAxis::HeightShare),
            _ => None,
        }
    }
    /// Largest position on this axis.
    pub fn max(self) -> f32 {
        match self {
            BlendAxis::Height => 100.0,
            _ => 1.0,
        }
    }
}

/// One more material in a gradient: it takes over from the layer below at `at`.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct MaterialLayer {
    /// Material id (`None` = the default bark).
    pub material: Option<u32>,
    /// Where it takes over, on the gradient's axis (a share, or metres for Height).
    pub at: f32,
}

impl Default for MaterialLayer {
    fn default() -> Self {
        MaterialLayer { material: None, at: 0.5 }
    }
}

/// Several materials on one generator's stems, blended along a gradient.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct MaterialBlend {
    pub axis: BlendAxis,
    /// The layers above the generator's own material, in order of `at`.
    pub layers: Vec<MaterialLayer>,
    /// Width of each transition, in the axis's units.
    pub width: f32,
    /// Noise mask: how far it moves the transition, as a share of the width (0 = a straight
    /// line, 1 = up to a whole width either way).
    pub noise: f32,
    /// Size of the noise patches in metres.
    pub noise_scale: f32,
    /// 0 = a soft blend over the whole width, 1 = a sharp (but still noisy) edge.
    pub contrast: f32,
}

impl Default for MaterialBlend {
    fn default() -> Self {
        MaterialBlend { axis: BlendAxis::Stem, layers: vec![], width: 0.1, noise: 0.0, noise_scale: 0.3, contrast: 0.0 }
    }
}

/// Most layers above the first one.
pub const MAX_LAYERS: usize = 4;

impl MaterialBlend {
    pub fn clamp(&mut self) {
        let max = self.axis.max();
        self.layers.truncate(MAX_LAYERS);
        for l in &mut self.layers {
            l.at = if l.at.is_finite() { l.at.clamp(0.0, max) } else { 0.5 };
        }
        self.layers.sort_by(|a, b| a.at.total_cmp(&b.at));
        let wmax = if self.axis == BlendAxis::Height { 20.0 } else { 1.0 };
        self.width = if self.width.is_finite() { self.width.clamp(0.0, wmax) } else { 0.1 };
        self.noise = if self.noise.is_finite() { self.noise.clamp(0.0, 1.0) } else { 0.0 };
        self.noise_scale = if self.noise_scale.is_finite() { self.noise_scale.clamp(0.01, 10.0) } else { 0.3 };
        self.contrast = if self.contrast.is_finite() { self.contrast.clamp(0.0, 1.0) } else { 0.0 };
    }

    /// The layer value at a point: the index of the layer it is in plus how far it has blended
    /// into the next one (e.g. 0.3 = 30 % of layer 1 over layer 0). `x` is the position on the
    /// axis, `n` the noise mask there (-1..1).
    pub fn layer_value(&self, x: f32, n: f32) -> f32 {
        (0..self.layers.len()).map(|k| self.weight(k, x, n)).sum()
    }

    /// Half the width over which a transition actually ramps (contrast narrows it).
    pub fn half_ramp(&self) -> f32 {
        0.5 * self.width * (1.0 - 0.95 * self.contrast)
    }

    /// How far the noise mask can move a transition, in the axis's units.
    pub fn noise_reach(&self) -> f32 {
        self.noise * self.width
    }

    /// Weight of layer `k + 1` (index `k` in `layers`) over the layers below it at axis position
    /// `x` with noise `n` (-1..1): a smooth step over the ramp around its `at`.
    pub fn weight(&self, k: usize, x: f32, n: f32) -> f32 {
        let Some(l) = self.layers.get(k) else { return 0.0 };
        let x = x + self.noise_reach() * n;
        let hw = self.half_ramp();
        if hw <= 1e-6 {
            return if x >= l.at { 1.0 } else { 0.0 };
        }
        let t = ((x - (l.at - hw)) / (2.0 * hw)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// The layers a part of a stem spanning axis positions `lo..hi` is drawn with: (layer `k`
    /// it is fully in, and whether it can blend into layer `k + 1` somewhere, noise included).
    pub fn span_layers(&self, lo: f32, hi: f32) -> (u32, bool) {
        let reach = self.half_ramp() + self.noise_reach() + 1e-5;
        let k = self.layers.iter().filter(|l| l.at + reach <= lo).count();
        let blends = self.layers.get(k).is_some_and(|l| l.at - reach < hi);
        (k as u32, blends)
    }
}

/// A surface material: colour (or tint over a texture), optional texture and normal map.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Material {
    pub id: u32,
    pub name: String,
    /// Base colour; multiplies the texture when there is one (white = texture as is).
    pub color: [u8; 3],
    /// Colour/albedo image (PNG or JPEG), absolute path.
    pub texture: Option<String>,
    /// Tangent-space normal map image (PNG or JPEG), absolute path.
    pub normal_map: Option<String>,
    /// How strongly the normal map bends the lighting (1 = as painted).
    pub normal_strength: f32,
    /// The normal map uses the DirectX convention (green = down) instead of OpenGL (green = up).
    pub normal_flip_green: bool,
    pub roughness: f32,
    /// Texture repeats around (u) and along (v) a stem. Leaves always use 1 x 1. The along
    /// value counts repeats per circumference of the stem, so [n, n] keeps a SQUARE image
    /// square; see `keep_proportions` for any other shape.
    pub tiling: [f32; 2],
    /// Work out the along repeats from the image's shape (`tiling[0]` times its width over
    /// its height), so a tall bark photo keeps its proportions; `tiling[1]` is then unused.
    pub keep_proportions: bool,
    /// Treat transparent texture pixels as holes (for leaf cards).
    pub alpha_cutout: bool,
    pub double_sided: bool,
    /// Leaf pictures in a texture atlas: each leaf using this material shows one of them
    /// (picked at random by weight), as a card shaped like the picture. Empty = the whole image.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub atlas: Vec<AtlasCell>,
}

/// One leaf picture in a texture atlas.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct AtlasCell {
    /// Left, top, width and height as shares of the image (0..1, origin top left).
    pub rect: [f32; 4],
    /// Where the leaf's stalk meets the twig, as shares of the rectangle from its left and top
    /// edges (default: the middle of its bottom edge).
    pub pivot: [f32; 2],
    /// How often this picture is used compared with the others.
    pub weight: f32,
}

impl Default for AtlasCell {
    fn default() -> Self {
        AtlasCell { rect: [0.0, 0.0, 1.0, 1.0], pivot: [0.5, 1.0], weight: 1.0 }
    }
}

impl AtlasCell {
    pub fn clamp(&mut self) {
        let f = |x: f32, lo: f32, hi: f32, d: f32| if x.is_finite() { x.clamp(lo, hi) } else { d };
        self.rect[0] = f(self.rect[0], 0.0, 0.999, 0.0);
        self.rect[1] = f(self.rect[1], 0.0, 0.999, 0.0);
        self.rect[2] = f(self.rect[2], 0.001, 1.0 - self.rect[0], 1.0 - self.rect[0]);
        self.rect[3] = f(self.rect[3], 0.001, 1.0 - self.rect[1], 1.0 - self.rect[1]);
        self.pivot = self.pivot.map(|p| f(p, 0.0, 1.0, 0.5));
        self.weight = f(self.weight, 0.0, 100.0, 1.0);
    }
    /// Texture coordinates (v up, as on meshes) of a point given as shares of the rectangle
    /// (`x` from its left edge, `y` up from its bottom edge).
    pub fn uv(&self, x: f32, y: f32) -> [f32; 2] {
        [self.rect[0] + x * self.rect[2], 1.0 - (self.rect[1] + self.rect[3]) + y * self.rect[3]]
    }
}

/// Most pictures in one material's leaf atlas.
pub const MAX_ATLAS_CELLS: usize = 64;

impl Default for Material {
    fn default() -> Self {
        Material {
            id: 0,
            name: "Material".into(),
            color: [255, 255, 255],
            texture: None,
            normal_map: None,
            normal_strength: 1.0,
            normal_flip_green: false,
            roughness: 0.85,
            tiling: [1.0, 1.0],
            keep_proportions: false,
            alpha_cutout: false,
            double_sided: false,
            atlas: vec![],
        }
    }
}

/// Width over height of an image file, read from its header once per path.
fn image_aspect(path: &str) -> Option<f32> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Option<f32>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(a) = cache.lock().ok()?.get(path) {
        return *a;
    }
    let a = image::image_dimensions(path).ok().filter(|(w, h)| *w > 0 && *h > 0).map(|(w, h)| w as f32 / h as f32);
    if let Ok(mut c) = cache.lock() {
        c.insert(path.to_string(), a);
    }
    a
}

impl Material {
    /// The tiling the bark is drawn with: `tiling`, or with `keep_proportions` the along
    /// repeats that keep the image's shape (texels square on the stem).
    pub fn effective_tiling(&self) -> [f32; 2] {
        match (self.keep_proportions, self.texture.as_deref().or(self.normal_map.as_deref()).and_then(image_aspect)) {
            (true, Some(aspect)) => [self.tiling[0], (self.tiling[0] * aspect).clamp(0.05, 50.0)],
            _ => self.tiling,
        }
    }

    /// The atlas picture for a leaf, from a random number 0..1 (None: the whole image).
    pub fn atlas_pick(&self, r: f32) -> Option<&AtlasCell> {
        let total: f32 = self.atlas.iter().map(|c| c.weight).sum();
        if self.atlas.is_empty() {
            return None;
        }
        if total <= 0.0 {
            return self.atlas.first();
        }
        let mut x = r * total;
        for c in &self.atlas {
            if x < c.weight {
                return Some(c);
            }
            x -= c.weight;
        }
        self.atlas.iter().rev().find(|c| c.weight > 0.0)
    }

    pub fn clamp(&mut self) {
        self.atlas.truncate(MAX_ATLAS_CELLS);
        for c in &mut self.atlas {
            c.clamp();
        }
        self.roughness = self.roughness.clamp(0.0, 1.0);
        self.normal_strength = if self.normal_strength.is_finite() { self.normal_strength.clamp(0.0, 4.0) } else { 1.0 };
        self.tiling = self.tiling.map(|t| t.clamp(0.05, 50.0));
        for p in [&mut self.texture, &mut self.normal_map] {
            if p.as_deref().map(|s| s.trim().is_empty()).unwrap_or(false) {
                *p = None;
            }
        }
    }
}

fn yes() -> bool {
    true
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Wind {
    pub strength: f32,
    pub direction: f32,
    pub gustiness: f32,
    pub speed: f32,
}

impl Default for Wind {
    fn default() -> Self {
        Wind { strength: 0.4, direction: 30.0, gustiness: 0.4, speed: 1.0 }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct TreeSettings {
    pub seed: u32,
    pub scale: f32,
    pub bark_color: [u8; 3],
    pub leaf_color: [u8; 3],
    pub leaf_color_variation: f32,
    pub autumn: f32,
    /// Fraction of leaves that have fallen (0 = full crown, 1 = bare winter tree).
    pub leaf_drop: f32,
    /// Strength of the ambient occlusion baked into vertex colours (0 = none).
    pub ao_strength: f32,
    /// Growth stage: 1 = mature tree as designed, smaller values = younger (fewer, thinner, shorter stems).
    pub age: f32,
    /// Branches grow out of their parent's surface as one connected mesh (a hole is cut in the
    /// parent and bridged to the branch) instead of being separate tubes stuck into it.
    pub join_branches: bool,
    /// How joined branches meet their parent (see [`JoinMethod`]).
    pub join_method: JoinMethod,
    pub wind: Wind,
}

/// How a branch meets its parent when `join_branches` is on.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum JoinMethod {
    /// One watertight mesh: an opening is cut into the parent and stitched to the branch.
    #[default]
    Stitch,
    /// Welding as in production tree tools: the parent stays whole; the branch base is wrapped
    /// onto the parent's surface with a curved fillet, and its normals, colour and wind blend
    /// into the parent's so the seam disappears. Parent and branch stay separate pieces.
    Weld,
}

impl Default for TreeSettings {
    fn default() -> Self {
        TreeSettings {
            seed: 1,
            scale: 1.0,
            bark_color: [110, 88, 70],
            leaf_color: [74, 122, 46],
            leaf_color_variation: 0.2,
            autumn: 0.0,
            leaf_drop: 0.0,
            ao_strength: 0.75,
            age: 1.0,
            join_branches: true,
            join_method: JoinMethod::Stitch,
            wind: Wind::default(),
        }
    }
}

impl TreeSettings {
    pub fn clamp(&mut self) {
        self.scale = self.scale.clamp(0.05, 20.0);
        self.leaf_color_variation = self.leaf_color_variation.clamp(0.0, 1.0);
        self.autumn = self.autumn.clamp(0.0, 1.0);
        self.leaf_drop = self.leaf_drop.clamp(0.0, 1.0);
        self.ao_strength = self.ao_strength.clamp(0.0, 1.0);
        self.age = self.age.clamp(0.05, 1.0);
        self.wind.strength = self.wind.strength.clamp(0.0, 2.0);
        self.wind.direction = self.wind.direction.rem_euclid(360.0);
        self.wind.gustiness = self.wind.gustiness.clamp(0.0, 1.0);
        self.wind.speed = self.wind.speed.clamp(0.05, 5.0);
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Document {
    pub name: String,
    pub settings: TreeSettings,
    pub generators: Vec<Generator>,
    pub next_id: u32,
    pub materials: Vec<Material>,
    /// Forces that bend stems while they grow (wind-swept shapes, light seeking, obstacles).
    pub forces: Vec<Force>,
    /// Optional volume that branches and leaves are cut to (topiary, hedges, fitting a space).
    pub pruning: Pruning,
    /// Hand edits of individual branches.
    pub nodes: Vec<NodeEdit>,
    /// Collision objects generated for game-engine physics.
    pub collision: Collision,
    /// Triangle budgets for the mesh levels of detail.
    pub budget: TriangleBudget,
    /// Growth animation from a sprout to the mature tree, with named stages.
    pub growth: Growth,
}

/// One moment of the growth animation (0 = nothing above ground, 1 = the mature tree), with an
/// optional name. Exports get a clip per stage.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct GrowthStage {
    /// Optional; unnamed stages are called "Stage 1", "Stage 2", ... by their place.
    pub name: String,
    pub at: f32,
}

impl Default for GrowthStage {
    fn default() -> Self {
        GrowthStage { name: String::new(), at: 1.0 }
    }
}

impl GrowthStage {
    pub fn named(name: &str, at: f32) -> Self {
        GrowthStage { name: name.into(), at }
    }
}

/// The growth animation: the same mesh moves from a seedling to the mature tree (the whole tree
/// grows at once, young trees are small trees in their mature proportions, branches come out of closed bark and
/// leaves keep coming), so engines can play it as morph targets.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Growth {
    /// Exports carry the growth animation (GLB: morph targets and clips; OBJ: one file per
    /// stage; JSON: the growth timing of every stem and leaf).
    pub export: bool,
    /// Named stages, in growing order; the last one is normally the mature tree (1.0).
    pub stages: Vec<GrowthStage>,
    /// Seconds the whole animation takes, from nothing to the mature tree.
    pub duration: f32,
    /// Shapes the animation is sampled at (morph targets); more is smoother and larger.
    pub keyframes: u32,
}

impl Default for Growth {
    fn default() -> Self {
        Growth {
            export: false,
            stages: vec![
                GrowthStage::named("Sprout", 0.12),
                GrowthStage::named("Sapling", 0.38),
                GrowthStage::named("Young", 0.68),
                GrowthStage::named("Mature", 1.0),
            ],
            duration: 12.0,
            keyframes: 16,
        }
    }
}

impl Growth {
    pub const MAX_STAGES: usize = 8;

    /// The stage's name, or "Stage <n>" (by its place in growing order) when it has none.
    pub fn label(&self, i: usize) -> String {
        match self.stages.get(i) {
            Some(st) if !st.name.trim().is_empty() => st.name.trim().to_string(),
            _ => format!("Stage {}", i + 1),
        }
    }

    /// Index of the stage at growth `g`, if one sits there.
    pub fn stage_at(&self, g: f32) -> Option<usize> {
        self.stages.iter().position(|s| (s.at - g).abs() < 0.002)
    }

    pub fn clamp(&mut self) {
        self.stages.truncate(Self::MAX_STAGES);
        for st in &mut self.stages {
            st.at = st.at.clamp(0.01, 1.0);
            st.name = st.name.trim().chars().take(40).collect();
        }
        self.stages.sort_by(|a, b| a.at.total_cmp(&b.at));
        self.duration = self.duration.clamp(0.5, 600.0);
        self.keyframes = self.keyframes.clamp(4, 48);
    }
}

/// Most triangles each mesh level of detail (LOD0 to LOD3) may have. The billboard (LOD4) is
/// always 16 triangles and has no budget.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct TriangleBudget {
    pub enabled: bool,
    /// Derive the four levels from `max`: LOD0 gets the maximum, each next level half of the one before.
    pub auto: bool,
    /// The maximum for LOD0 when `auto` is on.
    pub max: u32,
    /// Budgets for LOD0 to LOD3 when `auto` is off (0 = no limit for that level).
    pub levels: [u32; 4],
    /// Reduce detail until each level fits. Off: levels keep their usual detail and the ones
    /// over budget are only reported.
    pub fit: bool,
    /// Most leaf card area (m², one side) for LOD0 to LOD3, checked and reported (0 = no limit).
    /// Card area is what foliage costs in overdraw, which triangle counts do not show.
    pub leaf_area: [f32; 4],
}

impl Default for TriangleBudget {
    fn default() -> Self {
        TriangleBudget { enabled: false, auto: true, max: 50_000, levels: [50_000, 25_000, 12_500, 6_250], fit: true, leaf_area: [0.0; 4] }
    }
}

impl TriangleBudget {
    /// Share of the maximum each level gets when the budgets are derived from it.
    pub const RATIOS: [f32; 4] = [1.0, 0.5, 0.25, 0.125];
    pub const LIMIT: u32 = 5_000_000;

    /// The budget of a level of detail, if budgets are on and the level has one.
    pub fn level(&self, lod: u32) -> Option<u32> {
        if !self.enabled || lod > 3 {
            return None;
        }
        let b = self.derived()[lod as usize];
        (b > 0).then_some(b)
    }

    /// The four budgets in effect (derived from `max` when `auto` is on).
    pub fn derived(&self) -> [u32; 4] {
        if self.auto {
            Self::RATIOS.map(|r| (self.max as f32 * r).round() as u32)
        } else {
            self.levels
        }
    }

    /// The leaf card area budget of a level (m²), if budgets are on and the level has one.
    pub fn area_level(&self, lod: u32) -> Option<f32> {
        if !self.enabled || lod > 3 {
            return None;
        }
        let a = self.leaf_area[lod as usize];
        (a > 0.0).then_some(a)
    }

    pub fn clamp(&mut self) {
        self.max = self.max.clamp(16, Self::LIMIT);
        self.leaf_area = self.leaf_area.map(|a| if a.is_finite() { a.clamp(0.0, 100_000.0) } else { 0.0 });
        for l in &mut self.levels {
            *l = (*l).min(Self::LIMIT);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum CollisionNaming {
    /// `<Tree>_Collision_NN` with capsule data in extras (Unity or custom importers).
    #[default]
    Neutral,
    /// `UCX_<RenderMesh>_NN` (Unreal's convex collision convention).
    Unreal,
    /// `<Tree>_Collision_NN-convcolonly` (Godot's import hint).
    Godot,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Collision {
    pub enabled: bool,
    /// Deepest level that gets collision: 0 = trunks only, 1 = plus main branches, ...
    pub max_depth: u32,
    /// Stems thinner than this (metres) get no collision.
    pub min_radius: f32,
    /// Capsules per stem (more follows curved stems better).
    pub segments: u32,
    /// Capsule radius relative to the stem radius.
    pub radius_scale: f32,
    pub naming: CollisionNaming,
}

impl Default for Collision {
    fn default() -> Self {
        Collision { enabled: true, max_depth: 1, min_radius: 0.06, segments: 3, radius_scale: 1.0, naming: CollisionNaming::Neutral }
    }
}

impl Collision {
    pub fn clamp(&mut self) {
        self.max_depth = self.max_depth.min(4);
        self.min_radius = self.min_radius.clamp(0.0, 5.0);
        self.segments = self.segments.clamp(1, 12);
        self.radius_scale = self.radius_scale.clamp(0.2, 3.0);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ForceKind {
    /// Pushes every stem the same way.
    Direction,
    /// Pulls stems towards a point (negative strength pushes them away).
    Attractor,
}

impl ForceKind {
    pub fn label(self) -> &'static str {
        match self {
            ForceKind::Direction => "Direction",
            ForceKind::Attractor => "Attractor",
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Force {
    pub id: u32,
    pub name: String,
    pub kind: ForceKind,
    pub enabled: bool,
    /// For direction forces: the direction pushed towards.
    pub direction: [f32; 3],
    /// For attractors: the point in metres.
    pub position: [f32; 3],
    /// -2..2; negative reverses the force.
    pub strength: f32,
}

impl Default for Force {
    fn default() -> Self {
        Force { id: 0, name: "Force".into(), kind: ForceKind::Direction, enabled: true, direction: [1.0, 0.0, 0.0], position: [0.0, 8.0, 0.0], strength: 0.5 }
    }
}

impl Force {
    pub fn clamp(&mut self) {
        self.strength = self.strength.clamp(-2.0, 2.0);
        let d = glam::Vec3::from(self.direction);
        self.direction = if d.length_squared() < 1e-6 { [0.0, 1.0, 0.0] } else { d.normalize().into() };
        self.position = self.position.map(|v| v.clamp(-500.0, 500.0));
    }
    /// The unit direction this force pushes a stem at `p`.
    pub fn vector(&self, p: glam::Vec3) -> glam::Vec3 {
        match self.kind {
            ForceKind::Direction => glam::Vec3::from(self.direction).normalize_or_zero(),
            ForceKind::Attractor => (glam::Vec3::from(self.position) - p).normalize_or_zero(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum PruneShape {
    Box,
    Ellipsoid,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Pruning {
    pub enabled: bool,
    pub shape: PruneShape,
    /// Centre of the volume in metres.
    pub center: [f32; 3],
    /// Half-size of the volume along x, y, z in metres.
    pub size: [f32; 3],
}

impl Default for Pruning {
    fn default() -> Self {
        Pruning { enabled: false, shape: PruneShape::Ellipsoid, center: [0.0, 6.0, 0.0], size: [4.0, 3.5, 4.0] }
    }
}

impl Pruning {
    pub fn clamp(&mut self) {
        self.size = self.size.map(|v| v.clamp(0.05, 500.0));
        self.center = self.center.map(|v| v.clamp(-500.0, 500.0));
    }
    pub fn contains(&self, p: glam::Vec3) -> bool {
        let q = (p - glam::Vec3::from(self.center)) / glam::Vec3::from(self.size);
        match self.shape {
            PruneShape::Box => q.abs().max_element() <= 1.0,
            PruneShape::Ellipsoid => q.length_squared() <= 1.0,
        }
    }
    /// Fit the volume around a tree of this height / crown radius (slightly smaller, so it trims).
    pub fn fit(&mut self, height: f32, radius: f32) {
        self.center = [0.0, height * 0.6, 0.0];
        self.size = [radius * 0.65, height * 0.36, radius * 0.65];
        self.clamp();
    }
}

impl Default for Document {
    fn default() -> Self {
        preset("Oak").unwrap()
    }
}

impl Document {
    /// A generator whose stems are shed while the tree is young (`GenGrowth::shed`): a stem
    /// generator on a parent (a trunk is never shed), enabled.
    pub fn is_shed(&self, g: &Generator) -> bool {
        g.enabled && g.parent.is_some() && g.params.growth.sheds()
    }
    /// The document has stems that are shed while the tree is young.
    pub fn has_shed(&self) -> bool {
        self.generators.iter().any(|g| self.is_shed(g))
    }
    /// The grown tree's document: every shed generator disabled (with what grows on it, which
    /// then has nothing to grow on). Every generator rolls its own random numbers, so the rest
    /// of the tree is the same as with them.
    pub fn without_shed(&self) -> Document {
        let mut d = self.clone();
        let shed: Vec<u32> = d.generators.iter().filter(|g| self.is_shed(g)).map(|g| g.id).collect();
        for g in d.generators.iter_mut().filter(|g| shed.contains(&g.id)) {
            g.enabled = false;
        }
        d
    }
    pub fn get(&self, id: u32) -> Option<&Generator> {
        self.generators.iter().find(|g| g.id == id)
    }
    pub fn get_mut(&mut self, id: u32) -> Option<&mut Generator> {
        self.generators.iter_mut().find(|g| g.id == id)
    }
    pub fn children(&self, id: Option<u32>) -> impl Iterator<Item = &Generator> {
        self.generators.iter().filter(move |g| g.parent == id)
    }

    /// Add a generator under `parent` (None = a new trunk). Returns its id.
    pub fn add(&mut self, kind: GenKind, parent: Option<u32>, name: Option<String>) -> Result<u32, String> {
        let parent = if kind == GenKind::Trunk { None } else { parent };
        if kind != GenKind::Trunk {
            let Some(pid) = parent else {
                return Err("branches and leaves need a parent generator id".into());
            };
            let Some(p) = self.get(pid) else {
                return Err(format!("no generator with id {pid}"));
            };
            if kind == GenKind::Roots && p.kind != GenKind::Trunk {
                return Err("roots grow from a trunk; pick a trunk generator as parent".into());
            }
            if p.kind == GenKind::Leaves {
                return Err("leaves cannot have children; pick a trunk or branches generator".into());
            }
        }
        let id = self.next_id.max(self.generators.iter().map(|g| g.id + 1).max().unwrap_or(1));
        self.next_id = id + 1;
        let mut params = GenParams::for_kind(kind);
        // Child branches of branches are usually twigs: shorter and thinner.
        if kind == GenKind::Branches {
            if let Some(p) = parent.and_then(|p| self.get(p)) {
                if p.kind == GenKind::Roots {
                    params.count = 3;
                    params.start = 0.25;
                    params.length = 0.45;
                    params.angle = 40.0;
                    params.gravity = 0.3;
                    params.wiggle = 0.5;
                    params.sides = 5;
                    params.segments = 6;
                    params.taper = 0.1;
                    params.force_response = 0.0;
                }
                if p.kind == GenKind::Branches {
                    params.length = 0.4;
                    params.count = 8;
                    params.sides = 4;
                    params.segments = 4;
                }
            }
        }
        let name = name.unwrap_or_else(|| {
            let n = self.generators.iter().filter(|g| g.kind == kind).count() + 1;
            format!("{} {}", kind.label(), n)
        });
        self.generators.push(Generator { id, parent, name, kind, enabled: true, params, material: None, blend: None });
        Ok(id)
    }

    /// Remove a generator and everything below it. Returns how many were removed.
    pub fn remove(&mut self, id: u32) -> usize {
        let mut doomed = vec![id];
        let mut i = 0;
        while i < doomed.len() {
            let cur = doomed[i];
            doomed.extend(self.generators.iter().filter(|g| g.parent == Some(cur)).map(|g| g.id));
            i += 1;
        }
        let before = self.generators.len();
        self.generators.retain(|g| !doomed.contains(&g.id));
        before - self.generators.len()
    }

    /// Depth-first order with depth, for the hierarchy view.
    pub fn ordered(&self) -> Vec<(u32, usize)> {
        let mut out = Vec::new();
        fn walk(doc: &Document, parent: Option<u32>, depth: usize, out: &mut Vec<(u32, usize)>) {
            for g in doc.children(parent) {
                out.push((g.id, depth));
                walk(doc, Some(g.id), depth + 1, out);
            }
        }
        walk(self, None, 0, &mut out);
        out
    }

    pub fn node(&self, key: u32) -> Option<&NodeEdit> {
        self.nodes.iter().find(|n| n.key == key)
    }
    /// The edit for a branch, created if missing.
    pub fn node_mut(&mut self, key: u32) -> &mut NodeEdit {
        if let Some(i) = self.nodes.iter().position(|n| n.key == key) {
            &mut self.nodes[i]
        } else {
            self.nodes.push(NodeEdit { key, ..Default::default() });
            self.nodes.last_mut().unwrap()
        }
    }
    /// Drop edits that no longer change anything.
    pub fn tidy_nodes(&mut self) {
        for n in &mut self.nodes {
            n.clamp();
        }
        self.nodes.retain(|n| !n.is_noop());
    }

    pub fn add_force(&mut self, mut f: Force) -> u32 {
        let id = self.forces.iter().map(|f| f.id + 1).max().unwrap_or(1).max(1);
        f.id = id;
        if f.name.trim().is_empty() || f.name == "Force" {
            f.name = format!("{} {}", f.kind.label(), id);
        }
        f.clamp();
        self.forces.push(f);
        id
    }
    pub fn force_mut(&mut self, id: u32) -> Option<&mut Force> {
        self.forces.iter_mut().find(|f| f.id == id)
    }
    pub fn remove_force(&mut self, id: u32) -> bool {
        let n = self.forces.len();
        self.forces.retain(|f| f.id != id);
        n != self.forces.len()
    }

    pub fn material(&self, id: u32) -> Option<&Material> {
        self.materials.iter().find(|m| m.id == id)
    }
    pub fn material_mut(&mut self, id: u32) -> Option<&mut Material> {
        self.materials.iter_mut().find(|m| m.id == id)
    }
    /// The material a generator uses, if it has one assigned.
    pub fn gen_material(&self, gen_id: u32) -> Option<&Material> {
        self.get(gen_id).and_then(|g| g.material).and_then(|m| self.material(m))
    }
    /// How a gradient layer's material maps onto a generator's bark: the factor its texture
    /// coordinates (laid out with the generator's own material's tiling) are multiplied by so
    /// the layer shows its OWN tiling. [1, 1] for the generator's own material.
    pub fn layer_uv_scale(&self, gen_id: u32, mat_id: u32) -> [f32; 2] {
        let base = self.gen_material(gen_id).map(|m| m.effective_tiling()).unwrap_or([1.0, 1.0]);
        let own = self.material(mat_id).map(|m| m.effective_tiling()).unwrap_or(base);
        [own[0] / base[0].max(1e-6), own[1] / base[1].max(1e-6)]
    }
    pub fn add_material(&mut self, mut m: Material) -> u32 {
        let id = self.materials.iter().map(|m| m.id + 1).max().unwrap_or(1).max(1);
        m.id = id;
        if m.name.trim().is_empty() {
            m.name = format!("Material {id}");
        }
        m.clamp();
        self.materials.push(m);
        id
    }
    /// Delete a material; generators using it go back to the default look.
    pub fn remove_material(&mut self, id: u32) -> bool {
        let before = self.materials.len();
        self.materials.retain(|m| m.id != id);
        for g in &mut self.generators {
            if g.material == Some(id) {
                g.material = None;
            }
            if let Some(b) = &mut g.blend {
                for l in &mut b.layers {
                    if l.material == Some(id) {
                        l.material = None;
                    }
                }
            }
        }
        before != self.materials.len()
    }
    pub fn material_users(&self, id: u32) -> Vec<&Generator> {
        self.generators.iter().filter(|g| g.material == Some(id) || g.blend.as_ref().is_some_and(|b| b.layers.iter().any(|l| l.material == Some(id)))).collect()
    }
    /// The material id of layer `k` of a generator's stems (0 = its own material; ids of 0 =
    /// the default look). Layers past the last one use the last.
    pub fn layer_material_id(&self, gen_id: u32, k: u32) -> u32 {
        let Some(g) = self.get(gen_id) else { return 0 };
        if k == 0 {
            return g.material.unwrap_or(0);
        }
        match &g.blend {
            Some(b) if !b.layers.is_empty() => b.layers[(k as usize - 1).min(b.layers.len() - 1)].material.unwrap_or(0),
            _ => g.material.unwrap_or(0),
        }
    }

    pub fn clamp_all(&mut self) {
        self.settings.clamp();
        self.growth.clamp();
        self.pruning.clamp();
        self.tidy_nodes();
        for f in &mut self.forces {
            f.clamp();
        }
        for m in &mut self.materials {
            m.clamp();
        }
        let ids: Vec<u32> = self.materials.iter().map(|m| m.id).collect();
        for g in &mut self.generators {
            if g.material.map(|m| !ids.contains(&m)).unwrap_or(false) {
                g.material = None;
            }
            if g.kind == GenKind::Leaves {
                g.blend = None;
            }
            if let Some(b) = &mut g.blend {
                for l in &mut b.layers {
                    if l.material.map(|m| !ids.contains(&m)).unwrap_or(false) {
                        l.material = None;
                    }
                }
                b.clamp();
                if b.layers.is_empty() {
                    g.blend = None;
                }
            }
        }
        for g in &mut self.generators {
            let k = g.kind;
            g.params.clamp(k);
        }
    }
}

// ---------------------------------------------------------------- presets

pub const PRESETS: [(&str, &str); 8] = [
    ("Oak", "Broad, rounded deciduous tree with a sturdy flared trunk"),
    ("Pine", "Tall conifer with whorled branches and needles"),
    ("Birch", "Slender pale trunk with drooping twigs and small round leaves"),
    ("Willow", "Short trunk with long hanging curtains of narrow leaves"),
    ("Palm", "Leaning trunk topped with a crown of large fronds"),
    ("Maple (autumn)", "Wide hemispherical crown in autumn colours"),
    ("Shrub", "Low bush with several stems from the ground"),
    ("Sapling", "Young thin tree with few branches"),
];

struct B {
    doc: Document,
}

impl B {
    fn new(name: &str, settings: TreeSettings) -> Self {
        B { doc: Document { name: name.into(), settings, generators: vec![], next_id: 1, materials: vec![], forces: vec![], pruning: Pruning::default(), nodes: vec![], collision: Collision::default(), budget: TriangleBudget::default(), growth: Growth::default() } }
    }
    fn add(&mut self, kind: GenKind, parent: Option<u32>, name: &str, f: impl FnOnce(&mut GenParams)) -> u32 {
        let id = self.doc.add(kind, parent, Some(name.into())).unwrap();
        let g = self.doc.get_mut(id).unwrap();
        g.params = GenParams::for_kind(kind);
        f(&mut g.params);
        id
    }
}

pub fn preset_names() -> Vec<&'static str> {
    PRESETS.iter().map(|p| p.0).collect()
}

pub fn preset(name: &str) -> Option<Document> {
    use GenKind::*;
    let key = name.to_ascii_lowercase();
    let key = key.split_whitespace().next().unwrap_or("");
    let mut b;
    match key {
        "oak" => {
            b = B::new("Oak", TreeSettings::default());
            let t = b.add(Trunk, None, "Trunk", |p| {
                p.length = 7.0;
                p.radius = 0.38;
                p.taper = 0.12;
                p.splits = 3;
                p.split_at = 0.42;
                p.split_angle = 30.0;
                p.flare = 0.7;
                p.wiggle = 0.25;
            });
            let r = b.add(Roots, Some(t), "Roots", |p| {
                p.count = 7;
                p.length = 0.32;
                p.radius = 0.8;
            });
            b.add(Branches, Some(r), "Side roots", |p| {
                p.count = 3;
                p.start = 0.3;
                p.length = 0.45;
                p.angle = 45.0;
                p.gravity = 0.3;
                p.wiggle = 0.5;
                p.radius = 0.5;
                p.taper = 0.1;
                p.segments = 6;
                p.sides = 5;
                p.force_response = 0.0;
                p.profile = Profile::Cylindrical;
            });
            let br = b.add(Branches, Some(t), "Main branches", |p| {
                p.count = 15;
                p.start = 0.35;
                p.end = 0.95;
                p.length = 0.62;
                p.profile = Profile::Spherical;
                p.angle = 52.0;
                p.angle_variation = 12.0;
                p.gravity = 0.12;
                p.wiggle = 0.45;
                p.segments = 8;
                p.radius = 0.55;
                p.sides = 7;
            });
            let tw = b.add(Branches, Some(br), "Twigs", |p| {
                p.count = 12;
                p.start = 0.2;
                p.length = 0.42;
                p.angle = 45.0;
                p.angle_variation = 15.0;
                p.gravity = 0.1;
                p.wiggle = 0.5;
                p.segments = 4;
                p.sides = 4;
            });
            b.add(Leaves, Some(tw), "Leaves", |p| {
                p.count = 14;
                p.leaf_size = 0.42;
                p.leaf_shape = LeafShape::Broad;
            });
        }
        "pine" => {
            b = B::new(
                "Pine",
                TreeSettings { bark_color: [92, 68, 52], leaf_color: [38, 78, 44], leaf_color_variation: 0.1, ..Default::default() },
            );
            let t = b.add(Trunk, None, "Trunk", |p| {
                p.length = 15.0;
                p.radius = 0.3;
                p.taper = 0.03;
                p.flare = 0.4;
                p.wiggle = 0.1;
                p.segments = 16;
                p.sides = 10;
            });
            b.add(Roots, Some(t), "Roots", |p| {
                p.count = 5;
                p.length = 0.12;
                p.radius = 0.7;
                p.buried = 0.7;
            });
            let br = b.add(Branches, Some(t), "Whorled branches", |p| {
                p.count = 44;
                p.whorl = 4;
                p.phyllotaxis = 40.0;
                p.start = 0.15;
                p.end = 0.98;
                p.length = 0.32;
                p.profile = Profile::Conical;
                p.angle = 82.0;
                p.angle_variation = 8.0;
                p.gravity = 0.25;
                p.wiggle = 0.15;
                p.segments = 5;
                p.radius = 0.3;
                p.taper = 0.2;
                p.sides = 5;
            });
            let tw = b.add(Branches, Some(br), "Twigs", |p| {
                p.count = 10;
                p.start = 0.1;
                p.length = 0.38;
                p.angle = 55.0;
                p.gravity = 0.15;
                p.segments = 3;
                p.sides = 3;
            });
            b.add(Leaves, Some(tw), "Needles", |p| {
                p.count = 12;
                p.start = 0.05;
                p.leaf_size = 0.5;
                p.leaf_shape = LeafShape::Needle;
                p.angle = 40.0;
            });
        }
        "birch" => {
            b = B::new(
                "Birch",
                TreeSettings { bark_color: [222, 218, 206], leaf_color: [118, 150, 50], ..Default::default() },
            );
            let t = b.add(Trunk, None, "Trunk", |p| {
                p.length = 11.0;
                p.radius = 0.17;
                p.taper = 0.15;
                p.flare = 0.3;
                p.wiggle = 0.15;
                p.segments = 14;
                p.sides = 10;
            });
            let br = b.add(Branches, Some(t), "Branches", |p| {
                p.count = 22;
                p.start = 0.3;
                p.end = 0.98;
                p.length = 0.42;
                p.profile = Profile::Flame;
                p.angle = 38.0;
                p.gravity = 0.05;
                p.wiggle = 0.35;
                p.segments = 6;
                p.radius = 0.45;
                p.sides = 5;
            });
            let tw = b.add(Branches, Some(br), "Drooping twigs", |p| {
                p.count = 8;
                p.start = 0.3;
                p.length = 0.55;
                p.angle = 35.0;
                p.gravity = 0.6;
                p.wiggle = 0.3;
                p.segments = 5;
                p.sides = 3;
            });
            b.add(Leaves, Some(tw), "Leaves", |p| {
                p.count = 14;
                p.leaf_size = 0.2;
                p.leaf_shape = LeafShape::Round;
            });
        }
        "willow" => {
            b = B::new(
                "Willow",
                TreeSettings { bark_color: [96, 82, 66], leaf_color: [120, 148, 70], ..Default::default() },
            );
            let t = b.add(Trunk, None, "Trunk", |p| {
                p.length = 6.0;
                p.radius = 0.42;
                p.taper = 0.15;
                p.flare = 0.6;
                p.wiggle = 0.3;
                p.splits = 3;
                p.split_at = 0.5;
                p.split_angle = 30.0;
            });
            b.add(Roots, Some(t), "Roots", |p| {
                p.count = 8;
                p.length = 0.45;
                p.radius = 0.75;
                p.wiggle = 0.55;
            });
            let br = b.add(Branches, Some(t), "Limbs", |p| {
                p.count = 12;
                p.start = 0.5;
                p.end = 1.0;
                p.length = 0.75;
                p.profile = Profile::Spherical;
                p.angle = 48.0;
                p.gravity = -0.15;
                p.wiggle = 0.35;
                p.segments = 8;
                p.radius = 0.55;
            });
            let tw = b.add(Branches, Some(br), "Hanging strands", |p| {
                p.count = 20;
                p.start = 0.3;
                p.length = 0.95;
                p.profile = Profile::Cylindrical;
                p.angle = 30.0;
                p.gravity = 0.95;
                p.wiggle = 0.15;
                p.segments = 9;
                p.radius = 0.35;
                p.sides = 3;
            });
            b.add(Leaves, Some(tw), "Leaves", |p| {
                p.count = 26;
                p.start = 0.1;
                p.leaf_size = 0.32;
                p.leaf_aspect = 0.7;
                p.leaf_shape = LeafShape::Narrow;
                p.angle = 25.0;
            });
        }
        "palm" => {
            b = B::new(
                "Palm",
                TreeSettings { bark_color: [128, 108, 84], leaf_color: [70, 120, 40], ..Default::default() },
            );
            let t = b.add(Trunk, None, "Trunk", |p| {
                p.length = 9.0;
                p.radius = 0.2;
                p.taper = 0.7;
                p.flare = 0.5;
                p.angle = 10.0;
                p.gravity = -0.08;
                p.wiggle = 0.12;
                p.segments = 16;
            });
            b.add(Roots, Some(t), "Root ball", |p| {
                p.count = 22;
                p.length = 0.07;
                p.radius = 0.16;
                p.angle = 120.0;
                p.angle_variation = 15.0;
                p.wiggle = 0.3;
                p.sides = 4;
                p.segments = 5;
                p.buried = 0.6;
            });
            b.add(Leaves, Some(t), "Fronds", |p| {
                p.count = 16;
                p.start = 0.97;
                p.end = 1.0;
                p.random = 0.1;
                p.leaf_size = 3.6;
                p.leaf_size_variation = 0.15;
                p.leaf_shape = LeafShape::Frond;
                p.angle = 65.0;
                p.angle_variation = 25.0;
            });
        }
        "maple" => {
            b = B::new(
                "Maple (autumn)",
                TreeSettings { bark_color: [100, 84, 72], leaf_color: [86, 120, 44], autumn: 0.85, ..Default::default() },
            );
            let t = b.add(Trunk, None, "Trunk", |p| {
                p.length = 6.5;
                p.radius = 0.3;
                p.taper = 0.12;
                p.flare = 0.5;
                p.splits = 3;
                p.split_at = 0.48;
                p.split_angle = 26.0;
            });
            b.add(Roots, Some(t), "Roots", |p| {
                p.count = 6;
                p.length = 0.3;
                p.radius = 0.75;
            });
            let br = b.add(Branches, Some(t), "Main branches", |p| {
                p.count = 13;
                p.start = 0.35;
                p.length = 0.7;
                p.profile = Profile::Hemispherical;
                p.angle = 48.0;
                p.gravity = 0.05;
                p.wiggle = 0.35;
                p.segments = 8;
                p.radius = 0.55;
            });
            let tw = b.add(Branches, Some(br), "Twigs", |p| {
                p.count = 12;
                p.start = 0.2;
                p.length = 0.4;
                p.segments = 4;
                p.sides = 4;
            });
            b.add(Leaves, Some(tw), "Leaves", |p| {
                p.count = 14;
                p.leaf_size = 0.4;
                p.leaf_aspect = 0.9;
            });
        }
        "shrub" => {
            b = B::new("Shrub", TreeSettings { leaf_color: [60, 110, 50], ..Default::default() });
            let t = b.add(Trunk, None, "Stems", |p| {
                p.count = 7;
                p.length = 1.5;
                p.radius = 0.04;
                p.taper = 0.3;
                p.flare = 0.0;
                p.angle = 28.0;
                p.angle_variation = 12.0;
                p.gravity = 0.05;
                p.wiggle = 0.35;
                p.segments = 6;
                p.sides = 5;
            });
            let br = b.add(Branches, Some(t), "Branches", |p| {
                p.count = 9;
                p.start = 0.2;
                p.length = 0.5;
                p.profile = Profile::Spherical;
                p.angle = 45.0;
                p.segments = 4;
                p.sides = 3;
            });
            b.add(Leaves, Some(br), "Leaves", |p| {
                p.count = 16;
                p.start = 0.1;
                p.leaf_size = 0.13;
            });
        }
        "sapling" => {
            b = B::new("Sapling", TreeSettings::default());
            let t = b.add(Trunk, None, "Trunk", |p| {
                p.length = 3.2;
                p.radius = 0.05;
                p.taper = 0.2;
                p.flare = 0.2;
                p.wiggle = 0.3;
                p.segments = 10;
                p.sides = 6;
            });
            let br = b.add(Branches, Some(t), "Branches", |p| {
                p.count = 7;
                p.start = 0.4;
                p.length = 0.4;
                p.angle = 42.0;
                p.segments = 4;
                p.sides = 4;
            });
            b.add(Leaves, Some(br), "Leaves", |p| {
                p.count = 10;
                p.start = 0.2;
                p.leaf_size = 0.18;
            });
        }
        _ => return None,
    }
    b.doc.clamp_all();
    Some(b.doc)
}

// ---------------------------------------------------------------- reference

/// (field, applies to, range, meaning) — shown in the UI tooltips and over MCP.
pub const PARAM_REFERENCE: &[(&str, &str, &str, &str)] = &[
    ("count", "all", "trunk 1..24, others 0..200", "Trunk: number of trunks. Branches/leaves: how many grow on EACH parent stem."),
    ("start", "branches, leaves", "0..1", "Where along the parent children begin (0 = base, 1 = tip)."),
    ("end", "branches, leaves", "0..1", "Where along the parent children stop."),
    ("phyllotaxis", "branches, leaves", "0..360 deg", "Rotation around the parent between successive children (137.5 = golden angle)."),
    ("whorl", "branches, leaves", "1..12", "Children per node: 1 alternate, 2 opposite, 3+ whorled (like pines)."),
    ("random", "all", "0..1", "Placement jitter."),
    ("length", "all", "trunk metres 0.1..100; others 0.01..3", "Trunk: height in metres. Others: length as a fraction of the parent's length."),
    ("length_variation", "all", "0..1", "Random length variation."),
    ("profile", "branches", "conical|spherical|hemispherical|cylindrical|flame|inverse_conical", "How child length changes along the parent: shapes the crown."),
    ("angle", "all", "0..180 deg", "Angle away from the parent direction (trunk: lean from vertical)."),
    ("angle_variation", "all", "0..90 deg", "Random angle variation."),
    ("gravity", "trunk, branches, roots, leaves", "-1..1 (roots and leaves 0..1)", "Bend: positive droops downwards, negative curves upwards. Leaves: how far their weight swings them toward hanging straight down, 0 = along their angle off the stem, 1 = plumb. Roots and side roots: how steeply they head into the soil, 0 = run along the ground surface, 1 = dive straight down (they never rise out of the ground)."),
    ("wiggle", "trunk, branches", "0..1", "Random kinks along the stem."),
    ("segments", "trunk, branches", "1..64", "Length-wise resolution."),
    ("radius", "trunk, branches", "trunk metres; others 0.01..1", "Trunk: base radius in metres. Others: fraction of the parent's radius where attached."),
    ("taper", "trunk, branches", "0..1", "Tip radius as a fraction of the base radius."),
    ("flare", "trunk", "0..3", "Root flare at the base of the trunk."),
    ("sides", "trunk, branches", "3..32", "Radial resolution."),
    ("leaf_size", "leaves", "metres 0.01..10", "Leaf (or frond) length."),
    ("leaf_size_variation", "leaves", "0..1", "Random leaf size variation."),
    ("leaf_shape", "leaves", "broad|narrow|needle|round|frond|card|mesh", "Leaf outline. 'card' is a plain rectangle for leaf textures with transparency; 'mesh' uses your own OBJ model from leaf_mesh."),
    ("leaf_mesh", "leaves", "absolute path to .obj", "Custom leaf model for leaf_shape 'mesh' (leaf clusters, flowers, fruit). Normalised so +Y runs base to tip with length 1, +Z is the face; scaled by leaf size, width scales X. Under 5000 triangles."),
    ("leaf_aspect", "leaves", "0.05..2", "Leaf width relative to its length."),
    ("interval", "branches, leaves, roots", "0..20 m", "Spacing between children along the parent in metres; 0 uses count instead (spacing keeps density even on long and short parents)."),
    ("kink", "trunk, branches, roots", "0..120 deg", "One sudden bend somewhere in the middle of each stem."),
    ("jitter", "trunk, branches, roots", "0..1", "Small sharp zig-zags along the stem."),
    ("flutes", "trunk, branches, roots", "0..24", "Lengthwise grooves in the bark cross-section (0 = round)."),
    ("flute_depth", "trunk, branches, roots", "0..0.8", "Depth of the grooves as a fraction of the radius."),
    ("flatten", "trunk, branches, roots", "0..0.85", "Squash the cross-section into an ellipse."),
    ("bumps", "trunk, branches, roots", "0..0.6", "Lumpy bark displacement as a fraction of the radius."),
    ("bump_size", "trunk, branches, roots", "0.02..10 m", "Size of the bark lumps."),
    ("leaf_fold", "leaves", "0..89 deg", "Fold leaf halves up along the midrib."),
    ("leaf_bend", "leaves", "-1..1", "Curl leaf tips down (positive) or up (negative)."),
    ("leaf_face_up", "leaves", "0..1", "Turn leaves to face the sky."),
    ("break_chance", "branches", "0..1", "Chance that a branch is broken off: a short blunt stub with no children or leaves (dead wood, storm damage, pruned lower branches)."),
    ("break_length", "branches", "0.03..0.9", "Length of broken stubs relative to the full branch."),
    ("splits", "trunk, branches, roots", "0 or 2..5", "Split each stem of this generator into this many leaders (0 = no split), as in oaks and maples whose trunk divides into big limbs instead of running to the top. The leaders share the stem's wood, so each is thinner and tapers to a thin tip; a trunk's leaders curve back up. Children above the split spread over the leaders, and none grow in the crotch."),
    ("split_at", "trunk, branches, roots", "0.05..0.95", "Where along the stem it splits, as a fraction of its length."),
    ("split_angle", "trunk, branches, roots", "0..80 deg", "How far each leader leans away from the stem's direction."),
    ("weld", "branches", "0..1", "Welding: the branch base swells into a collar that blends into the parent instead of a hard joint."),
    ("curves", "all", "{key: {points: [[x,y],...]}}", "Shaping curves, see curve_reference / set_curve."),
    ("growth", "all", "{speed, speed_curve, radius_curve, start_size, start_angle, orientation_time, sync, start_pct, offset, offset_variation, shed}", "How this generator's stems or leaves grow in the growth animation; see the growth.* rows."),
    ("growth.speed", "all", "0.05..10", "Speed scale. Trunk: it reaches its full size in 1/speed of the growth. With sync auto a branch reaches its full size relative to its parent in 0.25/speed of the trunk's growth time. With the other syncs a stem takes its length relative to the trunk divided by speed (a stem as long as the trunk takes 1/speed of the trunk's time). Leaves unfold in 0.15/speed of the trunk's time."),
    ("growth.speed_curve", "all", "linear, ease_in, ease_out, s_curve", "How the size grows over the part's own growth: steady, slow start, slow end, or slow-fast-slow."),
    ("growth.radius_curve", "trunk, branches, roots", "linear, ease_in, ease_out, s_curve", "How the radius grows over the stem's own growth, against the speed curve (the length's): the same curve keeps the stem's mature proportions, so a 0.8 m sapling of a 17 m birch with a 34 cm foot is 16 mm thick; ease_in makes young stems slimmer, ease_out stouter (at most twice). A branch is never thicker than its parent where it comes out."),
    ("growth.start_size", "all", "0..1", "Share of its final size a stem or leaf has as soon as it appears (0 = grows from nothing)."),
    ("growth.start_angle", "all", "0..1", "Stems: share of the final angle to the parent at the start (below 1 they fold out of the parent). Leaves: share of the final orientation (below 1 they turn into place from lying along the twig)."),
    ("growth.orientation_time", "leaves", "0.1..5", "How long leaves take to turn into place, relative to unfolding (1 = together)."),
    ("growth.sync", "branches, roots, leaves", "auto, parent, siblings, parent_start_end, in_place", "When a part starts. auto (default): the whole tree grows at once, as real trees do: a branch's bud breaks a little after its parent's (low buds first) and the branch grows to its share of the parent's size while the parent keeps growing, so a young tree is a small tree in its mature proportions; leaves come out with the young shoots (a few with the trunk, some with the branch below their twig, most with their twig) and keep coming over the whole growth, so the crown gets denser. parent = Start % through the parent's growth; siblings = all of this generator together; parent_start_end = start and finish with the parent; in_place = when the parent has grown to its spot. Nothing starts before its parent."),
    ("growth.start_pct", "branches, roots, leaves", "0..1", "With sync parent or siblings: 0 = start as early as possible, 1 = as late."),
    ("growth.offset", "branches, roots, leaves", "0..3", "Delay added to every start, in shares of the trunk's growth time."),
    ("growth.offset_variation", "branches, roots, leaves", "0..3", "Random extra delay of up to this much per stem or leaf, so neighbours do not all start together."),
    ("growth.shed", "branches, leaves", "0..1", "Shed while young: 0 (default) keeps the stems or leaves for life; otherwise the growth point by which they have withered back into their parent and gone, with their twigs and leaves (they shrink away over the last quarter before it). For the low branches a young tree has and sheds as its bole lengthens: a pine sapling is needled to the ground, the grown pine's crown starts high. Add them as their own generator low on the trunk. For leaves: a sapling's own foliage, such as single leaves where the grown crown carries sprays of many (a spray shrunk onto a sapling shows tiny leaves). A shed generator is in the growth only: the grown tree, exports without growth, the billboard and the collision never have it."),
    ("growth.young_size", "leaves", "0.05..1", "Leaf size on a seedling, as a share of the grown size; it reaches 1 as the tree grows (default 0.45). Leaves do not shrink much on a young tree: a single leaf wants 0.8 to 0.9. A card that pictures a spray of many leaves is better kept off the young tree (a later offset, or shed single leaves in its place) than shrunk."),
    ("buried", "roots", "0..1", "How deep roots sink into the ground: 0 lies on top, 1 just under the surface."),
    ("force_response", "trunk, branches", "0..3", "How strongly the tree's forces (add_force) bend this generator; 0 ignores them."),
];

pub const SETTINGS_REFERENCE: &[(&str, &str, &str)] = &[
    ("seed", "integer", "Random seed: same seed + same parameters = same tree."),
    ("scale", "0.05..20", "Uniform scale for the whole tree."),
    ("bark_color", "[r,g,b] 0..255", "Bark colour."),
    ("leaf_color", "[r,g,b] 0..255", "Base leaf colour."),
    ("leaf_color_variation", "0..1", "Random per-leaf colour variation."),
    ("autumn", "0..1", "Blend leaves towards autumn yellows, oranges and reds."),
    ("leaf_drop", "0..1", "Season: fraction of leaves that have fallen (1 = bare winter tree). Which leaves fall is stable for a given seed."),
    ("join_method", "stitch|weld", "stitch: cut an opening in the parent and stitch it to the branch (one watertight mesh). weld: leave the parent whole and wrap the branch base onto its surface with a fillet, blending normals, colour and wind into the parent (the welding approach production tree tools use; separate pieces, lighter on LODs). Weld amount = each branch level's 'weld'."),
    ("join_branches", "true|false (roots and side roots join the same way; trunks with roots reach a little below ground so roots always grow out of bark)", "Branches are extruded from their parent's surface as one connected mesh: the parent's bark gets an opening that is stitched to the branch (no hidden geometry inside, no seam). Off = separate tubes pushed into the parent."),
    ("age", "0.05..1", "Growth stage: 1 = the mature tree as designed; lower values grow a younger version of the same individual (shorter, thinner, fewer branches; deeper levels appear last). Step it to export growth stages."),
    ("ao_strength", "0..1", "Ambient occlusion baked into vertex colours (COLOR_0): inner branches and leaves deep in the crown get darker. 0 turns it off."),
    ("wind.strength", "0..2", "Wind strength (preview and exported sway weights)."),
    ("wind.direction", "0..360 deg", "Direction the wind blows towards, around the vertical axis."),
    ("wind.gustiness", "0..1", "Irregular gusts."),
    ("wind.speed", "0.05..5", "Animation speed."),
    ("budget.enabled", "on/off", "Triangle budgets for LOD0 to LOD3. Levels over their budget get less detail (or are reported); the billboard is always 16 triangles."),
    ("budget.auto", "on/off", "On: derive the budgets from one maximum (LOD0 = maximum, LOD1 = 1/2, LOD2 = 1/4, LOD3 = 1/8). Off: set each level yourself."),
    ("budget.max", "16..5000000", "Most triangles for LOD0 when the budgets are derived from a maximum."),
    ("budget.levels", "[LOD0, LOD1, LOD2, LOD3], 0..5000000", "Most triangles per level when set per level; 0 = no limit for that level."),
    ("budget.fit", "on/off", "On: a level over its budget gets fewer sides, rings and (larger) leaves until it fits; when that is not enough, fewer leaves and the finest twigs left out. Off: levels keep their usual detail and the ones over budget are reported."),
    ("budget.leaf_area", "[LOD0, LOD1, LOD2, LOD3] m², 0 = no limit", "Most leaf card area per level (one side of each card): the overdraw cost of the foliage, which the triangle count does not show. Checked and reported with the opaque part the cut-out keeps; not fitted."),
    ("growth.export", "on/off", "Exports carry the growth animation: GLB gets morph targets with a Grow clip and one clip per stage, OBJ one file per stage, JSON the growth timing of every stem and leaf."),
    ("growth.stages", "[{name, at 0.01..1}], up to 8", "Moments of the growth (0 = nothing above ground, 1 = the mature tree), for example Sprout 0.12, Sapling 0.38, Young 0.68, Mature 1. Names are optional; unnamed stages are called Stage 1, Stage 2, ... by their place."),
    ("growth.duration", "0.5..600 s", "Seconds the whole growth animation takes."),
    ("growth.keyframes", "4..48", "Shapes the animation is sampled at (morph targets). More is smoother and makes larger files."),
];

/// (field, range, meaning) for materials — shown over MCP.
pub const MATERIAL_REFERENCE: &[(&str, &str, &str)] = &[
    ("name", "text", "Material name (used for material names in exports)."),
    ("color", "[r,g,b] 0..255", "Base colour. With a texture it tints the texture: use white to show the texture unchanged."),
    ("texture", "absolute path to PNG/JPEG or null", "Colour (albedo) image. Bark textures repeat around and along stems; leaf textures cover each leaf (use leaf_shape 'card' with alpha_cutout)."),
    ("normal_map", "absolute path to PNG/JPEG or null", "Tangent-space normal map: lights the bark in the viewport and is exported for engines."),
    ("normal_strength", "0..4 (1 = as painted)", "How strongly the normal map bends the lighting."),
    ("normal_flip_green", "true|false", "The normal map is DirectX style (green = down); default OpenGL style (green = up, as glTF expects)."),
    ("roughness", "0..1", "Surface roughness for engines."),
    ("tiling", "[u, v] 0.05..50", "How often a bark texture repeats around (u) and along (v) a stem; v counts repeats per circumference, so [n, n] keeps a square image square. A gradient layer shows its own material's tiling."),
    ("keep_proportions", "bool", "Take the along repeats from the image's shape (tiling u times width / height), so a tall bark photo is not squashed; tiling v is then unused."),
    ("alpha_cutout", "bool", "Transparent texture pixels become holes (leaf cards)."),
    ("double_sided", "bool", "Render both faces in engines. Leaf materials are always exported double-sided."),
    ("atlas", "[{rect [x, y, w, h], pivot [x, y], weight}], up to 64", "Leaf texture atlas: one rectangle per leaf picture (pixels of the texture, origin top left). Each leaf shows one picture, picked at random by weight, on a card shaped like it and hanging from its pivot (where the stalk meets the twig; default the middle of the bottom edge)."),
];

#[cfg(test)]
mod tiling_tests {
    use super::*;

    /// A tall bark photo keeps its shape with `keep_proportions`, and a gradient layer is drawn
    /// with its own tiling relative to the generator's material.
    #[test]
    fn layers_show_their_own_tiling_and_keep_their_proportions() {
        let dir = std::env::temp_dir().join(format!("arbor_tiling_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tall = dir.join("tall.png");
        image::RgbaImage::new(512, 2048).save(&tall).unwrap();
        let mut doc = Document::default();
        let grey = doc.add_material(Material { tiling: [2.0, 2.0], texture: Some(tall.display().to_string()), ..Default::default() });
        let orange = doc.add_material(Material { tiling: [1.0, 1.0], keep_proportions: true, texture: Some(tall.display().to_string()), ..Default::default() });
        let gen = doc.generators[0].id;
        doc.generators[0].material = Some(grey);
        assert_eq!(doc.material(grey).unwrap().effective_tiling(), [2.0, 2.0]);
        assert_eq!(doc.material(orange).unwrap().effective_tiling(), [1.0, 0.25]);
        assert_eq!(doc.layer_uv_scale(gen, grey), [1.0, 1.0]);
        assert_eq!(doc.layer_uv_scale(gen, orange), [0.5, 0.125]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
