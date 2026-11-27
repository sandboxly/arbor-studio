//! The crown as a volume: where a leaf sits in the tree's leaf mass, for the light.
//!
//! A leaf card's own normal is the card's plane, so a crown lit by it is a speckle: each card
//! catches the sun or not by its random turn, and the crown has no lit side, no terminator and
//! no dark side. Production foliage bends every leaf vertex's normal toward the direction out of
//! the crown (the trick that most separates game foliage from textured quads), and darkens the
//! light that reaches the crown's inside from the sky. This module measures both from the leaves
//! themselves:
//!
//! - THE FIELD: the leaf area splatted into a grid and blurred (about 6 % of the crown's size),
//!   a smooth leaf density. Its downhill direction points out of the leaf mass, toward the
//!   nearest open air; at the scale of the blur that follows the crown's lobes (a big oak's
//!   boughs each carry their own dome), and it is 0 at a lobe's core.
//! - THE ELLIPSOID: the leaves' bounding ellipsoid, whose outward direction is defined
//!   everywhere and ignores the lobes. `crown_lobes` blends from it (0) to the field (1), and
//!   where the field is flat (a core) the ellipsoid takes over.
//! - THE DEPTH: how far inside the leaf mass a point is (a distance transform of the blurred
//!   density's inside), 0 on the outer shell to 1 at the core, never 1 in a crown thinner than
//!   `CORE_MIN` metres. Exported as its own attribute (`_CROWN_DEPTH`), never multiplied into
//!   the colours: an engine applies it to the light that comes from the sky and leaves the
//!   albedo and the direct sun alone.
//!
//! Bending is per vertex (a card's normal turns across it with the field), but which side of a
//! card faces out is decided once per leaf, so a card edge-on to the outward direction cannot
//! end up with its corners pointing to opposite sides.

use glam::Vec3;

/// The blur, as a share of the crown's largest extent.
const BLUR: f32 = 0.06;
/// Grid cells along the crown's largest extent, and the smallest cell (m).
const CELLS: f32 = 40.0;
const MIN_CELL: f32 = 0.12;
/// The inside of the leaf mass: blurred density above this share of its median at the leaves.
const INSIDE: f32 = 0.3;
/// A crown thinner than this (m from its shell to its core) never reaches depth 1.
const CORE_MIN: f32 = 2.0;
/// Depth 0 is the shallowest 5 % of the leaves, 1 the deepest 1 %.
const SHELL_PCT: f32 = 0.05;
const CORE_PCT: f32 = 0.99;

pub struct Field {
    lo: Vec3,
    cell: f32,
    n: [usize; 3],
    dens: Vec<f32>,
    /// Metres in from the outside of the leaf mass (0 outside it).
    dist: Vec<f32>,
    centre: Vec3,
    radii: Vec3,
    /// A shell's density slope: flatter, the field gives way to the ellipsoid.
    slope: f32,
    shell: f32,
    core: f32,
}

