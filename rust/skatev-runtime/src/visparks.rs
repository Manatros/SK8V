//! High-res skateparks (docs/designs/2026-10-25-park-surfaces-from-visible.md):
//! inside the parks of `parks.rs`, GTA's coarse collision is replaced like for
//! like by the park's visible model mesh (SVVP sidecar from `visual-rails`).
//! Measured 2026-10-25 at Vespucci: GTA's bowl walls lie up to 20 cm over
//! the visible curve (13,068 cells of 10 cm off by more than 8 cm).
use bevy_math::{Vec2, Vec3};
use rustc_hash::FxHashMap as HashMap;
use std::path::Path;
use std::sync::OnceLock;
use svwc::Tri;

/// GTAxSkate HighResParks (Developer > World collision, default on).
pub static HIGH_RES_PARKS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
/// A GTA and a visible surface closer than this are the same surface (m):
/// above the measured 20 cm, below a riser pair or coping-to-deck distance.
pub const MATCH: f32 = 0.25;
/// Coverage sampling step over a GTA triangle (m).
const SAMPLE: f32 = 0.10;
/// A visible triangle replaces GTA's only where GTA's surface leaves it by
/// this much (m; the bowls' chords lie up to 20 cm off the curve) ...
const CURVE: f32 = 0.03;
/// ... and GTA's face is a surface this large (m2), not a rail or trim.
const SURFACE: f32 = 0.25;
/// Minimum local breadth (m) of an additionally admitted small bowl surface.
/// Actual coping strips reach 0.255 m; measured M2 support is 0.323/0.642 m.
const SUPPORT_WIDTH: f32 = 0.30;
/// A visible face of a curve lies within 40 deg of its GTA chord (a 45 deg
/// chamfer does not; the M2 hump between the bowls does).
const CURVE_FACING: f32 = 0.766;
/// A visible triangle covers a GTA triangle only when their normals agree this much.
const FACING: f32 = 0.5;

/// The parks' visible meshes, world space, by park id.
static FILE: OnceLock<Option<Vec<(String, Vec<[[f32; 3]; 3]>)>>> = OnceLock::new();

fn policy_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".policy");
    name.into()
}

fn policy_text(path: &Path) -> std::io::Result<String> {
    // The exporter source is the same as the visible rails'.
    let rails = crate::visrails::policy_text_for(path)?;
    Ok(rails.replacen("SVRL-POLICY 1", "SVVP-POLICY 1", 1))
}

pub fn write_policy(path: &Path) -> std::io::Result<()> {
    std::fs::write(policy_path(path), policy_text(path)?)
}

pub fn verified_policy(path: &Path) -> bool {
    policy_text(path).ok().zip(std::fs::read_to_string(policy_path(path)).ok())
        .is_some_and(|(expected, actual)| expected == actual.replace("\r\n", "\n"))
}

/// SVVP v1: magic, u32 version, u32 park count; per park: u8 id length, id
/// bytes, u32 triangle count, triangles (9 f32 world).
pub fn write(path: &Path, parks: &[(String, Vec<[[f32; 3]; 3]>)]) -> std::io::Result<()> {
    let mut b = b"SVVP".to_vec();
    b.extend(1u32.to_le_bytes());
    b.extend((parks.len() as u32).to_le_bytes());
    for (id, tris) in parks {
        b.push(id.len() as u8);
        b.extend(id.as_bytes());
        b.extend((tris.len() as u32).to_le_bytes());
        for v in tris.iter().flatten().flatten() {
            b.extend(v.to_le_bytes());
        }
    }
    std::fs::write(path, b)?;
    write_policy(path)
}

fn decode(bytes: &[u8]) -> Option<Vec<(String, Vec<[[f32; 3]; 3]>)>> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Option<&[u8]> { let s = bytes.get(at..at + n)?; at += n; Some(s) };
    if take(4)? != b"SVVP" { return None; }
    if u32::from_le_bytes(take(4)?.try_into().ok()?) != 1 { return None; }
    let parks = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
    if parks > 64 { return None; }
    let mut out = Vec::with_capacity(parks);
    for _ in 0..parks {
        let len = take(1)?[0] as usize;
        let id = String::from_utf8(take(len)?.to_vec()).ok()?;
        let n = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
        if n > 2_000_000 { return None; }
        let raw = take(n.checked_mul(36)?)?;
        let mut tris = Vec::with_capacity(n);
        for t in raw.chunks_exact(36) {
            let f = |k: usize| f32::from_le_bytes(t[k * 4..k * 4 + 4].try_into().unwrap());
            let tri = [[f(0), f(1), f(2)], [f(3), f(4), f(5)], [f(6), f(7), f(8)]];
            if !tri.iter().flatten().all(|v| v.is_finite()) { return None; }
            tris.push(tri);
        }
        out.push((id, tris));
    }
    (at == bytes.len()).then_some(out)
}

/// Reads the park meshes once; a missing, unverified or malformed file turns
/// the feature off (returns the error for the log).
pub fn load(path: &Path) -> Result<usize, String> {
    let file = FILE.get_or_init(|| {
        if !verified_policy(path) { return None; }
        decode(&std::fs::read(path).ok()?)
    });
    file.as_ref().map(|p| p.iter().map(|(_, t)| t.len()).sum())
        .ok_or_else(|| format!("high-res parks off: no verified park mesh file {}", path.display()))
}

#[cfg(test)]
fn set_for_test(parks: Vec<(String, Vec<[[f32; 3]; 3]>)>) { let _ = FILE.set(Some(parks)); }

fn v3(p: [f32; 3]) -> Vec3 { Vec3::from_array(p) }

fn normal(t: &[[f32; 3]; 3]) -> Vec3 {
    (v3(t[1]) - v3(t[0])).cross(v3(t[2]) - v3(t[0])).normalize_or_zero()
}

/// Distance from `p` to the plane of `t` when `p` projects inside `t`.
#[cfg(test)]
fn projected(p: Vec3, t: &[[f32; 3]; 3]) -> Option<f32> {
    let [a, b, c] = t.map(v3);
    let n = (b - a).cross(c - a);
    if n.length_squared() < 1e-12 { return None; }
    let n = n.normalize();
    let d = (p - a).dot(n);
    let q = p - n * d;
    let inside = [(a, b), (b, c), (c, a)].iter().all(|&(u, v)| (v - u).cross(q - u).dot(n) >= -1e-4);
    inside.then_some(d.abs())
}

fn centroid(t: &[[f32; 3]; 3]) -> Vec3 { (v3(t[0]) + v3(t[1]) + v3(t[2])) / 3.0 }

fn distance(p: Vec3, t: &[[f32; 3]; 3]) -> f32 {
    crate::world::closest_on_triangle(p, &t.map(v3)).distance(p)
}

/// Triangles by 1 m XY cell over their bounding box widened by `reach`.
struct Grid { cells: HashMap<(i32, i32), Vec<u32>> }

impl Grid {
    fn new<'a>(tris: impl Iterator<Item = &'a [[f32; 3]; 3]>, reach: f32) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::default();
        for (i, t) in tris.enumerate() {
            let lo = |k: usize| t.iter().map(|p| p[k]).fold(f32::MAX, f32::min) - reach;
            let hi = |k: usize| t.iter().map(|p| p[k]).fold(f32::MIN, f32::max) + reach;
            for x in lo(0).floor() as i32..=hi(0).floor() as i32 {
                for y in lo(1).floor() as i32..=hi(1).floor() as i32 {
                    cells.entry((x, y)).or_default().push(i as u32);
                }
            }
        }
        Grid { cells }
    }
    fn at(&self, p: Vec3) -> &[u32] {
        self.cells.get(&(p.x.floor() as i32, p.y.floor() as i32)).map_or(&[], |v| v.as_slice())
    }
}

/// Points on a triangle every `SAMPLE` m (barycentric grid), plus its corners
/// pulled 2 % inward.
fn samples(t: &[[f32; 3]; 3]) -> Vec<Vec3> {
    let p = t.map(v3);
    let c = (p[0] + p[1] + p[2]) / 3.0;
    let longest = p[0].distance(p[1]).max(p[1].distance(p[2])).max(p[2].distance(p[0]));
    let n = ((longest / SAMPLE).ceil() as usize).clamp(1, 120);
    let mut out: Vec<Vec3> = p.iter().map(|&q| q + (c - q) * 0.02).collect();
    // Edges every 2 cm, 1 cm inward: a GTA edge a few centimetres past the
    // visible one must not count as covered (2026-10-26 park bakes: strips
    // under 10 cm between the grid samples lost their floor).
    for k in 0..3 {
        let (a, b) = (p[k], p[(k + 1) % 3]);
        let steps = ((a.distance(b) / 0.02).ceil() as usize).clamp(1, 2000);
        for s in 0..=steps {
            let q = a.lerp(b, s as f32 / steps as f32);
            let inward = (c - q).normalize_or_zero() * 0.01;
            out.push(q + inward);
        }
    }
    for i in 0..=n {
        for j in 0..=n - i {
            let (a, b) = (i as f32 / n as f32, j as f32 / n as f32);
            out.push(p[0] + (p[1] - p[0]) * a + (p[2] - p[0]) * b);
        }
    }
    out
}

