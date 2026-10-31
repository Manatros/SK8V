//! Rails from GTA's visible models, for the whole game: every exterior
//! placement and every interior's (MLO) placed models, base game, update
//! and DLC packs. Written as one file of polylines in world space
//! (`<cache>.visrails`, read by the runtime's visrails.rs).
//!
//!     visual-rails OUT.visrails --gta <GTA dir> --keys <gta-keys dir>
//!     visual-rails --ydr FILE.ydr [...]      (one model's rails, model space)
#[path = "../bars.rs"]
mod bars;
#[path = "../vfs.rs"]
mod vfs;
#[path = "../packs.rs"]
mod packs;

use rage_formats::ydd::{Drawable, LodLevel};
use rage_formats::{parse_ydd, parse_ydr, parse_ymap, parse_ytyp, rage_joaat, Archetype, MloDef, Vec3, YmapEntity};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

type V = [f32; 3];

/// Tile size of the output (metres).
const CELL: f32 = 32.0;

/// The rails of a drawable's high-detail meshes, model space, each with its
/// geometry's shader name hash.
fn drawable_rails(d: &Drawable) -> Vec<(Vec<V>, u32)> {
    // A plant (any tree or grass shader): its branches and stems are thin
    // closed tubes, but no rail.
    if d.shader_group.as_ref().is_some_and(|g| g.shaders.iter().any(|s| vegetation().contains(&s.name_hash))) {
        return Vec::new();
    }
    let Some(lod) = d.lods.iter().find(|l| l.level == LodLevel::High).or_else(|| d.lods.first()) else { return Vec::new() };
    let mut out = Vec::new();
    for m in &lod.models {
        for g in &m.geometries {
            let (Some(vb), Some(ib)) = (&g.vertex_buffer, &g.index_buffer) else { continue };
            let Ok(verts) = vb.to_unified_vertices() else { continue };
            let pos: Vec<V> = verts.iter().map(|v| [v.position.x, v.position.y, v.position.z]).collect();
            let shader = d.shader_group.as_ref().and_then(|s| s.shaders.get(g.shader_id as usize)).map_or(0, |s| s.name_hash);
            out.extend(bars::rails(&pos, &ib.indices).into_iter().map(|r| (r, shader)));
        }
    }
    out
}

/// GTA's vegetation shaders (name hashes).
fn vegetation() -> &'static std::collections::HashSet<u32> {
    static SET: std::sync::OnceLock<std::collections::HashSet<u32>> = std::sync::OnceLock::new();
    SET.get_or_init(|| {
        ["trees", "trees_lod", "trees_lod2", "trees_lod_tnt", "trees_normal", "trees_normal_spec", "trees_normal_diffspec",
         "trees_normal_spec_tnt", "trees_normal_diffspec_tnt", "trees_tnt", "trees_camera_aligned", "trees_camera_facing",
         "trees_shadow_proxy", "grass", "grass_batch", "grass_fur", "grass_fur_mask", "grass_fur_lod"]
            .iter()
            .map(|n| rage_joaat(n))
            .collect()
    })
}

/// Where a model is placed: an exterior entity, or an entity of an interior
/// (MLO) placed by the interior's instance.
#[derive(Clone, Copy)]
enum Place {
    Outside(YmapEntity),
    Inside(YmapEntity, YmapEntity),
}

/// Which way each kind of stored rotation is applied: `true` = as stored,
/// `false` = its inverse (conjugate).
#[derive(Clone, Copy, Debug)]
struct Convention {
    outside: bool,
    instance: bool,
    child: bool,
}

/// GTA's convention, measured against GTA's collision (`--check`).
const GTA: Convention = Convention { outside: false, instance: true, child: false };

fn turn(e: &YmapEntity, p: V, as_stored: bool) -> V {
    let [x, y, z, w] = e.rotation;
    let q = if as_stored { [x, y, z, w] } else { [-x, -y, -z, w] };
    let s = Vec3::new(p[0] * e.scale_xy, p[1] * e.scale_xy, p[2] * e.scale_z);
    let v = rage_formats::ymap::rotate(s, q) + e.position;
    [v.x, v.y, v.z]
}

impl Place {
    fn apply(&self, p: V, c: Convention) -> V {
        match self {
            Place::Outside(e) => turn(e, p, c.outside),
            Place::Inside(outer, e) => turn(outer, turn(e, p, c.child), c.instance),
        }
    }
}

