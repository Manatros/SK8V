//! Rails from GTA's visible models (`skatev-world-cache` `visual-rails`,
//! `<cache>.visrails`): handrails, railings and coping bars wherever the map
//! or an interior places them, including the many GTA leaves out of every
//! collision copy (Manatros 2026-10-20: the Portola Drive station's stair
//! handrails "do not have collision boxes and grinding isn't quite right,
//! the grind lines sometimes diverge from the actual rail").
//!
//! In an area: the rails with something to stand on below them become grind
//! rails and get a thin collision box, and the edge finder's lines along
//! them go (the visible rail's own line is the exact one).

use bevy_math::Vec3;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

/// GTAxSkate VisualRails (World page, default on).
pub static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// A rail needs a floor this far below it (metres) somewhere along it:
/// handrails stand 0.8-1.1 m over their stairs, coping and ledges less.
/// From 15 cm: a railing's foot bar or kick plate on the floor is no rail
/// (2026-10-20 test: "at the bottom of the railing it is grindable, though
/// it is just normal floor").
const FLOOR_BELOW: std::ops::RangeInclusive<f32> = 0.15..=2.5;
/// Steepest grindable stretch of a visible-model rail (degrees). Stair
/// handrails run at 30-40; the metro's power poles and cable braces at
/// 53-90 became grind lines in the air (Manatros 2026-10-25 mark 13).
const STEEPEST: f32 = 50.0;
/// Shortest stretch kept after cutting out steep pieces (metres).
const SHORTEST: f32 = 0.3;

/// The stretches of `r` no steeper than `STEEPEST`, each at least `SHORTEST` long.
fn gentle_pieces(r: &[Vec3]) -> Vec<Vec<Vec3>> {
    let limit = STEEPEST.to_radians().tan();
    let mut out = Vec::new();
    let mut piece: Vec<Vec3> = Vec::new();
    let mut flush = |piece: &mut Vec<Vec3>, out: &mut Vec<Vec<Vec3>>| {
        let length: f32 = piece.windows(2).map(|w| w[0].distance(w[1])).sum();
        if piece.len() >= 2 && length >= SHORTEST { out.push(std::mem::take(piece)); } else { piece.clear(); }
    };
    for w in r.windows(2) {
        let flat = (w[1] - w[0]).truncate().length();
        if (w[1].z - w[0].z).abs() <= limit * flat {
            if piece.is_empty() { piece.push(w[0]); }
            piece.push(w[1]);
        } else {
            flush(&mut piece, &mut out);
        }
    }
    flush(&mut piece, &mut out);
    out
}

/// Half the width and the depth of a rail's collision box (metres), under its top line.
pub(crate) const BOX_HALF: f32 = 0.025;
pub(crate) const BOX_DEPTH: f32 = 0.06;
/// Most cells kept read (each 32 m square); past it the cache starts again.
const KEEP_CELLS: usize = 1024;

type Cell = (i32, i32);
type Rails = std::sync::Arc<Vec<Vec<Vec3>>>;

/// The tiled rail file (SVRL v2): its cell table, read once; a cell's rails
/// are read when an area first needs them.
struct File {
    path: std::path::PathBuf,
    cell: f32,
    table: HashMap<Cell, (u32, u64, u64)>,
    read: std::sync::Mutex<HashMap<Cell, Rails>>,
}

static FILE: OnceLock<Option<File>> = OnceLock::new();

fn policy_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".policy");
    name.into()
}

fn policy_source() -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for bytes in [
        include_bytes!("../../world-cache/src/bin/visual-rails.rs").as_slice(),
        include_bytes!("../../world-cache/src/vfs.rs").as_slice(),
        include_bytes!("../../world-cache/src/packs.rs").as_slice(),
        include_bytes!("../../world-cache/src/bars.rs").as_slice(),
    ] {
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'\r' && bytes.get(i + 1) == Some(&b'\n') { continue; }
            hash = (hash ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    hash
}

fn data_hash(path: &Path) -> std::io::Result<u64> {
    use std::io::Read;
    let mut reader = std::fs::File::open(path)?;
    let mut buffer = [0u8; 65536];
    let mut hash = 0xcbf29ce484222325u64;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 { return Ok(hash); }
        for &b in &buffer[..count] { hash = (hash ^ b as u64).wrapping_mul(0x100000001b3); }
    }
}

/// The same receipt text for another sidecar written by `visual-rails`
/// (high-res parks, visparks.rs).
pub fn policy_text_for(path: &Path) -> std::io::Result<String> { policy_text(path) }

fn policy_text(path: &Path) -> std::io::Result<String> {
    Ok(format!("SVRL-POLICY 1\nStoryArchives=1\nStateGroups=live\nsource {:016x}\ndata {:016x}\n", policy_source(), data_hash(path)?))
}

