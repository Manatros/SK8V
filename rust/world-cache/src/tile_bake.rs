//! Resource bounds walked through the runtime decoder, then final world rules.
use crate::{interiors, props, vfs};
use SkateVRuntime::{
    coords,
    tiles::{self, Tile},
    world,
};
use bevy_math::Vec3;
use rage_formats::{
    BoundTransform,
    resource::{ResReader, SYSTEM_BASE, prepare_rsc7, u16_le, u32_le, u64_le, vec3_le},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::Instant,
};
use svwc::bounds::{self, Place};

#[derive(Default)]
struct Collision {
    tris: Vec<svwc::Tri>,
    stairs: Vec<bool>,
    spare: Vec<svwc::Tri>,
    /// Per triangle of `tris` read back from the spool: its source file's
    /// index in `Spool::names` (face origins, design 2026-10-26 6.2).
    origins: Vec<u32>,
}

/// Cells actually touched by authored geometry, including vertical faces.
/// AABB-only marking would unnecessarily disable distant stable tiles.
fn mark_live_cells(data: &Collision, size: f32, cells: &mut BTreeSet<tiles::Cell>) {
    for t in data.tris.iter().chain(&data.spare) {
        let v = t.v.map(Vec3::from_array);
        let lo = tiles::cell(v[0].min(v[1]).min(v[2]).to_array(), size);
        let hi = tiles::cell(v[0].max(v[1]).max(v[2]).to_array(), size);
        for x in lo.0..=hi.0 { for y in lo.1..=hi.1 {
            if !tiles::clip_triangle(t.v, (x, y), size).is_empty() { cells.insert((x, y)); }
        }}
    }
}

fn touches_live(t: &svwc::Tri, size: f32, cells: &BTreeSet<tiles::Cell>) -> bool {
    let v = t.v.map(Vec3::from_array);
    let lo = tiles::cell(v[0].min(v[1]).min(v[2]).to_array(), size);
    let hi = tiles::cell(v[0].max(v[1]).max(v[2]).to_array(), size);
    cells.range((lo.0, i32::MIN)..=(hi.0, i32::MAX)).any(|&c| {
        c.1 >= lo.1 && c.1 <= hi.1 && !tiles::clip_triangle(t.v, c, size).is_empty()
    })
}


/// A tile's origin name table (design 2026-10-26 6.2).
#[derive(Default)]
struct Origins {
    names: Vec<String>,
    index: BTreeMap<String, u16>,
}
impl Origins {
    fn index(&mut self, name: String) -> u16 {
        if let Some(&k) = self.index.get(&name) { return k; }
        let k = u16::try_from(self.names.len()).expect("more than 65,535 face origins in one tile");
        self.index.insert(name.clone(), k);
        self.names.push(name);
        k
    }
}

/// Disk-backed raw collision grid. RAM holds one decoded resource / one tile's
/// support, independent of the total selected world area. Spools are temporary.
struct Spool {
    dir: PathBuf,
    next: u64,
    /// Source file names (file name only), indexed by the spool records.
    names: Vec<String>,
    index: BTreeMap<String, u32>,
}

/// Record: id u64, flags u32, origin u32, three vertices.
const RECORD: usize = 52;
impl Spool {
    fn new(out: &Path) -> std::io::Result<Self> {
        let dir = out.join("raw-spool");
        fs::create_dir_all(&dir)?;
        // Truncate only our known spool files from an interrupted run.
        for e in fs::read_dir(&dir)? {
            let p = e?.path();
            if p.extension().is_some_and(|e| e == "raw") {
                fs::remove_file(p)?;
            }
        }
        Ok(Self { dir, next: 0, names: Vec::new(), index: BTreeMap::new() })
    }
    fn push(
        &mut self,
        data: &mut Collision,
        support: &BTreeSet<tiles::Cell>,
        size: f32,
        origin: &str,
    ) -> std::io::Result<()> {
        let name = origin.rsplit(['/', '\\']).next().unwrap_or(origin).to_string();
        let origin = match self.index.get(&name) {
            Some(&k) => k,
            None => {
                let k = self.names.len() as u32;
                self.index.insert(name.clone(), k);
                self.names.push(name);
                k
            }
        };
        let mut buffers: BTreeMap<tiles::Cell, Vec<u8>> = BTreeMap::new();
        for (i, t) in data
            .tris
            .iter()
            .enumerate()
            .chain(data.spare.iter().map(|t| (usize::MAX, t)))
        {
            let v = t.v.map(Vec3::from_array);
            let min = v[0].min(v[1]).min(v[2]);
            let max = v[0].max(v[1]).max(v[2]);
            let a = tiles::cell(min.to_array(), size);
            let b = tiles::cell(max.to_array(), size);
            let mut record = Vec::with_capacity(RECORD);
            record.extend(self.next.to_le_bytes());
            self.next += 1;
            record.extend(
                (t.material as u32
                    | if i == usize::MAX {
                        1 << 16
                    } else if data.stairs.get(i).copied().unwrap_or(false) {
                        1 << 17
                    } else {
                        0
                    })
                .to_le_bytes(),
            );
            record.extend(origin.to_le_bytes());
            for p in t.v {
                for n in p {
                    record.extend(n.to_le_bytes());
                }
            }
            for x in a.0..=b.0 {
                for y in a.1..=b.1 {
                    if support.is_empty() || support.contains(&(x, y)) {
                        buffers.entry((x, y)).or_default().extend(&record);
                    }
                }
            }
        }
        for (c, b) in buffers {
            use std::fs::OpenOptions;
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.dir.join(format!("{}_{}.raw", c.0, c.1)))?
                .write_all(&b)?;
        }
        *data = Collision::default();
        Ok(())
    }
    fn around(&self, at: Vec3, half: f32, size: f32) -> std::io::Result<Collision> {
        let mut seen = BTreeSet::new();
        let mut out = Collision::default();
        for c in tiles::cells([at.x, at.y], half, size) {
            let p = self.dir.join(format!("{}_{}.raw", c.0, c.1));
            let bytes = match fs::read(p) {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            for r in bytes.chunks_exact(RECORD) {
                let id = u64::from_le_bytes(r[..8].try_into().unwrap());
                if !seen.insert(id) {
                    continue;
                }
                let flags = u32::from_le_bytes(r[8..12].try_into().unwrap());
                let origin = u32::from_le_bytes(r[12..16].try_into().unwrap());
                let t = svwc::Tri {
                    material: (flags & 255) as u8,
                    v: std::array::from_fn(|i| {
                        std::array::from_fn(|k| {
                            f32::from_le_bytes(
                                r[16 + (i * 3 + k) * 4..20 + (i * 3 + k) * 4]
                                    .try_into()
                                    .unwrap(),
                            )
                        })
                    }),
                };
                if !(0..2).all(|k| {
                    t.v.iter().any(|p| p[k] >= at[k] - half)
                        && t.v.iter().any(|p| p[k] <= at[k] + half)
                }) {
                    continue;
                }
                if flags & (1 << 16) != 0 {
                    out.spare.push(t);
                } else {
                    out.tris.push(t);
                    out.stairs.push(flags & (1 << 17) != 0);
                    out.origins.push(origin);
                }
            }
        }
        Ok(out)
    }
}
impl Drop for Spool {
    fn drop(&mut self) {
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|e| e == "raw") {
                    let _ = fs::remove_file(p);
                }
            }
        }
        let _ = fs::remove_dir(&self.dir);
    }
}

/// OUT MINX MAXX MINY MAXY GTA KEYS COLLISION CACHE.
/// All complete cells intersecting the collision search box, including empty
/// coverage (empty tiles prevent live geometry leaking back into the base).
pub fn run_box(a: Vec<String>) -> Result<(), Box<dyn Error>> {
    if a.len() != 9 {
        return Err("--bake-world-box OUT MINX MAXX MINY MAXY GTA KEYS COLLISION CACHE".into());
    }
    let b: Vec<f32> = a[1..5]
        .iter()
        .map(|n| n.parse())
        .collect::<Result<_, _>>()?;
    if b.iter().any(|n| !n.is_finite() || n.abs() > 20000.) || b[0] >= b[1] || b[2] >= b[3] {
        return Err("invalid box".into());
    }
    let mut region = BTreeSet::new();
    for x in (b[0] / 64.).floor() as i32..=(b[1] / 64.).floor() as i32 {
        for y in (b[2] / 64.).floor() as i32..=(b[3] / 64.).floor() as i32 {
            region.insert((x, y));
        }
    }
    bake(
        PathBuf::from(&a[0]),
        region,
        64.,
        &a[5..9],
        None,
        format!("Collision box {b:?}"),
    )
}

fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    })
}

fn hash_file(path: &Path) -> std::io::Result<u64> {
    use std::io::Read;
    let mut f = fs::File::open(path)?;
    let mut b = [0u8; 65536];
    let mut h = 0xcbf29ce484222325u64;
    loop {
        let n = f.read(&mut b)?;
        if n == 0 {
            return Ok(h);
        }
        for v in &b[..n] {
            h = (h ^ *v as u64).wrapping_mul(0x100000001b3);
        }
    }
}

fn placement(stack: &[BoundTransform], outer: &Place) -> Place {
    let mut rows = outer.rows.map(Vec3::from_array);
    for xf in stack {
        let child = xf.columns.map(|p| Vec3::from_array(array(p)));
        let dir = |p: Vec3| rows[0] * p.x + rows[1] * p.y + rows[2] * p.z;
        rows = [
            dir(child[0]),
            dir(child[1]),
            dir(child[2]),
            dir(child[3]) + rows[3],
        ];
    }
    Place {
        rows: rows.map(|p| p.to_array()),
        identity: outer.identity && stack.is_empty(),
    }
}

fn walk(
    r: &ResReader<'_>,
    va: u64,
    stack: &mut Vec<BoundTransform>,
    flags: (u32, u32),
    outer: &Place,
    clip: bounds::Clip,
    out: &mut Collision,
) -> Result<(), String> {
    if stack.len() > 8 {
        return Err("bound nesting exceeds limit".into());
    }
    let h = r.resolve(va, 0x70).ok_or("invalid bound header")?;
    if h[0x10] == 10 {
        // Match live::composite: a filtered composite excludes its entire
        // subtree. Child defaults must not turn a camera/vehicle-only group
        // into solid walking collision. Detail-only composite groups are
        // not traversed by the live decoder either.
        if !bounds::skater_collides(flags.0, flags.1) {
            return Ok(());
        }
        let b = r.resolve(va, 0xb0).ok_or("invalid composite")?;
        let n = u16_le(b, 0xa0) as usize;
        let pointers = r
            .read_u64_list(u64_le(b, 0x70), n)
            .ok_or("invalid children")?;
        let transforms = r.resolve(u64_le(b, 0x78), n * 64);
        let child_flags = r.resolve(u64_le(b, 0x90), n * 8);
        for (i, p) in pointers.into_iter().enumerate() {
            if p == 0 {
                continue;
            }
            let xf = transforms
                .map(|b| BoundTransform {
                    columns: [
                        vec3_le(b, i * 64),
                        vec3_le(b, i * 64 + 16),
                        vec3_le(b, i * 64 + 32),
                        vec3_le(b, i * 64 + 48),
                    ],
                })
                .unwrap_or_else(BoundTransform::identity);
            let f = child_flags
                .map(|b| (u32_le(b, i * 8), u32_le(b, i * 8 + 4)))
                .unwrap_or((0, 0));
            stack.push(xf);
            walk(r, p, stack, f, outer, clip, out)?;
            stack.pop();
        }
        return Ok(());
    }
    let spare = !bounds::skater_collides(flags.0, flags.1);
    if spare && !bounds::solid_to_sight(flags.0, flags.1) {
        return Ok(());
    }
    let place = placement(stack, outer);
    let mut counts = bounds::Counts::default();
    let target = if spare { &mut out.spare } else { &mut out.tris };
    match h[0x10] {
        4 | 8 => {
            let b = r.resolve(va, 0x130).ok_or("invalid geometry")?;
            let (nv, np) = (u32_le(b, 0xd0) as usize, u32_le(b, 0xd4) as usize);
            if nv == 0 || np == 0 {
                return Ok(());
            }
            let vb = r
                .resolve(u64_le(b, 0xb0), nv * 6)
                .ok_or("invalid vertices")?;
            let pb = r
                .resolve(u64_le(b, 0x88), np * 16)
                .ok_or("invalid polygons")?;
            let mb = r
                .resolve(u64_le(b, 0xf0), (b[0x120] as usize).max(4) * 8)
                .unwrap_or(&[]);
            let pm = r.resolve(u64_le(b, 0x118), np).unwrap_or(&[]);
            let mut unused = Vec::new();
            bounds::geometry_marked(
                vb,
                pb,
                mb,
                pm,
                array(vec3_le(b, 0x90)),
                array(vec3_le(b, 0xa0)),
                &place,
                clip,
                target,
                if spare { &mut unused } else { &mut out.stairs },
                &mut counts,
            );
        }
        0 | 1 | 3 | 12 | 13 => bounds::primitive(h, &place, clip, target, &mut counts),
        _ => {}
    }
    if !spare {
        out.stairs.resize(out.tris.len(), false);
    }
    Ok(())
}

