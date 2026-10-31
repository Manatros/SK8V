//! Train tracks as grind rails (Manatros 2026-10-19: grinding train tracks
//! the way his old SkateGTA plugin handled them). GTA's track nodes
//! (`common.rpf` data/levels/gta5/trains1..12.dat, copied to
//! `<cache>.tracks/`) lie on the track bed; its collision has a rail box
//! 0.15 m tall at 0.7175 m either side (measured on the freight line: 517 of
//! 700 samples). The finder's lines on those boxes came out on their edges
//! or the bed; these lines run along the top of each steel rail.

use bevy_math::{Vec2, Vec3};
use std::path::Path;
use std::sync::OnceLock;

/// Rail centre to the track's centre line (metres): measured on GTA's own
/// rails, freight line and metro alike (1.53 m centre to centre; the 1.435 m
/// standard gauge is between the inner faces, 2026-10-20: lines 5 cm inside
/// the rail).
pub const HALF_GAUGE: f32 = 0.765;
/// Rail top above the node height where no collision is found to snap to
/// (metres): GTA's rail boxes stand 0.13-0.17 m, the visible rail 0.19 m.
pub const RAIL_TOP: f32 = 0.17;
/// Nodes farther apart than this are not one track (a jump in the file).
const MAX_GAP: f32 = 40.0;

/// GTAxSkate TrackRails (World page, default on).
pub static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Every track's node line, GTA metres.
static TRACKS: OnceLock<Vec<Vec<Vec3>>> = OnceLock::new();

/// Reads a trains*.dat: a count line, then "x y z flag" per node.
pub fn parse(text: &str) -> Vec<Vec3> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let v: Vec<f32> = l.split_whitespace().take(3).filter_map(|s| s.parse().ok()).collect();
            (v.len() == 3 && v.iter().all(|x| x.is_finite())).then(|| Vec3::new(v[0], v[1], v[2]))
        })
        .collect()
}

/// Loads trains1..12.dat from `dir`; returns how many tracks it found.
pub fn load(dir: &Path) -> usize {
    TRACKS
        .get_or_init(|| {
            (1..=12)
                .filter_map(|n| std::fs::read_to_string(dir.join(format!("trains{n}.dat"))).ok())
                .map(|t| parse(&t))
                .filter(|t| t.len() >= 2)
                .collect()
        })
        .len()
}

/// The two steel rails of every track within the area (`centre` +- `half`),
/// GTA metres, as polylines.
pub fn rails_in(centre: [f32; 2], half: f32) -> Vec<Vec<Vec3>> {
    if !ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return Vec::new();
    }
    TRACKS.get().map_or_else(Vec::new, |tracks| rails_of(tracks, centre, half))
}

pub fn rails_of(tracks: &[Vec<Vec3>], centre: [f32; 2], half: f32) -> Vec<Vec<Vec3>> {
    let mut out = Vec::new();
    for nodes in tracks {
        let mut left = Vec::new();
        let mut right = Vec::new();
        let flush = |left: &mut Vec<Vec3>, right: &mut Vec<Vec3>, out: &mut Vec<Vec<Vec3>>| {
            for side in [left, right] {
                out.extend(sample_in_area(side, centre, half));
                side.clear();
            }
        };
        for (i, &p) in nodes.iter().enumerate() {
            let connected = |q: Vec3| p.is_finite() && q.is_finite() && q.distance(p) <= MAX_GAP;
            let prev = i.checked_sub(1).map(|j| nodes[j]).filter(|q| connected(*q));
            let next = nodes.get(i + 1).copied().filter(|q| connected(*q));
            if prev.is_none() {
                flush(&mut left, &mut right, &mut out);
            }
            if !p.is_finite() { continue; }
            let along = (next.unwrap_or(p) - prev.unwrap_or(p)).truncate().normalize_or_zero();
            if along == Vec2::ZERO { continue; }
            let side = Vec3::new(-along.y, along.x, 0.0) * HALF_GAUGE;
            let top = p + Vec3::Z * RAIL_TOP;
            left.push(top + side);
            right.push(top - side);
        }
        flush(&mut left, &mut right, &mut out);
    }
    out
}