impl Field {
    /// The field of the leaf triangles `tris[leaf_tri_start..]` over `pos`. None without leaves.
    pub fn of(pos: &[Vec3], tris: &[[u32; 3]], leaf_tri_start: usize) -> Option<Field> {
        let leaf_tris = tris.get(leaf_tri_start..).unwrap_or(&[]);
        let mut splats = Vec::with_capacity(leaf_tris.len());
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for t in leaf_tris {
            let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
            let area = (b - a).cross(c - a).length() * 0.5;
            // collapsed (a leaf not yet out in a growth keyframe): no leaf there
            if !(area > 1e-8) {
                continue;
            }
            let ctr = (a + b + c) / 3.0;
            lo = lo.min(ctr);
            hi = hi.max(ctr);
            splats.push((ctr, area));
        }
        if splats.is_empty() {
            return None;
        }
        let size = (hi - lo).max_element().max(0.1);
        let cell = (size / CELLS).max(MIN_CELL);
        let sigma = (size * BLUR).max(cell);
        let pad = Vec3::splat(3.0 * sigma + 2.0 * cell);
        let glo = lo - pad;
        let dims = (((hi + pad) - glo) / cell).ceil().as_uvec3() + glam::UVec3::ONE;
        let n = [dims.x as usize, dims.y as usize, dims.z as usize];
        let mut f = Field {
            lo: glo,
            cell,
            n,
            dens: vec![0.0; n[0] * n[1] * n[2]],
            dist: Vec::new(),
            centre: (lo + hi) * 0.5,
            radii: ((hi - lo) * 0.5).max(Vec3::splat(0.3)),
            slope: 0.0,
            shell: 0.0,
            core: CORE_MIN,
        };
        // trilinear splat of the leaf area
        for &(p, area) in &splats {
            let g = (p - f.lo) / cell - Vec3::splat(0.5);
            let i0 = g.floor();
            let t = g - i0;
            for k in 0..8 {
                let o = [(k & 1) as f32, ((k >> 1) & 1) as f32, ((k >> 2) & 1) as f32];
                let w = (if o[0] > 0.0 { t.x } else { 1.0 - t.x }) * (if o[1] > 0.0 { t.y } else { 1.0 - t.y }) * (if o[2] > 0.0 { t.z } else { 1.0 - t.z });
                let (x, y, z) = (i0.x as isize + o[0] as isize, i0.y as isize + o[1] as isize, i0.z as isize + o[2] as isize);
                if let Some(i) = f.index(x, y, z) {
                    f.dens[i] += area * w;
                }
            }
        }
        f.blur(sigma / cell);
        // the inside of the leaf mass, and how far in each cell is
        let mut at: Vec<f32> = splats.iter().map(|(p, _)| f.density(*p)).collect();
        at.sort_by(f32::total_cmp);
        let thr = at[at.len() / 2] * INSIDE;
        f.distance(thr);
        // A shell's slope: the median density falling to nothing over two blur widths. A
        // flatter field is the noise of the leaves' scatter inside a lobe, not a way out of it
        // (the median slope at the leaves was tried and trusted that noise: 8 % of a ball's
        // leaves pointed inward).
        f.slope = (at[at.len() / 2] / (2.0 * sigma)).max(1e-12);
        let mut d: Vec<f32> = splats.iter().map(|(p, _)| f.dist_at(*p)).collect();
        d.sort_by(f32::total_cmp);
        let pick = |q: f32| d[((d.len() - 1) as f32 * q) as usize];
        f.shell = pick(SHELL_PCT);
        f.core = (pick(CORE_PCT) - f.shell).max(CORE_MIN);
        Some(f)
    }

    fn index(&self, x: isize, y: isize, z: isize) -> Option<usize> {
        let [nx, ny, nz] = self.n;
        (x >= 0 && y >= 0 && z >= 0 && (x as usize) < nx && (y as usize) < ny && (z as usize) < nz).then(|| (z as usize * ny + y as usize) * nx + x as usize)
    }

