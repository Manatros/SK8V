//! Moving GTA entities (vehicles, peds, props) as Skate dynamic geometry.
//! The host supplies entity transforms and explicit box fallbacks each frame.
//! Owned model bounds become outward-facing triangles in Skate space and go through
//! Skate's own contact code (overlay patch 0003). Static Los Santos collision
//! never comes through here.
use crate::coords;
use bevy_math::{Quat, Vec3};
use std::{collections::HashMap, path::{Path, PathBuf}};

/// The biped moves through Skate's query controller, outside the board's
/// contact solve. Return one bounded push per touched object, at its actual
/// triangle (not its model box, which can fill a turnstile's openings).
/// GTA owns the object's hinge/rotation and consumes these like solved hits.
pub fn walking_pushes(
    feet: Vec3, velocity: Vec3, dt: f32,
    triangles: &[([[f32; 3]; 3], u32, u32)],
    bodies: &[skate_host::bridge::HostBody],
) -> Vec<skate_host::bridge::HostImpulse> {
    if !feet.is_finite() || !velocity.is_finite() || !(0.0..=0.1).contains(&dt) { return Vec::new(); }
    let velocity = Vec3::new(velocity.x, 0.0, velocity.z).clamp_length_max(4.0);
    if velocity.length() < 0.1 { return Vec::new(); }
    let mut best: HashMap<u32, (f32, Vec3, Vec3)> = HashMap::new();
    for &(t, tag, _) in triangles {
        if (tag >> 28) & 7 != 2 { continue; }
        let Some(body) = bodies.iter().find(|b| b.tag == tag) else { continue };
        let t = t.map(Vec3::from_array);
        for height in [0.35, 0.7, 1.05] {
            let centre = feet + Vec3::Y * height;
            let p = crate::world::closest_on_triangle(centre, &t);
            let delta = p - centre;
            if delta.length_squared() > 0.3f32.powi(2) { continue; }
            let toward = Vec3::new(delta.x, 0.0, delta.z).normalize_or_zero();
            let surface_velocity = Vec3::from_array(body.linear_velocity)
                + Vec3::from_array(body.angular_velocity).cross(p - Vec3::from_array(body.position));
            let relative = velocity - surface_velocity;
            let closing = relative.dot(toward).max(0.0);
            if closing > best.get(&tag).map_or(0.1, |b| b.0) {
                best.insert(tag, (closing, p, toward));
            }
        }
    }
    best.into_iter().map(|(tag, (closing, point, toward))| skate_host::bridge::HostImpulse {
        tag, point: point.to_array(),
        impulse: (toward * (80.0 * closing * (8.0 * dt).min(1.0))).to_array(),
    }).collect()
}

#[cfg(test)]
mod walking_tests {
    use super::*;

    #[test]
    fn walking_pushes_real_prop_faces_once_at_an_offset_and_never_empty_gaps() {
        let tag = DYNAMIC_TAG | (2<<28) | 1;
        let body = skate_host::bridge::HostBody {
            tag, position: [0.2,0.0,0.0], orientation: [0.0,0.0,0.0,1.0],
            linear_velocity: [0.0;3], angular_velocity: [0.0;3],
            inverse_mass: 0.0, inverse_inertia: [0.0;3], collision_group: 0, volumes: Vec::new(),
        };
        let face = ([[0.2,0.0,-1.0],[0.2,1.5,-1.0],[0.2,0.0,1.0]],tag,0);
        let hits = walking_pushes(Vec3::ZERO, Vec3::X, 1.0/60.0, &[face,face], &[body.clone()]);
        assert_eq!(hits.len(),1, "not one shove per triangle/height");
        assert!(hits[0].impulse[0] > 0.0 && hits[0].impulse[0] < 12.0);
        assert!(Vec3::from_array(hits[0].point).distance(Vec3::from_array(body.position)) > 0.1, "offset lets GTA's hinge rotate");
        for (feet,velocity) in [(Vec3::ZERO,-Vec3::X),(Vec3::ZERO,Vec3::ZERO),(Vec3::Z*5.0,Vec3::X)] {
            assert!(walking_pushes(feet,velocity,1.0/60.0,&[face],&[body.clone()]).is_empty());
        }
        assert!(walking_pushes(Vec3::ZERO,Vec3::X,1.0/60.0,&[face],&[]).is_empty(), "fixed objects have no exchange proxy");
        let mut moving = body;
        moving.linear_velocity = [2.0,0.0,0.0];
        assert!(walking_pushes(Vec3::ZERO,Vec3::X,1.0/60.0,&[face],&[moving]).is_empty(), "gate already moving away");
    }
}

#[derive(Clone, Debug)]
pub struct HostBody {
    pub model: u32,
    pub position: Vec3,
    pub axes: [Vec3; 3],
    pub fallback: HostBox,
    pub allow_box: bool,
    /// Host-sampled world velocities (GTA space, m/s and rad/s); `None` when
    /// the sample is not finite or implausible (no contact exchange).
    pub velocity: Option<(Vec3, Vec3)>,
    /// Vehicle `bumper_r` bone (GTA world), when the host found one.
    pub grab_point: Option<Vec3>,
    /// Never moves (the object dropper's props, `flags & 2`): built into the
    /// static area with its rails (`world::StaticWorld::set_fixed`).
    pub fixed: bool,
    /// A part broken off a vehicle (`flags & 4`): an object with the car's
    /// model; only its own bound from GTA is used, never the car's.
    pub vehicle_part: bool,
}

impl HostBody {
    pub fn from_abi(b: &crate::SvDynamicBody) -> Option<Self> {
        let v = |p: crate::SvVec3| Vec3::new(p.x, p.y, p.z);
        let position = v(b.position);
        let axes = [v(b.right), v(b.forward), v(b.up)];
        let determinant = axes[0].cross(axes[1]).dot(axes[2]);
        if !position.is_finite() || axes.iter().any(|a| !a.is_finite() || a.length_squared() > 10000.)
            || !determinant.is_finite() || determinant.abs() < 1e-6 { return None; }
        let q = b.fallback.rotation;
        let rotation = Quat::from_xyzw(q.x, q.y, q.z, q.w);
        let fallback = HostBox { tag: b.fallback.tag & !DYNAMIC_TAG,
            center: v(b.fallback.center), rotation: rotation.normalize(), half_extents: v(b.fallback.half_extents) };
        if b.flags & 1 != 0 && !usable(&fallback) { return None; }
        let velocity = Some((v(b.linear_velocity), v(b.angular_velocity)))
            .filter(|(l, a)| l.is_finite() && a.is_finite() && l.length() < 500.0 && a.length() < 200.0);
        let grab_point = (b.grab_flags & 1 != 0)
            .then(|| v(b.grab_point))
            .filter(|p| p.is_finite());
        Some(Self { model: b.model_hash, position, axes, fallback, allow_box: b.flags & 1 != 0, velocity, grab_point, fixed: b.flags & 2 != 0, vehicle_part: b.flags & 4 != 0 })
    }

    pub fn transform(&self, t: &svwc::Tri) -> ([[f32; 3]; 3], u32) {
        let mut points = t.v.map(|p| coords::to_skate(self.position + self.axes[0]*p[0] + self.axes[1]*p[1] + self.axes[2]*p[2]).to_array());
        if self.axes[0].cross(self.axes[1]).dot(self.axes[2]) < 0. { points.swap(1, 2); }
        (points, self.fallback.tag | DYNAMIC_TAG)
    }
}

/// Exact extracted model bounds, loaded once per model on the simulation worker.
/// There is no filesystem I/O on the ScriptHookV fiber. Failed models are logged
/// once; a fallback is used only when the host explicitly allows its small box.
pub struct Templates {
    directory: PathBuf,
    /// Each model's bound triangles with their packed Skate surface.
    models: HashMap<u32, Option<Vec<(svwc::Tri, u32)>>>,
    /// Model space bound box of each loaded template (min, max).
    extents: HashMap<u32, (Vec3, Vec3)>,
    /// Objects with no template whose bound GTA's physics level did not give
    /// yet: when last tried and how often (a fresh spawn may not be in it).
    misses: HashMap<u32, (std::time::Instant, u8)>,
    masses: HashMap<u32, Mass>,
    /// Each vehicle's own collision read from GTA (by body tag): when read,
    /// and its triangles (model space, packed surfaces). Absent parts
    /// (spoilers, extras, engine covers) are not in it, unlike the template.
    cars: HashMap<u32, (std::time::Instant, Vec<(svwc::Tri, u32)>)>,
    car_logs: u32,
    /// Vehicles whose own read failed (by tag): retried after CAR_RETRY, so
    /// one failing car cannot take every batch's read.
    car_failed: HashMap<u32, std::time::Instant>,
    /// Vehicle shapes remembered per model: another car of the same
    /// type uses it at once. The first good read of a model is kept in memory
    /// and in `<cache>.vehicle-models/<model>.svwc`; None: none on disk.
    car_models: HashMap<u32, Option<Vec<(svwc::Tri, u32)>>>,
    /// Grind rails per vehicle instance (model space), invalidated on shape changes.
    car_rails: HashMap<u32, Vec<Vec<Vec3>>>,
    car_rail_revision: HashMap<u32, u64>,
    /// Metres the skitch grab line sits behind the rearmost bound.
    pub standoff: f32,
}

