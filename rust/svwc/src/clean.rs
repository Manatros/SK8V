//! Triangle-soup clean-up shared by the offline bake (`skatev-world-cache`)
//! and the live collision reader (`skatev-runtime::live`): both must leave the
//! same collision behind, so the rules live here once.
use crate::Tri;
use rustc_hash::FxHashMap as HashMap;

/// Soft vegetation GTA authors as volumes or foliage meshes: bush and grass
/// clumps (GRASS_LONG, GRASS, GRASS_SHORT, BUSHES, TWIGS, LEAVES; HAY bales
/// and tree bark stay solid). Peds push through them.
pub fn soft_vegetation(material: u8) -> bool {
    matches!(material, 46..=48 | 50..=52)
}

/// A BUSHES / TWIGS / LEAVES piece GTA's peds cannot pass, by its placed
/// bounds: a hedge (at least 1 m tall and 2.5 m long) or a tree (at least
/// 3 m tall). Smaller clumps are bushes and stay passable.
pub fn hedge(material: u8, lo: [f32; 3], hi: [f32; 3]) -> bool {
    let (h, long) = (hi[2] - lo[2], (hi[0] - lo[0]).max(hi[1] - lo[1]));
    matches!(material, 50..=52) && (h >= 3.0 || (h >= 1.0 && long >= 2.5))
}

/// Removes repeated triangles: every corner within 1 cm of another triangle's
/// (same winding, any rotation of the corners), as GTA places some trees and
/// props twice a few millimetres apart. Reversed copies (two-sided surfaces)
/// stay. Returns how many were removed.
pub fn drop_repeats(tris: &mut Vec<Tri>) -> usize {
    const TOL: f32 = 0.01;
    let centroid = |t: &Tri| -> [f32; 3] { std::array::from_fn(|k| (t.v[0][k] + t.v[1][k] + t.v[2][k]) / 3.0) };
    let same = |a: &Tri, b: &Tri| {
        (0..3).any(|r| (0..3).all(|i| (0..3).all(|k| (a.v[i][k] - b.v[(i + r) % 3][k]).abs() < TOL)))
    };
    let mut blocks: HashMap<(i32, i32), Vec<u32>> = HashMap::default();
    for (i, t) in tris.iter().enumerate() {
        let c = centroid(t);
        if c.iter().all(|x| x.is_finite()) {
            blocks.entry(((c[0] / 64.0).floor() as i32, (c[1] / 64.0).floor() as i32)).or_default().push(i as u32);
        }
    }
    let mut keep = vec![true; tris.len()];
    for ids in blocks.values() {
        // Centroids within TOL of each other: the same 2*TOL cell, or the
        // neighbour on the side the centroid is nearer to (8 cells).
        let mut cells: HashMap<[i32; 3], Vec<u32>> = HashMap::default();
        for &i in ids {
            let c = centroid(&tris[i as usize]).map(|x| x / (2.0 * TOL));
            let base = c.map(|x| x.floor() as i32);
            let side = std::array::from_fn::<i32, 3, _>(|k| if c[k] - c[k].floor() < 0.5 { -1 } else { 1 });
            let repeat = (0..8).any(|m| {
                let cell = std::array::from_fn(|k| base[k] + if m >> k & 1 == 1 { side[k] } else { 0 });
                cells.get(&cell).is_some_and(|v| v.iter().any(|&j| same(&tris[i as usize], &tris[j as usize])))
            });
            if repeat {
                keep[i as usize] = false;
            } else {
                cells.entry(base).or_default().push(i);
            }
        }
    }
    let before = tris.len();
    let mut k = 0;
    tris.retain(|_| {
        k += 1;
        keep[k - 1]
    });
    before - tris.len()
}

/// Upward unit normal of a triangle ([0, 0, 0] for a degenerate one).
fn unit_normal(t: &Tri) -> [f32; 3] {
    let [a, b, c] = t.v;
    let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
    let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if l > 1e-12 { [n[0] / l, n[1] / l, n[2] / l] } else { [0.0, 0.0, 0.0] }
}

