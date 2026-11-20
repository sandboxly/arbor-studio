//! Triangle budgets for the mesh levels of detail (LOD0 to LOD3).
//!
//! A level over its budget is reduced further than usual: fewer sides around stems, fewer rings
//! along them and fewer (larger) leaves, on the scale from [`crate::grow::lod_blend`]. The search
//! keeps the most detailed mesh that fits. When even the least detail allowed is over budget, or
//! fitting is switched off, the level is reported as over budget.

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
    /// How far detail was reduced: 0 = the usual amount, 1 = the least detail allowed.
    pub reduction: f32,
}

impl LevelFit {
    /// One line for the interface and the agent.
    pub fn message(&self, fitting: bool) -> String {
        let Some(b) = self.budget else {
            return format!("LOD{}: {} triangles, no budget", self.lod, group(self.triangles));
        };
        if self.over && fitting {
            format!(
                "LOD{}: {} of {} triangles, over budget even at the least detail allowed. Lower the counts of the tree (leaves, branches, sides) or raise the budget.",
                self.lod,
                group(self.triangles),
                group(b)
            )
        } else if self.over {
            format!("LOD{}: {} of {} triangles, over budget by {}.", self.lod, group(self.triangles), group(b), group(self.triangles - b))
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
    let (sk0, m0) = at(doc, full, level, 0.0);
    let t0 = m0.tris.len() as u32;
    let budget = doc.budget.level(level);
    let mut fit = LevelFit { lod: level, budget, triangles: t0, usual_triangles: t0, reduced: false, over: false, reduction: 0.0 };
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
    if t1 > b {
        fit.triangles = t1;
        fit.over = true;
        fit.reduction = 1.0;
        return (sk1, m1, fit);
    }
    // Illinois false position on triangles(s) - budget; `hi` always fits.
    let target = b as f32;
    let (mut lo, mut flo) = (0.0f32, t0 as f32 - target);
    let (mut hi, mut fhi) = (1.0f32, t1 as f32 - target);
    let mut best = (1.0f32, sk1, m1);
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
