//! Collision from placed High-LOD railing meshes (SVVB v1).
use bevy_math::Vec3;
use std::{collections::{BTreeMap, BTreeSet, HashMap}, io, path::Path, sync::OnceLock};
use svwc::Tri;

pub const RAILING_MATERIALS: &[u8] = &[64];
pub const CELL: f32 = 32.0;
pub static VISIBLE_RAILINGS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub id: u32,
    pub model: u32,
    pub z: [f32; 2],
    pub mesh: Vec<[[f32; 3]; 3]>,
    pub footprint: Vec<[[f32; 2]; 3]>,
}

impl Object {
    pub fn bounds(&self) -> [f32; 4] {
        let mut b = [f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY];
        for p in self.footprint.iter().flatten() {
            b[0] = b[0].min(p[0] - 0.10); b[1] = b[1].max(p[0] + 0.10);
            b[2] = b[2].min(p[1] - 0.10); b[3] = b[3].max(p[1] + 0.10);
        }
        b
    }

    fn contains(&self, p: Vec3) -> bool {
        p.z >= self.z[0] && p.z <= self.z[1] && self.footprint.iter().any(|t| {
            let t = t.map(|p| Vec3::new(p[0], p[1], 0.));
            distance(p.with_z(0.), &t) <= 0.10 * 0.10 + 1e-8
        })
    }
}

fn cells(b: [f32; 4], size: f32) -> impl Iterator<Item = (i32, i32)> {
    ((b[0] / size).floor() as i32..=(b[1] / size).floor() as i32).flat_map(move |x|
        ((b[2] / size).floor() as i32..=(b[3] / size).floor() as i32).map(move |y| (x, y)))
}

/// The stored footprint is the XY triangles; containment widens their union by 10 cm.
pub fn write(path: &Path, objects: &[Object]) -> io::Result<()> {
    let mut table: BTreeMap<_, Vec<&Object>> = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for o in objects {
        if !valid(o) || !ids.insert(o.id) { return Err(invalid()); }
        for c in cells(o.bounds(), CELL) { table.entry(c).or_default().push(o); }
    }
    let mut head = b"SVVB".to_vec();
    head.extend(1u32.to_le_bytes()); head.extend(CELL.to_le_bytes()); head.extend((table.len() as u32).to_le_bytes());
    let mut body = Vec::new();
    for ((x, y), objects) in &table {
        head.extend(x.to_le_bytes()); head.extend(y.to_le_bytes()); head.extend((objects.len() as u32).to_le_bytes());
        head.extend((16 + table.len() as u64 * 20 + body.len() as u64).to_le_bytes());
        for o in objects {
            body.extend(o.id.to_le_bytes()); body.extend(o.model.to_le_bytes());
            for z in o.z { body.extend(z.to_le_bytes()); }
            body.extend((o.mesh.len() as u32).to_le_bytes());
            for v in o.mesh.iter().flatten().flatten() { body.extend(v.to_le_bytes()); }
            body.extend((o.footprint.len() as u32).to_le_bytes());
            for v in o.footprint.iter().flatten().flatten() { body.extend(v.to_le_bytes()); }
        }
    }
    head.extend(body); std::fs::write(path, head)?;
    write_policy(path)
}

// Use exactly the exporter Story/state receipt used by visible rails.
pub fn write_policy(path: &Path) -> io::Result<()> { crate::visrails::write_policy(path) }
pub fn verified_policy(path: &Path) -> bool { crate::visrails::verified_policy(path) }
fn invalid() -> io::Error { io::Error::new(io::ErrorKind::InvalidData, "malformed SVVB cell") }
fn valid(o: &Object) -> bool {
    !o.mesh.is_empty() && o.mesh.len() <= 3000 && !o.footprint.is_empty() && o.footprint.len() <= 3000
        && o.z.iter().all(|v| v.is_finite()) && o.z[0] <= o.z[1]
        && o.mesh.iter().flatten().flatten().chain(o.footprint.iter().flatten().flatten()).all(|v| v.is_finite())
}