/// Bind a newly exported rail file to the reviewed Story/state policy and
/// exact data bytes. Old all-DLC rail sidecars must never silently reactivate.
pub fn write_policy(path: &Path) -> std::io::Result<()> {
    std::fs::write(policy_path(path), policy_text(path)?)
}

pub fn verified_policy(path: &Path) -> bool {
    policy_text(path).ok().zip(std::fs::read_to_string(policy_path(path)).ok())
        .is_some_and(|(expected, actual)| expected == actual.replace("\r\n", "\n"))
}

/// Reads the header and cell table of SVRL v2: cell size and, per cell,
/// its rail count, byte offset and byte length (up to the next cell).
fn table(bytes: &[u8], file_len: u64) -> Option<(f32, HashMap<Cell, (u32, u64, u64)>)> {
    if bytes.get(0..4)? != b"SVRL" || u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?) != 2 {
        return None;
    }
    let cell = f32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?);
    let n = u32::from_le_bytes(bytes.get(12..16)?.try_into().ok()?) as usize;
    let mut rows = Vec::with_capacity(n);
    for k in 0..n {
        let e = bytes.get(16 + k * 20..36 + k * 20)?;
        let i = |o: usize| i32::from_le_bytes(e[o..o + 4].try_into().unwrap());
        rows.push(((i(0), i(4)), u32::from_le_bytes(e[8..12].try_into().ok()?), u64::from_le_bytes(e[12..20].try_into().ok()?)));
    }
    let mut ends: Vec<u64> = rows.iter().map(|r| r.2).collect();
    ends.sort_unstable();
    let mut t = HashMap::with_capacity(n);
    for (c, count, off) in rows {
        let next = ends.partition_point(|&o| o <= off);
        let end = ends.get(next).copied().unwrap_or(file_len);
        t.insert(c, (count, off, end.saturating_sub(off)));
    }
    (cell > 0.0).then_some((cell, t))
}

/// `count` rails from `bytes` (point count, then points).
fn rails_at(bytes: &[u8], count: u32) -> Option<Vec<Vec<Vec3>>> {
    let mut at = 0;
    let f = |at: usize| -> Option<f32> { Some(f32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?)) };
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let n = u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize;
        at += 4;
        let mut r = Vec::with_capacity(n.min(10_000));
        for _ in 0..n {
            r.push(Vec3::new(f(at)?, f(at + 4)?, f(at + 8)?));
            at += 12;
        }
        if r.len() >= 2 && r.iter().all(|p| p.is_finite()) {
            out.push(r);
        }
    }
    Some(out)
}

/// Opens the rails file; returns how many cells it holds (0: none).
pub fn load(path: &Path) -> usize {
    FILE.get_or_init(|| {
        if !verified_policy(path) { return None; }
        use std::io::Read;
        let mut f = std::fs::File::open(path).ok()?;
        let len = f.metadata().ok()?.len();
        let mut head = vec![0u8; 16];
        f.read_exact(&mut head).ok()?;
        let n = u32::from_le_bytes(head[12..16].try_into().ok()?) as usize;
        head.resize(16 + n * 20, 0);
        f.read_exact(&mut head[16..]).ok()?;
        let (cell, table) = table(&head, len)?;
        Some(File { path: path.to_path_buf(), cell, table, read: Default::default() })
    })
    .as_ref()
    .map_or(0, |f| f.table.len())
}

impl File {
    fn cell_rails(&self, c: Cell) -> Rails {
        let Some(&(count, offset, len)) = self.table.get(&c) else { return Default::default() };
        let mut read = self.read.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = read.get(&c) {
            return r.clone();
        }
        let load = || -> Option<Vec<Vec<Vec3>>> {
            use std::io::{Read, Seek};
            let mut f = std::fs::File::open(&self.path).ok()?;
            f.seek(std::io::SeekFrom::Start(offset)).ok()?;
            let mut buf = vec![0u8; len as usize];
            f.read_exact(&mut buf).ok()?;
            rails_at(&buf, count)
        };
        let rails: Rails = std::sync::Arc::new(load().unwrap_or_default());
        if read.len() >= KEEP_CELLS {
            read.clear();
        }
        read.insert(c, rails.clone());
        rails
    }
}

/// Rails supported by the supplied collision, without cross-build reuse.
pub fn rails_in(centre: [f32; 2], half: f32, tris: &[[Vec3; 3]]) -> Vec<Vec<Vec3>> {
    if !ENABLED.load(std::sync::atomic::Ordering::Relaxed) { return Vec::new(); }
    let Some(Some(file)) = FILE.get() else { return Vec::new() };
    let grid = crate::grid::XyGrid::triangles(1.0, tris);
    in_cells_reusing(file.cell, |c| file.cell_rails(c), centre, half, tris, &grid, &[], &crate::reuse::Dirty::everything(), &mut crate::reuse::LineCache::default())
}

