//! Grind rails from GTA's tubes (Manatros 2026-10-20: "the rails in this
//! skatepark aren't grindable"; fixes as general rules). GTA builds coping,
//! handrails and railing bars as capsule and cylinder polygons, mostly in its
//! high-detail (`hi@`) collision: at Vespucci 207 capsules hold every coping
//! tube and rail, while the walking collision only has a coarse slab there.
//! A tube's top line (its axis raised by its radius) is exactly where a board
//! grinds; the edge finder never sees it, a tube has no sharp edge.

use bevy_math::Vec3;
use std::collections::HashMap;

/// GTAxSkate TubeRails (World page, default on).
pub static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Thickest tube that is a rail (metres): coping and handrails are 2-8 cm.
const MAX_RADIUS: f32 = 0.2;
/// Shortest tube line kept (metres).
const MIN_LENGTH: f32 = 0.5;
/// Steepest tube, rise over length (rails.rs MAX_SLOPE).
const MAX_SLOPE: f32 = 0.7;
/// Tube ends this close (metres) join into one rail.
const JOIN: f32 = 0.05;

/// The rails of the latest area build (GTA metres).
/// The latest tube rails and widths with an XY cell index: the offline bake
/// sets every tube of the map at once, and scanning that whole list under a
/// lock for each tile and rail box kept one bake thread busy while the others
/// waited (2026-10-24). Read-only between `set` calls.
#[derive(Default)]
struct Latest {
    rails: Vec<Vec<Vec3>>,
    widths: Vec<(Vec3, Vec3, f32)>,
    rail_cells: HashMap<(i32, i32), Vec<u32>>,
    width_cells: HashMap<(i32, i32), Vec<u32>>,
}
static LATEST: std::sync::RwLock<Option<Latest>> = std::sync::RwLock::new(None);
const INDEX_CELL: f32 = 32.0;
fn index_cell(x: f32, y: f32) -> (i32, i32) {
    ((x / INDEX_CELL).floor() as i32, (y / INDEX_CELL).floor() as i32)
}
/// Cells of the XY box from `lo` to `hi`, padded by `pad` metres.
fn index_cells(lo: Vec3, hi: Vec3, pad: f32) -> impl Iterator<Item = (i32, i32)> {
    let (a, b) = (index_cell(lo.x - pad, lo.y - pad), index_cell(hi.x + pad, hi.y + pad));
    (a.0..=b.0).flat_map(move |x| (a.1..=b.1).map(move |y| (x, y)))
}

/// Preserve the authored tube diameter when making solid rail collision.
pub(crate) fn half_width(a: Vec3, b: Vec3) -> f32 {
    LATEST.read().unwrap_or_else(|e| e.into_inner()).as_ref().map_or(0.0, |l| l.half_width(a, b))
}

impl Latest {
    fn new(rails: Vec<Vec<Vec3>>, widths: Vec<(Vec3, Vec3, f32)>) -> Self {
        let mut latest = Latest::default();
        for (i, r) in rails.iter().enumerate() {
            let mut cells: Vec<(i32, i32)> = r.iter().map(|p| index_cell(p.x, p.y)).collect();
            cells.sort_unstable();
            cells.dedup();
            for c in cells {
                latest.rail_cells.entry(c).or_default().push(i as u32);
            }
        }
        for (i, &(p, q, _)) in widths.iter().enumerate() {
            // half_width matches within 2 cm of the segment.
            for c in index_cells(p.min(q), p.max(q), 0.05) {
                latest.width_cells.entry(c).or_default().push(i as u32);
            }
        }
        latest.rails = rails;
        latest.widths = widths;
        latest
    }

    fn rails_in(&self, centre: [f32; 2], half: f32) -> Vec<Vec<Vec3>> {
        let c = Vec3::new(centre[0], centre[1], 0.);
        // Candidates from the cells the square touches, then the same test,
        // in the original order.
        let mut found: Vec<u32> = index_cells(c - Vec3::splat(half), c + Vec3::splat(half), 0.)
            .filter_map(|cell| self.rail_cells.get(&cell)).flatten().copied().collect();
        found.sort_unstable();
        found.dedup();
        found.into_iter().map(|i| &self.rails[i as usize])
            .filter(|r| r.iter().any(|p| (p.x - centre[0]).abs() <= half && (p.y - centre[1]).abs() <= half))
            .cloned()
            .collect()
    }

    fn half_width(&self, a: Vec3, b: Vec3) -> f32 {
    let mid = (a + b) * 0.5;
    let Some(candidates) = self.width_cells.get(&index_cell(mid.x, mid.y)) else { return 0.0 };
    candidates.iter().map(|&i| self.widths[i as usize]).filter_map(|(p, q, r)| {
        let d = q - p;
        let t = ((mid - p).dot(d) / d.length_squared().max(1e-9)).clamp(0.0, 1.0);
        (mid.distance(p + d * t) < 0.02 && d.normalize_or_zero().dot((b-a).normalize_or_zero()).abs() > 0.99).then_some(r)
    }).fold(0.0, f32::max)
    }
}