/// Removes double-stacked coplanar floor layers (Manatros 2026-10-22: "when
/// geometry is layered in the same coordinates just combine them if you can so
/// it isnt double stacked anywhere, like if 2 terrain textures lay right ontop
/// of each other they dont get seen as different surfaces but as one").
/// An upward triangle whose footprint is covered by another upward surface
/// within 4 cm with aligned normals (>= 0.95 dot) is redundant: the lower
/// or duplicate face is removed. Returns how many were removed.
pub fn drop_coplanar_layers(tris: &mut Vec<Tri>) -> usize {
    const LAYER_TOL: f32 = 0.04;
    const MIN_DOT: f32 = 0.95;
    let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::default();
    for (i, t) in tris.iter().enumerate() {
        if !up(t) || !t.v.iter().flatten().all(|x| x.is_finite()) {
            continue;
        }
        let span_x = t.v.iter().map(|p| p[0]).fold(f32::MIN, f32::max) - t.v.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
        let span_y = t.v.iter().map(|p| p[1]).fold(f32::MIN, f32::max) - t.v.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        if span_x > 2000.0 || span_y > 2000.0 {
            continue;
        }
        for cell in cells(t) {
            grid.entry(cell).or_default().push(i as u32);
        }
    }
    if grid.is_empty() {
        return 0;
    }
    let normals: Vec<[f32; 3]> = tris.iter().map(unit_normal).collect();
    let mut keep = vec![true; tris.len()];
    for (i, t) in tris.iter().enumerate() {
        if !up(t) || !t.v.iter().flatten().all(|x| x.is_finite()) {
            continue;
        }
        let ni = normals[i];
        let covered_point = |p: [f32; 3]| {
            let cell = ((p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32);
            let Some(candidates) = grid.get(&cell) else { return false };
            candidates.iter().any(|&c| {
                let j = c as usize;
                if j == i { return false; }
                let nj = normals[j];
                let dot = ni[0] * nj[0] + ni[1] * nj[1] + ni[2] * nj[2];
                if dot < MIN_DOT { return false; }
                let Some(zj) = floor_at(&tris[j], [p[0], p[1]]) else { return false };
                let diff = zj - p[2];
                if diff > 1e-4 && diff <= LAYER_TOL {
                    true
                } else if diff.abs() <= 1e-4 && j < i {
                    true
                } else {
                    false
                }
            })
        };
        let fp = footprint(t);
        let covered = fp.iter().all(|&p| covered_point(p));
        if covered {
            keep[i] = false;
        }
    }
    let before = tris.len();
    let mut k = 0;
    tris.retain(|_| {
        k += 1;
        keep[k - 1]
    });
    before - tris.len()
}

/// Removes faces nobody can touch (design 2026-10-26-one-source-collision 2,
/// Manatros 2026-10-26: "so many faces that lie on top of each other"): a
/// triangle whose every sample has another triangle facing the same way
/// (normals within 18 deg) in front of it, 5 mm to `HIDDEN_GAP` away along
/// its own normal. Neither the board nor the skater fits between them.
/// Only triangles `droppable` says may go; only those `cover` says may cover
/// (switchable variants do neither). Triangles longer than 6 m are left
/// alone: their samples would be too far apart to prove the cover has no
/// hole. Returns how many were removed.
pub fn drop_hidden_layers(tris: &mut Vec<Tri>, droppable: impl Fn(&Tri) -> bool, cover: impl Fn(&Tri) -> bool) -> usize {
    const HIDDEN_GAP: f32 = 0.30;
    const MIN_GAP: f32 = 0.005;
    const MIN_DOT: f32 = 0.95;
    const STEP: f32 = 0.10;
    const LONGEST: f32 = 6.0;
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let finite = |t: &Tri| t.v.iter().flatten().all(|x| x.is_finite());
    let normals: Vec<[f32; 3]> = tris.iter().map(unit_normal).collect();
    let usable: Vec<bool> = tris.iter().enumerate().map(|(i, t)| finite(t) && normals[i] != [0.0; 3]).collect();
    let covers: Vec<bool> = tris.iter().enumerate().map(|(i, t)| usable[i] && cover(t)).collect();
    let skip: Vec<bool> = tris.iter().enumerate().map(|(i, t)| !usable[i] || !droppable(t)).collect();
    // Covers by 2 m cell over their box, widened by the gap.
    let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::default();
    for (i, t) in tris.iter().enumerate() {
        if !covers[i] { continue; }
        let lo = |k: usize| t.v.iter().map(|p| p[k]).fold(f32::MAX, f32::min) - HIDDEN_GAP;
        let hi = |k: usize| t.v.iter().map(|p| p[k]).fold(f32::MIN, f32::max) + HIDDEN_GAP;
        if hi(0) - lo(0) > 2000.0 || hi(1) - lo(1) > 2000.0 { continue; }
        for x in (lo(0) / CELL).floor() as i32..=(hi(0) / CELL).floor() as i32 {
            for y in (lo(1) / CELL).floor() as i32..=(hi(1) / CELL).floor() as i32 {
                grid.entry((x, y)).or_default().push(i as u32);
            }
        }
    }
    // Is the point p of triangle i covered by a triangle allowed by `cover`?
    let covered = |i: usize, p: [f32; 3], cover: &dyn Fn(usize) -> bool| {
        let ni = normals[i];
        let Some(list) = grid.get(&((p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32)) else { return false };
        list.iter().any(|&c| {
            let j = c as usize;
            if j == i || !cover(j) { return false; }
            let nj = normals[j];
            let along = dot(ni, nj);
            if along < MIN_DOT { return false; }
            let [a, b, cc] = tris[j].v;
            let d = dot(sub(a, p), nj) / along;
            if !(MIN_GAP..=HIDDEN_GAP).contains(&d) { return false; }
            let q = [p[0] + ni[0] * d, p[1] + ni[1] * d, p[2] + ni[2] * d];
            let edge = |u: [f32; 3], v: [f32; 3]| dot(cross(sub(v, u), sub(q, u)), nj) >= -1e-5;
            edge(a, b) && edge(b, cc) && edge(cc, a)
        })
    };
    // Centroid first (most faces are not covered), then corners pulled 2 %
    // in, edges and the inside every STEP; `all` stops at the first sample
    // that fails. Lazily: building every triangle's samples up front held
    // hundreds of thousands of allocations per tile context and starved the
    // whole-map bake (12 threads at 15 % CPU, 2026-10-26).
    let all_samples = |t: &Tri, mut ok: &mut dyn FnMut([f32; 3]) -> bool| -> bool {
        let [a, b, c] = t.v;
        let len = |u: [f32; 3], v: [f32; 3]| dot(sub(u, v), sub(u, v)).sqrt();
        let longest = len(a, b).max(len(b, c)).max(len(c, a));
        if longest > LONGEST { return false; }
        let m = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0, (a[2] + b[2] + c[2]) / 3.0];
        let lerp = |u: [f32; 3], v: [f32; 3], s: f32| [u[0] + (v[0] - u[0]) * s, u[1] + (v[1] - u[1]) * s, u[2] + (v[2] - u[2]) * s];
        if !ok(m) { return false; }
        for p in [a, b, c] { if !ok(lerp(p, m, 0.02)) { return false; } }
        let n = ((longest / STEP).ceil() as usize).max(1);
        for i in 0..=n {
            for j in 0..=n - i {
                let p = [a[0] + (b[0] - a[0]) * i as f32 / n as f32 + (c[0] - a[0]) * j as f32 / n as f32,
                         a[1] + (b[1] - a[1]) * i as f32 / n as f32 + (c[1] - a[1]) * j as f32 / n as f32,
                         a[2] + (b[2] - a[2]) * i as f32 / n as f32 + (c[2] - a[2]) * j as f32 / n as f32];
                if !ok(lerp(p, m, 0.01)) { return false; }
            }
        }
        let _ = &mut ok;
        true
    };
    // Pass 1 with every face as cover; pass 2 only with faces that stay.
    let mut hidden = vec![false; tris.len()];
    for i in 0..tris.len() {
        if skip[i] { continue; }
        hidden[i] = all_samples(&tris[i], &mut |p| covered(i, p, &|_| true));
    }
    let first = hidden.clone();
    for i in 0..tris.len() {
        if !first[i] { continue; }
        hidden[i] = all_samples(&tris[i], &mut |p| covered(i, p, &|j| !first[j]));
    }
    let before = tris.len();
    let mut k = 0;
    tris.retain(|_| {
        k += 1;
        !hidden[k - 1]
    });
    before - tris.len()
}

const CELL: f32 = 2.0;
/// Solid floor at most this far under a foliage triangle's centroid: bushes
/// and hedges (Rockford Hills hedges stand 3-4 m) reach higher than grass,
/// and grass a few metres over a floor is more likely terrain over a tunnel.
fn depth(material: u8) -> f32 {
    if material >= 50 { 1.5 } else { 0.8 }
}
/// Floors closer than this are the same surface (coplanar layers).
const EPS: f32 = 0.02;

fn up(t: &Tri) -> bool {
    let [a, b, c] = t.v;
    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    len > 1e-9 && n[2] / len >= 0.5
}

fn cells(t: &Tri) -> impl Iterator<Item = (i32, i32)> {
    let lo = |k: usize| t.v.iter().map(|p| p[k]).fold(f32::MAX, f32::min);
    let hi = |k: usize| t.v.iter().map(|p| p[k]).fold(f32::MIN, f32::max);
    let (x0, x1) = ((lo(0) / CELL).floor() as i32, (hi(0) / CELL).floor() as i32);
    let (y0, y1) = ((lo(1) / CELL).floor() as i32, (hi(1) / CELL).floor() as i32);
    (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
}

/// Height of upward-facing triangle `t` at XY `p`, if `p` lies over it.
pub fn floor_at(t: &Tri, p: [f32; 2]) -> Option<f32> {
    if up(t) { height(t, p) } else { None }
}

fn height(t: &Tri, p: [f32; 2]) -> Option<f32> {
    let [a, b, c] = t.v;
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if d.abs() < 1e-12 {
        return None;
    }
    let l0 = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let l1 = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    let l2 = 1.0 - l0 - l1;
    (l0 >= -1e-4 && l1 >= -1e-4 && l2 >= -1e-4).then(|| l0 * a[2] + l1 * b[2] + l2 * c[2])
}

/// Squared distance from `p` to triangle `t` (Ericson, closest point).
fn dist2(p: [f32; 3], t: &Tri) -> f32 {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let along = |a: [f32; 3], d: [f32; 3], s: f32| [a[0] + d[0] * s, a[1] + d[1] * s, a[2] + d[2] * s];
    let [a, b, c] = t.v;
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    let (va, vb, vc) = (d3 * d6 - d5 * d4, d5 * d2 - d1 * d6, d1 * d4 - d3 * d2);
    let q = if d1 <= 0.0 && d2 <= 0.0 {
        a
    } else if d3 >= 0.0 && d4 <= d3 {
        b
    } else if d6 >= 0.0 && d5 <= d6 {
        c
    } else if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        along(a, ab, d1 / (d1 - d3))
    } else if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        along(a, ac, d2 / (d2 - d6))
    } else if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        along(b, sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6)))
    } else {
        let den = 1.0 / (va + vb + vc);
        along(along(a, ab, vb * den), ac, vc * den)
    };
    let d = sub(p, q);
    dot(d, d)
}

/// Where gaps are: no walkable triangle within this of a point.
const GAP: f32 = 1.0;
/// A surface within this of a point is the same surface (not a gap).
const TOUCH: f32 = 0.25;

/// Fills the walkable collision's gaps from `extra` (GTA's bullet/camera copy,
/// `bounds::solid_to_sight`): an extra triangle is added when 3 of its 4
/// points (centroid and the half-way points to the corners) have no triangle
/// of `tris` within `GAP`. Where the ped collision already has a surface
/// (ramps over stairs, the coarse version of detailed ground) the copy stays
/// out; surfaces it lacks (owner 2026-10-07: a rooftop wall's top) come in.
/// Returns how many were added.
pub fn fill_gaps(tris: &mut Vec<Tri>, extra: Vec<Tri>) -> usize {
    let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::default();
    for (i, t) in tris.iter().enumerate() {
        if !t.v.iter().flatten().all(|x| x.is_finite()) {
            continue;
        }
        let span = t.v.iter().map(|p| p[0]).fold(f32::MIN, f32::max) - t.v.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
        let span_y = t.v.iter().map(|p| p[1]).fold(f32::MIN, f32::max) - t.v.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        if span > 2000.0 || span_y > 2000.0 {
            continue;
        }
        for cell in cells(t) {
            grid.entry(cell).or_default().push(i as u32);
        }
    }
    let covered = |p: [f32; 3]| {
        let (x0, x1) = (((p[0] - GAP) / CELL).floor() as i32, ((p[0] + GAP) / CELL).floor() as i32);
        let (y0, y1) = (((p[1] - GAP) / CELL).floor() as i32, ((p[1] + GAP) / CELL).floor() as i32);
        (x0..=x1).flat_map(|x| (y0..=y1).map(move |y| (x, y)))
            .filter_map(|c| grid.get(&c)).flatten()
            .any(|&i| dist2(p, &tris[i as usize]) <= GAP * GAP)
    };
    let half = |a: [f32; 3], b: [f32; 3]| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (a[2] + b[2]) / 2.0];
    let gaps: Vec<Tri> = extra
        .into_iter()
        .filter(|t| t.v.iter().flatten().all(|x| x.is_finite()))
        .filter(|t| !crate::bounds::is_water(t.material))
        .filter(|t| {
            let c = centroid(t);
            [c, half(c, t.v[0]), half(c, t.v[1]), half(c, t.v[2])].iter().filter(|p| !covered(**p)).count() >= 3
        })
        .collect();
    let n = gaps.len();
    tris.extend(gaps);
    n
}

