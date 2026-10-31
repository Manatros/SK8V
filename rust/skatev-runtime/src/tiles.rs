//! Final static collision tiles. All numbers are little endian, GTA metres.
//! SVWT v2: magic + version u32, then a zlib stream containing the v1 payload.
//! Payload: magic, version 1 u32, tile size f32, x/y i32, source UTF-8 length
//! u32 + bytes, triangle count u32 (9 f32 + tag u32 each), rail count u32
//! (point count u32 + 3 f32 per point). Payload version 2 then has the stair
//! nose rails in the same form (StepRails); version 1 has none. Geometry is
//! split at tile borders.
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

pub type Triangle = [[f32; 3]; 3];
pub type Cell = (i32, i32);
pub const VERSION: u32 = 3;
const MAX_PAYLOAD: u64 = 128 * 1024 * 1024;

/// Shared by writer and runtime; changing any static rule invalidates old tiles.
pub fn rules_hash() -> String {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HASH.get_or_init(|| {
        let mut h = 0xcbf29ce484222325u64;
        for bytes in [
            include_bytes!("world.rs").as_slice(),
            include_bytes!("live.rs").as_slice(),
            include_bytes!("baked.rs").as_slice(),
            include_bytes!("rails.rs").as_slice(),
            include_bytes!("curbs.rs").as_slice(),
            include_bytes!("stairs.rs").as_slice(),
            include_bytes!("reuse.rs").as_slice(),
            include_bytes!("grid.rs").as_slice(),
            include_bytes!("plies.rs").as_slice(),
            include_bytes!("tracks.rs").as_slice(),
            include_bytes!("visrails.rs").as_slice(),
            include_bytes!("visparks.rs").as_slice(),
            include_bytes!("tubes.rs").as_slice(),
            include_bytes!("materials.rs").as_slice(),
            include_bytes!("surfaces.rs").as_slice(),
            include_bytes!("coords.rs").as_slice(),
            include_bytes!("parks.rs").as_slice(),
            include_bytes!("../../svwc/src/bounds.rs").as_slice(),
            include_bytes!("../../svwc/src/clean.rs").as_slice(),
            // Extraction, placement and archive-state selection determine the
            // geometry just as much as the runtime's final surface rules.
            include_bytes!("../../world-cache/src/tile_bake.rs").as_slice(),
            include_bytes!("../../world-cache/src/main.rs").as_slice(),
            include_bytes!("../../world-cache/src/interiors.rs").as_slice(),
            include_bytes!("../../world-cache/src/interiors/doors.rs").as_slice(),
            include_bytes!("../../world-cache/src/vfs.rs").as_slice(),
            include_bytes!("../../world-cache/src/packs.rs").as_slice(),
            include_bytes!("../../world-cache/src/primitives.rs").as_slice(),
            include_bytes!("../../world-cache/src/props.rs").as_slice(),
            include_bytes!("../../world-cache/src/bin/visual-rails.rs").as_slice(),
            include_bytes!("../../world-cache/src/bars.rs").as_slice(),
        ] {
            h = hash_rule_bytes(h, bytes);
        }
        format!("rules {h:016x}")
    })
    .clone()
}
fn hash_rule_bytes(mut h: u64, bytes: &[u8]) -> u64 {
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\r' && bytes.get(i + 1) == Some(&b'\n') { continue; }
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    h
}

pub fn rules_compatible(s: &str) -> bool {
    s.trim() == rules_hash()
}

fn source_generation(source: &str) -> String {
    source.lines().collect::<Vec<_>>().join("\n")
}
pub fn compatible(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("rules.txt"))
        .ok()
        .is_some_and(|s| rules_compatible(&s))
}

/// State-dependent map regions must be supplied by GTA's active physics state.
/// Bind this mandatory policy file to the same rules and input inventory as
/// the tiles, so a stale/missing mask cannot silently restore phantom walls.
pub fn write_live_cells(dir: &Path, size: f32, source: &str, cells: &BTreeSet<Cell>) -> io::Result<()> {
    let body = live_cells_body(cells);
    let checksum = hash_rule_bytes(0xcbf29ce484222325, body.as_bytes());
    let text = format!("SVWT-LIVE 1\n{}\n{}\nsize {size}\ncells {} {checksum:016x}\n{body}",
        rules_hash(), source.lines().next().unwrap_or(""), cells.len());
    std::fs::write(dir.join("live-cells.txt"), text)
}

fn live_cells_body(cells: &BTreeSet<Cell>) -> String {
    cells.iter().map(|&(x, y)| format!("{x} {y}\n")).collect()
}

