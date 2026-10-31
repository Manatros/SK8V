//! The prebaked map (`<cache>-map.svwc` beside the world cache, baked by
//! `skatev-world-cache --bake-story-map` from invariant Story `.ybn` collision): the fallback where
//! GTA's live collision has holes. Its triangles only go in as gap-fill
//! candidates (`svwc::clean::fill_floor_gaps`): walkable floors where the live
//! collision has no floor within a metre, never doubling a live surface.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

static MAP: Mutex<Option<svwc::Cache<std::io::BufReader<std::fs::File>>>> = Mutex::new(None);

/// The prebaked map's path for the world cache at `cache`.
pub fn path(cache: &Path) -> PathBuf {
    let stem = cache.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    cache.with_file_name(format!("{stem}-map.svwc"))
}

fn policy_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".policy");
    name.into()
}

pub fn policy_source() -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for bytes in [
        include_bytes!("baked.rs").as_slice(),
        include_bytes!("../../world-cache/src/tile_bake.rs").as_slice(),
        include_bytes!("../../world-cache/src/main.rs").as_slice(),
        include_bytes!("../../world-cache/src/interiors.rs").as_slice(),
        include_bytes!("../../world-cache/src/interiors/doors.rs").as_slice(),
        include_bytes!("../../world-cache/src/vfs.rs").as_slice(),
        include_bytes!("../../world-cache/src/packs.rs").as_slice(),
        include_bytes!("../../svwc/src/bounds.rs").as_slice(),
        include_bytes!("../../svwc/src/clean.rs").as_slice(),
    ] {
        for (i,&byte) in bytes.iter().enumerate() {
            if byte == b'\r' && bytes.get(i+1) == Some(&b'\n') { continue; }
            hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
        }
    }
    hash
}

fn data_hash(path: &Path) -> std::io::Result<u64> {
    use std::io::Read;
    let mut reader = std::fs::File::open(path)?;
    let mut buffer = [0u8;65536];
    let mut hash = 0xcbf29ce484222325u64;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 { return Ok(hash); }
        for &byte in &buffer[..count] { hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3); }
    }
}

fn policy_text(path: &Path) -> std::io::Result<String> {
    Ok(format!("SVWC-POLICY 1\nStoryArchives=1\nStateGroups=live\nWalkingFloors=1\nsource {:016x}\ndata {:016x}\n",policy_source(),data_hash(path)?))
}

pub fn write_policy(path: &Path) -> std::io::Result<()> {
    std::fs::write(policy_path(path),policy_text(path)?)
}

pub fn verified_policy(path: &Path) -> bool {
    policy_text(path).ok().zip(std::fs::read_to_string(policy_path(path)).ok())
        .is_some_and(|(expected,actual)| expected == actual.replace("\r\n","\n"))
}