impl Templates {
    fn car_model_path(&self, model: u32) -> PathBuf {
        self.directory.with_extension("").with_extension("vehicle-models").join(format!("{model:08x}.svwc"))
    }

    /// A vehicle model's remembered shape: memory, else its file (read once).
    fn model_shape(&mut self, model: u32) -> Option<&Vec<(svwc::Tri, u32)>> {
        if !self.car_models.contains_key(&model) {
            let loaded = svwc::Cache::open(&self.car_model_path(model)).and_then(|mut c| c.read_all()).ok().filter(|t| !t.is_empty())
                .map(|t| t.into_iter().map(|t| { let s = crate::materials::surface(t.material).packed(); (t, s) }).collect());
            self.car_models.insert(model, loaded);
        }
        self.car_models.get(&model)?.as_ref()
    }

    /// The first good read of a vehicle model is remembered (memory + file).
    fn remember_model(&mut self, model: u32, shape: &[(svwc::Tri, u32)], messages: &mut Vec<String>) {
        if self.model_shape(model).is_some() {
            return;
        }
        let path = self.car_model_path(model);
        let tris: Vec<svwc::Tri> = shape.iter().map(|(t, _)| *t).collect();
        let saved = path.parent().map(std::fs::create_dir_all).transpose().ok().flatten().is_some()
            && std::fs::File::create(&path).and_then(|mut f| svwc::write(&mut f, &tris, 64.0)).is_ok();
        messages.push(format!("vehicle {model:08x}: shape remembered for this model ({} triangles{})", tris.len(), if saved { ", saved" } else { ", not saved" }));
        self.car_models.insert(model, Some(shape.to_vec()));
    }

    fn cache_car_shape(&mut self, tag: u32, shape: Vec<(svwc::Tri, u32)>) {
        if self.cars.get(&tag).is_none_or(|(_, old)| !same_shape(old, &shape)) {
            self.car_rails.remove(&tag);
        }
        self.cars.insert(tag, (std::time::Instant::now(), shape));
    }

    /// Rails from this instance's latest shape, falling back to its model.
    pub fn vehicle_rails(&mut self, body: &HostBody, messages: &mut Vec<String>) -> Vec<Vec<Vec3>> {
        let tag = body.fallback.tag;
        if !self.car_rails.contains_key(&tag) {
            let shape = self.cars.get(&tag).map(|(_, own)| own.clone())
                .or_else(|| self.model_shape(body.model).cloned())
                .or_else(|| self.models.get(&body.model).cloned().flatten());
            let Some(shape) = shape else { return Vec::new() };
            let tris: Vec<_> = shape.iter().map(|(t, _)| t.v.map(Vec3::from_array)).collect();
            let rails = object_rails(&tris);
            let revision = self.car_rail_revision.entry(tag).or_default();
            *revision += 1;
            messages.push(format!("car {tag:08x} rails r{revision}: {}", rails.len()));
            self.car_rails.insert(tag, rails);
        }
        let place = |p: Vec3| body.position + body.axes[0] * p.x + body.axes[1] * p.y + body.axes[2] * p.z;
        self.car_rails[&tag].iter().map(|r| r.iter().map(|&p| place(p)).collect()).collect()
    }
}

/// An object's rails (model space): the finder's and the old SkateGTA fold
/// rule's (a ramp's top bar), as one set. Lines at the object's own bottom
/// are its outline on the ground, not rails (2026-10-20: grindable edges
/// next to benches where nothing is).
pub fn object_rails(tris: &[[Vec3; 3]]) -> Vec<Vec<Vec3>> {
    if tris.is_empty() { return Vec::new(); }
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for t in tris {
        for p in t {
            min = min.min(*p);
            max = max.max(*p);
        }
    }
    let bottom = min.z;
    let mut rails = crate::rails::find_metres(tris).0;
    rails.extend(crate::rails::fold_rails(tris));
    rails.retain(|r| r.iter().map(|p| p.z).fold(f32::MAX, f32::min) > bottom + BASE_CLEARANCE);
    crate::rails::trim_overlaps(&mut rails);
    crate::rails::join_ends(&mut rails);
    crate::rails::dedupe_metres(&mut rails);
    crate::rails::join_ends(&mut rails);
    crate::rails::straighten(&mut rails, 0.03);
    // Lines stay within the object's box. A pipe's two top facet lines are
    // merged onto its crown by dedupe (rails.rs crown), for every object.
    for r in &mut rails {
        for p in r.iter_mut() {
            p.x = p.x.clamp(min.x, max.x);
            p.y = p.y.clamp(min.y, max.y);
            p.z = p.z.clamp(min.z, max.z);
        }
        r.dedup_by(|a, b| a.distance_squared(*b) < 1e-6);
    }
    rails.retain(|r| r.len() >= 2);
    rails
}

/// Rails lower than this above an object's bottom are its base outline (metres).
const BASE_CLEARANCE: f32 = 0.08;

fn same_shape(a: &[(svwc::Tri, u32)], b: &[(svwc::Tri, u32)]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|((a, sa), (b, sb))| a.v == b.v && a.material == b.material && sa == sb)
}

/// A vehicle whose own read failed is tried again after this.
const CAR_RETRY: std::time::Duration = std::time::Duration::from_secs(2);

/// How often a vehicle's own collision is read again (parts come off).
const CAR_REFRESH: std::time::Duration = std::time::Duration::from_secs(3);

/// Grab line height above the body's underside for a vehicle with no
/// `bumper_r` bone (Skate raises a line under its own reach floor anyway).
pub const NO_BONE_HEIGHT: f32 = 0.6;

/// Default skitch standoff (`SkitchStandoff` in the INI overrides it).
pub const SKITCH_STANDOFF: f32 = 0.3;

/// Authored mass data (`<cache>.prop-masses.txt`): fragment masses and
/// vehicles from handling.meta.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mass {
    pub kg: f32,
    pub inertia_multiplier: Vec3,
    pub centre_of_mass_offset: Vec3,
}

/// Parses `<cache>.prop-masses.txt` lines: model kg mult.xyz com.xyz.
pub fn parse_masses(text: &str) -> HashMap<u32, Mass> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split('#').next()?.split_whitespace().collect();
            if f.len() != 8 { return None; }
            let model = u32::from_str_radix(f[0], 16).ok()?;
            let n: Vec<f32> = f[1..].iter().map(|x| x.parse().ok()).collect::<Option<_>>()?;
            (n.iter().all(|x| x.is_finite()) && n[0] > 0.0).then(|| (model, Mass {
                kg: n[0],
                inertia_multiplier: Vec3::new(n[1], n[2], n[3]),
                centre_of_mass_offset: Vec3::new(n[4], n[5], n[6]),
            }))
        })
        .collect()
}

impl Templates {
    pub fn new(world_cache: &Path) -> Self {
        let masses = std::fs::read_to_string(world_cache.with_extension("prop-masses.txt"))
            .map(|t| parse_masses(&t))
            .unwrap_or_default();
        let _ = crate::vehicle_shapes::TEMPLATE_DIR.set(world_cache.with_extension("prop-models"));
        Self { directory: world_cache.with_extension("prop-models"), models: HashMap::new(), extents: HashMap::new(), misses: HashMap::new(), masses, cars: HashMap::new(), car_logs: 0, car_failed: HashMap::new(), car_models: HashMap::new(), car_rails: HashMap::new(), car_rail_revision: HashMap::new(), standoff: SKITCH_STANDOFF }
    }