#[derive(Default)]
struct File {
    objects: BTreeMap<u32, Object>,
    table: HashMap<(i32, i32), Vec<u32>>,
}

struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn u32(&mut self) -> io::Result<u32> {
        let b = self.0.get(..4).ok_or_else(invalid)?;
        let v = u32::from_le_bytes(b.try_into().unwrap()); self.0 = &self.0[4..]; Ok(v)
    }
    fn f32(&mut self) -> io::Result<f32> { Ok(f32::from_bits(self.u32()?)) }
    fn object(&mut self) -> io::Result<Object> {
        let (id, model) = (self.u32()?, self.u32()?);
        let z = [self.f32()?, self.f32()?];
        let n = self.u32()? as usize;
        if n > 3000 { return Err(invalid()); }
        let mut mesh = Vec::new();
        for _ in 0..n {
            let mut t = [[0.; 3]; 3];
            for v in t.iter_mut().flatten() { *v = self.f32()?; } mesh.push(t);
        }
        let n = self.u32()? as usize;
        if n > 3000 { return Err(invalid()); }
        let mut footprint = Vec::new();
        for _ in 0..n {
            let mut t = [[0.; 2]; 3];
            for v in t.iter_mut().flatten() { *v = self.f32()?; } footprint.push(t);
        }
        let o = Object { id, model, z, mesh, footprint };
        if !valid(&o) { return Err(invalid()); } Ok(o)
    }
}

fn decode(bytes: &[u8]) -> io::Result<File> {
    if bytes.get(..4) != Some(b"SVVB") { return Err(invalid()); }
    let mut r = Reader(&bytes[4..]);
    if r.u32()? != 1 || r.f32()? != CELL { return Err(invalid()); }
    let n = r.u32()? as usize;
    let head = 16usize.checked_add(n.checked_mul(20).ok_or_else(invalid)?).ok_or_else(invalid)?;
    if head > bytes.len() { return Err(invalid()); }
    let mut rows = Vec::new(); let mut seen = BTreeSet::new();
    for _ in 0..n {
        let c = (r.u32()? as i32, r.u32()? as i32); let count = r.u32()?;
        let lo = r.u32()? as u64; let hi = r.u32()? as u64;
        let off = usize::try_from(lo | hi << 32).map_err(|_| invalid())?;
        if off < head || off > bytes.len() || !seen.insert(c) || count == 0 { return Err(invalid()); }
        rows.push((off, c, count));
    }
    rows.sort_unstable();
    if rows.first().map_or(bytes.len(), |v| v.0) != head { return Err(invalid()); }
    let mut file = File::default();
    for (i, &(off, c, count)) in rows.iter().enumerate() {
        let end = rows.get(i + 1).map_or(bytes.len(), |v| v.0);
        let mut r = Reader(bytes.get(off..end).ok_or_else(invalid)?);
        let mut ids = Vec::new();
        for _ in 0..count {
            let o = r.object()?;
            if ids.contains(&o.id) || !cells(o.bounds(), CELL).any(|v| v == c) { return Err(invalid()); }
            ids.push(o.id);
            if let Some(old) = file.objects.get(&o.id) { if *old != o { return Err(invalid()); } }
            else { file.objects.insert(o.id, o); }
        }
        if !r.0.is_empty() { return Err(invalid()); }
        file.table.insert(c, ids);
    }
    for o in file.objects.values() {
        if !cells(o.bounds(), CELL).all(|c| file.table.get(&c).is_some_and(|ids| ids.contains(&o.id))) { return Err(invalid()); }
    }
    Ok(file)
}

static FILE: OnceLock<Option<File>> = OnceLock::new();
static LIVE: OnceLock<Option<(f32, BTreeSet<(i32, i32)>)>> = OnceLock::new();

pub fn load(path: &Path) -> Result<usize, String> {
    let f = FILE.get_or_init(|| {
        if !verified_policy(path) { return None; }
        std::fs::read(path).ok().and_then(|b| decode(&b).ok())
    });
    f.as_ref().map(|f| f.table.len()).ok_or_else(|| format!("visible railings disabled: unverified or malformed SVVB file {}", path.display()))
}