/// GTAxSkate: floors from the prebaked map (`extra`, GTA's own ped collision
/// read offline) where the live collision has a hole, after SK8V's
/// `fill_gaps`. As `fill_gaps`, but only upward walkable floors (GTA Z
/// normal >= 0.7) come in, and a nearby wall or underside does not cover a
/// hole in the floor (2026-10-18: falls through the map next to walls).
/// Returns (added floor triangles, rejected steep/non-floor triangles).
pub fn fill_floor_gaps(tris: &mut Vec<Tri>, extra: Vec<Tri>) -> (usize, usize) {
    let mut dropped_steep = 0;
    let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::default();
    for (i, t) in tris.iter().enumerate() {
        if !t.v.iter().flatten().all(|x| x.is_finite()) {
            continue;
        }
        let span = t.v.iter().map(|p| p[0]).fold(f32::MIN, f32::max) - t.v.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
        let span_y = t.v.iter().map(|p| p[1]).fold(f32::MIN, f32::max) - t.v.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        if span > 2000.0 || span_y > 2000.0 {
            continue;
        }
        for cell in cells(t) {
            grid.entry(cell).or_default().push(i as u32);
        }
    }
    let covered = |p: [f32; 3]| {
        let (x0, x1) = (((p[0] - GAP) / CELL).floor() as i32, ((p[0] + GAP) / CELL).floor() as i32);
        let (y0, y1) = (((p[1] - GAP) / CELL).floor() as i32, ((p[1] + GAP) / CELL).floor() as i32);
        (x0..=x1).flat_map(|x| (y0..=y1).map(move |y| (x, y)))
            .filter_map(|c| grid.get(&c)).flatten()
            .any(|&i| {
                // Covered by a floor right under or over the point (within
                // GAP), or by any surface it practically lies on. A nearby
                // wall or underside does not cover a hole in the floor
                // (2026-10-18: falls through the map next to walls).
                let t = &tris[i as usize];
                floor_at(t, [p[0], p[1]]).is_some_and(|z| (z - p[2]).abs() <= GAP) || dist2(p, t) <= TOUCH * TOUCH
            })
    };
    let half = |a: [f32; 3], b: [f32; 3]| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (a[2] + b[2]) / 2.0];
    let gaps: Vec<Tri> = extra
        .into_iter()
        .filter(|t| t.v.iter().flatten().all(|x| x.is_finite()))
        .filter(|t| !crate::bounds::is_water(t.material))
        .filter(|t| {
            let floor = normal_z(t) >= 0.7;
            dropped_steep += usize::from(!floor);
            floor
        })
        .filter(|t| {
            let c = centroid(t);
            [c, half(c, t.v[0]), half(c, t.v[1]), half(c, t.v[2])].iter().filter(|p| !covered(**p)).count() >= 3
        })
        .collect();
    let n = gaps.len();
    tris.extend(gaps);
    (n, dropped_steep)
}

fn centroid(t: &Tri) -> [f32; 3] {
    std::array::from_fn(|i| (t.v[0][i] + t.v[1][i] + t.v[2][i]) / 3.0)
}

/// The centroid and the half-way points from it to the corners.
fn footprint(t: &Tri) -> [[f32; 3]; 4] {
    let c = centroid(t);
    let half = |v: [f32; 3]| std::array::from_fn(|i| (c[i] + v[i]) / 2.0);
    [c, half(t.v[0]), half(t.v[1]), half(t.v[2])]
}

/// The footprint and the corners pulled 5 cm in (a corner on a shared edge
/// would otherwise test the neighbour's side).
fn footprint_and_corners(t: &Tri) -> [[f32; 3]; 7] {
    let c = centroid(t);
    let f = footprint(t);
    let corner = |v: [f32; 3]| {
        let d = ((c[0] - v[0]).powi(2) + (c[1] - v[1]).powi(2)).sqrt().max(1e-6);
        let k = (0.05 / d).min(0.5);
        std::array::from_fn(|i| v[i] + (c[i] - v[i]) * k)
    };
    [f[0], f[1], f[2], f[3], corner(t.v[0]), corner(t.v[1]), corner(t.v[2])]
}

