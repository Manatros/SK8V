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
    let candidates: Vec<Used> = near.iter().filter_map(|t| {
        let c = centroid(t);
        let (d, i) = nearest_gta(c)?;
        let partner = normal(&tris[i].v);
        if d > MATCH || normal(t).dot(partner) < CURVE_FACING || normal(t).z < -0.2 { return None; }
        let by_stairs = samples(t).into_iter()
            .any(|p| stair_grid.at(p).iter().any(|&k| distance(p, &stairs[k as usize].v) <= MATCH));
        if by_stairs { return None; }
        // How far this visible triangle lies from its own GTA surface: only
        // GTA surfaces that close (plus PARTNER_SLACK) are the same surface.
        // A ground 20 cm under a visible ledge top is not (its partner is
        // the GTA ledge top a few centimetres off).
        let a = v3(tris[i].v[0]);
        let deviation = t.iter().map(|&p| (v3(p) - a).dot(partner).abs()).fold(0.0f32, f32::max);
        Some(Used { v: **t, partner: i, limit: (deviation + PARTNER_SLACK).min(MATCH) })
    }).collect();
    // GTA's collision is the base (Manatros 2026-10-26 19:03 recording:
    // "plain GTA collision is a solid basis that just needs fine-tuning like
    // the high-res bowl"). Only the bowls' curves replace it, a whole GTA
    // face at a time: a GTA surface (not a rail, post or trim) that the
    // visible one leaves by 3 cm or more somewhere. Flat decks, ledges and
    // railings stay GTA's (their visible copies cut GTA's into dense
    // fragments, marks 5, 7, 8).
    let mut deviates: HashMap<usize, f32> = HashMap::default();
    for u in &candidates {
        let e = deviates.entry(u.partner).or_insert(0.0);
        *e = e.max(u.limit - PARTNER_SLACK);
    }
    let used: Vec<Used> = candidates.into_iter().filter(|u| {
        let gta = tris[u.partner].v.map(v3);
        let area = (gta[1] - gta[0]).cross(gta[2] - gta[0]).length() / 2.0;
        area >= SURFACE && deviates.get(&u.partner).is_some_and(|&d| d >= CURVE)
    }).map(|u| {
        // The whole face's deviation is the limit: the parts of the chord
        // close to the curve go with it.
        let d = deviates[&u.partner];
        Used { limit: (d + PARTNER_SLACK).min(MATCH), ..u }
    }).collect();
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
    let mut pieces: Vec<Vec<Vec2>> = vec![vec![to2(a), to2(b), to2(c)]];
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