    /// Contact-exchange proxies (Skate space) for the moving vehicles and
    /// objects among `bodies`. Mass is the authored mass when known, else the
    /// entity is kinematic for the solve. Inertia is that of the entity's
    /// bound box (template, else the host box), scaled by handling.meta's
    /// inertia multiplier for vehicles. Peds are excluded (separate path).
    pub fn proxies(&self, bodies: &[HostBody]) -> Vec<skate_host::bridge::HostBody> {
        let mut out = Vec::new();
        for b in bodies {
            let Some((linear, angular)) = b.velocity.filter(|_| !b.vehicle_part) else { continue };
            let kind = (b.fallback.tag >> 28) & 0x7;
            if kind == 1 {
                continue;
            }
            let scale = Vec3::new(b.axes[0].length(), b.axes[1].length(), b.axes[2].length());
            if !(scale.min_element() > 1e-5) { continue; }
            let unit = [b.axes[0] / scale.x, b.axes[1] / scale.y, b.axes[2] / scale.z];
            let mass = self.masses.get(&b.model).copied();
            let com_offset = mass.map_or(Vec3::ZERO, |m| m.centre_of_mass_offset);
            let (centre, half) = match self.extents.get(&b.model) {
                Some(&(lo, hi)) => {
                    let c = (lo + hi) * 0.5 + com_offset;
                    (b.position + b.axes[0] * c.x + b.axes[1] * c.y + b.axes[2] * c.z, (hi - lo) * 0.5 * scale)
                }
                None => {
                    let r = b.fallback.rotation;
                    (b.fallback.center + r * com_offset, b.fallback.half_extents)
                }
            };
            let (inverse_mass, inverse_inertia) = match mass {
                Some(m) => {
                    let h2 = half * half;
                    let i = Vec3::new(h2.y + h2.z, h2.x + h2.z, h2.x + h2.y) * (m.kg / 3.0) * m.inertia_multiplier;
                    let inv = |v: f32| if v > 1e-6 { 1.0 / v } else { 0.0 };
                    (1.0 / m.kg, [inv(i.x), inv(i.y), inv(i.z)])
                }
                None => (0.0, [0.0; 3]),
            };
            let basis = bevy_math::Mat3::from_cols(
                coords::to_skate(unit[0]), coords::to_skate(unit[1]), coords::to_skate(unit[2]));
            let q = Quat::from_mat3(&basis).normalize();
            out.push(skate_host::bridge::HostBody {
                tag: b.fallback.tag | DYNAMIC_TAG,
                position: coords::to_skate(centre).to_array(),
                orientation: q.to_array(),
                linear_velocity: coords::to_skate(linear).to_array(),
                angular_velocity: coords::to_skate(angular).to_array(),
                inverse_mass,
                inverse_inertia,
                collision_group: if kind == 0 { skate_host::bridge::VEHICLE_GROUP } else { 0 },
                volumes: Vec::new(),
            });
        }
        out
    }
    /// Skitch grab lines: every vehicle gets one. A two-point line across the
    /// body's full width (lo.x..hi.x) at the `bumper_r` bone's model-space
    /// height (vans, buses, planes, bikes have none: `NO_BONE_HEIGHT` above
    /// the body's underside), at the rearmost extent (lo.y) of the exact
    /// template, or of the host's model box when the model has no template:
    /// the bone sits inside the bumper, and on cars
    /// whose body reaches further back (SUV spare wheels, tow bars) a line at
    /// the bone held the rider inside the body. Approach is the outward rear
    /// normal, model (0, -1, 0). A line under Skate's reach is raised by the
    /// session (`Session::set_grab_lines`).
    /// The line is an invisible wall `standoff` behind that rearmost extent:
    /// at the bodywork the rider's hands met the car's contacts, GTA's
    /// physics on the car and the limb colliders at once.
    /// Points and approach stay in model space; the frame carries the
    /// entity's live axes and position into Skate space.
    /// Model space bound box (min, max) of a loaded template.
    pub fn extent(&self, model: u32) -> Option<(Vec3, Vec3)> {
        self.extents.get(&model).copied()
    }

    pub fn grab_lines(&self, bodies: &[HostBody]) -> Vec<skate_host::bridge::GrabLine> {
        let mut out = Vec::new();
        for b in bodies {
            if (b.fallback.tag >> 28) & 0x7 != 0 { continue; }
            let m = bevy_math::Mat3::from_cols(b.axes[0], b.axes[1], b.axes[2]);
            if !(m.determinant().abs() > 1e-6) { continue; }
            let (lo, hi) = match self.extents.get(&b.model) {
                Some(&e) => e,
                None => {
                    // No template: the host's model box, in model space.
                    let scale = Vec3::new(b.axes[0].length(), b.axes[1].length(), b.axes[2].length());
                    let c = m.inverse() * (b.fallback.center - b.position);
                    let half = b.fallback.half_extents / scale;
                    (c - half, c + half)
                }
            };
            let z = match b.grab_point {
                Some(world) => (m.inverse() * (world - b.position)).z,
                None => lo.z + NO_BONE_HEIGHT.min((hi.z - lo.z) * 0.5),
            };
            if !(lo.is_finite() && hi.is_finite() && z.is_finite()) { continue; }
            let row = |v: Vec3| { let s = coords::to_skate(v); [s.x, s.y, s.z, 0.0] };
            let (linear, _) = b.velocity.unwrap_or((Vec3::ZERO, Vec3::ZERO));
            out.push(skate_host::bridge::GrabLine {
                tag: b.fallback.tag | DYNAMIC_TAG,
                frame: [row(b.axes[0]), row(b.axes[1]), row(b.axes[2]), row(b.position)],
                velocity: row(linear),
                points: [[lo.x, lo.y - self.standoff, z, 0.0], [hi.x, lo.y - self.standoff, z, 0.0]],
                approach: [0.0, -1.0, 0.0, 0.0],
            });
        }
        out
    }
    fn install(&mut self, model: u32, tris: Vec<svwc::Tri>, source: &str, messages: &mut Vec<String>) {
        messages.push(format!("dynamic model {model:08x}: {} {source}{}", tris.len(),
            self.masses.get(&model).map_or(String::new(), |m| format!(", {:.1} kg", m.kg))));
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for t in &tris {
            for p in t.v {
                lo = lo.min(Vec3::from_array(p));
                hi = hi.max(Vec3::from_array(p));
            }
        }
        self.extents.insert(model, (lo, hi));
        let mut tris = tris;
        // A prop's TWIGS/LEAVES volumes are its canopy (the bound GTA keeps
        // for bullets and cars), not something to ride into: the trunk stays
        // (Manatros 2026-10-25: palms with giant hitboxes, 'can't even get near').
        let before = tris.len();
        tris.retain(|t| !matches!(t.material, 51 | 52));
        if tris.len() < before {
            messages.push(format!("dynamic model {model:08x}: {} canopy triangles left out", before - tris.len()));
        }
        if model == 0x03154ddc {
            // For the dropper rail (prop_skate_rail 0x03154ddc), physical collision and grind
            // properties are confined strictly to the rideable top bar (centroid_z >= hi.z - 0.08).
            // Diagonal leg braces, support struts and entry approach slopes are completely excluded
            // so wheels, trucks and board never collide with invisible spider legs.
            tris.retain(|t| {
                let centroid_z = (t.v[0][2] + t.v[1][2] + t.v[2][2]) / 3.0;
                centroid_z >= hi.z - 0.08
            });
        } else {
            let slopes = entry_slopes(&tris);
            if !slopes.is_empty() {
                messages.push(format!("dynamic model {model:08x}: {} approach slope triangles onto a raised floor", slopes.len()));
                tris.extend(slopes);
            }
        }
        // Rail ends flush with the line (GTA's bent rail ends poke up 2.2 cm
        // through their rotated collision boxes; world::flush_rail_ends_gta).
        let rails = object_rails(&tris.iter().map(|t| t.v.map(Vec3::from_array)).collect::<Vec<_>>());
        let mut shape: Vec<([[f32; 3]; 3], u32)> = tris.iter().map(|t| (t.v, u32::from(t.material))).collect();
        crate::world::flush_rail_ends_gta(&mut shape, &rails);
        for (t, (v, _)) in tris.iter_mut().zip(&shape) {
            t.v = *v;
        }
        let surfaced = tris.into_iter().map(|t| {
            let surface = crate::materials::surface(t.material).packed();
            (t, surface)
        }).collect();
        self.models.insert(model, Some(surfaced));
    }

    pub fn retain_bodies(&mut self, bodies: &[HostBody]) {
        self.cars.retain(|tag, _| bodies.iter().any(|b| b.fallback.tag == *tag));
        self.car_rails.retain(|tag, _| bodies.iter().any(|b| b.fallback.tag == *tag));
        self.car_rail_revision.retain(|tag, _| bodies.iter().any(|b| b.fallback.tag == *tag));
        self.car_failed.retain(|tag, at| at.elapsed() < CAR_RETRY && bodies.iter().any(|b| b.fallback.tag == *tag));
    }