/// Test source supplying cells directly.
#[cfg(test)]
fn in_cells(size: f32, cell: impl Fn(Cell) -> Rails, centre: [f32; 2], half: f32, tris: &[[Vec3; 3]], bars: &[[Vec3; 3]]) -> Vec<Vec<Vec3>> {
    let grid = crate::grid::XyGrid::triangles(1.0, tris);
    in_cells_reusing(size, cell, centre, half, tris, &grid, bars, &crate::reuse::Dirty::everything(), &mut crate::reuse::LineCache::default())
}

/// Rails with support checks reused where nearby geometry is unchanged.
/// `bars`: visible railing collision (visbars), which carries its own top
/// bar; the bake keeps it fixed per area, so reuse needs no key for it.
pub fn rails_in_reusing(centre: [f32; 2], half: f32, tris: &[[Vec3; 3]], grid: &crate::grid::XyGrid, bars: &[[Vec3; 3]], dirty: &crate::reuse::Dirty, cache: &mut crate::reuse::LineCache) -> Vec<Vec<Vec3>> {
    if !ENABLED.load(std::sync::atomic::Ordering::Relaxed) { return Vec::new(); }
    let Some(Some(file)) = FILE.get() else { return Vec::new() };
    in_cells_reusing(file.cell, |c| file.cell_rails(c), centre, half, tris, grid, bars, dirty, cache)
}

/// How close (metres) a visible rail must run to a visible railing's
/// collision to stand on it.
const ON_BAR: f32 = 0.10;
const WALL_SLAM_REACH: f32 = 0.29;
const WALL_SLAM_HEIGHTS: [f32; 2] = [0.114, 0.203];

/// True when `p` sits flush (`|z_floor - p.z| <= 4 cm`) on a solid shoulder slab
/// right in front of a vertical wall (`>= 20 cm` high) within `11.5 in` (`0.29 m`),
/// distinguishing barrier seams from freestanding handrails beside open air.
fn on_wall_slam_shoulder_with(
    p: Vec3,
    dir_xy: Vec3,
    visit_box: &dyn Fn([f32; 2], [f32; 2], &mut dyn FnMut([Vec3; 3]) -> bool) -> bool,
) -> bool {
    let sides = [Vec3::new(-dir_xy.y, dir_xy.x, 0.0), Vec3::new(dir_xy.y, -dir_xy.x, 0.0)];
    sides.into_iter().any(|side| {
        let end = p + side * WALL_SLAM_REACH;
        let lo = [p.x.min(end.x) - 0.02, p.y.min(end.y) - 0.02];
        let hi = [p.x.max(end.x) + 0.02, p.y.max(end.y) + 0.02];
        let mut wall_dists = [None::<f32>; 2];
        for (slot, &h) in wall_dists.iter_mut().zip(&WALL_SLAM_HEIGHTS) {
            let orig = p + Vec3::Z * h;
            let mut best: Option<f32> = None;
            visit_box(lo, hi, &mut |tri| {
                if let Some(t) = crate::rails::ray_triangle_hit(orig, side, WALL_SLAM_REACH, 1e-4, &tri) {
                    best = Some(best.map_or(t, |cur| cur.min(t)));
                }
                false
            });
            *slot = best;
        }
        let (Some(t1), Some(t2)) = (wall_dists[0], wall_dists[1]) else {
            return false;
        };
        let wall_dist = t1.min(t2).clamp(0.03, WALL_SLAM_REACH);
        let probe_dist = (wall_dist * 0.75).max(wall_dist - 0.04);
        let probe = p + side * probe_dist;
        visit_box([probe.x - 0.02, probe.y - 0.02], [probe.x + 0.02, probe.y + 0.02], &mut |t| {
            let n = (t[1] - t[0]).cross(t[2] - t[0]);
            n.z >= 0.65 * n.length().max(1e-9)
                && svwc::clean::floor_at(&svwc::Tri { v: t.map(|v| v.to_array()), material: 0 }, [probe.x, probe.y])
                    .is_some_and(|z| (z - p.z).abs() <= 0.04)
        })
    })
}

fn visit_grid_box(grid: &crate::grid::XyGrid, tris: &[[Vec3; 3]], lo: [f32; 2], hi: [f32; 2], f: &mut dyn FnMut([Vec3; 3]) -> bool) -> bool {
    for cx in grid.cell_of(lo[0])..=grid.cell_of(hi[0]) {
        for cy in grid.cell_of(lo[1])..=grid.cell_of(hi[1]) {
            if grid.cell(cx, cy).iter().any(|&i| f(tris[i as usize])) {
                return true;
            }
        }
    }
    false
}

