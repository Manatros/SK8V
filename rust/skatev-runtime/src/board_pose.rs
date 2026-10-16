//! Native drawable presentation contract. These are authored Skate joints,
//! never inferred from velocity or a host physics object.
use bevy_math::{Mat4, Quat, Vec3, Vec4};
use std::cell::Cell;

pub const NAMES: [&str; 7] = [
    "SKATEBOARD_ROOT", "TRUCK_FRONT", "TRUCK_BACK", "LEFT_WHEELFRONT",
    "LEFT_WHEELBACK", "RIGHT_WHEELFRONT", "RIGHT_WHEELBACK",
];

pub fn world(names: &[String], bones: &[Mat4], root: Mat4) -> Option<[Mat4; 7]> {
    let blender = Mat4::from_cols(Vec4::X, -Vec4::Z, Vec4::Y, Vec4::W);
    let c = Mat4::from_mat3(crate::coords::basis());
    let mut result = [Mat4::IDENTITY; 7];
    for (out, name) in result.iter_mut().zip(NAMES) {
        let index = names.iter().position(|n| n == name)?;
        *out = c * root * *bones.get(index)? * blender * c.transpose();
        if !out.is_finite() { return None; }
    }
    Some(result)
}

/// Presentation only. At riding speed Skate's wheels turn 190-280 degrees per
/// 60 Hz tick, so the drawn wheels alias (they creep backwards). Each wheel
/// keeps Skate's own spin direction and rate (unwrapped across ticks) but is
/// drawn turning at most MAX_SPIN per tick. Physics never reads this.
const MAX_SPIN: f32 = 1.0; // rad per tick; ponytail: by eye, raise if the wheels look slow
// Numerical angle noise only (0.006 degrees), not a movement/speed cutoff.
const ANGLE_NOISE: f32 = 1e-4;
#[derive(Clone, Copy)]
struct Spin { tick: u64, raw: [f32; 4], rate: [f32; 4], shown: [f32; 4] }
thread_local!(static SPIN: Cell<Option<Spin>> = const { Cell::new(None) });

