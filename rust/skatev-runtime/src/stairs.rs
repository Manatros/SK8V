//! Conservative fallback for unflagged mineral stair ramps. No detail-copy
//! matching: approved planar flights go through the same 17 cm terrace builder.
use bevy_math::Vec3;
use rustc_hash::FxHashMap;
use svwc::Tri;

/// GTAxSkate UnflaggedStairs (World page, test, default off): a fall through
/// Skate's collision at a replaced flight (2026-10-20 16:14) and the earlier
/// holes from guessed stairs keep it opt-in until verified.
pub static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn rebuild(tris: &mut Vec<Tri>, tubes: &[([f32; 3], [f32; 3], f32)]) -> (Vec<Tri>, usize) {
    if !ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return (Vec::new(), 0);
    }
    rebuild_enabled(tris, tubes)
}

fn rebuild_enabled(tris: &mut Vec<Tri>, tubes: &[([f32; 3], [f32; 3], f32)]) -> (Vec<Tri>, usize) {
    let (steps, ramps) = rebuild_keeping(tris, tubes);
    (steps, ramps.len())
}

/// The unflagged rebuild whatever the switch, returning the replaced ramps
/// too (the bake keeps both variants; the game picks one by its setting).
pub fn rebuild_keeping(tris: &mut Vec<Tri>, tubes: &[([f32; 3], [f32; 3], f32)]) -> (Vec<Tri>, Vec<Tri>) {
    let points: Vec<_> = tris.iter().map(|t| t.v.map(Vec3::from_array)).collect();
    let normal = |i: usize| (points[i][1] - points[i][0]).cross(points[i][2] - points[i][0]).normalize_or_zero();
    let candidates: Vec<_> = (0..tris.len()).filter(|&i| {
        matches!(tris[i].material, 1..=3 | 14..=17 | 81 | 174)
            && (40f32.to_radians().cos()..=25f32.to_radians().cos()).contains(&normal(i).z)
    }).collect();
    if candidates.is_empty() { return (Vec::new(), Vec::new()); }
    let mut parent: Vec<_> = (0..candidates.len()).collect();
    fn root(parent: &mut [usize], i: usize) -> usize {
        if parent[i] != i { parent[i] = root(parent, parent[i]); }
        parent[i]
    }
    let mut edges: FxHashMap<([i32; 3], [i32; 3]), Vec<usize>> = FxHashMap::default();
    for (k, &i) in candidates.iter().enumerate() {
        for e in 0..3 {
            let key = |v: Vec3| v.to_array().map(|x| (x * 100.).round() as i32);
            let (a, b) = (key(points[i][e]), key(points[i][(e + 1) % 3]));
            let peers = edges.entry(if a < b { (a, b) } else { (b, a) }).or_default();
            for &j in peers.iter() {
                if tris[i].material == tris[candidates[j]].material && normal(i).dot(normal(candidates[j])) > 0.9999 {
                    let (a, b) = (root(&mut parent, k), root(&mut parent, j));
                    parent[a] = b;
                }
            }
            peers.push(k);
        }
    }
    let mut groups: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    for (k, &i) in candidates.iter().enumerate() { groups.entry(root(&mut parent, k)).or_default().push(i); }
    let grid = crate::grid::XyGrid::triangles(2., &points);
    let mut drop = vec![false; tris.len()];
    let mut steps = Vec::new();
    for group in groups.values() {
        let n = normal(group[0]);
        let up = Vec3::new(-n.x, -n.y, 0.).normalize();
        let side = Vec3::new(-up.y, up.x, 0.);
        let origin = points[group[0]][0];
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let (mut u0, mut u1, mut v0, mut v1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        let mut area = 0.;
        let mut planar = true;
        for &i in group {
            let t = points[i];
            area += (t[1] - t[0]).cross(t[2] - t[0]).z * 0.5;
            for p in t {
                planar &= (p - origin).dot(n).abs() <= 0.02;
                lo = lo.min(p); hi = hi.max(p);
                u0 = u0.min(p.dot(up)); u1 = u1.max(p.dot(up));
                v0 = v0.min(p.dot(side)); v1 = v1.max(p.dot(side));
            }
        }
        let (length, width) = (u1 - u0, v1 - v0);
        // Full rectangular footprint prevents sparse strips/triangles passing
        // just because their bounding box is large enough.
        if !planar || length < 1.5 || width < 1.0 || !(0.95..=1.05).contains(&(area / (length * width))) { continue; }
        // Conservative AABB separation from *all* decoded tubes, independent
        // of TubeRails. Rejection may be wider than 0.5 m, never narrower.
        if tubes.iter().any(|&(a, b, radius)| {
            let (a, b) = (Vec3::from_array(a), Vec3::from_array(b));
            let pad = Vec3::splat(0.5 + radius);
            (a.min(b) - pad).cmple(hi).all() && (a.max(b) + pad).cmpge(lo).all()
        }) { continue; }
        let landing = |u: f32, z: f32| {
            [0.15, 0.5, 0.85].into_iter().all(|f| {
                let p = up * u + side * (v0 + width * f);
                grid.at(p.x, p.y).iter().any(|&j| {
                    let j = j as usize;
                    normal(j).z > 0.995 && svwc::clean::floor_at(&tris[j], [p.x, p.y]).is_some_and(|h| (h - z).abs() < 0.06)
                })
            })
        };
        if ![0.15, 0.4].into_iter().all(|d| landing(u0 - d, lo.z) && landing(u1 + d, hi.z)) { continue; }
        let flight: Vec<_> = group.iter().map(|&i| tris[i]).collect();
        let built = svwc::clean::terrace(&flight);
        if built.is_empty() { continue; }
        steps.extend(built);
        for &i in group { drop[i] = true; }
    }
    let mut removed = Vec::new();
    let mut i = 0;
    tris.retain(|t| { let keep = !drop[i]; if !keep { removed.push(*t); } i += 1; keep });
    (steps, removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scene(material: u8) -> Vec<Tri> {
        let mut tris = Vec::new();
        for (y0, y1, z0, z1) in [(-1., 0., 0., 0.), (0., 3., 0., 1.8), (3., 4., 1.8, 1.8)] {
            let [a, b, c, d] = [[0., y0, z0], [2., y0, z0], [2., y1, z1], [0., y1, z1]];
            tris.extend([Tri { v: [a, b, c], material }, Tri { v: [a, c, d], material }]);
        }
        tris
    }
    #[test]
    fn concrete_flight_between_landings_becomes_steps() {
        let mut tris = scene(1);
        let (steps, replaced) = rebuild_enabled(&mut tris, &[]);
        assert_eq!(replaced, 2);
        assert_eq!(tris.len(), 4);
        assert!(steps.len() > 20);
        assert!(steps.iter().any(|t| t.v.iter().all(|p| p[2] == t.v[0][2])));
    }
    #[test]
    fn coping_wood_metal_and_missing_landings_keep_ramps() {
        for material in [55, 69] { assert_eq!(rebuild_enabled(&mut scene(material), &[]).1, 0); }
        assert_eq!(rebuild_enabled(&mut scene(1), &[([0., 3., 1.8], [2., 3., 1.8], 0.05)]).1, 0);
        assert_eq!(rebuild_enabled(&mut scene(1), &[([0., 0., 0.], [0., 3., 1.8], 0.05)]).1, 0,
            "sloping coping remains protected too");
        let mut no_landing = scene(1); no_landing.truncate(4);
        assert_eq!(rebuild_enabled(&mut no_landing, &[]).1, 0);
        let mut narrow = scene(1);
        for t in &mut narrow { for p in &mut t.v { p[0] *= 0.4; } }
        assert_eq!(rebuild_enabled(&mut narrow, &[]).1, 0);
    }
}