fn cell_of(p: [f32; 3]) -> (i32, i32) {
    ((p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32)
}

/// Foliage meshes standing on ground (hedges and bush blobs GTA models as
/// triangle meshes with a vegetation material): removes vegetation triangles
/// with a solid upward floor under their centroid within `depth`. A
/// vegetation triangle with nothing solid under it *is* the ground (grassy
/// terrain, shrub-covered hillsides) and stays, as do hedges and trees
/// (`hedge`) and soil caps (`soil`), over each connected foliage mesh.
/// Returns how many were removed.
pub fn drop_over_ground(tris: &mut Vec<Tri>) -> usize {
    let solid = hedges(tris);
    // Only cells under foliage need floors.
    let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::default();
    for t in tris.iter().filter(|t| soft_vegetation(t.material)) {
        for p in footprint_and_corners(t) {
            grid.entry(cell_of(p)).or_default();
        }
    }
    if grid.is_empty() {
        return 0;
    }
    for (i, t) in tris.iter().enumerate() {
        if soft_vegetation(t.material) || !up(t) || !t.v.iter().flatten().all(|x| x.is_finite()) {
            continue;
        }
        // Skip absurdly large triangles' cell sweep (sea floor, map bounds).
        let span = t.v.iter().map(|p| p[0]).fold(f32::MIN, f32::max) - t.v.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
        let span_y = t.v.iter().map(|p| p[1]).fold(f32::MIN, f32::max) - t.v.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        if span > 2000.0 || span_y > 2000.0 {
            continue;
        }
        for cell in cells(t) {
            if let Some(list) = grid.get_mut(&cell) {
                list.push(i as u32);
            }
        }
    }
    let over: Vec<bool> = tris
        .iter()
        .zip(&solid)
        .map(|(t, &solid)| {
            if !soft_vegetation(t.material) || solid {
                return false;
            }
            // Solid floor under its whole footprint (centroid and the half-way
            // points to the corners), not the centroid alone: rc7 fell through
            // grass ground at (-181, 439) whose centroid lay over a wall top
            // while the rest of it was the only ground there. The corners
            // too (2026-10-25 mark 1, the same hillside): a wall's footing
            // 1 m under the lawn's inner half took the whole lawn triangle
            // and left its outer half a hole.
            // A corner may rest on the floor (a hedge's side standing on it).
            footprint_and_corners(t).iter().enumerate().all(|(k, p)| {
                let top = if k >= 4 { p[2] + EPS } else { p[2] - EPS };
                grid[&cell_of(*p)].iter().any(|&i| {
                    height(&tris[i as usize], [p[0], p[1]]).is_some_and(|z| z < top && z >= p[2] - depth(t.material))
                })
            })
        })
        .collect();
    let before = tris.len();
    let mut k = 0;
    tris.retain(|_| {
        k += 1;
        !over[k - 1]
    });
    before - tris.len()
}

/// Per triangle: part of a hedge or tree mesh (`hedge` over the bounds of
/// its connected foliage triangles; corners joined by identical bits).
/// Grass materials: a grass mesh of upward faces only, 2 m across, is a soil
/// cap (a planter's or raised bed's dirt, owner 2026-10-07), not a clump.
fn soil(material: u8) -> bool {
    matches!(material, 46..=48)
}

fn hedges(tris: &[Tri]) -> Vec<bool> {
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let mut parent: Vec<usize> = (0..tris.len()).collect();
    let mut corners: HashMap<[u32; 3], usize> = HashMap::default();
    for (i, t) in tris.iter().enumerate().filter(|(_, t)| soft_vegetation(t.material)) {
        for v in t.v {
            let j = *corners.entry(v.map(f32::to_bits)).or_insert(i);
            let (a, b) = (root(&mut parent, i), root(&mut parent, j));
            parent[a] = b;
        }
    }
    // Per mesh: bounds, and whether every triangle faces up.
    let mut bounds: HashMap<usize, ([f32; 3], [f32; 3], bool)> = HashMap::default();
    for (i, t) in tris.iter().enumerate().filter(|(_, t)| soft_vegetation(t.material)) {
        let b = bounds.entry(root(&mut parent, i)).or_insert(([f32::MAX; 3], [f32::MIN; 3], true));
        for v in t.v {
            for k in 0..3 {
                b.0[k] = b.0[k].min(v[k]);
                b.1[k] = b.1[k].max(v[k]);
            }
        }
        b.2 &= up(t);
    }
    (0..tris.len())
        .map(|i| bounds.get(&root(&mut parent, i)).is_some_and(|&(lo, hi, cap)| {
            let m = tris[i].material;
            hedge(m, lo, hi) || (soil(m) && cap)
        }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(z: f32, m: u8, x0: f32, x1: f32) -> [Tri; 2] {
        [
            Tri { v: [[x0, 0., z], [x1, 0., z], [x1, 4., z]], material: m },
            Tri { v: [[x0, 0., z], [x1, 4., z], [x0, 4., z]], material: m },
        ]
    }

    #[test]
    fn foliage_over_ground_goes_foliage_as_ground_stays() {
        let mut tris = Vec::new();
        tris.extend(quad(0.0, 4, 0.0, 4.0)); // tarmac
        tris.extend(quad(1.2, 50, 1.0, 3.0)); // bush top over it
        // A hedge side face standing on the tarmac.
        tris.push(Tri { v: [[1., 1., 0.], [3., 1., 0.], [3., 1., 1.2]], material: 50 });
        tris.extend(quad(0.0, 47, 10.0, 14.0)); // grass terrain, nothing under it
        tris.extend(quad(6.0, 50, 1.0, 3.0)); // canopy far above the ground (> depth)
        tris.extend(quad(3.5, 47, 1.0, 3.0)); // grass 3.5 m up: terrain over a tunnel
        assert_eq!(drop_over_ground(&mut tris), 3);
        assert_eq!(tris.iter().filter(|t| t.material == 4).count(), 2);
        assert_eq!(tris.iter().filter(|t| t.material == 47).count(), 4, "grass ground stays");
        assert_eq!(tris.iter().filter(|t| t.material == 50).count(), 2, "nothing within reach under it");
    }

    #[test]
    fn grass_ground_partly_over_a_wall_top_stays() {
        // rc7 (-181, 439): a grass ground triangle whose centroid lies 1 m over
        // a wall top 0.5 m wide, the rest of it over nothing.
        let mut tris = vec![
            Tri { v: [[0., 0., 1.], [6., 0., 1.], [0., 6., 1.]], material: 48 },
            Tri { v: [[1.75, -1., 0.], [2.25, -1., 0.], [2.25, 7., 0.]], material: 13 },
            Tri { v: [[1.75, -1., 0.], [2.25, 7., 0.], [1.75, 7., 0.]], material: 13 },
        ];
        assert_eq!(drop_over_ground(&mut tris), 0);
        assert_eq!(tris.len(), 3);
    }

    #[test]
    fn a_lawn_partly_over_a_buried_footing_stays() {
        // A lawn triangle (GRASS_SHORT) 6 m across; a wall's footing 0.6 m
        // under its middle only (2026-10-25 mark 1, Vinewood hillside).
        let lawn = Tri { v: [[0., 0., 1.], [6., 0., 1.], [3., 6., 1.]], material: 48 };
        let footing = |x0: f32, y0: f32, x1: f32, y1: f32| [
            Tri { v: [[x0, y0, 0.4], [x1, y0, 0.4], [x1, y1, 0.4]], material: 1 },
            Tri { v: [[x0, y0, 0.4], [x1, y1, 0.4], [x0, y1, 0.4]], material: 1 },
        ];
        let mut tris = vec![lawn];
        tris.extend(footing(1.0, 0.5, 5.0, 4.0));
        assert_eq!(drop_over_ground(&mut tris), 0, "the lawn's corners have nothing under them");
        // Pavement under all of a bush: it goes as before (a lone grass
        // mesh is planter soil and stays, `soil`).
        let mut tris = vec![Tri { material: 50, ..lawn }];
        tris.extend(footing(-1.0, -1.0, 7.0, 7.0));
        assert_eq!(drop_over_ground(&mut tris), 1);
    }

    #[test]
    fn hedges_and_trees_stay_over_ground() {
        let mut tris = Vec::new();
        tris.extend(quad(0.0, 4, -10.0, 10.0)); // tarmac
        // A 4 m hedge face, 1.5 m tall: two triangles sharing an edge.
        let (a, b, c, d) = ([0., 1., 0.], [4., 1., 0.], [4., 1., 1.5], [0., 1., 1.5]);
        tris.push(Tri { v: [a, b, c], material: 50 });
        tris.push(Tri { v: [a, c, d], material: 50 });
        // A bush 1.2 m wide and tall: goes.
        tris.push(Tri { v: [[6., 1., 0.], [7.2, 1., 0.], [7.2, 1., 1.2]], material: 50 });
        assert_eq!(drop_over_ground(&mut tris), 1);
        assert_eq!(tris.iter().filter(|t| t.material == 50).count(), 2, "the hedge stays");
        assert!(hedge(52, [0.0; 3], [1.4, 1.4, 4.6]), "a tree");
        assert!(!hedge(47, [0.0; 3], [6.0, 1.0, 1.5]), "grass is never a hedge");
    }

    #[test]
    fn gaps_fill_from_the_bullet_copy_and_covered_ground_does_not() {
        let mut tris: Vec<Tri> = quad(0.0, 4, -10.0, 10.0).to_vec(); // roof
        // The bullet copy: the same roof 5 cm higher (covered, stays out) and
        // a wall top 1.5 m up the walkable collision lacks (comes in).
        let mut extra: Vec<Tri> = quad(0.05, 4, -10.0, 10.0).to_vec();
        extra.extend(quad(1.5, 4, 12.0, 13.0));
        assert_eq!(fill_gaps(&mut tris, extra), 2);
        assert!(tris[2..].iter().all(|t| t.v[0][2] == 1.5), "only the wall top");
        assert!((dist2([0.5, 0.5, 2.0], &tris[0]) - 4.0).abs() < 1e-4, "point over the face");
        assert!((dist2([-11.0, 0.0, 0.0], &tris[0]) - 1.0).abs() < 1e-4, "point beside an edge");
    }

    #[test]
    fn a_wall_beside_a_hole_does_not_cover_it() {
        // A wall standing 0.5 m from a 1 m floor hole the bullet copy fills.
        let mut tris = vec![Tri { v: [[0.5, -5., 0.], [0.5, 5., 0.], [0.5, 5., 3.]], material: 4 }];
        let hole = Tri { v: [[-1., -0.5, 0.], [0., -0.5, 0.], [-0.5, 0.5, 0.]], material: 4 };
        assert_eq!(fill_floor_gaps(&mut tris, vec![hole]), (1, 0), "the hole's floor comes in");
        // The same floor already there (a coincident copy) stays out.
        let mut tris = vec![hole];
        assert_eq!(fill_floor_gaps(&mut tris, vec![hole]), (0, 0));
    }

    #[test]
    fn gap_fill_keeps_walkable_z_up_floors_only() {
        let slope = |rise: f32| Tri { v: [[0., 0., 0.], [1., 0., rise], [0., 1., 0.]], material: 4 };
        let floor = slope(1.0); // normal z = 1/sqrt(2), just above 0.7
        let steep = slope(1.03); // just below 0.7
        let wall = Tri { v: [[0., 0., 0.], [0., 1., 0.], [0., 1., 1.]], material: 4 };
        let underside = Tri { v: [floor.v[0], floor.v[2], floor.v[1]], material: 4 };
        let degenerate = slope(f32::INFINITY);
        let mut tris = Vec::new();
        assert_eq!(fill_floor_gaps(&mut tris, vec![floor, steep, wall, underside, degenerate]), (1, 3));
        assert_eq!(tris[0].v, floor.v);
        // Existing ped walls remain solid; the new rule only applies to extras.
        let mut tris = vec![wall];
        assert_eq!(fill_floor_gaps(&mut tris, vec![]), (0, 0));
        assert_eq!(tris[0].v, wall.v);
    }

    #[test]
    fn planter_soil_stays_grass_clumps_go() {
        let mut tris = Vec::new();
        tris.extend(quad(0.0, 4, -10.0, 10.0)); // street
        // A planter's dirt: a 3 m upward grass fan 0.8 m above the street.
        let c = [0.0, 2.0, 1.1];
        let ring: Vec<[f32; 3]> = (0..8).map(|k| { let a = k as f32 * std::f32::consts::FRAC_PI_4; [1.5 * a.cos(), 2.0 + 1.5 * a.sin(), 0.8] }).collect();
        for k in 0..8 {
            tris.push(Tri { v: [ring[k], ring[(k + 1) % 8], c], material: 47 });
        }
        // A grass clump 3 m across: an upward top and a side face.
        let (p, q, r) = ([5., 1., 0.5], [8., 1., 0.5], [8., 4., 0.5]);
        tris.push(Tri { v: [p, q, r], material: 47 });
        tris.push(Tri { v: [p, [8., 1., 0.0], q], material: 47 });
        assert_eq!(drop_over_ground(&mut tris), 2);
        assert_eq!(tris.iter().filter(|t| t.material == 47).count(), 8, "the dirt stays");
    }

    #[test]
    fn repeats_go_reversed_copies_stay() {
        let t = |v: [[f32; 3]; 3]| Tri { v, material: 1 };
        let (a, b, c) = ([0., 0., 0.], [1., 0., 0.], [0., 1., 0.]);
        let shift = |v: [[f32; 3]; 3], d: f32| t(v.map(|p| [p[0] + d, p[1] + d, p[2] + d]));
        let mut tris = vec![
            t([a, b, c]),
            t([b, c, [0.0002, 0., 0.]]),
            t([a, c, b]),
            t([a, b, [0., 2., 0.]]),
            shift([a, b, c], 0.004),
            shift([a, b, c], -0.006),
            shift([a, b, c], 0.015),
        ];
        assert_eq!(drop_repeats(&mut tris), 3, "rotated 0.2 mm, +4 mm and -6 mm copies go");
        assert_eq!(tris.len(), 4, "reversed winding, a different triangle and a 15 mm copy stay");
    }

    #[test]
    fn coplanar_duplicate_layers_are_dropped_and_top_detailed_surface_kept() {
        let t = |v: [[f32; 3]; 3]| Tri { v, material: 1 };
        let a = [0., 0., 0.];
        let b = [2., 0., 0.];
        let c = [0., 2., 0.];
        let bottom = t([a, b, c]);
        let top = t([[0., 0., 0.02], [2., 0., 0.02], [0., 2., 0.02]]);
        let side = t([[5., 5., 0.], [7., 5., 0.], [5., 7., 0.]]);
        let mut tris = vec![bottom, top, side];
        let dropped = drop_coplanar_layers(&mut tris);
        assert_eq!(dropped, 1, "lower coplanar layer must be excised");
        assert_eq!(tris.len(), 2);
        assert!((tris[0].v[0][2] - 0.02).abs() < 1e-4, "top layer kept");
        assert_eq!(tris[1].v[0], [5., 5., 0.], "side triangle kept");
    }

    #[test]
    fn coplanar_partial_overlaps_are_preserved_without_creating_holes() {
        let t = |v: [[f32; 3]; 3]| Tri { v, material: 1 };
        let base = t([[0., 0., 0.], [4., 0., 0.], [0., 4., 0.]]);
        // Overlaps only the inner corner, leaving outer region exposed
        let partial_top = t([[0., 0., 0.02], [2., 0., 0.02], [0., 2., 0.02]]);
        let mut tris = vec![base, partial_top];
        let dropped = drop_coplanar_layers(&mut tris);
        assert_eq!(dropped, 0, "partially overlapping face must not be dropped to prevent ground holes");
        assert_eq!(tris.len(), 2);
    }
}

/// Upward unit normal's z of a triangle (0 for a degenerate one).
fn normal_z(t: &Tri) -> f32 {
    let [a, b, c] = t.v;
    let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
    let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if l > 1e-12 { n[2] / l } else { 0.0 }
}

/// A stair ramp: sloped, not a landing and not a wall.
const RAMP_Z: (f32, f32) = (0.3, 0.97);
/// Where a step of the detailed copy may lie, from the ramp's surface: GTA's
/// ramps run over the step noses, so treads sit up to a riser below it.
const BELOW_RAMP: f32 = 0.45;
const ABOVE_RAMP: f32 = 0.25;
/// Treads this far apart in height are different steps.
const RISER_MIN: f32 = 0.08;
/// Highest step of an unflagged flight (`stairs_from_detail` candidates).
const RISER_MAX: f32 = 0.25;

/// Stair flights as steps (the maintainer, 2026-10-17: "make models and steps have the
/// correct shape that is also visible"). GTA's ped collision covers a flight
/// with a ramp whose material carries STAIRS (`stairs`, one flag per
/// triangle of `tris`); its bullet/camera copy (`extra`) has the steps. For
/// each connected group of stair ramps whose footprint holds detailed steps
/// (two treads or more at different heights), the ramps leave `tris` and
/// the steps leave `extra`, returned. A group without steps keeps its ramp.
/// Returns the steps and how many ramp triangles were replaced.
///
/// `candidates`: ramps without the STAIRS flag that may still cover steps
/// (inside a skatepark, `live.rs`): their group is replaced only when the
/// detailed copy shows a real flight, three or more treads `RISER_MIN`..
/// `RISER_MAX` apart, so bowls and quarter pipes keep their own surface.
pub fn stairs_from_detail(tris: &mut Vec<Tri>, stairs: &[bool], candidates: &[bool], extra: &mut Vec<Tri>) -> (Vec<Tri>, usize) {
    let (steps, ramps) = stairs_from_detail_keeping(tris, stairs, candidates, extra);
    (steps, ramps.len())
}

/// As [`stairs_from_detail`], returning the replaced ramp triangles too, so a
/// baked tile can keep both variants and the game picks one by its setting.
pub fn stairs_from_detail_keeping(tris: &mut Vec<Tri>, stairs: &[bool], candidates: &[bool], extra: &mut Vec<Tri>) -> (Vec<Tri>, Vec<Tri>) {
    let flagged = |i: usize| stairs.get(i).copied().unwrap_or(false);
    let ramps: Vec<usize> = (0..tris.len())
        .filter(|&i| flagged(i) || candidates.get(i).copied().unwrap_or(false))
        .filter(|&i| (RAMP_Z.0..RAMP_Z.1).contains(&normal_z(&tris[i])))
        .collect();
    if ramps.is_empty() {
        return (Vec::new(), Vec::new());
    }
    // Groups: ramps sharing a corner (1 cm).
    let mut parent: Vec<usize> = (0..ramps.len()).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let key = |v: [f32; 3]| v.map(|x| (x * 100.0).round() as i64);
    let mut corner: HashMap<[i64; 3], usize> = HashMap::default();
    for (k, &i) in ramps.iter().enumerate() {
        for v in tris[i].v {
            if let Some(&other) = corner.get(&key(v)) {
                let (a, b) = (root(&mut parent, k), root(&mut parent, other));
                parent[a] = b;
            } else {
                corner.insert(key(v), k);
            }
        }
    }
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::default();
    for (k, &i) in ramps.iter().enumerate() {
        for cell in cells(&tris[i]) {
            grid.entry(cell).or_default().push(k);
        }
    }
    // Each detailed triangle that lies in a ramp's footprint, by group.
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::default();
    for (e, t) in extra.iter().enumerate() {
        if normal_z(t) < -0.3 {
            continue; // undersides stay out
        }
        let c = centroid(t);
        let cell = ((c[0] / CELL).floor() as i32, (c[1] / CELL).floor() as i32);
        let Some(near) = grid.get(&cell) else { continue };
        let hit = near.iter().copied().find(|&k| {
            height(&tris[ramps[k]], [c[0], c[1]]).is_some_and(|z| c[2] >= z - BELOW_RAMP && c[2] <= z + ABOVE_RAMP)
        });
        if let Some(k) = hit {
            let g = root(&mut parent, k);
            groups.entry(g).or_default().push(e);
        }
    }
    let mut replaced_groups = Vec::new();
    let mut taken = vec![false; extra.len()];
    // Groups holding a ramp with the STAIRS flag.
    let mut flagged_groups = std::collections::HashSet::new();
    for (k, &i) in ramps.iter().enumerate() {
        if flagged(i) {
            flagged_groups.insert(root(&mut parent, k));
        }
    }
    for (&g, list) in &groups {
        let mut treads: Vec<f32> = list.iter().filter(|&&e| normal_z(&extra[e]) > 0.9).map(|&e| centroid(&extra[e])[2]).collect();
        treads.sort_by(f32::total_cmp);
        let heights = treads.windows(2).filter(|w| w[1] - w[0] >= RISER_MIN).count() + usize::from(!treads.is_empty());
        // Unflagged: distinct tread levels, each step a stair riser high.
        let mut flat: Vec<f32> = list.iter().filter(|&&e| normal_z(&extra[e]) > 0.97).map(|&e| centroid(&extra[e])[2]).collect();
        flat.sort_by(f32::total_cmp);
        let mut levels: Vec<f32> = Vec::new();
        for &z in &flat {
            if levels.last().is_none_or(|&l| z - l >= RISER_MIN) {
                levels.push(z);
            }
        }
        let flight = levels.len() >= 3 && levels.windows(2).all(|w| w[1] - w[0] <= RISER_MAX);
        if (flagged_groups.contains(&g) && heights >= 2) || flight {
            replaced_groups.push(g);
            for &e in list {
                taken[e] = true;
            }
        }
    }
    // Flagged flights the detailed copy has no steps for: steps built from the
    // ramp itself (`terrace`), so every flagged staircase rides as stairs, not
    // only where GTA happens to keep a stepped copy (Manatros 2026-10-20:
    // "stairs follow the same rules, make sure they always get detected the same way").
    let mut built: HashMap<usize, Vec<usize>> = HashMap::default();
    for (k, &i) in ramps.iter().enumerate() {
        let g = root(&mut parent, k);
        if flagged_groups.contains(&g) && !replaced_groups.contains(&g) {
            built.entry(g).or_default().push(i);
        }
    }
    if replaced_groups.is_empty() && built.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut synthetic = Vec::new();
    for members in built.values() {
        let group: Vec<Tri> = members.iter().map(|&i| tris[i]).collect();
        synthetic.extend(terrace(&group));
    }
    let mut drop = vec![false; tris.len()];
    for (k, &i) in ramps.iter().enumerate() {
        let g = root(&mut parent, k);
        if replaced_groups.contains(&g) || built.contains_key(&g) {
            drop[i] = true;
        }
    }
    let mut removed = Vec::new();
    let mut index = 0;
    tris.retain(|t| {
        index += 1;
        if drop[index - 1] {
            removed.push(*t);
        }
        !drop[index - 1]
    });
    let mut steps = Vec::new();
    let mut index = 0;
    extra.retain(|t| {
        index += 1;
        if taken[index - 1] {
            steps.push(*t);
            false
        } else {
            true
        }
    });
    steps.extend(synthetic);
    (steps, removed)
}

/// Height of `t` at XY `p` when its upward normal's z is at least `min_z`.
pub fn surface_at(t: &Tri, p: [f32; 2], min_z: f32) -> Option<f32> {
    if normal_z(t) >= min_z { height(t, p) } else { None }
}

/// Walking ramps laid over real steps of the same collision (Manatros
/// 2026-10-25, marks 3, 4, 7, 8 at Vespucci: GTA's park file `vb_34_*.ybn`
/// holds the treads and risers *and* a 34 degree ramp over their noses, so
/// the board rode the ramp). A connected group of sloped triangles goes when
/// under its whole footprint (centroid and points 80 % toward each corner)
/// level treads (under 2.6 degrees) form a flight under it: two levels or
/// more, each `RISER_MIN`..`RISER_MAX` apart (mark 3: an 8 degree ramp over
/// three long steps; mark 8: a ramp over the last two steps), with at least
/// half of it over a surface. The ramp is cut into 12 cm pieces: pieces
/// over a surface go, pieces over nothing stay (2026-10-25 19:50 test: at
/// the foot of the pool stairs the ramp is the only floor). The
/// steps stay, so nothing opens; bowls, banks and quarter pipes have no
/// treads under them and keep their surface. Returns the removed ramps.
pub fn ramps_over_steps(tris: &mut Vec<Tri>) -> (Vec<Tri>, Vec<Tri>) {
    // Support: any flat surface (under 5.7 degrees). Stair levels: only
    // level treads (under 2.6 degrees), so a gently sloped underlay holds a
    // ramp up but never fakes steps (Codex peer review 2026-10-25).
    const SUPPORT_Z: f32 = 0.995;
    const TREAD_Z: f32 = 0.999;
    // Ramp pieces (metres) kept where nothing is under them.
    const PIECE: f32 = 0.12;
    let mut support: HashMap<(i32, i32), Vec<usize>> = HashMap::default();
    for (i, t) in tris.iter().enumerate() {
        if normal_z(t) > SUPPORT_Z {
            for cell in cells(t) {
                support.entry(cell).or_default().push(i);
            }
        }
    }
    let ramps: Vec<usize> = (0..tris.len()).filter(|&i| (RAMP_Z.0..SUPPORT_Z).contains(&normal_z(&tris[i]))).collect();
    if ramps.is_empty() || support.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut parent: Vec<usize> = (0..ramps.len()).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let key = |v: [f32; 3]| v.map(|x| (x * 100.0).round() as i64);
    let mut corner: HashMap<[i64; 3], usize> = HashMap::default();
    for (k, &i) in ramps.iter().enumerate() {
        for v in tris[i].v {
            if let Some(&other) = corner.get(&key(v)) {
                let (a, b) = (root(&mut parent, k), root(&mut parent, other));
                parent[a] = b;
            } else {
                corner.insert(key(v), k);
            }
        }
    }
    // A point of a ramp (at its height) has a surface under it.
    let held = |p: [f32; 3]| {
        let cell = ((p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32);
        support.get(&cell).is_some_and(|list| list.iter().any(|&j| {
            height(&tris[j], [p[0], p[1]]).is_some_and(|h| h <= p[2] + 0.02 && h >= p[2] - BELOW_RAMP)
        }))
    };
    // A level tread: under 2.6 degrees and no more than a riser's minimum
    // high from corner to corner, so a large gently sloped triangle never
    // gives two "levels" (Codex peer review 2026-10-25: terrain at 2.5
    // degrees under a bank).
    let level = |t: &Tri| {
        let z = t.v.map(|p| p[2]);
        normal_z(t) > TREAD_Z
            && z.iter().copied().fold(f32::MIN, f32::max) - z.iter().copied().fold(f32::MAX, f32::min) <= RISER_MIN
    };
    // First classify each group on a coarse grid (at most 10 x 10 per
    // triangle): its tread levels and how much of it is held. Only flights
    // are cut into pieces (Codex peer review: cutting every slope of the
    // map first could hold tens of millions of pieces).
    let mut levels: HashMap<usize, Vec<f32>> = HashMap::default();
    let mut held_count: HashMap<usize, (usize, usize)> = HashMap::default();
    for (k, &i) in ramps.iter().enumerate() {
        let g = root(&mut parent, k);
        let t = tris[i];
        let n = 10usize;
        let e = held_count.entry(g).or_default();
        for i in 0..=n {
            for j in 0..=n - i {
                let (a, b) = (0.02 + 0.96 * i as f32 / n as f32, 0.02 + 0.96 * j as f32 / n as f32);
                if a + b > 0.98 {
                    continue;
                }
                let p: [f32; 3] = std::array::from_fn(|k| t.v[0][k] + (t.v[1][k] - t.v[0][k]) * a + (t.v[2][k] - t.v[0][k]) * b);
                e.0 += usize::from(held(p));
                e.1 += 1;
            }
        }
        for cell in cells(&t) {
            for &j in support.get(&cell).into_iter().flatten() {
                if !level(&tris[j]) {
                    continue;
                }
                let m = centroid(&tris[j]);
                if height(&t, [m[0], m[1]]).is_some_and(|z| m[2] <= z + 0.02 && m[2] >= z - BELOW_RAMP) {
                    levels.entry(g).or_default().push(m[2]);
                }
            }
        }
    }
    // A flight: two levels or more, each a stair riser apart, and at least
    // half the ramp over a surface; the rest of the ramp stays as pieces.
    let flights: std::collections::HashSet<usize> = levels.into_iter().filter_map(|(g, mut z)| {
        z.sort_by(f32::total_cmp);
        let mut distinct: Vec<f32> = Vec::new();
        for h in z {
            if distinct.last().is_none_or(|&l| h - l >= RISER_MIN) {
                distinct.push(h);
            }
        }
        let (ok, all) = held_count.get(&g).copied().unwrap_or((0, 1));
        (distinct.len() >= 2 && distinct.windows(2).all(|w| w[1] - w[0] <= RISER_MAX) && ok * 2 >= all).then_some(g)
    }).collect();
    let mut drop = vec![false; tris.len()];
    let mut kept = Vec::new();
    for (k, &i) in ramps.iter().enumerate() {
        if !flights.contains(&root(&mut parent, k)) {
            continue;
        }
        drop[i] = true;
        // Cut into pieces; a piece stays when any of seven points (middle,
        // corners and edge middles pulled 2 % in) has nothing under it
        // (19:50 bake check: a strip at the foot of the pool stairs).
        let t = tris[i];
        let edge = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
        let n = ((edge(t.v[0], t.v[1]).max(edge(t.v[1], t.v[2])).max(edge(t.v[2], t.v[0])) / PIECE).ceil() as usize).clamp(1, 200);
        let at = |i: usize, j: usize| -> [f32; 3] {
            let (a, b) = (i as f32 / n as f32, j as f32 / n as f32);
            std::array::from_fn(|k| t.v[0][k] + (t.v[1][k] - t.v[0][k]) * a + (t.v[2][k] - t.v[0][k]) * b)
        };
        for i in 0..n {
            for j in 0..n - i {
                let mut tri = vec![[at(i, j), at(i + 1, j), at(i, j + 1)]];
                if i + j + 1 < n {
                    tri.push([at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)]);
                }
                for v in tri {
                    let piece = Tri { v, material: t.material };
                    let c = centroid(&piece);
                    let pull = |p: [f32; 3]| std::array::from_fn::<f32, 3, _>(|k| p[k] + (c[k] - p[k]) * 0.02);
                    let mid = |a: [f32; 3], b: [f32; 3]| std::array::from_fn::<f32, 3, _>(|k| (a[k] + b[k]) * 0.5);
                    let points = [c, pull(v[0]), pull(v[1]), pull(v[2]), pull(mid(v[0], v[1])), pull(mid(v[1], v[2])), pull(mid(v[2], v[0]))];
                    if !points.iter().all(|&p| held(p)) {
                        kept.push(piece);
                    }
                }
            }
        }
    }
    let mut removed = Vec::new();
    let mut index = 0;
    tris.retain(|t| {
        index += 1;
        if drop[index - 1] {
            removed.push(*t);
        }
        !drop[index - 1]
    });
    (removed, kept)
}

#[cfg(test)]
mod ramp_tests {
    use super::*;

    fn quad(out: &mut Vec<Tri>, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], material: u8) {
        out.push(Tri { v: [a, b, c], material });
        out.push(Tri { v: [a, c, d], material });
    }

    /// Five 0.22 m steps 0.33 m deep along +y, 2 m wide, and GTA's ramp
    /// over their noses.
    fn flight(with_steps: bool) -> Vec<Tri> {
        let mut t = Vec::new();
        quad(&mut t, [0., -2., 0.], [2., -2., 0.], [2., 0., 0.], [0., 0., 0.], 1);
        if with_steps {
            for k in 0..5 {
                let (y0, y1, z) = (k as f32 * 0.33, (k + 1) as f32 * 0.33, k as f32 * 0.22);
                quad(&mut t, [0., y0, z], [2., y0, z], [2., y1, z], [0., y1, z], 1);
                quad(&mut t, [0., y1, z], [2., y1, z], [2., y1, z + 0.22], [0., y1, z + 0.22], 1);
            }
        }
        quad(&mut t, [0., 1.65, 1.1], [2., 1.65, 1.1], [2., 3.5, 1.1], [0., 3.5, 1.1], 1);
        quad(&mut t, [0., 0., 0.], [2., 0., 0.], [2., 1.65, 1.1], [0., 1.65, 1.1], 0);
        t
    }

    #[test]
    fn a_ramp_over_real_steps_goes_and_the_steps_stay() {
        let mut t = flight(true);
        let before = t.len();
        let (removed, kept) = ramps_over_steps(&mut t);
        assert_eq!(removed.len(), 2);
        assert!(removed.iter().all(|r| r.material == 0));
        assert!(kept.is_empty(), "every piece is over a step: {}", kept.len());
        assert_eq!(t.len(), before - 2);
    }

    #[test]
    fn a_ramp_without_steps_under_it_stays() {
        let mut t = flight(false);
        assert!(ramps_over_steps(&mut t).0.is_empty());
    }

    #[test]
    fn a_ramp_over_a_gap_keeps_its_piece_over_the_gap() {
        // Codex 2026-10-25: platforms y 0..0.8 at z 0 and y 1.2..2 at z 0.2.
        let mut t = Vec::new();
        quad(&mut t, [0., 0., 0.], [2., 0., 0.], [2., 0.8, 0.], [0., 0.8, 0.], 1);
        quad(&mut t, [0., 1.2, 0.2], [2., 1.2, 0.2], [2., 2., 0.2], [0., 2., 0.2], 1);
        quad(&mut t, [0., 0., 0.], [2., 0., 0.], [2., 2., 0.4], [0., 2., 0.4], 0);
        let (removed, kept) = ramps_over_steps(&mut t);
        // The ramp goes over the platforms, its pieces stay over the gap.
        assert_eq!(removed.len(), 2);
        assert!(!kept.is_empty());
        for p in &kept {
            let c = centroid(p);
            assert!(c[1] > 0.7 && c[1] < 1.3, "kept piece outside the gap: {c:?}");
        }
    }

    #[test]
    fn a_bank_over_sloped_ground_stays() {
        // Codex 2026-10-25: bank z = 0.1 + 0.16 y over terrain z = 0.08 y.
        let mut t = Vec::new();
        quad(&mut t, [0., 0., 0.], [2., 0., 0.], [2., 4., 0.32], [0., 4., 0.32], 1);
        quad(&mut t, [0., 0., 0.1], [2., 0., 0.1], [2., 4., 0.74], [0., 4., 0.74], 0);
        assert!(ramps_over_steps(&mut t).0.is_empty());
    }

    #[test]
    fn a_bank_over_gently_sloped_terrain_stays() {
        // Codex 2026-10-25: terrain z = 0.044 y (2.5 degrees), bank z = 0.05 + 0.11 y.
        let mut t = Vec::new();
        quad(&mut t, [0., 0., 0.], [2., 0., 0.], [2., 6., 0.264], [0., 6., 0.264], 1);
        quad(&mut t, [0., 0., 0.05], [2., 0., 0.05], [2., 6., 0.71], [0., 6., 0.71], 0);
        assert!(ramps_over_steps(&mut t).0.is_empty());
    }

    #[test]
    fn a_bowl_wall_over_a_buried_floor_stays() {
        let mut t = Vec::new();
        quad(&mut t, [0., 0., 0.], [4., 0., 0.], [4., 4., 0.], [0., 4., 0.], 1);
        quad(&mut t, [0., 0., 0.85], [4., 0., 0.85], [4., 2., 1.6], [0., 2., 1.6], 1);
        assert!(ramps_over_steps(&mut t).0.is_empty());
    }
}

/// Step height a built flight aims at (metres): GTA's stairs are 15-18 cm.
const BUILT_RISER: f32 = 0.17;

/// Steps under a stair ramp `group` (GTA's walking ramp over a flight's
/// noses): its rise in steps of about `BUILT_RISER`, each tread level at the
/// height of the nose below it and reaching uphill to the next nose, with a
/// vertical riser up to that nose. Every triangle is cut at the same levels,
/// so a group's pieces line up.
pub fn terrace(group: &[Tri]) -> Vec<Tri> {
    let lo = group.iter().flat_map(|t| t.v.iter().map(|p| p[2])).fold(f32::MAX, f32::min);
    let hi = group.iter().flat_map(|t| t.v.iter().map(|p| p[2])).fold(f32::MIN, f32::max);
    let rise = hi - lo;
    if !(rise > RISER_MIN) {
        return group.to_vec();
    }
    let steps = (rise / BUILT_RISER).round().max(1.0);
    let h = rise / steps;
    let level = |k: f32| lo + k * h;
    let mut out = Vec::new();
    for t in group {
        let material = t.material;
        let zmin = t.v.iter().map(|p| p[2]).fold(f32::MAX, f32::min);
        let zmax = t.v.iter().map(|p| p[2]).fold(f32::MIN, f32::max);
        let k0 = ((zmin - lo) / h - 1e-4).floor().max(0.0) as i32;
        let k1 = ((zmax - lo) / h + 1e-4).ceil() as i32;
        for k in k0..k1 {
            let (a, b) = (level(k as f32), level(k as f32 + 1.0));
            // The part of the triangle between the two levels, laid flat at `a`.
            let band = clip_z(&clip_z(&t.v, a, true), b, false);
            if band.len() >= 3 {
                for i in 1..band.len() - 1 {
                    let mut tri = [band[0], band[i], band[i + 1]].map(|p| [p[0], p[1], a]);
                    let n = (tri[1][0] - tri[0][0]) * (tri[2][1] - tri[0][1]) - (tri[1][1] - tri[0][1]) * (tri[2][0] - tri[0][0]);
                    if n.abs() < 1e-8 {
                        continue;
                    }
                    if n < 0.0 {
                        tri.swap(1, 2);
                    }
                    out.push(Tri { v: tri, material });
                }
            }
            // The riser up to the next nose: where the ramp crosses level `b`.
            if k + 1 < k1 || (b - zmax).abs() > 1e-4 {
                if let Some((p, q)) = cut_at(&t.v, b) {
                    let mut quad = [[p[0], p[1], a], [q[0], q[1], a], [q[0], q[1], b], [p[0], p[1], b]];
                    // Facing downhill: away from the triangle's higher corner.
                    let up = t.v.iter().copied().fold(t.v[0], |m, v| if v[2] > m[2] { v } else { m });
                    let (ex, ey) = (q[0] - p[0], q[1] - p[1]);
                    let (nx, ny) = (ey, -ex); // the quad's normal for this winding
                    if nx * (up[0] - p[0]) + ny * (up[1] - p[1]) > 0.0 {
                        quad.reverse();
                    }
                    out.push(Tri { v: [quad[0], quad[1], quad[2]], material });
                    out.push(Tri { v: [quad[0], quad[2], quad[3]], material });
                }
            }
        }
    }
    out
}

/// `poly` clipped to z >= `at` (`above`) or z <= `at`.
fn clip_z(poly: &[[f32; 3]], at: f32, above: bool) -> Vec<[f32; 3]> {
    let inside = |p: &[f32; 3]| if above { p[2] >= at - 1e-6 } else { p[2] <= at + 1e-6 };
    let mut out = Vec::new();
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        if inside(&p) {
            out.push(p);
        }
        if inside(&p) != inside(&q) {
            let s = (at - p[2]) / (q[2] - p[2]);
            out.push([p[0] + (q[0] - p[0]) * s, p[1] + (q[1] - p[1]) * s, at]);
        }
    }
    out
}

/// Where the triangle crosses height `at`: the segment's two ends.
fn cut_at(t: &[[f32; 3]; 3], at: f32) -> Option<([f32; 3], [f32; 3])> {
    let mut ends = Vec::new();
    for i in 0..3 {
        let (p, q) = (t[i], t[(i + 1) % 3]);
        if (p[2] - at) * (q[2] - at) < 0.0 {
            let s = (at - p[2]) / (q[2] - p[2]);
            ends.push([p[0] + (q[0] - p[0]) * s, p[1] + (q[1] - p[1]) * s, at]);
        }
    }
    (ends.len() == 2).then(|| (ends[0], ends[1]))
}

#[cfg(test)]
mod stair_tests {
    use super::*;

    /// A 4-step flight rising along +y (riser 0.18, tread 0.3, 2 m wide),
    /// as GTA's detailed copy: treads and risers, two triangles each.
    fn flight() -> Vec<Tri> {
        let mut out = Vec::new();
        let quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], out: &mut Vec<Tri>| {
            out.push(Tri { v: [a, b, c], material: 1 });
            out.push(Tri { v: [a, c, d], material: 1 });
        };
        for s in 0..4 {
            let (y0, y1, z0, z1) = (s as f32 * 0.3, (s + 1) as f32 * 0.3, s as f32 * 0.18, (s + 1) as f32 * 0.18);
            quad([0., y0, z0], [2., y0, z0], [2., y0, z1], [0., y0, z1], &mut out); // riser
            quad([0., y0, z1], [2., y0, z1], [2., y1, z1], [0., y1, z1], &mut out); // tread
        }
        out
    }

    /// GTA's ped ramp over the noses: from (y 0, z 0.18) to (y 0.9, z 0.72) and on to the top.
    fn ramp() -> Vec<Tri> {
        let (a, b, c, d) = ([0., 0., 0.18], [2., 0., 0.18], [2., 1.2, 0.72], [0., 1.2, 0.72]);
        vec![Tri { v: [a, b, c], material: 1 }, Tri { v: [a, c, d], material: 1 }]
    }

    #[test]
    fn a_stair_ramp_over_detailed_steps_becomes_the_steps() {
        let mut tris = ramp();
        tris.push(Tri { v: [[5., 5., 0.], [6., 5., 0.], [5., 6., 0.]], material: 2 }); // other ground stays
        let stairs = vec![true, true, false];
        let mut extra = flight();
        let detail = extra.len();
        let (steps, replaced) = stairs_from_detail(&mut tris, &stairs, &[], &mut extra);
        assert_eq!(replaced, 2, "both ramp triangles go");
        assert_eq!(tris.len(), 1, "only the other ground is left");
        assert_eq!(steps.len() + extra.len(), detail);
        assert!(steps.len() >= 12, "treads and risers come in: {}", steps.len());
    }

    #[test]
    fn a_flagged_ramp_without_a_stepped_copy_gets_built_steps() {
        // GTA's ramp over 3 noses (0.54 m rise), no detailed copy at all.
        let mut tris = ramp();
        let mut extra = Vec::new();
        let (steps, replaced) = stairs_from_detail(&mut tris, &[true, true], &[], &mut extra);
        assert_eq!(replaced, 2);
        assert!(tris.is_empty());
        let treads: Vec<f32> = steps.iter().filter(|t| normal_z(t) > 0.99).map(|t| t.v[0][2]).collect();
        let mut levels = treads.clone();
        levels.sort_by(f32::total_cmp);
        levels.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
        assert_eq!(levels.len(), 3, "three treads at three heights: {levels:?}");
        assert!(steps.iter().any(|t| normal_z(t).abs() < 0.01), "vertical risers");
        // Every tread lies at or under the ramp (the ramp runs over the noses).
        for t in steps.iter().filter(|t| normal_z(t) > 0.99) {
            let c = centroid(t);
            let ramp_z = 0.18 + (c[1] / 1.2) * 0.54;
            assert!(c[2] <= ramp_z + 1e-3, "tread above the ramp: {c:?}");
        }
    }

    #[test]
    fn an_unflagged_park_ramp_over_a_flight_becomes_steps_but_a_smooth_ramp_stays() {
        // Candidate (no flag) over a 4-step flight: replaced.
        let mut tris = ramp();
        let mut extra = flight();
        assert_eq!(stairs_from_detail(&mut tris, &[false, false], &[true, true], &mut extra).1, 2);
        // Candidate over a copy of itself (a smooth ramp or a transition): kept.
        let mut tris = ramp();
        let mut same = ramp();
        assert_eq!(stairs_from_detail(&mut tris, &[false, false], &[true, true], &mut same).1, 0);
        // Two treads only (a curb or a ledge under it): not a flight.
        let mut tris = ramp();
        let mut two: Vec<Tri> = flight().into_iter().filter(|t| centroid(t)[2] < 0.4 && normal_z(t) > 0.9).collect();
        assert_eq!(stairs_from_detail(&mut tris, &[false, false], &[true, true], &mut two).1, 0);
    }

    #[test]
    fn a_ramp_without_the_stairs_flag_stays_and_a_flagged_one_over_a_sloped_copy_is_built() {
        let mut tris = ramp();
        let mut extra = flight();
        assert_eq!(stairs_from_detail(&mut tris, &[false, false], &[], &mut extra).1, 0, "no STAIRS flag: kept");
        assert_eq!(tris.len(), 2);
        // Flagged, but the detailed copy is only a slope: the steps are built
        // from the ramp, and the sloped copy stays a gap filler.
        let mut flat_detail = vec![Tri { v: [[0., 0., 0.18], [2., 0., 0.18], [2., 1.2, 0.72]], material: 1 }];
        let (steps, replaced) = stairs_from_detail(&mut tris, &[true, true], &[], &mut flat_detail);
        assert_eq!(replaced, 2);
        assert!(!steps.is_empty() && tris.is_empty());
        assert_eq!(flat_detail.len(), 1);
    }
}

