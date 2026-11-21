//! Triangle budgets for the mesh levels of detail (LOD0 to LOD3).
//!
//! A level over its budget is reduced further than usual, on the scale from
//! [`crate::grow::lod_blend`]: first fewer sides around stems, fewer rings along them and fewer
//! (larger) leaves (reduction 0 to 1), then fewer leaves still and the finest twigs left out,
//! smallest first (1 to 2). The search keeps the most detailed mesh that fits. When even the
//! least detail allowed is over budget, or fitting is switched off, the level is reported as over
//! budget.

use crate::grow::{build_mesh_with, grow, lod, lod_blend, reduce_detail, Mesh, Skeleton};
use crate::model::Document;
use serde::Serialize;

/// How one level of detail relates to its budget.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LevelFit {
    pub lod: u32,
    /// The level's budget (None: budgets are off or this level has none).
    pub budget: Option<u32>,
    pub triangles: u32,
    /// Triangles at the level's usual detail, before any fitting.
    pub usual_triangles: u32,
    /// Detail was reduced below the usual amount to fit the budget.
    pub reduced: bool,
    /// The level is over its budget.
    pub over: bool,
    /// How far detail was reduced: 0 = the usual amount, 1 = the least stem detail (sides,
    /// rings), 2 = also only a few leaves left and most of the finest twigs left out.
    pub reduction: f32,
    /// Leaf card area of the level, and the part the alpha cut-out keeps.
    pub leaf_area: crate::leafarea::LeafArea,
    /// The level's leaf card area budget in m² (None: no limit).
    pub area_budget: Option<f32>,
    /// The leaf card area is over its budget.
    pub area_over: bool,
}

impl LevelFit {
    /// The leaf card area of the level in one line (m², and against its budget when it has one).
    pub fn area_message(&self) -> String {
        let a = &self.leaf_area;
        let mut s = format!("LOD{}: {:.1} m² of leaf cards", self.lod, a.card_m2);
        if let Some(o) = a.opaque_m2 {
            s += &format!(" ({o:.1} m² opaque after the cut-out)");
        }
        match self.area_budget {
            Some(b) if self.area_over => s += &format!(", over the {b:.1} m² budget."),
            Some(b) => s += &format!(", within the {b:.1} m² budget."),
            None => s += ".",
        }
        s
    }

    /// One line for the interface and the agent.
    pub fn message(&self, fitting: bool) -> String {
        let Some(b) = self.budget else {
            return format!("LOD{}: {} triangles, no budget", self.lod, group(self.triangles));
        };
        if self.over && fitting {
            format!(
                "LOD{}: {} of {} triangles, over budget even at the least detail allowed (fewest sides and rings, most leaves and the finest twigs left out). Lower the counts of the tree (branches, sides) or raise the budget.",
                self.lod,
                group(self.triangles),
                group(b)
            )
        } else if self.over {
            format!("LOD{}: {} of {} triangles, over budget by {}.", self.lod, group(self.triangles), group(b), group(self.triangles - b))
        } else if self.reduced && self.reduction > 1.0 {
            format!("LOD{}: {} of {} triangles, reduced from {} to fit (fewer leaves, finest twigs left out).", self.lod, group(self.triangles), group(b), group(self.usual_triangles))
        } else if self.reduced {
            format!("LOD{}: {} of {} triangles, reduced from {} to fit.", self.lod, group(self.triangles), group(b), group(self.usual_triangles))
        } else {
            format!("LOD{}: {} of {} triangles.", self.lod, group(self.triangles), group(b))
        }
    }
}