/// Train nodes can be 6-12 metres apart. Give crown projection support along
/// the entire span, including segments crossing an area with both nodes outside.
/// Sampling preserves the authored polyline; it never smooths through real gaps.
fn sample_in_area(line: &[Vec3], centre: [f32; 2], half: f32) -> Vec<Vec<Vec3>> {
    const SAMPLE: f32 = 0.5;
    let mut out = Vec::new();
    let mut run: Vec<Vec3> = Vec::new();
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let d = b - a;
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        for axis in 0..2 {
            let min = centre[axis] - half;
            let max = centre[axis] + half;
            if d[axis].abs() < 1e-8 {
                if a[axis] < min || a[axis] > max { hi = -1.0; break; }
            } else {
                let x = (min - a[axis]) / d[axis];
                let y = (max - a[axis]) / d[axis];
                lo = lo.max(x.min(y));
                hi = hi.min(x.max(y));
            }
        }
        if lo >= hi {
            if run.len() >= 2 { out.push(std::mem::take(&mut run)); }
            run.clear();
            continue;
        }
        let start = a + d * lo;
        let end = a + d * hi;
        if run.last().is_some_and(|p| p.distance_squared(start) > 1e-8) {
            if run.len() >= 2 { out.push(std::mem::take(&mut run)); }
            run.clear();
        }
        if run.is_empty() { run.push(start); }
        let steps = (start.distance(end) / SAMPLE).ceil().max(1.0) as usize;
        for k in 1..=steps { run.push(start.lerp(end, k as f32 / steps as f32)); }
    }
    if run.len() >= 2 { out.push(run); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_straight_track_gives_two_rails_on_top_either_side() {
        let track = vec![Vec3::new(0., 0., 10.), Vec3::new(0., 10., 10.), Vec3::new(0., 20., 10.)];
        let rails = rails_of(&[track], [0., 10.], 50.);
        assert_eq!(rails.len(), 2);
        for r in &rails {
            assert!(r.windows(2).all(|w| w[0].distance(w[1]) <= 0.501));
            assert!(r.iter().all(|p| (p.z - (10.0 + RAIL_TOP)).abs() < 1e-5 && (p.x.abs() - HALF_GAUGE).abs() < 1e-5));
        }
    }

    #[test]
    fn only_the_part_in_the_area_and_no_jumps() {
        let track = vec![Vec3::new(0., 0., 0.), Vec3::new(0., 10., 0.), Vec3::new(0., 20., 0.), Vec3::new(500., 20., 0.), Vec3::new(500., 30., 0.)];
        let rails = rails_of(&[track], [0., 5.], 12.);
        assert_eq!(rails.len(), 2, "{rails:?}");
        assert!(rails.iter().all(|r| r.iter().all(|p| p.y <= 17.0)));
    }

    #[test]
    fn crossing_spans_survive_even_when_neither_node_is_inside() {
        let rails = rails_of(&[vec![Vec3::new(-10.,0.,0.), Vec3::new(10.,0.,0.)]], [0.,0.], 2.);
        assert_eq!(rails.len(), 2);
        for r in rails {
            assert!((r.first().unwrap().x + 2.).abs() < 1e-5);
            assert!((r.last().unwrap().x - 2.).abs() < 1e-5);
            assert!(r.windows(2).all(|w| w[0].distance(w[1]) <= 0.501));
        }
    }

    #[test]
    fn a_remote_track_after_a_gap_does_not_rotate_the_local_rail_end() {
        let rails = rails_of(&[vec![Vec3::ZERO, Vec3::X*10., Vec3::new(500.,500.,0.), Vec3::new(510.,500.,0.)]], [5.,0.], 20.);
        assert_eq!(rails.len(), 2);
        assert!(rails.iter().flatten().all(|p| (p.y.abs()-HALF_GAUGE).abs() < 1e-5));
    }

    #[test]
    fn invalid_nodes_split_runs_without_nonfinite_rails() {
        let rails = rails_of(&[vec![Vec3::ZERO, Vec3::X, Vec3::NAN, Vec3::X*3., Vec3::X*4.]], [0.,0.], 20.);
        assert_eq!(rails.len(), 4);
        assert!(rails.iter().flatten().all(|p| p.is_finite()));
    }

    #[test]
    fn reads_a_trains_dat() {
        let nodes = parse("3\n1084.48 3231.45 39.2565 0 \n1080 3230 39.3 4\nbad line\n");
        assert_eq!(nodes, vec![Vec3::new(1084.48, 3231.45, 39.2565), Vec3::new(1080., 3230., 39.3)]);
    }
}