fn on_wall_slam_shoulder(p: Vec3, dir_xy: Vec3, grid: &crate::grid::XyGrid, tris: &[[Vec3; 3]]) -> bool {
    on_wall_slam_shoulder_with(p, dir_xy, &|lo, hi, f| visit_grid_box(grid, tris, lo, hi, f))
}

fn gap_hits_wall_slam_shoulder(a: Vec3, b: Vec3, visit_box: &dyn Fn([f32; 2], [f32; 2], &mut dyn FnMut([Vec3; 3]) -> bool) -> bool) -> bool {
    let h = Vec3::new(b.x - a.x, b.y - a.y, 0.0);
    let hlen = h.length();
    if hlen < 1e-4 {
        return false;
    }
    let dir_xy = h / hlen;
    [0.25, 0.5, 0.75]
        .into_iter()
        .any(|t| on_wall_slam_shoulder_with(a.lerp(b, t), dir_xy, visit_box))
}

pub(crate) fn shoulder_blocked_between(a: Vec3, b: Vec3, grid: &crate::grid::XyGrid, tris: &[[Vec3; 3]]) -> bool {
    gap_hits_wall_slam_shoulder(a, b, &|lo, hi, f| visit_grid_box(grid, tris, lo, hi, f))
}

pub(crate) fn shoulder_blocked_between_skate_tris(a: Vec3, b: Vec3, tris: &[[[f32; 3]; 3]]) -> bool {
    let sa = crate::coords::to_skate(a);
    let sb = crate::coords::to_skate(b);
    let pad = WALL_SLAM_REACH + 0.05;
    let smin_x = sa.x.min(sb.x) - pad;
    let smax_x = sa.x.max(sb.x) + pad;
    let smin_z = sa.z.min(sb.z) - pad;
    let smax_z = sa.z.max(sb.z) + pad;
    let near: Vec<[Vec3; 3]> = tris
        .iter()
        .filter(|t| {
            let max_x = t[0][0].max(t[1][0]).max(t[2][0]);
            let min_x = t[0][0].min(t[1][0]).min(t[2][0]);
            let max_z = t[0][2].max(t[1][2]).max(t[2][2]);
            let min_z = t[0][2].min(t[1][2]).min(t[2][2]);
            max_x >= smin_x && min_x <= smax_x && max_z >= smin_z && min_z <= smax_z
        })
        .map(|t| t.map(|p| crate::coords::from_skate(Vec3::from_array(p))))
        .collect();
    if near.is_empty() {
        return false;
    }
    gap_hits_wall_slam_shoulder(a, b, &|lo, hi, f| {
        near.iter().any(|&t| {
            let max_x = t[0].x.max(t[1].x).max(t[2].x);
            let min_x = t[0].x.min(t[1].x).min(t[2].x);
            let max_y = t[0].y.max(t[1].y).max(t[2].y);
            let min_y = t[0].y.min(t[1].y).min(t[2].y);
            max_x >= lo[0] && min_x <= hi[0] && max_y >= lo[1] && min_y <= hi[1] && f(t)
        })
    })
}

fn flush_visrail_run(runs: &mut Vec<Vec<Vec3>>, cur: &mut Vec<Vec3>) {
    if cur.len() >= 2 {
        runs.push(std::mem::take(cur));
    } else {
        cur.clear();
    }
}

fn push_visrail_seg(runs: &mut Vec<Vec<Vec3>>, cur: &mut Vec<Vec3>, a: Vec3, b: Vec3) {
    if cur.last().is_some_and(|last| last.distance_squared(a) < 1e-6) {
        cur.push(b);
    } else {
        flush_visrail_run(runs, cur);
        *cur = vec![a, b];
    }
}

