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
pub(crate) fn policy_text_for(path: &Path) -> std::io::Result<String> { policy_text(path) }

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
    in_cells_reusing(file.cell, |c| file.cell_rails(c), centre, half, tris, &grid, &crate::reuse::Dirty::everything(), &mut crate::reuse::LineCache::default())
}

/// Test source supplying cells directly.
#[cfg(test)]
fn in_cells(size: f32, cell: impl Fn(Cell) -> Rails, centre: [f32; 2], half: f32, tris: &[[Vec3; 3]]) -> Vec<Vec<Vec3>> {
    let grid = crate::grid::XyGrid::triangles(1.0, tris);
    in_cells_reusing(size, cell, centre, half, tris, &grid, &crate::reuse::Dirty::everything(), &mut crate::reuse::LineCache::default())
}

/// Rails with support checks reused where nearby geometry is unchanged.
pub fn rails_in_reusing(centre: [f32; 2], half: f32, tris: &[[Vec3; 3]], grid: &crate::grid::XyGrid, dirty: &crate::reuse::Dirty, cache: &mut crate::reuse::LineCache) -> Vec<Vec<Vec3>> {
    if !ENABLED.load(std::sync::atomic::Ordering::Relaxed) { return Vec::new(); }
    let Some(Some(file)) = FILE.get() else { return Vec::new() };
    in_cells_reusing(file.cell, |c| file.cell_rails(c), centre, half, tris, grid, dirty, cache)
}

fn in_cells_reusing(size: f32, cell: impl Fn(Cell) -> Rails, centre: [f32; 2], half: f32, tris: &[[Vec3; 3]], grid: &crate::grid::XyGrid, dirty: &crate::reuse::Dirty, cache: &mut crate::reuse::LineCache) -> Vec<Vec<Vec3>> {
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
    let raw: Vec<_> = found.iter().flat_map(|r| r.iter())
        .filter(|r| r.iter().any(|p| (p.x - centre[0]).abs() <= half && (p.y - centre[1]).abs() <= half))
        .cloned().collect();
    cache.apply(&raw, dirty, 1.0, |r| {
        gentle_pieces(r).into_iter().filter(|r| {
            let mid = r[r.len() / 2];
            stands(r[0]) || stands(mid) || stands(r[r.len() - 1])
        }).collect()
    })
}

/// A closed collision box under each rail's top line (six faces),
/// GTA metres, outward winding.
pub fn boxes(rails: &[Vec<Vec3>]) -> Vec<[Vec3; 3]> {
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
            let down = Vec3::Z * BOX_DEPTH.max(2.0 * half);
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
        let kept = in_cells(size, |c| if c == (0, 0) { rails.clone() } else { Default::default() }, [0.0, 0.0], 20.0, &floor);
        assert_eq!(kept.len(), 1, "the 1 m handrail stays, the 9 m high bar goes");
        let b = boxes(&kept);
        assert_eq!(b.len(), 12);
        assert!(b.iter().all(|t| t.iter().all(|p| p.z <= 1.0 + 1e-6 && p.z >= 1.0 - BOX_DEPTH - 1e-6)));
    }
}