pub fn live_policy(cells: &BTreeSet<Cell>) -> String {
    format!("live-policy {:016x}", hash_rule_bytes(0xcbf29ce484222325, live_cells_body(cells).as_bytes()))
}

fn read_live_cells(dir: &Path, size: f32) -> Option<(String, String, BTreeSet<Cell>)> {
    let text = std::fs::read_to_string(dir.join("live-cells.txt")).ok()?;
    let mut lines = text.lines();
    if lines.next()? != "SVWT-LIVE 1" || !rules_compatible(lines.next()?) { return None; }
    let source = lines.next()?.to_owned();
    if source.is_empty() { return None; }
    let declared: f32 = lines.next()?.strip_prefix("size ")?.parse().ok()?;
    if declared != size { return None; }
    let mut integrity = lines.next()?.split_whitespace();
    if integrity.next()? != "cells" { return None; }
    let count: usize = integrity.next()?.parse().ok()?;
    let checksum = u64::from_str_radix(integrity.next()?, 16).ok()?;
    if integrity.next().is_some() { return None; }
    let mut cells = BTreeSet::new();
    for line in lines {
        let mut fields = line.split_whitespace();
        let c = (fields.next()?.parse().ok()?, fields.next()?.parse().ok()?);
        if fields.next().is_some() || !cells.insert(c) { return None; }
    }
    if cells.len() != count || hash_rule_bytes(0xcbf29ce484222325, live_cells_body(&cells).as_bytes()) != checksum {
        return None;
    }
    Some((source, format!("live-policy {checksum:016x}"), cells))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    pub cell: Cell,
    pub size: f32,
    /// Source names, content hashes, and rule configuration.
    pub source: String,
    pub triangles: Vec<(Triangle, u32)>,
    pub rails: Vec<Vec<[f32; 3]>>,
    /// Stair nose lines, grind rails only with StepRails on.
    pub noses: Vec<Vec<[f32; 3]>>,
    /// Per triangle, an index into `origin_names`: the GTA file it was
    /// decoded from, `visible:<park>`, `cut:<file>` or `rebuilt` (design
    /// 2026-10-26-one-source-collision 6.2). Empty: not recorded.
    pub origins: Vec<u16>,
    pub origin_names: Vec<String>,
}

impl Tile {
    /// The origin name of triangle `i` ("" when not recorded).
    pub fn origin(&self, i: usize) -> &str {
        self.origins.get(i).and_then(|&k| self.origin_names.get(k as usize)).map_or("", |s| s.as_str())
    }
}

pub fn cell(p: [f32; 3], size: f32) -> Cell {
    ((p[0] / size).floor() as i32, (p[1] / size).floor() as i32)
}

pub fn cells(center: [f32; 2], half: f32, size: f32) -> Vec<Cell> {
    let lo = cell([center[0] - half, center[1] - half, 0.], size);
    let hi = cell([center[0] + half, center[1] + half, 0.], size);
    (lo.0..=hi.0)
        .flat_map(|x| (lo.1..=hi.1).map(move |y| (x, y)))
        .collect()
}

pub fn path(dir: &Path, c: Cell) -> PathBuf {
    dir.join(format!("{}_{}.svwt", c.0, c.1))
}

/// Clip a face to one tile. A face on a border belongs to the positive side.
pub fn clip_triangle(t: Triangle, c: Cell, size: f32) -> Vec<Triangle> {
    let mut poly = t.to_vec();
    for (axis, low) in [(0, c.0 as f32 * size), (1, c.1 as f32 * size)] {
        for (bound, sign) in [(low, 1.), (low + size, -1.)] {
            let old = std::mem::take(&mut poly);
            if old.is_empty() {
                return Vec::new();
            }
            for i in 0..old.len() {
                let (a, b) = (old[i], old[(i + 1) % old.len()]);
                let (da, db) = ((a[axis] - bound) * sign, (b[axis] - bound) * sign);
                if da >= 0. {
                    poly.push(a);
                }
                if (da < 0.) != (db < 0.) {
                    let f = da / (da - db);
                    let mut p = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * f);
                    p[axis] = bound;
                    poly.push(p);
                }
            }
        }
    }
    let mut out = Vec::new();
    for i in 1..poly.len().saturating_sub(1) {
        let t = [poly[0], poly[i], poly[i + 1]];
        let v = t.map(bevy_math::Vec3::from_array);
        let mid = ((v[0] + v[1] + v[2]) / 3.).to_array();
        if cell(mid, size) == c && (v[1] - v[0]).cross(v[2] - v[0]).length_squared() > 1e-10 {
            out.push(t);
        }
    }
    out
}

