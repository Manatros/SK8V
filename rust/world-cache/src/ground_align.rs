//! A60: a deterministic world-space displacement field, before cleanup/rails.
//! Canonical blocks, rather than the requesting tile context, own all evidence.
use SkateVRuntime::{
    grid::{XyGrid, height},
    parks,
};
use bevy_math::Vec3;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use svwc::Tri;

pub const BLOCK: f32 = 16.0;
pub const HALO: f32 = 0.6;
const CAP: f32 = 0.04;
const STEP_CAP: f32 = 0.06;
const COS15: f32 = 0.9659258;
const COS30: f32 = 0.8660254;
type Key = [u32; 3];
fn key(p: [f32; 3]) -> Key {
    p.map(|v| if v == 0.0 { 0 } else { v.to_bits() })
}
fn edge_key(a: Vec3, b: Vec3) -> (Key, Key) {
    let a = key(a.to_array());
    let b = key(b.to_array());
    if a < b { (a, b) } else { (b, a) }
}
fn normal(t: &[Vec3; 3]) -> Vec3 {
    (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero()
}

/// Only outdoor ground materials. Metals, timber, water, unknowns and props
/// never donate an alignment; their adjoining vertices are pinned too.
pub fn ground_material(m: u8) -> bool {
    matches!(m, 1..=9 | 11..=45 | 101 | 102 | 174 | 175 | 179)
}

struct Mesh {
    tris: Vec<[Vec3; 3]>,
    normals: Vec<Vec3>,
    grid: XyGrid,
}
impl Mesh {
    fn new(tris: Vec<[Vec3; 3]>, upward: bool) -> Self {
        let normals = tris
            .iter()
            .map(|t| {
                let n = normal(t);
                if upward && n.z < 0.0 { -n } else { n }
            })
            .collect();
        let grid = XyGrid::triangles(1.0, &tris);
        Self {
            tris,
            normals,
            grid,
        }
    }
    fn hits(&self, p: Vec3, cap: f32) -> Vec<(f32, Vec3)> {
        self.grid
            .at(p.x, p.y)
            .iter()
            .filter_map(|&i| {
                let i = i as usize;
                let z = height(&self.tris[i], p.x, p.y)?;
                ((z - p.z).abs() <= cap + 1e-6).then_some((z, self.normals[i]))
            })
            .collect()
    }
    fn top_compatible(&self, p: Vec3, n: Vec3) -> Result<(f32, Vec3), FaceReject> {
        let hits: Vec<_> = self
            .hits(p, CAP)
            .into_iter()
            .filter(|(_, v)| v.dot(n) >= COS15)
            .collect();
        if hits.is_empty() {
            return Err(FaceReject::VisZero);
        }
        let min_z = hits.iter().map(|h| h.0).fold(f32::MAX, f32::min);
        let max_z = hits.iter().map(|h| h.0).fold(f32::MIN, f32::max);
        // Overlapping layer-0 GTA road patch strips sit 1-4 cm above the base
        // road mesh (same surface band within CAP): follow the highest visible
        // layer so wheels never sink below it. Only wider splits stay ambiguous.
        if max_z - min_z > CAP + 1e-6 {
            return Err(FaceReject::VisMulti);
        }
        let best = hits
            .into_iter()
            .max_by(|a, b| {
                a.0.total_cmp(&b.0)
                    .then_with(|| a.1.dot(n).total_cmp(&b.1.dot(n)))
                    .then_with(|| a.1.z.total_cmp(&b.1.z))
                    .then_with(|| a.1.x.total_cmp(&b.1.x))
                    .then_with(|| a.1.y.total_cmp(&b.1.y))
            })
            .unwrap();
        Ok(best)
    }
}

fn segment_distance(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let d = b - a;
    p.distance(a + d * ((p - a).dot(d) / d.length_squared().max(1e-20)).clamp(0.0, 1.0))
}
fn closest_on_face(p: Vec3, t: &[Vec3; 3]) -> Vec3 {
    let n = normal(t);
    if n.length_squared() > 0.5 {
        let q = p - n * (p - t[0]).dot(n);
        if (0..3).all(|i| (t[(i + 1) % 3] - t[i]).cross(q - t[i]).dot(n) >= -1e-5) {
            return q;
        }
    }
    let mut best = t[0];
    let mut best_d2 = p.distance_squared(best);
    for i in 0..3 {
        let a = t[i];
        let b = t[(i + 1) % 3];
        let d = b - a;
        let c = a + d * ((p - a).dot(d) / d.length_squared().max(1e-20)).clamp(0.0, 1.0);
        let d2 = p.distance_squared(c);
        if d2 < best_d2 {
            best_d2 = d2;
            best = c;
        }
    }
    best
}
fn near_face(p: Vec3, t: &[Vec3; 3], radius: f32) -> bool {
    p.distance(closest_on_face(p, t)) <= radius
}

#[derive(Default, Clone, Copy, Debug)]
pub struct GuardStats {
    pub total_vertices: usize,
    pub in_park: usize,
    pub near_ineligible: usize,
    pub near_steep: usize,
    pub near_raw_step: usize,
    pub unmatched_edge_one: usize,
    pub unmatched_edge_multi: usize,
    pub corner_fold: usize,
    pub face_park_cross: usize,
    pub face_vis_zero: usize,
    pub face_vis_multi: usize,
    pub face_vis_step: usize,
    pub face_near_ineligible: usize,
    pub face_near_steep: usize,
    pub face_near_raw_step: usize,
    pub face_sample_step: usize,
    pub face_no_samples: usize,
    pub disp_vis_zero: usize,
    pub disp_vis_multi: usize,
    pub disp_vertex_deadband: usize,
    pub disp_vis_step: usize,
    pub disp_raw_ambiguous: usize,
    pub disp_median_deadband: usize,
    pub moved: usize,
}
impl GuardStats {
    pub fn add(&mut self, other: &Self) {
        self.total_vertices += other.total_vertices;
        self.in_park += other.in_park;
        self.near_ineligible += other.near_ineligible;
        self.near_steep += other.near_steep;
        self.near_raw_step += other.near_raw_step;
        self.unmatched_edge_one += other.unmatched_edge_one;
        self.unmatched_edge_multi += other.unmatched_edge_multi;
        self.corner_fold += other.corner_fold;
        self.face_park_cross += other.face_park_cross;
        self.face_vis_zero += other.face_vis_zero;
        self.face_vis_multi += other.face_vis_multi;
        self.face_vis_step += other.face_vis_step;
        self.face_near_ineligible += other.face_near_ineligible;
        self.face_near_steep += other.face_near_steep;
        self.face_near_raw_step += other.face_near_raw_step;
        self.face_sample_step += other.face_sample_step;
        self.face_no_samples += other.face_no_samples;
        self.disp_vis_zero += other.disp_vis_zero;
        self.disp_vis_multi += other.disp_vis_multi;
        self.disp_vertex_deadband += other.disp_vertex_deadband;
        self.disp_vis_step += other.disp_vis_step;
        self.disp_raw_ambiguous += other.disp_raw_ambiguous;
        self.disp_median_deadband += other.disp_median_deadband;
        self.moved += other.moved;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FaceReject {
    Ok,
    ParkCross,
    VisZero,
    VisMulti,
    VisStep,
    NearIneligible,
    NearSteep,
    NearRawStep,
    SampleStep,
    NoSamples,
}

/// Compute all vertices owned by this canonical block from COMPLETE raw
/// collision overlapping its halo (including excluded witnesses).
pub fn field(
    block: (i32, i32),
    collision: &[Tri],
    eligible: &[bool],
    drawn: &[svwc::ground::GroundTri],
) -> BTreeMap<Key, f32> {
    field_with_stats(block, collision, eligible, drawn, None).0
}

pub fn field_with_stats(
    block: (i32, i32),
    collision: &[Tri],
    eligible: &[bool],
    drawn: &[svwc::ground::GroundTri],
    mark_probes: Option<&[(&str, [f32; 2], f32)]>,
) -> (BTreeMap<Key, f32>, GuardStats) {
    assert_eq!(collision.len(), eligible.len());
    let raw = Mesh::new(
        collision
            .iter()
            .map(|t| t.v.map(Vec3::from_array))
            .collect(),
        false,
    );
    let vis = Mesh::new(
        drawn
            .iter()
            .filter(|t| t.layer == 0)
            .map(|t| t.p.map(Vec3::from_array))
            .collect(),
        true,
    );
    // An unmatched edge can be a boundary or one side of a T-junction.
    // Conservatively pin every point on it, including its endpoints. Unlike
    // detecting a remote subdivision, exact edge ownership is locally
    // complete at every point of the edge (both owners touch that point).
    let mut edges = HashMap::<(Key, Key), BTreeSet<Key>>::new();
    for t in &raw.tris {
        for i in 0..3 {
            edges
                .entry(edge_key(t[i], t[(i + 1) % 3]))
                .or_default()
                .insert(key(t[(i + 2) % 3].to_array()));
        }
    }
    // The full face is sampled once in a block; all its incident vertices
    // combine the same world-grid evidence. Invalid interior evidence vetoes
    // movement rather than letting a valid corner drag an unsupported span.
    let mut evidence = HashMap::<usize, FaceReject>::new();
    let mut vertices = BTreeMap::<Key, Vec3>::new();
    for t in collision {
        for p in t.v {
            if block_of(p) == block {
                vertices.insert(key(p), Vec3::from_array(p));
            }
        }
    }
    let mut stats = GuardStats::default();
    let mut mark_stats: Vec<(&str, GuardStats)> = mark_probes
        .unwrap_or(&[])
        .iter()
        .map(|(name, _, _)| (*name, GuardStats::default()))
        .collect();
    let out = vertices
        .into_iter()
        .map(|(k, p)| {
            stats.total_vertices += 1;
            let mut v_near_ineligible = false;
            let mut v_near_steep = false;
            let mut v_near_raw_step = false;
            let mut v_edge_one = false;
            let mut v_edge_multi = false;
            let mut v_fold = false;
            let mut normals = Vec::new();
            let v_park = parks::in_park(p.truncate());
            let mut pinned = v_park;
            // Pin protected surfaces within the edge-guard radius, not merely
            // identical corners: walls/kerbs/rails/props retain exact geometry.
            let mut ids = Vec::new();
            for x in raw.grid.cell_of(p.x - 0.3)..=raw.grid.cell_of(p.x + 0.3) {
                for y in raw.grid.cell_of(p.y - 0.3)..=raw.grid.cell_of(p.y + 0.3) {
                    ids.extend_from_slice(raw.grid.cell(x, y));
                }
            }
            ids.sort_unstable();
            ids.dedup();
            for &i in &ids {
                let i = i as usize;
                let t = &raw.tris[i];
                if (!eligible[i] || raw.normals[i].z < 0.5) && near_face(p, t, 0.3) {
                    if !eligible[i] { v_near_ineligible = true; }
                    if raw.normals[i].z < 0.5 { v_near_steep = true; }
                    pinned = true;
                }
                for e in 0..3 {
                    let len = edges[&edge_key(t[e], t[(e + 1) % 3])].len();
                    if len != 2 && segment_distance(p, t[e], t[(e + 1) % 3]) <= 0.0002 {
                        if len < 2 { v_edge_one = true; } else { v_edge_multi = true; }
                        pinned = true;
                    }
                }
                if t.iter().any(|q| key(q.to_array()) == k)
                    && eligible[i]
                    && raw.normals[i].z >= 0.5
                {
                    normals.push(raw.normals[i]);
                }
            }
            let n = normals.iter().copied().sum::<Vec3>().normalize_or_zero();
            if normals.is_empty()
                || normals
                    .iter()
                    .any(|a| normals.iter().any(|b| a.dot(*b) < COS30))
            {
                v_fold = true;
                pinned = true;
            }
            if !pinned {
                for &i in &ids {
                    let i = i as usize;
                    let c = closest_on_face(p, &raw.tris[i]);
                    if p.distance(c) <= 0.3
                        && ((c - p).dot(n).abs() > STEP_CAP
                            || (c - p).dot(raw.normals[i]).abs() > STEP_CAP)
                    {
                        v_near_raw_step = true;
                        pinned = true;
                        break;
                    }
                }
            }
            let incident: Vec<_> = raw
                .grid
                .at(p.x, p.y)
                .iter()
                .map(|&i| i as usize)
                .filter(|&i| {
                    raw.tris[i].iter().any(|q| key(q.to_array()) == k)
                        && eligible[i]
                        && raw.normals[i].z >= 0.5
                })
                .collect();
            let mut face_reject = FaceReject::Ok;
            if !pinned {
                for i in incident {
                    let rej = *evidence.entry(i).or_insert_with(|| {
                        face_check(&raw.tris[i], raw.normals[i], &raw, eligible, &vis)
                    });
                    if rej != FaceReject::Ok {
                        face_reject = rej;
                        pinned = true;
                        break;
                    }
                }
            }
            let mut vs = GuardStats::default();
            vs.total_vertices = 1;
            if v_park { vs.in_park = 1; }
            if v_near_ineligible { vs.near_ineligible = 1; }
            if v_near_steep { vs.near_steep = 1; }
            if v_near_raw_step { vs.near_raw_step = 1; }
            if v_edge_one { vs.unmatched_edge_one = 1; }
            if v_edge_multi { vs.unmatched_edge_multi = 1; }
            if v_fold { vs.corner_fold = 1; }
            match face_reject {
                FaceReject::Ok => {}
                FaceReject::ParkCross => vs.face_park_cross = 1,
                FaceReject::VisZero => vs.face_vis_zero = 1,
                FaceReject::VisMulti => vs.face_vis_multi = 1,
                FaceReject::VisStep => vs.face_vis_step = 1,
                FaceReject::NearIneligible => vs.face_near_ineligible = 1,
                FaceReject::NearSteep => vs.face_near_steep = 1,
                FaceReject::NearRawStep => vs.face_near_raw_step = 1,
                FaceReject::SampleStep => vs.face_sample_step = 1,
                FaceReject::NoSamples => vs.face_no_samples = 1,
            }
            let displacement = if pinned {
                0.0
            } else {
                displacement(p, n, &raw, &vis, &mut vs)
            };
            if displacement != 0.0 {
                vs.moved = 1;
            }
            stats.add(&vs);
            if let Some(probes) = mark_probes {
                for (idx, (_, xy, r)) in probes.iter().enumerate() {
                    if p.truncate().distance(bevy_math::Vec2::from_array(*xy)) <= *r {
                        mark_stats[idx].1.add(&vs);
                    }
                }
            }
            (k, displacement)
        })
        .collect();
    if let Some(_) = mark_probes {
        for (name, ms) in mark_stats {
            if ms.total_vertices > 0 {
                eprintln!("A60-MARK {name} block {block:?}: {ms:?}");
            }
        }
    }
    (out, stats)
}

fn face_check(
    t: &[Vec3; 3],
    n: Vec3,
    raw: &Mesh,
    eligible: &[bool],
    vis: &Mesh,
) -> FaceReject {
    // A face crossing an ownership circle must stay wholly GTA's here;
    // moving only its outer corners would also tilt the part inside a park.
    if parks::PARKS.iter().any(|park| {
        let c = park.centre;
        t.iter().any(|p| p.truncate().distance(c) <= park.radius)
            || height(t, c.x, c.y).is_some()
            || (0..3).any(|i| {
                let a = t[i].truncate();
                let d = t[(i + 1) % 3].truncate() - a;
                (a + d * ((c - a).dot(d) / d.length_squared().max(1e-20)).clamp(0.0, 1.0))
                    .distance(c)
                    <= park.radius
            })
    }) {
        return FaceReject::ParkCross;
    }
    let lo = t[0].min(t[1]).min(t[2]);
    let hi = t[0].max(t[1]).max(t[2]);
    let mut samples = BTreeMap::new();
    for x in (lo.x * 10.0).ceil() as i32..=(hi.x * 10.0).floor() as i32 {
        for y in (lo.y * 10.0).ceil() as i32..=(hi.y * 10.0).floor() as i32 {
            let (px, py) = (x as f32 / 10.0, y as f32 / 10.0);
            let Some(z) = height(t, px, py) else { continue };
            let (vz, vn) = match vis.top_compatible(Vec3::new(px, py, z), n) {
                Ok(hit) => hit,
                Err(rej) => return rej,
            };
            for dx in -3i32..=3 {
                for dy in -3i32..=3 {
                    if dx * dx + dy * dy > 9 {
                        continue;
                    }
                    let (x, y) = (px + dx as f32 * 0.1, py + dy as f32 * 0.1);
                    let predicted = vz - (vn.x * (x - px) + vn.y * (y - py)) / vn.z;
                    for (other_z, other_n) in vis.hits(Vec3::new(x, y, predicted), 0.5) {
                        let delta = Vec3::new(x - px, y - py, other_z - vz);
                        if vn.dot(other_n) < COS30
                            || delta.dot(vn).abs() > STEP_CAP
                            || delta.dot(other_n).abs() > STEP_CAP
                        {
                            return FaceReject::VisStep;
                        }
                    }
                }
            }
            let q = Vec3::new(px, py, z);
            for gx in raw.grid.cell_of(px - 0.3)..=raw.grid.cell_of(px + 0.3) {
                for gy in raw.grid.cell_of(py - 0.3)..=raw.grid.cell_of(py + 0.3) {
                    for &j in raw.grid.cell(gx, gy) {
                        let j = j as usize;
                        let c = closest_on_face(q, &raw.tris[j]);
                        if q.distance(c) <= 0.3 {
                            if !eligible[j] {
                                return FaceReject::NearIneligible;
                            }
                            if raw.normals[j].z < 0.5 {
                                return FaceReject::NearSteep;
                            }
                            if (c - q).dot(n).abs() > STEP_CAP
                                || (c - q).dot(raw.normals[j]).abs() > STEP_CAP
                            {
                                return FaceReject::NearRawStep;
                            }
                        }
                    }
                }
            }
            samples.insert((x, y), (vz - z, vz, vn));
        }
    }
    for (&(x, y), &(_, z, n)) in &samples {
        for dx in -3i32..=3 {
            for dy in -3i32..=3 {
                if dx * dx + dy * dy > 9 {
                    continue;
                }
                if let Some(&(_, other_z, other_n)) = samples.get(&(x + dx, y + dy)) {
                    let delta = Vec3::new(dx as f32 * 0.1, dy as f32 * 0.1, other_z - z);
                    if n.dot(other_n) < COS30
                        || delta.dot(n).abs() > STEP_CAP
                        || delta.dot(other_n).abs() > STEP_CAP
                    {
                        return FaceReject::SampleStep;
                    }
                }
            }
        }
    }
    if samples.is_empty() {
        FaceReject::NoSamples
    } else {
        FaceReject::Ok
    }
}

fn displacement(p: Vec3, n: Vec3, raw: &Mesh, vis: &Mesh, stats: &mut GuardStats) -> f32 {
    // World-aligned 10 cm samples, independent of triangulation, tile and
    // iteration order. Include the vertex itself so missing local evidence
    // cannot be borrowed from a neighbouring strip.
    let (vertex_z, vnormal) = match vis.top_compatible(p, n) {
        Ok(hit) => hit,
        Err(FaceReject::VisZero) => {
            stats.disp_vis_zero += 1;
            return 0.0;
        }
        Err(_) => {
            stats.disp_vis_multi += 1;
            return 0.0;
        }
    };
    if (vertex_z - p.z).abs() < 0.01 {
        stats.disp_vertex_deadband += 1;
        return 0.0;
    }
    let mut ds = Vec::new();
    ds.push(vertex_z - p.z);
    // Drawn kerb/step guard in an exact 30 cm neighbourhood. Tangent-plane
    // residuals do not mistake a smooth hill for a step.
    for dx in -3i32..=3 {
        for dy in -3i32..=3 {
            if dx * dx + dy * dy > 9 {
                continue;
            }
            let x = p.x + dx as f32 * 0.1;
            let y = p.y + dy as f32 * 0.1;
            let z = p.z - (n.x * (x - p.x) + n.y * (y - p.y)) / n.z;
            let q = Vec3::new(x, y, z);
            for (vz, vn) in vis.hits(q, 0.5) {
                let delta = Vec3::new(x - p.x, y - p.y, vz - vertex_z);
                if vn.dot(vnormal) < COS30
                    || delta.dot(vnormal).abs() > STEP_CAP
                    || delta.dot(vn).abs() > STEP_CAP
                {
                    stats.disp_vis_step += 1;
                    return 0.0;
                }
            }
        }
    }
    let (gx, gy) = ((p.x * 10.0).floor() as i32, (p.y * 10.0).floor() as i32);
    for ix in gx - 1..=gx + 1 {
        for iy in gy - 1..=gy + 1 {
            let (x, y) = (ix as f32 / 10.0, iy as f32 / 10.0);
            let z = p.z - (n.x * (x - p.x) + n.y * (y - p.y)) / n.z;
            let q = Vec3::new(x, y, z);
            let hits = raw.hits(q, CAP);
            let compatible: Vec<_> = hits
                .into_iter()
                .filter(|(_, cn)| cn.z >= 0.5 && cn.dot(n) >= COS30)
                .collect();
            // Collision triangles meeting on a shared edge represent the same
            // surface; different heights or normals remain ambiguous.
            let Some(&(cz, cn)) = compatible.first() else {
                continue;
            };
            if compatible
                .iter()
                .any(|&(z, nn)| (z - cz).abs() > 0.001 || nn.dot(cn) < COS15)
            {
                stats.disp_raw_ambiguous += 1;
                return 0.0;
            }
            if let Ok((vz, _)) = vis.top_compatible(Vec3::new(x, y, cz), cn) {
                ds.push(vz - cz);
            }
        }
    }
    if ds.is_empty() {
        return 0.0;
    }
    ds.sort_by(f32::total_cmp);
    let d = ds[ds.len() / 2];
    if d.abs() < 0.01 || d.abs() > CAP + 1e-6 {
        stats.disp_median_deadband += 1;
        return 0.0;
    }
    // Millimetre displacements, one result for every copy of this vertex.
    (d * 1000.0).round().clamp(-40.0, 40.0) / 1000.0
}

pub fn block_of(p: [f32; 3]) -> (i32, i32) {
    ((p[0] / BLOCK).floor() as i32, (p[1] / BLOCK).floor() as i32)
}

#[derive(Default)]
pub struct Fields {
    blocks: HashMap<(i32, i32), BTreeMap<Key, f32>>,
}
impl Fields {
    pub fn apply<E>(
        &mut self,
        tris: &mut [Tri],
        mut load: impl FnMut((i32, i32)) -> Result<BTreeMap<Key, f32>, E>,
    ) -> Result<(usize, f32), E> {
        let mut moved = 0;
        let mut max = 0.0f32;
        for t in tris {
            for p in &mut t.v {
                let b = block_of(*p);
                if !self.blocks.contains_key(&b) {
                    // Bounded per-worker cache. Eviction cannot affect the field.
                    if self.blocks.len() >= 1024 {
                        self.blocks.clear();
                    }
                    self.blocks.insert(b, load(b)?);
                }
                let d = self.blocks[&b].get(&key(*p)).copied().unwrap_or(0.0);
                if d != 0.0 {
                    p[2] += d;
                    moved += 1;
                    max = max.max(d.abs());
                }
            }
        }
        Ok((moved, max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn collision(z: f32) -> Vec<Tri> {
        let c = [0.0, 0.0, z];
        let r = [
            [-1.0, -1.0, z],
            [1.0, -1.0, z],
            [1.0, 1.0, z],
            [-1.0, 1.0, z],
        ];
        (0..4)
            .map(|i| Tri {
                v: [c, r[i], r[(i + 1) % 4]],
                material: 4,
            })
            .collect()
    }
    fn drawn(z: f32, layer: u8) -> svwc::ground::GroundTri {
        svwc::ground::GroundTri {
            p: [[-10.0, -10.0, z], [20.0, -10.0, z], [-10.0, 20.0, z]],
            uv: [[0.0; 2]; 3],
            map: 0,
            seam: 0,
            layer,
        }
    }
    fn at(t: &[Tri], d: &[svwc::ground::GroundTri]) -> f32 {
        field((0, 0), t, &vec![true; t.len()], d)[&key(t[0].v[0])]
    }
    #[test]
    fn strip_two_cm_above_lifts_and_hump_three_cm_above_lowers() {
        assert_eq!(at(&collision(0.0), &[drawn(0.02, 0)]), 0.02);
        assert_eq!(at(&collision(0.03), &[drawn(0.0, 0)]), -0.03);
    }
    #[test]
    fn offsets_over_four_cm_and_under_one_cm_are_unchanged() {
        for z in [0.08, -0.08, 0.009, -0.009] {
            assert_eq!(at(&collision(0.0), &[drawn(z, 0)]), 0.0);
        }
        assert_eq!(at(&collision(0.0), &[drawn(0.04, 0)]), 0.04);
    }
    #[test]
    fn no_drawn_ground_and_decals_are_unchanged() {
        assert_eq!(at(&collision(0.0), &[]), 0.0);
        assert_eq!(at(&collision(0.0), &[drawn(0.02, 1)]), 0.0);
        assert_eq!(at(&collision(0.0), &[drawn(0.02, 0), drawn(0.03, 1)]), 0.02);
    }
    #[test]
    fn overlapping_road_strip_picks_top_within_cap_and_rejects_wider_split() {
        assert_eq!(at(&collision(0.0), &[drawn(0.02, 0), drawn(0.03, 0)]), 0.03);
        assert_eq!(at(&collision(0.0), &[drawn(-0.03, 0), drawn(0.03, 0)]), 0.0);
    }
    #[test]
    fn walls_kerbs_and_protected_props_pin_their_neighbourhood() {
        let mut t = collision(0.0);
        t.push(Tri {
            v: [[0.2, -1.0, 0.0], [0.2, 1.0, 0.0], [0.2, 1.0, 0.15]],
            material: 4,
        });
        assert_eq!(at(&t, &[drawn(0.02, 0)]), 0.0);
        let t = collision(0.0);
        assert_eq!(
            field((0, 0), &t, &[false; 4], &[drawn(0.02, 0)])[&key(t[0].v[0])],
            0.0
        );
    }
    #[test]
    fn drawn_step_and_normal_mismatch_veto_alignment() {
        let mut high = drawn(0.12, 0);
        high.p = [[0.15, -1.0, 0.12], [2.0, -1.0, 0.12], [0.15, 2.0, 0.12]];
        assert_eq!(at(&collision(0.0), &[drawn(0.02, 0), high]), 0.0);
        let mut slope = drawn(0.02, 0);
        for p in &mut slope.p {
            p[2] = 0.02 + p[0] * 0.4;
        }
        assert_eq!(at(&collision(0.0), &[slope]), 0.0);
    }
    #[test]
    fn drawn_winding_does_not_change_alignment() {
        let mut d = drawn(0.02, 0);
        d.p.swap(1, 2);
        assert_eq!(at(&collision(0.0), &[d]), 0.02);
    }
    #[test]
    fn genuine_gutter_is_unchanged_and_shallower_drawn_gutter_reshapes_sides() {
        let mut t = collision(0.1);
        for tri in &mut t {
            for p in &mut tri.v {
                p[0] += 1.0;
                p[2] = p[0] * 0.1;
            }
        }
        let mut same = drawn(0.0, 0);
        for p in &mut same.p {
            p[2] = p[0] * 0.1;
        }
        assert!(
            field((0, 0), &t, &[true; 4], &[same])
                .values()
                .all(|d| *d == 0.0)
        );
        let mut shallow = drawn(0.0, 0);
        for p in &mut shallow.p {
            p[2] = p[0] * 0.08;
        }
        let d = field((0, 0), &t, &[true; 4], &[shallow])[&key([1.0, 0.0, 0.1])];
        assert!(
            (d + 0.02).abs() <= 0.003,
            "sample median must follow the shallow drawn side: {d}"
        );
    }
    #[test]
    fn parks_and_ineligible_materials_stay_unchanged() {
        let mut t = collision(3.0);
        for p in &mut t[0].v {
            p[0] -= 1374.0;
            p[1] -= 1396.0;
        }
        let b = block_of(t[0].v[0]);
        assert!(field(b, &t, &[true; 4], &[]).values().all(|d| *d == 0.0));
        for m in [0, 10, 55, 64, 69, 82, 125, 128, 255] {
            assert!(!ground_material(m));
        }
    }
    #[test]
    fn shared_vertices_and_overlapping_tile_contexts_have_no_seam_crack() {
        let mut t = collision(0.0);
        for tri in &mut t {
            for p in &mut tri.v {
                p[0] += 64.0;
            }
        }
        let mut d = drawn(0.02, 0);
        for p in &mut d.p {
            p[0] += 64.0;
        }
        let load = |b| Ok::<_, ()>(field(b, &t, &[true; 4], std::slice::from_ref(&d)));
        let mut left = vec![t[0]];
        let mut right = vec![t[1]];
        Fields::default().apply(&mut left, load).unwrap();
        Fields::default().apply(&mut right, load).unwrap();
        assert_eq!(left[0].v[0], right[0].v[0]);
        assert_eq!(left[0].v[2], right[0].v[1]);
        assert_eq!(left[0].v[0][2], 0.02);
        // Field ownership and output do not depend on input order either.
        let a = field((4, 0), &t, &[true; 4], std::slice::from_ref(&d));
        let b = field((4, 0), &[t[3], t[2], t[1], t[0]], &[true; 4], &[d]);
        assert_eq!(a, b);
    }
    #[test]
    fn t_junction_midpoint_and_long_edge_endpoints_are_all_pinned() {
        let t = vec![
            Tri {
                v: [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
                material: 4,
            },
            Tri {
                v: [[0.0, 0.0, 0.0], [1.0, -1.0, 0.0], [1.0, 0.0, 0.0]],
                material: 4,
            },
            Tri {
                v: [[1.0, 0.0, 0.0], [1.0, -1.0, 0.0], [2.0, 0.0, 0.0]],
                material: 4,
            },
        ];
        let f = field((0, 0), &t, &[true; 3], &[drawn(0.02, 0)]);
        for x in [0.0, 1.0, 2.0] {
            assert_eq!(f[&key([x, 0.0, 0.0])], 0.0);
        }
    }
    #[test]
    fn valid_corners_cannot_drag_a_face_across_an_interior_step_or_ambiguity() {
        let t = collision(0.0);
        let step_patch = svwc::ground::GroundTri {
            p: [[0.3, 0.2, 0.12], [0.8, 0.2, 0.12], [0.8, 0.7, 0.12]],
            ..drawn(0.12, 0)
        };
        assert_eq!(at(&t, &[drawn(0.02, 0), step_patch]), 0.0);
        let low_patch = svwc::ground::GroundTri {
            p: [[0.3, 0.2, -0.03], [0.8, 0.2, -0.03], [0.8, 0.7, -0.03]],
            ..drawn(-0.03, 0)
        };
        assert_eq!(at(&t, &[drawn(0.02, 0), low_patch]), 0.0);
    }
    #[test]
    fn asymmetric_closed_fan_with_sloped_drawn_plane_keeps_vertex_deadband() {
        let p = [0.0, 0.0, 0.0];
        let r = [
            [-0.2, -0.2, 0.0],
            [2.0, -0.2, 0.0],
            [2.0, 0.2, 0.0],
            [-0.2, 0.2, 0.0],
        ];
        let t: Vec<_> = (0..4)
            .map(|i| Tri {
                v: [p, r[i], r[(i + 1) % 4]],
                material: 4,
            })
            .collect();
        let mut slope = drawn(0.0, 0);
        for q in &mut slope.p {
            q[2] = 0.005 + 0.015 * q[0];
        }
        assert_eq!(field((0, 0), &t, &[true; 4], &[slope])[&key(p)], 0.0);
    }
    #[test]
    fn smooth_road_bend_under_6cm_step_aligns_while_real_step_vetoes() {
        let bend = |slope: f32| {
            let p = [0.0, 0.0, 0.0];
            let ring = [
                [-1.0, -1.0, 0.0],
                [0.0, -1.0, 0.0],
                [1.0, -1.0, slope],
                [1.0, 1.0, slope],
                [0.0, 1.0, 0.0],
                [-1.0, 1.0, 0.0],
            ];
            let t: Vec<_> = (0..6)
                .map(|i| Tri {
                    v: [p, ring[i], ring[(i + 1) % 6]],
                    material: 4,
                })
                .collect();
            let vis = vec![
                svwc::ground::GroundTri {
                    p: [[-10.0, -10.0, 0.02], [0.0, -10.0, 0.02], [0.0, 10.0, 0.02]],
                    ..drawn(0.02, 0)
                },
                svwc::ground::GroundTri {
                    p: [[-10.0, -10.0, 0.02], [0.0, 10.0, 0.02], [-10.0, 10.0, 0.02]],
                    ..drawn(0.02, 0)
                },
                svwc::ground::GroundTri {
                    p: [[0.0, -10.0, 0.02], [10.0, -10.0, 0.02 + 10.0 * slope], [0.0, 10.0, 0.02]],
                    ..drawn(0.02, 0)
                },
                svwc::ground::GroundTri {
                    p: [[10.0, -10.0, 0.02 + 10.0 * slope], [10.0, 10.0, 0.02 + 10.0 * slope], [0.0, 10.0, 0.02]],
                    ..drawn(0.02, 0)
                },
            ];
            field((0, 0), &t, &[true; 6], &vis)[&key(p)]
        };
        // Smooth 8.5 deg bend: 0.15 * 0.30 = 4.5 cm tangent residual (< STEP_CAP = 0.06) aligns.
        assert_eq!(bend(0.15), 0.02);
        // Real 7 cm kerb/step at 0.3 m (> STEP_CAP) still vetoes.
        let flat = drawn(0.02, 0);
        let step = svwc::ground::GroundTri {
            p: [[0.25, -1.0, 0.09], [2.0, -1.0, 0.09], [0.25, 2.0, 0.09]],
            ..drawn(0.09, 0)
        };
        assert_eq!(at(&collision(0.0), &[flat, step]), 0.0);
    }
    #[test]
    fn overlapping_curb_slab_without_riser_pins_nearby_road() {
        let mut t = collision(0.0);
        // Overlapping eligible sidewalk slab +8 cm high, 0.24 m away in y, no vertical riser.
        t.extend([
            Tri {
                v: [[-1.0, 0.24, 0.08], [1.0, 0.24, 0.08], [1.0, 1.0, 0.08]],
                material: 3,
            },
            Tri {
                v: [[-1.0, 0.24, 0.08], [1.0, 1.0, 0.08], [-1.0, 1.0, 0.08]],
                material: 3,
            },
        ]);
        assert_eq!(at(&t, &[drawn(0.02, 0)]), 0.0);
    }
}