    /// Separable Gaussian blur of the density, `s` cells wide.
    fn blur(&mut self, s: f32) {
        let r = (3.0 * s).ceil() as isize;
        let k: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * s * s)).exp()).collect();
        let ks: f32 = k.iter().sum();
        let [nx, ny, nz] = self.n;
        let stride = [1usize, nx, nx * ny];
        let len = [nx, ny, nz];
        for axis in 0..3 {
            let src = self.dens.clone();
            for z in 0..nz {
                for y in 0..ny {
                    for x in 0..nx {
                        let c = [x, y, z];
                        let i = (z * ny + y) * nx + x;
                        let mut acc = 0.0;
                        for (j, w) in k.iter().enumerate() {
                            let q = c[axis] as isize + j as isize - r;
                            if q >= 0 && (q as usize) < len[axis] {
                                acc += src[(i as isize + (q - c[axis] as isize) * stride[axis] as isize) as usize] * w;
                            }
                        }
                        self.dens[i] = acc / ks;
                    }
                }
            }
        }
    }

    /// Metres from each inside cell (density over `thr`) to the nearest outside one: a two-pass
    /// chamfer over the 26 neighbours (1, sqrt 2, sqrt 3 cells).
    fn distance(&mut self, thr: f32) {
        let [nx, ny, nz] = self.n;
        let big = f32::MAX / 4.0;
        let mut d: Vec<f32> = self.dens.iter().map(|&v| if v > thr { big } else { 0.0 }).collect();
        let w = |dx: isize, dy: isize, dz: isize| ((dx * dx + dy * dy + dz * dz) as f32).sqrt();
        let fwd: Vec<(isize, isize, isize)> = (-1..=1).flat_map(|dz| (-1..=1).flat_map(move |dy| (-1..=1).map(move |dx| (dx, dy, dz)))).filter(|&(dx, dy, dz)| (dz, dy, dx) < (0, 0, 0)).collect();
        for pass in 0..2 {
            let zs: Vec<usize> = if pass == 0 { (0..nz).collect() } else { (0..nz).rev().collect() };
            let ys: Vec<usize> = if pass == 0 { (0..ny).collect() } else { (0..ny).rev().collect() };
            let xs: Vec<usize> = if pass == 0 { (0..nx).collect() } else { (0..nx).rev().collect() };
            let sgn = if pass == 0 { 1 } else { -1 };
            for &z in &zs {
                for &y in &ys {
                    for &x in &xs {
                        let i = (z * ny + y) * nx + x;
                        if d[i] == 0.0 {
                            continue;
                        }
                        let mut best = d[i];
                        for &(dx, dy, dz) in &fwd {
                            let (dx, dy, dz) = (dx * sgn, dy * sgn, dz * sgn);
                            match self.index(x as isize + dx, y as isize + dy, z as isize + dz) {
                                Some(j) => best = best.min(d[j] + w(dx, dy, dz)),
                                None => best = best.min(w(dx, dy, dz)),
                            }
                        }
                        d[i] = best;
                    }
                }
            }
        }
        self.dist = d.into_iter().map(|v| v * self.cell).collect();
    }

    fn sample(&self, grid: &[f32], p: Vec3) -> f32 {
        let g = (p - self.lo) / self.cell - Vec3::splat(0.5);
        let i0 = g.floor();
        let t = g - i0;
        let mut acc = 0.0;
        for k in 0..8 {
            let o = [(k & 1) as isize, ((k >> 1) & 1) as isize, ((k >> 2) & 1) as isize];
            let w = (if o[0] > 0 { t.x } else { 1.0 - t.x }) * (if o[1] > 0 { t.y } else { 1.0 - t.y }) * (if o[2] > 0 { t.z } else { 1.0 - t.z });
            if let Some(i) = self.index(i0.x as isize + o[0], i0.y as isize + o[1], i0.z as isize + o[2]) {
                acc += grid[i] * w;
            }
        }
        acc
    }

    fn density(&self, p: Vec3) -> f32 {
        self.sample(&self.dens, p)
    }

    fn dist_at(&self, p: Vec3) -> f32 {
        self.sample(&self.dist, p)
    }

    fn gradient(&self, p: Vec3) -> Vec3 {
        let h = self.cell;
        let d = |o: Vec3| self.density(p + o) - self.density(p - o);
        Vec3::new(d(Vec3::X * h), d(Vec3::Y * h), d(Vec3::Z * h)) / (2.0 * h)
    }

    /// Out of the crown at `p`: the ellipsoid's direction (`lobes` 0) to the field's (1).
    pub fn outward(&self, p: Vec3, lobes: f32) -> Vec3 {
        let q = (p - self.centre) / (self.radii * self.radii);
        let e = q.normalize_or(Vec3::Y);
        if lobes <= 0.0 {
            return e;
        }
        let g = -self.gradient(p);
        let len = g.length();
        // a lobe's core (the field is flat there) falls back to the ellipsoid
        let trust = lobes * (len / self.slope).clamp(0.0, 1.0);
        (e * (1.0 - trust) + g / len.max(1e-12) * trust).normalize_or(e)
    }

    /// 0 on the crown's outer shell .. 1 at its core.
    pub fn depth(&self, p: Vec3) -> f32 {
        ((self.dist_at(p) - self.shell) / self.core).clamp(0.0, 1.0)
    }
}

