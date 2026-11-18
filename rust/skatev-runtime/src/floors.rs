//! Top floor wins (design docs/designs/2026-11-18-top-floor-wins.md; Manatros
//! 2026-11-18: "make sure to always use the top layer collision when 2 faces
//! are very close to each other"). GTA stacks floors 1.5..30 cm apart (a road
//! sheet over the canal ground, thin boxes on a slope); the visible surface is
//! the upper one, but a board that slips under its edge rides the lower one
//! with its wheels in the drawn ground. `svwc::clean::drop_hidden_layers` only
//! drops a lower face that is covered whole and at most 6 m long, so large
//! partly covered ones stayed. Here every such face is split and its covered
//! pieces go; where a dropped piece meets kept lower floor a riser closes the
//! gap, so the upper sheet's edge is a step and nothing rolls under it.
//!
//! Runs on each decoded tile (`tiles::Cache::get`), outside the rules hash:
//! installed tiles stay valid. Park circles and tagged faces are untouched.
use crate::tiles::{Tile, Triangle};
use rustc_hash::FxHashMap as HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

/// Switch (default on). Read when a tile is decoded.
pub static TOP_FLOORS: AtomicBool = AtomicBool::new(true);
/// Totals since start (log).
pub static DROPPED: AtomicUsize = AtomicUsize::new(0);
pub static RISERS: AtomicUsize = AtomicUsize::new(0);
pub static SPLIT_FACES: AtomicUsize = AtomicUsize::new(0);

/// Park, slab, visbar, step/ramp variants, fallback, live, railbox bits (17..29).
const SPECIAL: u32 = 0x3FFE_0000;
/// No grind line from a riser.
const NO_GRIND: u32 = 1 << 30;
/// A floor faces up at least this much (unit normal z, 45 deg).
const MIN_UP: f32 = 0.7;
/// Gaps the board can get stuck in: above the coplanar pass's range, up to
/// `drop_hidden_layers`' gap (neither board nor skater fits between).
const MIN_GAP: f32 = 0.015;
const MAX_GAP: f32 = 0.30;
/// Normals of a floor and its cover within 37 deg (13:04: a 25 deg bank over a 16 deg floor).
const MIN_DOT: f32 = 0.8;
/// Smallest piece edge (m).
/// Riser segments at most this long (m).
const RISER_STEP: f32 = 0.5;
/// Park circles widened by this (m) are left alone.
const PARK_MARGIN: f32 = 5.0;
const CELL: f32 = 2.0;

type V = [f32; 3];

fn sub(a: V, b: V) -> V { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn cross(a: V, b: V) -> V { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
fn dot(a: V, b: V) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn centroid(t: &Triangle) -> V { [(t[0][0] + t[1][0] + t[2][0]) / 3., (t[0][1] + t[1][1] + t[2][1]) / 3., (t[0][2] + t[1][2] + t[2][2]) / 3.] }

fn unit_normal(t: &Triangle) -> V {
    let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    let l = dot(n, n).sqrt();
    if l > 1e-9 && l.is_finite() { [n[0] / l, n[1] / l, n[2] / l] } else { [0.; 3] }
}

fn bounds(t: &Triangle) -> ([f32; 3], [f32; 3]) {
    let mut lo = t[0];
    let mut hi = t[0];
    for p in &t[1..] {
        for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); }
    }
    (lo, hi)
}

/// Height of a non-vertical triangle over (x, y), when (x, y) lies on it.
fn height_at(t: &Triangle, x: f32, y: f32) -> Option<f32> {
    let [a, b, c] = *t;
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if d.abs() < 1e-12 { return None; }
    let w0 = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / d;
    let w1 = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / d;
    let w2 = 1. - w0 - w1;
    (w0 >= -1e-5 && w1 >= -1e-5 && w2 >= -1e-5).then(|| w0 * a[2] + w1 * b[2] + w2 * c[2])
}

fn in_park(p: V) -> bool {
    crate::parks::PARKS.iter().any(|park| {
        let (dx, dy) = (p[0] - park.centre.x, p[1] - park.centre.y);
        dx * dx + dy * dy <= (park.radius + PARK_MARGIN).powi(2)
    })
}