fn decode(
    bytes: &[u8],
    place: &Place,
    clip: bounds::Clip,
    out: &mut Collision,
) -> Result<(), String> {
    let (system, graphics) = prepare_rsc7(bytes).map_err(|e| e.to_string())?;
    walk(
        &ResReader {
            system: &system,
            graphics: &graphics,
        },
        SYSTEM_BASE,
        &mut Vec::new(),
        (0, 0),
        place,
        clip,
        out,
    )
}

/// SKATEV_BAKE_PROBE=x,y,z,r: name every input whose triangles come within r
/// of the point (which file holds a surface seen in the game).
fn probe_report(label: &str, tris: &[svwc::Tri]) {
    static PROBE: std::sync::OnceLock<Option<[f32; 4]>> = std::sync::OnceLock::new();
    let Some(p) = *PROBE.get_or_init(|| {
        let v: Vec<f32> = std::env::var("SKATEV_BAKE_PROBE").ok()?.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
    }) else { return };
    let near: Vec<&svwc::Tri> = tris.iter().filter(|t| {
        let c = [0, 1, 2].map(|k| (t.v[0][k] + t.v[1][k] + t.v[2][k]) / 3.0);
        ((c[0] - p[0]).powi(2) + (c[1] - p[1]).powi(2) + (c[2] - p[2]).powi(2)).sqrt() <= p[3]
    }).collect();
    if !near.is_empty() {
        let mut materials = std::collections::BTreeMap::new();
        for t in &near { *materials.entry(t.material).or_insert(0) += 1; }
        eprintln!("probe: {} triangles from {label}, materials {materials:?}", near.len());
        if std::env::var("SKATEV_BAKE_PROBE_TRIS").is_ok() {
            for t in &near { eprintln!("probe tri {} {:?}", t.material, t.v); }
        }
    }
}

fn identity() -> Place {
    Place {
        rows: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [0., 0., 0.]],
        identity: true,
    }
}

/// Static map bounds in the Story archive namespace. Local models are placed
/// through their instances; procedural spawn surfaces are not collision.
fn is_world_bound(path: &Path) -> bool {
    let path = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    let name = path.rsplit('/').next().unwrap_or(&path);
    path.contains("/levels/gta5/")
        && !path.contains("/interiors/") && !path.contains("/props/")
        && name.ends_with(".ybn") && !name.starts_with("ma@")
        // hi@ is GTA's vehicle-only high-detail copy. Read as world ground it
        // doubled surfaces everywhere (Vespucci bowl decks: 965 coincident
        // triangles vs 22 before 6cce017; seams and phantom lips, 2026-10-24).
        && !name.starts_with("hi@")
}

/// A prop drawable's embedded bound, placed and walked like a map bound (so
/// its cylinders and capsules become tube rails). Drawable bound: `gtaDrawable` +0xC8.
fn decode_drawable(bytes: &[u8], place: &Place, clip: bounds::Clip, out: &mut Collision) -> Result<(), String> {
    let (system, graphics) = prepare_rsc7(bytes).map_err(|e| e.to_string())?;
    let r = ResReader { system: &system, graphics: &graphics };
    let head = r.resolve(SYSTEM_BASE, 0xD0).ok_or("invalid drawable header")?;
    let bound = u64_le(head, 0xC8);
    if bound == 0 { return Ok(()); }
    walk(&r, bound, &mut Vec::new(), (0, 0), place, clip, out)
}

fn story_world_bounds(meta: &Path) -> Vec<PathBuf> {
    interiors::layered(&meta.join("base"), Some(&meta.join("layers")), "ybn", is_world_bound)
}

fn mlo_place(e: &rage_formats::YmapEntity) -> Place {
    let origin = interiors::place(e, rage_formats::math::Vec3::ZERO, false);
    let mut rotation = e.clone();
    rotation.position = rage_formats::math::Vec3::ZERO;
    let rows = [rage_formats::math::Vec3::new(1.,0.,0.), rage_formats::math::Vec3::new(0.,1.,0.),
        rage_formats::math::Vec3::new(0.,0.,1.)].map(|v| array(interiors::place(&rotation,v,false)));
    Place { rows:[rows[0],rows[1],rows[2],array(origin)], identity:false }
}

fn fallback_floor(t: &svwc::Tri) -> bool {
    if svwc::bounds::is_water(t.material) {
        return false;
    }
    let [a,b,c] = t.v.map(Vec3::from_array);
    let n = (b-a).cross(c-a);
    let length = n.length();
    length > 1e-9 && n.z / length >= 0.5
}

/// A verified, invariant Story-only floor cache. Do not reuse an all-DLC flat
/// extraction: its inactive floors can otherwise return through gap filling.
pub fn run_story_map(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    if args.len() != 3 { return Err("--bake-story-map OUT GTA KEYS".into()); }
    let out = Path::new(&args[0]);
    let gta = Path::new(&args[1]);
    let keys = Path::new(&args[2]);
    vfs::mount_world(gta, gta_archives::keys(keys).map_err(|e|e.to_string())?)?;
    let meta = Path::new(vfs::ROOT);
    interiors::validate_manifests(meta)?;
    let local = interiors::interior_bound_hashes(meta);
    let stateful = interiors::state_dependent_maps(meta);
    let shells = interiors::Shells::load(meta, &[meta.join("base")], Some(&meta.join("layers")));
    let mut floors = Vec::new();
    let mut inputs = 0usize;
    let mut append = |bytes: &[u8], place: &Place| -> Result<(), String> {
        let mut data = Collision::default();
        decode(bytes, place, (-20000.,-20000.,20000.,20000.), &mut data)?;
        floors.extend(data.tris.into_iter().filter(fallback_floor));
        bounds::take_tubes(); // no rail state is retained by a floor export
        inputs += 1;
        Ok(())
    };
    for path in interiors::layered(&meta.join("base"),Some(&meta.join("layers")),"ymap",
        |p| !stateful.contains(&interiors::stem_hash(p))) {
        let bytes = vfs::read(&path)?;
        if bytes.starts_with(b"PSIN") {
            eprintln!("PSIN placement map not supported: {}",path.display());
            continue;
        }
        let instances = rage_formats::parse_ymap_mlo_instances(&bytes)
            .map_err(|e|format!("{}: {e}",path.display()))?;
        for inst in instances {
            let place = mlo_place(&inst.entity);
            for bound in shells.bound_paths(&inst.entity) {
                append(&vfs::read(&bound)?,&place).map_err(|e|format!("{}: {e}",bound.display()))?;
            }
        }
    }
    for path in story_world_bounds(meta) {
        let hash = interiors::stem_hash(&path);
        if local.contains(&hash) || stateful.contains(&hash) { continue; }
        append(&vfs::read(&path)?,&identity()).map_err(|e|format!("{}: {e}",path.display()))?;
    }
    drop(append);
    if floors.is_empty() { return Err("Story fallback contains no walking floors".into()); }
    if let Some(parent) = out.parent() { fs::create_dir_all(parent)?; }
    let mut writer = BufWriter::new(fs::File::create(out)?);
    let records = svwc::write(&mut writer,&floors,64.)?;
    writer.flush()?;
    drop(writer);
    let reopened = svwc::Cache::open(out)?;
    if reopened.record_count as usize != records { return Err("Story fallback record count mismatch".into()); }
    SkateVRuntime::baked::write_policy(out)?;
    eprintln!("Story fallback: {inputs} invariant sources, {} walking floors, {records} records, {} tiles; peak {} bytes",
        floors.len(),reopened.tile_count(),peak_memory());
    Ok(())
}

fn verify_coverage(dir: &Path, box_: [f32;4]) -> Result<(usize,usize,usize,usize),Box<dyn Error>> {
    if box_.iter().any(|n| !n.is_finite() || n.abs()>20000.) || box_[0]>=box_[1] || box_[2]>=box_[3] {
        return Err("invalid verification bounds".into());
    }
    let size: f32 = fs::read_to_string(dir.join("tile-size.txt"))?.trim().parse()?;
    if !size.is_finite() || !(1. ..=4096.).contains(&size) { return Err("invalid tile size".into()); }
    let mut cache = tiles::Cache::default();
    let (mut baked,mut live,mut triangles,mut rails) = (0,0,0,0);
    let mut source = None;
    for x in (box_[0]/size).floor() as i32..=(box_[1]/size).floor() as i32 {
        for y in (box_[2]/size).floor() as i32..=(box_[3]/size).floor() as i32 {
            let c = (x,y);
            let selected = cache.select(dir,[(x as f32+0.5)*size,(y as f32+0.5)*size],0.)
                .ok_or_else(||format!("cell {c:?}: invalid rules or state policy"))?;
            if selected.live_only == [c] {
                if tiles::path(dir,c).exists() { return Err(format!("cell {c:?}: stale baked output in a live-only cell").into()); }
                live += 1;
            } else if let Some(tile) = selected.baked.get(&c) {
                if source.as_ref().is_some_and(|s| s != &tile.source) { return Err("mixed tile source generations".into()); }
                source.get_or_insert_with(||tile.source.clone());
                baked += 1;
                triangles += tile.triangles.len();
                rails += tile.rails.len();
            } else { return Err(format!("cell {c:?}: missing, corrupt or incompatible invariant tile").into()); }
        }
    }
    Ok((baked,live,triangles,rails))
}

/// Installation gate: complete coverage is invariant tiles plus intentional
/// live cells, never an arbitrary count of files that might be stale.
pub fn run_verify_world(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    if args.len()!=5 { return Err("--verify-world WORLD MINX MAXX MINY MAXY".into()); }
    let world = Path::new(&args[0]);
    let map = world.join("sk8v-map.svwc");
    if !SkateVRuntime::baked::verified_policy(&map) { return Err("unverified Story fallback map".into()); }
    let map = svwc::Cache::open(&map)?;
    if map.tile_count()==0 || map.record_count==0 { return Err("empty Story fallback map".into()); }
    if !SkateVRuntime::visrails::verified_policy(&world.join("sk8v.visrails")) {
        return Err("unverified Story visual rails".into());
    }
    let bounds = [args[1].parse()?,args[2].parse()?,args[3].parse()?,args[4].parse()?];
    let (baked,live,triangles,rails) = verify_coverage(&world.join("tiles"),bounds)?;
    println!("Verified world: {baked} invariant cells + {live} live cells, {triangles} triangles, {rails} rail records");
    Ok(())
}