/// Called once at runtime startup, even with BakedWorld off.
pub fn load_live(dir: &Path) -> Result<(), String> {
    let policy = LIVE.get_or_init(|| {
        let size: f32 = std::fs::read_to_string(dir.join("tile-size.txt")).ok()?.trim().parse().ok()?;
        if !size.is_finite() || size <= 0. { return None; }
        let (_, _, cells) = crate::tiles::read_live_cells(dir, size)?;
        Some((size, cells))
    });
    policy.as_ref().map(|_| ()).ok_or_else(|| "visible railings disabled: missing or invalid global tiles/live-cells.txt".to_owned())
}

/// The baker already computed the global state mask from its source inventory.
pub fn set_live(size: f32, cells: BTreeSet<(i32, i32)>) { let _ = LIVE.set(Some((size, cells))); }

#[derive(Clone, Debug)]
pub struct Report {
    pub key: u64,
    pub triangles: Vec<[[f32; 3]; 3]>,
    pub reason: &'static str,
    pub matched: f32,
    pub covering: Vec<(u32, u32)>,
    pub added: usize,
}
impl Report {
    pub fn line(&self, tile: (i32, i32)) -> String {
        format!("{} {} {:016x} {} {} {:.6} {} {}", tile.0, tile.1, self.key, self.triangles.len(), self.reason,
            self.matched, self.added, self.covering.iter().map(|(id, h)| format!("{id}:{h:08x}")).collect::<Vec<_>>().join(","))
    }
}
#[derive(Default)]
pub struct Replacement {
    pub removed: Vec<Tri>,
    pub added: Vec<Tri>,
    pub reports: Vec<Report>,
}

/// `keep`: vertex keys of authored prop faces (baked static props, already
/// exact and adjusted, design 2026-10-31-round-rail-collision): a component
/// with any of them stays as it is.
pub fn replace(tris: &mut Vec<Tri>, keep: &std::collections::HashSet<u64>) -> Replacement {
    let (Some(Some(f)), Some(Some((size, live)))) = (FILE.get(), LIVE.get()) else { return Replacement::default() };
    let b = tris.iter().flat_map(|t| t.v).fold([f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY], |b, p|
        [b[0].min(p[0]), b[1].max(p[0]), b[2].min(p[1]), b[3].max(p[1])]);
    let ids: BTreeSet<_> = cells(b, CELL).filter_map(|c| f.table.get(&c)).flatten().copied().collect();
    let objects: Vec<_> = ids.iter().filter_map(|id| f.objects.get(id))
        .filter(|o| !cells(o.bounds(), *size).any(|c| live.contains(&c))).collect();
    replace_with(tris, &objects, keep)
}

// Handles degenerate XY projections (vertical posts) as segments as well.
fn distance(p: Vec3, t: &[Vec3; 3]) -> f32 {
    if (t[1] - t[0]).cross(t[2] - t[0]).length_squared() > 1e-16 {
        return crate::world::closest_on_triangle(p, t).distance_squared(p);
    }
    (0..3).map(|i| {
        let a = t[i]; let d = t[(i + 1) % 3] - a;
        (a + d * ((p - a).dot(d) / d.length_squared().max(1e-20)).clamp(0., 1.)).distance_squared(p)
    }).fold(f32::INFINITY, f32::min)
}