fn wrap(a: f32) -> f32 { (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI }

/// Wheel angle about its own x axis, measured against the authored truck x
/// axis with a stable fallback if that axis is parallel to the axle.
fn wheel_angle(wheel: Mat4, truck: Mat4) -> (Vec3, f32) {
    let axle = wheel.x_axis.truncate().normalize_or_zero();
    // Choosing the smallest dot every frame can alternate between two equally
    // perpendicular axes through float noise, injecting a false quarter turn.
    let reference = [truck.x_axis, truck.y_axis, truck.z_axis]
        .map(|v| v.truncate().normalize_or_zero())
        .into_iter()
        .map(|v| v - axle * v.dot(axle))
        .find(|v| v.length_squared() > 0.25)
        .unwrap_or(Vec3::ZERO)
        .normalize_or_zero();
    let y = wheel.y_axis.truncate();
    (axle, y.dot(axle.cross(reference)).atan2(y.dot(reference)))
}

pub fn present_wheels(world: &mut [Mat4; 7], tick: u64) {
    present_wheels_with_motion(world, tick, None);
}

/// Use Skate's own relative physical angular motion only to disambiguate
/// direction. The existing authored-angle magnitude and visual cap stay intact.
pub fn present_wheels_with_motion(world: &mut [Mat4; 7], tick: u64, angular: Option<[Vec3; 4]>) {
    let measured: [(Vec3, f32); 4] = std::array::from_fn(|w| wheel_angle(world[3 + w], world[1 + w % 2]));
    let raw = measured.map(|m| m.1);
    let spin = SPIN.with(|cell| {
        let next = match cell.get() {
            Some(p) if tick == p.tick => p,
            Some(p) if tick > p.tick && tick - p.tick <= 3 => {
                let n = (tick - p.tick) as f32;
                let mut s = Spin { tick, raw, ..p };
                for w in 0..4 {
                    let d = wrap(raw[w] - p.raw[w]);
                    let expected = p.rate[w] * n;
                    // An unchanged source angle is stopped. History alone
                    // would otherwise prefer TAU after a fast turn and latch
                    // onto full revolutions forever, even through slow starts.
                    let turned = if d.abs() <= ANGLE_NOISE { 0.0 } else {
                        [d - std::f32::consts::TAU, d, d + std::f32::consts::TAU]
                        .into_iter()
                        .min_by(|a, b| (a - expected).abs().total_cmp(&(b - expected).abs()))
                        .unwrap()
                    };
                    s.rate[w] = turned / n;
                    let directed = angular.and_then(|a| {
                        let axial = a[w].dot(measured[w].0);
                        axial.is_finite().then_some(if axial.abs() <= ANGLE_NOISE * 60.0 {
                            0.0
                        } else {
                            turned.abs().copysign(axial)
                        })
                    }).unwrap_or(turned);
                    s.shown[w] = wrap(p.shown[w] + directed.clamp(-MAX_SPIN * n, MAX_SPIN * n));
                }
                s
            }
            _ => Spin { tick, raw, rate: [0.0; 4], shown: raw },
        };
        cell.set(Some(next));
        next
    });
    for w in 0..4 {
        let (axle, angle) = measured[w];
        let r = Quat::from_axis_angle(axle, wrap(spin.shown[w] - angle));
        let m = &mut world[3 + w];
        m.x_axis = (r * m.x_axis.truncate()).extend(0.0);
        m.y_axis = (r * m.y_axis.truncate()).extend(0.0);
        m.z_axis = (r * m.z_axis.truncate()).extend(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_vertex_matches_existing_skin_coordinate_chain() {
        let names = NAMES.map(str::to_string);
        let bones = std::array::from_fn::<_, 7, _>(|i| {
            Mat4::from_rotation_y(i as f32 * 0.21)
                * Mat4::from_translation(Vec3::new(i as f32 * 0.1, 0.2, -0.3))
        });
        let root = Mat4::from_translation(Vec3::new(-1300., 24., 1400.))
            * Mat4::from_rotation_y(1.2);
        let actual = world(&names, &bones, root).unwrap();
        let c = Mat4::from_mat3(crate::coords::basis());
        let blender = Mat4::from_cols(Vec4::X, -Vec4::Z, Vec4::Y, Vec4::W);
        let inverse_bind = Mat4::from_rotation_z(0.8)
            * Mat4::from_translation(Vec3::new(0.1, 0.03, -0.1));
        let vertex = Vec3::new(0.08, -0.04, 0.3);
        for i in 0..7 {
            let expected = crate::coords::from_skate(
                (root * bones[i] * blender * inverse_bind).transform_point3(vertex));
            let converted = (actual[i] * c * inverse_bind * c.transpose())
                .transform_point3(c.transform_point3(vertex));
            assert!(expected.distance(converted) < 0.0003);
        }
    }

    #[test]
    fn fast_wheels_keep_their_direction_and_slow_ones_their_angle() {
        let run = |per_tick: f32, ticks: u64| {
            SPIN.with(|c| c.set(None));
            let mut shown = 0.0;
            for t in 0..ticks {
                // Skate's rate ramps up from rest, as from a push.
                let angle = per_tick * (t * t) as f32 / (2 * ticks) as f32;
                let mut w = [Mat4::from_rotation_z(1.5707964); 7];
                for i in 3..7 { w[i] = Mat4::from_rotation_x(angle); }
                let skate = wheel_angle(w[3], w[1]).1;
                present_wheels(&mut w, 100 + t);
                shown = wrap(wheel_angle(w[3], w[1]).1 - skate);
            }
            shown
        };
        // Ending at 0.2 rad/tick: drawn exactly as Skate has it.
        let slow = run(0.2, 60);
        assert!(slow.abs() < 1e-3, "{slow}");
        // Ending at 3.3 rad/tick (aliases to -3.0): still forward, capped.
        SPIN.with(|c| c.set(None));
        let mut last = 0.0f32;
        for t in 0..120u64 {
            let rate = 3.3 * t as f32 / 119.0;
            last += rate;
            let mut w = [Mat4::IDENTITY; 7];
            for i in 3..7 { w[i] = Mat4::from_rotation_x(last); }
            present_wheels(&mut w, 500 + t);
        }
        let s = SPIN.with(|c| c.get()).unwrap();
        assert!((s.rate[0] - 3.3).abs() < 1e-3, "rate {}", s.rate[0]);
        let before = s.shown[0];
        let mut w = [Mat4::IDENTITY; 7];
        for i in 3..7 { w[i] = Mat4::from_rotation_x(last + 3.3); }
        present_wheels(&mut w, 620);
        assert!((wrap(SPIN.with(|c| c.get()).unwrap().shown[0] - before) - MAX_SPIN).abs() < 1e-3);
        // The drawn wheel stays a rotation about its own axle.
        assert!((w[3].x_axis.truncate() - Vec3::X).length() < 1e-4);
    }

    #[test]
    fn missing_or_invalid_bones_fail_closed() {
        let names = NAMES.map(str::to_string);
        let mut bones = [Mat4::IDENTITY; 7];
        assert!(world(&names[..6], &bones, Mat4::IDENTITY).is_none());
        assert!(world(&names, &bones[..6], Mat4::IDENTITY).is_none());
        bones[4].w_axis.x = f32::NAN;
        assert!(world(&names, &bones, Mat4::IDENTITY).is_none());
    }

    #[test]
    fn fast_wheels_stop_and_resume_at_the_authored_slow_rate() {
        for direction in [-1.0, 1.0] {
            for stride in 1..=3 {
                SPIN.with(|c| c.set(None));
                let mut angle = 0.0;
                let mut tick = 1000;
                // Reach an unwrapped rate beyond PI, as in the existing
                // forward/backward high-speed presentation contract.
                for t in 0..120 {
                    angle += direction * 3.3 * t as f32 / 119.0;
                    let mut w = [Mat4::IDENTITY; 7];
                    for wheel in &mut w[3..] { *wheel = Mat4::from_rotation_x(angle); }
                    present_wheels(&mut w, tick);
                    tick += 1;
                }
                let stopped = SPIN.with(|c| c.get()).unwrap().shown;
                for _ in 0..20 {
                    let mut w = [Mat4::IDENTITY; 7];
                    for wheel in &mut w[3..] { *wheel = Mat4::from_rotation_x(angle); }
                    present_wheels(&mut w, tick);
                    let s = SPIN.with(|c| c.get()).unwrap();
                    for i in 0..4 {
                        assert!(s.rate[i].abs() < 1e-4, "stationary rate {}, direction {direction}, stride {stride}", s.rate[i]);
                        assert!(wrap(s.shown[i] - stopped[i]).abs() < 1e-4);
                    }
                    tick += stride;
                }
                let before = SPIN.with(|c| c.get()).unwrap().shown;
                angle += direction * 0.1 * stride as f32;
                let mut w = [Mat4::IDENTITY; 7];
                for wheel in &mut w[3..] { *wheel = Mat4::from_rotation_x(angle); }
                present_wheels(&mut w, tick);
                let s = SPIN.with(|c| c.get()).unwrap();
                for i in 0..4 {
                    assert!((s.rate[i] - direction * 0.1).abs() < 1e-4);
                    assert!((wrap(s.shown[i] - before[i]) - direction * 0.1 * stride as f32).abs() < 1e-4);
                }
            }
        }
    }

    #[test]
    fn wheel_angle_is_invariant_under_whole_board_rotation() {
        let wheel = Mat4::from_rotation_x(0.37);
        let (_, expected) = wheel_angle(wheel, Mat4::IDENTITY);
        for i in 0..100 {
            let world = Mat4::from_rotation_z(i as f32 * 0.017)
                * Mat4::from_rotation_y(i as f32 * 0.031)
                * Mat4::from_rotation_x(i as f32 * 0.047);
            let (_, angle) = wheel_angle(world * wheel, world);
            assert!(wrap(angle - expected).abs() < 1e-4, "stationary relative angle changed at {i}: {angle} vs {expected}");
        }
    }

    #[test]
    fn tiny_axle_tilts_do_not_change_the_reference_by_a_quarter_turn() {
        let (_, a) = wheel_angle(Mat4::from_rotation_z(1e-5) * Mat4::from_rotation_y(2e-5), Mat4::IDENTITY);
        let (_, b) = wheel_angle(Mat4::from_rotation_z(2e-5) * Mat4::from_rotation_y(1e-5), Mat4::IDENTITY);
        assert!(wrap(a - b).abs() < ANGLE_NOISE, "false wheel spin from axle tilt: {a} vs {b}");
    }

    fn direction_trace(with_motion: bool, per_tick: f32, stride: u64, mirrored_rear: bool) -> Vec<[f32; 4]> {
        SPIN.with(|c| c.set(None));
        let whole = Mat4::from_rotation_z(0.41) * Mat4::from_rotation_y(-0.23);
        let world_axle = whole.transform_vector3(Vec3::X);
        let mut previous: Option<[f32; 4]> = None;
        let mut output = Vec::new();
        for t in 0..12 {
            let phase = per_tick * (t * stride) as f32;
            let mut w = [whole; 7];
            w[1] = whole * Mat4::from_rotation_y(-std::f32::consts::FRAC_PI_2);
            w[2] = whole * Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
            for i in 0..4 {
                let rear = i % 2 == 1;
                let bind = if rear && mirrored_rear { Mat4::from_rotation_y(std::f32::consts::PI) } else { Mat4::IDENTITY };
                let source_phase = if rear && !mirrored_rear { -phase } else { phase };
                w[3 + i] = whole * bind * Mat4::from_rotation_x(source_phase);
            }
            let motion = with_motion.then_some([world_axle * per_tick * 60.0; 4]);
            present_wheels_with_motion(&mut w, 2000 + t * stride, motion);
            let shown = SPIN.with(|c| c.get()).unwrap().shown;
            if let Some(last) = previous {
                output.push(std::array::from_fn(|i| {
                    let axle = w[3 + i].x_axis.truncate().normalize_or_zero();
                    wrap(shown[i] - last[i]) * axle.dot(world_axle)
                }));
            }
            previous = Some(shown);
        }
        output
    }

    #[test]
    fn physical_direction_resolves_rear_phase_and_mirrors_without_changing_speed() {
        for per_tick in [-3.3, -0.15, 0.15, 3.3] {
            for stride in 1..=3 {
                for mirrored in [false, true] {
                    let before = direction_trace(false, per_tick, stride, mirrored);
                    let after = direction_trace(true, per_tick, stride, mirrored);
                    for (old, new) in before.iter().zip(after.iter()) {
                        for i in 0..4 {
                            assert!(new[i] * per_tick > 0.0, "wheel {i} reversed: {new:?}; rate {per_tick}, stride {stride}, mirrored {mirrored}");
                            assert!((old[i].abs() - new[i].abs()).abs() < 1e-4, "accepted speed changed: {old:?} -> {new:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn physical_stop_and_reverse_do_not_change_raw_angle_magnitude() {
        SPIN.with(|c| c.set(None));
        let mut previous = [0.0; 4];
        for t in 0..8 {
            let mut w = [Mat4::IDENTITY; 7];
            for wheel in &mut w[3..] { *wheel = Mat4::from_rotation_x(t as f32 * 0.2); }
            let velocity = if t < 3 { Vec3::X * 12.0 } else if t < 5 { Vec3::ZERO } else { -Vec3::X * 12.0 };
            present_wheels_with_motion(&mut w, 3000 + t, Some([velocity; 4]));
            let shown = SPIN.with(|c| c.get()).unwrap().shown;
            if t > 0 {
                for i in 0..4 {
                    let delta = wrap(shown[i] - previous[i]);
                    let expected = if t < 3 { 0.2 } else if t < 5 { 0.0 } else { -0.2 };
                    assert!((delta - expected).abs() < 1e-4);
                }
            }
            previous = shown;
        }
    }

    #[test]
    fn invalid_physical_direction_falls_back_to_authored_presentation() {
        let with_hint = |angular| {
            SPIN.with(|c| c.set(None));
            for t in 0..4 {
                let mut w = [Mat4::IDENTITY; 7];
                for wheel in &mut w[3..] { *wheel = Mat4::from_rotation_x(t as f32 * 0.2); }
                present_wheels_with_motion(&mut w, 4000 + t, angular);
            }
            SPIN.with(|c| c.get()).unwrap().shown
        };
        assert_eq!(with_hint(None), with_hint(Some([Vec3::splat(f32::NAN); 4])));
    }
}