#[cfg(test)]
mod hidden_layer_tests {
    use super::*;

    fn floor(x0: f32, x1: f32, y0: f32, y1: f32, z: f32) -> Vec<Tri> {
        vec![Tri { v: [[x0, y0, z], [x1, y0, z], [x1, y1, z]], material: 1 }, Tri { v: [[x0, y0, z], [x1, y1, z], [x0, y1, z]], material: 1 }]
    }

    #[test]
    fn a_floor_under_a_larger_floor_goes_and_the_rest_stay() {
        // 10 cm under a larger floor: hidden.
        let mut t = floor(0., 4., 0., 4., 1.0);
        t.extend(floor(1., 2., 1., 2., 0.9));
        assert_eq!(drop_hidden_layers(&mut t, |_| true, |_| true), 2);
        assert_eq!(t.len(), 2);
        // 10 cm under a smaller floor: partly open, stays.
        let mut t = floor(1., 2., 1., 2., 1.0);
        t.extend(floor(0., 4., 0., 4., 0.9));
        assert_eq!(drop_hidden_layers(&mut t, |_| true, |_| true), 0);
        // 40 cm under: reachable, stays.
        let mut t = floor(0., 4., 0., 4., 1.0);
        t.extend(floor(1., 2., 1., 2., 0.6));
        assert_eq!(drop_hidden_layers(&mut t, |_| true, |_| true), 0);
        // A slab: top and bottom face opposite ways, both stay.
        let mut t = floor(0., 4., 0., 4., 1.0);
        t.extend(floor(0., 4., 0., 4., 0.9).into_iter().map(|t| Tri { v: [t.v[0], t.v[2], t.v[1]], material: 1 }));
        assert_eq!(drop_hidden_layers(&mut t, |_| true, |_| true), 0);
    }