/// Opens the prebaked map beside `cache`; false (and no fallback) when it is
/// missing or unreadable.
pub fn open(cache: &Path) -> Result<usize, String> {
    *MAP.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let p = path(cache);
    if !verified_policy(&p) { return Err("prebaked floor map has no matching Story/state provenance; rebuild with --bake-story-map".to_owned()); }
    let map = svwc::Cache::open(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let tiles = map.tile_count();
    *MAP.lock().unwrap_or_else(|e| e.into_inner()) = Some(map);
    Ok(tiles)
}

/// The prebaked triangles over the square of half-size `half` around
/// `centre` (GTA space, metres); empty without a prebaked map.
pub fn around(centre: [f32; 2], half: f32) -> Vec<svwc::Tri> {
    let mut guard = MAP.lock().unwrap_or_else(|e| e.into_inner());
    let Some(map) = guard.as_mut() else { return Vec::new() };
    map.query(centre, half).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_policy_rejects_old_or_changed_data_and_loads_verified_map() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../out/tests/story-floors-{}",std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cache = dir.join("fixture.svwc");
        let map = path(&cache);
        let floor = svwc::Tri { v:[[19000.,19000.,1.],[19001.,19000.,1.],[19000.,19001.,1.]],material:4 };
        let mut bytes = Vec::new();
        svwc::write(&mut bytes,&[floor],64.).unwrap();
        std::fs::write(&map,&bytes).unwrap();
        assert!(!verified_policy(&map),"old all-DLC maps must not be accepted without provenance");
        assert!(open(&cache).is_err());
        write_policy(&map).unwrap();
        assert!(verified_policy(&map));
        assert!(open(&cache).unwrap() > 0);
        assert_eq!(around([19000.,19000.],1.),[floor]);
        std::fs::write(policy_path(&map),"SVWC-POLICY 1\nsource obsolete\n").unwrap();
        assert!(!verified_policy(&map));
        assert!(open(&cache).is_err());
        assert!(around([19000.,19000.],1.).is_empty(),"failed validation clears previously loaded fallback");
        write_policy(&map).unwrap();
        std::fs::write(&map,[bytes.as_slice(),b"changed"].concat()).unwrap();
        assert!(!verified_policy(&map),"receipt binds exact data bytes");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// SKATEV_MAP=<map.svwc> cargo test -p skatev-runtime baked -- --ignored --nocapture
    #[test]
    #[ignore]
    fn floors_at_the_rc6_hole() {
        let mut map = svwc::Cache::open(std::path::Path::new(&std::env::var("SKATEV_MAP").unwrap())).unwrap();
        for (x, y) in [(-147.1f32, 449.07f32), (-140.5, 448.8), (-152.6, 450.9), (-136.0, 453.0), (-181.4, 439.4)] {
            let floors: Vec<f32> = map.query([x, y], 0.5).unwrap().iter()
                .filter_map(|t| svwc::clean::floor_at(t, [x, y])).collect();
            println!("({x}, {y}): floors at z {floors:?}");
        }
        for half in [60.0f32, 120.0] {
            let t = std::time::Instant::now();
            let n = map.query([-147.0, 449.0], half).unwrap().len();
            println!("area half {half} m: {n} prebaked triangles in {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
        }
    }

    #[test]
    fn the_map_sits_beside_the_world_cache() {
        let p = super::path(std::path::Path::new("fixture/world/sk8v.svwc"));
        assert_eq!(p, std::path::Path::new("fixture/world/sk8v-map.svwc"));
    }
}

#[cfg(test)]
mod track_probe {
    use bevy_math::Vec3;

    /// Collision floor heights across a train track at sample nodes (prebaked
    /// map): how GTA's collision sits relative to the track node height.
    #[test]
    #[ignore]
    fn collision_across_track_nodes() {
        super::open(std::path::Path::new(&std::env::var_os("SKATEV_TEST_CACHE").expect("set SKATEV_TEST_CACHE to the local world cache"))).expect("map");
        let pairs = [([-1144.04f32, -2893.72, -9.32361], [-1139.73f32, -2888.36, -9.32364]),
                     ([-1197.93, -318.686, 13.3353], [-1211.36, -326.478, 13.1316]),
                     ([2543.07, -160.206, 92.3057], [2540.5, -165.645, 92.2969]),
                     ([1254.62, -782.056, 44.045], [1256.91, -791.816, 44.5376])];
        for (a, b) in pairs {
            let (a, b) = (Vec3::from_array(a), Vec3::from_array(b));
            let mid = (a + b) * 0.5;
            let along = (b - a).normalize();
            let side = Vec3::new(-along.y, along.x, 0.0).normalize();
            let tris = super::around([mid.x, mid.y], 4.0);
            let mut line = String::new();
            for off in [-1.2f32, -0.9, -0.7175, -0.6, -0.3, 0.0, 0.3, 0.6, 0.7175, 0.9, 1.2] {
                let p = mid + side * off;
                let mut zs: Vec<f32> = tris.iter().filter_map(|t| svwc::clean::floor_at(t, [p.x, p.y])).filter(|z| (z - mid.z).abs() < 2.0).collect();
                zs.sort_by(f32::total_cmp);
                line += &format!(" {off:+.2}:{}", zs.iter().map(|z| format!("{:+.2}", z - mid.z)).collect::<Vec<_>>().join("/"));
            }
            println!("node z {:.2}:{line}", mid.z);
        }
    }
}

#[cfg(test)]
mod track_probe_many {
    /// Histogram: highest collision floor within 2 m above/below the node at
    /// the centre line and at the rail lines (+-0.7175 m), over trains1.dat.
    #[test]
    #[ignore]
    fn collision_heights_along_the_freight_line() {
        super::open(std::path::Path::new(&std::env::var_os("SKATEV_TEST_CACHE").expect("set SKATEV_TEST_CACHE to the local world cache"))).expect("map");
        let text = std::fs::read_to_string(std::env::var("TRACK").unwrap()).unwrap();
        let nodes: Vec<[f32; 3]> = text.lines().skip(1).filter_map(|l| {
            let v: Vec<f32> = l.split_whitespace().take(3).filter_map(|s| s.parse().ok()).collect();
            (v.len() == 3).then(|| [v[0], v[1], v[2]])
        }).collect();
        let mut centre = std::collections::BTreeMap::<i32, usize>::new();
        let mut rail = std::collections::BTreeMap::<i32, usize>::new();
        for w in nodes.windows(2).step_by(7) {
            let (a, b) = (w[0], w[1]);
            let mid = [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2., (a[2] + b[2]) / 2.];
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = (dx * dx + dy * dy).sqrt().max(1e-6);
            let side = [-dy / l, dx / l];
            let tris = super::around([mid[0], mid[1]], 3.0);
            let top = |off: f32| tris.iter().filter_map(|t| svwc::clean::floor_at(t, [mid[0] + side[0] * off, mid[1] + side[1] * off]))
                .filter(|z| (z - mid[2]).abs() < 1.0).map(|z| z - mid[2]).fold(f32::NAN, f32::max);
            let c = top(0.0);
            if c.is_finite() { *centre.entry((c * 20.0).round() as i32).or_default() += 1; }
            let side_off: f32 = std::env::var("OFF").ok().and_then(|s| s.parse().ok()).unwrap_or(0.7175);
            for off in [-side_off, side_off] {
                let r = top(off);
                if r.is_finite() { *rail.entry((r * 100.0).round() as i32).or_default() += 1; }
            }
        }
        println!("centre (5 cm bins): {:?}", centre.iter().map(|(k, n)| format!("{:+.2}:{n}", *k as f32 / 20.0)).collect::<Vec<_>>());
        println!("rails  (1 cm bins): {:?}", rail.iter().filter(|(_, n)| **n > 5).map(|(k, n)| format!("{:+.2}:{n}", *k as f32 / 100.0)).collect::<Vec<_>>());
    }
}