/// The drawable an archetype shows (its own .ydr or its dictionary's member).
fn drawable_of(a: &Archetype, files: &BTreeMap<String, gta_archives::Entry>, keys: &gta_archives::GtaKeys) -> Option<Drawable> {
    let by = |ext: &str, h: u32| files.iter().find(|(n, _)| n.ends_with(ext) && rage_joaat(n.trim_end_matches(ext)) == h).map(|(_, e)| e);
    if a.in_drawable_dictionary() {
        let e = by(".ydd", a.drawable_dictionary_hash)?;
        parse_ydd(&e.read(keys).ok()?).ok()?.into_iter().find(|d| d.hash == a.model_hash()).map(|d| d.drawable)
    } else {
        parse_ydr(&by(".ydr", a.model_hash())?.read(keys).ok()?).ok()
    }
}

/// Points on a drawable's high-detail surface (vertices), model space.
fn surface_points(d: &Drawable) -> Vec<V> {
    let Some(lod) = d.lods.iter().find(|l| l.level == LodLevel::High).or_else(|| d.lods.first()) else { return Vec::new() };
    let mut out = Vec::new();
    for m in &lod.models {
        for g in &m.geometries {
            let Some(vb) = &g.vertex_buffer else { continue };
            let Ok(verts) = vb.to_unified_vertices() else { continue };
            out.extend(verts.iter().map(|v| [v.position.x, v.position.y, v.position.z]));
        }
    }
    let step = (out.len() / 400).max(1);
    out.into_iter().step_by(step).collect()
}

/// Collision triangles by 1 m cell, and the distance from a point to the
/// nearest one (capped at 2 m).
struct Surface {
    tris: Vec<[Vec3; 3]>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

impl Surface {
    fn new(tris: Vec<[Vec3; 3]>) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, t) in tris.iter().enumerate() {
            let lo = t[0].min(t[1]).min(t[2]);
            let hi = t[0].max(t[1]).max(t[2]);
            if hi.x - lo.x > 200.0 || hi.y - lo.y > 200.0 { continue; }
            for x in lo.x.floor() as i32 - 2..=hi.x.floor() as i32 + 2 {
                for y in lo.y.floor() as i32 - 2..=hi.y.floor() as i32 + 2 {
                    cells.entry((x, y)).or_default().push(i as u32);
                }
            }
        }
        Surface { tris, cells }
    }
    fn distance(&self, p: V) -> f32 {
        let q = Vec3::new(p[0], p[1], p[2]);
        self.cells.get(&(q.x.floor() as i32, q.y.floor() as i32)).map_or(2.0, |c| {
            c.iter().map(|&i| point_triangle(q, &self.tris[i as usize])).fold(2.0, f32::min)
        })
    }
}

fn point_triangle(p: Vec3, t: &[Vec3; 3]) -> f32 {
    // Distance to the plane inside the triangle, else to its edges.
    let (a, b, c) = (t[0], t[1], t[2]);
    let n = (b - a).cross(c - a);
    let l = n.length();
    let edge = |a: Vec3, b: Vec3| {
        let d = b - a;
        let s = if d.dot(d) > 1e-12 { ((p - a).dot(d) / d.dot(d)).clamp(0.0, 1.0) } else { 0.0 };
        (p - (a + d * s)).length()
    };
    let best = edge(a, b).min(edge(b, c)).min(edge(c, a));
    if l < 1e-9 { return best; }
    let n = n * (1.0 / l);
    let h = (p - a).dot(n);
    let q = p - n * h;
    let inside = (b - a).cross(q - a).dot(n) >= 0.0 && (c - b).cross(q - b).dot(n) >= 0.0 && (a - c).cross(q - c).dot(n) >= 0.0;
    if inside { h.abs().min(best) } else { best }
}