fn trim_wall_slam_shoulders(rail: &[Vec3], grid: &crate::grid::XyGrid, tris: &[[Vec3; 3]]) -> Vec<Vec<Vec3>> {
    let mut runs: Vec<Vec<Vec3>> = Vec::new();
    let mut cur: Vec<Vec3> = Vec::new();
    for w in rail.windows(2) {
        let (a, b) = (w[0], w[1]);
        let h = Vec3::new(b.x - a.x, b.y - a.y, 0.0);
        let hlen = h.length();
        if hlen < 1e-4 {
            continue;
        }
        let dir_xy = h / hlen;
        let steps = (hlen / 0.10).ceil().max(1.0) as usize;
        let flags: Vec<bool> = (0..steps)
            .map(|k| !on_wall_slam_shoulder(a.lerp(b, (k as f32 + 0.5) / steps as f32), dir_xy, grid, tris))
            .collect();
        if flags.iter().all(|&ok| ok) {
            push_visrail_seg(&mut runs, &mut cur, a, b);
        } else {
            for (k, &ok) in flags.iter().enumerate() {
                if ok {
                    push_visrail_seg(&mut runs, &mut cur, a.lerp(b, k as f32 / steps as f32), a.lerp(b, (k + 1) as f32 / steps as f32));
                } else {
                    flush_visrail_run(&mut runs, &mut cur);
                }
            }
        }
    }
    flush_visrail_run(&mut runs, &mut cur);
    runs.retain(|r| r.windows(2).map(|w| (w[1] - w[0]).length()).sum::<f32>() >= SHORTEST);
    runs
}

fn in_cells_reusing(size: f32, cell: impl Fn(Cell) -> Rails, centre: [f32; 2], half: f32, tris: &[[Vec3; 3]], grid: &crate::grid::XyGrid, bars: &[[Vec3; 3]], dirty: &crate::reuse::Dirty, cache: &mut crate::reuse::LineCache) -> Vec<Vec<Vec3>> {
    // A rail is filed under its middle point's cell: one cell more round.
    let span = |v: f32| ((v - half) / size).floor() as i32 - 1..=((v + half) / size).floor() as i32 + 1;
    let mut found: Vec<Rails> = Vec::new();
    for x in span(centre[0]) {
        for y in span(centre[1]) {
            let r = cell((x, y));
            if !r.is_empty() {
                found.push(r);
            }
        }
    }
    if found.is_empty() {
        return Vec::new();
    }
    let stands = |p: Vec3| {
        grid.at(p.x, p.y).iter().any(|&i| {
            let t = tris[i as usize];
            let n = (t[1] - t[0]).cross(t[2] - t[0]);
            if n.z < 0.5 * n.length().max(1e-9) { return false; }
            svwc::clean::floor_at(&svwc::Tri { v: t.map(|v| v.to_array()), material: 0 }, [p.x, p.y])
                .is_some_and(|z| FLOOR_BELOW.contains(&(p.z - z)))
        })
    };
    // A railing's top bar over a bowl has no floor in reach below its bends
    // (Vespucci 2026-11-01: 2.8 m down); GTA's box under it used to carry a
    // line there. Its own visible collision now does.
    let bar_grid = (!bars.is_empty()).then(|| crate::grid::XyGrid::triangles(1.0, bars));
    // ... and only at the top of the railing's mesh: no bar more than 10 cm
    // over it within 20 cm aside (peer review 2026-11-01: a bottom rail
    // overhanging a drop lies on its own mesh too).
    let on_bar = |p: Vec3| bar_grid.as_ref().is_some_and(|g| {
        let list = g.at(p.x, p.y);
        let on = list.iter().any(|&i| crate::world::closest_on_triangle(p, &bars[i as usize]).distance_squared(p) <= ON_BAR * ON_BAR);
        let up = p + Vec3::Z * 0.3;
        on && !list.iter().any(|&i| {
            let q = crate::world::closest_on_triangle(up, &bars[i as usize]);
            q.z > p.z + 0.10 && (q - p).truncate().length() <= 0.20
        })
    });
    // Only where no floor is in reach at all: a foot bar over its deck stays
    // out (goldfish 2026-11-01: it lies on its own bar mesh too).
    let floor_near = |p: Vec3| grid.at(p.x, p.y).iter().any(|&i| {
        let t = tris[i as usize];
        let n = (t[1] - t[0]).cross(t[2] - t[0]);
        n.z >= 0.5 * n.length().max(1e-9) && svwc::clean::floor_at(&svwc::Tri { v: t.map(|v| v.to_array()), material: 0 }, [p.x, p.y])
            .is_some_and(|z| (0.0..*FLOOR_BELOW.start()).contains(&(p.z - z)))
    });
    let stands = |p: Vec3| stands(p) || (!floor_near(p) && on_bar(p));
    let raw: Vec<_> = found.iter().flat_map(|r| r.iter())
        .filter(|r| r.iter().any(|p| (p.x - centre[0]).abs() <= half && (p.y - centre[1]).abs() <= half))
        .cloned().collect();
    cache.apply(&raw, dirty, 1.0, |r| {
        gentle_pieces(r)
            .into_iter()
            .filter(|r| {
                let mid = r[r.len() / 2];
                stands(r[0]) || stands(mid) || stands(r[r.len() - 1])
            })
            .flat_map(|r| trim_wall_slam_shoulders(&r, &grid, tris))
            .collect()
    })
}