/// What replacing did in one area.
#[derive(Default)]
pub struct Replacement {
    #[cfg(test)]
    support_work: (usize, usize, usize), // queries, total visited, worst query
    /// GTA triangles replaced (kept as the HighResParks-off variant).
    pub removed: Vec<Tri>,
    /// Visible triangles added, then the cut GTA pieces and moved copies.
    pub added: Vec<Tri>,
    /// Per added triangle, the vertex key of the GTA triangle whose surface
    /// record (audio, seam) it takes.
    pub surface_from: Vec<u64>,
    /// Per added triangle: a piece (or moved copy) of a GTA triangle, not a
    /// visible one.
    pub cut: Vec<bool>,
    /// Per added triangle: the park id (origin names, design 6.2).
    pub park: Vec<&'static str>,
}

/// Replaces GTA park surfaces in `tris` by the parks' visible meshes
/// (design 2026-10-26-one-source-collision 1). `stairs` are the area's stair
/// ramps and steps: never cut, and no visible triangle near them is used.
pub fn replace(tris: &mut Vec<Tri>, stairs: &[Tri]) -> Replacement {
    let Some(Some(parks)) = FILE.get() else { return Replacement::default() };
    let mut out = Replacement::default();
    for park in crate::parks::PARKS {
        let Some((_, visible)) = parks.iter().find(|(id, _)| id == park.id) else { continue };
        replace_park(tris, stairs, visible, park.centre, park.radius, park.id, &mut out);
    }
    out
}

/// Smallest piece of a cut GTA triangle that is kept (m2), and the thinnest
/// (area / longest edge, m): nothing stands on less, and the gap is smaller.
const PIECE_AREA: f32 = 2e-4;
const PIECE_WIDTH: f32 = 0.003;
/// A vertex this close to a visible footprint edge moves onto it (m).
const ON_EDGE: f32 = 0.001;
/// A GTA surface counts as a visible triangle's own when it lies within the
/// triangle's distance to its partner plus this (m).
const PARTNER_SLACK: f32 = 0.05;

/// A visible triangle in use: its GTA partner (nearest same-facing park
/// triangle) and how far from it a GTA surface still is the same surface.
struct Used { v: [[f32; 3]; 3], partner: usize, limit: f32 }

/// Local surface support, independent of source triangulation size. Only full
/// manifold edges connect; quantization identifies topology, never moves a face.
/// See docs/designs/2026-10-28-connected-park-surfaces.md.
struct SurfaceSupport {
    faces: Vec<[Vec3; 3]>,
    normals: Vec<Vec3>,
    links: Vec<Vec<(usize, [Vec3; 2])>>,
    indices: HashMap<usize, usize>,
    seen: Vec<u32>,
    epoch: u32,
    stack: Vec<usize>,
    points: Vec<Vec2>,
    hull: Vec<Vec2>,
    #[cfg(test)]
    work: (usize, usize, usize),
}

