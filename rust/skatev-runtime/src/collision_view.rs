//! Developer view of Skate collision and grindable surfaces.
//! While the host has it switched on (`sv_set_collision_view`), installed
//! static collision and host bodies are kept here in GTA space. The host
//! queries nearby classified geometry through `sv_collision_view` to draw it.
//! Nothing is kept while off.
use crate::coords;
use bevy_math::Vec3;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// What a triangle is to Skate (`SvViewTri::kind`).
pub const FLOOR: u32 = 0;
pub const BANK: u32 = 1;
pub const WALL: u32 = 2;
pub const UNDERSIDE: u32 = 3;
/// A curb bevel: collision only, grind probes ignore it.
pub const NO_GRIND: u32 = 4;
/// A host body: vehicles, props, peds' parts, dropped objects before their
/// static rebuild.
pub const MOVING: u32 = 5;
/// A gap-fill floor (GTA's bullet/camera copy or the prebaked map): what the
/// Fallbacks switch removes.
pub const FALLBACK: u32 = 6;
/// From GTA's live collision at play time, not from the baked tiles.
pub const LIVE: u32 = 7;
/// `SvViewLine::kind`: a grind rail.
pub const RAIL: u32 = 0;
/// `SvViewLine::kind`: a skitch grab line (a vehicle's rear bumper).
pub const SKITCH: u32 = 1;

static ON: AtomicBool = AtomicBool::new(false);
/// Switched on since the last publication: publish the installed area.
static WANTED: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct View {
    statics: Vec<([Vec3; 3], u32)>,
    rails: Vec<Vec<Vec3>>,
    /// Host bodies: GTA-space triangles at the sample (for the reach test),
    /// the same in the entity's model space, and its GTA handle (0: none).
    moving: Vec<([Vec3; 3], [Vec3; 3], u32)>,
    skitch: Vec<[Vec3; 2]>,
}

static VIEW: Mutex<Option<View>> = Mutex::new(None);
/// The last query's static triangles: (centre, radius, cap, statics count, result).
/// The host asks every frame; the static map is scanned again only when the
/// centre has moved a metre or a new area came in.
static STATIC_PART: Mutex<Option<(Vec3, f32, usize, usize, Vec<(f32, [Vec3; 3], u32)>)>> = Mutex::new(None);

pub fn set_on(on: bool) {
    let was = ON.swap(on, Ordering::Relaxed);
    if on && !was {
        WANTED.store(true, Ordering::Relaxed);
    }
    if !on {
        *VIEW.lock().unwrap() = None;
        *STATIC_PART.lock().unwrap() = None;
    }
}

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// True once after the view was switched on: capture the installed area.
pub fn take_rebuild() -> bool {
    WANTED.swap(false, Ordering::Relaxed)
}

fn kind(t: &[Vec3; 3], tag: u32) -> u32 {
    if tag & crate::live::FALLBACK_TAG != 0 {
        return FALLBACK;
    }
    if tag & crate::live::LIVE_TAG != 0 {
        return LIVE;
    }
    if tag & skate_host::bridge::NO_GRIND_TAG != 0 {
        return NO_GRIND;
    }
    let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
    // The rail finder's walkable limit (rails.rs UPWARD_Z).
    if n.z >= 0.65 { FLOOR } else if n.z > 0.2 { BANK } else if n.z >= -0.2 { WALL } else { UNDERSIDE }
}

/// A static build (Skate space, as `world::Area`).
pub fn publish_area(triangles: &[[[f32; 3]; 3]], tags: &[u32], rails: &[Vec<[f32; 3]>]) {
    if !on() {
        return;
    }
    let g = |p: [f32; 3]| coords::from_skate(Vec3::from_array(p));
    let statics = triangles.iter().zip(tags).map(|(t, &tag)| {
        let p = t.map(g);
        (p, kind(&p, tag))
    }).collect();
    let rails = rails.iter().map(|r| r.iter().map(|&p| g(p)).collect()).collect();
    *STATIC_PART.lock().unwrap() = None;
    let mut view = VIEW.lock().unwrap();
    let (moving, skitch) = view.take().map(|v| (v.moving, v.skitch)).unwrap_or_default();
    *view = Some(View { statics, rails, moving, skitch });
}

/// Rails currently installed in the session (Skate space).
pub fn publish_rails(rails: &[Vec<[f32; 3]>]) {
    if on() {
        VIEW.lock().unwrap().get_or_insert_with(View::default).rails = rails.iter()
            .map(|r| r.iter().map(|&p| coords::from_skate(Vec3::from_array(p))).collect()).collect();
    }
}