/// Places every model near `at` (x, y, radius) both ways and prints how far
/// its surface lies from GTA's collision on average: the right convention
/// is the one that matches. Exteriors against the world collision; interior
/// children in their interior's own space against its collision.
fn check_conventions(at: [f32; 3], placed: &[(u32, Place)], archetypes: &HashMap<u32, Archetype>,
                     files: &BTreeMap<String, gta_archives::Entry>, keys: &gta_archives::GtaKeys) {
    let prefix = std::env::var("CHECK_YBN").unwrap_or_default();
    let mut world = Vec::new();
    for (n, e) in files.iter().filter(|(n, _)| n.ends_with(".ybn") && n.starts_with(&prefix)) {
        let Some(y) = e.read(keys).ok().and_then(|b| rage_formats::parse_ybn(&b).ok()) else { continue };
        let tris = y.triangles();
        let near = tris.iter().any(|t| t.vertices.iter().any(|v| (v.x - at[0]).abs() < at[2] && (v.y - at[1]).abs() < at[2]));
        if near {
            eprintln!("collision {n}: {} triangles", tris.len());
            world.extend(tris.iter().map(|t| t.vertices));
        }
    }
    let (lo, hi) = world.iter().flatten().fold((Vec3::new(1e9, 1e9, 1e9), Vec3::new(-1e9, -1e9, -1e9)), |(l, h), v| (l.min(*v), h.max(*v)));
    eprintln!("world collision box {lo:?}..{hi:?}");
    let world = Surface::new(world);
    let mut sums: HashMap<String, (f64, usize, usize)> = HashMap::new();
    let mut interior_cache: HashMap<u32, Option<Surface>> = HashMap::new();
    for (hash, place) in placed {
        let pos = match place { Place::Outside(e) => e.position, Place::Inside(o, _) => o.position };
        if (pos.x - at[0]).abs() > at[2] || (pos.y - at[1]).abs() > at[2] { continue; }
        let Some(a) = archetypes.get(hash) else { continue };
        let Some(d) = drawable_of(a, files, keys) else { continue };
        let pts = surface_points(&d);
        if pts.is_empty() { continue; }
        let mean_of = |surf: &Surface, f: &dyn Fn(V) -> V| pts.iter().map(|&p| surf.distance(f(p)) as f64).sum::<f64>() / pts.len() as f64;
        // Per model: which way fits clearly better (5 cm), among clearly turned ones.
        let mut score = |kind: &str, e: &YmapEntity, surf: &Surface, f: &dyn Fn(V, bool) -> V| {
            if e.rotation[3].abs() > 0.985 { return; } // under ~20 deg
            let (s, i) = (mean_of(surf, &|p| f(p, true)), mean_of(surf, &|p| f(p, false)));
            if s.min(i) > 0.5 { return; } // no collision of its own here
            let key = if s + 0.05 < i { "as stored" } else if i + 0.05 < s { "inverted" } else { "tie" };
            let e2 = sums.entry(format!("{kind}: {key} fits better")).or_default();
            e2.0 += s.min(i);
            e2.1 += 1;
            if std::env::var("CHECK_SHOW").is_ok() { eprintln!("{kind} {:08x} at {:?}: as stored {s:.3}, inverted {i:.3}", e.archetype_hash, e.position); }
        };
        if std::env::var("CHECK_SHOW").is_ok() {
            if let Place::Outside(e) = place {
                let lo = pts.iter().fold([f32::MAX; 3], |m, p| [m[0].min(p[0]), m[1].min(p[1]), m[2].min(p[2])]);
                let hi = pts.iter().fold([f32::MIN; 3], |m, p| [m[0].max(p[0]), m[1].max(p[1]), m[2].max(p[2])]);
                let w = turn(e, pts[0], false);
                eprintln!("model {hash:08x} at ({:.1} {:.1} {:.1}) rot {:?} scale {} {}; local box {lo:?}..{hi:?}; first point placed {w:?}, distance {:.2}",
                    e.position.x, e.position.y, e.position.z, e.rotation, e.scale_xy, e.scale_z, world.distance(w));
            }
        }
        match place {
            Place::Outside(e) => {
                score("outside", e, &world, &|p, stored| turn(e, p, stored));
            }
            Place::Inside(outer, e) => {
                // The interior's collision: the bound named like its archetype.
                let surf = interior_cache.entry(outer.archetype_hash).or_insert_with(|| {
                    let mlo = archetypes.get(&outer.archetype_hash)?;
                    let name = files.keys().find(|n| n.ends_with(".ybn") && rage_joaat(n.trim_end_matches(".ybn")) == mlo.model_hash())?.clone();
                    let y = files.get(&name)?.read(keys).ok().and_then(|b| rage_formats::parse_ybn(&b).ok())?;
                    eprintln!("interior collision {name}");
                    Some(Surface::new(y.triangles().iter().map(|t| t.vertices).collect()))
                });
                let Some(surf) = surf.as_ref() else { continue };
                score("interior child", e, surf, &|p, stored| turn(e, p, stored));
            }
        }
    }
    let mut rows: Vec<_> = sums.into_iter().collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, (sum, n, close)) in rows {
        let _ = close;
        println!("{name}: {n} models (mean best distance {:.3} m)", sum / n.max(1) as f64);
    }
}