/// Split a line at grid planes. Each nonzero segment has one owner.
pub fn split_rail(rail: &[[f32; 3]], size: f32) -> Vec<(Cell, Vec<[f32; 3]>)> {
    let mut out: Vec<(Cell, Vec<[f32; 3]>)> = Vec::new();
    for w in rail.windows(2) {
        let (a, b) = (w[0], w[1]);
        let mut cuts = vec![0., 1.];
        for k in 0..2 {
            if a[k] == b[k] {
                continue;
            }
            let lo = (a[k].min(b[k]) / size).floor() as i32;
            let hi = (a[k].max(b[k]) / size).floor() as i32;
            for n in lo + 1..=hi {
                let f = (n as f32 * size - a[k]) / (b[k] - a[k]);
                if f > 0. && f < 1. {
                    cuts.push(f);
                }
            }
        }
        cuts.sort_by(f32::total_cmp);
        cuts.dedup();
        for f in cuts.windows(2) {
            let p = |f: f32| {
                if f == 0. {
                    a
                } else if f == 1. {
                    b
                } else {
                    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * f)
                }
            };
            if f[1] - f[0] > 1e-7 {
                let owner = cell(p((f[0] + f[1]) * 0.5), size);
                let (start, end) = (p(f[0]), p(f[1]));
                if let Some((last_owner, rail)) = out.last_mut()
                    && *last_owner == owner
                    && rail.last() == Some(&start)
                {
                    rail.push(end);
                } else {
                    out.push((owner, vec![start, end]));
                }
            }
        }
    }
    out
}

impl Tile {
    pub fn write(&self, mut w: impl Write) -> io::Result<()> {
        w.write_all(b"SVWT")?;
        w.write_all(&VERSION.to_le_bytes())?;
        let mut encoder = flate2::write::ZlibEncoder::new(w, flate2::Compression::best());
        self.write_raw(&mut encoder)?;
        encoder.finish()?;
        Ok(())
    }

    fn write_raw(&self, mut w: impl Write) -> io::Result<()> {
        w.write_all(b"SVWT")?;
        for n in [
            3,
            self.size.to_bits(),
            self.cell.0 as u32,
            self.cell.1 as u32,
            self.source.len() as u32,
        ] {
            w.write_all(&n.to_le_bytes())?;
        }
        w.write_all(self.source.as_bytes())?;
        w.write_all(&(self.triangles.len() as u32).to_le_bytes())?;
        for (t, tag) in &self.triangles {
            for p in t {
                for n in p {
                    w.write_all(&n.to_le_bytes())?;
                }
            }
            w.write_all(&tag.to_le_bytes())?;
        }
        for lines in [&self.rails, &self.noses] {
            w.write_all(&(lines.len() as u32).to_le_bytes())?;
            for rail in lines {
                w.write_all(&(rail.len() as u32).to_le_bytes())?;
                for p in rail {
                    for n in p {
                        w.write_all(&n.to_le_bytes())?;
                    }
                }
            }
        }
        w.write_all(&(self.origin_names.len() as u32).to_le_bytes())?;
        for name in &self.origin_names {
            w.write_all(&(name.len() as u32).to_le_bytes())?;
            w.write_all(name.as_bytes())?;
        }
        w.write_all(&(self.origins.len() as u32).to_le_bytes())?;
        for o in &self.origins {
            w.write_all(&o.to_le_bytes())?;
        }
        Ok(())
    }

    pub fn read(mut r: impl Read) -> io::Result<Self> {
        let mut header = [0; 8];
        r.read_exact(&mut header)?;
        if &header[..4] != b"SVWT" || u32::from_le_bytes(header[4..].try_into().unwrap()) != VERSION
        {
            return Err(io::Error::other("unsupported world tile"));
        }
        let mut decoder = flate2::bufread::ZlibDecoder::new(io::BufReader::new(r));
        let mut raw = Vec::new();
        (&mut decoder).take(MAX_PAYLOAD + 1).read_to_end(&mut raw)?;
        if raw.len() as u64 > MAX_PAYLOAD {
            return Err(io::Error::other("tile payload exceeds limit"));
        }
        let mut tail = [0];
        if decoder.into_inner().read(&mut tail)? != 0 {
            return Err(io::Error::other("trailing compressed data"));
        }
        Self::read_raw(raw.as_slice())
    }