/// OUT X Y HALF GTA KEYS COLLISION CACHE [TILE_SIZE], in GTA metres.
pub fn run(a: Vec<String>) -> Result<(), Box<dyn Error>> {
    if !(8..=9).contains(&a.len()) {
        return Err("--bake-world OUT X Y HALF GTA KEYS COLLISION CACHE [TILE_SIZE]".into());
    }
    let out = PathBuf::from(&a[0]);
    let center = [a[1].parse::<f32>()?, a[2].parse::<f32>()?];
    let half: f32 = a[3].parse()?;
    let size: f32 = a.get(8).map(|s| s.parse()).transpose()?.unwrap_or(64.);
    if !center.iter().all(|v| v.is_finite())
        || !half.is_finite()
        || half <= 0.
        || half > 20000.
        || !size.is_finite()
        || !(8. ..=128.).contains(&size)
    {
        return Err("invalid region or tile size (8..128 m)".into());
    }
    bake(
        out,
        tiles::cells(center, half, size).into_iter().collect(),
        size,
        &a[4..8],
        None,
        format!("Square centre {center:?}, half-size {half} m"),
    )
}

/// OUT LISTFILE GTA KEYS COLLISION CACHE (fixed 64 m grid).
pub fn run_tiles(a: Vec<String>) -> Result<(), Box<dyn Error>> {
    if a.len() != 6 {
        return Err("--bake-world-tiles OUT LISTFILE GTA KEYS COLLISION CACHE".into());
    }
    let region = parse_cells(&fs::read_to_string(&a[1])?)?;
    bake(
        PathBuf::from(&a[0]),
        region,
        64.,
        &a[2..6],
        None,
        format!("Explicit tile list {}", a[1]),
    )
}

fn parse_cells(text: &str) -> Result<BTreeSet<tiles::Cell>, Box<dyn Error>> {
    let mut cells = BTreeSet::new();
    for (i, line) in text.lines().enumerate() {
        let words: Vec<_> = line.split('#').next().unwrap().split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        if words.len() != 2 {
            return Err(format!("tile list line {}: expected tx ty", i + 1).into());
        }
        let c = (words[0].parse::<i32>()?, words[1].parse::<i32>()?);
        if c.0.abs_diff(0) > 1_000_000 || c.1.abs_diff(0) > 1_000_000 {
            return Err("tile coordinate out of range".into());
        }
        cells.insert(c);
    }
    if cells.is_empty() {
        return Err("empty tile selection".into());
    }
    Ok(cells)
}

/// Tiles whose closed XY square is within radius of a point (Euclidean metres).
fn near_point(cells: &mut BTreeSet<tiles::Cell>, p: Vec3, radius: f32, size: f32) {
    for x in ((p.x - radius) / size).ceil() as i32 - 1..=((p.x + radius) / size).floor() as i32 {
        for y in ((p.y - radius) / size).ceil() as i32 - 1..=((p.y + radius) / size).floor() as i32
        {
            let q = bevy_math::Vec2::new(
                p.x.clamp(x as f32 * size, (x + 1) as f32 * size),
                p.y.clamp(y as f32 * size, (y + 1) as f32 * size),
            );
            if q.distance_squared(p.truncate()) <= radius * radius {
                cells.insert((x, y));
            }
        }
    }
}

/// OUT GTA KEYS COLLISION CACHE [MARGIN]. Conservative superset: all lines,
/// since collision overhead alone cannot distinguish a tunnel from a bridge.
pub fn run_tracks(a: Vec<String>) -> Result<(), Box<dyn Error>> {
    if !(5..=6).contains(&a.len()) {
        return Err("--bake-world-along-tracks OUT GTA KEYS COLLISION CACHE [MARGIN]".into());
    }
    let margin: f32 = a.get(5).map(|s| s.parse()).transpose()?.unwrap_or(40.);
    if !margin.is_finite() || !(0. ..=512.).contains(&margin) {
        return Err("invalid track margin (0..512 m)".into());
    }
    let mut region = BTreeSet::new();
    let dir = Path::new(&a[4]).with_extension("tracks");
    let mut nodes = 0;
    for n in 1..=12 {
        let text = fs::read_to_string(dir.join(format!("trains{n}.dat")))?;
        let line = SkateVRuntime::tracks::parse(&text);
        if line.len() < 2 {
            return Err(format!("trains{n}.dat: fewer than two nodes").into());
        }
        nodes += line.len();
        for p in line {
            near_point(&mut region, p, margin, 64.);
        }
    }
    bake(
        PathBuf::from(&a[0]),
        region,
        64.,
        &a[1..5],
        Some(margin),
        format!(
            "All 12 track lines, {nodes} nodes, {margin} m node-to-tile XY distance; conservative superset of metro/tunnels (no reliable street-height classification)"
        ),
    )
}