/// The host bodies' triangles (Skate space, as `set_dynamic_surfaces`), each
/// also in its entity's model space (`bodies`: the sample they came from) so
/// the host draws it on the entity as it is this frame, tied to the model.
pub fn publish_bodies(triangles: &[([[f32; 3]; 3], u32, u32)], bodies: &[crate::dynamic::HostBody]) {
    if !on() {
        return;
    }
    let frames: std::collections::HashMap<u32, (Vec3, bevy_math::Mat3)> = bodies
        .iter()
        .filter_map(|b| {
            let m = bevy_math::Mat3::from_cols(b.axes[0], b.axes[1], b.axes[2]);
            (m.determinant().abs() > 1e-6).then(|| (b.fallback.tag | crate::dynamic::DYNAMIC_TAG, (b.position, m.inverse())))
        })
        .collect();
    let moving = triangles
        .iter()
        .map(|(t, tag, _)| {
            let world = t.map(|p| coords::from_skate(Vec3::from_array(p)));
            match frames.get(tag) {
                Some(&(at, inverse)) => (world, world.map(|p| inverse * (p - at)), tag & 0x0FFF_FFFF),
                None => (world, world, 0),
            }
        })
        .collect();
    VIEW.lock().unwrap().get_or_insert_with(View::default).moving = moving;
}

/// The skitch grab lines as Skate holds them this tick.
pub fn publish_skitch(lines: &[skate_host::bridge::GrabLine]) {
    if !on() {
        return;
    }
    let skitch = lines.iter().map(|l| {
        let row = |k: usize| Vec3::new(l.frame[k][0], l.frame[k][1], l.frame[k][2]);
        let at = |p: [f32; 4]| coords::from_skate(row(0) * p[0] + row(1) * p[1] + row(2) * p[2] + row(3));
        [at(l.points[0]), at(l.points[1])]
    }).collect();
    VIEW.lock().unwrap().get_or_insert_with(View::default).skitch = skitch;
}

fn segment_distance_squared(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let d = b - a;
    let length = d.length_squared();
    let t = if length > 0.0 { (p - a).dot(d) / length } else { 0.0 };
    (a + d * t.clamp(0.0, 1.0)).distance_squared(p)
}

fn triangle_distance_squared(p: Vec3, t: &[Vec3; 3]) -> f32 {
    // The usual triangle region calculation divides by the triangle's area;
    // collapsed faces must instead be treated as their remaining edges.
    if (t[1] - t[0]).cross(t[2] - t[0]).length_squared() <= 1e-12 {
        return (0..3).map(|i| segment_distance_squared(p, t[i], t[(i + 1) % 3]))
            .fold(f32::INFINITY, f32::min);
    }
    crate::world::closest_on_triangle(p, t).distance_squared(p)
}