    fn read_raw(mut r: &[u8]) -> io::Result<Self> {
        fn word(r: &mut impl Read) -> io::Result<u32> {
            let mut b = [0; 4];
            r.read_exact(&mut b)?;
            Ok(u32::from_le_bytes(b))
        }
        fn count(r: &mut impl Read, max: u32) -> io::Result<usize> {
            let n = word(r)?;
            if n > max {
                return Err(io::Error::other("tile count exceeds limit"));
            }
            Ok(n as usize)
        }
        fn point(r: &mut impl Read) -> io::Result<[f32; 3]> {
            let p = [
                f32::from_bits(word(r)?),
                f32::from_bits(word(r)?),
                f32::from_bits(word(r)?),
            ];
            if p.iter().any(|v| !v.is_finite()) {
                return Err(io::Error::other("non-finite tile point"));
            }
            Ok(p)
        }
        let mut magic = [0; 4];
        r.read_exact(&mut magic)?;
        let payload = if magic == *b"SVWT" { word(&mut r)? } else { 0 };
        // Payload 3 adds face origins; older tiles come from another bake.
        if payload != 3 {
            return Err(io::Error::other("unsupported world tile"));
        }
        let size = f32::from_bits(word(&mut r)?);
        if !size.is_finite() || !(1. ..=4096.).contains(&size) {
            return Err(io::Error::other("invalid tile size"));
        }
        let cell = (word(&mut r)? as i32, word(&mut r)? as i32);
        let mut source = vec![0; count(&mut r, 16_000_000)?];
        r.read_exact(&mut source)?;
        let source = String::from_utf8(source).map_err(io::Error::other)?;
        let mut triangles = Vec::new();
        for _ in 0..count(&mut r, 4_000_000)? {
            triangles.push((
                [point(&mut r)?, point(&mut r)?, point(&mut r)?],
                word(&mut r)?,
            ));
        }
        let lines = |r: &mut &[u8]| -> io::Result<Vec<Vec<[f32; 3]>>> {
            let mut rails = Vec::new();
            for _ in 0..count(r, 1_000_000)? {
                let n = count(r, 1_000_000)?;
                if n < 2 {
                    return Err(io::Error::other("short tile rail"));
                }
                let mut rail = Vec::new();
                for _ in 0..n {
                    rail.push(point(r)?);
                }
                rails.push(rail);
            }
            Ok(rails)
        };
        let rails = lines(&mut r)?;
        let noses = lines(&mut r)?;
        let mut origin_names = Vec::new();
        for _ in 0..count(&mut r, 65_536)? {
            let mut name = vec![0; count(&mut r, 4096)?];
            r.read_exact(&mut name)?;
            origin_names.push(String::from_utf8(name).map_err(io::Error::other)?);
        }
        let n = count(&mut r, 4_000_000)?;
        if n != 0 && n != triangles.len() {
            return Err(io::Error::other("tile origin count differs from its triangles"));
        }
        let mut origins = Vec::with_capacity(n);
        for _ in 0..n {
            let mut b = [0; 2];
            r.read_exact(&mut b)?;
            let o = u16::from_le_bytes(b);
            if o as usize >= origin_names.len() {
                return Err(io::Error::other("tile origin index past its name table"));
            }
            origins.push(o);
        }
        let mut tail = [0];
        if r.read(&mut tail)? != 0 {
            return Err(io::Error::other("trailing tile data"));
        }
        Ok(Self {
            cell,
            size,
            source,
            triangles,
            rails,
            noses,
            origins,
            origin_names,
        })
    }
}

/// An unreadable or mismatched tile uses live collision, independently of neighbours.
pub struct Selection {
    pub size: f32,
    pub baked: BTreeMap<Cell, std::sync::Arc<Tile>>,
    pub missing: Vec<Cell>,
    pub live_only: Vec<Cell>,
    /// Tiles present on disk but from another bake (source, rules or map
    /// policy differ): they fall back to live collision. Non-zero means an
    /// incomplete or mixed installation (2026-10-23: only two slices rebaked).
    pub stale: usize,
}

