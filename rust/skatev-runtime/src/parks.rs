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
}

pub const PARKS: &[Park] = &[
    Park { id: "vespucci", centre: Vec2::new(-1374., -1396.), radius: 60. },
    Park { id: "hawick", centre: Vec2::new(280., -206.), radius: 30. },
    Park { id: "littleseoul", centre: Vec2::new(-950., -786.), radius: 30. },
    Park { id: "strawberry", centre: Vec2::new(-266., -1571.), radius: 30. },
    Park { id: "lamesa", centre: Vec2::new(718., -1231.), radius: 30. },
];

/// The id of the park `x`, `y` (GTA metres) lies in.
pub fn park_at(x: f32, y: f32) -> Option<&'static str> {
    PARKS.iter().find(|k| k.centre.distance(Vec2::new(x, y)) <= k.radius).map(|k| k.id)
}

/// Whether `p` (GTA metres) lies in one of `PARKS`.
pub fn in_park(p: Vec2) -> bool {
    PARKS.iter().any(|k| k.centre.distance(p) <= k.radius)
}