    #[test]
    fn fixed_faces_neither_go_nor_cover_and_a_dropped_cover_does_not_count() {
        let mut t = floor(0., 4., 0., 4., 1.0);
        t.extend(floor(1., 2., 1., 2., 0.9));
        assert_eq!(drop_hidden_layers(&mut t, |_| true, |t| t.v[0][2] < 0.95), 0, "a face that may not cover does not cover");
        let mut t2 = floor(0., 4., 0., 4., 1.0);
        t2.extend(floor(1., 2., 1., 2., 0.9));
        assert_eq!(drop_hidden_layers(&mut t2, |t| t.v[0][2] > 0.95, |_| true), 0, "a face that may not go stays");
        // C under B under A: B (big) is covered by A only where A is; C is
        // covered by B alone, B goes, so C stays.
        let mut t = floor(0., 1., 0., 1., 1.2);
        t.extend(floor(0., 1., 0., 1., 1.0));
        t.extend(floor(0., 1., 0., 1., 0.8));
        let dropped = drop_hidden_layers(&mut t, |_| true, |_| true);
        assert_eq!(dropped, 2, "only B goes (both its triangles); C was covered by B alone, A is 40 cm off: {t:?}");
        assert!(t.iter().any(|t| t.v[0][2] == 0.8), "C stays");
    }
}