fn bake(
    out: PathBuf,
    mut region: BTreeSet<tiles::Cell>,
    size: f32,
    paths: &[String],
    station_margin: Option<f32>,
    selection: String,
) -> Result<(), Box<dyn Error>> {
    let start = Instant::now();
    let gta = Path::new(&paths[0]);
    let keys = Path::new(&paths[1]);
    // Retained in the CLI for older callers. A flattened extraction loses
    // archive provenance and can contain Online overrides of Story bounds.
    let _legacy_collision = Path::new(&paths[2]);
    let cache = Path::new(&paths[3]);
    fs::create_dir_all(&out)?;
    vfs::mount_world(gta, gta_archives::keys(keys).map_err(|e| e.to_string())?)?;
    let meta = Path::new(vfs::ROOT);
    let manifest_count = interiors::validate_manifests(meta)?;
    eprintln!("Validated {manifest_count} world manifests before state classification");
    let local = interiors::interior_bound_hashes(meta);
    let shells = interiors::Shells::load(meta, &[meta.join("base")], Some(&meta.join("layers")));

    // The same 128 m support square as the runtime, around each tile centre.
    let margin = world::RADIUS;
    let lo = [
        region.iter().map(|c| c.0).min().unwrap() as f32 * size - margin,
        region.iter().map(|c| c.1).min().unwrap() as f32 * size - margin,
    ];
    let hi = [
        (region.iter().map(|c| c.0).max().unwrap() + 1) as f32 * size + margin,
        (region.iter().map(|c| c.1).max().unwrap() + 1) as f32 * size + margin,
    ];
    let clip = if station_margin.is_some() {
        (-20000., -20000., 20000., 20000.)
    } else {
        (lo[0], lo[1], hi[0], hi[1])
    };
    let mut support = support_cells(&region, size, margin);
    let mut stations = Vec::new();
    let mut data = Collision::default();
    let mut spool = Spool::new(&out)?;
    let mut sources = BTreeMap::new();
    let mut live_cells = BTreeSet::new();
    // Named YTYPs recover MLO archetype hashes even when their bound names
    // differ. Read this metadata once, before walking placements.
    let mut metro_mlos = BTreeSet::new();
    for path in interiors::layered(
        &meta.join("base"),
        Some(&meta.join("layers")),
        "ytyp",
        |p| {
            let name = p.to_string_lossy().to_ascii_lowercase();
            name.contains("metro") || name.contains("subway")
        },
    ) {
        let bytes = vfs::read(&path)?;
        let ytyp =
            rage_formats::parse_ytyp(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        metro_mlos.extend(ytyp.mlos.into_iter().map(|m| m.name_hash));
        sources.insert(path.display().to_string(), hash(&bytes));
    }

    let files = story_world_bounds(meta);
    // SKATEV_BAKE_LIST=<text>: print the collision inputs whose path holds it.
    if let Ok(needle) = std::env::var("SKATEV_BAKE_LIST") {
        for f in files.iter().filter(|f| f.to_string_lossy().to_ascii_lowercase().contains(&needle.to_ascii_lowercase())) {
            eprintln!("input: {}", f.display());
        }
    }
    bounds::take_tubes();
    let toggled = interiors::state_dependent_maps(meta);
    let ymaps = interiors::layered(
        &meta.join("base"),
        Some(&meta.join("layers")),
        "ymap",
        |_| true,
    );
    eprintln!("Reading {} placement maps", ymaps.len());
    let mut instances = 0;
    let mut skipped = Vec::new();
    for path in ymaps {
        let bytes = vfs::read(&path)?;
        let stateful = toggled.contains(&interiors::stem_hash(&path));
        if bytes.starts_with(b"PSIN") {
            // The existing interior reader accepts RSC7 Meta maps only.
            eprintln!("PSIN placement map not supported: {}", path.display());
            sources.insert(path.display().to_string(), hash(&bytes));
            skipped.push(path.display().to_string());
            continue;
        }
        let placements = rage_formats::parse_ymap_mlo_instances(&bytes)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        for inst in placements {
            let e = inst.entity;
            // MLOs can span several tiles; only the decoder clips them.
            let origin = interiors::place(&e, rage_formats::math::Vec3::ZERO, false);
            let mut rotation = e.clone();
            rotation.position = rage_formats::math::Vec3::ZERO;
            let rows = [
                rage_formats::math::Vec3::new(1., 0., 0.),
                rage_formats::math::Vec3::new(0., 1., 0.),
                rage_formats::math::Vec3::new(0., 0., 1.),
            ]
            .map(|v| array(interiors::place(&rotation, v, false)));
            let place = Place {
                rows: [rows[0], rows[1], rows[2], array(origin)],
                identity: false,
            };
            let bound_paths = shells.bound_paths(&e);
            let is_metro = station_margin.is_some() && (metro_mlos.contains(&e.archetype_hash)
                || bound_paths.iter().any(|p| {
                    let name = p.to_string_lossy().to_ascii_lowercase();
                    name.contains("metro") || name.contains("subway")
                }));
            for bound in bound_paths {
                let b = if bound.starts_with(meta) {
                    vfs::read(&bound)?
                } else {
                    fs::read(&bound)?
                };
                let before = (data.tris.len(), data.spare.len());
                let probed = data.tris.len();
                decode(
                    &b,
                    &place,
                    // Whole shells for stations and state-dependent
                    // placements (their footprint decides the cells), the
                    // slice box for everything else (memory per worker).
                    if is_metro || stateful { (-20000., -20000., 20000., 20000.) } else { clip },
                    &mut data,
                )
                .map_err(|err| format!("{}: {err}", bound.display()))?;
                probe_report(&format!("{} (placed by {})", bound.display(), path.display()), &data.tris[probed..]);
                if stateful {
                    mark_live_cells(&data, size, &mut live_cells);
                    data = Collision::default();
                    sources.insert(bound.display().to_string(), hash(&b));
                    continue;
                }
                if is_metro {
                    if let Some(radius) = station_margin {
                        // Include complete placed shell footprints, not only their origins.
                        for t in data.tris[before.0..].iter().chain(&data.spare[before.1..]) {
                            let min =
                                t.v.map(Vec3::from_array)
                                    .into_iter()
                                    .reduce(Vec3::min)
                                    .unwrap();
                            let max =
                                t.v.map(Vec3::from_array)
                                    .into_iter()
                                    .reduce(Vec3::max)
                                    .unwrap();
                            region.extend(tiles::cells(
                                [(min.x + max.x) * 0.5, (min.y + max.y) * 0.5],
                                (max.x - min.x).max(max.y - min.y) * 0.5 + radius,
                                size,
                            ));
                        }
                        support = support_cells(&region, size, margin);
                        stations.push(format!(
                            "{} at {:.3} {:.3} {:.3}",
                            bound.display(),
                            origin.x,
                            origin.y,
                            origin.z
                        ));
                    }
                }
                sources.insert(bound.display().to_string(), hash(&b));
                spool.push(&mut data, &BTreeSet::new(), size, &bound.display().to_string())?;
                instances += 1;
            }
            sources.insert(path.display().to_string(), hash(&bytes));
        }
    }

    eprintln!("Reading {} map bounds", files.len());
    for (i, path) in files.into_iter().enumerate() {
        if i % 1000 == 0 {
            eprintln!("map bounds: {i} read; {} triangles", data.tris.len());
        }
        if local.contains(&interiors::stem_hash(&path)) {
            continue;
        }
        let bytes = vfs::read(&path)?;
        if toggled.contains(&interiors::stem_hash(&path)) {
            decode(&bytes, &identity(), (-20000., -20000., 20000., 20000.), &mut data)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            mark_live_cells(&data, size, &mut live_cells);
            data = Collision::default();
            sources.insert(path.display().to_string(), hash(&bytes));
            continue;
        }
        let probed = data.tris.len();
        decode(&bytes, &identity(), clip, &mut data)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        probe_report(&path.display().to_string(), &data.tris[probed..]);
        spool.push(&mut data, &support, size, &path.display().to_string())?;
        sources.insert(path.display().to_string(), hash(&bytes));
    }
    // World YBNs do not contain ordinary drawable props' embedded physics.
    // The legacy exporter already placed these bounds; the tiled path must
    // retain that source too. Do this after station support expansion so map
    // order cannot omit furniture near a newly selected station boundary.
    // One owner per placement (design 2026-10-31-static-authored-prop-collision):
    // only placements GTA keeps out of its object pool are baked; benches and
    // other object-pool props stay live in the host (b4c201a), doors dynamic.
    let mut prop_library = props::Library::load(meta);
    prop_library.require_static_placements();
    let mut prop_stats = props::Stats::default();
    let mut prop_triangles = 0usize;
    let mut prop_models: std::collections::HashMap<PathBuf, Option<Vec<u8>>> = std::collections::HashMap::new();
    for p in interiors::layered(&meta.join("base"), Some(&meta.join("layers")), "ytyp", |_| true) {
        sources.insert(p.display().to_string(), hash(&vfs::read(&p)?));
    }
    for p in interiors::layered(&meta.join("base"), Some(&meta.join("layers")), "ymap", |_| true) {
        let bytes = vfs::read(&p)?;
        if bytes.starts_with(b"PSIN") { continue; }
        let header = rage_formats::parse_ymap_header(&bytes)
            .map_err(|e| format!("{}: {e}", p.display()))?;
        if header.flags & rage_formats::YmapHeader::FLAG_LOD != 0 { continue; }
        let stateful = toggled.contains(&interiors::stem_hash(&p))
            || header.flags & rage_formats::YmapHeader::FLAG_SCRIPTED != 0;
        // Props of a state-dependent or scripted map file are left out of the
        // bake (skip only). Marking their cells live (10-22) moved 140 cells,
        // the whole Vespucci/Del Perro beach strip included, to raw live
        // collision without the baked rules (Manatros' 10-24 test: "the problem
        // parts still have the old problems").
        sources.insert(p.display().to_string(), hash(&bytes));
        if stateful { continue; }
        let mut data = Collision::default();
        for (model, at, reach) in prop_library.static_placements(&bytes, &p, &mut prop_stats) {
            let [x, y] = [at.position.x, at.position.y];
            if x + reach < clip.0 || x - reach > clip.2 || y + reach < clip.1 || y - reach > clip.3 { continue; }
            let Some(model_bytes) = prop_models.entry(model.clone()).or_insert_with(|| vfs::read(&model).ok()) else { continue };
            let [r, f, u] = at.axes.map(|v| [v.x, v.y, v.z]);
            let place = Place { rows: [r, f, u, [x, y, at.position.z]], identity: false };
            decode_drawable(model_bytes, &place, clip, &mut data).map_err(|e| format!("{}: {e}", model.display()))?;
        }
        if !data.tris.is_empty() {
            prop_triangles += data.tris.len();
            spool.push(&mut data, &support, size, &format!("props:{}", p.display()))?;
        }
    }
    let mut used: Vec<_> = prop_models.iter().filter(|(_, b)| b.is_some()).map(|(p, _)| p.clone()).collect();
    used.sort();
    for p in used {
        sources.insert(p.display().to_string(), hash(prop_models[&p].as_deref().unwrap()));
    }
    eprintln!("Authored prop collision: {} exterior + {} interior static placements; {prop_triangles} invariant triangles; {} object-pool placements left live; {} with embedded collision disabled; {} door leaves retained by dynamic collision",
        prop_stats.placed, prop_stats.mlo_furniture, prop_stats.live_owned, prop_stats.no_collision, prop_stats.dynamic_doors);
    drop(prop_library);
    for p in vfs::files_under(meta, "ymf").unwrap_or_default() {
        sources.insert(p.display().to_string(), hash(&vfs::read(&p)?));
    }
    eprintln!(
        "Staged {} collision records on disk, {} resident detail triangles, {instances} MLO bounds placed",
        spool.next,
        data.spare.len()
    );
    let tubes: Vec<_> = bounds::take_tubes()
        .into_iter()
        .filter(|(a, b, r)| {
            // A stateful source may have published tubes while its footprint
            // was inspected. Never turn those into static rail collisions.
            if tiles::split_rail(&[*a, *b], size).iter().any(|(c, _)| live_cells.contains(c)) {
                return false;
            }
            let min = Vec3::from_array(*a).min(Vec3::from_array(*b)) - Vec3::splat(*r);
            let max = Vec3::from_array(*a).max(Vec3::from_array(*b)) + Vec3::splat(*r);
            let t = svwc::Tri {
                v: [min.to_array(), max.to_array(), min.to_array()],
                material: 0,
            };
            intersects_support(&t, &support, size)
        })
        .collect();
    // SKATEV_BAKE_PROBE also names the collision tubes (capsules,
    // cylinders) near the point: they become rails by tubes.rs.
    if let Some(p) = std::env::var("SKATEV_BAKE_PROBE").ok().map(|v| v.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 4) {
        for (a, b, r) in &tubes {
            let (a3, b3, p3) = (Vec3::from_array(*a), Vec3::from_array(*b), Vec3::new(p[0], p[1], p[2]));
            let ab = b3 - a3;
            let t = ((p3 - a3).dot(ab) / ab.length_squared().max(1e-9)).clamp(0., 1.);
            if (a3 + ab * t).distance(p3) <= p[3] { eprintln!("probe tube {a:?} -> {b:?} radius {r}"); }
        }
    }
    let fallback = SkateVRuntime::baked::path(cache);
    if fallback.exists() {
        if !SkateVRuntime::baked::verified_policy(&fallback) {
            return Err("raw floor fallback has no verified Story/state policy; regenerate with --bake-story-map".into());
        }
        SkateVRuntime::baked::open(cache)?;
        sources.insert(fallback.display().to_string(), hash_file(&fallback)?);
    }
    // Cleanup runs on each tile's complete 128 m context, then is released.
    SkateVRuntime::tubes::set(&tubes);
    let tracks = cache.with_extension("tracks");
    SkateVRuntime::tracks::load(&tracks);
    let visual = cache.with_extension("visrails");
    if visual.exists() && !SkateVRuntime::visrails::verified_policy(&visual) {
        return Err("visible-rail sidecar lacks a current verified Story/state policy receipt; regenerate it with visual-rails".into());
    }
    SkateVRuntime::visrails::load(&visual);
    // High-res parks: the parks' visible meshes (visparks.rs). Without a
    // verified file the parks keep GTA's collision.
    let parks = cache.with_extension("visparks");
    match SkateVRuntime::visparks::load(&parks) {
        Ok(n) => eprintln!("high-res parks: {n} visible park triangles"),
        // Like the visible rails: tiles without the park meshes would make
        // the HighResParks switch silently do nothing (peer review 2026-10-26).
        Err(e) => return Err(format!("{e}; regenerate it with visual-rails").into()),
    }
    for p in [&tracks, &visual, &parks] {
        if p.is_file() {
            sources.insert(p.display().to_string(), hash(&fs::read(p)?));
        }
        if p.is_dir() {
            let mut paths: Vec<_> = fs::read_dir(p)?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect();
            paths.sort();
            for p in paths {
                sources.insert(p.display().to_string(), hash(&fs::read(&p)?));
            }
        }
    }
    let surface_file = SkateVRuntime::surfaces::sidecar_path(cache);
    if surface_file.exists() {
        sources.insert(
            surface_file.display().to_string(),
            hash_file(&surface_file)?,
        );
    }
    for (name, bytes) in [
        (
            "world.rs",
            include_bytes!("../../skatev-runtime/src/world.rs").as_slice(),
        ),
        (
            "live.rs",
            include_bytes!("../../skatev-runtime/src/live.rs").as_slice(),
        ),
        (
            "rails.rs",
            include_bytes!("../../skatev-runtime/src/rails.rs").as_slice(),
        ),
        (
            "curbs.rs",
            include_bytes!("../../skatev-runtime/src/curbs.rs").as_slice(),
        ),
        (
            "plies.rs",
            include_bytes!("../../skatev-runtime/src/plies.rs").as_slice(),
        ),
        (
            "tubes.rs",
            include_bytes!("../../skatev-runtime/src/tubes.rs").as_slice(),
        ),
        (
            "tracks.rs",
            include_bytes!("../../skatev-runtime/src/tracks.rs").as_slice(),
        ),
        (
            "visrails.rs",
            include_bytes!("../../skatev-runtime/src/visrails.rs").as_slice(),
        ),
        (
            "visparks.rs",
            include_bytes!("../../skatev-runtime/src/visparks.rs").as_slice(),
        ),
        (
            "bounds.rs",
            include_bytes!("../../svwc/src/bounds.rs").as_slice(),
        ),
        (
            "clean.rs",
            include_bytes!("../../svwc/src/clean.rs").as_slice(),
        ),
        ("tile_bake.rs", include_bytes!("tile_bake.rs").as_slice()),
        ("interiors.rs", include_bytes!("interiors.rs").as_slice()),
        ("interiors/doors.rs", include_bytes!("interiors/doors.rs").as_slice()),
        ("vfs.rs", include_bytes!("vfs.rs").as_slice()),
        ("packs.rs", include_bytes!("packs.rs").as_slice()),
        ("primitives.rs", include_bytes!("primitives.rs").as_slice()),
        ("props.rs", include_bytes!("props.rs").as_slice()),
        ("main.rs", include_bytes!("main.rs").as_slice()),
        (
            "materials.rs",
            include_bytes!("../../skatev-runtime/src/materials.rs").as_slice(),
        ),
        (
            "surfaces.rs",
            include_bytes!("../../skatev-runtime/src/surfaces.rs").as_slice(),
        ),
        (
            "parks.rs",
            include_bytes!("../../skatev-runtime/src/parks.rs").as_slice(),
        ),
        (
            "coords.rs",
            include_bytes!("../../skatev-runtime/src/coords.rs").as_slice(),
        ),
        (
            "tiles.rs",
            include_bytes!("../../skatev-runtime/src/tiles.rs").as_slice(),
        ),
    ] {
        sources.insert(format!("rules/{name}"), hash(bytes));
    }
    let mask_text: String = live_cells.iter().map(|c| format!("{} {}\n", c.0, c.1)).collect();
    sources.insert("policy/live-cells-v1".to_owned(), hash(mask_text.as_bytes()));
    // Per worker thread: its own floor-map and surface-sidecar handles. One
    // shared handle behind a lock kept 8 threads at about one core (2026-10-24).
    struct Lookups {
        recovered: SkateVRuntime::surfaces::Recovered,
        floors: Option<svwc::Cache<std::io::BufReader<fs::File>>>,
    }
    let open_lookups = || -> Result<Lookups, Box<dyn Error + Send + Sync>> {
        Ok(Lookups {
            recovered: SkateVRuntime::surfaces::Recovered::open(cache),
            floors: if fallback.exists() { Some(svwc::Cache::open(&fallback)?) } else { None },
        })
    };
    let mut source = format!(
        "SVWT rules v2; runtime {}; tile {size}; support half-size {margin}; stairs,tubes,tracks,visual,rail-tops,curbs,plies=on; FNV1a64\n",
        env!("CARGO_PKG_VERSION")
    );
    for (p, h) in &sources {
        source.push_str(&format!("{h:016x} {p}\n"));
    }
    source.push_str(&format!("{}\n", tiles::rules_hash()));
    // A deterministic identity of the complete source list and rules.
    source.insert_str(0, &format!("source {:016x}\n", hash(source.as_bytes())));
    fs::write(out.join("sources.txt"), &source)?;
    // Keep the full inventory once on disk, not once in every tile/LRU entry.
    let source = format!(
        "source {:016x}\n{}\n{}\nSVWT v2; tile {size}; support {margin}; static defaults=on\n",
        hash(source.as_bytes()),
        tiles::rules_hash(), tiles::live_policy(&live_cells)
    );
    tiles::write_live_cells(&out, size, &source, &live_cells)?;
    fs::write(
        out.join("warnings.txt"),
        skipped
            .iter()
            .map(|p| format!("Unsupported PSIN placement map: {p}\n"))
            .collect::<String>(),
    )?;
    // A whole-region OBJ for inspection is opt-in (SKATEV_BAKE_OBJ=1): for the
    // whole map it grew past 10 GB and its formatting held the collecting
    // thread (2026-10-24).
    let mut obj = if std::env::var("SKATEV_BAKE_OBJ").is_ok_and(|v| v == "1") {
        let mut w = BufWriter::new(fs::File::create(out.join("region.obj"))?);
        writeln!(w, "mtllib region.mtl")?;
        fs::write(
            out.join("region.mtl"),
            "newmtl normal\nKd 0.6 0.7 0.8\nnewmtl NO_GRIND\nKd 1 0.3 0.1\nnewmtl rails\nKd 0.1 1 0.2\n",
        )?;
        Some(w)
    } else {
        None
    };
    let (mut triangles, mut no_grind, mut rails, mut length, mut vertex) = (0, 0, 0, 0.0f32, 1);
    let load_time = start.elapsed().as_secs_f64();
    fs::write(
        out.join("selection.txt"),
        region
            .iter()
            .map(|c| format!("{} {}\n", c.0, c.1))
            .collect::<String>(),
    )?;
    fs::write(out.join("stations.txt"), stations.join("\n"))?;
    let mut validation = Validation::default();
    let ramps = std::sync::atomic::AtomicUsize::new(0);
    let step_count = std::sync::atomic::AtomicUsize::new(0);
    let build = |c: tiles::Cell, lookups: &mut Lookups| -> Result<(Tile, Validation), Box<dyn Error + Send + Sync>> {
        let mut validation = Validation::default();
        let at = Vec3::new((c.0 as f32 + 0.5) * size, (c.1 as f32 + 0.5) * size, 0.);
        let mut data = spool.around(at, margin, size)?;
        // Face origins by vertex key, before the cleanup reorders and drops.
        let input_origin: std::collections::HashMap<u64, u32> = data.tris.iter().zip(&data.origins)
            .map(|(t, &o)| (svwc::surfaces::key(&t.v), o)).collect();
        let mut census = SkateVRuntime::live::Census::default();
        // Both stair variants: the game's settings pick (SkateVRuntime::live::shown).
        let cleaned = SkateVRuntime::live::clean_static(
            &mut data.tris,
            &data.stairs,
            &tubes,
            data.spare,
            &mut census,
            true,
        );
        ramps.fetch_add(census.stair_ramps, std::sync::atomic::Ordering::Relaxed);
        step_count.fetch_add(census.steps, std::sync::atomic::Ordering::Relaxed);
        let is_step = |t: &svwc::Tri| cleaned.tag_of(t) & SkateVRuntime::live::STEP_TAG != 0;
        let surface_tags = lookups.recovered.around([at.x, at.y], margin)?;
        let local_tris: Vec<_> = data
            .tris
            .iter()
            .map(|t| t.v.map(Vec3::from_array))
            .collect();
        let input = data.tris.iter().enumerate().map(|(i, t)| {
            let tag = SkateVRuntime::surfaces::tag(
                t.material,
                surface_tags.get(&cleaned.surface_key(t)),
            );
            let _ = i;
            (t.v, tag | cleaned.tag_of(t))
        });
        let input: Vec<_> = input.collect();
        // SKATEV_BAKE_DUMP="x y r file": the cleaned input of the tile holding
        // x y within r, as the rail finder gets it (one triangle per line:
        // nine coordinates and the tag), for world.rs rail_diagnosis_at_a_point.
        if let Ok(spec) = std::env::var("SKATEV_BAKE_DUMP") {
            let p: Vec<&str> = spec.split_whitespace().collect();
            if let [x, y, r, file] = p[..] {
                let (x, y, r): (f32, f32, f32) = (x.parse().unwrap_or(0.), y.parse().unwrap_or(0.), r.parse().unwrap_or(0.));
                if tiles::cell([x, y, 0.], size) == c {
                    let text: String = input.iter().filter(|(t, _)| t.iter().any(|q| (q[0] - x).abs() < r && (q[1] - y).abs() < r))
                        .map(|(t, tag)| format!("{} {tag}\n", t.iter().flatten().map(|v| v.to_string()).collect::<Vec<_>>().join(" ")))
                        .collect();
                    let _ = fs::write(file, text);
                }
            }
        }
        // Park edges: the visible model's chamfered and rounded edges hide
        // from the rail finder (2026-10-26 park bake: 264 of 1,010 m of
        // lines lost, ledges at the bowls). Lines found on GTA's own park
        // collision come back where the visible surface really folds along
        // them (design 2026-10-26-one-source-collision, park edges).
        let gta_park: Vec<([[f32; 3]; 3], u32)> = if cleaned.alternatives.iter().any(|(_, x)| x & SkateVRuntime::live::PARKGTA_TAG != 0) {
            input.iter().filter(|(_, tag)| tag & SkateVRuntime::live::PARKMESH_TAG == 0).cloned()
                .chain(cleaned.alternatives.iter().filter(|(_, x)| x & SkateVRuntime::live::PARKGTA_TAG != 0)
                    .map(|(t, x)| (t.v, SkateVRuntime::surfaces::tag(t.material, surface_tags.get(&svwc::surfaces::key(&t.v))) | (x & !SkateVRuntime::live::PARKGTA_TAG))))
                .collect()
        } else { Vec::new() };
        let mut area = world::build_area_tagged(at, input.into_iter());
        if !gta_park.is_empty() {
            let gta_area = world::build_area_tagged(at, gta_park.into_iter());
            let added = world::park_edge_lines(&mut area, &gta_area.rails);
            validation.park_edge_lines += added;
        }
        let mut tile = Tile {
            cell: c,
            size,
            source: source.clone(),
            triangles: Vec::new(),
            rails: Vec::new(), noses: Vec::new(), origins: Vec::new(), origin_names: Vec::new()
        };
        let mut names = Origins::default();
        let mut origin_of = |v: &[[f32; 3]; 3], tag: u32| -> u16 {
            let key = svwc::surfaces::key(v);
            let file = |k: u64| input_origin.get(&k).map(|&o| spool.names[o as usize].as_str());
            let name = if tag & SkateVRuntime::live::PARKMESH_TAG != 0 {
                if tag & SkateVRuntime::live::PARKCUT_TAG != 0 {
                    format!("cut:{}", cleaned.surface_from.get(&key).and_then(|&k| file(k)).unwrap_or("?"))
                } else {
                    format!("visible:{}", SkateVRuntime::parks::park_at(v[0][0], v[0][1]).unwrap_or("park"))
                }
            } else {
                file(key).map_or_else(|| "rebuilt".to_string(), str::to_string)
            };
            names.index(name)
        };
        for (t, tag) in area.triangles.into_iter().zip(area.tags) {
            let gta = t.map(|p| coords::from_skate(Vec3::from_array(p)).to_array());
            let origin = origin_of(&gta, tag);
            for t in tiles::clip_triangle(gta, c, size) {
                tile.triangles.push((t, tag));
                tile.origins.push(origin);
            }
        }
        // The other setting of each stair switch, kept beside the world
        // (not part of the rule processing above).
        for (t, extra) in &cleaned.alternatives {
            let tag = SkateVRuntime::surfaces::tag(t.material, surface_tags.get(&svwc::surfaces::key(&t.v))) | extra;
            let origin = origin_of(&t.v, tag);
            for t in tiles::clip_triangle(t.v, c, size) {
                tile.triangles.push((t, tag));
                tile.origins.push(origin);
            }
        }
        tile.origin_names = names.names;
        // Step noses of the alternative step variant (unflagged flights rebuilt
        // as steps), for StepRails: the same rules on that flight alone, where
        // only the noses have a drop beneath (Codex review 2026-10-24).
        let step_variant = SkateVRuntime::live::STEP_TAG | SkateVRuntime::live::USTEP_TAG;
        let alt_steps: Vec<_> = cleaned
            .alternatives
            .iter()
            .filter(|(_, extra)| extra & step_variant != 0)
            .map(|(t, extra)| (t.v, *extra))
            .collect();
        let mut alt_noses = Vec::new();
        if !alt_steps.is_empty() {
            let alt = world::build_area_tagged(at, alt_steps.into_iter());
            alt_noses.extend(alt.rails);
            alt_noses.extend(alt.noses);
        }
        let mut noses = area.noses;
        noses.extend(alt_noses);
        for (lines, into) in [(area.rails, &mut tile.rails), (noses, &mut tile.noses)] {
            for r in lines {
                let gta: Vec<_> = r
                    .into_iter()
                    .map(|p| coords::from_skate(Vec3::from_array(p)).to_array())
                    .collect();
                into.extend(
                    tiles::split_rail(&gta, size)
                        .into_iter()
                        .filter(|(owner, _)| *owner == c)
                        .map(|(_, r)| r),
                );
            }
        }
        for i in (0..data.tris.len()).filter(|i| {
            is_step(&data.tris[*i]) && !tiles::clip_triangle(data.tris[*i].v, c, size).is_empty()
        }) {
            let v = data.tris[i].v.map(Vec3::from_array);
            let normal = (v[1] - v[0]).cross(v[2] - v[0]).normalize_or_zero();
            if normal.z.abs() > 0.99 {
                validation.treads += 1;
            }
            if normal.z.abs() < 0.01 {
                validation.risers += 1;
            }
            if validation.step_examples.len() < 3 {
                validation.step_examples.push(v[0]);
            }
        }
        let temp = tiles::path(&out, c).with_extension("tmp");
        let mut writer = BufWriter::new(fs::File::create(&temp)?);
        tile.write(&mut writer)?;
        writer.flush()?;
        drop(writer);
        let saved = Tile::read(&mut std::io::BufReader::new(fs::File::open(&temp)?))?;
        if saved != tile {
            return Err(format!("tile {c:?}: read-back mismatch").into());
        }
        fs::rename(&temp, tiles::path(&out, c))?;
        validation.check(&saved, at, &local_tris);
        Ok((tile, validation))
    };
    let region_list: Vec<_> = region.iter().copied().collect();
    // Each thread holds a tile's 128 m context: on a 16 GB PC 12 threads ran
    // out of memory and paged (15 % CPU, 2026-10-24). SKATEV_BAKE_THREADS
    // overrides; tiles do not depend on the thread count.
    let num_workers = std::env::var("SKATEV_BAKE_THREADS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0)
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()).min(12));
    let mut resumed = 0;
    let mut tasks = Vec::new();
    for c in region_list {
        if live_cells.contains(&c) {
            // This exact numeric cell path is inside the caller's output
            // directory. An old resumed tile must not override live state.
            match fs::remove_file(tiles::path(&out, c)) {
                Ok(()) => {},
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                Err(e) => return Err(e.into()),
            }
            continue;
        }
        let existing = fs::File::open(tiles::path(&out, c))
            .ok()
            .and_then(|f| Tile::read(std::io::BufReader::new(f)).ok())
            .filter(|t| t.cell == c && t.size == size && t.source == source);
        if let Some(tile) = existing {
            resumed += 1;
            account(&tile, &mut obj, &mut vertex, &mut no_grind, &mut length)?;
            triangles += tile.triangles.len();
            rails += tile.rails.len();
        } else {
            tasks.push(c);
        }
    }

    if !tasks.is_empty() {
        let task_idx = std::sync::atomic::AtomicUsize::new(0);
        let (tx, rx) = std::sync::mpsc::channel();
        let build_ref = &build;
        let tasks_ref = &tasks;

        std::thread::scope(|s| {
            for _ in 0..num_workers {
                let tx = tx.clone();
                let task_idx = &task_idx;
                s.spawn(move || {
                    let mut lookups = open_lookups();
                    loop {
                        let idx = task_idx.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if idx >= tasks_ref.len() {
                            break;
                        }
                        let c = tasks_ref[idx];
                        let res = lookups.as_mut().map_err(|e| e.to_string().into()).and_then(|l| build_ref(c, l));
                        if tx.send((c, res)).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(tx);

            for (c, res) in rx {
                let (tile, local_validation) = match res {
                    Ok(pair) => pair,
                    Err(e) => return Err(e.to_string().into()),
                };
                validation.merge(local_validation);
                account(&tile, &mut obj, &mut vertex, &mut no_grind, &mut length)?;
                triangles += tile.triangles.len();
                rails += tile.rails.len();
                eprintln!(
                    "tile {c:?}: {} triangles, {} rail segments",
                    tile.triangles.len(),
                    tile.rails.len()
                );
            }
            Ok::<(), Box<dyn Error>>(())
        })?;
    }
    let workers = num_workers;
    if let Some(o) = obj.as_mut() { o.flush()?; }
    fs::write(out.join("tile-size.txt"), format!("{size}\n"))?;
    fs::write(out.join("rules.txt"), tiles::rules_hash())?;
    let ramps = ramps.load(std::sync::atomic::Ordering::Relaxed);
    let step_count = step_count.load(std::sync::atomic::Ordering::Relaxed);
    let report = format!(
        "Selection: {selection}; tile size: {size} m\nCoverage: selected complete grid cells\nTiles: {}\nTriangles: {triangles}\nNO_GRIND triangles: {no_grind}\nRails (split segments): {rails}\nTotal rail length: {length:.3} m\nMLO bounds placed: {instances}\nUnsupported PSIN placement maps: {}\nTube primitives: {}\nStair ramps replaced: {ramps}; step triangles: {}\nTime: {:.3} s\n",
        region.len(),
        skipped.len(),
        tubes.len(),
        step_count,
        start.elapsed().as_secs_f64()
    );
    let report = format!(
        "{report}Authored drawable prop placements (static only): {} exterior + {} interior; invariant source triangles: {prop_triangles}; object-pool placements left live: {}; dynamic doors excluded: {}\nShared input loading and cleanup: {load_time:.3} s\nTile workers: {workers}\nPlaced metro/subway bounds: {}\n{}",
        prop_stats.placed, prop_stats.mlo_furniture, prop_stats.live_owned, prop_stats.dynamic_doors,
        stations.len(),
        validation.report()
    );
    let tile_bytes: u64 = region
        .iter()
        .map(|&c| fs::metadata(tiles::path(&out, c)).map_or(0, |m| m.len()))
        .sum();
    let report = format!(
        "{report}State-dependent cells (live only): {} worldwide, {} selected\nResumed tiles: {resumed}\nTile bytes: {tile_bytes}\nPeak process working set: {} bytes\nRaw collision staged on disk; one resource / tile context in RAM\n",
        live_cells.len(), region.intersection(&live_cells).count(),
        peak_memory()
    );
    fs::write(out.join("report.txt"), &report)?;
    print!("{report}");
    Ok(())
}

/// Counts a tile into the report and, when asked, appends it to the OBJ.
fn account(tile: &Tile, obj: &mut Option<BufWriter<fs::File>>, vertex: &mut usize, no_grind: &mut usize, length: &mut f32) -> std::io::Result<()> {
    for (t, tag) in &tile.triangles {
        let ng = *tag & skate_no_grind() != 0;
        *no_grind += usize::from(ng);
        if let Some(obj) = obj.as_mut() {
            writeln!(obj, "usemtl {}", if ng { "NO_GRIND" } else { "normal" })?;
            for p in t {
                writeln!(obj, "v {} {} {}", p[0], p[1], p[2])?;
            }
            writeln!(obj, "f {} {} {}", *vertex, *vertex + 1, *vertex + 2)?;
            *vertex += 3;
        }
    }
    if let Some(obj) = obj.as_mut() {
        writeln!(obj, "usemtl rails")?;
    }
    for r in &tile.rails {
        if let Some(obj) = obj.as_mut() {
            for p in r {
                writeln!(obj, "v {} {} {}", p[0], p[1], p[2])?;
            }
            write!(obj, "l")?;
            for i in 0..r.len() {
                write!(obj, " {}", *vertex + i)?;
            }
            writeln!(obj)?;
            *vertex += r.len();
        }
        *length += r.windows(2).map(|w| Vec3::from_array(w[0]).distance(Vec3::from_array(w[1]))).sum::<f32>();
    }
    Ok(())
}

fn skate_no_grind() -> u32 {
    SkateVRuntime::world::NO_GRIND_TILE_TAG
}

fn array(v: rage_formats::math::Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn support_cells(region: &BTreeSet<tiles::Cell>, size: f32, margin: f32) -> BTreeSet<tiles::Cell> {
    region
        .iter()
        .flat_map(|&(x, y)| {
            tiles::cells(
                [(x as f32 + 0.5) * size, (y as f32 + 0.5) * size],
                margin,
                size,
            )
        })
        .collect()
}
fn intersects_support(t: &svwc::Tri, support: &BTreeSet<tiles::Cell>, size: f32) -> bool {
    let v = t.v.map(Vec3::from_array);
    let lo = v[0].min(v[1]).min(v[2]);
    let hi = v[0].max(v[1]).max(v[2]);
    let a = tiles::cell(lo.to_array(), size);
    let b = tiles::cell(hi.to_array(), size);
    (a.0..=b.0).any(|x| (a.1..=b.1).any(|y| support.contains(&(x, y))))
}

#[derive(Default)]
struct Validation {
    track_samples: usize,
    supported: usize,
    missing: usize,
    slot_candidates: usize,
    platforms: usize,
    handrails: usize,
    treads: usize,
    risers: usize,
    track_examples: Vec<Vec3>,
    slot_examples: Vec<Vec3>,
    missing_examples: Vec<Vec3>,
    platform_examples: Vec<Vec3>,
    handrail_examples: Vec<Vec3>,
    step_examples: Vec<Vec3>,
    /// Park edge lines taken from GTA's own park collision (world::park_edge_lines).
    park_edge_lines: usize,
}
fn segments(lines: &[Vec<Vec3>]) -> (Vec<(Vec3, Vec3)>, SkateVRuntime::grid::XyGrid) {
    let edges: Vec<_> = lines
        .iter()
        .flat_map(|r| r.windows(2).map(|w| (w[0], w[1])))
        .collect();
    let mut grid = SkateVRuntime::grid::XyGrid::new(1.);
    for (i, &(a, b)) in edges.iter().enumerate() {
        grid.insert(
            i as u32,
            a.min(b) - Vec3::splat(0.5),
            a.max(b) + Vec3::splat(0.5),
        );
    }
    (edges, grid)
}
fn nearest(p: Vec3, edges: &[(Vec3, Vec3)], grid: &SkateVRuntime::grid::XyGrid) -> Option<Vec3> {
    grid.at(p.x, p.y)
        .iter()
        .map(|&i| {
            let (a, b) = edges[i as usize];
            let d = b - a;
            a + d
                * ((p - a).truncate().dot(d.truncate()) / d.truncate().length_squared().max(1e-12))
                    .clamp(0., 1.)
        })
        .min_by(|a, b| a.distance_squared(p).total_cmp(&b.distance_squared(p)))
}
fn example(examples: &mut Vec<Vec3>, p: Vec3) {
    if examples.len() < 5 && examples.iter().all(|q| q.distance(p) > 20.) {
        examples.push(p);
    }
}
impl Validation {
    fn merge(&mut self, other: Self) {
        self.track_samples += other.track_samples;
        self.supported += other.supported;
        self.missing += other.missing;
        self.slot_candidates += other.slot_candidates;
        self.platforms += other.platforms;
        self.handrails += other.handrails;
        self.treads += other.treads;
        self.risers += other.risers;
        self.park_edge_lines += other.park_edge_lines;
        for p in other.track_examples {
            example(&mut self.track_examples, p);
        }
        for p in other.slot_examples {
            example(&mut self.slot_examples, p);
        }
        for p in other.missing_examples {
            example(&mut self.missing_examples, p);
        }
        for p in other.platform_examples {
            example(&mut self.platform_examples, p);
        }
        for p in other.handrail_examples {
            example(&mut self.handrail_examples, p);
        }
        for p in other.step_examples {
            example(&mut self.step_examples, p);
        }
    }
    fn check(&mut self, tile: &Tile, at: Vec3, collision: &[[Vec3; 3]]) {
        let raw = SkateVRuntime::tracks::rails_in([at.x, at.y], world::RADIUS + 3.);
        if raw.is_empty() {
            return;
        }
        let snapped = world::onto_rail_boxes(raw, collision);
        let (track_edges, track_grid) = segments(&snapped);
        let final_lines: Vec<Vec<Vec3>> = tile
            .rails
            .iter()
            .map(|r| r.iter().copied().map(Vec3::from_array).collect())
            .collect();
        let (final_edges, final_grid) = segments(&final_lines);
        let collision_grid = SkateVRuntime::grid::XyGrid::triangles(1., collision);
        for line in &snapped {
            for &p in line {
                if tiles::cell(p.to_array(), tile.size) != tile.cell {
                    continue;
                }
                self.track_samples += 1;
                let supported = collision_grid
                    .at(p.x, p.y)
                    .iter()
                    .filter_map(|&i| SkateVRuntime::grid::height(&collision[i as usize], p.x, p.y))
                    .any(|z| (p.z - z - 0.01).abs() < 0.025);
                if supported {
                    self.supported += 1;
                    example(&mut self.track_examples, p);
                }
                if !nearest(p, &final_edges, &final_grid).is_some_and(|q| q.distance(p) < 0.06) {
                    self.missing += 1;
                    example(&mut self.missing_examples, p);
                }
            }
        }
        for &(a, b) in &final_edges {
            let p = (a + b) * 0.5;
            if let Some(q) = nearest(p, &track_edges, &track_grid) {
                let xy = q.truncate().distance(p.truncate());
                if xy <= 0.3 && (p.z - q.z).abs() <= 0.4 && p.distance(q) > 0.06 {
                    self.slot_candidates += 1;
                    example(&mut self.slot_examples, p);
                }
            }
            // Candidate platform edges: long horizontal lines 1..6 m from a
            // snapped steel rail, 0.2..2 m above it. Search neighbouring buckets.
            if a.distance(b) > 1. && (a.z - b.z).abs() < 0.08 {
                let near_platform = (-6..=6).any(|x| {
                    (-6..=6).any(|y| {
                        let probe = p + Vec3::new(x as f32, y as f32, 0.);
                        nearest(probe, &track_edges, &track_grid).is_some_and(|q| {
                            let d = p.truncate().distance(q.truncate());
                            (1. ..=6.).contains(&d) && (0.2..=2.).contains(&(p.z - q.z))
                        })
                    })
                });
                if near_platform {
                    self.platforms += 1;
                    example(&mut self.platform_examples, p);
                }
            }
            let run = (b - a).truncate().length();
            if run > 1.
                && (0.15..=0.9).contains(&((b.z - a.z).abs() / run))
                && !nearest(p, &track_edges, &track_grid).is_some_and(|q| q.distance(p) < 0.5)
            {
                self.handrails += 1;
                example(&mut self.handrail_examples, p);
            }
        }
    }
    fn report(&self) -> String {
        format!(
            "\nValidation (GTA XYZ metres; geometry checks, in-game ride QA still required):\nTrack node rail samples: {}; on collision top + 0.01 m (tolerance 0.025 m): {}; unsupported/default height: {}\nExpected snapped track points absent from final rails within 0.06 m: {}\nExtra slot-line candidate segment midpoints within 0.30 m XY / 0.40 m Z, excluding 0.06 m track match: {}\nPlatform-edge candidates (geometric): {}; sloping handrail candidates (geometric): {}\nRebuilt stair tread/riser source faces touching output tiles: {} / {}\nTrack top examples: {:?}\nMissing track examples: {:?}\nSlot candidates: {:?}\nPlatform-edge examples: {:?}\nSloping handrail examples: {:?}\nRebuilt step examples: {:?}\nCandidate categories require inspection: crossing rails, curvature, unrelated ledges/slopes can be counted. Counts are tile-local samples, not unique physical objects. Unsupported points retain the runtime default; they are not evidence of steel collision.\n",
            self.track_samples,
            self.supported,
            self.track_samples - self.supported,
            self.missing,
            self.slot_candidates,
            self.platforms,
            self.handrails,
            self.treads,
            self.risers,
            self.track_examples,
            self.missing_examples,
            self.slot_examples,
            self.platform_examples,
            self.handrail_examples,
            self.step_examples
        ) + &format!("Park edge lines from GTA's own park collision (visible folds of 30 deg or more): {}
", self.park_edge_lines)
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;

    #[test]
    fn world_inputs_skip_vehicle_detail_local_and_procedural_bounds() {
        for path in [
            "gta-archives/base/x64j/levels/gta5/map.rpf/map.ybn",
        ] { assert!(is_world_bound(Path::new(path)), "{path}"); }
        for path in [
            "gta-archives/layers/01_patch/x64/levels/gta5/map.rpf/hi@map.ybn",
            "gta-archives/base/x64j/levels/gta5/map.rpf/ma@map.ybn",
            "gta-archives/base/x64j/levels/gta5/interiors/house.rpf/house.ybn",
            "gta-archives/base/x64j/levels/gta5/props/prop.rpf/prop.ybn",
            "gta-archives/base/x64j/models/cdimages/vehicles.rpf/car.ybn",
            "gta-archives/base/x64j/levels/gta5/map.rpf/map.ydr",
        ] { assert!(!is_world_bound(Path::new(path)), "{path}"); }
    }

    #[test]
    fn story_fallback_keeps_existing_half_upward_floor_threshold() {
        let floor = svwc::Tri { v:[[0.,0.,0.],[1.,0.,0.],[0.,1.,0.]],material:1 };
        assert!(fallback_floor(&floor));
        let bank = svwc::Tri { v:[[0.,0.,0.],[1.,0.,0.],[0.,1.,1.4]],material:1 };
        assert!(fallback_floor(&bank),"preserve walkable slopes below the rail-finder threshold");
        let wall = svwc::Tri { v:[[0.,0.,0.],[1.,0.,0.],[0.,0.,1.]],material:1 };
        assert!(!fallback_floor(&wall));
        assert!(!fallback_floor(&svwc::Tri { v:[floor.v[0],floor.v[2],floor.v[1]],material:1 }));
        assert!(!fallback_floor(&svwc::Tri { v:[[0.;3];3],material:1 }));
    }

    #[test]
    fn coverage_gate_requires_every_invariant_cell_and_omits_stale_live_tiles() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../out/tests/coverage-gate-{}",std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("tile-size.txt"),"64").unwrap();
        fs::write(dir.join("rules.txt"),tiles::rules_hash()).unwrap();
        let live = BTreeSet::from([(1,0)]);
        let source = format!("source fixture\n{}\n{}",tiles::rules_hash(),tiles::live_policy(&live));
        tiles::write_live_cells(&dir,64.,&source,&live).unwrap();
        let tile = Tile { cell:(0,0),size:64.,source,triangles:vec![([[1.,1.,0.],[2.,1.,0.],[1.,2.,0.]],0)],rails:vec![] , noses: Vec::new(), origins: Vec::new(), origin_names: Vec::new()};
        let save = || tile.write(fs::File::create(tiles::path(&dir,(0,0))).unwrap()).unwrap();
        save();
        assert_eq!(verify_coverage(&dir,[0.,127.,0.,63.]).unwrap(),(1,1,1,0));
        fs::remove_file(tiles::path(&dir,(0,0))).unwrap();
        assert!(verify_coverage(&dir,[0.,127.,0.,63.]).is_err(),"unintended missing cells cannot be called complete");
        save();
        fs::write(tiles::path(&dir,(1,0)),b"stale").unwrap();
        assert!(verify_coverage(&dir,[0.,127.,0.,63.]).is_err(),"old live-cell files must be absent from a newly generated payload");
        fs::remove_file(tiles::path(&dir,(1,0))).unwrap();
        fs::write(dir.join("live-cells.txt"),b"truncated").unwrap();
        assert!(verify_coverage(&dir,[0.,127.,0.,63.]).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn story_bounds_override_only_through_enabled_layers() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../out/tests/story-inputs-{}", std::process::id()));
        let write = |relative: &str, data: &str| {
            let path = dir.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        };
        write("base/x64a/levels/gta5/a.rpf/a.ybn", "base");
        write("base/x64a/levels/gta5/props/a.rpf/local.ybn", "local");
        write("layers.txt", "00_update\n01_patch\n");
        write("layers/00_update/x64/levels/gta5/a.rpf/a.ybn", "update");
        write("layers/00_update/x64/levels/gta5/a.rpf/hi@a.ybn", "detail");
        write("layers/01_patch/content.xml", "<contentChangeSets><Item><changeSetName>LIVE</changeSetName><filesToEnable><Item>dlc:/x64/levels/gta5/live.rpf</Item></filesToEnable></Item></contentChangeSets>");
        write("layers/01_patch/setup2.xml", "<ContentChangeSets><Item>LIVE</Item></ContentChangeSets>");
        write("layers/01_patch/x64/levels/gta5/live.rpf/a.ybn", "live patch");
        write("layers/01_patch/x64/levels/gta5/unused.rpf/a.ybn", "unused patch");
        let selected = story_world_bounds(&dir);
        assert_eq!(selected.len(), 1);
        let normal = selected.iter().find(|p| p.file_name().unwrap() == "a.ybn").unwrap();
        assert_eq!(fs::read_to_string(normal).unwrap(), "live patch");
        assert!(!selected.iter().any(|p| p.file_name().unwrap() == "hi@a.ybn"), "no vehicle-only hi@ copy");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "requires local GTA, keys and old flat collision inputs"]
    fn audit_story_collision_inputs() {
        let gta = PathBuf::from(std::env::var_os("SKATEV_TEST_GTA").expect("SKATEV_TEST_GTA"));
        let keys = PathBuf::from(std::env::var_os("SKATEV_TEST_KEYS").expect("SKATEV_TEST_KEYS"));
        let flat = PathBuf::from(std::env::var_os("SKATEV_TEST_COLLISION").expect("SKATEV_TEST_COLLISION"));
        vfs::mount_world(&gta, gta_archives::keys(&keys).unwrap()).unwrap();
        let meta = Path::new(vfs::ROOT);
        interiors::validate_manifests(meta).unwrap();
        let local = interiors::interior_bound_hashes(meta);
        let selected = story_world_bounds(meta);
        let mut old = Vec::new();
        crate::collect_ybn(&flat, true, &mut old).unwrap();
        let old: BTreeMap<_, _> = old.into_iter().map(|p| (p.file_name().unwrap().to_string_lossy().to_ascii_lowercase(), p)).collect();
        let mut matched = 0usize;
        let mut changed = 0usize;
        let mut missing_flat = 0usize;
        let mut selected_names = BTreeSet::new();
        for p in &selected {
            if local.contains(&interiors::stem_hash(p)) { continue; }
            let name = p.file_name().unwrap().to_string_lossy().to_ascii_lowercase();
            selected_names.insert(name.clone());
            let current = vfs::read(p).unwrap();
            if let Some(previous) = old.get(&name) {
                if hash(&current) == hash(&fs::read(previous).unwrap()) { matched += 1; }
                else { changed += 1; println!("STORY_OVERRIDE {name} source={}", p.display()); }
            } else { missing_flat += 1; println!("STORY_MISSING_FROM_FLAT {name}"); }
        }
        let mut extra = 0usize;
        for name in old.keys() {
            if !selected_names.contains(name) { extra += 1; println!("FLAT_NOT_WORLD {name}"); }
        }
        println!("STORY_INPUTS selected_world={} raw_selected={} flat={} matching_bytes={matched} changed_bytes={changed} missing_from_flat={missing_flat} extra_flat={extra}", selected_names.len(), selected.len(), old.len());
    }
    #[test]
    fn authored_prop_bound_is_placed_and_clipped_like_map_collision() {
        // A drawable whose embedded bound (+0xC8) is one authored box of
        // half-size (1, 2, 3), placed at (64, 1, 0) across a tile boundary.
        let mut b = vec![0u8; 0x800];
        b[0xC8..0xD0].copy_from_slice(&(SYSTEM_BASE + 0x600).to_le_bytes());
        b[0x610] = 3;
        for (at, v) in [(0x620, [1f32, 2., 3.]), (0x630, [-1., -2., -3.])] {
            for (i, f) in v.into_iter().enumerate() { b[at + i * 4..at + i * 4 + 4].copy_from_slice(&f.to_le_bytes()); }
        }
        let bytes = rage_formats::resource::build_rsc7(165, &b, &[]);
        let place = Place { rows: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [64., 1., 0.]], identity: false };
        let mut data = Collision::default();
        decode_drawable(&bytes, &place, (62., 0., 64., 3.), &mut data).unwrap();
        assert!(!data.tris.is_empty(), "faces crossing the clip remain");
        assert_eq!(data.stairs.len(), data.tris.len());
        assert!(data.tris.iter().all(|t| t.v.iter().all(|v| (62.9..=65.1).contains(&v[0]))), "placed at x 63..65");
        let mut far = Collision::default();
        decode_drawable(&bytes, &place, (200., 200., 264., 264.), &mut far).unwrap();
        assert!(far.tris.is_empty(), "faces outside the clip are not emitted");
    }
    #[test]
    fn stateful_mask_covers_crossing_floors_and_vertical_walls() {
        let data = Collision {
            tris: vec![svwc::Tri { v: [[63., 1., 0.], [65., 1., 0.], [63., 2., 0.]], material: 1 }],
            stairs: vec![false],
            spare: vec![svwc::Tri { v: [[0., 1., 0.], [0., 2., 0.], [0., 1., 2.]], material: 1 }],
            origins: Vec::new(),
        };
        let mut cells = BTreeSet::new();
        mark_live_cells(&data, 64., &mut cells);
        assert_eq!(cells, BTreeSet::from([(0, 0), (1, 0)]));
        assert!(touches_live(&data.tris[0], 64., &BTreeSet::from([(1, 0)])));
        assert!(!touches_live(&data.tris[0], 64., &BTreeSet::from([(2, 0)])));
        assert!(touches_live(&data.spare[0], 64., &BTreeSet::from([(0, 0)])));
    }
    #[test]
    #[ignore = "requires local GTA/keys/collision inventory environment variables"]
    fn audit_world_bound_provenance() {
        let gta = PathBuf::from(std::env::var_os("SKATEV_TEST_GTA").expect("SKATEV_TEST_GTA"));
        let keys = PathBuf::from(std::env::var_os("SKATEV_TEST_KEYS").expect("SKATEV_TEST_KEYS"));
        let dir = PathBuf::from(std::env::var_os("SKATEV_TEST_COLLISION").expect("SKATEV_TEST_COLLISION"));
        vfs::mount_world(&gta, gta_archives::keys(&keys).unwrap()).unwrap();
        let meta = Path::new(vfs::ROOT);
        let manifest_count = interiors::validate_manifests(meta).expect("strict manifest preflight");
        println!("MANIFESTS validated={manifest_count} failures=0");
        let local = interiors::interior_bound_hashes(meta);
        let toggled = interiors::state_dependent_maps(meta);
        let mut files = Vec::new();
        crate::collect_ybn(&dir, true, &mut files).unwrap();
        files.sort();
        let mut toggled_world = Vec::new();
        let mut local_files = Vec::new();
        for p in &files {
            let h = interiors::stem_hash(p);
            if local.contains(&h) { local_files.push(p.file_name().unwrap().to_string_lossy().to_string()); }
            else if toggled.contains(&h) { toggled_world.push(p.file_name().unwrap().to_string_lossy().to_string()); }
        }
        println!("PROVENANCE files={} local_hashes={} toggled_hashes={} local_files={} toggled_world_files={}", files.len(), local.len(), toggled.len(), local_files.len(), toggled_world.len());
        let mut touched = BTreeSet::new();
        let mut faces = 0usize;
        let mut failures = 0usize;
        for p in &files {
            if local.contains(&interiors::stem_hash(p)) || !toggled.contains(&interiors::stem_hash(p)) { continue; }
            let mut data = Collision::default();
            if decode(&fs::read(p).unwrap(), &identity(), (-20000., -20000., 20000., 20000.), &mut data).is_err() {
                failures += 1; continue;
            }
            let mut cells = BTreeSet::new();
            for t in data.tris.into_iter().chain(data.spare) {
                faces += 1;
                let v = t.v.map(Vec3::from_array);
                let lo = tiles::cell(v[0].min(v[1]).min(v[2]).to_array(), 64.);
                let hi = tiles::cell(v[0].max(v[1]).max(v[2]).to_array(), 64.);
                for x in lo.0..=hi.0 { for y in lo.1..=hi.1 {
                    if !tiles::clip_triangle(t.v, (x,y), 64.).is_empty() { cells.insert((x,y)); }
                }}
            }
            println!("STATEFUL_CELLS {} count={} cells={cells:?}", p.file_name().unwrap().to_string_lossy(), cells.len());
            touched.extend(cells);
        }
        let city = touched.iter().filter(|&&(x,y)| (-35..=23).contains(&x) && (-55..=15).contains(&y)).count();
        println!("STATEFUL_COVERAGE decoded_faces={faces} failures={failures} world_cells={} city_cells={city} world={touched:?}", touched.len());
        for n in toggled_world { println!("TOGGLED_WORLD_BOUND {n}"); }
        for n in local_files { println!("LOCAL_BOUND {n}"); }
    }
    #[test]
    #[ignore = "requires local GTA collision inventory via SKATEV_TEST_COLLISION"]
    fn audit_filtered_nested_composites() {
        fn visit(r: &ResReader<'_>, va: u64, flags: (u32, u32), depth: usize,
                 groups: &mut usize, leaked: &mut usize, extent: &mut Option<(Vec3, Vec3)>) {
            if depth > 8 { return; }
            let Some(h) = r.resolve(va, 0xb0) else { return; };
            if h[0x10] != 10 { return; }
            if !bounds::skater_collides(flags.0, flags.1) {
                *groups += 1;
                let mut data = Collision::default();
                if walk(r, va, &mut Vec::new(), (0, 0), &identity(),
                    (-20000., -20000., 20000., 20000.), &mut data).is_ok() {
                    *leaked += data.tris.len();
                    for p in data.tris.into_iter().flat_map(|t| t.v) {
                        let p = Vec3::from_array(p);
                        *extent = Some(match *extent { Some((a,b)) => (a.min(p), b.max(p)), None => (p,p) });
                    }
                }
                return;
            }
            let n = u16_le(h, 0xa0) as usize;
            let Some(list) = r.read_u64_list(u64_le(h, 0x70), n) else { return; };
            let child_flags = r.resolve(u64_le(h, 0x90), n * 8);
            for (i, p) in list.into_iter().enumerate().filter(|(_, p)| *p != 0) {
                let f = child_flags.map(|b| (u32_le(b, i*8), u32_le(b, i*8+4))).unwrap_or((0,0));
                visit(r, p, f, depth + 1, groups, leaked, extent);
            }
        }
        let dir = PathBuf::from(std::env::var_os("SKATEV_TEST_COLLISION").expect("SKATEV_TEST_COLLISION"));
        let mut files = Vec::new();
        crate::collect_ybn(&dir, true, &mut files).unwrap();
        files.sort();
        let (mut affected, mut groups, mut leaked, mut failures) = (0usize, 0usize, 0usize, 0usize);
        for path in &files {
            let bytes = fs::read(path).unwrap();
            let Ok((system, graphics)) = prepare_rsc7(&bytes) else { failures += 1; continue; };
            let (before, before_groups) = (leaked, groups);
            let mut extent = None;
            visit(&ResReader { system: &system, graphics: &graphics }, SYSTEM_BASE, (0,0), 0, &mut groups, &mut leaked, &mut extent);
            if groups != before_groups {
                affected += 1;
                println!("FILTERED {} groups={} solid_descendants={} local_extent={extent:?}", path.file_name().unwrap().to_string_lossy(), groups - before_groups, leaked - before);
            }
        }
        println!("AUDIT files={} decode_failures={failures} affected_files={affected} filtered_groups={groups} solid_descendants={leaked}", files.len());
    }
    #[test]
    fn filtered_composite_does_not_promote_unflagged_descendants() {
        // Root -> flagged nested composite -> unflagged authored box.
        // Resource addresses use the system block; absent frames are identity.
        let mut system = vec![0u8; 0x340];
        for (at, list, flags) in [(0usize, 0x300usize, 0x310usize), (0x100, 0x320, 0x330)] {
            system[at + 0x10] = 10;
            system[at + 0x70..at + 0x78].copy_from_slice(&(SYSTEM_BASE + list as u64).to_le_bytes());
            system[at + 0x90..at + 0x98].copy_from_slice(&(SYSTEM_BASE + flags as u64).to_le_bytes());
            system[at + 0xa0..at + 0xa2].copy_from_slice(&1u16.to_le_bytes());
        }
        system[0x300..0x308].copy_from_slice(&(SYSTEM_BASE + 0x100).to_le_bytes());
        system[0x320..0x328].copy_from_slice(&(SYSTEM_BASE + 0x200).to_le_bytes());
        system[0x210] = 3;
        for k in 0..3 {
            system[0x220 + k * 4..0x224 + k * 4].copy_from_slice(&1f32.to_le_bytes());
            system[0x230 + k * 4..0x234 + k * 4].copy_from_slice(&(-1f32).to_le_bytes());
        }
        for (ty, include, expected) in [
            (0u32, 0u32, 12usize),
            (0x3e, bounds::PED, 12),
            (2, bounds::TEST_WEAPON | bounds::TEST_CAMERA, 0),
            (0x20, 0x80, 0),
            (bounds::FOLIAGE, bounds::PED, 0),
        ] {
            system[0x310..0x314].copy_from_slice(&ty.to_le_bytes());
            system[0x314..0x318].copy_from_slice(&include.to_le_bytes());
            let reader = ResReader { system: &system, graphics: &[] };
            let mut out = Collision::default();
            walk(&reader, SYSTEM_BASE, &mut Vec::new(), (0, 0), &identity(),
                (-100., -100., 100., 100.), &mut out).unwrap();
            assert_eq!(out.tris.len(), expected, "parent flags {ty:x}/{include:x}");
            assert!(out.spare.is_empty());
        }
    }
    #[test]
    fn tile_list_is_sorted_deduplicated_and_strict() {
        assert_eq!(
            parse_cells("-2 3\n1 -1 # comment\n-2 3\n")
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>(),
            vec![(-2, 3), (1, -1)]
        );
        for s in ["", "1", "1 2 3", "a 2", "2147483648 1", "-2147483648 0"] {
            assert!(parse_cells(s).is_err());
        }
    }
    #[test]
    fn sparse_support_preserves_stair_alignment_and_crossing_faces() {
        let tri = |x: f32| svwc::Tri {
            v: [[x, 1., 0.], [x + 1., 1., 0.], [x, 2., 0.]],
            material: 1,
        };
        let mut data = Collision {
            tris: vec![tri(1.), tri(1000.), tri(-63.)],
            stairs: vec![true, true, false],
            spare: vec![tri(1.), tri(1000.)],
            origins: Vec::new(),
        };
        let support = BTreeSet::from([(0, 0), (-1, 0)]);
        let out = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../out/tests/sparse-{}", std::process::id()));
        fs::create_dir_all(&out).unwrap();
        let mut spool = Spool::new(&out).unwrap();
        spool.push(&mut data, &support, 64., "test.ybn").unwrap();
        let data = spool.around(Vec3::ZERO, 128., 64.).unwrap();
        assert_eq!(data.tris.len(), 2);
        assert_eq!(data.stairs.iter().filter(|&&s| s).count(), 1);
        assert_eq!(data.spare.len(), 1);
        assert!(data.tris.iter().any(|t| t.v[0][0] == -63.));
        drop(spool);
        fs::remove_dir(out).unwrap();
        let crossing = svwc::Tri {
            v: [[-200., 1., 0.], [200., 1., 0.], [200., 2., 0.]],
            material: 1,
        };
        assert!(intersects_support(&crossing, &support, 64.));
        assert!(!intersects_support(&tri(200.), &support, 64.));
    }

    #[test]
    fn circular_margin_selects_intersecting_tiles_with_negative_coordinates() {
        let mut cells = BTreeSet::new();
        near_point(&mut cells, Vec3::new(-1., -1., -10.), 2., 64.);
        assert_eq!(cells, BTreeSet::from([(-1, -1), (-1, 0), (0, -1), (0, 0)]));
        cells.clear();
        near_point(&mut cells, Vec3::new(32., 32., 0.), 32., 64.);
        assert!(cells.contains(&(-1, 0)) && cells.contains(&(1, 0)));
        assert!(!cells.contains(&(1, 1)));
    }
}

#[cfg(windows)]
fn peak_memory() -> usize {
    #[repr(C)]
    struct Counters {
        cb: u32,
        faults: u32,
        peak: usize,
        rest: [usize; 7],
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(p: *mut std::ffi::c_void, c: *mut Counters, n: u32) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
    }
    let mut c = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        faults: 0,
        peak: 0,
        rest: [0; 7],
    };
    unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut c,
            std::mem::size_of::<Counters>() as u32,
        );
    }
    c.peak
}
#[cfg(not(windows))]
fn peak_memory() -> usize {
    0
}

#[cfg(test)]
mod spool_tests {
    use super::*;
    #[test]
    fn disk_grid_deduplicates_border_faces_and_preserves_flags() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../out/tests/spool-{}", std::process::id()));
        fs::create_dir_all(&out).unwrap();
        let mut spool = Spool::new(&out).unwrap();
        let t = svwc::Tri {
            v: [[-1., 0., 2.], [1., 0., 2.], [-1., 1., 2.]],
            material: 55,
        };
        let mut data = Collision {
            tris: vec![t.clone()],
            stairs: vec![true],
            spare: vec![t.clone()],
            origins: Vec::new(),
        };
        spool.push(&mut data, &BTreeSet::new(), 64., "test.ybn").unwrap();
        assert!(data.tris.is_empty() && data.spare.is_empty());
        let loaded = spool.around(Vec3::ZERO, 128., 64.).unwrap();
        assert_eq!(loaded.tris.len(), 1);
        assert_eq!(loaded.spare.len(), 1);
        assert_eq!(loaded.tris[0].v, t.v);
        assert_eq!(loaded.tris[0].material, 55);
        assert_eq!(loaded.stairs, [true]);
        assert!(
            spool
                .around(Vec3::new(500., 500., 0.), 10., 64.)
                .unwrap()
                .tris
                .is_empty()
        );
        drop(spool);
        assert!(!out.join("raw-spool").exists());
        fs::remove_dir(out).unwrap();
    }
}