/// The crown's normals and depth of one shape of a mesh (its own positions, or a growth
/// keyframe's): `flat` the leaves' own normals, `leaf_first` where each leaf's vertices begin.
/// Returns (normals, depth), both per vertex; bark keeps its normals and gets a depth too.
pub fn shape(pos: &[Vec3], flat: &[Vec3], leaf: &[bool], leaf_first: &[u32], tris: &[[u32; 3]], leaf_tri_start: usize, amount: f32, lobes: f32) -> Option<(Vec<Vec3>, Vec<f32>)> {
    let f = Field::of(pos, tris, leaf_tri_start)?;
    let mut nrm = flat.to_vec();
    let depth: Vec<f32> = pos.iter().map(|p| f.depth(*p)).collect();
    if amount > 0.0 {
        for (li, &first) in leaf_first.iter().enumerate() {
            let first = first as usize;
            let end = leaf_first.get(li + 1).map(|&e| e as usize).unwrap_or(pos.len()).min(pos.len());
            if first >= end || !leaf[first] {
                continue;
            }
            // which side of the leaf faces out: once for the whole leaf, at its middle
            let mid = pos[first..end].iter().fold(Vec3::ZERO, |a, p| a + *p) / (end - first) as f32;
            let side = if flat[first..end].iter().fold(Vec3::ZERO, |a, n| a + *n).dot(f.outward(mid, lobes)) < 0.0 { -1.0 } else { 1.0 };
            for v in first..end {
                let out = f.outward(pos[v], lobes);
                let n = flat[v] * side;
                nrm[v] = (n * (1.0 - amount) + out * amount).normalize_or(out);
            }
        }
    }
    Some((nrm, depth))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ball of small cards: the bent normals point out of it, the depth is 0 at its rim and
    /// grows inward.
    #[test]
    fn a_ball_of_leaves() {
        let mut pos = vec![];
        let mut nrm = vec![];
        let mut tris = vec![];
        let mut first = vec![];
        let mut h = 0x9e37_79b9u32;
        let mut r = || {
            h ^= h << 13;
            h ^= h >> 17;
            h ^= h << 5;
            h as f32 / u32::MAX as f32
        };
        for _ in 0..4000 {
            let p = loop {
                let p = Vec3::new(r(), r(), r()) * 10.0 - Vec3::splat(5.0);
                if p.length() < 5.0 {
                    break p + Vec3::Y * 8.0;
                }
            };
            let n = Vec3::new(r() - 0.5, r() - 0.5, r() - 0.5).normalize_or(Vec3::X);
            let (a, b) = (n.any_orthonormal_vector() * 0.15, n.cross(n.any_orthonormal_vector()) * 0.15);
            let base = pos.len() as u32;
            first.push(base);
            pos.extend([p - a - b, p + a - b, p + a + b]);
            nrm.extend([n; 3]);
            tris.push([base, base + 1, base + 2]);
        }
        let leaf = vec![true; pos.len()];
        let (bent, depth) = shape(&pos, &nrm, &leaf, &first, &tris, 0, 1.0, 1.0).unwrap();
        let centre = Vec3::Y * 8.0;
        let (mut out, mut rim, mut core) = (0, vec![], vec![]);
        for (i, p) in pos.iter().enumerate() {
            if bent[i].dot(*p - centre) > 0.0 {
                out += 1;
            }
            let rr = (*p - centre).length();
            if rr > 4.6 {
                rim.push(depth[i]);
            } else if rr < 1.5 {
                core.push(depth[i]);
            }
        }
        assert!(out as f32 > pos.len() as f32 * 0.97, "{out} of {}", pos.len());
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        assert!(mean(&rim) < 0.15 && mean(&core) > 0.6, "rim {} core {}", mean(&rim), mean(&core));
        // amount 0 keeps the leaves' own normals
        let (same, _) = shape(&pos, &nrm, &leaf, &first, &tris, 0, 0.0, 1.0).unwrap();
        assert_eq!(same, nrm);
    }
}