/// Rails over the tubes `(end, end, radius)` (GTA metres, z up).
pub fn rails_of(tubes: &[([f32; 3], [f32; 3], f32)]) -> Vec<Vec<Vec3>> {
    let mut pieces: Vec<(Vec3, Vec3)> = tubes
        .iter()
        .filter(|(_, _, r)| *r <= MAX_RADIUS)
        .filter_map(|&(a, b, r)| {
            let (a, b) = (Vec3::from_array(a), Vec3::from_array(b));
            let along = b - a;
            let len = along.length();
            (len >= 0.05 && (along.z / len).abs() <= MAX_SLOPE).then(|| (a + Vec3::Z * r, b + Vec3::Z * r))
        })
        .collect();
    // The same tube decoded twice (overlapping bounds): once.
    pieces.sort_by(|p, q| (p.0.x, p.0.y, p.0.z).partial_cmp(&(q.0.x, q.0.y, q.0.z)).unwrap_or(std::cmp::Ordering::Equal));
    pieces.dedup_by(|p, q| p.0.distance(q.0) < 0.01 && p.1.distance(q.1) < 0.01);
    let mut used = vec![false; pieces.len()];
    let mut rails = Vec::new();
    for start in 0..pieces.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut rail = vec![pieces[start].0, pieces[start].1];
        for forward in [true, false] {
            loop {
                let end = if forward { rail[rail.len() - 1] } else { rail[0] };
                let next = (0..pieces.len()).find(|&k| !used[k] && (pieces[k].0.distance(end) < JOIN || pieces[k].1.distance(end) < JOIN));
                let Some(k) = next else { break };
                used[k] = true;
                let (a, b) = pieces[k];
                let other = if a.distance(end) < JOIN { b } else { a };
                if forward {
                    rail.push(other);
                } else {
                    rail.insert(0, other);
                }
            }
        }
        let length: f32 = rail.windows(2).map(|w| w[0].distance(w[1])).sum();
        if length >= MIN_LENGTH {
            rails.push(rail);
        }
    }
    rails
}

/// Keeps the rails of the tubes the latest map read decoded (`live.rs`).
pub fn set(tubes: &[([f32; 3], [f32; 3], f32)]) -> usize {
    let rails = if ENABLED.load(std::sync::atomic::Ordering::Relaxed) { rails_of(tubes) } else { Vec::new() };
    let n = rails.len();
    let widths: Vec<(Vec3, Vec3, f32)> = tubes.iter()
        .filter(|(_, _, r)| r.is_finite() && *r > 0.0 && *r <= MAX_RADIUS)
        .map(|&(a,b,r)| (Vec3::from_array(a) + Vec3::Z*r, Vec3::from_array(b) + Vec3::Z*r, r)).collect();
    let latest = Latest::new(rails, widths);
    *LATEST.write().unwrap_or_else(|e| e.into_inner()) = Some(latest);
    n
}

/// The tube rails within the area (GTA metres).
pub fn rails_in(centre: [f32; 2], half: f32) -> Vec<Vec<Vec3>> {
    LATEST.read().unwrap_or_else(|e| e.into_inner()).as_ref().map_or_else(Vec::new, |l| l.rails_in(centre, half))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handrail_of_three_capsules_is_one_rail_on_top_and_a_post_is_none() {
        let r = 0.03;
        let tubes = [
            ([0., 0., 1.0], [2., 0., 1.0], r),
            ([2., 0., 1.0], [4., 0., 1.5], r), // down a flight: 14 degrees
            ([4., 0., 1.5], [6., 0., 1.5], r),
            ([1., 1., 0.0], [1., 1., 1.0], r), // a post
            ([0., 5., 0.5], [3., 5., 0.5], 0.6), // a fat bollard row
        ];
        let rails = rails_of(&tubes);
        assert_eq!(rails.len(), 1, "{rails:?}");
        assert_eq!(rails[0].len(), 4);
        assert!((rails[0][0].z - 1.03).abs() < 1e-5, "on the top");
    }

    #[test]
    fn indexed_lookups_equal_a_scan_of_every_tube() {
        // Deterministic pseudo-random tubes over 600 m, some crossing cells.
        let mut seed = 12345u64;
        let mut rnd = || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); ((seed >> 33) as f32) / (1u64 << 31) as f32 };
        let tubes: Vec<([f32; 3], [f32; 3], f32)> = (0..3000).map(|_| {
            let a = [rnd() * 600. - 300., rnd() * 600. - 300., rnd() * 5.];
            let d = [rnd() * 40. - 20., rnd() * 40. - 20., rnd() * 0.5];
            (a, [a[0] + d[0], a[1] + d[1], a[2] + d[2]], 0.01 + rnd() * 0.1)
        }).collect();
        let rails = rails_of(&tubes);
        let widths: Vec<(Vec3, Vec3, f32)> = tubes.iter()
            .map(|&(a, b, r)| (Vec3::from_array(a) + Vec3::Z * r, Vec3::from_array(b) + Vec3::Z * r, r)).collect();
        let latest = Latest::new(rails.clone(), widths.clone());
        for k in 0..40 {
            let centre = [rnd() * 600. - 300., rnd() * 600. - 300.];
            let half = 5. + k as f32 * 4.;
            let scan: Vec<Vec<Vec3>> = rails.iter()
                .filter(|r| r.iter().any(|p| (p.x - centre[0]).abs() <= half && (p.y - centre[1]).abs() <= half)).cloned().collect();
            assert_eq!(latest.rails_in(centre, half), scan);
        }
        for &(p, q, r) in widths.iter().take(500) {
            let (a, b) = (p.lerp(q, 0.3), p.lerp(q, 0.7));
            let mid = (a + b) * 0.5;
            let scan = widths.iter().filter_map(|&(p, q, r)| {
                let d = q - p;
                let t = ((mid - p).dot(d) / d.length_squared().max(1e-9)).clamp(0.0, 1.0);
                (mid.distance(p + d * t) < 0.02 && d.normalize_or_zero().dot((b - a).normalize_or_zero()).abs() > 0.99).then_some(r)
            }).fold(0.0, f32::max);
            assert_eq!(latest.half_width(a, b), scan);
            assert!(scan >= r);
        }
    }

}