/// 120000 -> "120,000".
pub fn group(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The skeleton and mesh of one level at reduction `s` (0 = usual detail).
fn at(doc: &Document, full: &Skeleton, level: u32, s: f32) -> (Skeleton, Mesh) {
    let lodf = if s <= 0.0 { lod(level) } else { lod_blend(level, s) };
    let mut sk = full.clone();
    if lodf != lod(0) {
        reduce_detail(doc, &mut sk, &lodf);
    }
    let m = build_mesh_with(doc, &sk, &lodf, level);
    (sk, m)
}

/// One mesh level of detail (0..=3) of the tree grown as `full`, fitted to its budget.
pub fn fit_level(doc: &Document, full: &Skeleton, level: u32) -> (Skeleton, Mesh, LevelFit) {
    let (sk, m, mut fit) = fit_triangles(doc, full, level);
    fit.leaf_area = crate::leafarea::measure(doc, &m);
    fit.area_budget = doc.budget.area_level(level);
    fit.area_over = fit.area_budget.is_some_and(|b| fit.leaf_area.card_m2 > b);
    (sk, m, fit)
}

fn fit_triangles(doc: &Document, full: &Skeleton, level: u32) -> (Skeleton, Mesh, LevelFit) {
    let (sk0, m0) = at(doc, full, level, 0.0);
    let t0 = m0.tris.len() as u32;
    let budget = doc.budget.level(level);
    let mut fit = LevelFit {
        lod: level,
        budget,
        triangles: t0,
        usual_triangles: t0,
        reduced: false,
        over: false,
        reduction: 0.0,
        leaf_area: Default::default(),
        area_budget: None,
        area_over: false,
    };
    let Some(b) = budget else { return (sk0, m0, fit) };
    if t0 <= b {
        return (sk0, m0, fit);
    }
    if !doc.budget.fit {
        fit.over = true;
        return (sk0, m0, fit);
    }
    let (sk1, m1) = at(doc, full, level, 1.0);
    let t1 = m1.tris.len() as u32;
    fit.reduced = true;
    let target = b as f32;
    // the stage that reaches the budget: stem detail (0..1), or leaves and twigs (1..2)
    let (lo0, flo0, hi0, sk_hi, m_hi) = if t1 <= b {
        (0.0f32, t0 as f32 - target, 1.0f32, sk1, m1)
    } else {
        let (sk2, m2) = at(doc, full, level, 2.0);
        let t2 = m2.tris.len() as u32;
        if t2 > b {
            fit.triangles = t2;
            fit.over = true;
            fit.reduction = 2.0;
            return (sk2, m2, fit);
        }
        (1.0f32, t1 as f32 - target, 2.0f32, sk2, m2)
    };
    // Illinois false position on triangles(s) - budget; `hi` always fits.
    let fhi0 = m_hi.tris.len() as f32 - target;
    let (mut lo, mut flo) = (lo0, flo0);
    let (mut hi, mut fhi) = (hi0, fhi0);
    let mut best = (hi0, sk_hi, m_hi);
    let mut last = 0i8;
    for _ in 0..8 {
        let span = hi - lo;
        let s = (hi - fhi * span / (fhi - flo)).clamp(lo + 0.02 * span, hi - 0.02 * span);
        let (sk, m) = at(doc, full, level, s);
        let f = m.tris.len() as f32 - target;
        if f <= 0.0 {
            hi = s;
            fhi = f;
            let close = m.tris.len() as f32 >= 0.97 * target;
            best = (s, sk, m);
            if close {
                break;
            }
            if last == -1 {
                flo *= 0.5;
            }
            last = -1;
        } else {
            lo = s;
            flo = f;
            if last == 1 {
                fhi *= 0.5;
            }
            last = 1;
        }
        if hi - lo < 0.004 {
            break;
        }
    }
    let (s, sk, m) = best;
    fit.triangles = m.tris.len() as u32;
    fit.reduction = (s * 1000.0).round() / 1000.0;
    (sk, m, fit)
}

/// One mesh level of detail (0..=3), fitted to its budget.
pub fn level_mesh(doc: &Document, level: u32) -> (Skeleton, Mesh, LevelFit) {
    let full = grow(doc, 0);
    fit_level(doc, &full, level)
}

/// LOD0 to LOD3 against their budgets (the tree is grown once).
pub fn report(doc: &Document) -> Vec<LevelFit> {
    let full = grow(doc, 0);
    (0..4).map(|l| fit_level(doc, &full, l).2).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::preset;

    #[test]
    fn budgets_are_met_or_reported() {
        let mut doc = preset("Oak").unwrap();
        let usual = report(&doc);
        assert!(usual.iter().all(|f| f.budget.is_none() && !f.over));
        doc.budget.enabled = true;
        doc.budget.auto = true;
        doc.budget.max = 12_000;
        let r = report(&doc);
        for f in &r {
            let b = f.budget.unwrap();
            assert!(f.over || f.triangles <= b, "{f:?}");
            assert!(f.triangles <= f.usual_triangles, "{f:?}");
        }
        assert!(r[0].reduced && r[0].triangles <= 12_000 && r[0].triangles > 10_000, "{:?}", r[0]);
        // an impossible budget is reported, not silently ignored
        doc.budget.max = 100;
        assert!(report(&doc).iter().all(|f| f.over));
        // without fitting, levels keep their usual detail and are only reported
        doc.budget.max = 12_000;
        doc.budget.fit = false;
        let r = report(&doc);
        assert!(r[0].over && r[0].triangles == r[0].usual_triangles);
    }

    #[test]
    fn grouping() {
        assert_eq!(group(119996), "119,996");
        assert_eq!(group(16), "16");
        assert_eq!(group(1_000_000), "1,000,000");
    }
}