impl SurfaceSupport {
    fn new(tris: &[Tri], park: &[usize]) -> Self {
        type Point = [i32; 3];
        let faces: Vec<_> = park.iter().map(|&i| tris[i].v.map(v3)).collect();
        let normals: Vec<_> = park.iter().map(|&i| normal(&tris[i].v)).collect();
        let mut edges = std::collections::BTreeMap::<(Point, Point), Vec<(usize, [Vec3; 2])>>::new();
        for (i, face) in faces.iter().enumerate() {
            if normals[i] == Vec3::ZERO || !face.iter().all(|p| p.is_finite()) { continue; }
            let q = face.map(|p| p.to_array().map(|c| (c * 1e4).round() as i32));
            for k in 0..3 {
                let j = (k + 1) % 3;
                if q[k] == q[j] { continue; }
                let key = if q[k] < q[j] { (q[k], q[j]) } else { (q[j], q[k]) };
                edges.entry(key).or_default().push((i, [face[k], face[j]]));
            }
        }
        let mut links = vec![Vec::new(); faces.len()];
        for owners in edges.values() {
            if let [(a, edge_a), (b, edge_b)] = owners.as_slice() {
                if a != b {
                    links[*a].push((*b, *edge_a));
                    links[*b].push((*a, *edge_b));
                }
            }
        }
        Self { seen: vec![0; faces.len()], faces, normals, links,
            indices: park.iter().enumerate().map(|(local, &global)| (global, local)).collect(),
            epoch: 0, stack: Vec::new(), points: Vec::new(), hull: Vec::new(),
            #[cfg(test)] work: (0, 0, 0) }
    }

    fn qualifies(&mut self, global: usize, visible_centre: Vec3) -> bool {
        #[cfg(test)] { self.work.0 += 1; }
        #[cfg(test)] let mut visited = 0;
        let seed = self.indices[&global];
        let n = self.normals[seed];
        if n == Vec3::ZERO || !n.is_finite() { return false; }
        let centre = crate::world::closest_on_triangle(visible_centre, &self.faces[seed]);
        let a = n.abs();
        let axis = if a.x <= a.y && a.x <= a.z { Vec3::X } else if a.y <= a.z { Vec3::Y } else { Vec3::Z };
        let u = n.cross(axis).normalize();
        let frame = [u, n.cross(u), n];
        let local = |p: Vec3| { let d = p - centre; Vec3::new(d.dot(frame[0]), d.dot(frame[1]), d.dot(frame[2])) };
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 { self.seen.fill(0); self.epoch = 1; }
        self.stack.clear();
        self.points.clear();
        self.hull.clear();
        self.stack.push(seed);
        self.seen[seed] = self.epoch;
        let mut total = 0.0f64;
        let half = SURFACE.sqrt();
        while let Some(i) = self.stack.pop() {
            #[cfg(test)] { visited += 1; self.work.1 += 1; self.work.2 = self.work.2.max(visited); }
            let (poly, len) = clipped_support_polygon(self.faces[i].map(&local), half);
            let area = support_polygon_area(&poly[..len]);
            if area <= 0.0 { continue; }
            total += area as f64;
            self.points.extend(poly[..len].iter().map(|p| p.truncate()));
            if total >= SURFACE as f64 && support_min_width(&mut self.points, &mut self.hull) >= SUPPORT_WIDTH { return true; }
            for &(j, edge) in &self.links[i] {
                // An unsuccessful edge must not prevent a later valid route.
                if self.seen[j] == self.epoch || self.normals[j].dot(n) < CURVE_FACING
                    || self.normals[j].dot(self.normals[i]) < CURVE_FACING
                    || !support_edge_intersects(edge.map(&local), half) { continue; }
                self.seen[j] = self.epoch;
                self.stack.push(j);
            }
        }
        false
    }

    fn adjacent_partner(&mut self, global: usize, d: f32, visible_centre: Vec3, visible_n: Vec3, park: &[usize]) -> Option<usize> {
        let seed = *self.indices.get(&global)?;
        let mut best: Option<(f32, usize)> = None;
        for &(j, edge) in &self.links[seed] {
            let nj = self.normals[j];
            if nj.z <= 0.5 || visible_n.dot(nj) < CURVE_FACING { continue; }
            let dj = crate::world::closest_on_triangle(visible_centre, &self.faces[j]).distance(visible_centre);
            if dj > MATCH || dj > d + PARTNER_SLACK || segment_distance(visible_centre, edge) > MATCH { continue; }
            let candidate = (dj, park[j]);
            if best.is_none_or(|b| candidate.0.total_cmp(&b.0).then(candidate.1.cmp(&b.1)).is_lt()) {
                best = Some(candidate);
            }
        }
        let (_, partner) = best?;
        self.qualifies(partner, visible_centre).then_some(partner)
    }
}

fn segment_distance(p: Vec3, [a, b]: [Vec3; 2]) -> f32 {
    let e = b - a;
    let len2 = e.length_squared();
    let t = if len2 > 1e-12 { ((p - a).dot(e) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (a + e * t).distance(p)
}

fn support_edge_intersects([a, b]: [Vec3; 2], half: f32) -> bool {
    let d = b - a;
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for k in 0..3 {
        if d[k].abs() < 1e-12 {
            if a[k].abs() > half { return false; }
        } else {
            let (s, t) = ((-half - a[k]) / d[k], (half - a[k]) / d[k]);
            lo = lo.max(s.min(t)); hi = hi.min(s.max(t));
            if lo > hi { return false; }
        }
    }
    true
}

fn clipped_support_polygon(face: [Vec3; 3], half: f32) -> ([Vec3; 12], usize) {
    // A triangle clipped by six planes has at most nine corners.
    let mut poly = [Vec3::ZERO; 12];
    poly[..3].copy_from_slice(&face);
    let mut len = 3;
    for k in 0..3 {
        for sign in [-1.0f32, 1.0] {
            if len < 3 { return (poly, 0); }
            let mut next = [Vec3::ZERO; 12];
            let mut count = 0;
            for j in 0..len {
                let (a, b) = (poly[j], poly[(j + 1) % len]);
                let (fa, fb) = (half - sign * a[k], half - sign * b[k]);
                if fa >= 0.0 { next[count] = a; count += 1; }
                if (fa >= 0.0) != (fb >= 0.0) {
                    next[count] = a + (b - a) * (fa / (fa - fb)); count += 1;
                }
            }
            poly = next; len = count;
        }
    }
    (poly, len)
}

fn support_polygon_area(poly: &[Vec3]) -> f32 {
    (1..poly.len().saturating_sub(1)).map(|i| (poly[i] - poly[0]).cross(poly[i + 1] - poly[0]).length() * 0.5).sum()
}

#[cfg(test)]
fn clipped_support_area(face: [Vec3; 3], half: f32) -> f32 {
    let (poly, len) = clipped_support_polygon(face, half);
    support_polygon_area(&poly[..len])
}

/// Minimum caliper width of the projected clipped support. Area alone admits
/// long coping strips, especially along the cube's diagonal.
fn support_min_width(points: &mut Vec<Vec2>, hull: &mut Vec<Vec2>) -> f32 {
    points.sort_unstable_by(|a,b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup();
    hull.clear();
    if points.len() < 3 { return 0.0; }
    let cross = |a: Vec2,b: Vec2,c: Vec2| (b-a).perp_dot(c-a);
    for &p in points.iter() {
        while hull.len() >= 2 && cross(hull[hull.len()-2],hull[hull.len()-1],p) <= 0.0 { hull.pop(); }
        hull.push(p);
    }
    let lower = hull.len();
    for &p in points.iter().rev().skip(1) {
        while hull.len() > lower && cross(hull[hull.len()-2],hull[hull.len()-1],p) <= 0.0 { hull.pop(); }
        hull.push(p);
    }
    hull.pop(); // repeated first point
    if hull.len() < 3 { return 0.0; }
    let mut width = f32::MAX;
    for i in 0..hull.len() {
        let e = hull[(i+1)%hull.len()] - hull[i];
        if e.length_squared() == 0.0 { continue; }
        let axis = Vec2::new(-e.y,e.x).normalize();
        let (mut lo, mut hi) = (f32::MAX,f32::MIN);
        for &p in hull.iter() { let d = p.dot(axis); lo = lo.min(d); hi = hi.max(d); }
        width = width.min(hi-lo);
    }
    if width == f32::MAX { 0.0 } else { width }
}

fn replace_park(tris: &mut Vec<Tri>, stairs: &[Tri], visible: &[[[f32; 3]; 3]], centre: Vec2, radius: f32, id: &'static str, out: &mut Replacement) {
    let inside = |t: &[[f32; 3]; 3]| centroid(t).truncate().distance(centre) <= radius;
    let park: Vec<usize> = (0..tris.len()).filter(|&i| inside(&tris[i].v)).collect();
    if park.is_empty() { return; }
    // Visible triangles near this area only.
    let lo = |k: usize| park.iter().flat_map(|&i| tris[i].v).map(|p| p[k]).fold(f32::MAX, f32::min) - 1.0;
    let hi = |k: usize| park.iter().flat_map(|&i| tris[i].v).map(|p| p[k]).fold(f32::MIN, f32::max) + 1.0;
    let (x0, x1, y0, y1) = (lo(0), hi(0), lo(1), hi(1));
    let near: Vec<&[[f32; 3]; 3]> = visible.iter()
        .filter(|t| inside(t) && { let c = centroid(t); c.x >= x0 && c.x <= x1 && c.y >= y0 && c.y <= y1 })
        .collect();
    if near.is_empty() { return; }
    let gta_grid = Grid::new(park.iter().map(|&i| &tris[i].v), MATCH);
    let stair_grid = Grid::new(stairs.iter().map(|t| &t.v), MATCH);
    // 1.1: used visible triangles: near a GTA park triangle facing the same
    // way, and with no sample near a stair (a large visible triangle reaching
    // over a flight would lie on its steps).
    let nearest_gta = |p: Vec3| gta_grid.at(p).iter().map(|&k| park[k as usize])
        .map(|i| (distance(p, &tris[i].v), i)).min_by(|a, b| a.0.total_cmp(&b.0));
    // Candidates: near a GTA park surface facing nearly the same way (40
    // deg: a curve's facets do, a ledge's 45 deg chamfer does not), clear of
    // stairs, not facing down.
    let mut support = None;
    let candidates: Vec<(Used, usize)> = near.iter().filter_map(|t| {
        let c = centroid(t);
        let (d, i) = nearest_gta(c)?;
        let partner_n = normal(&tris[i].v);
        if d > MATCH || normal(t).z < -0.2 { return None; }
        if normal(t).dot(partner_n) < CURVE_FACING {
            let gta = tris[i].v.map(v3);
            let area = (gta[1] - gta[0]).cross(gta[2] - gta[0]).length() / 2.0;
            if area >= SURFACE || partner_n.z <= 0.5 || normal(t).z <= 0.5 { return None; }
        }
        let by_stairs = samples(t).into_iter()
            .any(|p| stair_grid.at(p).iter().any(|&k| distance(p, &stairs[k as usize].v) <= MATCH));
        if by_stairs { return None; }
        let partner = if normal(t).dot(partner_n) >= CURVE_FACING {
            i
        } else {
            support.get_or_insert_with(|| SurfaceSupport::new(tris, &park))
                .adjacent_partner(i, d, c, normal(t), &park)?
        };
        // How far this visible triangle lies from its own GTA surface: only
        // GTA surfaces that close (plus PARTNER_SLACK) are the same surface.
        // A ground 20 cm under a visible ledge top is not (its partner is
        // the GTA ledge top a few centimetres off). When `partner` was
        // transferred across a shared edge from a tilted small chord `i`, use
        // the larger of the two plane deviations so the displaced chord still
        // qualifies as a curve and is cut by the visible surface.
        let dev = |k: usize| {
            let a = v3(tris[k].v[0]);
            let n = normal(&tris[k].v);
            t.iter().map(|&p| (v3(p) - a).dot(n).abs()).fold(0.0f32, f32::max)
        };
        let deviation = dev(i).max(dev(partner));
        Some((Used { v: **t, partner, limit: (deviation + PARTNER_SLACK).min(MATCH) }, i))
    }).collect();
    // GTA's collision is the base (Manatros 2026-10-26 19:03 recording:
    // "plain GTA collision is a solid basis that just needs fine-tuning like
    // the high-res bowl"). Only the bowls' curves replace it, a whole GTA
    // face at a time: a GTA surface (not a rail, post or trim) that the
    // visible one leaves by 3 cm or more somewhere. Flat decks, ledges and
    // railings stay GTA's (their visible copies cut GTA's into dense
    // fragments, marks 5, 7, 8).
    let mut deviates: HashMap<usize, f32> = HashMap::default();
    for &(ref u, chord) in &candidates {
        let e = deviates.entry(chord).or_insert(0.0);
        *e = e.max(u.limit - PARTNER_SLACK);
    }
    let used: Vec<Used> = candidates.into_iter().filter_map(|(u, chord)| {
        let d = deviates[&chord].max(deviates.get(&u.partner).copied().unwrap_or(0.0));
        if d < CURVE { return None; }
        // Rescue upward bowl facets only. Admitting small sidewall details
        // exposes old cut/snap remnants as sloping phantom surfaces (native
        // witnesses in the design); their boundary ownership needs its own fix.
        if u.partner == chord {
            let gta = tris[u.partner].v.map(v3);
            let area = (gta[1] - gta[0]).cross(gta[2] - gta[0]).length() / 2.0;
            if area < SURFACE && !(normal(&tris[u.partner].v).z > 0.5 && normal(&u.v).z > 0.5
                && support.get_or_insert_with(|| SurfaceSupport::new(tris, &park))
                    .qualifies(u.partner, centroid(&u.v))) {
                return None;
            }
        }
        // The whole face's deviation is the limit: the parts of the chord
        // close to the curve go with it.
        Some(Used { limit: (d + PARTNER_SLACK).min(MATCH), ..u })
    }).collect();
    #[cfg(test)]
    if let Some(support) = &support { out.support_work = support.work; }
    if used.is_empty() { return; }
    let vis_grid = Grid::new(used.iter().map(|u| &u.v), MATCH);
    // 1.2/1.4: GTA triangles near a used visible one facing the same way are
    // cut by the visible footprints, and the vertices on the footprints'
    // edges move onto them.
    let mut changed: Vec<(usize, Vec<[[f32; 3]; 3]>)> = Vec::new();
    let mut moved: HashMap<[i32; 3], [f32; 3]> = HashMap::default();
    let quant = |p: [f32; 3]| p.map(|c| (c * 1e4).round() as i32);
    let mut cover: Vec<u32> = Vec::new();
    for i in 0..tris.len() {
        let t = tris[i].v;
        let n = normal(&t);
        if n == Vec3::ZERO { continue; }
        let (tlo, thi) = bounds(&t, MATCH);
        cover.clear();
        for x in tlo.x.floor() as i32..=thi.x.floor() as i32 {
            for y in tlo.y.floor() as i32..=thi.y.floor() as i32 {
                cover.extend(vis_grid.cells.get(&(x, y)).map_or(&[][..], |v| v.as_slice()));
            }
        }
        cover.sort_unstable();
        cover.dedup();
        cover.retain(|&k| {
            let u = &used[k as usize];
            let (vlo, vhi) = bounds(&u.v, u.limit);
            vlo.cmple(thi).all() && vhi.cmpge(tlo).all() && normal(&u.v).dot(n) > FACING
        });
        if cover.is_empty() { continue; }
        let pieces = cut(&t, cover.iter().map(|&k| &used[k as usize]));
        if pieces.len() == 1 && pieces[0] == t { continue; }
        // Snapping can squeeze a piece into a sliver: those go too (they
        // broke the lip test of a park curb, 2026-10-26).
        let snapped: Vec<[[f32; 3]; 3]> = pieces.iter().map(|p| p.map(|q| snap(q, &used, &vis_grid)))
            .filter(|t| {
                let p = t.map(v3);
                let area = (p[1] - p[0]).cross(p[2] - p[0]).length() / 2.0;
                let longest = p[0].distance(p[1]).max(p[1].distance(p[2])).max(p[2].distance(p[0]));
                area >= PIECE_AREA && area / longest >= PIECE_WIDTH
            })
            .collect();
        for &corner in &t {
            let s = snap(corner, &used, &vis_grid);
            if s != corner { moved.insert(quant(corner), s); }
        }
        changed.push((i, snapped));
    }
    if changed.is_empty() { return; }
    // Neighbours sharing a moved corner move it too (no crack at the seam).
    let done: std::collections::HashSet<usize> = changed.iter().map(|c| c.0).collect();
    for i in 0..tris.len() {
        if done.contains(&i) { continue; }
        let t = tris[i].v;
        if !t.iter().any(|&p| moved.contains_key(&quant(p))) { continue; }
        changed.push((i, vec![t.map(|p| moved.get(&quant(p)).copied().unwrap_or(p))]));
    }
    for u in &used {
        out.added.push(Tri { v: u.v, material: tris[u.partner].material });
        out.surface_from.push(svwc::surfaces::key(&tris[u.partner].v));
        out.cut.push(false);
        out.park.push(id);
    }
    let mut replaced = vec![false; tris.len()];
    for (i, pieces) in changed {
        replaced[i] = true;
        let key = svwc::surfaces::key(&tris[i].v);
        for p in pieces {
            out.added.push(Tri { v: p, material: tris[i].material });
            out.surface_from.push(key);
            out.cut.push(true);
            out.park.push(id);
        }
    }
    let mut index = 0;
    tris.retain(|t| {
        let drop = replaced[index];
        index += 1;
        if drop { out.removed.push(*t); }
        !drop
    });
}

fn bounds(t: &[[f32; 3]; 3], widen: f32) -> (Vec3, Vec3) {
    let p = t.map(v3);
    (p[0].min(p[1]).min(p[2]) - Vec3::splat(widen), p[0].max(p[1]).max(p[2]) + Vec3::splat(widen))
}

/// `t` minus the regions the visible triangles `cover` take from it: each
/// visible triangle projected along its own normal onto `t`'s plane, where
/// it lies within `MATCH` of that plane (a convex region). The rest comes back
/// as triangles in `t`'s plane; pieces too small to stand on are dropped.
fn cut<'a>(t: &[[f32; 3]; 3], cover: impl Iterator<Item = &'a Used>) -> Vec<[[f32; 3]; 3]> {
    let [a, b, c] = t.map(v3);
    let n = (b - a).cross(c - a).normalize();
    let eu = (b - a).normalize();
    let ew = n.cross(eu);
    let to2 = |p: Vec3| Vec2::new((p - a).dot(eu), (p - a).dot(ew));
    let to3 = |q: Vec2| a + eu * q.x + ew * q.y;
    let corners = [to2(a), to2(b), to2(c)];
    let face_area = polygon_area(&corners);
    let mut pieces: Vec<Vec<Vec2>> = vec![corners.to_vec()];
    let mut was_cut = false;
    for u in cover {
        let (v, limit) = (&u.v, u.limit);
        let nv = normal(v);
        let along = nv.dot(n);
        if along <= FACING { continue; }
        // Visible corners projected along nv onto t's plane.
        let x: Vec<Vec2> = v.iter().map(|&p| { let p = v3(p); to2(p - nv * ((p - a).dot(n) / along)) }).collect();
        // Half-planes (point, inward normal) of the region: the projected
        // triangle's edges, then |signed distance along nv| <= MATCH.
        let ccw = (x[1] - x[0]).perp_dot(x[2] - x[0]);
        if ccw.abs() < 1e-9 { continue; }
        let mut planes: Vec<(Vec2, Vec2, f32)> = (0..3).map(|k| {
            let (p, q) = (x[k], x[(k + 1) % 3]);
            let e = q - p;
            let inward = if ccw > 0.0 { Vec2::new(-e.y, e.x) } else { Vec2::new(e.y, -e.x) };
            (inward, p, 0.0)
        }).collect();
        // s(X) = (v0 - X) . nv for X on t's plane: affine in the 2D point.
        let v0 = v3(v[0]);
        let s0 = (v0 - a).dot(nv);
        let g = Vec2::new(-eu.dot(nv), -ew.dot(nv));
        // s0 + g.X <= limit  and  -(s0 + g.X) <= limit, as inward form c + m.X >= 0.
        planes.push((-g, Vec2::ZERO, limit - s0));
        planes.push((g, Vec2::ZERO, limit + s0));
        let mut next = Vec::new();
        for poly in pieces {
            // Untouched polygons stay whole (no needless split).
            let mut probe = poly.clone();
            for (k, &(m, p, c)) in planes.iter().enumerate() {
                probe = clip(&probe, |q| if k < 3 { m.dot(q - p) } else { c + m.dot(q) });
                if probe.len() < 3 { break; }
            }
            if probe.len() < 3 || polygon_area(&probe) < 1e-8 { next.push(poly); continue; }
            was_cut = true;
            let mut rest = poly;
            for (k, &(m, p, c)) in planes.iter().enumerate() {
                let inside = |q: Vec2| if k < 3 { m.dot(q - p) } else { c + m.dot(q) };
                let outside_piece = clip(&rest, |q| -inside(q));
                if outside_piece.len() >= 3 { next.push(outside_piece); }
                rest = clip(&rest, inside);
                if rest.len() < 3 { break; }
            }
        }
        pieces = next;
        if pieces.is_empty() { break; }
    }
    let mut out = Vec::new();
    for poly in pieces {
        // Cut remnants (design 2026-10-31 remnant cleanup): clipping leaves
        // near-duplicate and collinear corners whose fan made 100-300 tiny
        // triangles per railing mark (M23-M26) and the bowl connector (M21/22);
        // the same surface with clean corners (GTA's own corners always stay).
        // A remnant narrower than REMNANT_WIDTH that is only a small part of
        // its face is a strip along the visible edge, not a surface; a long
        // thin face the visible mesh barely touched keeps its remnant.
        let poly = if was_cut { simplify(&poly, &corners) } else { poly };
        if poly.len() < 3 { continue; }
        if was_cut && convex_width(&poly) < REMNANT_WIDTH && polygon_area(&poly) <= REMNANT_SHARE * face_area { continue; }
        for k in 1..poly.len() - 1 {
            let tri = [to3(poly[0]), to3(poly[k]), to3(poly[k + 1])];
            let area = (tri[1] - tri[0]).cross(tri[2] - tri[0]).length() / 2.0;
            let longest = tri[0].distance(tri[1]).max(tri[1].distance(tri[2])).max(tri[2].distance(tri[0]));
            if area < PIECE_AREA || area / longest < PIECE_WIDTH { continue; }
            out.push(tri.map(|p| p.to_array()));
        }
    }
    // An uncut triangle comes back exactly as it was.
    if out.len() == 1 && pieces_equal(&out[0], t) { return vec![*t]; }
    out
}

/// Narrowest cut remnant kept (m): a strip below this beside the visible
/// surface is a gap no wheel can catch, not a surface to ride.
const REMNANT_WIDTH: f32 = 0.02;
/// ... and that is at most this share of its original face's area.
const REMNANT_SHARE: f32 = 0.25;
/// Corners closer than this to a neighbour or to the line through their
/// neighbours carry no shape (m).
const CORNER: f32 = 0.001;

/// A convex remnant without near-duplicate or collinear cut corners; the
/// face's own corners are kept exactly (its neighbours share them).
fn simplify(poly: &[Vec2], keep: &[Vec2]) -> Vec<Vec2> {
    let mut p = poly.to_vec();
    loop {
        let n = p.len();
        if n < 3 { return p; }
        let drop = (0..n).find(|&k| {
            if keep.iter().any(|q| q.distance(p[k]) < 1e-6) { return false; }
            let (a, b, c) = (p[(k + n - 1) % n], p[k], p[(k + 1) % n]);
            let ac = c - a;
            a.distance(b) < CORNER || ac.length() < CORNER || ((b - a).perp_dot(ac) / ac.length()).abs() < CORNER
        });
        match drop { Some(k) => { p.remove(k); } None => return p }
    }
}

/// Width of a convex polygon: the smallest of its edge-to-farthest-corner distances.
fn convex_width(poly: &[Vec2]) -> f32 {
    let n = poly.len();
    (0..n).map(|k| {
        let (a, e) = (poly[k], poly[(k + 1) % n] - poly[k]);
        let len = e.length().max(1e-12);
        poly.iter().map(|&q| (e.perp_dot(q - a) / len).abs()).fold(0.0, f32::max)
    }).fold(f32::INFINITY, f32::min)
}

fn pieces_equal(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> bool {
    a.iter().zip(b).all(|(p, q)| v3(*p).distance(v3(*q)) < 1e-5)
}

fn polygon_area(poly: &[Vec2]) -> f32 {
    (1..poly.len().saturating_sub(1)).map(|k| (poly[k] - poly[0]).perp_dot(poly[k + 1] - poly[0])).sum::<f32>().abs() / 2.0
}

/// Convex polygon `poly` clipped to `f(q) >= 0` (Sutherland-Hodgman, one plane).
fn clip(poly: &[Vec2], f: impl Fn(Vec2) -> f32) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(poly.len() + 1);
    for k in 0..poly.len() {
        let (p, q) = (poly[k], poly[(k + 1) % poly.len()]);
        let (fp, fq) = (f(p), f(q));
        if fp >= 0.0 { out.push(p); }
        if (fp >= 0.0) != (fq >= 0.0) {
            out.push(p + (q - p) * (fp / (fp - fq)));
        }
    }
    out
}

/// A vertex on the edge of a used visible triangle's footprint (projected
/// along that triangle's normal, within `MATCH` of its plane) moves onto it;
/// the smallest move wins. Depends only on the point, so every triangle
/// sharing the vertex moves it the same way.
fn snap(p: [f32; 3], used: &[Used], grid: &Grid) -> [f32; 3] {
    let x = v3(p);
    let mut best: Option<(f32, Vec3)> = None;
    for &k in grid.at(x) {
        let u = &used[k as usize];
        let v = u.v.map(v3);
        let nv = (v[1] - v[0]).cross(v[2] - v[0]).normalize_or_zero();
        if nv == Vec3::ZERO { continue; }
        let s = (v[0] - x).dot(nv);
        if s.abs() > u.limit { continue; }
        let q = x + nv * s;
        // On the triangle (closed) and within ON_EDGE of one of its edges.
        let inside = (0..3).all(|e| (v[(e + 1) % 3] - v[e]).cross(q - v[e]).dot(nv) >= -ON_EDGE * v[(e + 1) % 3].distance(v[e]));
        if !inside { continue; }
        let edge = (0..3).map(|e| {
            let (a, b) = (v[e], v[(e + 1) % 3]);
            let t = ((q - a).dot(b - a) / (b - a).length_squared().max(1e-12)).clamp(0.0, 1.0);
            (a + (b - a) * t).distance(q)
        }).fold(f32::MAX, f32::min);
        if edge > ON_EDGE { continue; }
        if best.is_none_or(|(d, _)| s.abs() < d) { best = Some((s.abs(), q)); }
    }
    match best {
        Some((d, q)) if d > 1e-5 => q.to_array(),
        _ => p,
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    const AT: Vec2 = Vec2::new(-1374., -1396.);

    fn run(gta: &mut Vec<Tri>, stairs: &[Tri], visible: &[[[f32; 3]; 3]]) -> Replacement {
        let mut out = Replacement::default();
        replace_park(gta, stairs, visible, AT, 60., "vespucci", &mut out);
        out
    }

    /// A strip of a bowl wall: the visible curve z = 0.5 x^2 for x 0..1 (fine),
    /// and GTA's single chord over it (up to 12 cm above), 2 m wide in y.
    fn wall() -> (Vec<Tri>, Vec<[[f32; 3]; 3]>) {
        let x0 = -1355.0;
        let y0 = -1396.0;
        let curve = |x: f32| 0.5 * x * x;
        let mut visible = Vec::new();
        let n = 10;
        for k in 0..n {
            let (a, b) = (k as f32 / n as f32, (k + 1) as f32 / n as f32);
            let p = |x: f32, y: f32| [x0 + x, y0 + y, curve(x)];
            visible.push([p(a, 0.), p(b, 0.), p(b, 2.)]);
            visible.push([p(a, 0.), p(b, 2.), p(a, 2.)]);
        }
        let gta = vec![
            Tri { v: [[x0, y0, 0.], [x0 + 1., y0, 0.5], [x0 + 1., y0 + 2., 0.5]], material: 1 },
            Tri { v: [[x0, y0, 0.], [x0 + 1., y0 + 2., 0.5], [x0, y0 + 2., 0.]], material: 1 },
        ];
        (gta, visible)
    }

    fn area(t: &[[f32; 3]; 3]) -> f32 {
        let p = t.map(v3);
        (p[1] - p[0]).cross(p[2] - p[0]).length() / 2.0
    }

    /// Square floor x0..x1, y0..y1 at height z as two triangles.
    fn square(x0: f32, x1: f32, y0: f32, y1: f32, z: f32) -> Vec<[[f32; 3]; 3]> {
        vec![[[x0, y0, z], [x1, y0, z], [x1, y1, z]], [[x0, y0, z], [x1, y1, z], [x0, y1, z]]]
    }

    /// Layered area: points on GTA triangles that lie under/over a visible
    /// added triangle within 30 cm (the 2026-10-26 overlap measure).
    fn layered(gta: &[Tri], added: &[Tri], cut: &[bool]) -> usize {
        let visible: Vec<&Tri> = added.iter().zip(cut).filter(|(_, c)| !**c).map(|(t, _)| t).collect();
        let pieces: Vec<&Tri> = gta.iter().chain(added.iter().zip(cut).filter(|(_, c)| **c).map(|(t, _)| t)).collect();
        let mut hits = 0;
        for g in pieces {
            for p in samples(&g.v) {
                if visible.iter().any(|v| projected(p, &v.v).is_some_and(|d| d > 0.005 && d < 0.3) && normal(&v.v).dot(normal(&g.v)) > 0.9) {
                    hits += 1;
                }
            }
        }
        hits
    }

    #[test]
    fn support_hull_measures_breadth_without_counting_duplicates_or_collinear_points() {
        let mut hull = Vec::new();
        for mut points in [vec![],vec![Vec2::ZERO],vec![Vec2::ZERO,Vec2::X,Vec2::X*2.,Vec2::X]] {
            assert_eq!(support_min_width(&mut points, &mut hull),0.0);
        }
        for angle in [0.0f32,0.37,std::f32::consts::FRAC_PI_4,1.2] {
            let rotate = |p: Vec2| Vec2::new(p.x*angle.cos()-p.y*angle.sin(),p.x*angle.sin()+p.y*angle.cos());
            let mut points = vec![Vec2::new(-2.,-0.1),Vec2::new(2.,-0.1),Vec2::new(2.,0.1),Vec2::new(-2.,0.1),Vec2::ZERO].into_iter().map(rotate).collect::<Vec<_>>();
            points.push(points[0]);
            assert!((support_min_width(&mut points,&mut hull)-0.2).abs()<1e-5);
            points.extend([rotate(Vec2::new(-1.,-0.4)),rotate(Vec2::new(1.,0.4))]);
            assert!(support_min_width(&mut points,&mut hull)>SUPPORT_WIDTH, "later support adds breadth");
        }
    }

    #[test]
    fn connected_support_can_reach_a_broad_patch_after_a_narrow_neck() {
        let xs=[-1.0f32,-0.3,0.0,0.45,1.0];
        let ys=[-0.4f32,-0.12,0.12,0.4];
        let mut faces=Vec::new();
        let s=std::f32::consts::FRAC_1_SQRT_2;
        for x in 0..4 { for y in 0..3 {
            if y!=1 && x!=3 { continue; }
            for t in square(xs[x],xs[x+1],ys[y],ys[y+1],0.) {
                faces.push(Tri { v: t.map(|[x,y,z]| [s*(x-y),s*(x+y),z]), material: 1 });
            }
        }}
        let park:Vec<_>=(0..faces.len()).collect();
        let seed=faces.iter().position(|t| distance(Vec3::ZERO,&t.v)<1e-6).unwrap();
        for reverse in [false,true] {
            let mut support=SurfaceSupport::new(&faces,&park);
            if reverse { for links in &mut support.links { links.reverse(); } }
            assert!(support.qualifies(seed,Vec3::ZERO), "reverse={reverse}");
        }
    }

    #[test]
    fn local_support_uses_clipped_area_and_survives_subdivision_and_rotation() {
        for n in [1, 4, 20] {
            for angle in [0.0f32, 0.37, std::f32::consts::FRAC_PI_4, 1.2] {
                for (width, expected) in [(2.0, true), (0.05, false), (0.2, false), (0.29, false), (0.35, true)] {
                    let mut faces = Vec::new();
                    for k in 0..n {
                        let x0 = -10.0 + 20.0 * k as f32 / n as f32;
                        let x1 = -10.0 + 20.0 * (k + 1) as f32 / n as f32;
                        for t in square(x0, x1, -width / 2.0, width / 2.0, 0.0) {
                            faces.push(Tri { v: t.map(|[x,y,z]| [x * angle.cos() - y * angle.sin(), x * angle.sin() + y * angle.cos(), z]), material: 1 });
                        }
                    }
                    let park: Vec<_> = (0..faces.len()).collect();
                    let seed = faces.iter().position(|t| distance(Vec3::ZERO, &t.v) < 1e-5).unwrap();
                    let mut support = SurfaceSupport::new(&faces, &park);
                    assert_eq!(support.qualifies(seed, Vec3::ZERO), expected, "n={n} angle={angle} width={width}");
                    assert_eq!(support.qualifies(seed, Vec3::ZERO), expected, "repeat");
                    faces.reverse();
                    let seed = faces.iter().position(|t| distance(Vec3::ZERO, &t.v) < 1e-5).unwrap();
                    assert_eq!(SurfaceSupport::new(&faces, &park).qualifies(seed, Vec3::ZERO), expected, "reverse");
                }
            }
        }
        // A very large face contributes only its clipped square, not its full area.
        let t = [Vec3::new(-10.,-10.,0.),Vec3::new(10.,-10.,0.),Vec3::new(0.,10.,0.)];
        assert!((clipped_support_area(t, 0.5) - 1.0).abs() < 1e-6);
        assert!(!support_edge_intersects([Vec3::new(2.,-1.,0.),Vec3::new(2.,1.,0.)], 0.5));
        assert!(support_edge_intersects([Vec3::new(-2.,0.,0.),Vec3::new(2.,0.,0.)], 0.5));
    }

    #[test]
    fn support_cannot_wrap_a_thin_tube() {
        for radius in [0.025, 0.05, 0.18] {
          for sides in [8, 32, 128] {
            let mut faces = Vec::new();
            for k in 0..sides {
                let p = |j: usize, y: f32| {
                    let angle = std::f32::consts::TAU * (j % sides) as f32 / sides as f32;
                    [radius * angle.cos(), y, radius * angle.sin()]
                };
                for v in [[p(k,-10.),p(k,10.),p(k+1,10.)],[p(k,-10.),p(k+1,10.),p(k+1,-10.)]] {
                    faces.push(Tri { v, material: 1 });
                }
            }
            let park: Vec<_> = (0..faces.len()).collect();
            assert!(!SurfaceSupport::new(&faces, &park).qualifies(0, Vec3::new(radius,0.,0.)), "radius={radius} sides={sides}");
          }
        }
    }

    #[test]
    fn disconnected_corner_nonmanifold_and_external_edges_do_not_donate_support() {
        let seed = Tri { v: [[0.,0.,0.],[0.1,0.,0.],[0.,0.1,0.]], material: 1 };
        for other in [
            // Nearby disconnected sheet; corner contact; a shared edge outside cube.
            [[-1.,-1.,0.01],[1.,-1.,0.01],[0.,1.,0.01]],
            [[0.,0.,0.],[-1.,0.,0.],[0.,-1.,0.]],
        ] {
            let faces = vec![seed, Tri { v: other, material: 2 }];
            assert!(!SurfaceSupport::new(&faces, &[0,1]).qualifies(0, Vec3::ZERO));
        }
        let broad = Tri { v: [[0.1,0.,0.],[0.,0.,0.],[0.,-10.,0.]], material: 2 };
        let faces = vec![seed, broad, broad];
        assert!(!SurfaceSupport::new(&faces, &[0,1,2]).qualifies(0, Vec3::ZERO), "three edge owners");
        let a = Tri { v: [[2.,-1.,0.],[2.,1.,0.],[-0.1,0.,0.]], material: 1 };
        let b = Tri { v: [[2.,-1.,0.],[2.,1.,0.],[-10.,0.,0.]], material: 2 };
        let faces = vec![a,b];
        assert!(normal(&a.v).dot(normal(&b.v)) > 0.99);
        assert!(clipped_support_area(a.v.map(v3), 0.5) + clipped_support_area(b.v.map(v3), 0.5) > SURFACE);
        assert!(!SurfaceSupport::new(&faces, &[0,1]).qualifies(0, Vec3::ZERO), "edge outside cube");
    }

    #[test]
    #[ignore = "local reconstructed park fixture and verified visible sidecar required"]
    fn reconstructed_park_support_work() {
        let path = std::env::var("SKATEV_SUPPORT_FIXTURE").expect("fixture path");
        let bytes = std::fs::read(path).unwrap();
        let input: Vec<Tri> = bytes.chunks_exact(36).map(|raw| {
            let f = |i: usize| f32::from_le_bytes(raw[4*i..4*i+4].try_into().unwrap());
            Tri { v: [[f(0),f(1),f(2)],[f(3),f(4),f(5)],[f(6),f(7),f(8)]], material: 1 }
        }).collect();
        let visible = decode(&std::fs::read(std::env::var("SKATEV_SUPPORT_VISIBLE").expect("sidecar path")).unwrap()).unwrap();
        let visible = &visible.iter().find(|(id,_)| id == "vespucci").unwrap().1;
        for _ in 0..3 {
            let mut gta = input.clone();
            let start = std::time::Instant::now();
            let out = run(&mut gta, &[], visible);
            eprintln!("reconstructed park: input={} removed={} added={} support queries/visited/worst={:?} elapsed={:?}", input.len(), out.removed.len(), out.added.len(), out.support_work, start.elapsed());
            if let Ok(path) = std::env::var("SKATEV_SUPPORT_OUTPUT") {
                let parents: HashMap<_, _> = input.iter().map(|t| (svwc::surfaces::key(&t.v), t.v)).collect();
                let mut bytes = Vec::new();
                for ((t, &key), &cut) in out.added.iter().zip(&out.surface_from).zip(&out.cut) {
                    bytes.push(u8::from(cut));
                    for v in t.v.into_iter().chain(parents[&key]).flatten() { bytes.extend(v.to_le_bytes()); }
                }
                std::fs::write(path, bytes).unwrap();
            }
            assert!(out.support_work.0 > 0);
        }
    }

    #[test]
    fn small_sidewall_support_does_not_admit_vertical_visible_details() {
        for nz in [0.0f32, 0.25, 0.75, 1.0] {
            let nx = (1.0 - nz * nz).sqrt();
            let p = |[x,y,z]: [f32;3]| [-1355.0 + nz*x + nx*z, -1396.0 + y, 2.0 - nx*x + nz*z];
            let mut gta = Vec::new();
            for x in 0..10 {
                for y in 0..10 {
                    for t in square(x as f32*0.2,(x+1) as f32*0.2,y as f32*0.2,(y+1) as f32*0.2,0.) {
                        gta.push(Tri { v: t.map(p), material: 1 });
                    }
                }
            }
            let original = gta.clone();
            let visible: Vec<_> = square(0.,2.,0.,2.,0.05).into_iter().map(|t| t.map(p)).collect();
            let out = run(&mut gta, &[], &visible);
            if nz <= 0.5 {
                assert!(out.added.is_empty(), "small sidewall nz={nz} must keep GTA ownership");
                assert_eq!(gta.iter().map(|t| t.v).collect::<Vec<_>>(), original.iter().map(|t| t.v).collect::<Vec<_>>());
                let mut large: Vec<Tri> = square(0.,2.,0.,2.,0.).into_iter().map(|t| Tri { v: t.map(p), material: 1 }).collect();
                assert!(!run(&mut large, &[], &visible).added.is_empty(), "legacy large sidewall nz={nz} stays eligible");
            } else {
                assert!(!out.added.is_empty(), "upward bank nz={nz} must be eligible");
            }
        }
    }

    #[test]
    fn small_upward_rescue_requires_both_source_and_visible_to_face_up() {
        for (source_z, visible_z) in [(0.25f32,0.75f32),(0.75,0.25)] {
            let source_x = (1.0 - source_z*source_z).sqrt();
            let visible_x = (1.0 - visible_z*visible_z).sqrt();
            let source_n = Vec3::new(source_x,0.,source_z);
            let source_u = Vec3::new(source_z,0.,-source_x);
            let visible_u = Vec3::new(visible_z,0.,-visible_x);
            let centre = Vec3::new(-1355.,-1396.,2.);
            let p = |x: f32,y: f32| (centre + source_u*x + Vec3::Y*y).to_array();
            let mut gta = Vec::new();
            for x in 0..10 { for y in 0..10 {
                for t in square(-1.+x as f32*0.2,-1.+(x+1) as f32*0.2,-1.+y as f32*0.2,-1.+(y+1) as f32*0.2,0.) {
                    gta.push(Tri { v: t.map(|[x,y,_]| p(x,y)), material: 1 });
                }
            }}
            let original = gta.iter().map(|t| t.v).collect::<Vec<_>>();
            let visible: Vec<_> = square(-1.,1.,-1.,1.,0.).into_iter().map(|t| t.map(|[x,y,_]| (centre + visible_u*x + Vec3::Y*y + source_n*0.05).to_array())).collect();
            let out = run(&mut gta, &[], &visible);
            assert!(out.added.is_empty(), "source z={source_z}, visible z={visible_z}");
            assert_eq!(gta.iter().map(|t| t.v).collect::<Vec<_>>(), original);
        }
    }

    #[test]
    fn small_connected_wall_facets_still_receive_the_visible_curve() {
        let (_, visible) = wall();
        let mut gta = Vec::new();
        // Each source facet is 0.186 m2, on the original broad chord.
        for k in 0..6 {
            let y0 = -1396.0 + 2.0 * k as f32 / 6.0;
            let y1 = -1396.0 + 2.0 * (k + 1) as f32 / 6.0;
            for v in [
                [[-1355.,y0,0.],[-1354.,y0,0.5],[-1354.,y1,0.5]],
                [[-1355.,y0,0.],[-1354.,y1,0.5],[-1355.,y1,0.]],
            ] { gta.push(Tri { v, material: 1 }); }
        }
        assert!(gta.iter().all(|t| area(&t.v) < SURFACE));
        let out = run(&mut gta, &[], &visible);
        assert_eq!(out.removed.len(), 12);
        assert_eq!(out.cut.iter().filter(|&&c| !c).count(), visible.len());
        assert_eq!(layered(&gta, &out.added, &out.cut), 0);
    }

    #[test]
    fn adjacent_upward_partner_rescues_chord_seam_and_keeps_flat_neighbour_untouched() {
        let (x0, y0) = (-1379.3f32, -1383.9f32);
        // Closer small GTA chord i (material 1): outer corner at x0-0.28 raised by 0.21 m,
        // tilting its normal >40 deg away from the visible bank (dot < CURVE_FACING).
        let chord = Tri {
            v: [[x0 - 0.28, y0 + 1.1, 3.61], [x0, y0, 3.40], [x0, y0 + 1.1, 3.50]],
            material: 1,
        };
        // Shared-edge compatible bank j (material 7) and its connected broad support (material 7).
        let bank = Tri {
            v: [[x0 + 0.26, y0, 3.50], [x0, y0 + 1.1, 3.50], [x0, y0, 3.40]],
            material: 7,
        };
        let bank_pair = Tri {
            v: [[x0 + 0.26, y0, 3.50], [x0 + 0.26, y0 + 1.1, 3.55], [x0, y0 + 1.1, 3.50]],
            material: 7,
        };
        let support_ext = Tri {
            v: [[x0 + 0.26, y0, 3.50], [x0 + 0.70, y0 + 0.55, 3.52], [x0 + 0.26, y0 + 1.1, 3.55]],
            material: 7,
        };
        let vis_seam = [[x0, y0 + 1.1, 3.50], [x0 - 0.28, y0 + 1.1, 3.40], [x0, y0, 3.40]];
        // Flat visible triangle over `bank` (within 1 mm): must NOT be dragged into `used`.
        let vis_flat = [[x0, y0, 3.4005], [x0 + 0.26, y0, 3.5005], [x0, y0 + 1.1, 3.5005]];
        assert!(area(&chord.v) < SURFACE && area(&bank.v) < SURFACE);
        assert!(normal(&vis_seam).dot(normal(&chord.v)) < CURVE_FACING);
        assert!(normal(&vis_seam).dot(normal(&bank.v)) >= CURVE_FACING);
        let bank_key = svwc::surfaces::key(&bank.v);
        for reverse in [false, true] {
            let mut gta = vec![chord, bank, bank_pair, support_ext];
            if reverse { gta.reverse(); }
            let out = run(&mut gta, &[], &[vis_seam, vis_flat]);
            let added_vis: Vec<_> = out.added.iter().zip(&out.cut).zip(&out.surface_from)
                .filter(|((_, c), _)| !**c).collect();
            assert_eq!(added_vis.len(), 1, "only the displaced seam triangle is admitted (reverse={reverse})");
            assert_eq!(added_vis[0].0.0.v, vis_seam);
            assert_eq!(added_vis[0].0.0.material, 7, "inherits adjacent bank material");
            assert_eq!(*added_vis[0].1, bank_key, "inherits adjacent bank surface key");
            assert_eq!(layered(&gta, &out.added, &out.cut), 0);
        }
    }

    #[test]
    fn adjacent_partner_rejects_disconnected_nonmanifold_steep_large_and_distant_neighbours() {
        let (x0, y0) = (-1379.3f32, -1383.9f32);
        let chord = Tri {
            v: [[x0 - 0.28, y0 + 1.1, 3.61], [x0, y0, 3.40], [x0, y0 + 1.1, 3.50]],
            material: 1,
        };
        let bank = Tri {
            v: [[x0 + 0.26, y0, 3.50], [x0, y0 + 1.1, 3.50], [x0, y0, 3.40]],
            material: 7,
        };
        let bank_pair = Tri {
            v: [[x0 + 0.26, y0, 3.50], [x0 + 0.26, y0 + 1.1, 3.55], [x0, y0 + 1.1, 3.50]],
            material: 7,
        };
        let support_ext = Tri {
            v: [[x0 + 0.26, y0, 3.50], [x0 + 0.70, y0 + 0.55, 3.52], [x0 + 0.26, y0 + 1.1, 3.55]],
            material: 7,
        };
        let vis_seam = [[x0, y0 + 1.1, 3.50], [x0 - 0.28, y0 + 1.1, 3.40], [x0, y0, 3.40]];
        // 1. Disconnected compatible layer (shifted 1 mm in X so no manifold edge is shared).
        let shift = |t: Tri, dx: f32| Tri { v: t.v.map(|[x, y, z]| [x + dx, y, z]), ..t };
        let mut disconnected = vec![chord, shift(bank, 0.001), shift(bank_pair, 0.001), shift(support_ext, 0.001)];
        assert!(run(&mut disconnected, &[], &[vis_seam]).added.is_empty(), "disconnected layer");
        // 2. Corner-only contact (1 shared vertex at [x0, y0, 3.40]).
        let corner_only = Tri {
            v: [[x0 + 0.26, y0, 3.50], [x0 + 0.01, y0 + 1.1, 3.50], [x0, y0, 3.40]],
            material: 7,
        };
        let mut corner_gta = vec![chord, corner_only, bank_pair, support_ext];
        assert!(run(&mut corner_gta, &[], &[vis_seam]).added.is_empty(), "corner contact");
        // 3. Non-manifold shared edge (3 incident faces on the shared edge).
        let mut nonmanifold = vec![chord, bank, bank, bank_pair, support_ext];
        assert!(run(&mut nonmanifold, &[], &[vis_seam]).added.is_empty(), "non-manifold edge");
        // 4. Steep/vertical nearest source (normal.z <= 0.5).
        let steep_chord = Tri {
            v: [[x0 - 0.05, y0 + 1.1, 3.90], [x0, y0, 3.40], [x0, y0 + 1.1, 3.50]],
            material: 1,
        };
        assert!(normal(&steep_chord.v).z <= 0.5);
        let mut steep_gta = vec![steep_chord, bank, bank_pair, support_ext];
        assert!(run(&mut steep_gta, &[], &[vis_seam]).added.is_empty(), "steep nearest source");
        // 5. Large nearest source (area >= SURFACE = 0.25 m2).
        let large_chord = Tri {
            v: [[x0 - 0.60, y0 + 1.1, 3.85], [x0, y0, 3.40], [x0, y0 + 1.1, 3.50]],
            material: 1,
        };
        assert!(area(&large_chord.v) >= SURFACE && normal(&large_chord.v).z > 0.5);
        let mut large_gta = vec![large_chord, bank, bank_pair, support_ext];
        assert!(run(&mut large_gta, &[], &[vis_seam]).added.is_empty(), "large nearest source");
        // 6. Neighbour distance > nearest + PARTNER_SLACK (0.05 m).
        let vis_close_to_chord = [[x0 - 0.02, y0 + 1.0, 3.51], [x0 - 0.26, y0 + 1.05, 3.42], [x0 - 0.08, y0 + 0.2, 3.42]];
        let d_chord = distance(centroid(&vis_close_to_chord), &chord.v);
        let d_bank = distance(centroid(&vis_close_to_chord), &bank.v);
        assert!(d_bank > d_chord + PARTNER_SLACK && d_bank <= MATCH);
        let mut slack_gta = vec![chord, bank, bank_pair, support_ext];
        assert!(run(&mut slack_gta, &[], &[vis_close_to_chord]).added.is_empty(), "exceeds partner slack");
        // 7. Nearest compatible neighbour lacks support breadth (<0.30 m): must not skip to a farther neighbour.
        let mut narrow_gta = vec![chord, bank];
        assert!(run(&mut narrow_gta, &[], &[vis_seam]).added.is_empty(), "insufficient neighbour support");
        // 8. Direct unit checks for `adjacent_partner` distance bounds (`dj > MATCH` and `segment_distance > MATCH`).
        let park: Vec<usize> = (0..4).collect();
        let gta = vec![chord, bank, bank_pair, support_ext];
        let mut sup = SurfaceSupport::new(&gta, &park);
        let c = centroid(&vis_seam);
        let vn = normal(&vis_seam);
        assert_eq!(sup.adjacent_partner(0, distance(c, &chord.v), c, vn, &park), Some(1));
        let wrap_bank = Tri {
            v: [[x0 - 0.50, y0 + 0.55, 3.30], [x0, y0, 3.40], [x0, y0 + 1.1, 3.50]],
            material: 7,
        };
        let wrap_ext = Tri {
            v: [[x0 - 0.50, y0 + 0.55, 3.30], [x0 - 0.80, y0, 3.30], [x0, y0, 3.40]],
            material: 7,
        };
        let mut wrap_sup = SurfaceSupport::new(&vec![chord, wrap_bank, wrap_ext], &[0, 1, 2]);
        let far_edge_c = Vec3::new(x0 - 0.35, y0 + 0.55, 3.45);
        assert!(wrap_sup.qualifies(1, far_edge_c));
        assert!(distance(far_edge_c, &wrap_bank.v) <= 0.24 && distance(far_edge_c, &wrap_bank.v) <= MATCH);
        assert!(segment_distance(far_edge_c, [Vec3::new(x0, y0, 3.40), Vec3::new(x0, y0 + 1.1, 3.50)]) > MATCH);
        assert_eq!(wrap_sup.adjacent_partner(0, 0.24, far_edge_c, vn, &[0, 1, 2]), None, "shared edge outside MATCH");
        let far_face_c = Vec3::new(x0, y0 - 0.30, 3.40);
        assert!(distance(far_face_c, &bank.v) > MATCH);
        assert_eq!(sup.adjacent_partner(0, 0.24, far_face_c, vn, &park), None, "neighbour outside MATCH");
    }

    #[test]
    fn a_faceted_wall_is_replaced_by_the_visible_curve_without_a_layer() {
        let (mut gta, visible) = wall();
        let out = run(&mut gta, &[], &visible);
        assert!(gta.is_empty(), "the chord goes");
        assert_eq!(out.removed.len(), 2);
        assert_eq!(out.added.iter().zip(&out.cut).filter(|(_, c)| !**c).count(), visible.len());
        assert!(out.added.iter().all(|t| t.material == 1));
        assert_eq!(out.surface_from.len(), out.added.len());
        assert_eq!(layered(&gta, &out.added, &out.cut), 0, "no GTA surface left under the visible one");
    }

    #[test]
    fn a_half_covered_floor_keeps_its_uncovered_half_as_a_piece_and_meets_the_visible_edge() {
        // GTA floor 2 x 2 m at z 0; the visible floor covers its first metre 5 cm higher.
        let x0 = -1360.0;
        let y0 = -1400.0;
        let mut gta: Vec<Tri> = square(x0, x0 + 2., y0, y0 + 2., 0.).into_iter().map(|v| Tri { v, material: 3 }).collect();
        let visible = square(x0, x0 + 1., y0, y0 + 2., 0.05);
        let out = run(&mut gta, &[], &visible);
        assert!(gta.is_empty(), "both GTA triangles are cut");
        let pieces: Vec<&Tri> = out.added.iter().zip(&out.cut).filter(|(_, c)| **c).map(|(t, _)| t).collect();
        let piece_area: f32 = pieces.iter().map(|t| area(&t.v)).sum();
        assert!((piece_area - 2.0).abs() < 0.02, "the uncovered half stays: {piece_area}");
        let total: f32 = out.added.iter().map(|t| area(&t.v)).sum();
        assert!((total - 4.0).abs() < 0.04, "no hole and no double: {total}");
        // The cut edge at x0 + 1 moved onto the visible floor (5 cm up); the far edge stays.
        for p in pieces.iter().flat_map(|t| t.v) {
            if (p[0] - (x0 + 1.)).abs() < 1e-3 { assert!((p[2] - 0.05).abs() < 1e-3, "{p:?}"); }
            if (p[0] - (x0 + 2.)).abs() < 1e-3 { assert!(p[2].abs() < 1e-3, "{p:?}"); }
        }
        assert_eq!(layered(&gta, &out.added, &out.cut), 0);
    }

    /// Design 2026-10-31 remnant cleanup: a strip narrower than REMNANT_WIDTH
    /// beside the visible surface goes; a real remnant stays whole.
    #[test]
    fn a_sliver_beside_the_visible_edge_goes_and_a_real_remnant_stays() {
        let (x0, y0) = (-1360.0, -1400.0);
        for (covered, strip_kept) in [(1.99f32, false), (1.70, true)] {
            let mut gta: Vec<Tri> = square(x0, x0 + 2., y0, y0 + 2., 0.).into_iter().map(|v| Tri { v, material: 3 }).collect();
            let visible = square(x0, x0 + covered, y0, y0 + 2., 0.05);
            let out = run(&mut gta, &[], &visible);
            let pieces: Vec<&Tri> = out.added.iter().zip(&out.cut).filter(|(_, c)| **c).map(|(t, _)| t).collect();
            let piece_area: f32 = pieces.iter().map(|t| area(&t.v)).sum();
            if strip_kept {
                assert!((piece_area - 0.6).abs() < 0.02, "30 cm remnant stays: {piece_area}");
                assert!(pieces.len() <= 4, "clean corners, not a fan of slivers: {}", pieces.len());
            } else {
                assert!(pieces.is_empty(), "1 cm strip is not a surface: {piece_area}");
            }
        }
    }

    #[test]
    fn a_long_thin_face_the_visible_mesh_barely_touches_keeps_its_remnant() {
        // A 3 m x 1.5 cm lip top; a visible face covers only its first 10 cm.
        let lip = [[0., 0., 0.], [3., 0., 0.], [3., 0.015, 0.]];
        let cover = Used { v: [[-0.5, -0.5, 0.01], [0.1, -0.5, 0.01], [0.1, 0.5, 0.01]], partner: 0, limit: MATCH };
        let kept: f32 = cut(&lip, [&cover].into_iter()).iter().map(area).sum();
        let whole = area(&lip);
        assert!(kept > 0.9 * whole, "remnant {kept} of {whole} must stay");
    }

    #[test]
    fn remnant_corners_lose_duplicates_and_collinear_points_only() {
        let sq = [Vec2::new(0., 0.), Vec2::new(0.5, 0.0002), Vec2::new(1., 0.), Vec2::new(1., 1.), Vec2::new(1.0002, 1.), Vec2::new(0., 1.)];
        let s = simplify(&sq, &[]);
        let kept = simplify(&sq, &[Vec2::new(0.5, 0.0002)]);
        assert!(kept.contains(&Vec2::new(0.5, 0.0002)), "an original GTA corner is never moved: {kept:?}");
        assert_eq!(s.len(), 4, "{s:?}");
        assert!((polygon_area(&s) - 1.0).abs() < 1e-3);
        assert!((convex_width(&s) - 1.0).abs() < 1e-3);
        assert!((convex_width(&[Vec2::ZERO, Vec2::new(3., 0.), Vec2::new(3., 0.01), Vec2::new(0., 0.01)]) - 0.01).abs() < 1e-5);
    }

    #[test]
    fn a_ground_under_a_visible_ledge_top_is_not_its_surface() {
        // Ground 4 x 4 m at z 0; a ledge 1 x 1 m whose GTA top lies at 0.20
        // and whose visible top lies at 0.21. The ground must stay untouched
        // (it is not the ledge's surface): no ramp up to the ledge.
        let x0 = -1360.0;
        let y0 = -1400.0;
        let ground: Vec<Tri> = square(x0, x0 + 4., y0, y0 + 4., 0.).into_iter().map(|v| Tri { v, material: 3 }).collect();
        let ledge: Vec<Tri> = square(x0 + 1., x0 + 2., y0 + 1., y0 + 2., 0.20).into_iter().map(|v| Tri { v, material: 4 }).collect();
        let mut gta: Vec<Tri> = ground.iter().chain(&ledge).cloned().collect();
        let visible = square(x0 + 1., x0 + 2., y0 + 1., y0 + 2., 0.21);
        let out = run(&mut gta, &[], &visible);
        for g in &ground {
            assert!(gta.iter().any(|t| t.v == g.v), "the ground triangle {:?} must stay as it was", g.v);
        }
        assert!(out.removed.iter().all(|t| t.material == 4), "only the ledge top is replaced");
    }

    #[test]
    fn rails_trims_and_flat_matches_stay_gta_s() {
        // A railing bar: GTA's thin box face (0.05 x 2 m) with a round visible tube 4 cm off it.
        let x0 = -1360.0;
        let y0 = -1400.0;
        let mut gta = vec![Tri { v: [[x0, y0, 1.0], [x0 + 2., y0, 1.0], [x0 + 2., y0 + 0.05, 1.0]], material: 2 }];
        let visible = vec![[[x0, y0, 1.04], [x0 + 2., y0, 1.04], [x0 + 2., y0 + 0.05, 1.04]]];
        let out = run(&mut gta, &[], &visible);
        assert!(out.added.is_empty() && gta.len() == 1, "a rail-sized face is no bowl");
        // A flat deck the visible model matches within 2 cm stays GTA's.
        let mut deck: Vec<Tri> = square(x0, x0 + 2., y0, y0 + 2., 0.).into_iter().map(|v| Tri { v, material: 3 }).collect();
        let out = run(&mut deck, &[], &square(x0, x0 + 2., y0, y0 + 2., 0.02));
        assert!(out.added.is_empty() && deck.len() == 2);
    }

    #[test]
    fn an_invisible_wall_and_a_visible_only_detail_stay_as_they_are() {
        let (mut gta, mut visible) = wall();
        gta.push(Tri { v: [[-1350., -1396., 0.], [-1350., -1394., 0.], [-1350., -1394., 2.]], material: 1 });
        visible.push([[-1345., -1396., 3.], [-1344., -1396., 3.], [-1344., -1395., 3.]]);
        let out = run(&mut gta, &[], &visible);
        assert_eq!(gta.len(), 1, "the invisible wall stays");
        assert!(out.added.iter().all(|t| t.v[0][0] < -1350.), "the detail is not added");
    }

    #[test]
    fn stairs_and_back_faces_are_never_used() {
        let (gta, visible) = wall();
        let flipped: Vec<_> = visible.iter().map(|t| [t[0], t[2], t[1]]).collect();
        let out = run(&mut gta.clone(), &[], &flipped);
        assert!(out.added.is_empty(), "back faces");
        // A stair triangle under the first 30 cm: no visible triangle with a
        // sample near it is used.
        let stair = vec![Tri { v: [[-1355., -1396., 0.1], [-1354.7, -1396., 0.1], [-1354.7, -1394., 0.1]], material: 1 }];
        let mut g = gta.clone();
        let out = run(&mut g, &stair, &visible);
        for t in out.added.iter().zip(&out.cut).filter(|(_, c)| !**c).map(|(t, _)| t) {
            let close = samples(&t.v).into_iter().any(|p| distance(p, &stair[0].v) <= MATCH);
            assert!(!close, "{:?}", t.v);
        }
    }

    #[test]
    fn an_uncut_triangle_comes_back_exactly() {
        let t = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let far = Used { v: [[5., 5., 0.], [6., 5., 0.], [5., 6., 0.]], partner: 0, limit: 0.1 };
        assert_eq!(cut(&t, std::iter::once(&far)), vec![t]);
        let over = Used { v: [[-1., -1., 0.05], [3., -1., 0.05], [-1., 3., 0.05]], partner: 0, limit: 0.1 };
        assert!(cut(&t, std::iter::once(&over)).is_empty(), "fully covered");
    }

    #[test]
    fn file_round_trip_and_malformed_rejection() {
        let parks = vec![("vespucci".to_string(), wall().1)];
        let mut b = b"SVVP".to_vec();
        b.extend(1u32.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b.push(8);
        b.extend(b"vespucci");
        b.extend((parks[0].1.len() as u32).to_le_bytes());
        for v in parks[0].1.iter().flatten().flatten() { b.extend(v.to_le_bytes()); }
        assert_eq!(decode(&b).unwrap(), parks);
        assert!(decode(&b[..b.len() - 1]).is_none());
        let mut bad = b.clone();
        bad.push(0);
        assert!(decode(&bad).is_none());
    }

    #[test]
    fn outside_every_park_nothing_changes() {
        let (gta, visible) = wall();
        let moved: Vec<Tri> = gta.iter().map(|t| Tri { v: t.v.map(|p| [p[0] + 5000., p[1], p[2]]), material: 1 }).collect();
        let mut tris = moved.clone();
        run(&mut tris, &[], &visible);
        assert_eq!(tris.len(), moved.len());
        let _ = set_for_test;
    }
}