/// Triangles touching the sphere within `radius` of `centre` (moving first, then
/// static, nearest kept when over `cap`), and the rail segments touching it.
pub fn query(centre: Vec3, radius: f32, cap: usize, line_cap: usize) -> (Vec<([Vec3; 3], u32, u32)>, Vec<([Vec3; 2], u32)>) {
    let view = VIEW.lock().unwrap();
    let Some(view) = view.as_ref() else { return (Vec::new(), Vec::new()) };
    let r2 = radius * radius;
    let score = |(t, k): ([Vec3; 3], u32)| (triangle_distance_squared(centre, &t), t, k);
    let line_distance = |s: &[Vec3]| segment_distance_squared(centre, s[0], s[1]);
    let mut kept = STATIC_PART.lock().unwrap();
    let fresh = kept.as_ref().is_some_and(|(c, r, n, count, _)| c.distance(centre) < 1.0 && *r == radius && *n == cap && *count == view.statics.len());
    if !fresh {
        let mut statics: Vec<_> = view.statics.iter().copied().map(score).filter(|(d, _, _)| *d <= r2).collect();
        if statics.len() > cap {
            statics.select_nth_unstable_by(cap, |a, b| a.0.total_cmp(&b.0));
            statics.truncate(cap);
        }
        *kept = Some((centre, radius, cap, view.statics.len(), statics));
    }
    // Host bodies first (in model space with their entity), then the map.
    let mut tris: Vec<(f32, [Vec3; 3], u32, u32)> = view.moving.iter()
        .map(|&(world, local, entity)| { let (d, _, k) = score((world, MOVING)); (d, local, k, entity) })
        .filter(|(d, _, _, _)| *d <= r2)
        .chain(kept.as_ref().map(|k| k.4.iter().map(|&(d, t, kind)| (d, t, kind, 0)).collect::<Vec<_>>()).unwrap_or_default())
        .collect();
    if tris.len() > cap {
        tris.select_nth_unstable_by(cap, |a, b| a.0.total_cmp(&b.0));
        tris.truncate(cap);
    }
    // Reserve capacity for the nearest accepted grab lines before map rails.
    let mut skitch: Vec<_> = view.skitch.iter().copied()
        .filter(|s| line_distance(s) <= r2)
        .collect();
    skitch.sort_by(|a, b| {
        line_distance(a).total_cmp(&line_distance(b))
    });
    let mut lines: Vec<_> = skitch.into_iter().take(line_cap).map(|s| (s, SKITCH)).collect();
    let mut map_lines: Vec<_> = view.rails.iter().flat_map(|r| r.windows(2))
        .map(|w| (line_distance(w), [w[0], w[1]]))
        .filter(|(d, _)| *d <= r2)
        .collect();
    let remaining = line_cap - lines.len();
    if map_lines.len() > remaining {
        map_lines.select_nth_unstable_by(remaining, |a, b| a.0.total_cmp(&b.0));
        map_lines.truncate(remaining);
    }
    lines.extend(map_lines.into_iter().map(|(_, segment)| (segment, RAIL)));
    (tris.into_iter().map(|(_, t, k, e)| (t, k, e)).collect(), lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The view is process-wide state: its tests take turns.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn large_crossing_faces_are_drawn_and_ranked_by_their_surface() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_on(true);
        let wall = [Vec3::new(0.1, -100., -100.), Vec3::new(0.1, 100., -100.), Vec3::new(0.1, 0., 100.)];
        let other = [Vec3::new(0.8, 0., 0.), Vec3::new(0.8, 0.1, 0.), Vec3::new(0.8, 0., 0.1)];
        *VIEW.lock().unwrap() = Some(View { statics: vec![(other, WALL), (wall, WALL)], ..Default::default() });
        let (tris, _) = query(Vec3::ZERO, 1., 1, 0);
        assert_eq!(tris.len(), 1);
        assert_eq!(tris[0].0, wall, "nearest wall interior wins even with its corners and centroid far away");
        set_on(false);
    }

    #[test]
    fn large_moving_faces_use_world_surface_distance_but_return_local_geometry() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_on(true);
        let world = [Vec3::new(0.1, -100., -100.), Vec3::new(0.1, 100., -100.), Vec3::new(0.1, 0., 100.)];
        let local = world.map(|p| p - Vec3::X * 10.);
        *VIEW.lock().unwrap() = Some(View { moving: vec![(world, local, 42)], ..Default::default() });
        let (tris, _) = query(Vec3::ZERO, 1., 1, 0);
        assert_eq!(tris, vec![(local, MOVING, 42)]);
        set_on(false);
    }

    #[test]
    fn crossing_rails_and_skitch_lines_do_not_require_nearby_endpoints() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_on(true);
        let crossing = [Vec3::new(-100., 0.1, 0.), Vec3::new(100., 0.1, 0.)];
        let near_end = [Vec3::new(0.8, 0., 0.), Vec3::new(0.9, 0., 0.)];
        *VIEW.lock().unwrap() = Some(View { rails: vec![near_end.to_vec(), crossing.to_vec()], ..Default::default() });
        assert_eq!(query(Vec3::ZERO, 1., 0, 1).1, vec![(crossing, RAIL)]);
        VIEW.lock().unwrap().as_mut().unwrap().skitch = vec![near_end, crossing];
        assert_eq!(query(Vec3::ZERO, 1., 0, 1).1, vec![(crossing, SKITCH)]);
        set_on(false);
    }

    #[test]
    fn collapsed_faces_and_zero_length_lines_have_finite_distance() {
        let p = Vec3::new(0., 1., 0.);
        assert_eq!(segment_distance_squared(p, Vec3::ZERO, Vec3::ZERO), 1.);
        assert_eq!(triangle_distance_squared(p, &[Vec3::ZERO; 3]), 1.);
        assert_eq!(triangle_distance_squared(p, &[Vec3::ZERO, Vec3::ZERO, Vec3::X]), 1.);
        assert_eq!(triangle_distance_squared(p, &[-Vec3::X, Vec3::X, Vec3::X * 2.]), 1.);
    }

    #[test]
    fn drawing_budget_keeps_nearby_rails_instead_of_file_order() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_on(true);
        let rails: Vec<_> = [8., 1., 5.].into_iter().map(|x| {
            [Vec3::new(x, 0., 0.), Vec3::new(x + 0.1, 0., 0.)].map(|p| coords::to_skate(p).to_array()).to_vec()
        }).collect();
        publish_area(&[], &[], &rails);
        let (_, lines) = query(Vec3::ZERO, 10., 0, 1);
        assert_eq!(lines.len(), 1);
        assert!((lines[0].0[0].x - 1.).abs() < 1e-5);
        set_on(false);
    }

    #[test]
    fn nearest_skitch_lines_have_capacity_and_empty_sets_clear_them() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_on(true);
        let point = |x: f32| coords::to_skate(Vec3::new(x, 0., 0.)).to_array();
        publish_area(&[], &[], &[vec![point(0.), point(1.)]]);
        let line = |x: f32| skate_host::bridge::GrabLine {
            tag: 7, frame: [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.], [0.; 4]],
            velocity: [0.; 4], points: [[x, 1.5, 0., 0.], [x + 0.1, 1.5, 0., 0.]], approach: [0.; 4],
        };
        // These represent the accepted (raised) endpoints returned by the session.
        publish_skitch(&[line(3.), line(1.)]);
        let (_, lines) = query(Vec3::ZERO, 10., 0, 1);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].1, SKITCH);
        assert!((lines[0].0[0] - Vec3::new(1., 0., 1.5)).length() < 1e-5);
        assert!(query(Vec3::ZERO, 10., 0, 0).1.is_empty());
        publish_skitch(&[]);
        let (_, lines) = query(Vec3::ZERO, 10., 0, 1);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].1, RAIL, "empty accepted set removed both grab lines");
        set_on(false);
    }

    #[test]
    fn host_bodies_come_back_in_their_entitys_model_space() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_on(true);
        // An entity 0x2b605 at (2, 3, 0), turned 90 degrees (right = +y, forward = -x).
        let tag = (0u32 << 28) | 0x2b605;
        let body = crate::dynamic::HostBody {
            model: 1, position: Vec3::new(2., 3., 0.), axes: [Vec3::Y, -Vec3::X, Vec3::Z],
            fallback: crate::dynamic::HostBox { tag, center: Vec3::ZERO, rotation: bevy_math::Quat::IDENTITY, half_extents: Vec3::ONE },
            allow_box: false, velocity: None, grab_point: None, fixed: false, vehicle_part: false,
        };
        // Model point (1, 0, 0) is world (2, 4, 0).
        let world = [[2., 4., 0.], [2., 4., 1.], [1., 3., 0.]].map(|p: [f32; 3]| coords::to_skate(Vec3::from_array(p)).to_array());
        publish_bodies(&[(world, tag | crate::dynamic::DYNAMIC_TAG, 0)], &[body]);
        let (tris, _) = query(Vec3::new(2., 3., 0.), 5.0, 100, 100);
        let t = tris.iter().find(|t| t.1 == MOVING).expect("the body's triangle");
        assert_eq!(t.2, 0x2b605, "its entity handle");
        assert!((t.0[0] - Vec3::new(1., 0., 0.)).length() < 1e-5, "model space: {:?}", t.0);
        assert!((t.0[1] - Vec3::new(1., 0., 1.)).length() < 1e-5);
        set_on(false);
    }

    #[test]
    fn kinds_follow_the_surface_and_only_near_parts_are_given() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_on(true);
        let floor = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]].map(|p: [f32; 3]| coords::to_skate(Vec3::from_array(p)).to_array());
        let wall = [[0., 0., 0.], [0., 0., 1.], [1., 0., 0.]].map(|p: [f32; 3]| coords::to_skate(Vec3::from_array(p)).to_array());
        let far = [[50., 0., 0.], [51., 0., 0.], [50., 1., 0.]].map(|p: [f32; 3]| coords::to_skate(Vec3::from_array(p)).to_array());
        let rail = vec![[0.5, 0.5, 0.5], [3., 0.5, 0.5]].into_iter().map(|p: [f32; 3]| coords::to_skate(Vec3::from_array(p)).to_array()).collect();
        publish_area(&[floor, wall, far], &[0, 0, 0], &[rail]);
        let (tris, lines) = query(Vec3::ZERO, 5.0, 100, 100);
        let kinds: Vec<u32> = tris.iter().map(|t| t.1).collect();
        assert!(tris.iter().all(|t| t.2 == 0), "map triangles carry no entity");
        assert_eq!(tris.len(), 2, "the far triangle is left out");
        assert!(kinds.contains(&FLOOR) && kinds.contains(&WALL), "{kinds:?}");
        assert_eq!(lines.len(), 1);
        assert!((lines[0].0[0] - Vec3::splat(0.5)).length() < 1e-4, "rails come back in GTA space");
        set_on(false);
        assert!(query(Vec3::ZERO, 5.0, 100, 100).0.is_empty(), "nothing is kept while off");
    }
}
