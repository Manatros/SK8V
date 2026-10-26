//! What collision lies at a spot (design 2026-10-26-one-source-collision 6.3,
//! 6.4; Manatros 2026-10-26: "add IDs to the different faces to see which
//! bake id they got"). The installed area's faces with their origins are
//! kept here whether or not the collision view is on; Developer > Mark this
//! spot asks for them (`sv_describe_spot`) and the runtime log gets a
//! MARK-INFO block.
use bevy_math::Vec3;
use rustc_hash::FxHashMap as HashMap;
use std::sync::Mutex;

/// Per triangle an index into `names` (area level: "tile x_y: file").
#[derive(Default, Clone)]
pub struct Origins {
    pub per: Vec<u32>,
    pub names: Vec<String>,
    index: HashMap<String, u32>,
}

impl Origins {
    pub fn id(&mut self, name: &str) -> u32 {
        if let Some(&k) = self.index.get(name) {
            return k;
        }
        let k = self.names.len() as u32;
        self.index.insert(name.to_owned(), k);
        self.names.push(name.to_owned());
        k
    }

    pub fn push(&mut self, name: &str) {
        let k = self.id(name);
        self.per.push(k);
    }
}

struct Installed {
    /// GTA space, with tag and origin name.
    faces: Vec<([Vec3; 3], u32, String)>,
    rails: Vec<Vec<Vec3>>,
}

/// A moving body near the player as the last dynamic update had it.
#[derive(Clone)]
pub struct Body {
    pub model: u32,
    pub position: Vec3,
    pub triangles: usize,
    pub fixed: bool,
    pub allow_box: bool,
}

static INSTALLED: Mutex<Option<Installed>> = Mutex::new(None);
static BODIES: Mutex<Vec<Body>> = Mutex::new(Vec::new());
static STATUS: Mutex<String> = Mutex::new(String::new());

/// Tag bits and their names (`live.rs`, `skate_host::bridge::NO_GRIND_TAG`).
fn tag_names(tag: u32) -> String {
    let bits: [(u32, &str); 11] = [
        (crate::live::PARKMESH_TAG, "park-visible"),
        (crate::live::PARKCUT_TAG, "park-cut"),
        (crate::live::PARKGTA_TAG, "park-gta"),
        (crate::live::URAMP_TAG, "unflagged-ramp"),
        (crate::live::USTEP_TAG, "unflagged-step"),
        (crate::live::FALLBACK_TAG, "fallback"),
        (crate::live::STEP_TAG, "step"),
        (crate::live::RAMP_TAG, "ramp"),
        (crate::live::LIVE_TAG, "live"),
        (crate::live::RAILBOX_TAG, "railbox"),
        (skate_host::bridge::NO_GRIND_TAG, "no-grind"),
    ];
    let names: Vec<&str> = bits.iter().filter(|(b, _)| tag & b != 0).map(|(_, n)| *n).collect();
    if names.is_empty() { "-".to_owned() } else { names.join("+") }
}

fn kind(t: &[Vec3; 3]) -> &'static str {
    let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
    if n.z >= 0.65 { "floor" } else if n.z > 0.2 { "bank" } else if n.z >= -0.2 { "wall" } else { "underside" }
}

/// The area just installed into Skate (Skate space, as `world::Area`).
pub fn install(area: &crate::world::Area) {
    let g = |p: [f32; 3]| crate::coords::from_skate(Vec3::from_array(p));
    let faces = area.triangles.iter().zip(&area.tags).enumerate().map(|(i, (t, &tag))| {
        let name = area.origins.per.get(i).and_then(|&k| area.origins.names.get(k as usize)).cloned()
            .unwrap_or_else(|| "live GTA collision (no baked tile here, or plain GTA compare)".to_owned());
        (t.map(g), tag, name)
    }).collect();
    let rails = area.rails.iter().map(|r| r.iter().map(|&p| g(p)).collect()).collect();
    *INSTALLED.lock().unwrap() = Some(Installed { faces, rails });
}