    /// Objects with no template are read from GTA's physics level (`level`):
    /// retried every 2 s, 10 times, until it has them.
    pub fn triangles(&mut self, bodies: &[HostBody], level: Option<&crate::live::Level>) -> (Vec<([[f32; 3]; 3], u32, u32)>, Vec<String>) {
        const RETRY: std::time::Duration = std::time::Duration::from_secs(2);
        const TRIES: u8 = 10;
        let mut triangles = Vec::new();
        let mut messages = Vec::new();
        // Vehicles: their own collision from GTA, one new read per batch,
        // each car read again every CAR_REFRESH. SK8V's template is the
        // model's full bound with every optional part (spoilers, extras):
        // Manatros 2026-10-19, "the car has the spoiler as hitbox" when it has
        // none. The template stays the fallback until the first read.
        let mut read_budget = 1;
        let unread = bodies.iter().any(|b| {
            (b.fallback.tag >> 28) & 0x7 == 0 && !self.cars.contains_key(&b.fallback.tag) && !self.car_failed.contains_key(&b.fallback.tag)
                && self.car_models.get(&b.model).is_none_or(|m| m.is_none())
        });
        for body in bodies {
            let vehicle = (body.fallback.tag >> 28) & 0x7 == 0;
            if (vehicle || body.vehicle_part)
                && let Some(level) = level
            {
                let own = self.cars.get(&body.fallback.tag);
                // A refresh waits while a car without any shape is waiting for its first read.
                let stale = own.is_none() || (!unread && own.is_some_and(|(at, _)| at.elapsed() >= CAR_REFRESH));
                if stale && read_budget > 0 && !self.car_failed.contains_key(&body.fallback.tag) {
                    read_budget -= 1;
                    let speed = body.velocity.map_or(0.0, |(l, _)| l.length());
                    match level.model_near(body.position, body.axes, 0.25 + speed * 0.15) {
                        Ok(tris) if !tris.is_empty() && tris.iter().all(|t| t.v.iter().flatten().all(|x| x.is_finite())) => {
                            if !self.cars.contains_key(&body.fallback.tag) && self.car_logs < 20 {
                                self.car_logs += 1;
                                let template = self.models.get(&body.model).and_then(|m| m.as_ref()).map_or(0, |m| m.len());
                                messages.push(format!("vehicle {:08x}: its own collision from GTA, {} triangles (template {template})", body.model, tris.len()));
                            }
                            let surfaced: Vec<(svwc::Tri, u32)> = tris.into_iter().map(|t| { let s = crate::materials::surface(t.material).packed(); (t, s) }).collect();
                            if vehicle {
                                self.remember_model(body.model, &surfaced, &mut messages);
                            }
                            self.cache_car_shape(body.fallback.tag, surfaced);
                        }
                        // Keep the last good read; without one the template below.
                        _ => {
                            if let Some(e) = self.cars.get_mut(&body.fallback.tag) {
                                e.0 = std::time::Instant::now();
                            }
                            self.car_failed.insert(body.fallback.tag, std::time::Instant::now());
                        }
                    }
                }
                if let Some((_, own)) = self.cars.get(&body.fallback.tag) {
                    triangles.extend(own.iter().map(|(t, surface)| { let (p, id) = body.transform(t); (p, id, *surface) }));
                    continue;
                }
                // A broken-off part without its own read yet: no collision
                // rather than the whole car's.
                if body.vehicle_part {
                    continue;
                }
                if let Some(shape) = self.model_shape(body.model) {
                    triangles.extend(shape.iter().map(|(t, surface)| { let (p, id) = body.transform(t); (p, id, *surface) }));
                    continue;
                }
            }
            let live = level.filter(|_| (body.fallback.tag >> 28) & 0x7 == 2);
            if body.model != 0 && !self.models.contains_key(&body.model) {
                let path = self.directory.join(format!("{:08x}.svwc", body.model));
                let loaded = svwc::Cache::open(&path).and_then(|mut cache| {
                    if cache.record_count > 1_000_000 { return Err(std::io::Error::other("model record limit exceeded")); }
                    let tris = cache.read_all()?;
                    if tris.is_empty() { return Err(std::io::Error::other("empty model bound")); }
                    Ok(tris)
                });
                match (loaded, live) {
                    (Ok(tris), _) => self.install(body.model, tris, "exact bound triangles", &mut messages),
                    (Err(e), Some(level)) => match level.model_at(body.position, body.axes) {
                        Ok(tris) => self.install(body.model, tris, "bound triangles read from GTA (no template)", &mut messages),
                        Err(live) => {
                            messages.push(format!("dynamic model {:08x}: no template ({e}) and not in GTA's physics level yet ({live}); box fallback {}", body.model, body.allow_box));
                            self.models.insert(body.model, None);
                            self.misses.insert(body.model, (std::time::Instant::now(), 1));
                        }
                    },
                    (Err(e), None) => {
                        messages.push(format!("dynamic model {:08x}: exact bound unavailable ({e}); box fallback {}", body.model, body.allow_box));
                        self.models.insert(body.model, None);
                    }
                }
            } else if let (Some(level), Some((at, tries))) = (live, self.misses.get(&body.model).copied())
                && tries < TRIES
                && at.elapsed() >= RETRY
            {
                match level.model_at(body.position, body.axes) {
                    Ok(tris) => {
                        self.misses.remove(&body.model);
                        self.install(body.model, tris, "bound triangles read from GTA (no template)", &mut messages);
                    }
                    Err(e) => {
                        self.misses.insert(body.model, (std::time::Instant::now(), tries + 1));
                        if tries + 1 == TRIES {
                            messages.push(format!("dynamic model {:08x}: not in GTA's physics level after {TRIES} tries ({e})", body.model));
                        }
                    }
                }
            }
            if let Some(Some(model)) = self.models.get(&body.model) {
                triangles.extend(model.iter().map(|(t, surface)| { let (p, id) = body.transform(t); (p, id, *surface) }));
            } else if let Some(shape) = crate::vehicle_shapes::surveyed(body.model) {
                // No extracted bound: the host's survey of the model (vehicle_shapes.rs).
                triangles.extend(shape.iter().map(|t| { let (p, id) = body.transform(t); (p, id, 0) }));
            } else if body.allow_box {
                triangles.extend(box_triangles(&body.fallback).into_iter().map(|(p, id)| (p, id, 0)));
            }
        }
        (triangles, messages)
    }

    /// A body's loaded template in GTA world space (counterclockwise) with
    /// each triangle's packed surface, for the static build; None until
    /// `triangles` has loaded the model, or when it has no template.
    pub fn gta_triangles(&self, body: &HostBody) -> Option<Vec<([[f32; 3]; 3], u32)>> {
        let model = match self.cars.get(&body.fallback.tag) {
            Some((_, own)) => own,
            None => match self.car_models.get(&body.model) {
                Some(Some(shape)) => shape,
                _ => self.models.get(&body.model)?.as_ref()?,
            },
        };
        let mirrored = body.axes[0].cross(body.axes[1]).dot(body.axes[2]) < 0.;
        Some(model.iter().map(|(t, surface)| {
            let mut p = t.v.map(|p| (body.position + body.axes[0] * p[0] + body.axes[1] * p[1] + body.axes[2] * p[2]).to_array());
            if mirrored { p.swap(1, 2); }
            (p, *surface)
        }).collect())
    }
}

/// A raised floor this high (metres above the model's bottom) gets an
/// approach slope where it ends at the model's outline (`entry_slopes`).
const RAISED_FLOOR: std::ops::RangeInclusive<f32> = 0.03..=0.35;
/// Rise over run of an approach slope (about 14 degrees).
const ENTRY_GRADE: f32 = 0.25;