fn stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    name.rsplit_once('.').map_or(name.clone(), |(s, _)| s.to_string())
}

/// The skateparks' visible meshes (design 2026-10-25-park-surfaces-from-visible
/// 1.1-1.3): placed, non-fragment drawables within a park's radius + 60 m,
/// High LOD, without vegetation drawables and without decal, glass, alpha,
/// cutout and emissive shaders; triangles whose centroid lies in the circle.
fn export_parks(out: &Path, placed: &[(u32, Place)], archetypes: &HashMap<u32, Archetype>,
    files: &BTreeMap<String, gta_archives::Entry>, keys: &gta_archives::GtaKeys) -> anyhow::Result<()> {
    let excluded: HashSet<u32> = files.keys().filter(|n| n.ends_with(".sps")).map(|n| n.trim_end_matches(".sps").to_lowercase())
        .filter(|n| ["decal", "glass", "alpha", "cutout", "emissive"].iter().any(|w| n.contains(w)))
        .map(|n| rage_joaat(&n)).collect();
    let mut parks = Vec::new();
    for park in SkateVRuntime::parks::PARKS {
        let (cx, cy, r) = (park.centre.x, park.centre.y, park.radius);
        let mut tris = Vec::new();
        let mut models = 0usize;
        for (hash, place) in placed {
            let at = match place { Place::Outside(e) => e.position, Place::Inside(o, _) => o.position };
            if ((at.x - cx).powi(2) + (at.y - cy).powi(2)).sqrt() > r + 60.0 { continue; }
            let Some(a) = archetypes.get(hash) else { continue };
            if a.is_fragment() { continue; }
            let Some(d) = drawable_of(a, files, keys) else { continue };
            if d.shader_group.as_ref().is_some_and(|g| g.shaders.iter().any(|s| vegetation().contains(&s.name_hash))) { continue; }
            let Some(lod) = d.lods.iter().find(|l| l.level == LodLevel::High) else { continue };
            let before = tris.len();
            for g in lod.models.iter().flat_map(|m| &m.geometries) {
                if let Some(sh) = d.shader_group.as_ref().and_then(|s| s.shaders.get(g.shader_id as usize)) {
                    if excluded.contains(&sh.name_hash) || excluded.contains(&sh.file_name_hash) { continue; }
                }
                let (Some(vb), Some(ib)) = (&g.vertex_buffer, &g.index_buffer) else { continue };
                let Ok(verts) = vb.to_unified_vertices() else { continue };
                let pos: Vec<V> = verts.iter().map(|v| place.apply([v.position.x, v.position.y, v.position.z], GTA)).collect();
                for t in ib.indices.chunks_exact(3) {
                    let (Some(&a), Some(&b), Some(&c)) = (pos.get(t[0] as usize), pos.get(t[1] as usize), pos.get(t[2] as usize)) else { continue };
                    let m = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0];
                    if ((m[0] - cx).powi(2) + (m[1] - cy).powi(2)).sqrt() > r { continue; }
                    if !a.iter().chain(&b).chain(&c).all(|v| v.is_finite()) { continue; }
                    tris.push([a, b, c]);
                }
            }
            models += usize::from(tris.len() > before);
        }
        eprintln!("park {}: {} visible triangles from {models} models", park.id, tris.len());
        parks.push((park.id.to_string(), tris));
    }
    SkateVRuntime::visparks::write(out, &parks)?;
    eprintln!("wrote park meshes to {}", out.display());
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--ydr") {
        for f in &args[1..] {
            let d = parse_ydr(&std::fs::read(f)?)?;
            for (r, _) in drawable_rails(&d) {
                let len: f32 = r.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2) + (w[1][2] - w[0][2]).powi(2)).sqrt()).sum();
                println!("{f}: rail {len:.2} m, {} points, from ({:.2}, {:.2}, {:.2}) to ({:.2}, {:.2}, {:.2})", r.len(),
                    r[0][0], r[0][1], r[0][2], r[r.len() - 1][0], r[r.len() - 1][1], r[r.len() - 1][2]);
            }
        }
        return Ok(());
    }
    let out = PathBuf::from(args.first().ok_or_else(|| anyhow::anyhow!("usage: visual-rails OUT --gta DIR --keys DIR"))?);
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let gta = arg("--gta").ok_or_else(|| anyhow::anyhow!("--gta DIR"))?;
    let keys = gta_archives::keys(&arg("--keys").ok_or_else(|| anyhow::anyhow!("--keys DIR"))?)?;
    // GTA's mount order: base archives, then the update, then DLC packs; a later file of the same name wins.
    let archives = vfs::story_archives(&gta, &keys)?;
    // --check X Y R: measure the rotation conventions against GTA's collision near (X, Y).
    let check: Option<[f32; 3]> = args.iter().position(|a| a == "--check").and_then(|i| {
        let f = |k: usize| args.get(i + k).and_then(|s| s.parse::<f32>().ok());
        Some([f(1)?, f(2)?, f(3)?])
    });
    let checking = check.is_some();
    let want = |p: &str| {
        let p = p.to_lowercase();
        let ybn = checking && p.ends_with(".ybn") && !p.contains("/hi@") && !p.contains("/ma@");
        (ybn || p.ends_with(".ydr") || p.ends_with(".ydd") || p.ends_with(".ytyp") || p.ends_with(".ymap") || p.ends_with(".ymf") || p.ends_with(".sps")) && !p.contains("_lod") && !p.contains("slod")
    };
    let mut files: BTreeMap<String, gta_archives::Entry> = BTreeMap::new();
    let mut failed = Vec::new();
    let mut toggled = HashSet::new();
    for (name, archive) in &archives {
        let read_text = |name: &str| archive.files().iter().find(|f| f.path.eq_ignore_ascii_case(name))
            .and_then(|f| archive.extract(f, &keys).ok()).and_then(|b| String::from_utf8(b).ok());
        let metadata = read_text("content.xml").zip(read_text("setup2.xml"));
        anyhow::ensure!(!name.starts_with("patch") || metadata.is_some(),
            "Story patch {name} lacks readable content.xml/setup2.xml; refusing unfiltered rail export");
        let enabled = metadata.map(|(content, setup)| packs::parse(&content, &setup));
        for e in gta_archives::walk(archive.clone(), &keys, &want, &mut failed) {
            if !packs::allows(Path::new(""), &enabled, Path::new(&e.path)) { continue; }
            let ext = e.path.rsplit('.').next().unwrap_or("").to_lowercase();
            // Each archive has many `_manifest.ymf` files; do not collapse
            // those by stem before collecting the stateful map groups.
            if ext == "ymf" {
                let bytes = e.read(&keys).map_err(|err| anyhow::anyhow!("manifest {}: {err}", e.path))?;
                let (_, manifest) = rage_formats::parse_ymf(&bytes)
                    .map_err(|err| anyhow::anyhow!("manifest {}: {err}", e.path))?;
                for group in manifest.map_data_groups {
                    toggled.insert(group.name.hash);
                    toggled.extend(group.bounds.into_iter().map(|b| b.hash));
                }
                continue;
            }
            files.insert(format!("{}.{ext}", stem(&e.path)), e);
        }
        eprintln!("{name}: {} files so far", files.len());
    }
    anyhow::ensure!(failed.is_empty(), "{} archive entries failed while collecting rail sources: {:?}", failed.len(), failed);
    // Archetypes and interiors.
    let mut archetypes: HashMap<u32, Archetype> = HashMap::new();
    let mut mlos: HashMap<u32, MloDef> = HashMap::new();
    for (name, e) in files.iter().filter(|(n, _)| n.ends_with(".ytyp")) {
        let Ok(bytes) = e.read(&keys) else { continue };
        match parse_ytyp(&bytes) {
            Ok(y) => {
                for a in y.archetypes { archetypes.insert(a.name_hash, a); }
                for m in y.mlos { mlos.insert(m.name_hash, m); }
            }
            Err(err) => eprintln!("ytyp {name}: {err}"),
        }
    }
    eprintln!("{} archetypes, {} interiors", archetypes.len(), mlos.len());
    // Placements: exterior entities, and every interior's entities placed by its instance.
    let mut placed: Vec<(u32, Place)> = Vec::new();
    for e in files.iter().filter(|(n, _)| n.ends_with(".ymap") && !toggled.contains(&rage_joaat(&stem(n)))).map(|(_, e)| e) {
        let Ok(bytes) = e.read(&keys) else { continue };
        let Ok(map) = parse_ymap(&bytes) else { continue };
        if map.header.flags & rage_formats::YmapHeader::FLAG_LOD != 0 { continue; }
        for ent in map.entities.iter().filter(|e| !e.is_mlo_instance) {
            placed.push((ent.archetype_hash, Place::Outside(*ent)));
        }
        for inst in &map.mlo_instances {
            let Some(mlo) = mlos.get(&inst.entity.archetype_hash) else { continue };
            let mut inner: Vec<YmapEntity> = mlo.entities.clone();
            for set in &mlo.entity_sets {
                if inst.default_entity_sets.contains(&set.name_hash) { inner.extend(set.entities.iter().copied()); }
            }
            for ent in inner {
                placed.push((ent.archetype_hash, Place::Inside(inst.entity, ent)));
            }
        }
    }
    eprintln!("{} placements", placed.len());
    if let Some(at) = check {
        check_conventions(at, &placed, &archetypes, &files, &keys);
        return Ok(());
    }
    // --surface X0 X1 Y0 Y1: the visible high-detail surface of every model
    // placed in the box (world space, decals and vegetation left out) as
    // OUT (an OBJ). For comparing a park's visible shape with its collision.
    // --surface-list FILE: one "name x0 x1 y0 y1" per line, each written as
    // OUT/name.obj in one pass over the archives.
    let mut boxes: Vec<(PathBuf, [f32; 4])> = Vec::new();
    if let Some(i) = args.iter().position(|a| a == "--surface") {
        let f = |k: usize| args.get(i + k).and_then(|s| s.parse::<f32>().ok()).ok_or_else(|| anyhow::anyhow!("--surface X0 X1 Y0 Y1"));
        boxes.push((out.clone(), [f(1)?, f(2)?, f(3)?, f(4)?]));
    }
    if let Some(list) = arg("--surface-list") {
        std::fs::create_dir_all(&out)?;
        for line in std::fs::read_to_string(list)?.lines().filter(|l| !l.trim().is_empty()) {
            let w: Vec<&str> = line.split_whitespace().collect();
            anyhow::ensure!(w.len() == 5, "--surface-list line: name x0 x1 y0 y1");
            let v: Vec<f32> = w[1..].iter().map(|s| s.parse()).collect::<Result<_, _>>()?;
            boxes.push((out.join(format!("{}.obj", w[0])), [v[0], v[1], v[2], v[3]]));
        }
    }
    for (out, [x0, x1, y0, y1]) in boxes.iter().cloned() {
        let decals: HashSet<u32> = ["decal", "decal_dirt", "decal_tnt", "decal_spec_only", "decal_normal_only", "decal_emissive_only",
            "decal_shadow_only", "decal_amb_only", "decal_glue", "normal_decal", "normal_decal_tnt", "normal_decal_pxm",
            "normal_decal_pxm_tnt", "normal_spec_decal", "normal_spec_decal_detail", "normal_spec_decal_nopuddle",
            "normal_spec_decal_tnt", "normal_spec_decal_pxm", "spec_decal", "parallax_specmap", "mirror_decal",
            "emissive_alpha", "normal_spec_alpha", "normal_alpha", "spec_alpha", "alpha", "cutout_fence", "cutout_fence_normal",
            "glass", "glass_pv", "glass_env", "glass_spec", "glass_reflect", "glass_emissive", "glass_normal_spec_reflect"]
            .iter().map(|n| rage_joaat(n)).collect();
        let mut lines = String::new();
        let mut vertices = 0usize;
        let mut models = 0usize;
        for (hash, place) in &placed {
            let at = match place { Place::Outside(e) => e.position, Place::Inside(o, _) => o.position };
            // SURFACE_REACH (metres, default 60): how far a model's origin may lie
            // from the box (road and terrain models are large).
            let reach: f32 = std::env::var("SURFACE_REACH").ok().and_then(|v| v.parse().ok()).unwrap_or(60.0);
            if at.x < x0 - reach || at.x > x1 + reach || at.y < y0 - reach || at.y > y1 + reach { continue; }
            let Some(a) = archetypes.get(hash) else { continue };
            if a.is_fragment() { continue; }
            let Some(d) = drawable_of(a, &files, &keys) else { continue };
            if d.shader_group.as_ref().is_some_and(|g| g.shaders.iter().any(|s| vegetation().contains(&s.name_hash))) { continue; }
            let Some(lod) = d.lods.iter().find(|l| l.level == LodLevel::High).or_else(|| d.lods.first()) else { continue };
            let mut any = false;
            let name = files.keys().find(|n| rage_joaat(n.rsplit_once('.').map_or(n.as_str(), |(s, _)| s)) == a.model_hash()).cloned().unwrap_or_else(|| format!("{:08x}", a.model_hash()));
            let inside = matches!(place, Place::Inside(..));
            lines.push_str(&format!("o {name}{}
", if inside { " (interior)" } else { "" }));
            for m in &lod.models {
                for g in &m.geometries {
                    let shader = d.shader_group.as_ref().and_then(|s| s.shaders.get(g.shader_id as usize)).map_or(0, |s| s.name_hash);
                    if decals.contains(&shader) { continue; }
                    let (Some(vb), Some(ib)) = (&g.vertex_buffer, &g.index_buffer) else { continue };
                    let Ok(verts) = vb.to_unified_vertices() else { continue };
                    let pos: Vec<V> = verts.iter().map(|v| place.apply([v.position.x, v.position.y, v.position.z], GTA)).collect();
                    for t in ib.indices.chunks_exact(3) {
                        let tri = [pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]];
                        let c = [(tri[0][0] + tri[1][0] + tri[2][0]) / 3.0, (tri[0][1] + tri[1][1] + tri[2][1]) / 3.0];
                        if c[0] < x0 || c[0] > x1 || c[1] < y0 || c[1] > y1 { continue; }
                        for p in tri { lines.push_str(&format!("v {} {} {}
", p[0], p[1], p[2])); }
                        lines.push_str(&format!("f {} {} {}
", vertices + 1, vertices + 2, vertices + 3));
                        vertices += 3;
                        any = true;
                    }
                }
            }
            models += usize::from(any);
        }
        std::fs::write(&out, lines)?;
        eprintln!("surface: {} triangles from {models} models in x {x0}..{x1}, y {y0}..{y1} -> {}", vertices / 3, out.display());
    }
    if !boxes.is_empty() {
        return Ok(());
    }
    export_parks(&out.with_extension("visparks"), &placed, &archetypes, &files, &keys)?;
    // Each archetype's rails once (model space).
    let mut model_rails: HashMap<u32, Arc<Vec<(Vec<V>, u32)>>> = HashMap::new();
    let mut by_shader: HashMap<u32, usize> = HashMap::new();
    let mut by_model: HashMap<u32, usize> = HashMap::new();
    let mut dicts: HashMap<u32, HashMap<u32, Drawable>> = HashMap::new();
    let by_hash: HashMap<u32, &gta_archives::Entry> = files.iter().map(|(n, e)| (rage_joaat(n.rsplit_once('.').map_or(n.as_str(), |(s, _)| s)), e)).collect();
    let ydd_by_hash: HashMap<u32, &gta_archives::Entry> = files.iter().filter(|(n, _)| n.ends_with(".ydd")).map(|(n, e)| (rage_joaat(n.trim_end_matches(".ydd")), e)).collect();
    let ydr_by_hash: HashMap<u32, &gta_archives::Entry> = files.iter().filter(|(n, _)| n.ends_with(".ydr")).map(|(n, e)| (rage_joaat(n.trim_end_matches(".ydr")), e)).collect();
    let _ = by_hash;
    let mut total = 0usize;
    let mut written: Vec<Vec<V>> = Vec::new();
    let mut seen: std::collections::HashSet<([i64; 3], [i64; 3], [i64; 3])> = std::collections::HashSet::new();
    for (k, (hash, place)) in placed.iter().enumerate() {
        if k % 50000 == 0 { eprintln!("placement {k}/{}: {} rails", placed.len(), written.len()); }
        let rails = model_rails.entry(*hash).or_insert_with(|| {
            let Some(a) = archetypes.get(hash) else { return Arc::new(Vec::new()) };
            if a.is_fragment() { return Arc::new(Vec::new()); }
            let model = a.model_hash();
            let d = if a.in_drawable_dictionary() {
                // Retain rail polylines per model, not every decoded drawable
                // dictionary for the whole city. These meshes can be large.
                if !dicts.contains_key(&a.drawable_dictionary_hash) && dicts.len() >= 4 {
                    dicts.clear();
                }
                let dict = dicts.entry(a.drawable_dictionary_hash).or_insert_with(|| {
                    ydd_by_hash.get(&a.drawable_dictionary_hash).and_then(|e| e.read(&keys).ok()).and_then(|b| parse_ydd(&b).ok())
                        .map(|v| v.into_iter().map(|d| (d.hash, d.drawable)).collect()).unwrap_or_default()
                });
                dict.get(&model).cloned()
            } else {
                ydr_by_hash.get(&model).and_then(|e| e.read(&keys).ok()).and_then(|b| parse_ydr(&b).ok())
            };
            Arc::new(d.map(|d| drawable_rails(&d)).unwrap_or_default())
        }).clone();
        for (r, shader) in rails.iter() {
            let r: Vec<V> = r.iter().map(|&p| place.apply(p, GTA)).collect();
            // The same rail placed twice (a model and its copy in another
            // map file, overlapping placements): once, by its ends and middle
            // on a 2 cm grid.
            let key = |p: V| p.map(|x| (x / 0.02).round() as i64);
            let (a, m, z) = (key(r[0]), key(r[r.len() / 2]), key(r[r.len() - 1]));
            let k = if a <= z { (a, m, z) } else { (z, m, a) };
            if !seen.insert(k) {
                continue;
            }
            written.push(r);
            *by_shader.entry(*shader).or_default() += 1;
            *by_model.entry(*hash).or_default() += 1;
            total += 1;
        }
    }
    eprintln!("{} models with rails of {} looked at; {total} rails placed", model_rails.values().filter(|r| !r.is_empty()).count(), model_rails.len());
    let names: HashMap<u32, &str> = files.keys().map(|n| (rage_joaat(n.rsplit_once('.').map_or(n.as_str(), |(s, _)| s)), n.as_str())).collect();
    let mut s: Vec<_> = by_shader.into_iter().collect();
    s.sort_by(|a, b| b.1.cmp(&a.1));
    for (h, n) in s.iter().take(40) { eprintln!("shader {h:08x}: {n} rails"); }
    let mut m: Vec<_> = by_model.into_iter().collect();
    m.sort_by(|a, b| b.1.cmp(&a.1));
    for (h, n) in m.iter().take(80) { eprintln!("model {} ({h:08x}): {n} rails", names.get(h).copied().unwrap_or("?")); }
    // SVRL v2, tiled: magic, version, cell size, cell count, then the
    // cells (x, y as i32, rail count, byte offset from the file start), then
    // per cell its rails (point count, points as f32 x3). A rail belongs to
    // the cell of its middle point; readers look one cell further round.
    let mut cells: std::collections::BTreeMap<(i32, i32), Vec<usize>> = std::collections::BTreeMap::new();
    for (i, r) in written.iter().enumerate() {
        let m = r[r.len() / 2];
        cells.entry(((m[0] / CELL).floor() as i32, (m[1] / CELL).floor() as i32)).or_default().push(i);
    }
    let mut body: Vec<u8> = Vec::new();
    let mut table: Vec<(i32, i32, u32, u64)> = Vec::new();
    let head = 16 + cells.len() as u64 * 20;
    for (&(x, y), ids) in &cells {
        table.push((x, y, ids.len() as u32, head + body.len() as u64));
        for &i in ids {
            let r = &written[i];
            body.extend((r.len() as u32).to_le_bytes());
            for p in r { for v in p { body.extend(v.to_le_bytes()); } }
        }
    }
    let mut f = std::io::BufWriter::new(std::fs::File::create(&out)?);
    f.write_all(b"SVRL")?;
    f.write_all(&2u32.to_le_bytes())?;
    f.write_all(&CELL.to_le_bytes())?;
    f.write_all(&(table.len() as u32).to_le_bytes())?;
    for (x, y, n, off) in &table {
        f.write_all(&x.to_le_bytes())?;
        f.write_all(&y.to_le_bytes())?;
        f.write_all(&n.to_le_bytes())?;
        f.write_all(&off.to_le_bytes())?;
    }
    f.write_all(&body)?;
    f.flush()?;
    drop(f);
    SkateVRuntime::visrails::write_policy(&out)?;
    eprintln!("wrote {} rails to {}", written.len(), out.display());
    let _: &Path = &out;
    Ok(())
}