fn side_vote(component: &[usize], tris: &[Tri], objects: &[&Object]) -> f32 {
    let mesh: Vec<_> = objects.iter().flat_map(|o| o.mesh.iter().map(|t| t.map(Vec3::from_array))).collect();
    let (mut close, mut count) = (0usize, 0usize);
    for &i in component {
        let t = tris[i].v.map(Vec3::from_array);
        let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
        if n.z.abs() >= 0.3 { continue; }
        let u = (t[1] - t[0]).normalize_or_zero(); let v = n.cross(u);
        let xy = t.map(|p| [(p - t[0]).dot(u), (p - t[0]).dot(v)]);
        let lo = [xy.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min), xy.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min)];
        let hi = [xy.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max), xy.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max)];
        let before = count;
        let mut sample = |p: Vec3| { count += 1; close += usize::from(mesh.iter().any(|t| distance(p, t) <= 0.03 * 0.03 + 1e-8)); };
        // Cell centres of a 5 cm grid on the face's plane, clipped to its triangle.
        for x in (lo[0] / 0.05).floor() as i32..=(hi[0] / 0.05).ceil() as i32 {
            for y in (lo[1] / 0.05).floor() as i32..=(hi[1] / 0.05).ceil() as i32 {
                let p = t[0] + u * ((x as f32 + 0.5) * 0.05) + v * ((y as f32 + 0.5) * 0.05);
                if distance(p, &t) < 1e-10 { sample(p); }
            }
        }
        if count == before { count += 1; close += usize::from(mesh.iter().any(|m| distance((t[0]+t[1]+t[2])/3., m) <= 0.0009)); }
    }
    if count == 0 { 0. } else { close as f32 / count as f32 }
}

