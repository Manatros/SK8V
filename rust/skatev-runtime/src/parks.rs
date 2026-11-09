//! GTA V's skateparks (the old SkateGTA plugin's list, found in the map's
//! data). Their surfaces are GTA's walking collision like everywhere else:
//! GTA keeps no finer floors or steps for them (its `hi@` copy at Vespucci
//! holds only the coping tubes, rails and posts, 2026-10-20 bake), so a
//! surface swap there only doubled or holed the bowls. Their tubes become
//! rails by the general rule (`tubes.rs`).

use bevy_math::Vec2;

pub struct Park {
    pub id: &'static str,
    pub centre: Vec2,
    /// The park's reach round the centre, metres.
    pub radius: f32,
    /// Height of its riding level (m).
    pub floor: f32,
}

pub const PARKS: &[Park] = &[
    Park { id: "vespucci", centre: Vec2::new(-1374., -1396.), radius: 60., floor: 2.5 },
    Park { id: "hawick", centre: Vec2::new(280., -206.), radius: 30., floor: 60.5 },
    Park { id: "littleseoul", centre: Vec2::new(-950., -786.), radius: 30., floor: 15. },
    Park { id: "strawberry", centre: Vec2::new(-266., -1571.), radius: 30., floor: 31. },
    Park { id: "lamesa", centre: Vec2::new(718., -1231.), radius: 30., floor: 24. },
];

/// The id of the park `x`, `y` (GTA metres) lies in.
pub fn park_at(x: f32, y: f32) -> Option<&'static str> {
    PARKS.iter().find(|k| k.centre.distance(Vec2::new(x, y)) <= k.radius).map(|k| k.id)
}

/// Whether `p` (GTA metres) lies in one of `PARKS`.
pub fn in_park(p: Vec2) -> bool {
    PARKS.iter().any(|k| k.centre.distance(p) <= k.radius)
}

/// Whether `p` is at most `margin` outside a park's circle.
pub fn near_park(p: Vec2, margin: f32) -> bool {
    PARKS.iter().any(|k| k.centre.distance(p) <= k.radius + margin)
}

/// Whether `p` stands on or just above a park's riding level: its floor, up to 3 m
/// (a ramp's foot); not the roofs and flats around the parks.
pub fn at_park_floor(p: bevy_math::Vec3) -> bool {
    PARKS.iter().any(|k| k.centre.distance(p.truncate()) <= k.radius && p.z >= k.floor - 1.0 && p.z <= k.floor + 3.0)
}

#[cfg(test)]
mod floor_tests {
    use super::*;

    #[test]
    fn a_park_floor_is_its_riding_level_inside_its_circle() {
        let v = bevy_math::Vec3::new;
        assert!(at_park_floor(v(285.4, -214.6, 60.7)), "Hawick's ramp foot");
        assert!(!at_park_floor(v(285.4, -214.6, 70.0)), "a roof over it");
        assert!(!at_park_floor(v(709.1, -1209.8, 50.5)), "La Mesa's roofs");
        assert!(!at_park_floor(v(0.0, 0.0, 60.7)), "no park here");
    }
}