impl Selection {
    pub fn load(dir: &Path, center: [f32; 2], half: f32) -> Option<Self> {
        Cache::default().select(dir, center, half)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_and_bad_version() {
        let t = Tile { origins: Vec::new(), origin_names: Vec::new(), noses: vec![vec![[0., 0.3, 1.2], [1., 0.3, 1.2]]],
            cell: (-2, 3),
            size: 64.,
            source: "rules v1; test:123".into(),
            triangles: vec![([[0., 0., 1.], [1., 0., 1.], [0., 1., 1.]], 0x8000_1234)],
            rails: vec![vec![[0., 0., 1.], [1., 0., 1.]]],
        };
        let mut b = Vec::new();
        t.write(&mut b).unwrap();
        assert_eq!(Tile::read(b.as_slice()).unwrap(), t);
        assert!(Tile::read(&b[..b.len() - 1]).is_err());
        b[4] = 99;
        assert!(Tile::read(b.as_slice()).is_err());
    }
    #[test]
    fn border_split_conserves_area_and_length() {
        let t = [[-1., -1., 0.], [2., -1., 0.], [-1., 2., 0.]];
        let area = |t: Triangle| {
            let p = t.map(bevy_math::Vec3::from_array);
            (p[1] - p[0]).cross(p[2] - p[0]).length() / 2.
        };
        let parts: Vec<_> = cells([0., 0.], 3., 1.)
            .into_iter()
            .flat_map(|c| clip_triangle(t, c, 1.).into_iter().map(move |t| (c, t)))
            .collect();
        assert!((parts.iter().map(|(_, t)| area(*t)).sum::<f32>() - area(t)).abs() < 1e-6);
        for (c, t) in &parts {
            assert_eq!(
                cell(
                    std::array::from_fn(|k| (t[0][k] + t[1][k] + t[2][k]) / 3.),
                    1.
                ),
                *c
            );
        }
        let parts = split_rail(&[[-2., 0., 0.], [2., 0., 0.]], 1.);
        assert_eq!(parts.len(), 4);
        assert_eq!(
            parts.iter().map(|(_, r)| r[1][0] - r[0][0]).sum::<f32>(),
            4.
        );
        let vertical = [[0., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        assert!(clip_triangle(vertical, (-1, 0), 1.).is_empty());
        assert_eq!(clip_triangle(vertical, (0, 0), 1.).len(), 1);
    }
    #[test]
    fn missing_and_corrupt_tiles_use_live() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../out/tests/tiles-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tile-size.txt"), "64").unwrap();
        std::fs::write(dir.join("rules.txt"), rules_hash()).unwrap();
        write_live_cells(&dir, 64., &rules_hash(), &BTreeSet::new()).unwrap();
        let t = Tile { origins: Vec::new(), origin_names: Vec::new(), noses: Vec::new(),
            cell: (0, 0),
            size: 64.,
            source: format!("{}\n{}", rules_hash(), live_policy(&BTreeSet::new())),
            triangles: vec![],
            rails: vec![],
        };
        t.write(std::fs::File::create(path(&dir, t.cell)).unwrap())
            .unwrap();
        std::fs::write(path(&dir, (1, 0)), b"corrupt").unwrap();
        let s = Selection::load(&dir, [64., 32.], 31.).unwrap();
        assert_eq!(s.baked.keys().copied().collect::<Vec<_>>(), [(0, 0)]);
        assert_eq!(s.missing, [(1, 0)]);
        std::fs::remove_file(path(&dir, (0, 0))).unwrap();
        assert!(
            Selection::load(&dir, [64., 32.], 31.)
                .unwrap()
                .baked
                .is_empty()
        );
        std::fs::remove_file(path(&dir, (1, 0))).unwrap();
        std::fs::remove_file(dir.join("tile-size.txt")).unwrap();
        std::fs::remove_file(dir.join("rules.txt")).unwrap();
        std::fs::remove_file(dir.join("live-cells.txt")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}

/// Keep only the missing tiles of the processed live result, split at borders.
pub fn keep_live(mut area: crate::world::Area, selection: &Selection) -> crate::world::Area {
    use crate::coords;
    use bevy_math::Vec3;
    let mut triangles = Vec::new();
    let mut tags = Vec::new();
    for (t, tag) in area.triangles.iter().zip(&area.tags) {
        let gta = t.map(|p| coords::from_skate(Vec3::from_array(p)).to_array());
        for &c in &selection.missing {
            for t in clip_triangle(gta, c, selection.size) {
                triangles.push(t.map(|p| coords::to_skate(Vec3::from_array(p)).to_array()));
                // Marked: the collision view shows what is not from the bake.
                tags.push(*tag | crate::live::LIVE_TAG);
            }
        }
    }
    let clip_lines = |lines: &[Vec<[f32; 3]>]| -> Vec<Vec<[f32; 3]>> {
        let mut out = Vec::new();
        for r in lines {
            let gta: Vec<_> = r.iter().map(|p| coords::from_skate(Vec3::from_array(*p)).to_array()).collect();
            for (c, r) in split_rail(&gta, selection.size) {
                if selection.missing.contains(&c) {
                    out.push(r.into_iter().map(|p| coords::to_skate(Vec3::from_array(p)).to_array()).collect());
                }
            }
        }
        out
    };
    area.noses = clip_lines(&area.noses);
    let mut rails = Vec::new();
    for r in &area.rails {
        let gta: Vec<_> = r
            .iter()
            .map(|p| coords::from_skate(Vec3::from_array(*p)).to_array())
            .collect();
        for (c, r) in split_rail(&gta, selection.size) {
            if selection.missing.contains(&c) {
                rails.push(
                    r.into_iter()
                        .map(|p| coords::to_skate(Vec3::from_array(p)).to_array())
                        .collect(),
                );
            }
        }
    }
    area.triangles = triangles;
    area.tags = tags;
    area.rails = rails;
    area
}

/// Decoded tile LRU. Only the collision builder thread performs disk I/O.
pub struct Cache {
    entries: BTreeMap<Cell, (std::sync::Arc<Tile>, u64, usize)>,
    clock: u64,
    pub budget: usize,
    pub bytes: usize,
}
impl Default for Cache {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            clock: 0,
            budget: 96 * 1024 * 1024,
            bytes: 0,
        }
    }
}
impl Cache {
    fn get(&mut self, dir: &Path, c: Cell, size: f32) -> Option<std::sync::Arc<Tile>> {
        self.clock += 1;
        if let Some((t, used, _)) = self.entries.get_mut(&c) {
            *used = self.clock;
            return Some(t.clone());
        }
        let t = Tile::read(io::BufReader::new(std::fs::File::open(path(dir, c)).ok()?)).ok()?;
        if t.cell != c || t.size != size || !t.source.lines().any(rules_compatible) {
            return None;
        }
        let bytes = t.source.capacity()
            + t.triangles.capacity() * std::mem::size_of::<(Triangle, u32)>()
            + t.rails.capacity() * std::mem::size_of::<Vec<[f32; 3]>>()
            + t.rails.iter().map(|r| r.capacity() * 12).sum::<usize>();
        let t = std::sync::Arc::new(t);
        if bytes <= self.budget {
            while self.bytes + bytes > self.budget {
                let oldest = *self.entries.iter().min_by_key(|(_, v)| v.1)?.0;
                self.bytes -= self.entries.remove(&oldest)?.2;
            }
            self.bytes += bytes;
            self.entries.insert(c, (t.clone(), self.clock, bytes));
        }
        Some(t)
    }
    pub fn select(&mut self, dir: &Path, center: [f32; 2], half: f32) -> Option<Selection> {
        if !compatible(dir) {
            self.entries.clear();
            self.bytes = 0;
            return None;
        }
        let size: f32 = std::fs::read_to_string(dir.join("tile-size.txt"))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        if !size.is_finite() || !(1. ..=4096.).contains(&size) {
            return None;
        }
        let (policy_source, policy_line, live_cells) = read_live_cells(dir, size)?;
        // A teleport should discard far tiles immediately, even below budget.
        let near = cell([center[0], center[1], 0.], size);
        let reach = (half * 3. / size).ceil() as u32 + 1;
        self.entries.retain(|c, (_, _, bytes)| {
            let keep = c.0.abs_diff(near.0) <= reach && c.1.abs_diff(near.1) <= reach;
            if !keep {
                self.bytes -= *bytes;
            }
            keep
        });
        let mut s = Selection {
            size,
            baked: BTreeMap::new(),
            missing: Vec::new(),
            live_only: Vec::new(),
            stale: 0,
        };
        let mut source = None;
        for c in cells(center, half, size) {
            if live_cells.contains(&c) {
                s.missing.push(c);
                s.live_only.push(c);
                continue;
            }
            if let Some(t) = self.get(dir, c, size)
                .filter(|t| t.source.lines().next() == Some(policy_source.as_str())
                    && t.source.lines().any(|line| line == policy_line)
                    && source.as_ref().is_none_or(|s| s == &source_generation(&t.source))) {
                source.get_or_insert_with(|| source_generation(&t.source));
                s.baked.insert(c, t);
            } else {
                if path(dir, c).exists() {
                    s.stale += 1;
                }
                s.missing.push(c);
            }
        }
        Some(s)
    }
    pub fn prefetch(&mut self, dir: &Path, center: [f32; 2], lead: [f32; 2], half: f32) {
        if !compatible(dir) {
            return;
        }
        let Ok(text) = std::fs::read_to_string(dir.join("tile-size.txt")) else {
            return;
        };
        let Ok(size) = text.trim().parse::<f32>() else {
            return;
        };
        if !size.is_finite() || !(1. ..=4096.).contains(&size) {
            return;
        }
        for c in prefetch_cells(center, lead, half, size) {
            self.get(dir, c, size);
        }
    }
}
/// Swept footprint to twice the build lead; near tiles are read first.
pub fn prefetch_cells(center: [f32; 2], lead: [f32; 2], half: f32, size: f32) -> Vec<Cell> {
    let steps = ((lead[0].abs().max(lead[1].abs()) * 2. / size).ceil() as usize).max(1);
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for i in 0..=steps {
        let f = 2. * i as f32 / steps as f32;
        for c in cells(
            [center[0] + lead[0] * f, center[1] + lead[1] * f],
            half,
            size,
        ) {
            if seen.insert(c) {
                out.push(c);
            }
        }
    }
    out
}

#[cfg(test)]
mod hybrid_tests {
    use super::*;
    #[test]
    fn prefetch_sweeps_twice_lead_and_handles_negative_velocity() {
        let forward = prefetch_cells([32., 32.], [48., 0.], 32., 64.);
        assert!(forward.contains(&(2, 0)));
        assert!(forward.contains(&(0, 0)));
        assert!(!forward.contains(&(-2, 0)));
        let back = prefetch_cells([32., 32.], [-48., 0.], 32., 64.);
        assert!(back.contains(&(-2, 0)));
        assert_eq!(
            forward
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            forward.len()
        );
    }
    #[test]
    fn deflate_is_lossless_and_reduces_repeated_geometry() {
        let t = Tile { origins: Vec::new(), origin_names: Vec::new(), noses: Vec::new(),
            cell: (0, 0),
            size: 64.,
            source: format!("{}\n{}", rules_hash(), live_policy(&BTreeSet::new())),
            triangles: vec![([[0.12345, 0., 2.], [1., 0., 2.], [0., 1., 2.]], 123); 30000],
            rails: vec![],
        };
        let mut compressed = Vec::new();
        t.write(&mut compressed).unwrap();
        assert!(compressed.len() < 300000);
        assert_eq!(Tile::read(compressed.as_slice()).unwrap(), t);
        let mut extra = compressed.clone();
        extra.push(1);
        assert!(Tile::read(extra.as_slice()).is_err());
    }
    #[test]
    fn lru_retains_recent_tiles_and_rejects_stale_rules() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../out/tests/lru-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tile-size.txt"), "64").unwrap();
        std::fs::write(dir.join("rules.txt"), rules_hash()).unwrap();
        write_live_cells(&dir, 64., &rules_hash(), &BTreeSet::new()).unwrap();
        for c in [(0, 0), (1, 0)] {
            Tile { origins: Vec::new(), origin_names: Vec::new(), noses: Vec::new(),
                cell: c,
                size: 64.,
                source: format!("{}\n{}", rules_hash(), live_policy(&BTreeSet::new())),
                triangles: vec![([[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], 0)],
                rails: vec![],
            }
            .write(std::fs::File::create(path(&dir, c)).unwrap())
            .unwrap();
        }
        let mut cache = Cache::default();
        let first = cache.select(&dir, [32., 32.], 1.).unwrap();
        cache.budget = cache.bytes;
        std::fs::remove_file(path(&dir, (0, 0))).unwrap();
        let hit = cache.select(&dir, [32., 32.], 1.).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &first.baked[&(0, 0)],
            &hit.baked[&(0, 0)]
        ));
        cache.select(&dir, [96., 32.], 1.).unwrap();
        assert!(cache.bytes <= cache.budget);
        assert!(!cache.entries.contains_key(&(0, 0)));
        assert_eq!(
            cache.select(&dir, [32., 32.], 1.).unwrap().missing,
            [(0, 0)]
        );
        std::fs::write(dir.join("rules.txt"), "obsolete").unwrap();
        assert!(cache.select(&dir, [96., 32.], 1.).is_none());
        for p in [
            path(&dir, (1, 0)),
            dir.join("tile-size.txt"),
            dir.join("rules.txt"),
            dir.join("live-cells.txt"),
        ] {
            std::fs::remove_file(p).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn unknown_rules_and_mixed_asset_sources_fall_back_to_live() {
        assert!(!rules_compatible("rules 7b8e427532206ab0"));
        assert!(!rules_compatible("rules b2087032771621ab"));
        assert!(!rules_compatible("rules unverified"));
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../out/tests/tile-sources-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tile-size.txt"), "64").unwrap();
        std::fs::write(dir.join("rules.txt"), rules_hash()).unwrap();
        write_live_cells(&dir, 64., "source map-a", &BTreeSet::new()).unwrap();
        for (cell, source) in [((0, 0), "map-a"), ((1, 0), "map-b")] {
            Tile { origins: Vec::new(), origin_names: Vec::new(), noses: Vec::new(), cell, size: 64., source: format!("source {source}\n{}\n{}", rules_hash(), live_policy(&BTreeSet::new())),
                triangles: vec![], rails: vec![] }
                .write(std::fs::File::create(path(&dir, cell)).unwrap()).unwrap();
        }
        let s = Cache::default().select(&dir, [64., 32.], 31.).unwrap();
        assert_eq!(s.baked.keys().copied().collect::<Vec<_>>(), [(0, 0)]);
        assert_eq!(s.missing, [(1, 0)]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stateful_cells_override_even_cached_tiles_and_policy_is_mandatory() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../out/tests/tile-stateful-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tile-size.txt"), "64").unwrap();
        std::fs::write(dir.join("rules.txt"), rules_hash()).unwrap();
        let source = format!("source map-a\n{}\n{}", rules_hash(), live_policy(&BTreeSet::new()));
        Tile { origins: Vec::new(), origin_names: Vec::new(), noses: Vec::new(), cell: (0, 0), size: 64., source: source.clone(), triangles: vec![], rails: vec![] }
            .write(std::fs::File::create(path(&dir, (0, 0))).unwrap()).unwrap();
        let mut cache = Cache::default();
        assert!(cache.select(&dir, [32., 32.], 1.).is_none(), "no state policy cannot mean all-static");
        write_live_cells(&dir, 64., &source, &BTreeSet::new()).unwrap();
        assert_eq!(cache.select(&dir, [32., 32.], 1.).unwrap().baked.len(), 1);
        write_live_cells(&dir, 64., &source, &BTreeSet::from([(0, 0)])).unwrap();
        let selected = cache.select(&dir, [32., 32.], 1.).unwrap();
        assert!(selected.baked.is_empty());
        assert_eq!(selected.missing, [(0, 0)]);
        assert_eq!(selected.live_only, [(0, 0)]);
        write_live_cells(&dir, 64., "source wrong-map", &BTreeSet::new()).unwrap();
        assert!(cache.select(&dir, [32., 32.], 1.).unwrap().baked.is_empty());
        write_live_cells(&dir, 32., &source, &BTreeSet::new()).unwrap();
        assert!(cache.select(&dir, [32., 32.], 1.).is_none(), "wrong tile size rejected");
        write_live_cells(&dir, 64., &source, &BTreeSet::new()).unwrap();
        let policy = dir.join("live-cells.txt");
        let valid = std::fs::read_to_string(&policy).unwrap();
        for suffix in ["broken\n", "0 0\n0 0\n", "0 0 extra\n"] {
            std::fs::write(&policy, format!("{valid}{suffix}")).unwrap();
            assert!(cache.select(&dir, [32., 32.], 1.).is_none(), "malformed policy rejected");
        }
        write_live_cells(&dir, 64., &source, &BTreeSet::from([(0, 0), (1, 0)])).unwrap();
        let valid = std::fs::read_to_string(&policy).unwrap();
        assert!(read_live_cells(&dir, 64.).is_some());
        for damaged in [
            valid.strip_suffix("1 0\n").unwrap().to_owned(),
            valid.replace("0 0\n1 0\n", ""),
            valid.replace("1 0\n", "2 0\n"),
        ] {
            std::fs::write(&policy, damaged).unwrap();
            assert!(read_live_cells(&dir, 64.).is_none(), "complete-line truncation/coordinate edits rejected");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn old_city_tiles_cannot_bypass_corrected_generator_rules() {
        let suffix = "SVWT v2; tile 64; support 128; static defaults=on\n";
        let a = format!("source 0ff73dd18eb09026\nrules 7b8e427532206ab0\n{suffix}");
        let b = format!("source fb62fd587ffc13af\nrules b2087032771621ab\n{suffix}");
        assert_ne!(source_generation(&a), source_generation(&b));
        assert_ne!(source_generation(&a), source_generation(&b.replace("fb62fd587ffc13af", "0000000000000001")));
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../out/tests/tile-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tile-size.txt"), "64").unwrap();
        std::fs::write(dir.join("rules.txt"), "rules b2087032771621ab").unwrap();
        for (cell, source) in [((0, 0), a), ((1, 0), b)] {
            Tile { origins: Vec::new(), origin_names: Vec::new(), noses: Vec::new(), cell, size: 64., source, triangles: vec![], rails: vec![] }
                .write(std::fs::File::create(path(&dir, cell)).unwrap()).unwrap();
        }
        assert!(Cache::default().select(&dir, [64., 32.], 31.).is_none(),
            "old output requires a fresh bake after decoder/placement changes");
        std::fs::remove_dir_all(dir).unwrap();
        let seed = 0xcbf29ce484222325;
        assert_eq!(hash_rule_bytes(seed, b"one\ntwo\n"), hash_rule_bytes(seed, b"one\r\ntwo\r\n"));
        assert_ne!(hash_rule_bytes(seed, b"one\rtwo"), hash_rule_bytes(seed, b"onetwo"));
    }
}