fn replace_with(tris: &mut Vec<Tri>, objects: &[&Object], keep: &std::collections::HashSet<u64>) -> Replacement {
    let mut out = Replacement::default();
    // Spatial buckets plus actual Euclidean distance: corners within 1 cm,
    // including opposite sides of a bucket boundary, connect transitively.
    let mut buckets: HashMap<[i32; 3], Vec<(Vec3, usize)>> = HashMap::new();
    let mut parent: Vec<_> = (0..tris.len()).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize { while p[i] != i { p[i] = p[p[i]]; i = p[i]; } i }
    for (i, t) in tris.iter().enumerate().filter(|(_, t)| RAILING_MATERIALS.contains(&t.material)) {
        for p in t.v.map(Vec3::from_array) {
            let k = p.to_array().map(|v| (v / 0.01).floor() as i32);
            for x in -1..=1 { for y in -1..=1 { for z in -1..=1 {
                if let Some(list) = buckets.get(&[k[0]+x, k[1]+y, k[2]+z]) {
                    for &(q, j) in list { if p.distance_squared(q) <= 0.0001 + 1e-9 {
                        let a = root(&mut parent, i); let b = root(&mut parent, j); parent[a] = b;
                    } }
                }
            } } }
            buckets.entry(k).or_default().push((p, i));
        }
    }
    let mut components: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, t) in tris.iter().enumerate().filter(|(_, t)| RAILING_MATERIALS.contains(&t.material)) {
        let _ = t; components.entry(root(&mut parent, i)).or_default().push(i);
    }
    let mut remove: BTreeSet<usize> = BTreeSet::new(); let mut add: BTreeSet<u32> = BTreeSet::new();
    for component in components.values() {
        let corners: BTreeSet<_> = component.iter().flat_map(|&i| tris[i].v).map(|p| p.map(f32::to_bits)).collect();
        let corners: Vec<_> = corners.into_iter().map(|p| Vec3::from_array(p.map(f32::from_bits))).collect();
        let covering: Vec<_> = objects.iter().copied().filter(|o| corners.iter().filter(|&&p| o.contains(p)).count() * 4 >= corners.len()).collect();
        let covered = corners.iter().filter(|&&p| covering.iter().any(|o| o.contains(p))).count() as f32 / corners.len() as f32 >= 0.9;
        let matched = if covered && !component.iter().any(|&i| keep.contains(&svwc::surfaces::key(&tris[i].v))) { side_vote(component, tris, &covering) } else { 0. };
        let authored = component.iter().any(|&i| keep.contains(&svwc::surfaces::key(&tris[i].v)));
        let reason = if authored { "authored" } else if !covered { "uncovered" } else if matched >= 0.6 { "already-right" } else { "replaced" };
        if reason == "replaced" { remove.extend(component); add.extend(covering.iter().map(|o| o.id)); }
        out.reports.push(Report {
            key: component.iter().map(|&i| svwc::surfaces::key(&tris[i].v)).min().unwrap(),
            triangles: component.iter().map(|&i| tris[i].v).collect(), reason, matched,
            covering: covering.iter().map(|o| (o.id, o.model)).collect(), added: 0,
        });
    }
    let kept: Vec<_> = tris.iter().enumerate().filter(|(i, t)| !remove.contains(i) && RAILING_MATERIALS.contains(&t.material))
        .map(|(_, t)| t.v.map(Vec3::from_array)).collect();
    let mut added_counts = BTreeMap::new();
    for o in objects.iter().filter(|o| add.contains(&o.id)) {
        let first = out.added.len();
        out.added.extend(o.mesh.iter().filter(|t| {
            let p = t.map(Vec3::from_array); let c = (p[0]+p[1]+p[2])/3.;
            !kept.iter().any(|t| distance(c, t) <= 0.0009 + 1e-8)
        }).map(|&v| Tri { v, material: 64 }));
        added_counts.insert(o.id, out.added.len() - first);
    }
    let mut counted = BTreeSet::new();
    out.reports.sort_by_key(|r| r.key);
    for r in &mut out.reports {
        if r.reason == "replaced" {
            r.added = r.covering.iter().filter(|(id, _)| counted.insert(*id)).map(|(id, _)| added_counts.get(id).copied().unwrap_or(0)).sum();
        }
    }
    let mut i = 0;
    tris.retain(|t| { let drop = remove.contains(&i); i += 1; if drop { out.removed.push(*t); } !drop });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn box_mesh(x0: f32, x1: f32, z0: f32, z1: f32) -> Vec<[[f32;3];3]> {
        let a = [[x0,-0.04,z0],[x1,-0.04,z0],[x1,0.04,z0],[x0,0.04,z0],
            [x0,-0.04,z1],[x1,-0.04,z1],[x1,0.04,z1],[x0,0.04,z1]];
        [[0,2,1],[0,3,2],[4,5,6],[4,6,7],[0,1,5],[0,5,4],[1,2,6],[1,6,5],
            [2,3,7],[2,7,6],[3,0,4],[3,4,7]].map(|t| t.map(|i| a[i])).to_vec()
    }
    fn object(id: u32, x0: f32, x1: f32, accurate: bool) -> Object {
        let mut mesh = box_mesh(x0,x1,if accurate {0.} else {0.94},1.);
        if !accurate { mesh.extend(box_mesh(x0,x0+0.04,0.,1.)); mesh.extend(box_mesh(x1-0.04,x1,0.,1.)); }
        Object { id, model: 42, z: [-0.15,1.15], footprint: mesh.iter().map(|t| t.map(|p| [p[0],p[1]])).collect(), mesh }
    }
    fn slab() -> Vec<Tri> { box_mesh(0.,2.,0.,1.).into_iter().map(|v| Tri {v,material:64}).collect() }
    #[test]
    fn slab_removed_mesh_added_and_exact_off_alternative() {
        let o = object(1,0.,2.,false); let original = slab(); let mut tris = original.clone();
        let r = replace_with(&mut tris,&[&o],&Default::default());
        assert!(tris.is_empty()); assert_eq!(r.added.len(),o.mesh.len());
        assert_eq!(r.removed.iter().map(|t| t.v).collect::<Vec<_>>(),original.iter().map(|t| t.v).collect::<Vec<_>>());
        assert_eq!(r.reports[0].reason,"replaced");
        assert!(r.reports[0].line((-1,2)).contains("12 replaced"));
        assert!(r.reports[0].line((-1,2)).ends_with("1:0000002a"));
        // Same predicate as shown(), independent of stair flags.
        for visible in [false,true] {
            assert_eq!(crate::live::shown_with_railings(crate::live::SLAB_TAG, visible), !visible);
            assert_eq!(crate::live::shown_with_railings(crate::live::VISBAR_TAG, visible), visible);
        }
    }
    #[test]
    fn authored_prop_railing_stays() {
        // A baked prop's own railing (lowered under its grind line) is never swapped.
        let o = object(1,0.,2.,false); let mut tris = slab();
        let keep: std::collections::HashSet<u64> = [svwc::surfaces::key(&tris[3].v)].into();
        let r = replace_with(&mut tris,&[&o],&keep);
        assert_eq!(tris.len(),12); assert!(r.added.is_empty()); assert!(r.removed.is_empty());
        assert_eq!(r.reports[0].reason,"authored");
    }
    #[test]
    fn accurate_tube_floor_and_uncovered_stay() {
        let o = object(1,0.,2.,true); let mut tris = slab();
        let r = replace_with(&mut tris,&[&o],&Default::default());
        assert_eq!(tris.len(),12); assert!(r.added.is_empty()); assert_eq!(r.reports[0].reason,"already-right");
        assert!(r.reports[0].matched >= 0.6);
        for t in &mut tris { t.material=1; }
        let r = replace_with(&mut tris,&[&o],&Default::default()); assert_eq!(tris.len(),12); assert!(r.reports.is_empty());
        let mut tris = slab(); let r = replace_with(&mut tris,&[],&Default::default());
        assert_eq!(tris.len(),12); assert_eq!(r.reports[0].reason,"uncovered"); assert!(r.added.is_empty());
    }
    #[test]
    fn union_of_two_objects_and_kept_mesh_suppression() {
        let a = object(1,0.,1.,false); let b = object(2,1.,2.,false); let mut tris = slab();
        let r = replace_with(&mut tris,&[&a,&b],&Default::default());
        assert!(tris.is_empty()); assert_eq!(r.reports[0].covering.len(),2);
        assert_eq!(r.added.len(),a.mesh.len()+b.mesh.len());
        // A separate accurate railing within the same object suppresses its
        // coincident mesh, while the slab still gets replaced.
        let mut o = object(3,0.,2.,false); let accurate = box_mesh(3.,4.,0.94,1.);
        o.mesh.extend(accurate.clone()); o.footprint = o.mesh.iter().map(|t| t.map(|p| [p[0],p[1]])).collect();
        let mut tris = slab(); tris.extend(accurate.iter().map(|&v| Tri {v, material:64}));
        let r = replace_with(&mut tris,&[&o],&Default::default());
        assert_eq!(tris.len(),accurate.len());
        assert!(r.added.iter().all(|t| t.v.iter().all(|p| p[0] <= 2.)));
    }
    #[test]
    fn file_round_trip_all_touched_cells_and_malformed_cell() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/tests/visbars");
        std::fs::create_dir_all(&dir).unwrap(); let path = dir.join("roundtrip.visbars");
        let o = object(7,31.,33.,false);
        write(&path,&[o.clone()]).unwrap(); assert!(verified_policy(&path));
        let bytes = std::fs::read(&path).unwrap(); let f = decode(&bytes).unwrap();
        assert_eq!(f.objects[&7],o); assert_eq!(f.table.len(),4);
        for c in [(0,-1),(0,0),(1,-1),(1,0)] { assert_eq!(f.table[&c],vec![7]); }
        for n in [0,4,15,bytes.len()-1] { assert!(decode(&bytes[..n]).is_err()); }
        let mut bad = bytes.clone(); bad[28..36].copy_from_slice(&0u64.to_le_bytes()); assert!(decode(&bad).is_err());
        let mut bad = bytes; let offset = u64::from_le_bytes(bad[28..36].try_into().unwrap()) as usize;
        bad[offset+16..offset+20].copy_from_slice(&u32::MAX.to_le_bytes()); assert!(decode(&bad).is_err());
        std::fs::remove_file(&path).unwrap(); std::fs::remove_file(path.with_extension("visbars.policy")).unwrap();
    }
    #[test]
    fn live_mask_checks_whole_footprint_and_degenerate_projection() {
        let o = object(1,31.,33.,false);
        let live = BTreeSet::from([(1,0)]);
        assert!(cells(o.bounds(),32.).any(|c| live.contains(&c)));
        let t = [Vec3::ZERO,Vec3::Z,Vec3::Z*2.];
        assert!(distance(Vec3::new(0.01,0.,1.),&t) < 0.00011);
    }
}