/// Approach slopes (model space) onto a raised flat floor that ends at the
/// model's outline: the half pipe's flat bottom stands 0.217 m on its base,
/// a step the board ran into at both open ends (Manatros 2026-10-19). For
/// each upward flat face edge 0.03..0.35 m up lying on the outline, a ramp
/// down to the bottom, outward. Collision only.
pub fn entry_slopes(tris: &[svwc::Tri]) -> Vec<svwc::Tri> {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for t in tris {
        for p in t.v {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    const ON: f32 = 0.06;
    let mut out = Vec::new();
    for t in tris {
        let [a, b, c] = t.v;
        let n = (Vec3::from_array(b) - Vec3::from_array(a)).cross(Vec3::from_array(c) - Vec3::from_array(a)).normalize_or_zero();
        if n.z < 0.98 || !t.v.iter().all(|p| RAISED_FLOOR.contains(&(p[2] - lo[2]))) {
            continue;
        }
        for k in 0..3 {
            let (p, q) = (t.v[k], t.v[(k + 1) % 3]);
            // The outline side both ends lie on, and its outward direction.
            let side = [(0usize, lo[0], -1.0f32), (0, hi[0], 1.0), (1, lo[1], -1.0), (1, hi[1], 1.0)]
                .into_iter()
                .find(|&(axis, at, _)| (p[axis] - at).abs() < ON && (q[axis] - at).abs() < ON);
            let Some((axis, _, sign)) = side else { continue };
            let run = |z: f32| (z - lo[2]) / ENTRY_GRADE;
            let mut p2 = p;
            let mut q2 = q;
            p2[axis] += sign * run(p[2]);
            q2[axis] += sign * run(q[2]);
            p2[2] = lo[2];
            q2[2] = lo[2];
            for mut tri in [[p, q, q2], [p, q2, p2]] {
                let m = (Vec3::from_array(tri[1]) - Vec3::from_array(tri[0])).cross(Vec3::from_array(tri[2]) - Vec3::from_array(tri[0]));
                if m.z < 0.0 {
                    tri.swap(1, 2);
                }
                out.push(svwc::Tri { v: tri, material: t.material });
            }
        }
    }
    out
}

/// GTAxSkate ObjectRails (World page, default on): standing objects get
/// rails like parked cars (worker.rs).
pub static OBJECT_RAILS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// High bit marks host entities in triangle tags; static map tags never set it.
pub const DYNAMIC_TAG: u32 = 0x8000_0000;

#[derive(Clone, Copy, Debug)]
pub struct HostBox {
    pub tag: u32,
    pub center: Vec3,
    pub rotation: Quat,
    pub half_extents: Vec3,
}

/// The 12 outward triangles of a box, in Skate space, tagged.
pub fn box_triangles(b: &HostBox) -> [([[f32; 3]; 3], u32); 12] {
    let corner = |sx: f32, sy: f32, sz: f32| {
        let local = Vec3::new(
            sx * b.half_extents.x,
            sy * b.half_extents.y,
            sz * b.half_extents.z,
        );
        coords::to_skate(b.center + b.rotation * local).to_array()
    };
    // Quads listed counterclockwise seen from outside (GTA space).
    let quads: [[(f32, f32, f32); 4]; 6] = [
        [(-1., -1., 1.), (1., -1., 1.), (1., 1., 1.), (-1., 1., 1.)], // +z
        [
            (-1., -1., -1.),
            (-1., 1., -1.),
            (1., 1., -1.),
            (1., -1., -1.),
        ], // -z
        [(1., -1., -1.), (1., 1., -1.), (1., 1., 1.), (1., -1., 1.)], // +x
        [
            (-1., -1., -1.),
            (-1., -1., 1.),
            (-1., 1., 1.),
            (-1., 1., -1.),
        ], // -x
        [(-1., 1., -1.), (-1., 1., 1.), (1., 1., 1.), (1., 1., -1.)], // +y
        [
            (-1., -1., -1.),
            (1., -1., -1.),
            (1., -1., 1.),
            (-1., -1., 1.),
        ], // -y
    ];
    let tag = b.tag | DYNAMIC_TAG;
    let mut out = [([[0.0; 3]; 3], tag); 12];
    for (i, q) in quads.iter().enumerate() {
        let p = q.map(|(x, y, z)| corner(x, y, z));
        out[i * 2] = ([p[0], p[1], p[2]], tag);
        out[i * 2 + 1] = ([p[0], p[2], p[3]], tag);
    }
    out
}

/// Boxes that are finite, non-degenerate and not absurdly large.
pub fn usable(b: &HostBox) -> bool {
    b.center.is_finite()
        && b.rotation.is_finite()
        && b.half_extents.is_finite()
        && b.half_extents.min_element() > 0.01
        && b.half_extents.max_element() < 40.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rail_cache_is_per_instance_and_uses_own_shape() {
        let mut templates = Templates { directory: PathBuf::new(), models: HashMap::new(), extents: HashMap::new(), misses: HashMap::new(), cars: HashMap::new(), car_logs: 0, car_failed: HashMap::new(), car_models: HashMap::new(), car_rails: HashMap::new(), car_rail_revision: HashMap::new(), masses: HashMap::new(), standoff: 0.0 };
        let body = HostBody {
            model: 123, position: Vec3::ZERO, axes: [Vec3::X, Vec3::Y, Vec3::Z],
            fallback: HostBox { tag: 7, center: Vec3::ZERO, rotation: Quat::IDENTITY, half_extents: Vec3::ONE },
            allow_box: false, velocity: None, grab_point: None, fixed: false, vehicle_part: false,
        };
        // A remembered undamaged model must not supply rails for this empty instance.
        templates.car_models.insert(123, Some(vec![(svwc::Tri {
            v: [[0., 0., 1.], [4., 0., 1.], [0., 4., 1.]], material: 0,
        }, 0)]));
        templates.cars.insert(7, (std::time::Instant::now(), Vec::new()));
        let mut messages = Vec::new();
        assert!(templates.vehicle_rails(&body, &mut messages).is_empty());
        assert_eq!(messages.len(), 1);
        assert!(templates.vehicle_rails(&body, &mut messages).is_empty());
        assert_eq!(messages.len(), 1, "unchanged cache does not log again");
        let mut other = body.clone();
        other.fallback.tag = 8;
        templates.car_rails.insert(8, vec![vec![Vec3::X, Vec3::Y]]);
        assert_eq!(templates.vehicle_rails(&other, &mut messages), vec![vec![Vec3::X, Vec3::Y]]);
        assert!(templates.vehicle_rails(&body, &mut messages).is_empty());
        let tri = (svwc::Tri { v: [[0.; 3]; 3], material: 0 }, 0);
        assert!(same_shape(&[tri], &[tri]));
        let mut damaged = tri;
        damaged.0.v[0][0] = 1.;
        assert!(!same_shape(&[tri], &[damaged]), "equal counts do not hide damage");
        templates.cache_car_shape(7, vec![tri]);
        templates.vehicle_rails(&body, &mut messages);
        let revision = templates.car_rail_revision[&7];
        templates.cache_car_shape(7, vec![tri]);
        assert!(templates.car_rails.contains_key(&7), "unchanged shape retains rails");
        templates.cache_car_shape(7, vec![damaged]);
        assert!(!templates.car_rails.contains_key(&7), "changed shape invalidates rails");
        templates.vehicle_rails(&body, &mut messages);
        assert_eq!(templates.car_rail_revision[&7], revision + 1);
        assert!(templates.car_rails.contains_key(&8), "other instance stays cached");
    }

    /// The dropper's props on flat ground through the static build: the
    /// map's rail finder must give the grindable ones rails. Uses the
    /// installed world cache's templates; skipped where they are absent.
    #[test]
    fn dropped_props_built_into_the_static_area_get_rails() {
        let Some(cache_path) = std::env::var_os("SKATEV_TEST_CACHE") else { return };
        let cache = std::path::Path::new(&cache_path);
        if !cache.with_extension("prop-models").is_dir() { return; }
        let mut models = Templates::new(cache);
        let fallback = HostBox { tag: 5, center: Vec3::ZERO, rotation: Quat::IDENTITY, half_extents: Vec3::ONE };
        let mut found = Vec::new();
        for (name, model) in [("kicker", 0x731F4403u32), ("flat ramp", 0x7DBDB93C), ("quarter pipe", 0x326D3DB1), ("half pipe", 0xF66F582F),
                              ("spine", 0x8AE73FD0), ("funbox", 0x4EFA1782), ("barrier", 0xF3C33285), ("picnic table", 0xA24CE57E),
                              ("bench", 0x6BA514AC), ("rail", 0x03154DDC)] {
            let body = HostBody { model, position: Vec3::new(3., 4., 0.), axes: [Vec3::X, Vec3::Y, Vec3::Z], fallback, allow_box: false,
                                  velocity: None, grab_point: None, fixed: true, vehicle_part: false };
            models.triangles(&[body.clone()], None);
            let Some(prop) = models.gta_triangles(&body) else { found.push(format!("{name}: no template")); continue };
            let ground = [[[-40., -40., 0.], [40., -40., 0.], [40., 40., 0.]], [[-40., -40., 0.], [40., 40., 0.], [-40., 40., 0.]]];
            let area = crate::world::build_area_tagged(Vec3::ZERO, ground.into_iter().map(|t| (t, 0)).chain(prop.iter().copied()));
            let on_prop = area.rails.iter().filter(|r| r.iter().any(|p| {
                let g = crate::coords::from_skate(Vec3::from_array(*p));
                (g.x - 3.).abs() < 6. && (g.y - 4.).abs() < 6. && g.z > 0.05
            })).count();
            found.push(format!("{name}: {} triangles, {on_prop} rails on it", prop.len()));
            if name == "rail" || name == "bench" || name == "barrier" {
                assert!(on_prop > 0, "{name} must be grindable: {found:?}");
            }
        }
        println!("{}", found.join("\n"));
    }

    /// A parked car's template through the rail finder (worker: parked
    /// vehicles' rails join the grind world): its roof/hood/trunk edges.
    #[test]
    fn a_parked_cars_template_gives_rails_on_its_top() {
        let Some(cache_path) = std::env::var_os("SKATEV_TEST_CACHE") else { return };
        let cache = std::path::Path::new(&cache_path);
        if !cache.with_extension("prop-models").join("1cbdc10b.svwc").is_file() { return; }
        let mut models = Templates::new(cache);
        let fallback = HostBox { tag: 1, center: Vec3::ZERO, rotation: Quat::IDENTITY, half_extents: Vec3::ONE };
        let body = HostBody { model: 0x1cbdc10b, position: Vec3::new(10., 20., 0.5), axes: [Vec3::X, Vec3::Y, Vec3::Z], fallback,
                              allow_box: false, velocity: None, grab_point: None, fixed: false, vehicle_part: false };
        models.triangles(&[body.clone()], None);
        let tris: Vec<[Vec3; 3]> = models.gta_triangles(&body).unwrap().iter().map(|(p, _)| p.map(Vec3::from_array)).collect();
        let (rails, _) = crate::rails::find_metres(&tris);
        let top = tris.iter().flatten().map(|v| v.z).fold(f32::MIN, f32::max);
        let high = rails.iter().filter(|r| r.iter().all(|p| p.z > 0.5 + 0.5)).count();
        println!("car 1cbdc10b: {} rails, {high} above 1 m (top {top:.2})", rails.len());
        assert!(high > 0, "a car gets rails along its upper edges");
    }

    #[test]
    fn a_vehicle_shape_is_remembered_on_disk_for_the_next_car_of_that_model() {
        let root = std::env::temp_dir().join(format!("skatev-car-memory-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        let shape = vec![(svwc::Tri { v: [[0., 0., 1.], [1., 0., 1.], [0., 1., 1.]], material: 3 }, 7u32)];
        let mut first = Templates::new(&root.join("world.svwc"));
        let mut messages = Vec::new();
        first.remember_model(0xabc, &shape, &mut messages);
        assert!(messages[0].contains("saved"), "{messages:?}");
        let mut next = Templates::new(&root.join("world.svwc"));
        let loaded = next.model_shape(0xabc).expect("read back from its file");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0.v, shape[0].0.v);
        assert!(next.model_shape(0xdef).is_none(), "an unknown model has none");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cached_exact_mesh_wins_over_box_and_follows_entity_transform() {
        let root = std::env::temp_dir().join(format!("skatev-model-test-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let directory = root.join("world.prop-models");
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("00000001.svwc");
        let authored = svwc::Tri { v: [[2.,0.,0.], [2.,1.,0.], [2.,0.,1.]], material: 3 };
        svwc::write(&mut std::fs::File::create(&file).unwrap(), &[authored], 64.).unwrap();
        let fallback = HostBox { tag: 19, center: Vec3::ZERO, rotation: Quat::IDENTITY, half_extents: Vec3::ONE };
        let mut body = HostBody { model: 1, position: Vec3::ZERO, axes: [Vec3::X, Vec3::Y, Vec3::Z], fallback, allow_box: true, velocity: None, grab_point: None, fixed: false, vehicle_part: false };
        let mut models = Templates::new(&root.join("world.svwc"));
        let (first, messages) = models.triangles(&[body.clone()], None);
        assert_eq!(first[0].2, crate::materials::surface(3).packed());
        assert_eq!(first[0].1, 19 | DYNAMIC_TAG);
        assert_eq!(first.len(), 1, "a model's opening must not be filled by its fallback box");
        assert_eq!(messages.len(), 1);
        std::fs::remove_file(&file).unwrap();
        body.position = Vec3::new(10., 20., 30.);
        let (moved, messages) = models.triangles(&[body.clone()], None);
        assert_eq!(moved.len(), 1, "model data is cached independently of moving instances");
        assert!(messages.is_empty());
        assert_eq!(moved[0].0[0], [12., 30., -20.]);
        body.model = 2;
        body.allow_box = false;
        assert!(models.triangles(&[body.clone()], None).0.is_empty());
        body.allow_box = true;
        assert_eq!(models.triangles(&[body], None).0.len(), 12, "missing models need explicit fallback permission");
        assert!(models.triangles(&[], None).0.is_empty(), "removed entities leave no geometry");
        std::fs::remove_dir(&directory).unwrap();
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn exact_bounds_preserve_gaps_and_full_affine_transform() {
        let fallback = HostBox { tag: 0x21234567, center: Vec3::ZERO, rotation: Quat::IDENTITY, half_extents: Vec3::ONE };
        let body = HostBody { model: 1, position: Vec3::new(10.,20.,30.), axes: [Vec3::Y*2., -Vec3::X, Vec3::Z], fallback, allow_box: true, velocity: None, grab_point: None, fixed: false, vehicle_part: false };
        let t = svwc::Tri { v: [[2.,0.,0.], [2.,1.,0.], [2.,0.,1.]], material: 3 };
        let (points, tag) = body.transform(&t);
        assert_eq!(tag, 0xA1234567);
        assert_eq!(points, [[10.,30.,-24.], [9.,30.,-24.], [10.,31.,-24.]]);
        // Geometry stays the authored side face; no bounding-box face fills the opening at local x=0.
        assert!(points.iter().all(|p| p[2] == -24.));
    }

    #[test]
    fn grab_line_spans_template_width_at_bone_height_behind_the_body_and_skips_non_vehicles() {
        let mut t = Templates { directory: PathBuf::new(), models: HashMap::new(), extents: HashMap::new(), misses: HashMap::new(), cars: HashMap::new(), car_logs: 0, car_failed: HashMap::new(), car_models: HashMap::new(), car_rails: HashMap::new(), car_rail_revision: HashMap::new(), masses: HashMap::new(), standoff: 0.0 };
        t.extents.insert(7, (Vec3::new(-1.0, -2.5, -0.6), Vec3::new(1.0, 2.5, 0.8)));
        let fallback = HostBox { tag: 0x0000_0042, center: Vec3::ZERO, rotation: Quat::IDENTITY, half_extents: Vec3::ONE };
        let body = HostBody { model: 7, position: Vec3::new(10.0, 20.0, 1.0), axes: [Vec3::X, Vec3::Y, Vec3::Z], fallback,
            allow_box: false, velocity: Some((Vec3::new(0.0, 5.0, 0.0), Vec3::ZERO)), grab_point: Some(Vec3::new(10.2, 17.6, 0.7)), fixed: false, vehicle_part: false };
        let lines = t.grab_lines(&[body.clone()]);
        assert_eq!(lines.len(), 1);
        let l = &lines[0];
        assert_eq!(l.tag, 0x42 | DYNAMIC_TAG);
        assert_eq!(l.points[0][0], -1.0);
        assert_eq!(l.points[1][0], 1.0);
        // The bone is 0.1 m inside the body's rear (-2.5): the line is on the rear.
        assert!((l.points[0][1] + 2.5).abs() < 1e-5 && (l.points[0][2] + 0.3).abs() < 1e-5);
        assert_eq!(l.approach, [0.0, -1.0, 0.0, 0.0]);
        // A standoff moves the wall straight back, clear of the bodywork.
        t.standoff = 0.3;
        assert!((t.grab_lines(&[body.clone()])[0].points[1][1] + 2.8).abs() < 1e-5);
        t.standoff = 0.0;
        assert_eq!(l.frame[3], [10.0, 1.0, -20.0, 0.0]);
        assert_eq!(l.velocity, [0.0, 0.0, -5.0, 0.0]);
        let mut ped = body.clone();
        ped.fallback.tag = 1 << 28 | 0x42;
        assert!(t.grab_lines(&[ped]).is_empty());
        // No bone: a line at NO_BONE_HEIGHT above the underside (-0.6 + 0.6).
        let mut no_bone = body.clone();
        no_bone.grab_point = None;
        let l = &t.grab_lines(&[no_bone.clone()])[0];
        assert!((l.points[0][2] - 0.0).abs() < 1e-5 && (l.points[0][1] + 2.5).abs() < 1e-5);
        // No template either: the host box (here 1 x 2.5 x 0.7 half extents, centred 0.1 up) stands in.
        let mut boxed = no_bone;
        boxed.model = 99;
        boxed.fallback.center = Vec3::new(10.0, 20.0, 1.1);
        boxed.fallback.half_extents = Vec3::new(1.0, 2.5, 0.7);
        let l = &t.grab_lines(&[boxed])[0];
        assert!((l.points[0][0] + 1.0).abs() < 1e-5 && (l.points[1][0] - 1.0).abs() < 1e-5);
        assert!((l.points[0][1] + 2.5).abs() < 1e-5 && l.points[0][2].abs() < 1e-5);
    }

    #[test]
    fn dynamic_body_abi_is_stable_and_rejects_singular_transforms() {
        assert_eq!(std::mem::size_of::<crate::SvDynamicBody>(), 144);
        assert!(HostBody::from_abi(&crate::SvDynamicBody::default()).is_none());
        let axis = |x, y, z| crate::SvVec3 { x, y, z };
        let mut b = crate::SvDynamicBody {
            size: 144,
            right: axis(1.0, 0.0, 0.0),
            forward: axis(0.0, 1.0, 0.0),
            up: axis(0.0, 0.0, 1.0),
            linear_velocity: axis(3.0, 0.0, 0.0),
            angular_velocity: axis(0.0, 0.0, 1.0),
            ..Default::default()
        };
        b.fallback.rotation.w = 1.0;
        b.grab_point = axis(0.0, -2.0, 0.5);
        assert_eq!(HostBody::from_abi(&b).unwrap().velocity, Some((Vec3::new(3.0, 0.0, 0.0), Vec3::Z)));
        assert_eq!(HostBody::from_abi(&b).unwrap().grab_point, None, "no grab line without its flag");
        b.grab_flags = 1;
        assert_eq!(HostBody::from_abi(&b).unwrap().grab_point, Some(Vec3::new(0.0, -2.0, 0.5)));
        b.linear_velocity.x = f32::NAN;
        assert_eq!(HostBody::from_abi(&b).unwrap().velocity, None, "non-finite samples are dropped");
    }

    #[test]
    fn masses_parse_and_proxies_use_authored_mass_and_box_inertia() {
        let masses = parse_masses("# header
eb70965f 1200.000 1.2 1.2 1.3 0 0.05 -0.25  # blista BLISTA
bad line
000b75b9 2.000 1 1 1 0 0 0
");
        assert_eq!(masses.len(), 2);
        assert_eq!(masses[&0xeb70965f].kg, 1200.0);
        let mut t = Templates { directory: PathBuf::new(), models: HashMap::new(), extents: HashMap::new(), misses: HashMap::new(), cars: HashMap::new(), car_logs: 0, car_failed: HashMap::new(), car_models: HashMap::new(), car_rails: HashMap::new(), car_rail_revision: HashMap::new(), masses, standoff: 0.0 };
        let fallback = HostBox { tag: 9, center: Vec3::new(5.0, 6.0, 1.0), rotation: Quat::IDENTITY, half_extents: Vec3::new(1.0, 2.0, 0.5) };
        let body = HostBody { model: 0xeb70965f, position: Vec3::new(5.0, 6.0, 0.3), axes: [Vec3::X, Vec3::Y, Vec3::Z], fallback,
            allow_box: true, velocity: Some((Vec3::new(0.0, 8.0, 0.0), Vec3::new(0.0, 0.0, 0.5))), grab_point: None, fixed: false, vehicle_part: false };
        let p = t.proxies(&[body.clone()]);
        assert_eq!(p.len(), 1);
        assert!((p[0].inverse_mass - 1.0 / 1200.0).abs() < 1e-9);
        // Box inertia about local x: m/3 (hy^2 + hz^2) * multiplier.
        let ix = 1200.0 / 3.0 * (4.0 + 0.25) * 1.2;
        assert!((1.0 / p[0].inverse_inertia[0] - ix).abs() < 1e-2 * ix);
        // GTA velocity (0, 8, 0) is Skate (0, 0, -8); spin about GTA z is about Skate y.
        assert_eq!(p[0].linear_velocity, [0.0, 0.0, -8.0]);
        assert_eq!(p[0].angular_velocity, [0.0, 0.5, 0.0]);
        // Peds never become proxies; an unknown mass is kinematic.
        let mut ped = body.clone();
        ped.fallback.tag = 1 << 28 | 3;
        assert!(t.proxies(&[ped]).is_empty());
        let mut unknown = body.clone();
        unknown.model = 42;
        assert_eq!(t.proxies(&[unknown])[0].inverse_mass, 0.0);
        t.masses.clear();
    }

    #[test]
    fn box_faces_point_outward_in_skate_space() {
        let b = HostBox {
            tag: 7,
            center: Vec3::new(10.0, -20.0, 3.0),
            rotation: Quat::from_rotation_z(0.6),
            half_extents: Vec3::new(2.0, 1.0, 0.75),
        };
        let center = coords::to_skate(b.center);
        for (t, tag) in box_triangles(&b) {
            assert_eq!(tag, 7 | DYNAMIC_TAG);
            let [p0, p1, p2] = t.map(Vec3::from_array);
            let n = (p1 - p0).cross(p2 - p0);
            let mid = (p0 + p1 + p2) / 3.0;
            assert!(n.dot(mid - center) > 0.0, "inward face {t:?}");
        }
    }

    #[test]
    fn top_face_is_at_box_top() {
        let b = HostBox {
            tag: 1,
            center: Vec3::new(0.0, 0.0, 1.0),
            rotation: Quat::IDENTITY,
            half_extents: Vec3::splat(0.5),
        };
        let tris = box_triangles(&b);
        // GTA z = 1.5 -> skate y = 1.5 for the first (+z) face.
        assert!(tris[0].0.iter().all(|p| (p[1] - 1.5).abs() < 1e-6));
    }

    #[test]
    fn rejects_degenerate_and_huge_boxes() {
        let ok = HostBox {
            tag: 0,
            center: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            half_extents: Vec3::ONE,
        };
        assert!(usable(&ok));
        assert!(!usable(&HostBox {
            half_extents: Vec3::new(1.0, 1.0, 0.0),
            ..ok
        }));
        assert!(!usable(&HostBox {
            half_extents: Vec3::splat(100.0),
            ..ok
        }));
        assert!(!usable(&HostBox {
            center: Vec3::NAN,
            ..ok
        }));
    }
}

#[cfg(test)]
mod dropper_shape_probe {
    use super::*;

    /// Prints each dropper prop template's lowest point and the lowest edge
    /// heights (how far a ramp's toe stands above the template's bottom).
    #[test]
    #[ignore]
    fn dropper_templates_bottoms() {
        let cache_path = std::env::var_os("SKATEV_TEST_CACHE").expect("set SKATEV_TEST_CACHE to the local world cache");
        let cache = std::path::Path::new(&cache_path);
        let dir = cache.with_extension("prop-models");
        for (name, model) in [("kicker", 0x731F4403u32), ("flat ramp", 0x7DBDB93C), ("quarter pipe", 0x326D3DB1), ("half pipe", 0xF66F582F),
                              ("spine", 0x8AE73FD0), ("funbox", 0x4EFA1782), ("rail", 0x03154DDC), ("bench", 0x6BA514AC)] {
            let Ok(tris) = svwc::Cache::open(&dir.join(format!("{model:08x}.svwc"))).and_then(|mut c| c.read_all()) else { println!("{name}: no template"); continue };
            let zs: Vec<f32> = tris.iter().flat_map(|t| t.v.iter().map(|p| p[2])).collect();
            let lo = zs.iter().copied().fold(f32::MAX, f32::min);
            let hi = zs.iter().copied().fold(f32::MIN, f32::max);
            // Upward riding faces (normal z > 0.2): lowest vertex above the bottom,
            // and the flat (n.z > 0.98) faces' heights (the deck the board rolls on first).
            let normal = |t: &svwc::Tri| {
                let [a, b, c] = t.v;
                let u = [b[0]-a[0], b[1]-a[1], b[2]-a[2]]; let v = [c[0]-a[0], c[1]-a[1], c[2]-a[2]];
                let n = [u[1]*v[2]-u[2]*v[1], u[2]*v[0]-u[0]*v[2], u[0]*v[1]-u[1]*v[0]];
                let l = (n[0]*n[0]+n[1]*n[1]+n[2]*n[2]).sqrt().max(1e-9);
                n[2] / l
            };
            let up: Vec<&svwc::Tri> = tris.iter().filter(|t| normal(t) > 0.2).collect();
            let low_up = up.iter().flat_map(|t| t.v.iter().map(|p| p[2])).fold(f32::MAX, f32::min) - lo;
            let mut flats: Vec<f32> = up.iter().filter(|t| normal(t) > 0.98).map(|t| t.v[0][2] - lo).collect();
            flats.sort_by(f32::total_cmp); flats.dedup_by(|a, b| (*a - *b).abs() < 0.005);
            let steps: Vec<f32> = vec![low_up];
            let near: Vec<String> = flats.iter().take(6).map(|s| format!("{:.3}", s)).collect();
            if name == "kicker" || name == "half pipe" {
                let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
                for t in &tris { for p in t.v { x0 = x0.min(p[0]); x1 = x1.max(p[0]); y0 = y0.min(p[1]); y1 = y1.max(p[1]); } }
                println!("  {name} footprint x {x0:.2}..{x1:.2} y {y0:.2}..{y1:.2}");
                for t in up.iter().filter(|t| t.v.iter().all(|p| p[2] - lo < 0.45)) {
                    let xs = t.v.map(|p| p[0]); let ys = t.v.map(|p| p[1]); let zs = t.v.map(|p| p[2] - lo);
                    println!("  {name} up n.z {:.2} x {:.2}..{:.2} y {:.2}..{:.2} z {:.3}..{:.3}", normal(t),
                        xs.iter().copied().fold(f32::MAX, f32::min), xs.iter().copied().fold(f32::MIN, f32::max),
                        ys.iter().copied().fold(f32::MAX, f32::min), ys.iter().copied().fold(f32::MIN, f32::max),
                        zs.iter().copied().fold(f32::MAX, f32::min), zs.iter().copied().fold(f32::MIN, f32::max));
                }
            }
            println!("{name}: {} tris, z {lo:.3}..{hi:.3}; riding faces start {:.3} above the bottom; flat levels {:?}", tris.len(), steps[0], near);
        }
    }
}

#[cfg(test)]
mod entry_slope_tests {
    use super::*;

    #[test]
    fn a_raised_floor_at_the_outline_gets_a_slope_down_to_the_bottom() {
        // A 4 x 4 m box base with its flat top 0.2 m up: four sides, slopes outward.
        let top = [[0., 0., 0.2], [4., 0., 0.2], [4., 4., 0.2], [0., 4., 0.2]];
        let mut tris = vec![svwc::Tri { v: [top[0], top[1], top[2]], material: 3 }, svwc::Tri { v: [top[0], top[2], top[3]], material: 3 }];
        tris.push(svwc::Tri { v: [[0., 0., 0.], [4., 0., 0.], [4., 4., 0.]], material: 3 }); // the bottom
        let slopes = entry_slopes(&tris);
        assert_eq!(slopes.len(), 8, "two triangles on each of the four outline edges");
        for t in &slopes {
            let lowest = t.v.iter().map(|p| p[2]).fold(f32::MAX, f32::min);
            assert_eq!(lowest, 0.0, "each slope reaches the bottom");
            assert!(t.v.iter().any(|p| p[0] < -0.5 || p[0] > 4.5 || p[1] < -0.5 || p[1] > 4.5), "outward");
        }
        // A floor at the bottom or a high seat gets none.
        let flat = vec![svwc::Tri { v: [[0., 0., 0.], [4., 0., 0.], [4., 4., 0.]], material: 3 }];
        assert!(entry_slopes(&flat).is_empty());
    }

    #[test]
    #[ignore]
    fn the_half_pipe_gets_slopes_at_its_open_ends() {
        let dir = std::env::var_os("SKATEV_TEST_PROP_MODELS").expect("set SKATEV_TEST_PROP_MODELS to the local prop templates");
        let path = std::path::Path::new(&dir).join("f66f582f.svwc");
        let tris = svwc::Cache::open(&path).and_then(|mut c| c.read_all()).expect("half pipe template");
        let slopes = entry_slopes(&tris);
        println!("half pipe: {} slope triangles", slopes.len());
        assert!(!slopes.is_empty());
    }
}

#[cfg(test)]
mod dropper_park_repro {
    use super::*;

    /// Manatros' 2026-10-19 22:35 drops at the Vespucci park on the prebaked
    /// map: how many rails each prop gets, and why its edges are rejected.
    #[test]
    #[ignore]
    fn vespucci_drops_get_rails() {
        let cache_path = std::env::var_os("SKATEV_TEST_CACHE").expect("set SKATEV_TEST_CACHE to the local world cache");
        let cache = std::path::Path::new(&cache_path);
        crate::baked::open(cache).expect("prebaked map");
        let mut models = Templates::new(cache);
        let drops = [("rail", 0x03154DDCu32, -1333.57f32, -1431.39f32, 3.31f32, 19f32), ("rail", 0x03154DDC, -1333.62, -1436.45, 3.31, 341.),
                     ("bench", 0x6BA514AC, -1334.93, -1427.40, 3.34, 285.), ("barrier", 0xF3C33285, -1335.31, -1435.66, 3.31, 279.),
                     ("picnic", 0xA24CE57E, -1338.10, -1422.26, 3.31, 315.)];
        let centre = [-1336.0f32, -1430.0f32];
        let ground: Vec<([[f32; 3]; 3], u32)> = crate::baked::around(centre, 40.0).into_iter().map(|t| (t.v, 0)).collect();
        let mut tris = ground.clone();
        for (i, &(_, model, x, y, z, h)) in drops.iter().enumerate() {
            let r = h.to_radians();
            let axes = [Vec3::new(r.cos(), r.sin(), 0.), Vec3::new(-r.sin(), r.cos(), 0.), Vec3::Z];
            let fallback = HostBox { tag: 10 + i as u32, center: Vec3::ZERO, rotation: Quat::IDENTITY, half_extents: Vec3::ONE };
            let body = HostBody { model, position: Vec3::new(x, y, z), axes, fallback, allow_box: false, velocity: None, grab_point: None, fixed: true, vehicle_part: false };
            models.triangles(&[body.clone()], None);
            tris.extend(models.gta_triangles(&body).expect("template"));
        }
        let at = Vec3::new(-1334.5, -1432.0, 3.6);
        crate::rails::set_diagnostics(Some(at));
        let area = crate::world::build_area_tagged(Vec3::new(centre[0], centre[1], 3.3), tris.into_iter());
        crate::rails::set_diagnostics(None);
        for &(name, _, x, y, z, _) in &drops {
            let near: Vec<String> = area.rails.iter().filter(|r| r.iter().any(|p| {
                let g = crate::coords::from_skate(Vec3::from_array(*p));
                (g.x - x).abs() < 2.5 && (g.y - y).abs() < 2.5 && g.z > z + 0.1
            })).map(|r| {
                let a = crate::coords::from_skate(Vec3::from_array(r[0])); let b = crate::coords::from_skate(Vec3::from_array(*r.last().unwrap()));
                format!("({:.2},{:.2},{:.2})-({:.2},{:.2},{:.2})", a.x, a.y, a.z, b.x, b.y, b.z)
            }).collect();
            println!("{name} at ({x}, {y}): {} rails {:?}", near.len(), near);
        }
        let mut reasons: Vec<String> = crate::rails::take_rejected().iter().filter(|r| r.mid.z > 3.45).map(|r| format!("({:.2},{:.2},{:.2}) {:.2} m {}", r.mid.x, r.mid.y, r.mid.z, r.length, r.reason)).collect();
        reasons.sort();
        for r in reasons { println!("  rejected {r}"); }
    }
}

#[cfg(test)]
mod dropper_regression_20261021 {
    use super::*;
    use std::collections::BTreeSet;

    fn segments(rails: &[Vec<Vec3>]) -> Vec<([i32; 3], [i32; 3])> {
        let mut out = Vec::new();
        for rail in rails {
            for w in rail.windows(2) {
                let key = |p: Vec3| p.to_array().map(|v| (v * 10000.).round() as i32);
                let (a, b) = (key(w[0]), key(w[1]));
                out.push(if a < b { (a, b) } else { (b, a) });
            }
        }
        out.sort();
        out
    }

    #[test]
    fn coplanar_table_top_has_no_diagonal_grind_seam() {
        let a = Vec3::new(-1., -0.5, 0.8);
        let b = Vec3::new(1., -0.5, 0.8);
        let c = Vec3::new(1., 0.5, 0.8);
        let d = Vec3::new(-1., 0.5, 0.8);
        // A separate low triangle establishes the object's bottom.
        let tris = [[a,b,c], [a,c,d], [Vec3::ZERO, Vec3::X * 0.01, Vec3::Y * 0.01]];
        let rails = object_rails(&tris);
        for rail in &rails {
            for w in rail.windows(2) {
                let mid = (w[0]+w[1])*0.5;
                assert!(!(mid.z > 0.7 && mid.x.abs() < 0.9 && mid.y.abs() < 0.4), "coplanar interior seam: {w:?}");
            }
        }
    }

    /// Local asset regression: run explicitly on the test machine. No assets
    /// are checked in, and missing templates are an error rather than a pass.
    #[test]
    #[ignore = "requires the locally installed GTA dropper collision templates"]
    fn installed_dropper_templates_keep_stable_nonduplicate_rails() {
        let directory = std::env::var_os("SKATEV_TEST_PROP_MODELS").expect("set SKATEV_TEST_PROP_MODELS to the local prop templates");
        let dir = std::path::Path::new(&directory);
        for (name, model) in [("picnic",0xa24ce57eu32),("flat ramp",0x7dbdb93c),("half pipe",0xf66f582f),("rail",0x03154ddc)] {
            let path = dir.join(format!("{model:08x}.svwc"));
            let tris = svwc::Cache::open(&path).and_then(|mut c| c.read_all()).expect("installed dropper template");
            let faces: Vec<_> = tris.iter().map(|t| t.v.map(Vec3::from_array)).collect();
            let first = object_rails(&faces);
            let baseline = segments(&first);
            assert!(!baseline.is_empty(), "{name}: no grindable edges");
            assert_eq!(baseline.len(), baseline.iter().copied().collect::<BTreeSet<_>>().len(), "{name}: duplicate grind segments");
            let bottom = faces.iter().flatten().map(|p|p.z).fold(f32::MAX,f32::min);
            assert!(first.iter().flatten().all(|p| p.is_finite() && p.z > bottom + BASE_CLEARANCE), "{name}: base/invalid rail");
            let start = std::time::Instant::now();
            for _ in 0..32 { assert_eq!(segments(&object_rails(&faces)), baseline, "{name}: unstable repeated rails"); }
            println!("{name}: {} source triangles, {} rails, {} segments; repeated rail extraction {:.3} ms/call", faces.len(),first.len(),baseline.len(),start.elapsed().as_secs_f64()*1000./32.);
        }
    }
}
