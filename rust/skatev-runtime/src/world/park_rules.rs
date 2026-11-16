//! Park-only cleanup (design 2026-11-16, BACKLOG B137.4; A49/A50/A52).
use bevy_math::Vec3;
use std::cell::RefCell;

#[derive(Default)]
struct Audit {
    active: bool,
    events: Vec<Event>,
    tracks: Vec<Vec<Vec3>>,
}
struct Event {
    rule: &'static str,
    lines: Vec<Vec<Vec3>>,
    faces: Vec<[[f32; 3]; 3]>,
    boxes: usize,
    spot: bool,
}
thread_local! { static AUDIT: RefCell<Audit> = RefCell::new(Audit::default()); }
pub fn begin() {
    AUDIT.with(|a| {
        *a.borrow_mut() = Audit {
            active: true,
            ..Default::default()
        }
    });
}
/// Capture actual main-pass train edges, including orphan_steel without nodes.
pub(crate) fn remember_tracks(tracks: &[Vec<Vec3>]) {
    AUDIT.with(|a| {
        let mut a = a.borrow_mut();
        if a.active {
            a.tracks = tracks.to_vec();
        }
    });
}
pub fn kept_tracks() -> Vec<Vec<Vec3>> {
    AUDIT.with(|a| a.borrow().tracks.clone())
}
pub fn recording(on: bool) {
    AUDIT.with(|a| a.borrow_mut().active = on);
}
pub(crate) fn record(
    rule: &'static str,
    lines: Vec<Vec<Vec3>>,
    faces: Vec<[[f32; 3]; 3]>,
    boxes: usize,
    spot: bool,
) {
    AUDIT.with(|a| {
        let mut a = a.borrow_mut();
        if a.active {
            a.events.push(Event {
                rule,
                lines,
                faces,
                boxes,
                spot,
            });
        }
    });
}
pub fn report(
    cell: crate::tiles::Cell,
    size: f32,
    origins: &std::collections::HashMap<u64, u32>,
    source_names: &[String],
) {
    let events = AUDIT.with(|a| std::mem::take(&mut a.borrow_mut().events));
    for rule in ["R1", "R1-spot", "R2", "R3a", "R3b"] {
        let (mut lines, mut metres, mut boxes, mut faces, mut outside) = (0, 0.0f32, 0, 0, 0);
        let mut names = std::collections::BTreeSet::new();
        for e in events.iter().filter(|e| e.rule == rule) {
            for r in &e.lines {
                let clipped = crate::tiles::split_rail(
                    &r.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
                    size,
                )
                .into_iter()
                .filter(|(c, _)| *c == cell)
                .map(|(_, r)| r)
                .collect::<Vec<_>>();
                if !clipped.is_empty() {
                    lines += 1;
                }
                metres += clipped
                    .iter()
                    .flat_map(|r| r.windows(2))
                    .map(|w| Vec3::from_array(w[0]).distance(Vec3::from_array(w[1])))
                    .sum::<f32>();
                outside +=
                    usize::from(!e.spot && r.iter().any(|p| !crate::parks::in_park(p.truncate())));
            }
            if let Some(t) = e.faces.first() {
                let mid = e
                    .faces
                    .iter()
                    .flatten()
                    .map(|p| Vec3::from_array(*p))
                    .sum::<Vec3>()
                    / (e.faces.len() * 3) as f32;
                if crate::tiles::cell(mid.to_array(), size) == cell {
                    boxes += e.boxes;
                }
                let _ = t;
            }
            for t in &e.faces {
                if crate::tiles::clip_triangle(*t, cell, size).is_empty() {
                    continue;
                }
                faces += 1;
                if let Some(n) = origins.get(&svwc::surfaces::key(t)) {
                    names.insert(source_names[*n as usize].as_str());
                }
                let mid = t.iter().map(|p| Vec3::from_array(*p)).sum::<Vec3>() / 3.;
                outside += usize::from(!e.spot && !crate::parks::in_park(mid.truncate()));
            }
        }
        eprintln!(
            "PARK-RULE tile {},{} {rule} lines {lines} metres {metres:.3} boxes {boxes} faces {faces} outside {outside} origins {:?}",
            cell.0, cell.1, names
        );
    }
}
pub(crate) fn enabled(rule: &str) -> bool {
    std::env::var_os(format!("SKATEV_{rule}_OFF")).is_none()
}
pub(crate) fn length(r: &[Vec3]) -> f32 {
    r.windows(2).map(|w| w[0].distance(w[1])).sum()
}
fn samples(r: &[Vec3], step: f32) -> Vec<(Vec3, Vec3, f32)> {
    r.windows(2)
        .flat_map(|w| {
            let len = w[0].distance(w[1]);
            let n = (len / step).ceil().max(1.) as usize;
            (0..n).map(move |k| {
                (
                    w[0].lerp(w[1], (k as f32 + 0.5) / n as f32),
                    (w[1] - w[0]).normalize_or_zero(),
                    len / n as f32,
                )
            })
        })
        .collect()
}
fn on_lines(p: Vec3, dir: Vec3, lines: &[Vec<Vec3>], distance: f32, cos: f32) -> bool {
    lines.iter().flat_map(|r| r.windows(2)).any(|w| {
        let ab = w[1] - w[0];
        ab.normalize_or_zero().dot(dir).abs() >= cos
            && p.distance(
                w[0] + ab * ((p - w[0]).dot(ab) / ab.length_squared().max(1e-12)).clamp(0., 1.),
            ) <= distance
    })
}
struct Segments {
    edges: Vec<(Vec3, Vec3)>,
    grid: crate::grid::XyGrid,
}
impl Segments {
    fn new(lines: &[Vec<Vec3>], pad: f32) -> Self {
        let edges: Vec<_> = lines
            .iter()
            .flat_map(|r| r.windows(2).map(|w| (w[0], w[1])))
            .collect();
        let mut grid = crate::grid::XyGrid::new(1.0);
        for (i, (a, b)) in edges.iter().enumerate() {
            grid.insert(
                i as u32,
                a.min(*b) - Vec3::splat(pad),
                a.max(*b) + Vec3::splat(pad),
            );
        }
        Self { edges, grid }
    }
    fn near(&self, p: Vec3, dir: Vec3, distance: f32, cos: f32) -> bool {
        self.grid.at(p.x, p.y).iter().any(|i| {
            let (a, b) = self.edges[*i as usize];
            let ab = b - a;
            ab.normalize_or_zero().dot(dir).abs() >= cos
                && p.distance(
                    a + ab * ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0., 1.),
                ) <= distance
        })
    }
}
fn inside(r: &[Vec3]) -> bool {
    !r.is_empty() && r.iter().all(|p| crate::parks::in_park(p.truncate()))
}
/// Every sample must have a visible floor below; lack of evidence keeps the tube.
pub(crate) fn floating(
    r: &[Vec3],
    visible: &[[[f32; 3]; 3]],
    bars: &[Vec<Vec3>],
    train: bool,
) -> bool {
    if train || !inside(r) || r.len() < 2 {
        return false;
    }
    let ss = samples(r, 0.10);
    let mut supported = 0.0;
    let mut total = 0.0;
    for (p, dir, weight) in ss {
        let floor = visible
            .iter()
            .filter_map(|t| {
                let v = t.map(Vec3::from_array);
                let n = (v[1] - v[0]).cross(v[2] - v[0]);
                if n.z <= 0.2 * n.length() {
                    return None;
                }
                svwc::clean::floor_at(&svwc::Tri { v: *t, material: 0 }, [p.x, p.y])
                    .filter(|z| *z <= p.z + 0.001)
            })
            .max_by(f32::total_cmp);
        if !floor.is_some_and(|z| p.z - z > 0.20) {
            return false;
        }
        total += weight;
        if on_lines(p, dir, bars, 0.30, 0.0) {
            supported += weight;
        }
    }
    // Explicit endpoints too: a tube resting on a slope at either end stays.
    for p in [r[0], *r.last().unwrap()] {
        if !visible
            .iter()
            .filter_map(|t| svwc::clean::floor_at(&svwc::Tri { v: *t, material: 0 }, [p.x, p.y]))
            .filter(|z| *z <= p.z + 0.001)
            .max_by(f32::total_cmp)
            .is_some_and(|z| p.z - z > 0.20)
        {
            return false;
        }
    }
    total > 0. && supported < 0.80 * total
}
pub(crate) fn drop_floating(
    lines: &mut Vec<Vec<Vec3>>,
    visible: &[[[f32; 3]; 3]],
    bars: &[Vec<Vec3>],
    tracks: &[Vec<Vec3>],
) -> Vec<Vec<Vec3>> {
    let mut dropped = Vec::new();
    if enabled("R2") {
        lines.retain(|r| {
            let train = samples(r, 0.1)
                .iter()
                .any(|(p, d, _)| on_lines(*p, *d, tracks, 0.30, 0.0));
            if floating(r, visible, bars, train) {
                dropped.push(r.clone());
                false
            } else {
                true
            }
        });
    }
    dropped
}
/// Cross-source only. Kept visible points are never moved or averaged.
pub(crate) fn prefer_visible(gta: &mut Vec<Vec<Vec3>>, visual: &[Vec<Vec3>]) -> Vec<Vec<Vec3>> {
    let mut removed = Vec::new();
    let mut tails = Vec::new();
    let visual = Segments::new(visual, 0.06);
    if !enabled("R3a") {
        return removed;
    }
    gta.retain(|r| {
        if !inside(r) {
            return true;
        }
        let ss = samples(r, 0.025);
        let total = length(r);
        let hit = ss
            .iter()
            .filter(|(p, d, _)| visual.near(*p, *d, 0.06, 5f32.to_radians().cos()))
            .map(|s| s.2)
            .sum::<f32>();
        if total <= 0. || hit < 0.8 * total {
            return true;
        }
        // Same uncovered-tail policy as dedupe_within, in metres (>=35 cm).
        let first_tail = tails.len();
        let mut run = Vec::new();
        let flush = |run: &mut Vec<Vec3>, tails: &mut Vec<Vec<Vec3>>| {
            if length(run) >= 0.35 {
                tails.push(std::mem::take(run));
            } else {
                run.clear();
            }
        };
        for w in r.windows(2) {
            let n = (w[0].distance(w[1]) / 0.01).ceil().max(1.) as usize;
            for k in 0..=n {
                let p = w[0].lerp(w[1], k as f32 / n as f32);
                if visual.near(
                    p,
                    (w[1] - w[0]).normalize_or_zero(),
                    0.06,
                    5f32.to_radians().cos(),
                ) {
                    flush(&mut run, &mut tails);
                } else if run.last() != Some(&p) {
                    run.push(p);
                }
            }
        }
        flush(&mut run, &mut tails);
        let kept = Segments::new(&tails[first_tail..], 0.002);
        let mut gone = Vec::new();
        for w in r.windows(2) {
            let n = (w[0].distance(w[1]) / 0.01).ceil().max(1.) as usize;
            for k in 0..n {
                let a = w[0].lerp(w[1], k as f32 / n as f32);
                let b = w[0].lerp(w[1], (k + 1) as f32 / n as f32);
                if kept.near((a + b) * 0.5, (b - a).normalize_or_zero(), 0.002, 0.) {
                    if gone.len() > 1 {
                        removed.push(std::mem::take(&mut gone));
                    } else {
                        gone.clear();
                    }
                } else {
                    if gone.last() != Some(&a) {
                        gone.push(a);
                    }
                    gone.push(b);
                }
            }
        }
        if gone.len() > 1 {
            removed.push(gone);
        }
        false
    });
    gta.extend(tails);
    removed
}
/// The finder also sees visible park faces. Preserve their source before the
/// named found/visual merge; visrails alone contains bars, not park coping.
/// Exact (2 mm) edge witnesses keep an 8 mm GTA neighbour in the GTA list.
pub(crate) fn take_visible_finder(
    found: &mut Vec<Vec<Vec3>>,
    tris: &[[Vec3; 3]],
    tags: &[u32],
) -> Vec<Vec<Vec3>> {
    let mesh: Vec<Vec<Vec3>> = tris
        .iter()
        .zip(tags)
        .filter(|(_, tag)| {
            **tag & crate::live::PARKMESH_TAG != 0 && **tag & crate::live::PARKCUT_TAG == 0
        })
        .flat_map(|(t, _)| {
            (0..3)
                .map(|k| vec![t[k], t[(k + 1) % 3]])
                .collect::<Vec<_>>()
        })
        .collect();
    let mut visual = Vec::new();
    if !enabled("R3a") || mesh.is_empty() {
        return visual;
    }
    let mesh = Segments::new(&mesh, 0.002);
    found.retain(|r| {
        let ss = samples(r, 0.05);
        let owned = inside(r)
            && !ss.is_empty()
            && ss
                .iter()
                .all(|(p, d, _)| mesh.near(*p, *d, 0.002, 5f32.to_radians().cos()));
        if owned {
            visual.push(r.clone());
        }
        !owned
    });
    visual
}