/// The tile's floors (each one both a candidate to go and a cover).
struct Floors {
    tris: Vec<Triangle>,
    normals: Vec<V>,
    lo: Vec<V>,
    hi: Vec<V>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

impl Floors {
    fn new(tris: Vec<Triangle>) -> Self {
        let normals: Vec<V> = tris.iter().map(unit_normal).collect();
        let (lo, hi): (Vec<V>, Vec<V>) = tris.iter().map(bounds).unzip();
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::default();
        for (i, (l, h)) in lo.iter().zip(&hi).enumerate() {
            if h[0] - l[0] > 2000. || h[1] - l[1] > 2000. { continue; }
            for x in (l[0] / CELL).floor() as i32..=(h[0] / CELL).floor() as i32 {
                for y in (l[1] / CELL).floor() as i32..=(h[1] / CELL).floor() as i32 {
                    grid.entry((x, y)).or_default().push(i as u32);
                }
            }
        }
        Self { tris, normals, lo, hi, grid }
    }

    fn near(&self, x: f32, y: f32) -> &[u32] {
        self.grid.get(&((x / CELL).floor() as i32, (y / CELL).floor() as i32)).map_or(&[], |v| v.as_slice())
    }

    /// The other floors that may cover floor `own` somewhere.
    fn covers_of(&self, own: usize, n: V) -> Vec<usize> {
        let (lo, hi) = (self.lo[own], self.hi[own]);
        let mut out: Vec<usize> = Vec::new();
        for x in (lo[0] / CELL).floor() as i32..=(hi[0] / CELL).floor() as i32 {
            for y in (lo[1] / CELL).floor() as i32..=(hi[1] / CELL).floor() as i32 {
                let Some(list) = self.grid.get(&(x, y)) else { continue };
                for &j in list {
                    let j = j as usize;
                    if j != own && dot(n, self.normals[j]) >= MIN_DOT
                        && self.lo[j][0] <= hi[0] && self.hi[j][0] >= lo[0]
                        && self.lo[j][1] <= hi[1] && self.hi[j][1] >= lo[1]
                        && self.hi[j][2] >= lo[2] + MIN_GAP && self.lo[j][2] <= hi[2] + MAX_GAP
                    { out.push(j); }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Pass 1: where each other floor lies MIN_GAP..MAX_GAP above floor `own`
    /// (convex, counter-clockwise, inside both outlines), with its plane.
    fn raw_regions(&self, own: usize) -> Vec<(Poly, Plane, usize)> {
        let n = self.normals[own];
        let covers = self.covers_of(own, n);
        let face = self.tris[own];
        let Some(low) = Plane::of(&face) else { return Vec::new() };
        let outline = ccw(face.iter().map(|p| [p[0], p[1]]).collect());
        let mut out = Vec::new();
        for &j in &covers {
            let Some(top) = Plane::of(&self.tris[j]) else { continue };
            let gap = top.minus(&low);
            let mut r = ccw(self.tris[j].iter().map(|p| [p[0], p[1]]).collect());
            r = clip(&r, |q| gap.z(q) - MIN_GAP);
            r = clip(&r, |q| MAX_GAP - gap.z(q));
            for k in 0..outline.len() {
                let (u, v) = (outline[k], outline[(k + 1) % outline.len()]);
                r = clip(&r, |q| (v[0] - u[0]) * (q[1] - u[1]) - (v[1] - u[1]) * (q[0] - u[0]));
            }
            if r.len() >= 3 && area(&r) >= 1e-4 { out.push((r, top, j)); }
        }
        out
    }

    /// Whether point q (on a floor at height z with normal n, floor `own`) has
    /// an effective cover: a floor MIN_GAP..MAX_GAP above it that is not
    /// itself covered there (regions from pass 1).
    fn covered_eff(&self, own: usize, q: [f32; 2], z: f32, n: V, raw: &[Vec<(Poly, Plane, usize)>]) -> bool {
        self.near(q[0], q[1]).iter().any(|&j| {
            let j = j as usize;
            j != own && dot(n, self.normals[j]) >= MIN_DOT
                && height_at(&self.tris[j], q[0], q[1]).is_some_and(|zj| zj - z >= MIN_GAP && zj - z <= MAX_GAP)
                && !raw[j].iter().any(|(r, _, _)| inside(r, q))
        })
    }

    /// Pass 2: floor `own` minus its effectively covered parts (exact convex
    /// pieces), plus risers where a covered part meets open lower floor.
    /// None when nothing measurable is covered.
    fn cut(&self, own: usize, walls: &Walls, raw: &[Vec<(Poly, Plane, usize)>]) -> Option<(Vec<Triangle>, Vec<Triangle>)> {
        if raw[own].is_empty() { return None; }
        let n = self.normals[own];
        let face = self.tris[own];
        let low = Plane::of(&face)?;
        let outline = ccw(face.iter().map(|p| [p[0], p[1]]).collect());
        let mut pieces: Vec<Poly> = vec![outline.clone()];
        let mut regions: Vec<(Poly, Plane)> = Vec::new();
        for (r, top, j) in &raw[own] {
            // A cover that is itself covered there does not count (A over B over
            // C: B's covered part must not take C away under a far A).
            let mut eff = vec![r.clone()];
            for (rj, _, _) in &raw[*j] {
                let mut next = Vec::with_capacity(eff.len() + 3);
                for e in eff { next.extend(subtract(&e, rj)); }
                eff = next;
            }
            for e in eff {
                if area(&e) < 1e-4 { continue; }
                let mut next = Vec::with_capacity(pieces.len() + 3);
                for p in pieces { next.extend(subtract(&p, &e)); }
                pieces = next;
                regions.push((e, *top));
            }
        }
        let kept_area: f32 = pieces.iter().map(area).sum();
        if area(&outline) - kept_area < 0.005 { return None; }
        let mut kept = Vec::new();
        for p in &pieces {
            if area(p) < 1e-5 { continue; }
            for i in 1..p.len() - 1 {
                let t = [low.point(p[0]), low.point(p[i]), low.point(p[i + 1])];
                // Keep the source winding (upward).
                let tn = cross(sub(t[1], t[0]), sub(t[2], t[0]));
                kept.push(if dot(tn, n) < 0. { [t[0], t[2], t[1]] } else { t });
            }
        }
        // Risers along each covered region's border where open lower floor
        // continues just outside it.
        let mut added = Vec::new();
        for (r, top) in &regions {
            let c = centroid2(r);
            for k in 0..r.len() {
                let (a, b) = (r[k], r[(k + 1) % r.len()]);
                let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
                if len < 0.02 { continue; }
                let mut o = [(b[1] - a[1]) / len, -(b[0] - a[0]) / len];
                let m = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
                if (c[0] - m[0]) * o[0] + (c[1] - m[1]) * o[1] > 0. { o = [-o[0], -o[1]]; }
                let steps = ((len / RISER_STEP).ceil() as usize).max(1);
                for s in 0..steps {
                    let (f0, f1) = (s as f32 / steps as f32, (s + 1) as f32 / steps as f32);
                    let p0 = [a[0] + (b[0] - a[0]) * f0, a[1] + (b[1] - a[1]) * f0];
                    let p1 = [a[0] + (b[0] - a[0]) * f1, a[1] + (b[1] - a[1]) * f1];
                    let pm = [(p0[0] + p1[0]) * 0.5, (p0[1] + p1[1]) * 0.5];
                    let q = [pm[0] + o[0] * 0.02, pm[1] + o[1] * 0.02];
                    let qz = low.z(q);
                    // Open floor just outside: a floor there, within 2 cm of
                    // this floor's plane, with no effective cover.
                    let open = self.near(q[0], q[1]).iter().any(|&j| {
                        let j = j as usize;
                        height_at(&self.tris[j], q[0], q[1]).is_some_and(|z| (z - qz).abs() <= 0.02)
                            && !self.covered_eff(j, q, qz, self.normals[j], raw)
                    });
                    if !open { continue; }
                    let (b0, b1) = (low.point(p0), low.point(p1));
                    let (t0, t1) = (top.point(p0), top.point(p1));
                    let (mid_low, mid_top) = ((b0[2] + b1[2]) * 0.5, (t0[2] + t1[2]) * 0.5);
                    if walls.closes([pm[0], pm[1], mid_low], mid_low + (mid_top - mid_low) * 0.25, mid_top - (mid_top - mid_low) * 0.25) { continue; }
                    for mut t in [[b0, b1, t1], [b0, t1, t0]] {
                        let tn = cross(sub(t[1], t[0]), sub(t[2], t[0]));
                        if dot(tn, tn) < 1e-10 { continue; }
                        if tn[0] * o[0] + tn[1] * o[1] < 0. { t.swap(1, 2); }
                        added.push(t);
                    }
                }
            }
        }
        Some((kept, added))
    }
}

/// Point q inside (or on) convex counter-clockwise r.
fn inside(r: &Poly, q: [f32; 2]) -> bool {
    r.len() >= 3 && (0..r.len()).all(|k| {
        let (a, b) = (r[k], r[(k + 1) % r.len()]);
        (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0]) >= -1e-6
    })
}

type Poly = Vec<[f32; 2]>;

/// z = a x + b y + d.
#[derive(Clone, Copy)]
struct Plane { a: f32, b: f32, d: f32 }

impl Plane {
    fn of(t: &Triangle) -> Option<Self> {
        let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
        if n[2].abs() < 1e-9 { return None; }
        let (a, b) = (-n[0] / n[2], -n[1] / n[2]);
        Some(Self { a, b, d: t[0][2] - a * t[0][0] - b * t[0][1] })
    }
    fn z(&self, q: [f32; 2]) -> f32 { self.a * q[0] + self.b * q[1] + self.d }
    fn point(&self, q: [f32; 2]) -> V { [q[0], q[1], self.z(q)] }
    fn minus(&self, o: &Plane) -> Plane { Plane { a: self.a - o.a, b: self.b - o.b, d: self.d - o.d } }
}

fn signed_area(p: &Poly) -> f32 {
    // Relative to the first vertex: world coordinates (thousands of metres)
    // would cancel away in f32.
    let Some(&o) = p.first() else { return 0. };
    let mut s = 0.;
    for i in 1..p.len().saturating_sub(1) {
        let (a, b) = ([p[i][0] - o[0], p[i][1] - o[1]], [p[i + 1][0] - o[0], p[i + 1][1] - o[1]]);
        s += a[0] * b[1] - b[0] * a[1];
    }
    s * 0.5
}

fn area(p: &Poly) -> f32 { signed_area(p).abs() }

fn ccw(mut p: Poly) -> Poly {
    if signed_area(&p) < 0. { p.reverse(); }
    p
}

fn centroid2(p: &Poly) -> [f32; 2] {
    let k = p.len().max(1) as f32;
    [p.iter().map(|q| q[0]).sum::<f32>() / k, p.iter().map(|q| q[1]).sum::<f32>() / k]
}

/// The part of convex polygon p where f >= 0 (f linear).
fn clip(p: &Poly, f: impl Fn([f32; 2]) -> f32) -> Poly {
    let mut out = Vec::with_capacity(p.len() + 1);
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        let (fa, fb) = (f(a), f(b));
        if fa >= 0. { out.push(a); }
        if (fa >= 0.) != (fb >= 0.) {
            let t = fa / (fa - fb);
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    // Clipping through a vertex repeats it; repeated points make zero-length
    // edges that `subtract` cannot use.
    out.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5);
    while out.len() > 1 && (out[0][0] - out[out.len() - 1][0]).abs() < 1e-5 && (out[0][1] - out[out.len() - 1][1]).abs() < 1e-5 { out.pop(); }
    if out.len() < 3 { Vec::new() } else { out }
}

/// Convex p minus convex counter-clockwise r: up to r.len() convex pieces.
fn subtract(p: &Poly, r: &Poly) -> Vec<Poly> {
    let mut out = Vec::new();
    let mut rest = p.clone();
    for k in 0..r.len() {
        if rest.is_empty() { break; }
        let (a, b) = (r[k], r[(k + 1) % r.len()]);
        if (b[0] - a[0]).abs() < 1e-5 && (b[1] - a[1]).abs() < 1e-5 { continue; }
        // Left of a->b is inside a counter-clockwise r.
        let left = |q: [f32; 2]| (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0]);
        let outside = clip(&rest, |q| -left(q));
        if area(&outside) > 1e-7 { out.push(outside); }
        rest = clip(&rest, left);
    }
    // No overlap: p stays whole.
    if rest.is_empty() || area(&rest) < 1e-7 { return vec![p.clone()]; }
    out
}

/// The tile's near-vertical faces, to find edges GTA already closes.
struct Walls {
    lo: Vec<V>,
    hi: Vec<V>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

impl Walls {
    fn new(tris: impl Iterator<Item = Triangle>) -> Self {
        let (mut lo, mut hi, mut grid) = (Vec::new(), Vec::new(), HashMap::<(i32, i32), Vec<u32>>::default());
        for t in tris {
            if unit_normal(&t)[2].abs() >= 0.6 || !t.iter().flatten().all(|x| x.is_finite()) { continue; }
            let (l, h) = bounds(&t);
            if h[0] - l[0] > 2000. || h[1] - l[1] > 2000. { continue; }
            let i = lo.len() as u32;
            for x in (l[0] / CELL).floor() as i32..=(h[0] / CELL).floor() as i32 {
                for y in (l[1] / CELL).floor() as i32..=(h[1] / CELL).floor() as i32 {
                    grid.entry((x, y)).or_default().push(i);
                }
            }
            lo.push(l);
            hi.push(h);
        }
        Self { lo, hi, grid }
    }

    /// A wall within 6 cm of p spanning heights z0..z1.
    fn closes(&self, p: V, z0: f32, z1: f32) -> bool {
        const NEAR: f32 = 0.06;
        self.grid.get(&((p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32)).is_some_and(|list| list.iter().any(|&i| {
            let (l, h) = (self.lo[i as usize], self.hi[i as usize]);
            l[0] - NEAR <= p[0] && h[0] + NEAR >= p[0] && l[1] - NEAR <= p[1] && h[1] + NEAR >= p[1] && l[2] <= z0 && h[2] >= z1
        }))
    }
}

/// Applies the rule to a decoded tile. Returns (faces split, riser triangles added).
pub fn top_floor_wins(tile: &mut Tile) -> (usize, usize) {
    let is_floor = |t: &Triangle, tag: u32| {
        tag & SPECIAL == 0 && t.iter().flatten().all(|x| x.is_finite())
            && unit_normal(t)[2] >= MIN_UP && !in_park(centroid(t))
    };
    let floor_index: Vec<Option<usize>> = {
        let mut k = 0;
        tile.triangles.iter().map(|(t, tag)| is_floor(t, *tag).then(|| { k += 1; k - 1 })).collect()
    };
    let floors = Floors::new(tile.triangles.iter().filter(|(t, tag)| is_floor(t, *tag)).map(|(t, _)| *t).collect());
    if floors.tris.len() < 2 { return (0, 0); }
    let walls = Walls::new(tile.triangles.iter().filter(|(_, tag)| tag & SPECIAL == 0).map(|(t, _)| *t));
    let has_origins = tile.origins.len() == tile.triangles.len();
    // Two passes over independent floors (covers are read only), each split
    // over a few threads: raw cover regions, then the cut. Assembled in
    // source order.
    type Result = Option<(Vec<Triangle>, Vec<Triangle>)>;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).saturating_sub(2).clamp(1, 8);
    let count = floors.tris.len();
    let chunk = count.div_ceil(threads).max(1);
    let mut raw: Vec<Vec<(Poly, Plane, usize)>> = Vec::with_capacity(count);
    std::thread::scope(|scope| {
        let floors = &floors;
        let handles: Vec<_> = (0..count).step_by(chunk).map(|start| {
            scope.spawn(move || (start..(start + chunk).min(count)).map(|own| floors.raw_regions(own)).collect::<Vec<_>>())
        }).collect();
        for h in handles { raw.extend(h.join().unwrap_or_default()); }
    });
    if raw.len() != count { return (0, 0); }
    let mut cut: Vec<Result> = vec![None; count];
    std::thread::scope(|scope| {
        let (floors, walls, raw) = (&floors, &walls, &raw);
        let handles: Vec<_> = (0..count).step_by(chunk).map(|start| {
            scope.spawn(move || (start..(start + chunk).min(count)).map(|own| (own, floors.cut(own, walls, raw))).collect::<Vec<_>>())
        }).collect();
        for h in handles {
            for (own, r) in h.join().unwrap_or_default() { cut[own] = r; }
        }
    });
    let mut results: Vec<Result> = vec![None; tile.triangles.len()];
    for (i, f) in floor_index.iter().enumerate() {
        if let Some(own) = *f { results[i] = cut[own].take(); }
    }
    let mut triangles = Vec::with_capacity(tile.triangles.len());
    let mut origins = Vec::with_capacity(if has_origins { tile.triangles.len() } else { 0 });
    let (mut dropped_n, mut risers_n, mut split_n) = (0, 0, 0);
    for (i, (&(t, tag), r)) in tile.triangles.iter().zip(results).enumerate() {
        let origin = if has_origins { tile.origins[i] } else { 0 };
        let Some((kept, added)) = r else {
            triangles.push((t, tag));
            if has_origins { origins.push(origin); }
            continue;
        };
        split_n += 1;
        dropped_n += 1;
        risers_n += added.len();
        for k in kept {
            triangles.push((k, tag));
            if has_origins { origins.push(origin); }
        }
        for w in added {
            triangles.push((w, tag | NO_GRIND));
            if has_origins { origins.push(origin); }
        }
    }
    tile.triangles = triangles;
    if has_origins { tile.origins = origins; }
    let _ = dropped_n;
    RISERS.fetch_add(risers_n, Relaxed);
    SPLIT_FACES.fetch_add(split_n, Relaxed);
    (split_n, risers_n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(tris: Vec<(Triangle, u32)>) -> Tile {
        Tile { cell: (0, 0), size: 64., source: String::new(), origins: vec![0; tris.len()], origin_names: vec!["t".into()],
               triangles: tris, rails: Vec::new(), noses: Vec::new() }
    }
    /// A square floor [x0, x1] x [y0, y1] at height z as two upward triangles.
    fn quad(x0: f32, y0: f32, x1: f32, y1: f32, z: f32) -> [Triangle; 2] {
        [[[x0, y0, z], [x1, y0, z], [x1, y1, z]], [[x0, y0, z], [x1, y1, z], [x0, y1, z]]]
    }
    fn area_up(t: &Tile, z: f32) -> f32 {
        t.triangles.iter().filter(|(t, _)| unit_normal(t)[2] > 0.7 && (t[0][2] - z).abs() < 1e-3)
            .map(|(t, _)| { let n = cross(sub(t[1], t[0]), sub(t[2], t[0])); dot(n, n).sqrt() / 2. }).sum()
    }
    // Far from every park.
    const X: f32 = 1000.;

    #[test]
    fn covered_part_of_a_large_lower_floor_goes_and_a_riser_closes_the_edge() {
        let mut tris: Vec<(Triangle, u32)> = quad(X, 0., X + 10., 10., 1.0).into_iter().map(|t| (t, 7)).collect();
        tris.extend(quad(X + 4., 0., X + 10., 10., 1.10).into_iter().map(|t| (t, 7)));
        let mut t = tile(tris);
        let (dropped, risers) = top_floor_wins(&mut t);
        assert!(dropped > 0 && risers > 0, "dropped {dropped}, risers {risers}");
        // Upper sheet untouched (60 m2); the lower floor keeps about its open 40 m2.
        assert!((area_up(&t, 1.10) - 60.).abs() < 1e-3, "upper {} lower {} n {}", area_up(&t, 1.10), area_up(&t, 1.0), t.triangles.len());
        let low = area_up(&t, 1.0);
        assert!((38.5..=41.5).contains(&low), "lower floor kept {low} m2");
        // Nothing of the lower floor is left under the sheet, beyond one leaf from its edge.
        assert!(t.triangles.iter().all(|(t, _)| !((t[0][2] - 1.0).abs() < 1e-3 && unit_normal(t)[2] > 0.7 && centroid(t)[0] > X + 4.15)));
        // Risers stand at the sheet's edge, facing the open floor, from 1.0 to 1.1 m, no grind line.
        let walls: Vec<_> = t.triangles.iter().filter(|(t, _)| unit_normal(t)[2].abs() < 0.1).collect();
        assert!(!walls.is_empty());
        assert!(walls.iter().any(|(w, _)| unit_normal(w)[0] < -0.6), "most risers face the open floor");
        for (w, tag) in &walls {
            assert_eq!(*tag, 7 | NO_GRIND);
            assert!(w.iter().all(|p| (p[0] - (X + 4.)).abs() < 0.16 && p[2] >= 0.999 && p[2] <= 1.101));
            assert!(unit_normal(w)[0] <= 0.1, "riser never faces into the sheet: {:?}", unit_normal(w));
        }
        assert_eq!(t.origins.len(), t.triangles.len());
    }

    #[test]
    fn floor_under_a_high_sheet_stays() {
        let mut tris: Vec<(Triangle, u32)> = quad(X, 0., X + 10., 10., 1.0).into_iter().map(|t| (t, 0)).collect();
        tris.extend(quad(X + 4., 0., X + 10., 10., 1.40).into_iter().map(|t| (t, 0)));
        let mut t = tile(tris.clone());
        assert_eq!(top_floor_wins(&mut t), (0, 0));
        assert_eq!(t.triangles, tris);
    }

    #[test]
    fn coplanar_layers_are_left_to_the_coplanar_pass() {
        let mut tris: Vec<(Triangle, u32)> = quad(X, 0., X + 10., 10., 1.0).into_iter().map(|t| (t, 0)).collect();
        tris.extend(quad(X + 4., 0., X + 10., 10., 1.01).into_iter().map(|t| (t, 0)));
        let mut t = tile(tris.clone());
        assert_eq!(top_floor_wins(&mut t), (0, 0));
        assert_eq!(t.triangles, tris);
    }

    #[test]
    fn parks_and_tagged_faces_are_untouched() {
        let v = crate::parks::PARKS[0].centre;
        let mut tris: Vec<(Triangle, u32)> = quad(v.x - 5., v.y - 5., v.x + 5., v.y + 5., 2.0).into_iter().map(|t| (t, 0)).collect();
        tris.extend(quad(v.x, v.y - 5., v.x + 5., v.y + 5., 2.1).into_iter().map(|t| (t, 0)));
        let mut t = tile(tris.clone());
        assert_eq!(top_floor_wins(&mut t), (0, 0));
        assert_eq!(t.triangles, tris);
        // A step-tagged floor under a sheet stays.
        let step = crate::live::STEP_TAG;
        let mut tris: Vec<(Triangle, u32)> = quad(X, 0., X + 10., 10., 1.0).into_iter().map(|t| (t, step)).collect();
        tris.extend(quad(X + 4., 0., X + 10., 10., 1.10).into_iter().map(|t| (t, 0)));
        let mut t = tile(tris.clone());
        assert_eq!(top_floor_wins(&mut t), (0, 0));
        assert_eq!(t.triangles, tris);
    }

    #[test]
    fn uncovered_faces_keep_their_identity() {
        let tris: Vec<(Triangle, u32)> = quad(X, 0., X + 10., 10., 1.0).into_iter()
            .chain(quad(X + 20., 0., X + 30., 10., 1.10)).map(|t| (t, 3)).collect();
        let mut t = tile(tris.clone());
        assert_eq!(top_floor_wins(&mut t), (0, 0));
        assert_eq!(t.triangles, tris);
    }

    #[test]
    fn thin_box_on_a_slope_leaves_no_floor_under_its_top() {
        // The canal slope (-878.8 -1038.9): a 13 m slope with an 8 cm thick
        // box lying on it (top, no bottom needed for the rule).
        let slope = |x: f32| 5.0 - 0.2 * (x - X);
        let mut tris: Vec<(Triangle, u32)> = vec![
            ([[X, 0., slope(X)], [X + 13., 0., slope(X + 13.)], [X + 13., 4., slope(X + 13.)]], 0),
            ([[X, 0., slope(X)], [X + 13., 4., slope(X + 13.)], [X, 4., slope(X)]], 0),
        ];
        tris.push(([[X + 4., 1.6, slope(X + 4.) + 0.08], [X + 9., 1.6, slope(X + 9.) + 0.08], [X + 9., 2.0, slope(X + 9.) + 0.08]], 0));
        tris.push(([[X + 4., 1.6, slope(X + 4.) + 0.08], [X + 9., 2.0, slope(X + 9.) + 0.08], [X + 4., 2.0, slope(X + 4.) + 0.08]], 0));
        let mut t = tile(tris);
        let (dropped, risers) = top_floor_wins(&mut t);
        assert!(dropped > 0 && risers > 0);
        // Under the box's middle only the box top remains.
        let under: Vec<f32> = t.triangles.iter().filter_map(|(t, _)| height_at(t, X + 6.3, 1.87)).collect();
        assert_eq!(under.len(), 1, "{under:?}");
        assert!((under[0] - (slope(X + 6.3) + 0.08)).abs() < 1e-3);
    }

    /// Real tiles: GTAXSKATE_TILES=<tiles dir> GTAXSKATE_TILES_OUT=<dir>
    /// GTAXSKATE_CELLS="x,y x,y" writes each processed tile and prints counts.
    #[test]
    #[ignore]
    fn real_tiles() {
        let (Ok(dir), Ok(out), Ok(cells)) = (std::env::var("GTAXSKATE_TILES"), std::env::var("GTAXSKATE_TILES_OUT"), std::env::var("GTAXSKATE_CELLS")) else { return };
        std::fs::create_dir_all(&out).unwrap();
        for c in cells.split_whitespace() {
            let (x, y) = c.split_once(',').unwrap();
            let cell = (x.parse().unwrap(), y.parse().unwrap());
            let path = crate::tiles::path(std::path::Path::new(&dir), cell);
            let mut t = Tile::read(std::io::BufReader::new(std::fs::File::open(&path).unwrap())).unwrap();
            let before = t.triangles.len();
            let start = std::time::Instant::now();
            let (d, r) = top_floor_wins(&mut t);
            eprintln!("tile {c}: {before} -> {} triangles, {d} faces cut, {r} riser triangles, {:.0} ms", t.triangles.len(), start.elapsed().as_secs_f64() * 1e3);
            t.write(std::fs::File::create(crate::tiles::path(std::path::Path::new(&out), cell)).unwrap()).unwrap();
        }
    }

    #[test]
    fn a_covered_cover_does_not_take_the_floor_far_below() {
        // C at 1.0, B at 1.2 over all of it, A at 1.4 over the right half.
        // B's right half goes (under A); C's right half stays (A is 40 cm up,
        // B does not count there); C's left half goes under B.
        let mut tris: Vec<(Triangle, u32)> = quad(X, 0., X + 10., 10., 1.0).into_iter().map(|t| (t, 0)).collect();
        tris.extend(quad(X, 0., X + 10., 10., 1.2).into_iter().map(|t| (t, 0)));
        tris.extend(quad(X + 5., 0., X + 10., 10., 1.4).into_iter().map(|t| (t, 0)));
        let mut t = tile(tris);
        top_floor_wins(&mut t);
        let c = area_up(&t, 1.0);
        let b = area_up(&t, 1.2);
        assert!((c - 50.).abs() < 0.5, "C keeps the right half: {c}");
        assert!((b - 50.).abs() < 0.5, "B keeps the left half: {b}");
        assert!((area_up(&t, 1.4) - 50.).abs() < 1e-3);
        let left_c = t.triangles.iter().any(|(t, _)| (t[0][2] - 1.0).abs() < 1e-3 && unit_normal(t)[2] > 0.7 && centroid(t)[0] < X + 4.9);
        assert!(!left_c, "C's left half went under B");
    }

    #[test]
    fn subtract_square_minus_half() {
        let p: Poly = ccw(vec![[0., 0.], [10., 0.], [10., 10.]]);
        let r: Poly = ccw(vec![[4., 0.], [10., 0.], [10., 10.]]);
        let out = subtract(&p, &r);
        let a: f32 = out.iter().map(area).sum();
        let r2: Poly = ccw(vec![[4., 0.], [10., 10.], [4., 10.]]);
        let out2: Vec<Poly> = out.iter().flat_map(|q| subtract(q, &r2)).collect();
        assert!((out2.iter().map(area).sum::<f32>() - 8.).abs() < 1e-3);
        assert!((a - 20.).abs() < 1e-3);
    }

}
