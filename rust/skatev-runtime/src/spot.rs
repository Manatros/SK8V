//! What collision lies at a spot (design 2026-10-26-one-source-collision 6.3,
//! 6.4; Manatros 2026-10-26: "add IDs to the different faces to see which
//! bake id they got"). The installed area's faces with their origins are
//! kept here whether or not the collision view is on; Developer > Mark this
//! spot asks for them (`sv_describe_spot`) and the runtime log gets a
//! MARK-INFO block.
use bevy_math::Vec3;
use rustc_hash::FxHashMap as HashMap;
use std::sync::{Arc, Mutex};

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
    /// GTA space, with tag and origin index.
    faces: Vec<([Vec3; 3], u32, u32)>,
    origins: Arc<Vec<String>>,
    rails: Vec<Vec<Vec3>>,
}

impl Installed {
    fn origin_name(&self, id: u32) -> &str {
        self.origins
            .get(id as usize)
            .map(|s| s.as_str())
            .unwrap_or("live GTA collision (no baked tile here, or plain GTA compare)")
    }
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

/// Contact witness (design 2026-11-02-contact-witness): the last hard
/// (non-floor) contacts of the board with the static world, each with the
/// face it hit as installed then, newest last.
struct Hit {
    tick: u64,
    body: String,
    point: Vec3,
    normal: Vec3,
    face: Option<([Vec3; 3], u32, String)>,
}
static HITS: Mutex<std::collections::VecDeque<Hit>> = Mutex::new(std::collections::VecDeque::new());
/// How far back Mark this spot reports (Skate ticks, 60 a second).
const HIT_TICKS: u64 = 180;
const HIT_KEEP: usize = 400;
static LAST_TICK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// One tick's hard contacts (Skate space, as `Session::take_static_hits`).
pub fn record_hits(tick: u64, hits: Vec<(usize, [f32; 3], [f32; 3], String)>) {
    LAST_TICK.store(tick, std::sync::atomic::Ordering::Relaxed);
    // The faces first (INSTALLED, then HITS: never both held, so no lock order to keep).
    let new: Vec<Hit> = if hits.is_empty() {
        Vec::new()
    } else {
        let installed = INSTALLED.lock().unwrap();
        // Board parts only (wheels, trucks, deck): the skater's own volumes
        // touch walls in every bail and read as "Attached(n)" (peer review).
        hits.into_iter().filter(|h| h.3.starts_with("Board(")).map(|(index, point, normal, body)| Hit {
            tick,
            body,
            point: crate::coords::from_skate(Vec3::from_array(point)),
            normal: crate::coords::basis() * Vec3::from_array(normal),
            face: installed.as_ref().and_then(|a| {
                let (t, tag, orig_id) = *a.faces.get(index)?;
                Some((t, tag, a.origin_name(orig_id).to_owned()))
            }),
        }).collect()
    };
    let mut kept = HITS.lock().unwrap();
    while kept.front().is_some_and(|h| h.tick + HIT_TICKS < tick || h.tick > tick) {
        kept.pop_front();
    }
    for hit in new {
        // One entry per face and body, renewed by each contact, so a board
        // resting against a lip cannot push the stopping face out (peer review).
        let face = |h: &Hit| h.face.as_ref().map(|f| f.0);
        if let Some(i) = kept.iter().position(|h| h.body == hit.body && face(h).is_some() && face(h) == face(&hit)) {
            kept.remove(i);
        }
        kept.push_back(hit);
        if kept.len() > HIT_KEEP {
            kept.pop_front();
        }
    }
}

/// The witness lines for MARK-INFO: per face hit, newest first.
fn hit_lines(now: u64) -> String {
    let kept = HITS.lock().unwrap();
    if kept.is_empty() {
        return "  hard contacts in the last 3 s: none\n".to_owned();
    }
    let mut out = format!("  hard contacts in the last 3 s: {}\n", kept.len());
    // One line per face (its newest contact) and body, newest first.
    let mut seen: Vec<(String, String)> = Vec::new();
    for h in kept.iter().rev() {
        let key = (h.face.as_ref().map_or("?".to_owned(), |f| format!("{:?}", f.0)), h.body.clone());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        if seen.len() > 16 {
            break;
        }
        let ago = now.saturating_sub(h.tick) as f32 / 60.0;
        out.push_str(&format!("    {ago:.2} s ago {} at {:.2} {:.2} {:.2} normal {:.2} {:.2} {:.2}", h.body, h.point.x, h.point.y, h.point.z,
            h.normal.x, h.normal.y, h.normal.z));
        match &h.face {
            Some((t, tag, origin)) => {
                let c = (t[0] + t[1] + t[2]) / 3.0;
                let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
                let area = (t[1] - t[0]).cross(t[2] - t[0]).length() / 2.0;
                out.push_str(&format!(": {} centre {:.2} {:.2} {:.2} normal {:.2} {:.2} {:.2} area {:.4} m2 tags {} from {origin}\n",
                    kind(t), c.x, c.y, c.z, n.x, n.y, n.z, area, tag_names(*tag)));
                out.push_str(&format!("      corners {:.3} {:.3} {:.3} / {:.3} {:.3} {:.3} / {:.3} {:.3} {:.3}\n",
                    t[0].x, t[0].y, t[0].z, t[1].x, t[1].y, t[1].z, t[2].x, t[2].y, t[2].z));
            }
            None => out.push_str(": face not in the installed area (rebuilt since)\n"),
        }
    }
    out
}
static STATUS: Mutex<String> = Mutex::new(String::new());

/// Tag bits and their names (`live.rs`, `skate_host::bridge::NO_GRIND_TAG`).
fn tag_names(tag: u32) -> String {
    let bits: [(u32, &str); 13] = [
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
        (crate::live::VISBAR_TAG, "visbar"),
        (crate::live::SLAB_TAG, "slab"),
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
    let mut names = area.origins.names.clone();
    let fallback = "live GTA collision (no baked tile here, or plain GTA compare)".to_owned();
    let fallback_idx = if let Some(i) = names.iter().position(|n| n == &fallback) {
        i as u32
    } else {
        let i = names.len() as u32;
        names.push(fallback);
        i
    };
    let origins = Arc::new(names);
    let faces = area.triangles.iter().zip(&area.tags).enumerate().map(|(i, (t, &tag))| {
        let origin_idx = area.origins.per.get(i).copied().unwrap_or(fallback_idx);
        (t.map(g), tag, origin_idx)
    }).collect();
    let rails = area.rails.iter().map(|r| r.iter().map(|&p| g(p)).collect()).collect();
    *INSTALLED.lock().unwrap() = Some(Installed { faces, origins, rails });
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
    // The witness first: HITS and INSTALLED are never held together.
    let witness = hit_lines(LAST_TICK.load(std::sync::atomic::Ordering::Relaxed));
    let installed = INSTALLED.lock().unwrap();
    let Some(area) = installed.as_ref() else {
        out.push_str("  no collision area installed\n");
        return (out, 0);
    };
    let near: Vec<&([Vec3; 3], u32, u32)> = area.faces.iter().filter(|(t, _, _)| face_distance(centre, t) <= radius).collect();
    // Grouped by origin, kind and tags.
    let mut groups: std::collections::BTreeMap<(String, &str, String), usize> = Default::default();
    for (t, tag, orig_idx) in &near {
        let origin = area.origin_name(*orig_idx).to_owned();
        *groups.entry((origin, kind(t), tag_names(*tag))).or_default() += 1;
    }
    out.push_str(&format!("  {} faces:\n", near.len()));
    for ((origin, kind, tags), n) in groups.iter().take(60) {
        out.push_str(&format!("    {n:4} {kind:9} tags {tags:28} from {origin}\n"));
    }
    // The nearest faces one by one (what the board touches).
    let mut nearest: Vec<_> = near.iter().map(|f| (face_distance(centre, &f.0), *f)).collect();
    nearest.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (d, (t, tag, orig_idx)) in nearest.iter().take(12) {
        let origin = area.origin_name(*orig_idx);
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
    out.push_str(&witness);
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

    /// Design 2026-11-02 contact witness: hard contacts are kept 3 s, one
    /// line per face and body, newest first; older ones go.
    #[test]
    fn hard_contacts_are_kept_three_seconds_one_line_per_face() {
        let hit = |i: usize, body: &str| (i, [0.0, 0.5, 0.0], [1.0, 0.1, 0.0], body.to_owned());
        record_hits(1000, vec![hit(7, "Board(RightFrontWheel)"), hit(7, "Board(RightFrontWheel)"), hit(8, "Board(Deck)")]);
        record_hits(1100, vec![hit(9, "Board(LeftBackWheel)")]);
        let lines = hit_lines(1100);
        assert!(lines.contains("hard contacts in the last 3 s: 4"), "{lines}");
        // No area installed in this test: the faces are named as rebuilt since.
        // Two contacts of the front wheel make one line: three lines in all.
        assert_eq!(lines.matches("face not in the installed area").count(), 3, "one line per face and body: {lines}");
        assert!(lines.find("LeftBackWheel").unwrap() < lines.find("Deck").unwrap(), "newest first: {lines}");
        // 3 s (180 ticks) after tick 1000 those are gone; the one at 1100 stays.
        record_hits(1279, Vec::new());
        assert!(hit_lines(1279).contains("hard contacts in the last 3 s: 1"), "{}", hit_lines(1279));
    }
}