/// Visible-model edge map, never collision folds. Boundary/non-manifold edges stay.
pub(crate) fn shallow_creases(visible: &[[[f32; 3]; 3]]) -> Vec<Vec<Vec3>> {
    let key = |p: [f32; 3]| p.map(|x| (x * 1000.).round() as i64);
    let mut by_edge: std::collections::BTreeMap<_, Vec<(Vec3, Vec3, Vec3)>> = Default::default();
    for t in visible {
        let v = t.map(Vec3::from_array);
        let n = (v[1] - v[0]).cross(v[2] - v[0]).normalize_or_zero();
        for k in 0..3 {
            let (a, b) = (key(t[k]), key(t[(k + 1) % 3]));
            by_edge
                .entry(if a < b { (a, b) } else { (b, a) })
                .or_default()
                .push((v[k], v[(k + 1) % 3], n));
        }
    }
    by_edge
        .values()
        .filter(|v| v.len() == 2 && v[0].2.dot(v[1].2) > 20f32.to_radians().cos())
        .map(|v| vec![v[0].0, v[0].1])
        .collect()
}
pub(crate) fn drop_creases(
    lines: &mut Vec<Vec<Vec3>>,
    visible: &[[[f32; 3]; 3]],
    tracks: &[Vec<Vec3>],
) -> Vec<Vec<Vec3>> {
    drop_creases_with_distance(lines, visible, tracks, 0.30)
}
fn drop_creases_with_distance(
    lines: &mut Vec<Vec<Vec3>>,
    visible: &[[[f32; 3]; 3]],
    tracks: &[Vec<Vec3>],
    protected_distance: f32,
) -> Vec<Vec<Vec3>> {
    if !enabled("R3b") {
        return Vec::new();
    }
    let creases = Segments::new(&shallow_creases(visible), 0.025);
    let ends: Vec<_> = lines
        .iter()
        .enumerate()
        .flat_map(|(i, r)| {
            [r.first(), r.last()]
                .into_iter()
                .flatten()
                .map(move |p| (i, *p))
        })
        .collect();
    let mut i = 0;
    let mut removed = Vec::new();
    lines.retain(|r| {
        let index = i;
        i += 1;
        if r.len() < 2 || length(r) >= 1.0 || !inside(r) {
            return true;
        }
        if [r[0], *r.last().unwrap()].iter().any(|p| {
            ends.iter()
                .any(|(j, q)| *j != index && p.distance(*q) <= 0.10)
        }) {
            return true;
        }
        let ss = samples(r, 0.025);
        let drop = !ss.is_empty()
            && ss
                .iter()
                .all(|(p, d, _)| creases.near(*p, *d, 0.025, 5f32.to_radians().cos()))
            && !ss
                .iter()
                .any(|(p, d, _)| on_lines(*p, *d, tracks, protected_distance, 0.0));
        if drop {
            removed.push(r.clone());
        }
        !drop
    });
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3::new(-1374. + x, -1396. + y, z)
    }
    fn line(y: f32, z: f32, len: f32) -> Vec<Vec3> {
        vec![at(0., y, z), at(len, y, z)]
    }
    fn deck() -> Vec<[[f32; 3]; 3]> {
        vec![
            [
                at(-2., -2., 0.).to_array(),
                at(10., -2., 0.).to_array(),
                at(10., 2., 0.).to_array(),
            ],
            [
                at(-2., -2., 0.).to_array(),
                at(10., 2., 0.).to_array(),
                at(-2., 2., 0.).to_array(),
            ],
        ]
    }
    #[test]
    fn r2_tube_38cm_above_visible_deck_goes() {
        assert!(floating(&line(0., 0.38, 5.), &deck(), &[], false));
    }
    #[test]
    fn r2_tube_on_deck_stays() {
        assert!(!floating(&line(0., 0.1, 5.), &deck(), &[], false));
    }
    #[test]
    fn r2_visible_rail_10cm_away_keeps_tube() {
        assert!(!floating(
            &line(0., 0.38, 5.),
            &deck(),
            &[line(0.1, 0.38, 5.)],
            false
        ));
    }
    #[test]
    fn r2_train_line_stays() {
        assert!(!floating(&line(0., 0.38, 5.), &deck(), &[], true));
    }
    #[test]
    fn r2_missing_floor_or_one_supported_end_keeps_tube() {
        assert!(!floating(&line(0., 0.38, 5.), &[], &[], false));
        let mut r = line(0., 0.38, 5.);
        r[0].z = 0.1;
        assert!(!floating(&r, &deck(), &[], false));
    }
    #[test]
    fn r3a_8mm_gta_duplicate_goes_without_moving_visible() {
        let v = vec![line(0., 1., 5.)];
        let before = v.clone();
        let mut gta = vec![line(0.008, 1., 5.)];
        assert_eq!(prefer_visible(&mut gta, &v).len(), 1);
        assert!(gta.is_empty());
        assert_eq!(v, before);
    }
    #[test]
    fn r3a_audit_excludes_the_retained_unique_tail() {
        let mut gta = vec![line(0.008, 1., 6.)];
        let gone = prefer_visible(&mut gta, &[line(0., 1., 5.)]);
        assert!(
            (gone.iter().map(|r| length(r)).sum::<f32>()
                + gta.iter().map(|r| length(r)).sum::<f32>()
                - 6.)
                .abs()
                < 0.02
        );
    }
    #[test]
    fn r3a_visible_edge_witness_keeps_source_lists_separate() {
        let t = deck();
        let tris: Vec<_> = t.iter().map(|t| t.map(Vec3::from_array)).collect();
        let exact = vec![at(-2., -2., 0.), at(10., -2., 0.)];
        let offset: Vec<_> = exact.iter().map(|p| *p + Vec3::Y * 0.008).collect();
        let mut found = vec![exact.clone(), offset.clone()];
        let visual = take_visible_finder(&mut found, &tris, &[crate::live::PARKMESH_TAG; 2]);
        assert_eq!(visual, vec![exact]);
        assert_eq!(found, vec![offset]);
    }
    #[test]
    fn r3a_same_source_twin_curbs_52mm_apart_stay() {
        let mut gta = vec![line(0., 1., 5.), line(0.052, 1., 5.)];
        let before = gta.clone();
        prefer_visible(&mut gta, &[]);
        assert_eq!(gta, before);
    }
    #[test]
    fn r3a_uncovered_tail_and_crossing_line_stay() {
        let mut gta = vec![line(0.008, 1., 6.)];
        prefer_visible(&mut gta, &[line(0., 1., 5.)]);
        assert_eq!(gta.len(), 1);
        assert!(length(&gta[0]) >= 0.85);
        let mut crossing = vec![vec![at(2., -1., 1.), at(2., 1., 1.)]];
        prefer_visible(&mut crossing, &[line(0., 1., 5.)]);
        assert_eq!(crossing.len(), 1);
    }
    fn crease(steep: bool) -> Vec<[[f32; 3]; 3]> {
        vec![
            [
                at(0., 0., 1.).to_array(),
                at(0.71, 0., 1.).to_array(),
                at(0., 1., 1.).to_array(),
            ],
            [
                at(0.71, 0., 1.).to_array(),
                at(0., 0., 1.).to_array(),
                at(
                    0.,
                    if steep { 0. } else { -1. },
                    if steep { 2. } else { 1. },
                )
                .to_array(),
            ],
        ]
    }
    #[test]
    fn r3b_71cm_flat_crease_goes() {
        let mut r = vec![line(0., 1., 0.71)];
        assert_eq!(drop_creases(&mut r, &crease(false), &[]).len(), 1);
        assert!(r.is_empty());
    }
    #[test]
    fn r3b_71cm_right_angle_edge_stays() {
        let mut r = vec![line(0., 1., 0.71)];
        drop_creases(&mut r, &crease(true), &[]);
        assert_eq!(r.len(), 1);
    }
    #[test]
    fn r3b_connected_end_and_train_line_stay() {
        let mut r = vec![line(0., 1., 0.71), vec![at(0.71, 0., 1.), at(0.71, 1., 1.)]];
        drop_creases(&mut r, &crease(false), &[]);
        assert_eq!(r.len(), 2);
        let mut r = vec![line(0., 1., 0.71)];
        drop_creases(&mut r, &crease(false), &[line(0., 1., 0.71)]);
        assert_eq!(r.len(), 1);
    }
    #[test]
    fn bake_audit_preserves_actual_orphan_tracks_across_auxiliary_builds() {
        begin();
        let actual = vec![line(0., 1., 0.71)];
        remember_tracks(&actual);
        recording(false);
        remember_tracks(&[line(1., 1., 1.)]);
        assert_eq!(
            kept_tracks(),
            actual,
            "auxiliary builds cannot replace the main train/orphan source witnesses"
        );
        let mut lines = actual.clone();
        drop_creases(&mut lines, &crease(false), &kept_tracks());
        assert_eq!(lines, actual);
    }
    #[test]
    fn universal_rules_do_nothing_outside_park_areas() {
        let r = vec![Vec3::new(0., 0., 1.), Vec3::new(1., 0., 1.)];
        assert!(!floating(&r, &deck(), &[], false));
        let mut lines = vec![r.clone()];
        prefer_visible(&mut lines, &[r.clone()]);
        drop_creases(&mut lines, &deck(), &[]);
        assert_eq!(lines, vec![r]);
    }
}