/// A closed collision box under each rail's top line (six faces),
/// GTA metres, outward winding.
pub fn boxes(rails: &[Vec<Vec3>]) -> Vec<[Vec3; 3]> {
    boxes_deep(rails, None)
}

/// As [`boxes`], `depth` m deep when given (a track rail down to its bed).
pub fn boxes_deep(rails: &[Vec<Vec3>], depth: Option<f32>) -> Vec<[Vec3; 3]> {
    let mut out = Vec::new();
    for r in rails {
        for w in r.windows(2) {
            let (a, b) = (w[0], w[1]);
            let along = (b - a).normalize_or_zero();
            let half = crate::tubes::half_width(a, b).max(BOX_HALF);
            let side = Vec3::new(-along.y, along.x, 0.0).normalize_or_zero() * half;
            if side == Vec3::ZERO {
                continue;
            }
            let down = Vec3::Z * depth.unwrap_or(BOX_DEPTH.max(2.0 * half));
            // Corners at a and b: top-left, top-right, bottom-right, bottom-left.
            let ring = |p: Vec3| [p + side, p - side, p - side - down, p + side - down];
            let (pa, pb) = (ring(a), ring(b));
            let centre = (a + b) * 0.5 - down * 0.5;
            for k in 0..4 {
                let (i, j) = (k, (k + 1) % 4);
                let quad = [pa[i], pb[i], pb[j], pa[j]];
                // Outward: away from the box's centre line.
                let n = (quad[1] - quad[0]).cross(quad[2] - quad[0]);
                let face = (quad[0] + quad[2]) * 0.5 - centre;
                let quad = if n.dot(face) < 0.0 { [quad[0], quad[3], quad[2], quad[1]] } else { quad };
                out.push([quad[0], quad[1], quad[2]]);
                out.push([quad[0], quad[2], quad[3]]);
            }
            for mut quad in [pa, pb] {
                let n = (quad[1] - quad[0]).cross(quad[2] - quad[0]);
                if n.dot((quad[0] + quad[2]) * 0.5 - centre) < 0.0 {
                    quad = [quad[0], quad[3], quad[2], quad[1]];
                }
                out.push([quad[0], quad[1], quad[2]]);
                out.push([quad[0], quad[2], quad[3]]);
            }
        }
    }
    // Collinear joined segments share caps. Remove the internal pair and
    // keep one copy of identical boxes (the same rail from two sources).
    let mut unique: rustc_hash::FxHashMap<_, Option<[Vec3;3]>> = Default::default();
    for t in out {
        let mut key = t.map(|p| p.to_array().map(|v| if v == 0.0 { 0 } else { v.to_bits() }));
        key.sort_unstable();
        let entry = unique.entry(key).or_insert(Some(t));
        if let Some(old) = entry {
            if (old[1]-old[0]).cross(old[2]-old[0]).dot((t[1]-t[0]).cross(t[2]-t[0])) < 0.0 {
                *entry = None;
            }
        }
    }
    unique.into_values().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_receipt_rejects_old_or_changed_rail_data() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../out/tests/visual-policy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture.visrails");
        std::fs::write(&path, b"test rail data").unwrap();
        assert!(!verified_policy(&path), "old data without a receipt is rejected");
        write_policy(&path).unwrap();
        assert!(verified_policy(&path));
        let good = std::fs::read_to_string(policy_path(&path)).unwrap();
        std::fs::write(policy_path(&path), good.replace("StoryArchives=1", "StoryArchives=0")).unwrap();
        assert!(!verified_policy(&path));
        std::fs::write(policy_path(&path), good).unwrap();
        std::fs::write(&path, b"different rail data").unwrap();
        assert!(!verified_policy(&path), "a copied receipt cannot bless another file");
        std::fs::remove_file(policy_path(&path)).unwrap();
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn solid_boxes_are_closed_outward_and_reject_no_approach_direction() {
        let a = Vec3::new(0.0, 0.0, 1.0);
        let b = Vec3::new(2.0, 0.0, 1.5);
        let tris = boxes(&[vec![a, b]]);
        let centre = (a+b)*0.5 - Vec3::Z * BOX_DEPTH*0.5;
        let mut edges = std::collections::HashMap::new();
        for t in &tris {
            assert!((t[1]-t[0]).cross(t[2]-t[0]).dot((t[0]+t[1]+t[2])/3.0-centre) > 0.0);
            for i in 0..3 {
                let mut edge = [t[i].to_array().map(f32::to_bits), t[(i+1)%3].to_array().map(f32::to_bits)];
                edge.sort();
                *edges.entry(edge).or_insert(0) += 1;
            }
        }
        assert!(edges.values().all(|&count| count == 2), "no open ends or unpaired edges");
        assert!(boxes(&[vec![a,a]]).is_empty());
        assert_eq!(boxes(&[vec![a,b],vec![a,b]]).len(),12, "identical boxes have one surface");
        assert_eq!(boxes(&[vec![a,(a+b)*0.5,b]]).len(),20, "joined segments have no internal end caps");
    }

    #[test]
    fn steep_pieces_are_no_rails() {
        let v = |x: f32, z: f32| Vec3::new(x, 0.0, z);
        // A pole (vertical), a 35 degree handrail with a vertical post at its
        // end, and a short level piece between two steep ones.
        assert!(gentle_pieces(&[v(0., 0.), v(0., 3.)]).is_empty());
        let rail = gentle_pieces(&[v(0., 1.), v(2., 2.4), v(2., 3.4)]);
        assert_eq!(rail, vec![vec![v(0., 1.), v(2., 2.4)]]);
        assert!(gentle_pieces(&[v(0., 0.), v(0., 1.), v(0.2, 1.), v(0.2, 2.)]).is_empty());
    }

    #[test]
    fn reads_the_file_and_keeps_rails_over_floors_only() {
        let mut body = Vec::new();
        for r in [[[0.0f32, 0.0, 1.0], [5.0, 0.0, 1.0]], [[0.0, 3.0, 9.0], [5.0, 3.0, 9.0]]] {
            body.extend(2u32.to_le_bytes());
            for p in r {
                for x in p {
                    body.extend(x.to_le_bytes());
                }
            }
        }
        let mut file = b"SVRL".to_vec();
        file.extend(2u32.to_le_bytes());
        file.extend(32f32.to_le_bytes());
        file.extend(1u32.to_le_bytes());
        file.extend(0i32.to_le_bytes());
        file.extend(0i32.to_le_bytes());
        file.extend(2u32.to_le_bytes());
        file.extend(36u64.to_le_bytes());
        let total = 36 + body.len() as u64;
        let (size, t) = table(&file, total).unwrap();
        assert_eq!(t[&(0, 0)], (2, 36, body.len() as u64));
        let rails: Rails = std::sync::Arc::new(rails_at(&body, 2).unwrap());
        let v = Vec3::new;
        let floor = vec![[v(-10., -10., 0.), v(10., -10., 0.), v(10., 10., 0.)], [v(-10., -10., 0.), v(10., 10., 0.), v(-10., 10., 0.)]];
        let kept = in_cells(size, |c| if c == (0, 0) { rails.clone() } else { Default::default() }, [0.0, 0.0], 20.0, &floor, &[]);
        assert_eq!(kept.len(), 1, "the 1 m handrail stays, the 9 m high bar goes");
        let b = boxes(&kept);
        assert_eq!(b.len(), 12);
        assert!(b.iter().all(|t| t.iter().all(|p| p.z <= 1.0 + 1e-6 && p.z >= 1.0 - BOX_DEPTH - 1e-6)));
        // The high bar on a visible railing's collision (its top 5 cm under
        // the line) stands on it; 30 cm under, it does not.
        let bar = |z: f32| vec![[v(0., 2.95, z), v(5., 2.95, z), v(5., 3.05, z)]];
        let rail = |c: Cell| if c == (0, 0) { rails.clone() } else { Default::default() };
        assert_eq!(in_cells(size, rail, [0.0, 0.0], 20.0, &floor, &bar(8.95)).len(), 2);
        assert_eq!(in_cells(size, rail, [0.0, 0.0], 20.0, &floor, &bar(8.7)).len(), 1);
        // A foot bar 10 cm over the floor, on its own bar mesh: still no rail.
        let foot = vec![[v(0., 2.95, 8.95), v(5., 2.95, 8.95), v(5., 3.05, 8.95)]];
        let deck = vec![[v(-10., 2., 8.9), v(10., 2., 8.9), v(10., 4., 8.9)], [v(-10., 2., 8.9), v(10., 4., 8.9), v(-10., 4., 8.9)]];
        assert!(in_cells(size, rail, [0.0, 0.0], 20.0, &deck, &foot).iter().all(|r| r[0].z < 5.));
        // A bottom bar over a drop with its top bar 0.9 m above: no rail; the top bar is one.
        let two = vec![[v(0., 2.95, 8.95), v(5., 2.95, 8.95), v(5., 3.05, 8.95)], [v(0., 2.95, 9.85), v(5., 2.95, 9.85), v(5., 3.05, 9.85)]];
        assert_eq!(in_cells(size, rail, [0.0, 0.0], 20.0, &floor, &two).len(), 1, "only the 1 m handrail on the floor");
    }

    #[test]
    fn a_visrail_on_a_lower_barrier_shoulder_next_to_upper_wall_is_trimmed_while_stair_handrail_stays() {
        let v = Vec3::new;
        let quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| vec![[a, b, c], [a, c, d]];
        // Road slab at z = 0.0, jersey-barrier lower shoulder at z = 0.65 (y = 0.0..0.20),
        // and vertical upper barrier wall at y = 0.20 rising from z = 0.65 to z = 1.00.
        let mut tris = Vec::new();
        tris.extend(quad(v(-5., -5., 0.), v(5., -5., 0.), v(5., 5., 0.), v(-5., 5., 0.)));
        tris.extend(quad(v(-4., 0., 0.65), v(4., 0., 0.65), v(4., 0.20, 0.65), v(-4., 0.20, 0.65)));
        tris.extend(quad(v(-4., 0.20, 0.65), v(4., 0.20, 0.65), v(4., 0.20, 1.00), v(-4., 0.20, 1.00)));

        // Mark 1111-3: visible trim at outer edge of lower shoulder (z = 0.651, y = 0.015),
        // plus a freestanding handrail 1.5 m away at z = 0.90 (0.90 m above road) beside a wall.
        tris.extend(quad(v(-4., -1.70, 0.), v(4., -1.70, 0.), v(4., -1.70, 1.50), v(-4., -1.70, 1.50)));
        let rails: Rails = std::sync::Arc::new(vec![
            vec![v(-3., 0.015, 0.651), v(3., 0.015, 0.651)],
            vec![v(-3., -1.52, 0.90), v(3., -1.52, 0.90)],
        ]);
        let kept = in_cells(32.0, |c| if c == (0, 0) { rails.clone() } else { Default::default() }, [0.0, 0.0], 10.0, &tris, &[]);
        assert_eq!(kept.len(), 1, "lower barrier shoulder visrail dropped, stair handrail kept: {kept:?}");
        assert!((kept[0][0].y - -1.52).abs() < 1e-4);

        // A freestanding rail 18 cm in front of a wall (with open air between them)
        // must remain un-trimmed.
        let mut free_tris = Vec::new();
        free_tris.extend(quad(v(-5., -5., 0.), v(5., -5., 0.), v(5., 5., 0.), v(-5., 5., 0.)));
        free_tris.extend(quad(v(-4., 0., 0.90), v(4., 0., 0.90), v(4., 0.06, 0.90), v(-4., 0.06, 0.90)));
        free_tris.extend(quad(v(-4., 0.24, 0.), v(4., 0.24, 0.), v(4., 0.24, 1.50), v(-4., 0.24, 1.50)));
        let free_rails: Rails = std::sync::Arc::new(vec![vec![v(-3., 0.005, 0.901), v(3., 0.005, 0.901)]]);
        let kept_free = in_cells(32.0, |c| if c == (0, 0) { free_rails.clone() } else { Default::default() }, [0.0, 0.0], 10.0, &free_tris, &[]);
        assert_eq!(kept_free.len(), 1, "freestanding visrail beside open air 18 cm from wall stays: {kept_free:?}");

        // Trimmed 20 cm pillar shoulder gap between two >= 30 cm pieces must NOT rejoin in guarded join.
        let mut pillar_tris = Vec::new();
        pillar_tris.extend(quad(v(-5., -5., 0.), v(5., -5., 0.), v(5., 5., 0.), v(-5., 5., 0.)));
        pillar_tris.extend(quad(v(-2., 0., 0.65), v(2., 0., 0.65), v(2., 0.20, 0.65), v(-2., 0.20, 0.65)));
        pillar_tris.extend(quad(v(-0.10, 0.16, 0.65), v(0.10, 0.16, 0.65), v(0.10, 0.16, 1.00), v(-0.10, 0.16, 1.00)));
        let pillar_grid = crate::grid::XyGrid::triangles(1.0, &pillar_tris);
        let pillar_rail: Rails = std::sync::Arc::new(vec![vec![v(-1.5, 0.015, 0.651), v(1.5, 0.015, 0.651)]]);
        let mut split = in_cells(32.0, |c| if c == (0, 0) { pillar_rail.clone() } else { Default::default() }, [0.0, 0.0], 10.0, &pillar_tris, &[]);
        assert_eq!(split.len(), 2, "20 cm blocked pillar trims middle without dropping halves: {split:?}");
        assert_eq!(crate::rails::join_ends_guarded(&mut split, &|p, q| p.distance(q) <= 0.05 || !shoulder_blocked_between(p, q, &pillar_grid, &pillar_tris)), 0);
        assert_eq!(split.len(), 2, "guarded join refuses to bridge back across trimmed 20 cm shoulder: {split:?}");
    }
}