pub fn set_bodies(bodies: Vec<Body>) {
    *BODIES.lock().unwrap() = bodies;
}

/// Legend line 2 of the collision view (design 6.3).
pub fn set_status(status: String) {
    *STATUS.lock().unwrap() = status;
}

pub fn status() -> String {
    STATUS.lock().unwrap().clone()
}

fn face_distance(p: Vec3, t: &[Vec3; 3]) -> f32 {
    crate::world::closest_on_triangle(p, t).distance(p)
}

fn segment_distance(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

/// The MARK-INFO text for GTA point `centre` and how many faces it names.
pub fn describe(centre: Vec3, radius: f32) -> (String, usize) {
    let mut out = format!("MARK-INFO at {:.2} {:.2} {:.2} (within {radius:.1} m)\n  {}\n", centre.x, centre.y, centre.z, status());
    let installed = INSTALLED.lock().unwrap();
    let Some(area) = installed.as_ref() else {
        out.push_str("  no collision area installed\n");
        return (out, 0);
    };
    let near: Vec<&([Vec3; 3], u32, String)> = area.faces.iter().filter(|(t, _, _)| face_distance(centre, t) <= radius).collect();
    // Grouped by origin, kind and tags.
    let mut groups: std::collections::BTreeMap<(String, &str, String), usize> = Default::default();
    for (t, tag, origin) in &near {
        *groups.entry((origin.clone(), kind(t), tag_names(*tag))).or_default() += 1;
    }
    out.push_str(&format!("  {} faces:\n", near.len()));
    for ((origin, kind, tags), n) in groups.iter().take(60) {
        out.push_str(&format!("    {n:4} {kind:9} tags {tags:28} from {origin}\n"));
    }
    // The nearest faces one by one (what the board touches).
    let mut nearest: Vec<_> = near.iter().map(|f| (face_distance(centre, &f.0), *f)).collect();
    nearest.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (d, (t, tag, origin)) in nearest.iter().take(12) {
        let c = (t[0] + t[1] + t[2]) / 3.0;
        out.push_str(&format!("    face {d:.2} m: {} centre {:.2} {:.2} {:.2} tags {} from {origin}\n", kind(t), c.x, c.y, c.z, tag_names(*tag)));
    }
    let rails: Vec<&Vec<Vec3>> = area.rails.iter()
        .filter(|r| r.windows(2).any(|w| segment_distance(centre, w[0], w[1]) <= radius + 1.0))
        .collect();
    out.push_str(&format!("  {} grind lines within {:.1} m:\n", rails.len(), radius + 1.0));
    for r in rails.iter().take(20) {
        let len: f32 = r.windows(2).map(|w| w[0].distance(w[1])).sum();
        let (a, b) = (r[0], r[r.len() - 1]);
        out.push_str(&format!("    {len:5.2} m, {} points, {:.2} {:.2} {:.2} -> {:.2} {:.2} {:.2}\n", r.len(), a.x, a.y, a.z, b.x, b.y, b.z));
    }
    let bodies = BODIES.lock().unwrap();
    let close: Vec<&Body> = bodies.iter().filter(|b| b.position.distance(centre) <= radius + 3.0).collect();
    out.push_str(&format!("  {} moving or placed bodies within {:.1} m:\n", close.len(), radius + 3.0));
    for b in close.iter().take(20) {
        out.push_str(&format!("    model {:08x} at {:.2} {:.2} {:.2}: {} triangles{}{}\n", b.model, b.position.x, b.position.y, b.position.z, b.triangles,
            if b.fixed { ", fixed (dropped object)" } else { "" },
            if b.allow_box && b.triangles == 12 { ", BOX (no exact collision)" } else { "" }));
    }
    (out, near.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_read_as_names() {
        assert_eq!(tag_names(0), "-");
        assert_eq!(tag_names(crate::live::PARKMESH_TAG | crate::live::PARKCUT_TAG), "park-visible+park-cut");
    }
}
