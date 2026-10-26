//! Static GTA collision around the skater, as Skate collision + grind rails.
//! The static map is GTA's own loaded collision, read from its physics level
//! (`live.rs`), never from per-frame GTA raycasts; moving GTA entities are
//! `dynamic.rs`. Beside the cache path only the sidecars are read (`.svsd`
//! surfaces, `crackmaps.svgj` ground joints), by triangle vertex bits and
//! position.
use crate::{coords, curbs, rails};
use bevy_math::Vec3;
use std::path::Path;

/// Half-size of the XY square of collision built around the skater.
pub static OPTIONS_VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub fn options_version() -> u64 { OPTIONS_VERSION.load(std::sync::atomic::Ordering::Acquire) }

pub const RADIUS: f32 = 128.0;
/// A grind line keeps a point only where it leaves the straight path by
/// more than this (metres): straight rails get straight lines.
const STRAIGHT: f32 = 0.03;
/// The same for track edge lines: a steel head is 7 cm wide, so a 3 cm
/// chord on a curve left it (Codex 2026-10-25, mark 11: 25 of 57 samples
/// of one edge line over the bed, not the steel).
const TRACK_STRAIGHT: f32 = 0.005;
/// Shared bake tag; grind probes ignore these faces.
pub const NO_GRIND_TILE_TAG: u32 = skate_host::bridge::NO_GRIND_TAG;
/// Horizontal drift from the built centre that triggers a rebuild.
pub const RECENTRE: f32 = 64.0;

pub struct Area {
    pub center: Vec3,
    /// Skate space (Y up), counterclockwise.
    pub triangles: Vec<[[f32; 3]; 3]>,
    /// Packed Skate audio/physics/seam surface per triangle, plus
    /// `NO_GRIND_TAG` for curb bevels (grind probes ignore them).
    pub tags: Vec<u32>,
    /// Skate space polylines.
    pub rails: Vec<Vec<[f32; 3]>>,
    /// Stair nose lines (Skate space), grind rails only with StepRails on.
    pub noses: Vec<Vec<[f32; 3]>>,
    pub census: rails::RailCensus,
    pub bevels: curbs::BevelCensus,
    /// Where the triangles came from (live physics level census), for the log.
    pub note: String,
    /// GTA's bounds the triangles were read from (`live::changed_near`).
    pub bounds: Vec<crate::live::Seen>,
    /// Watch GTA state changes even when a live patch currently has no bounds.
    pub live_required: bool,
    /// Triangles new or gone since the last build (`reuse`); None: built
    /// from scratch.
    pub changed: Option<usize>,
    /// StaticWorld::set_fixed version whose triangles this area holds.
    pub fixed: u64,
    pub options: u64,
    /// Per triangle its origin ("tile x_y: file", "live GTA collision", ...;
    /// design 2026-10-26-one-source-collision 6.4). Empty: not recorded.
    pub origins: crate::spot::Origins,
}

/// Installed collision retained for coverage checks and view publication.
impl Area {
    pub fn needs_live_watch(&self) -> bool {
        self.live_required || !self.bounds.is_empty()
    }

    pub fn has_floor(&self, at: Vec3) -> bool {
        self.triangles.iter().any(|t| {
            let t = svwc::Tri { v: t.map(|p| coords::from_skate(Vec3::from_array(p)).to_array()), material: 0 };
            svwc::clean::floor_at(&t, [at.x, at.y]).is_some_and(|z| (z - at.z).abs() < 4.0)
        })
    }

    /// Only loss of previously available nearby support blocks a swap.
    /// High airborne positions with no old floor remain valid.
    pub fn loses_floor(&self, old: &Area, at: Vec3, predicted: Vec3) -> bool {
        [at, predicted].into_iter().any(|p| old.has_floor(p) && !self.has_floor(p))
    }

    pub fn publish_view(&self) {
        crate::collision_view::publish_area(&self.triangles, &self.tags, &self.rails);
        crate::spot::install(self);
    }
}

/// Restore rail continuity after tile clipping and fixed/live source assembly.
/// Normalization runs on the area builder, never on the simulation tick. GTA
/// space is required by the rail crown/deduplication rules (Z up, metres).
/// Grind segments an area's static lines may use: Skate's grind index holds
/// 65,535 entries, cars and props need the rest. Over it, the whole area
/// failed and the previous collision stayed (2026-10-25 test).
pub const STATIC_RAIL_SEGMENTS: usize = 52_000;

/// Drops the lines farthest from `centre` (Skate coordinates) until at most
/// `max` segments remain; returns how many lines went.
pub fn cap_rail_segments(rails: &mut Vec<Vec<[f32; 3]>>, centre: Vec3, max: usize) -> usize {
    let total: usize = rails.iter().map(|r| r.len().saturating_sub(1)).sum();
    if total <= max {
        return 0;
    }
    let distance = |r: &Vec<[f32; 3]>| r.iter().map(|p| Vec3::from_array(*p).distance(centre)).fold(f32::MAX, f32::min);
    let mut order: Vec<usize> = (0..rails.len()).collect();
    order.sort_by(|&a, &b| distance(&rails[a]).total_cmp(&distance(&rails[b])));
    let (mut used, mut keep) = (0usize, vec![false; rails.len()]);
    for i in order {
        let n = rails[i].len().saturating_sub(1);
        if used + n <= max { used += n; keep[i] = true; }
    }
    let before = rails.len();
    let mut k = keep.into_iter();
    rails.retain(|_| k.next().unwrap_or(false));
    before - rails.len()
}

/// Lines lying wholly within `tolerance` of a longer kept line go (copies).
fn drop_exact_copies(rails: &mut Vec<Vec<Vec3>>, tolerance: f32) {
    let length = |r: &Vec<Vec3>| r.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>();
    let mut order: Vec<usize> = (0..rails.len()).collect();
    order.sort_by(|&a, &b| length(&rails[b]).total_cmp(&length(&rails[a])));
    let near = |p: Vec3, line: &[Vec3]| line.windows(2).any(|w| {
        let ab = w[1] - w[0];
        let t = ((p - w[0]).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
        (w[0] + ab * t).distance(p) <= tolerance
    });
    let mut kept: Vec<usize> = Vec::new();
    for i in order {
        let r = &rails[i];
        let copy = kept.iter().any(|&k| {
            let samples = r.windows(2).flat_map(|w| [w[0], (w[0] + w[1]) * 0.5]).chain(r.last().copied());
            samples.into_iter().all(|p| near(p, &rails[k]))
        });
        if !copy { kept.push(i); }
    }
    kept.sort_unstable();
    let mut k = kept.into_iter().peekable();
    let mut i = 0;
    rails.retain(|_| { let keep = k.peek() == Some(&i); if keep { k.next(); } i += 1; keep });
}

fn normalize_assembled_rails(rails: &mut Vec<Vec<[f32; 3]>>) {
    let mut gta: Vec<Vec<Vec3>> = rails.iter().map(|r| r.iter()
        .map(|&p| coords::from_skate(Vec3::from_array(p))).collect()).collect();
    // Trim before coverage sampling, then join even tiny uncovered tails:
    // otherwise the 9 cm coverage radius can discard their contribution.
    // Tile seams give exact copies (one source, split at a border): only
    // those go here. The 9 cm rail dedupe fused a track's two edge lines
    // (4-9 cm apart) and moved lines away from their baked collision
    // (Codex analysis 2026-10-25).
    crate::rails::trim_overlaps(&mut gta);
    crate::rails::join_ends(&mut gta);
    drop_exact_copies(&mut gta, 0.02);
    crate::rails::join_ends(&mut gta);
    // Every line was simplified in its tile; only the joins are new. A
    // 3 cm chord here undid the track edges' tighter fit (TRACK_STRAIGHT).
    crate::rails::straighten(&mut gta, TRACK_STRAIGHT);
    *rails = gta.into_iter().map(|r| r.into_iter()
        .map(|p| coords::to_skate(p).to_array()).collect()).collect();
}

pub struct StaticWorld {
    /// Recovered sound surfaces beside the cache (`.svsd`), if baked.
    recovered: crate::surfaces::Recovered,
    /// Visual ground with texture joint maps beside the cache (`crackmaps.svgj`).
    ground: crate::ground::GroundSource,
    /// GTA's own loaded collision (`live.rs`), once the host has found it;
    /// until then no area can be built.
    live: Option<crate::live::Level>,
    tiles: std::path::PathBuf,
    tile_cache: crate::tiles::Cache,
    /// What the last build found, reused where nothing nearby changed.
    reuse: crate::reuse::AreaCache,
    /// The sidecar reads (recovered surfaces, ground joints) depend on the
    /// centre only: kept for rebuilds in place.
    sidecars_at: Option<[f32; 2]>,
    recovered_area: std::collections::HashMap<u64, svwc::surfaces::Surface>,
    /// Entities that never move (the object dropper's props): GTA-space
    /// triangles with packed surfaces, built in like the map (collision and
    /// rails), and the version of that set.
    fixed: Vec<([[f32; 3]; 3], u32)>,
    fixed_version: u64,
}

impl StaticWorld {
    /// `path` names the cache the sidecars sit beside; the cache itself is
    /// not read.
    pub fn open(path: &Path) -> Self {
        Self {
            recovered: crate::surfaces::Recovered::open(path),
            ground: crate::ground::GroundSource::open(path),
            live: None,
            tile_cache: Default::default(),
            tiles: path.parent().unwrap_or(Path::new(".")).join("tiles"),
            reuse: Default::default(),
            sidecars_at: None,
            recovered_area: Default::default(),
            fixed: Vec::new(),
            fixed_version: 0,
        }
    }

    /// Where GTA's loaded collision is (None: it is not available).
    pub fn set_live(&mut self, level: Option<crate::live::Level>) {
        self.live = level;
    }

    /// The never-moving entities' triangles (GTA space) for the next builds.
    pub fn set_fixed(&mut self, version: u64, triangles: Vec<([[f32; 3]; 3], u32)>) {
        self.fixed = triangles;
        self.fixed_version = version;
    }

    /// The recovered-surface sidecar's load state, for the log.
    pub fn recovered_status(&self) -> &str {
        self.recovered.status()
    }

    /// The ground-joint sidecar's load state, for the log.
    pub fn ground_status(&self) -> &str {
        self.ground.status()
    }

    /// Collision and rails around GTA point `center`, from GTA's loaded
    /// collision (which already holds whatever map states are active).
    pub fn around(&mut self, center: Vec3) -> Result<Area, String> {
        let mut area = self.around_with_tiles(center, crate::BAKED_WORLD.load(std::sync::atomic::Ordering::Relaxed))?;
        let dropped = cap_rail_segments(&mut area.rails, coords::to_skate(center), STATIC_RAIL_SEGMENTS);
        if dropped > 0 {
            area.note.push_str(&format!("; WARNING {dropped} far grind lines left out (grind index limit)"));
        }
        Ok(area)
    }

    pub fn prefetch(&mut self, center: Vec3, lead: Vec3) {
        if crate::BAKED_WORLD.load(std::sync::atomic::Ordering::Relaxed) {
            self.tile_cache.prefetch(&self.tiles, [center.x,center.y], [lead.x,lead.y], RADIUS);
        }
    }

    /// Legend line 2 (design 6.3): which bake the area's tiles are from.
    fn tile_status(&self, selection: &crate::tiles::Selection) -> String {
        let first = selection.baked.values().next();
        let line = |prefix: &str| first.and_then(|t| t.source.lines().find(|l| l.starts_with(prefix)))
            .and_then(|l| l.split_whitespace().nth(1)).map(|h| h.chars().take(8).collect::<String>()).unwrap_or_else(|| "-".into());
        let installed = std::fs::read_to_string(self.tiles.parent().unwrap_or(&self.tiles).join("collision-build.json")).ok()
            .and_then(|j| j.split("\"preparedUtc\"").nth(1).and_then(|r| r.split('"').nth(1)).map(|d| d.replace('T', " ").chars().take(16).collect::<String>()))
            .unwrap_or_else(|| "?".into());
        format!("tiles: rules {} source {} (prepared {installed} UTC), live cells {}, tiles refused {}",
            line("rules"), line("source"), selection.missing.len(), selection.stale)
    }

    fn around_with_tiles(&mut self, center: Vec3, enabled: bool) -> Result<Area, String> {
        let selection = if enabled {
            let selection = self.tile_cache.select(&self.tiles, [center.x, center.y], RADIUS);
            if selection.is_none() && (self.tiles.join("rules.txt").exists() || self.tiles.join("tile-size.txt").exists()) {
                return Err("Baked world data has an invalid or incompatible map-state policy; keep the previous collision area and rebuild/reinstall matching tiles".to_owned());
            }
            selection
        } else { None };
        let Some(selection) = selection else {
            crate::spot::set_status(if enabled { "tiles: NONE installed: GTA's live collision".to_owned() } else { "tiles: off (plain GTA compare): GTA's live collision".to_owned() });
            return self.around_live(center, true);
        };
        let mut area = if selection.missing.is_empty() {
            Area { noses: Vec::new(), center, triangles: Vec::new(), tags: Vec::new(), rails: Vec::new(),
                census: Default::default(), bevels: Default::default(), note: String::new(),
                bounds: Vec::new(), live_required: false, changed: None, fixed: self.fixed_version, options: options_version(), origins: Default::default() }
        } else {
            let live = self.around_live_selected(center, false, Some(&selection))?;
            crate::tiles::keep_live(live, &selection)
        };
        let step_rails = crate::live::STEP_RAILS.load(std::sync::atomic::Ordering::Relaxed);
        // What came from live cells so far; then each tile's faces with
        // their baked origin (design 6.4).
        area.origins = crate::spot::Origins::default();
        for _ in 0..area.triangles.len() { area.origins.push("live GTA collision (state-dependent cell)"); }
        for tile in selection.baked.values() {
            // Both stair variants are in the tile; the settings pick (live::shown).
            for (i, (t, tag)) in tile.triangles.iter().enumerate().filter(|(_, (_, tag))| crate::live::shown(*tag)) {
                area.triangles.push(t.map(|p| coords::to_skate(Vec3::from_array(p)).to_array()));
                area.tags.push(*tag);
                let origin = format!("tile {}_{}: {}", tile.cell.0, tile.cell.1, tile.origin(i));
                area.origins.push(&origin);
            }
            area.rails.extend(tile.rails.iter().map(|r| r.iter().map(|p| coords::to_skate(Vec3::from_array(*p)).to_array()).collect()));
            if step_rails {
                area.rails.extend(tile.noses.iter().map(|r| r.iter().map(|p| coords::to_skate(Vec3::from_array(*p)).to_array()).collect()));
            }
        }
        // Dropped objects remain live and receive the same rules on each build.
        if !self.fixed.is_empty() {
            let first = build_area_rules(center, self.fixed.iter().copied(), &mut Default::default(), false, false);
            let mut shape = self.fixed.clone();
            flush_rail_ends(&mut shape, &first.rails);
            let mut fixed = build_area_rules(center, shape.into_iter(), &mut Default::default(), false, false);
            hide_switched_off(&mut fixed);
            for _ in 0..fixed.triangles.len() { area.origins.push("dropped object (object dropper)"); }
            area.triangles.extend(fixed.triangles);
            area.tags.extend(fixed.tags);
            area.rails.extend(fixed.rails);
        }
        // Baking and live-cell clipping split continuous rails at grid planes.
        // Reconnect the installed union, including fixed props, exactly once.
        normalize_assembled_rails(&mut area.rails);
        // Auxiliary queries must not retain a previous live area's material or
        // seam geometry. The optional raw map carries original GTA material IDs
        // and face keys; packed Skate tile tags cannot recover either exactly.
        let p = [center.x, center.y];
        let mut raw = crate::baked::around(p, RADIUS);
        // The auxiliary map has no map-state provenance either. Clearing a
        // crossing face is safer than reporting a floor/material from an
        // inactive state; collision itself comes from the live patch above.
        raw.retain(|t| !touches_live_only(t, &selection));
        self.recovered_area = self.recovered.around(p, RADIUS)?;
        crate::ground::publish(self.ground.around(p, RADIUS)?);
        self.sidecars_at = Some(p);
        crate::surfaces::publish(crate::surfaces::GridIndex::build(raw.iter().filter_map(|t| {
            self.recovered_area.get(&svwc::surfaces::key(&t.v)).map(|r| (t.v, r.families))
        })));
        crate::ground_at::publish(crate::ground_at::GroundIndex::build(raw.iter().map(|t| (t.v, t.material))));
        area.fixed = self.fixed_version;
        area.note.push_str(&format!("; baked world: {} tiles, {} live tiles", selection.baked.len(), selection.missing.len()));
        crate::spot::set_status(self.tile_status(&selection));
        if selection.stale > 0 {
            area.note.push_str(&format!("; WARNING {} installed tiles are from another bake and were ignored (mixed install: rebake/reinstall the whole set)", selection.stale));
        }
        Ok(area)
    }

    fn around_live(&mut self, center: Vec3, include_fixed: bool) -> Result<Area, String> {
        self.around_live_selected(center, include_fixed, None)
    }

    fn around_live_selected(&mut self, center: Vec3, include_fixed: bool, selection: Option<&crate::tiles::Selection>) -> Result<Area, String> {
        let level = self.live.ok_or("GTA's loaded collision (its physics level) has not been found yet")?;
        let options = options_version();
        let read = std::time::Instant::now();
        let (tris, census, bounds, cleaned) = if let Some(selection) = selection {
            level.walk_selected([center.x, center.y], RADIUS, selection)?
        } else {
            level.walk([center.x, center.y], RADIUS)?
        };
        let walked = read.elapsed();
        let mut note = census.to_string();
        let p = [center.x, center.y];
        if !tris.iter().any(|t| svwc::clean::floor_at(t, p).is_some_and(|z| (z - center.z).abs() < 4.0)) {
            note.push_str(&format!("; NO FLOOR under ({:.1}, {:.1}, {:.1}): {}", p[0], p[1], center.z, level.under(p).unwrap_or_else(|e| e)));
        }
        let in_place = self.sidecars_at == Some(p);
        if !in_place {
            self.recovered_area = self.recovered.around(p, RADIUS)?;
            crate::ground::publish(self.ground.around(p, RADIUS)?);
            self.sidecars_at = Some(p);
        }
        let recovered = &self.recovered_area;
        if self.recovered.is_loaded() {
            let matched = tris.iter().filter(|t| recovered.contains_key(&svwc::surfaces::key(&t.v))).count();
            note.push_str(&format!("; surface records matched {matched} of {} triangles", tris.len()));
        }
        // Recovered joint lines of this area for the audio tick's wheel lookup.
        if self.recovered.is_loaded() {
            crate::surfaces::publish(crate::surfaces::GridIndex::build(tris.iter().filter_map(|t| {
                recovered.get(&svwc::surfaces::key(&t.v)).map(|r| (t.v, r.families))
            })));
        }
        crate::ground_at::publish(crate::ground_at::GroundIndex::build(tris.iter().map(|t| (t.v, t.material))));
        let (read, build) = (read.elapsed(), std::time::Instant::now());
        let mut area = build_area_rules(
            center,
            tris.iter()
                .enumerate()
                .map(|(_, t)| {
                    let tag = crate::surfaces::tag(t.material, recovered.get(&cleaned.surface_key(t)));
                    // Rebuilt stair steps carry NO_GRIND (no grinding on the
                    // noses); gap fills FALLBACK (clean_static).
                    (t.v, tag | cleaned.tag_of(t))
                })
                .chain(self.fixed.iter().copied().filter(|_| include_fixed)),
            &mut self.reuse,
            // Visual and track rails in live cells too: keep_live clips the
            // result to the live cells, so baked cells get no second copy
            // (before, live cells had no visual or track rails at all).
            true,
            true,
        );
        hide_switched_off(&mut area);
        area.live_required = true;
        area.fixed = self.fixed_version;
        area.options = options;
        if !self.fixed.is_empty() {
            note.push_str(&format!("; {} triangles of dropped objects built in", self.fixed.len()));
        }
        if let Some(n) = area.changed {
            note.push_str(&format!("; {n} triangles changed since the last build, the rest reused"));
        }
        note.push_str(&format!(
            "; read {} ms (GTA's bounds {} ms{}), built {} ms",
            read.as_millis(),
            walked.as_millis(),
            if in_place { ", sidecars kept" } else { "" },
            build.elapsed().as_millis()
        ));
        area.note = note;
        area.bounds = bounds;
        Ok(area)
    }
}

fn touches_live_only(t: &svwc::Tri, selection: &crate::tiles::Selection) -> bool {
    let min = [t.v.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min),
        t.v.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min)];
    let max = [t.v.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max),
        t.v.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max)];
    selection.live_only.iter().any(|&(x,y)| min[0] <= (x as f32+1.)*selection.size && max[0] >= x as f32*selection.size
        && min[1] <= (y as f32+1.)*selection.size && max[1] >= y as f32*selection.size)
}

/// Converts GTA-space triangles to a Skate collision area with rails.
/// At a grind line's end, collision corners poking up to 10 cm above the
/// line (within 30 cm past the end, 10 cm to the side) go down to its height,
/// on small parts only (every vertex within 0.4 m of the end). GTA's
/// prop_skate_rail approximates its bent ends with rotated boxes whose corner
/// stands 2.2 cm over the bar's top: the board was kicked off at each end
/// (Manatros 2026-10-24/25, "the ends should round downwards").
fn flush_rail_ends(shape: &mut [([[f32; 3]; 3], u32)], rails: &[Vec<[f32; 3]>]) {
    let gta: Vec<Vec<Vec3>> = rails.iter().map(|r| r.iter().map(|p| coords::from_skate(Vec3::from_array(*p))).collect()).collect();
    flush_rail_ends_gta(shape, &gta);
}

/// `flush_rail_ends` with the lines already in GTA coordinates (prop
/// templates in model space, dynamic.rs).
pub(crate) fn flush_rail_ends_gta(shape: &mut [([[f32; 3]; 3], u32)], rails: &[Vec<Vec3>]) {
    // Each end: its point, outward direction, and the shape's top along the
    // line as a reference (height and slope from 0.5 and 1.0 m inside; the
    // line itself is lifted onto the poking corner and straightened, so it
    // cannot be the reference). A rising rail keeps rising to its end.
    let top_at = |q: Vec3| shape.iter().filter_map(|(t, _)| {
        let tri = svwc::Tri { v: *t, material: 0 };
        svwc::clean::floor_at(&tri, [q.x, q.y]).filter(|z| *z <= q.z + 0.05)
    }).fold(None, |m: Option<f32>, z| Some(m.map_or(z, |m| m.max(z))));
    struct End { at: Vec3, dir: Vec3, top: f32, slope: f32 }
    let end = |line: &[Vec3]| -> Option<End> {
        let e = line[0];
        let mut run = 0.0;
        let mut inside = None;
        for w in line.windows(2) {
            run += w[0].distance(w[1]);
            if run >= 0.5 { inside = Some(w[1]); break; }
        }
        let dir = (e - inside?).truncate().normalize_or_zero().extend(0.0);
        if dir == Vec3::ZERO { return None; }
        let (q5, q10) = (e - dir * 0.5, e - dir * 1.0);
        let z5 = top_at(Vec3::new(q5.x, q5.y, e.z))?;
        let z10 = top_at(Vec3::new(q10.x, q10.y, e.z)).unwrap_or(z5);
        let slope = (z5 - z10) / 0.5;
        Some(End { at: e, dir, top: z5 + slope * 0.5, slope })
    };
    let ends: Vec<End> = rails.iter().filter(|r| r.len() >= 2).flat_map(|r| {
        let back: Vec<Vec3> = r.iter().rev().copied().collect();
        [end(r), end(&back)]
    }).flatten().collect();
    if ends.is_empty() {
        return;
    }
    // By position only: a vertex shared by several triangles moves the same
    // in all of them (no cracks). Only object shapes come here, never the map.
    for (t, _) in shape.iter_mut() {
        let mut v = t.map(Vec3::from_array);
        let mut changed = false;
        for e in &ends {
            for p in &mut v {
                let rel = *p - e.at;
                let along = rel.dot(e.dir);
                let side = (rel - e.dir * along).truncate().length();
                let reference = e.top + e.slope * along;
                if (-0.3..=0.3).contains(&along) && side <= 0.1 && p.z > reference + 1e-4 && p.z <= reference + 0.1 {
                    p.z = reference;
                    changed = true;
                }
            }
        }
        if changed {
            *t = v.map(|p| p.to_array());
        }
    }
}

/// Triangles a world switch hides (rail boxes off by default; live::shown).
/// Baked tiles are filtered the same way when they are loaded.
fn hide_switched_off(area: &mut Area) {
    if area.tags.iter().all(|&t| crate::live::shown(t)) {
        return;
    }
    let (mut triangles, mut tags) = (Vec::with_capacity(area.triangles.len()), Vec::with_capacity(area.tags.len()));
    for (t, tag) in area.triangles.drain(..).zip(area.tags.drain(..)) {
        if crate::live::shown(tag) {
            triangles.push(t);
            tags.push(tag);
        }
    }
    area.triangles = triangles;
    area.tags = tags;
}

pub fn build_area(center: Vec3, tris: impl Iterator<Item = [[f32; 3]; 3]>) -> Area {
    build_area_materials(center, tris.map(|t| (t, 0)))
}

pub fn build_area_materials(center: Vec3, tris: impl Iterator<Item = ([[f32; 3]; 3], u8)>) -> Area {
    build_area_tagged(center, tris.map(|(t, material)| (t, crate::materials::surface(material).packed())))
}

/// As `build_area_materials`, with each triangle's audio tag already chosen.
pub fn build_area_tagged(center: Vec3, tris: impl Iterator<Item = ([[f32; 3]; 3], u32)>) -> Area {
    build_area_reusing(center, tris, &mut crate::reuse::AreaCache::default())
}

/// As `build_area_tagged`, redoing the rails, bevels and ply filters only
/// where the triangles changed since the build `cache` holds (`reuse`); the
/// result is the same as a build from scratch.
pub fn build_area_reusing(center: Vec3, tris: impl Iterator<Item = ([[f32; 3]; 3], u32)>, cache: &mut crate::reuse::AreaCache) -> Area {
    build_area_rules(center, tris, cache, true, true)
}

fn build_area_rules(center: Vec3, tris: impl Iterator<Item = ([[f32; 3]; 3], u32)>, cache: &mut crate::reuse::AreaCache, static_rails: bool, live_tubes: bool) -> Area {
    let (gta, surfaces): (Vec<[Vec3; 3]>, Vec<u32>) = tris
        .map(|(t, tag)| (t.map(Vec3::from_array), tag))
        .filter(|(p, _)| p.iter().all(|v| v.is_finite())
            && (p[1] - p[0]).cross(p[2] - p[0]).length_squared() > 1e-10)
        .unzip();
    let options = options_version();
    cache.for_options(options);
    let (dirty, keys) = cache.diff(&gta);
    let identical = cache.rail_index.as_ref().is_some_and(|(old, _)| *old == gta);
    let (found, census) = if identical && !rails::diagnosing() {
        cache.raw_rails.clone().unwrap_or_else(|| rails::find_metres_reusing(&gta, &dirty, &mut cache.rails))
    } else {
        rails::find_metres_reusing(&gta, &dirty, &mut cache.rails)
    };
    cache.raw_rails = Some((found.clone(), census));
    let steps: Vec<usize> = (0..gta.len()).filter(|&i| surfaces[i] & skate_host::bridge::NO_GRIND_TAG != 0).collect();
    let (found, mut noses) = split_step_edges(found, &gta, &steps);
    // Flights GTA models as real steps: no rail on every nose.
    let (mut found, flight_noses) = rails::off_flight_noses_split(found);
    noses.extend(flight_noses);
    // GTA's tubes (coping, handrails, railing bars) and train tracks: a line
    // on top of each (tubes.rs, tracks.rs), merged with the finder's lines
    // on the same rail (one kept, then lifted onto the top below).
    let mut all = if live_tubes { crate::tubes::rails_in([center.x, center.y], RADIUS + 3.0) } else { Vec::new() };
    // Rails GTA builds without a solid body of their own (tubes, and the
    // visible-model rails below) are the only ones that get a box: a box on
    // the finder's lines (ledges, walls) sat beside real geometry as
    // invisible obstacles (2026-10-20 test: "stuck on invisible geometry").
    let mut boxed = all.clone();
    // One index shared by track snapping, visual support, boxing and lifting.
    // Indices refer to triangle positions, so require identical order too.
    let (indexed_tris, rail_grid) = cache.rail_index.take().filter(|(old, _)| *old == gta)
        .unwrap_or_else(|| (gta.clone(), crate::grid::XyGrid::triangles(1.0, &gta)));
    let raw_tracks = if static_rails { crate::tracks::rails_in([center.x, center.y], RADIUS + 3.0) } else { Vec::new() };
    let tracks = cache.tracks.apply(&raw_tracks, &dirty, 1.0, |r| onto_rail_boxes_indexed(vec![r.to_vec()], &gta, &rail_grid));
    // A track's rail sits in a slot of GTA's collision whose edges the
    // finder takes for lips: 3-5 lines per rail, the board took a slot edge
    // and sank in or slipped under (2026-10-20 metro video). Only the
    // track's own line stays there.
    if !tracks.is_empty() {
        found = off_lines(found, &tracks, TRACK_SIDE, TRACK_UP);
    }
    // Rails from GTA's visible models (visrails.rs): handrails and railings
    // GTA's collision leaves out (the Portola Drive station's stair rails)
    // or gives only a rough block. Their own line is exact; the finder's
    // lines along them (stair edges, walls beside them) go, and each gets a
    // thin collision box for board contact.
    let visual = if static_rails {
        crate::visrails::rails_in_reusing([center.x, center.y], RADIUS + 3.0, &gta, &rail_grid, &dirty, &mut cache.visual)
    } else {
        Vec::new()
    };
    // A visible rail's bends (the curved piece between two straight
    // tubes) are no tube to the finder: its pieces join across them.
    let visual = {
        let mut visual = visual;
        crate::rails::join_ends_within(&mut visual, crate::rails::BEND_GAP);
        visual
    };
    if !visual.is_empty() {
        found = off_lines(found, &visual, VISUAL_SIDE, VISUAL_UP);
        all = off_lines(all, &visual, VISUAL_SIDE, VISUAL_UP);
    }
    if !all.is_empty() {
        all.extend(found);
        crate::rails::trim_overlaps(&mut all);
        crate::rails::join_ends(&mut all);
        crate::rails::dedupe_metres(&mut all);
        found = all;
    }
    // Every rail onto the top of what it runs along (a general rule); the
    // visible rails already are their top.
    let mut found = cache.tops.apply(&found, &dirty, 1.0, |r| onto_tops_indexed(vec![r.to_vec()], &gta, &rail_grid));
    // The boxes follow the same one-line-per-rail result as the grind lines:
    // the raw tube copy above kept a tube line beside each visual line and
    // overlapping capsules, so copings got two or three stacked boxes (546
    // coincident box faces at the Vespucci bowls; lips and bumps, 2026-10-24).
    if !visual.is_empty() {
        boxed = off_lines(boxed, &visual, VISUAL_SIDE, VISUAL_UP);
    }
    crate::rails::trim_overlaps(&mut boxed);
    crate::rails::dedupe_metres(&mut boxed);
    boxed.extend(visual.iter().cloned());
    crate::rails::join_ends(&mut boxed);
    crate::rails::dedupe_metres(&mut boxed);
    crate::rails::straighten(&mut boxed, STRAIGHT);
    found.extend(visual);
    // Tubes and visual rails can also describe slot edges. Top lifting can
    // bring an initially distant finder line into the slot. Apply ownership
    // after every source and keep the steel snap's centre and 1 cm clearance.
    // Track lines are authoritative and join after the merge: a rail head's
    // two edge lines lie closer than DUPLICATE_SPAN and must stay two.
    let mut found = authoritative_tracks(found, Vec::new());
    if !tracks.is_empty() {
        found = cut_track_slots(found, &tracks);
    }
    // Protection uses final installed lines. Retain per-triangle answers only
    // while those lines are identical; rail options/files can change without
    // changing GTA triangles (and therefore without dirtying curbs).
    crate::rails::trim_overlaps(&mut found);
    crate::rails::join_ends(&mut found);
    crate::rails::dedupe_metres(&mut found);
    crate::rails::join_ends(&mut found);
    crate::rails::straighten(&mut found, STRAIGHT);
    let track_boxes = track_gap_boxes(&tracks, &gta, &rail_grid);
    // No grind line without an edge (track lines have their steel head),
    // and only a railing's top bar.
    let found = off_stacked(off_flush(found, &gta, &rail_grid));
    // No grinding under a roof: stretches with a surface right above them
    // (stacked rim lines under a bowl's cap, a coping under a deck) go.
    let mut found = off_buried(found, &gta, &rail_grid);
    join_along_edges(&mut found, &gta, &rail_grid);
    let mut tracks = off_buried(tracks, &gta, &rail_grid);
    // Fewer points on the final lines (the cuts and the two track edges
    // carry many collinear ones): Skate's grind index holds 65,535 segments
    // per area (2026-10-25: 'grind octree exceeds uint16 entry capacity').
    // Track edges keep their curve on the steel head.
    crate::rails::straighten(&mut found, STRAIGHT);
    crate::rails::straighten(&mut tracks, TRACK_STRAIGHT);
    found.extend(tracks);
    // Flights again on the merged set: a step's metal nosing trim in GTA's
    // visible model becomes a visual rail on every nose (Vespucci funbox
    // stairs, 2026-10-24 bake).
    let (found, flight_noses) = crate::rails::off_flight_noses_split(found);
    noses.extend(flight_noses);
    // StepRails: every nose grindable (Manatros 2026-10-24: some prefer it).
    let mut found = found;
    if crate::live::STEP_RAILS.load(std::sync::atomic::Ordering::Relaxed) {
        found.append(&mut noses);
    }
    // Only tube/visual rails receive missing-solid boxes; finder ledges and
    // tracks keep their authored collision. A nearby face alone is not proof
    // that a tube/visual rail already has a solid body.
    let same_rails = cache.clearance_rails == found;
    if !same_rails {
        cache.clearance.clear();
        cache.uncovered = Default::default();
        cache.clearance_rails = found.clone();
    }
    let clearance = RailClearance::new(&found);
    let previous = std::mem::take(&mut cache.clearance);
    let answers = std::cell::RefCell::new(rustc_hash::FxHashMap::default());
    let touches = |t: &[Vec3; 3]| {
        let key = crate::reuse::tri_key(t);
        let mut answers = answers.borrow_mut();
        *answers.entry(key).or_insert_with(|| previous.get(&key).copied().unwrap_or_else(|| clearance.touches(t)))
    };
    // Rebuilt steps keep their risers: no curb bevels on a flight.
    let mut is_step = vec![false; gta.len()];
    for &i in &steps {
        is_step[i] = true;
    }
    let grid = std::sync::OnceLock::new();
    let grid = || grid.get_or_init(|| crate::grid::XyGrid::triangles(curbs::CELL, &gta));
    let same_steps = cache.step_tags == is_step;
    if !same_steps { cache.uncovered = Default::default(); }
    let bevel_result = if identical && same_rails && same_steps {
        cache.bevel_result.take()
    } else { None }.unwrap_or_else(|| curbs::bevels_reusing_filtered(&gta, grid(), &dirty, &mut cache.curbs, |t, i| is_step[i] || touches(t)));
    cache.bevel_result = Some(bevel_result.clone());
    cache.step_tags = is_step.clone();
    let curbs::Bevels { triangles: bevel_tris, sources, buried, census: bevels } = bevel_result;
    // The ply filters' answers, and the keys of what they dropped for the next build.
    let set = |found: &[usize]| found.iter().map(|&i| keys[i]).collect::<rustc_hash::FxHashSet<u64>>();
    let dropped = |set: &rustc_hash::FxHashSet<u64>| keys.iter().enumerate().filter_map(|(i, k)| set.contains(k).then_some(i)).collect::<Vec<_>>();
    let interior = if !dirty.any() { dropped(&cache.interior) } else { crate::plies::interior_reusing(&gta, grid(), |i, pad| dirty.kept(&gta[i], keys[i], pad, &cache.interior)) };
    cache.interior = set(&interior);
    let caps = if !dirty.any() { dropped(&cache.caps) } else { crate::plies::abutting_caps_reusing(&gta, |i, pad| dirty.kept(&gta[i], keys[i], pad, &cache.caps)) };
    cache.caps = set(&caps);
    let solids = if !dirty.any() { dropped(&cache.buried) } else { crate::plies::buried_faces_reusing(&gta, |i, pad| dirty.kept(&gta[i], keys[i], pad, &cache.buried)) };
    cache.buried = set(&solids);
    // Rails were found on the full GTA set; buried risers then leave the
    // collision (their bevels cover them).
    let mut keep = vec![true; gta.len()];
    for &i in buried.iter().filter(|&&i| !is_step[i]).chain(&interior).chain(&caps).chain(&solids) {
        keep[i] = false;
    }
    // Rail boxes are no longer built (design 2026-10-26-one-source-collision
    // 4.5): 9.2 M baked triangles, 13.7 % of all, always hidden since 10-25.
    let _ = &boxed;
    let mut rail_boxes: Vec<[Vec3; 3]> = Vec::new();
    // Reuse coincident GTA faces rather than placing a second contact plane
    // over them. Only final kept GTA faces can replace a box face.
    rail_boxes.retain(|t| !t.iter().chain(std::iter::once(&((t[0]+t[1]+t[2])/3.0))).all(|&p| {
        rail_grid.at(p.x, p.y).iter().any(|&i| {
            if !keep[i as usize] { return false; }
            let g = &gta[i as usize];
            let n = (t[1]-t[0]).cross(t[2]-t[0]).normalize_or_zero();
            let m = (g[1]-g[0]).cross(g[2]-g[0]).normalize_or_zero();
            n.dot(m) > 0.999 && closest_on_triangle(p, g).distance_squared(p) < 1e-8
        })
    }));
    let mut collision: Vec<([Vec3; 3], u32)> = gta
        .iter()
        .zip(&surfaces)
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|((t, s), _)| (*t, *s))
        .chain(bevel_tris.iter().zip(&sources).filter(|&(_, &i)| !is_step[i]).map(|(t, &i)| (*t, surfaces[i] | skate_host::bridge::NO_GRIND_TAG)))
        .chain(rail_boxes.iter().map(|t| (*t, crate::materials::surface(METAL_MATERIAL).packed() | crate::live::RAILBOX_TAG)))
        .chain(track_boxes.iter().map(|t| (*t, crate::materials::surface(METAL_MATERIAL).packed())))
        .collect();
    // NO_GRIND_TAG skips an entire face in Skate's contact probes, rather
    // than deduplicating grind lines. Keep physical stair faces, but restore
    // probe contact beside any installed rail. Boxes are wholly untagged:
    // their top and long faces support the board at the explicit rail line.
    for (t, tag) in &mut collision {
        if *tag & skate_host::bridge::NO_GRIND_TAG != 0 && touches(t) {
            *tag &= !skate_host::bridge::NO_GRIND_TAG;
        }
    }
    cache.clearance = answers.into_inner();
    cache.rail_index = Some((indexed_tris, rail_grid));
    spatial_order(&mut collision);
    Area {
        tags: collision.iter().map(|(_, s)| *s).collect(),
        center,
        triangles: collision.iter().map(|(p, _)| p.map(|v| coords::to_skate(v).to_array())).collect(),
        rails: found
            .into_iter()
            .map(|r| {
                r.into_iter()
                    .map(|p| coords::to_skate(p).to_array())
                    .collect()
            })
            .collect(),
        noses: noses.into_iter().map(|r| r.into_iter().map(|p| coords::to_skate(p).to_array()).collect()).collect(),
        census,
        bevels,
        note: String::new(),
        bounds: Vec::new(),
        live_required: false,
        changed: dirty.compared().then_some(dirty.changed),
        fixed: 0,
        options,
        origins: Default::default(),
    }
}

/// GTAxSkate CurbBevels (World page, default on).
pub static CURB_BEVELS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Whole-face clearance from the final grind lines, in GTA metres. Test
/// interiors too: a long triangle can cross a rail with all corners far away.
/// Capsule distance handles the common case without allocating polygons or
/// building planes. Only the ambiguous band needs the original 32-sided
/// cylinder (at most 1.5 mm extra sideways clearance), exact +/- 0.3 m Z.
struct RailClearance {
    grid: crate::grid::XyGrid,
    planes: Vec<(Vec3, Vec3, std::sync::OnceLock<Vec<(Vec3, f32)>>)>,
}

impl RailClearance {
    fn new(rails: &[Vec<Vec3>]) -> Self {
        let mut grid = crate::grid::XyGrid::new(2.0);
        let mut planes = Vec::new();
        for w in rails.iter().flat_map(|r| r.windows(2)) {
            let (a, b) = (w[0], w[1]);
            grid.insert(planes.len() as u32, a.min(b) - Vec3::splat(0.302), a.max(b) + Vec3::splat(0.302));
            planes.push((a, b, std::sync::OnceLock::new()));
        }
        Self { grid, planes }
    }

    fn clip_planes(a: Vec3, b: Vec3) -> Vec<(Vec3, f32)> {
        let d = b - a;
        // n.p + k*s <= c, 0 <= s <= 1. Eliminate the rail
        // parameter s by pairing its lower and upper bounds. What
        // remains are planes bounding the cylinder swept along a..b.
        let mut upper = vec![(Vec3::ZERO, 1.0, 1.0)];
        let mut lower = vec![(Vec3::ZERO, -1.0, 0.0)];
        let mut flat = Vec::new();
        let normals = (0..32).map(|i| {
            let angle = i as f32 * std::f32::consts::TAU / 32.0;
            Vec3::new(angle.cos(), angle.sin(), 0.0)
        }).chain([Vec3::Z, -Vec3::Z]);
        for n in normals {
            let k = -n.dot(d);
            let c = 0.3;
            if k > 1e-6 { upper.push((n, k, c)); }
            else if k < -1e-6 { lower.push((n, k, c)); }
            else { flat.push((n, c)); }
        }
        for &(nu, ku, cu) in &upper {
            for &(nl, kl, cl) in &lower {
                let n = nu / ku - nl / kl;
                let c = cu / ku - cl / kl;
                let len = n.length();
                if len > 1e-6 { flat.push((n / len, c / len)); }
            }
        }
        flat
    }

    fn clipped(origin: Vec3, planes: &[(Vec3, f32)], t: &[Vec3; 3]) -> bool {
        // Local coordinates avoid cancellation at GTA map offsets.
        let mut polygon = t.map(|p| p - origin).to_vec();
        let mut clipped = Vec::new();
        for &(n, c) in planes {
            clipped.clear();
            for j in 0..polygon.len() {
                let (a, b) = (polygon[j], polygon[(j + 1) % polygon.len()]);
                let (da, db) = (n.dot(a) - c, n.dot(b) - c);
                if da <= 1e-5 { clipped.push(a); }
                if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
                    clipped.push(a.lerp(b, da / (da - db)));
                }
            }
            std::mem::swap(&mut polygon, &mut clipped);
            if polygon.is_empty() { return false; }
        }
        true
    }

    fn touches(&self, t: &[Vec3; 3]) -> bool {
        let lo = t[0].min(t[1]).min(t[2]);
        let hi = t[0].max(t[1]).max(t[2]);
        let mut seen = rustc_hash::FxHashSet::default();
        for x in self.grid.cell_of(lo.x)..=self.grid.cell_of(hi.x) {
            for y in self.grid.cell_of(lo.y)..=self.grid.cell_of(hi.y) {
                for &i in self.grid.cell(x, y) {
                    if !seen.insert(i) { continue; }
                    let (origin, end, planes) = &self.planes[i as usize];
                    // A cylinder contains the 0.3 m capsule and is contained
                    // by the sqrt(2)*0.3 m capsule. Exact distance rejects or
                    // accepts almost all faces without constructing clip planes.
                    let distance = segment_triangle_distance_squared(*origin, *end, t);
                    if distance <= 0.3 * 0.3 { return true; }
                    if distance > 2.0 * 0.302 * 0.302 { continue; }
                    let planes = planes.get_or_init(|| Self::clip_planes(*origin, *end));
                    if Self::clipped(*origin, planes, t) { return true; }
                }
            }
        }
        false
    }
}

/// Exact segment/triangle distance: plane intersection, endpoints and edges.
fn segment_triangle_distance_squared(a: Vec3, b: Vec3, t: &[Vec3; 3]) -> f32 {
    let n = (t[1] - t[0]).cross(t[2] - t[0]);
    let d = b - a;
    let den = n.dot(d);
    if den.abs() > 1e-12 {
        let s = n.dot(t[0] - a) / den;
        if (0.0..=1.0).contains(&s) {
            let p = a + s * d;
            if closest_on_triangle(p, t).distance_squared(p) < 1e-10 { return 0.; }
        }
    }
    let mut best = closest_on_triangle(a, t).distance_squared(a).min(closest_on_triangle(b, t).distance_squared(b));
    for k in 0..3 {
        let (c, e) = (t[k], t[(k + 1) % 3] - t[k]);
        let r = a - c;
        let (dd, ee, de, dr, er) = (d.length_squared(), e.length_squared(), d.dot(e), d.dot(r), e.dot(r));
        if ee < 1e-12 { continue; }
        let denom = dd * ee - de * de;
        let mut s = if denom > 1e-12 { ((de * er - dr * ee) / denom).clamp(0., 1.) } else { 0. };
        let u = ((de * s + er) / ee).clamp(0., 1.);
        if dd > 1e-12 { s = ((de * u - dr) / dd).clamp(0., 1.); }
        best = best.min((a + d * s).distance_squared(c + e * u));
    }
    best
}

/// A track line point looks this far either side for GTA's rail (metres) ...
const RAIL_SEARCH: f32 = 0.08;
/// ... and this high above the track bed.
const RAIL_SEARCH_UP: f32 = 0.35;
/// The line rides this much over the rail collision's top (metres).
const RAIL_CLEAR: f32 = 0.01;

/// Track lines snapped onto GTA's rail collision: each point takes the
/// highest surface within `RAIL_SEARCH` either side (across the track, from
/// the bed up to `RAIL_SEARCH_UP`) and moves over the middle of it. The
/// rails' height differs between lines (0.13-0.17 m); a fixed one put the
/// line 2-4 cm inside the metro's rail boxes, the board clipped in and
/// slipped under (2026-10-20). Points with nothing there keep the default.
pub fn onto_rail_boxes(lines: Vec<Vec<Vec3>>, gta: &[[Vec3; 3]]) -> Vec<Vec<Vec3>> {
    let grid = crate::grid::XyGrid::triangles(1.0, gta);
    onto_rail_boxes_indexed(lines, gta, &grid)
}

fn onto_rail_boxes_indexed(lines: Vec<Vec<Vec3>>, gta: &[[Vec3; 3]], grid: &crate::grid::XyGrid) -> Vec<Vec<Vec3>> {
    if lines.is_empty() {
        return lines;
    }
    let floor = |x: f32, y: f32, lo: f32, hi: f32| -> Option<f32> {
        grid.at(x, y)
            .iter()
            .filter_map(|&i| {
                let t = gta[i as usize];
                let n = (t[1] - t[0]).cross(t[2] - t[0]);
                if n.z.abs() < 1e-6 || n.normalize_or_zero().z.abs() < 0.7 {
                    return None;
                }
                let tri = svwc::Tri { v: t.map(|p| p.to_array()), material: 0 };
                svwc::clean::floor_at(&tri, [x, y]).filter(|z| *z >= lo && *z <= hi)
            })
            .fold(None, |m: Option<f32>, z| Some(m.map_or(z, |m| m.max(z))))
    };
    lines
        .into_iter()
        .flat_map(|line| {
            if line.len() < 2 { return Vec::new(); }
            let widths = std::cell::RefCell::new(Vec::new());
            let snapped: Vec<Option<Vec3>> = (0..line.len())
                .map(|i| {
                    let p = line[i];
                    let (a, b) = (line[i.saturating_sub(1)], line[(i + 1).min(line.len() - 1)]);
                    let along = (b - a).truncate().normalize_or_zero();
                    let side = bevy_math::Vec2::new(-along.y, along.x);
                    let bed = p.z - crate::tracks::RAIL_TOP;
                    // Highest surface across the rail, and where it is.
                    let samples: Vec<(f32, f32)> = (-8..=8)
                        .filter_map(|k| {
                            let off = k as f32 * RAIL_SEARCH / 8.0;
                            let q = p.truncate() + side * off;
                            floor(q.x, q.y, bed + 0.03, bed + RAIL_SEARCH_UP).map(|z| (off, z))
                        })
                        .collect();
                    let top = samples.iter().map(|s| s.1).fold(None, |m: Option<f32>, z| Some(m.map_or(z, |m| m.max(z))))?;
                    let on_top: Vec<f32> = samples.iter().filter(|s| s.1 >= top - 0.01).map(|s| s.0).collect();
                    let middle = on_top.iter().sum::<f32>() / on_top.len() as f32;
                    let (lo, hi) = on_top.iter().fold((f32::MAX, f32::MIN), |(a, b), &o| (a.min(o), b.max(o)));
                    widths.borrow_mut().push(hi - lo);
                    let q = p.truncate() + side * middle;
                    Some(Vec3::new(q.x, q.y, top + RAIL_CLEAR))
                })
                .collect();
            let (Some(first), Some(last)) = (snapped.iter().position(|s| s.is_some()), snapped.iter().rposition(|s| s.is_some())) else { return Vec::new() };
            if first >= last { return Vec::new(); }
            let mut result = Vec::with_capacity(last - first + 1);
            for i in first..=last {
                if let Some(p) = snapped[i] {
                    result.push(p);
                } else {
                    let prev = (first..i).rev().find(|&k| snapped[k].is_some()).unwrap();
                    let next = (i + 1..=last).find(|&k| snapped[k].is_some()).unwrap();
                    let t = (i - prev) as f32 / (next - prev) as f32;
                    result.push(snapped[prev].unwrap().lerp(snapped[next].unwrap(), t));
                }
            }
            if result.len() < 2 { return Vec::new(); }
            // Both edges of the rail head grindable (Manatros 2026-10-24/25:
            // "the track rails should have 2 separate grindable edges"); a
            // head narrower than EDGE_MIN keeps one line on its middle.
            let mut w = widths.into_inner();
            w.sort_by(f32::total_cmp);
            let half = w.get(w.len() / 2).copied().unwrap_or(0.0) * 0.5;
            if half * 2.0 < EDGE_MIN { return vec![result]; }
            let side = |i: usize| {
                let (a, b) = (result[i.saturating_sub(1)], result[(i + 1).min(result.len() - 1)]);
                let along = (b - a).truncate().normalize_or_zero();
                Vec3::new(-along.y, along.x, 0.0)
            };
            let left = (0..result.len()).map(|i| result[i] + side(i) * half).collect();
            let right = (0..result.len()).map(|i| result[i] - side(i) * half).collect();
            vec![left, right]
        })
        .collect()
}

/// Headroom (metres) a grind line needs: a surface closer above buries it.
const HEADROOM: f32 = 1.2;

/// Rails without `HEADROOM` over them, cut out in 0.25 m steps; pieces under
/// 0.5 m go. Measured on the Vespucci bowl rim (2026-10-25): lines at 5.16,
/// 4.6 and 3.7 m under a 5.8 m cap ("edges that interrupt", stacked grinds).
fn off_buried(rails: Vec<Vec<Vec3>>, gta: &[[Vec3; 3]], grid: &crate::grid::XyGrid) -> Vec<Vec<Vec3>> {
    let covered = |p: Vec3, _: Vec3| grid.at(p.x, p.y).iter().any(|&i| {
        let t = gta[i as usize];
        let n = (t[1] - t[0]).cross(t[2] - t[0]);
        if n.length_squared() < 1e-12 { return false; }
        let tri = svwc::Tri { v: t.map(|q| q.to_array()), material: 0 };
        svwc::clean::floor_at(&tri, [p.x, p.y]).is_some_and(|z| z > p.z + 0.08 && z < p.z + HEADROOM)
    });
    cut_where(rails, covered)
}

/// How far to each side a grind line's surface is looked at (metres): both
/// distances must show an edge-less surface for a stretch to go, so a line
/// on a ledge or wall top narrower than twice the outer one keeps it.
const FLUSH_SIDES: [f32; 2] = [0.12, 0.35];
/// A surface whose two sides average less than this below the line's own
/// surface drops away: an edge (metres).
const FLUSH_DROP: f32 = 0.03;

/// A grind line needs an edge (Manatros 2026-10-25, mark 5: a seam of the
/// bean bowl's visible model climbing the wall became a rail; earlier "a
/// grind line in the floor"). Stretches where the collision runs on at the
/// line's height on both sides (flush, or a concave crease) go, cut out as
/// `off_buried` does. Copings, curbs, ledges and stair noses drop away on a
/// side; free-standing rails have no surface at the line and stay.
fn off_flush(rails: Vec<Vec<Vec3>>, gta: &[[Vec3; 3]], grid: &crate::grid::XyGrid) -> Vec<Vec<Vec3>> {
    cut_where(rails, |p, along| flush_at(gta, grid, p, along))
}

/// Whether the collision runs on at `p`'s height on both sides of a line
/// running `along` (no lip there; `off_flush`).
fn flush_at(gta: &[[Vec3; 3]], grid: &crate::grid::XyGrid, p: Vec3, along: Vec3) -> bool {
    // The surface nearest `near` in height (walkable or steeper up to 78 deg).
    let surface = |x: f32, y: f32, near: f32, below: f32, above: f32| {
        grid.at(x, y).iter().filter_map(|&i| {
            let tri = svwc::Tri { v: gta[i as usize].map(|q| q.to_array()), material: 0 };
            svwc::clean::surface_at(&tri, [x, y], 0.2)
        }).filter(|&z| z >= near - below && z <= near + above).min_by(|a, b| (a - near).abs().total_cmp(&(b - near).abs()))
    };
    let flush = |p: Vec3, along: Vec3| {
        let side = Vec3::new(-along.y, along.x, 0.0).normalize_or_zero();
        if side == Vec3::ZERO { return false; }
        let Some(z0) = surface(p.x, p.y, p.z, 0.08, 0.08) else { return false };
        let [near, far] = FLUSH_SIDES;
        [side, -side].iter().map(|&s| {
            let (a, b) = (p + s * near, p + s * far);
            surface(a.x, a.y, z0, 0.6, 0.3).zip(surface(b.x, b.y, z0, 0.6, 0.3))
        }).collect::<Option<Vec<_>>>().is_some_and(|sides| {
            // Each side runs on smoothly (Codex 2026-10-25: a 10 cm drop
            // beside rising ground averaged out to flush) ...
            let smooth = sides.iter().all(|&(a, b)| ((b - z0) - (a - z0) * far / near).abs() <= 0.04);
            // ... and together they do not fall away (a convex edge).
            let (l, r) = (sides[0], sides[1]);
            smooth && (l.0 + r.0) * 0.5 >= z0 - FLUSH_DROP && (l.1 + r.1) * 0.5 >= z0 - FLUSH_DROP
        })
    };
    flush(p, along)
}

/// Where the high-res parks' visible surface folds by at least this much
/// along a grind line found on GTA's own park collision, that line stays
/// (cos 30 deg).
const PARK_FOLD_COS: f32 = 0.866;

/// The visible park model's chamfered and rounded edges hide from the rail
/// finder (each fold under its 45 deg seam limit, or its top face counted
/// as a roof). Stretches of `gta_rails` (Skate space, found on GTA's own
/// park collision) that `area` has no line within 15 cm of are added where,
/// over two thirds of every 10 cm, the area's surface folds by 30 deg or
/// more along them (within 6 cm aside and 8 cm in height, edge running the
/// same way within 25 deg). A smooth visible hump has no such fold, so
/// GTA's facet creases (2026-10-25 M12, 2026-10-26 M2) stay out. Returns
/// how many lines were added.
pub fn park_edge_lines(area: &mut Area, gta_rails: &[Vec<[f32; 3]>]) -> usize {
    let g = |p: [f32; 3]| coords::from_skate(Vec3::from_array(p));
    let tris: Vec<[Vec3; 3]> = area.triangles.iter().map(|t| t.map(g)).collect();
    // Folds: edges shared by two faces (vertex keys, 1 mm) at 30 deg or more.
    let key = |v: Vec3| v.to_array().map(|x| (x * 1000.0).round() as i64);
    let mut by_edge: std::collections::HashMap<([i64; 3], [i64; 3]), Vec<usize>> = Default::default();
    for (i, t) in tris.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (key(t[k]), key(t[(k + 1) % 3]));
            by_edge.entry(if a < b { (a, b) } else { (b, a) }).or_default().push(i);
        }
    }
    let normal = |t: &[Vec3; 3]| (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
    let mut folds: Vec<(Vec3, Vec3)> = Vec::new();
    for faces in by_edge.values().filter(|f| f.len() == 2) {
        let (t, u) = (&tris[faces[0]], &tris[faces[1]]);
        if normal(t).dot(normal(u)) > PARK_FOLD_COS { continue; }
        // The shared edge's two vertices.
        let shared: Vec<Vec3> = t.iter().copied().filter(|p| u.iter().any(|q| key(*q) == key(*p))).collect();
        if shared.len() == 2 { folds.push((shared[0], shared[1])); }
    }
    let mut grid: std::collections::HashMap<(i32, i32), Vec<u32>> = Default::default();
    for (i, (a, b)) in folds.iter().enumerate() {
        let (lo, hi) = (a.min(*b) - Vec3::splat(0.1), a.max(*b) + Vec3::splat(0.1));
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                grid.entry((x, y)).or_default().push(i as u32);
            }
        }
    }
    let on_fold = |p: Vec3, dir: Vec3| grid.get(&(p.x.floor() as i32, p.y.floor() as i32)).is_some_and(|list| list.iter().any(|&i| {
        let (a, b) = folds[i as usize];
        let ab = b - a;
        let l2 = ab.length_squared();
        if l2 < 1e-8 || (ab / l2.sqrt()).dot(dir).abs() < 25f32.to_radians().cos() { return false; }
        let q = a + ab * ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
        (q - p).truncate().length() <= 0.06 && (q.z - p.z).abs() <= 0.08
    }));
    // Lines the area already has, as segments.
    let have: Vec<(Vec3, Vec3)> = area.rails.iter().flat_map(|r| r.windows(2).map(|w| (g(w[0]), g(w[1])))).collect();
    let covered = |p: Vec3| have.iter().any(|&(a, b)| {
        let ab = b - a;
        let q = a + ab * ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
        q.distance(p) <= 0.15
    });
    let mut added = 0;
    // A run of missing 10 cm samples ends where the line is covered or after
    // 30 cm off any fold (peer review 2026-10-26: a 4 m line on a fold for
    // 3 m must not bring its smooth metre back); it keeps up to its last
    // folded sample.
    const OFF_FOLD: usize = 3;
    for r in gta_rails {
        let r: Vec<Vec3> = r.iter().map(|&p| g(p)).collect();
        let mut run: Vec<Vec3> = Vec::new();
        let (mut on, mut total, mut misses) = (0usize, 0usize, 0usize);
        let mut last_on: Option<usize> = None;
        let mut flush = |run: &mut Vec<Vec3>, on: &mut usize, total: &mut usize, last_on: &mut Option<usize>, area: &mut Area| {
            if let Some(k) = *last_on {
                *total -= run.len() - (k + 1);
                run.truncate(k + 1);
                let len: f32 = run.windows(2).map(|w| w[0].distance(w[1])).sum();
                if run.len() >= 2 && len >= 0.5 && *on * 3 >= *total * 2 {
                    area.rails.push(run.iter().map(|&p| coords::to_skate(p).to_array()).collect());
                    added += 1;
                }
            }
            run.clear();
            *on = 0;
            *total = 0;
            *last_on = None;
        };
        for w in r.windows(2) {
            let (a, b) = (w[0], w[1]);
            let dir = (b - a).normalize_or_zero();
            let n = ((a.distance(b) / 0.1).ceil() as usize).max(1);
            for k in 0..=n {
                let p = a.lerp(b, k as f32 / n as f32);
                if covered(p) {
                    flush(&mut run, &mut on, &mut total, &mut last_on, area);
                    misses = 0;
                    continue;
                }
                if run.last().is_some_and(|q: &Vec3| q.distance(p) < 1e-4) { continue; }
                if on_fold(p, dir) {
                    run.push(p);
                    total += 1;
                    on += 1;
                    last_on = Some(run.len() - 1);
                    misses = 0;
                } else if run.is_empty() {
                    continue;
                } else {
                    misses += 1;
                    if misses > OFF_FOLD {
                        flush(&mut run, &mut on, &mut total, &mut last_on, area);
                        misses = 0;
                    } else {
                        run.push(p);
                        total += 1;
                    }
                }
            }
        }
        flush(&mut run, &mut on, &mut total, &mut last_on, area);
    }
    if added > 0 {
        let mut gta: Vec<Vec<Vec3>> = area.rails.iter().map(|r| r.iter().map(|&p| g(p)).collect()).collect();
        crate::rails::join_ends(&mut gta);
        crate::rails::straighten(&mut gta, STRAIGHT);
        area.rails = gta.into_iter().map(|r| r.into_iter().map(|p| coords::to_skate(p).to_array()).collect()).collect();
    }
    added
}

/// Gap a grind line may be bridged across when the collision has an edge
/// all along it (m).
const EDGE_JOIN: f32 = 1.3;

/// Grind lines broken along one clean edge are one line (Manatros
/// 2026-10-26: "more connected and less fractured, especially where the
/// geometry is clean"; the Vespucci pool rim came out in pieces where the
/// rounded coping's facets meet): two ends up to `EDGE_JOIN` apart join when
/// every 5 cm of the gap lies within 2 cm of a triangle edge running the
/// same way (25 deg) and the gap is no flush seam. A real opening (a gap
/// in a railing) has no edge across it and stays open.
fn join_along_edges(rails: &mut Vec<Vec<Vec3>>, gta: &[[Vec3; 3]], grid: &crate::grid::XyGrid) {
    let on_edge = |s: Vec3, dir: Vec3| grid.at(s.x, s.y).iter().any(|&i| {
        let t = &gta[i as usize];
        (0..3).any(|k| {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let ab = b - a;
            let len2 = ab.length_squared();
            if len2 < 1e-6 || (ab / len2.sqrt()).dot(dir).abs() < 25f32.to_radians().cos() { return false; }
            let u = ((s - a).dot(ab) / len2).clamp(0.0, 1.0);
            (a + ab * u).distance(s) <= 0.02
        })
    });
    let bridge = |p: Vec3, q: Vec3| {
        let dir = (q - p).normalize_or_zero();
        let steps = ((p.distance(q) / 0.05).ceil() as usize).max(2);
        (1..steps).all(|k| on_edge(p.lerp(q, k as f32 / steps as f32), dir))
            && [0.25, 0.5, 0.75].iter().all(|&f| !flush_at(gta, grid, p.lerp(q, f), dir))
    };
    crate::rails::join_ends_bridging(rails, EDGE_JOIN, &bridge);
}

/// How far under another rail a lower bar of the same railing lies
/// (metres), and how far to the side at most.
const STACK_BELOW: std::ops::RangeInclusive<f32> = 0.1..=0.8;
const STACK_SIDE: f32 = 0.12;
const STACK_TURN: f32 = 20.0 * std::f32::consts::PI / 180.0;

/// Only a railing's top bar is grindable (Manatros 2026-10-25 mark 9: the
/// metro handrail's lower bar made a second line 40 cm under the top one):
/// stretches with another rail right above them go.
fn off_stacked(rails: Vec<Vec<Vec3>>) -> Vec<Vec<Vec3>> {
    // Every segment by 1 m cell, with its rail.
    let mut cells: rustc_hash::FxHashMap<(i32, i32), Vec<(usize, Vec3, Vec3)>> = Default::default();
    for (k, r) in rails.iter().enumerate() {
        for w in r.windows(2) {
            let (lo, hi) = (w[0].min(w[1]), w[0].max(w[1]));
            for x in (lo.x - STACK_SIDE).floor() as i32..=(hi.x + STACK_SIDE).floor() as i32 {
                for y in (lo.y - STACK_SIDE).floor() as i32..=(hi.y + STACK_SIDE).floor() as i32 {
                    cells.entry((x, y)).or_default().push((k, w[0], w[1]));
                }
            }
        }
    }
    let mut out = Vec::new();
    for (k, rail) in rails.into_iter().enumerate() {
        let under = |p: Vec3, along: Vec3| cells.get(&(p.x.floor() as i32, p.y.floor() as i32)).is_some_and(|list| list.iter().any(|&(j, a, b)| {
            if j == k { return false; }
            let ab = (b - a).truncate();
            // Only a bar running the same way (Codex peer review: a crossing
            // handrail cut the ledge line under it).
            let (d, e) = (ab.normalize_or_zero(), along.truncate().normalize_or_zero());
            if d != bevy_math::Vec2::ZERO && e != bevy_math::Vec2::ZERO && d.dot(e).abs() < STACK_TURN.cos() { return false; }
            let t = if ab.length_squared() < 1e-8 { 0.0 } else { ((p.truncate() - a.truncate()).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) };
            let q = a.lerp(b, t);
            q.truncate().distance(p.truncate()) <= STACK_SIDE && STACK_BELOW.contains(&(q.z - p.z))
        }));
        out.extend(cut_where(vec![rail], under));
    }
    out
}

/// Rails with the stretches where `cut(point, direction)` holds removed, in
/// 0.25 m samples with 1 cm boundaries; pieces under 0.25 m go.
fn cut_where(rails: Vec<Vec<Vec3>>, cut: impl Fn(Vec3, Vec3) -> bool) -> Vec<Vec<Vec3>> {
    // The point between a kept `a` and a cut `b` (or the reverse) where
    // the cut starts, to 1 cm (Codex 2026-10-25: cuts at the last 0.25 m
    // sample lost up to 25 cm each side and every piece under 0.5 m).
    let boundary = |a: Vec3, b: Vec3, a_clear: bool| {
        let dir = (b - a).normalize_or_zero();
        let (mut lo, mut hi) = (a, b);
        for _ in 0..12 {
            let mid = (lo + hi) * 0.5;
            if cut(mid, dir) != a_clear { lo = mid; } else { hi = mid; }
            if lo.distance(hi) < 0.01 { break; }
        }
        if a_clear { lo } else { hi }
    };
    let mut out = Vec::new();
    for rail in rails {
        // Samples every 0.25 m at most; a wholly kept line stays as it is.
        let mut samples: Vec<(Vec3, bool)> = Vec::new();
        for w in rail.windows(2) {
            let dir = (w[1] - w[0]).normalize_or_zero();
            let steps = ((w[1] - w[0]).length() / 0.25).ceil().max(1.0) as usize;
            for k in 0..steps {
                let p = w[0].lerp(w[1], k as f32 / steps as f32);
                samples.push((p, cut(p, dir)));
            }
        }
        if let (Some(&last), Some(w)) = (rail.last(), rail.windows(2).last()) {
            samples.push((last, cut(last, (w[1] - w[0]).normalize_or_zero())));
        }
        if samples.iter().all(|s| !s.1) {
            out.push(rail);
            continue;
        }
        let mut piece: Vec<Vec3> = Vec::new();
        for (k, &(p, is_cut)) in samples.iter().enumerate() {
            let prev = k.checked_sub(1).map(|j| samples[j]);
            match (prev, is_cut) {
                (Some((q, false)), true) => {
                    piece.push(boundary(q, p, true));
                    let length: f32 = piece.windows(2).map(|w| w[0].distance(w[1])).sum();
                    if piece.len() >= 2 && length >= 0.25 { out.push(std::mem::take(&mut piece)); } else { piece.clear(); }
                }
                (Some((q, true)), false) => { piece.push(boundary(q, p, false)); piece.push(p); }
                (_, false) => { if piece.last() != Some(&p) { piece.push(p); } }
                _ => {}
            }
        }
        let length: f32 = piece.windows(2).map(|w| w[0].distance(w[1])).sum();
        if piece.len() >= 2 && length >= 0.25 { out.push(piece); }
    }
    out
}

/// Narrowest rail head (metres) that gets a line on each edge.
const EDGE_MIN: f32 = 0.03;
/// Longest stretch without GTA rail collision boxed between found rail (m).
const TRACK_GAP_MAX: f32 = 30.0;

/// Where a track edge line has no GTA surface just under it, between
/// stretches that have one (a missing piece of a visible rail), a box keeps
/// it solid (Manatros 2026-10-25: "partly solid rails, then no collision").
/// Bare line ends and wholly bare lines get none: the node path may not
/// follow a visible rail there.
fn track_gap_boxes(tracks: &[Vec<Vec3>], gta: &[[Vec3; 3]], grid: &crate::grid::XyGrid) -> Vec<[Vec3; 3]> {
    let supported = |p: Vec3| grid.at(p.x, p.y).iter().any(|&i| {
        let t = gta[i as usize];
        let tri = svwc::Tri { v: t.map(|q| q.to_array()), material: 0 };
        svwc::clean::floor_at(&tri, [p.x, p.y]).is_some_and(|z| z <= p.z + 0.005 && z >= p.z - 0.06)
    });
    let mut gaps = Vec::new();
    for line in tracks {
        let ok: Vec<bool> = line.iter().map(|&p| supported(p)).collect();
        let mut i = 0;
        while i < line.len() {
            if ok[i] { i += 1; continue; }
            let start = i;
            while i < line.len() && !ok[i] { i += 1; }
            if start == 0 || i == line.len() { continue; }
            let run: Vec<Vec3> = line[start - 1..=i].to_vec();
            let length: f32 = run.windows(2).map(|w| w[0].distance(w[1])).sum();
            if length <= TRACK_GAP_MAX { gaps.push(run); }
        }
    }
    crate::visrails::boxes(&gaps)
}

fn authoritative_tracks(rails: Vec<Vec<Vec3>>, tracks: Vec<Vec<Vec3>>) -> Vec<Vec<Vec3>> {
    if tracks.is_empty() { return rails; }
    let mut rails = cut_track_slots(rails, &tracks);
    rails.extend(tracks);
    rails
}

/// Remove exact parameter intervals inside the finite track capsules. Testing
/// just segment endpoints lets long slot edges survive bends and crossings.
fn cut_track_slots(rails: Vec<Vec<Vec3>>, tracks: &[Vec<Vec3>]) -> Vec<Vec<Vec3>> {
    let edges: Vec<_> = tracks.iter().flat_map(|r| r.windows(2).map(|w| (w[0],w[1]))).collect();
    let mut grid = crate::grid::XyGrid::new(2.);
    for (i,&(a,b)) in edges.iter().enumerate() {
        grid.insert(i as u32,a.min(b)-Vec3::splat(TRACK_SIDE),a.max(b)+Vec3::splat(TRACK_SIDE));
    }
    // Intersect an interval with lo <= origin + slope*t <= hi.
    let slab = |range: &mut (f32,f32), origin: f32, slope: f32, lo: f32, hi: f32| {
        if slope.abs()<1e-9 { return origin>=lo && origin<=hi; }
        let (a,b) = ((lo-origin)/slope,(hi-origin)/slope);
        range.0=range.0.max(a.min(b)); range.1=range.1.min(a.max(b));
        range.0<=range.1
    };
    let mut out = Vec::new();
    for rail in rails {
        let mut piece = Vec::new();
        for w in rail.windows(2) {
            let (p,delta)=(w[0],w[1]-w[0]);
            let lo=w[0].min(w[1]); let hi=w[0].max(w[1]);
            let mut candidates=std::collections::BTreeSet::new();
            for x in grid.cell_of(lo.x)..=grid.cell_of(hi.x) {
                for y in grid.cell_of(lo.y)..=grid.cell_of(hi.y) {
                    candidates.extend(grid.cell(x,y).iter().copied());
                }
            }
            let mut cuts=Vec::new();
            for i in candidates {
                let (a,b)=edges[i as usize]; let d=b-a;
                let len=d.truncate().length();
                if len<1e-6 { continue; }
                let unit=d.truncate()/len;
                let origin=(p-a).truncate().dot(unit);
                let slope=delta.truncate().dot(unit);
                // Middle of the capsule: bounded strip, height follows track.
                let mut range=(0.,1.);
                if slab(&mut range,origin,slope,0.,len) &&
                    slab(&mut range,unit.perp_dot((p-a).truncate()),unit.perp_dot(delta.truncate()),-TRACK_SIDE,TRACK_SIDE) &&
                    slab(&mut range,p.z-a.z-origin*d.z/len,delta.z-slope*d.z/len,-TRACK_UP,TRACK_UP) {
                    cuts.push(range);
                }
                // Round caps: projection beyond either endpoint, disc in XY.
                for (end,left) in [(a,true),(b,false)] {
                    let mut range=(0.,1.);
                    if !slab(&mut range,origin,slope,
                        if left { f32::NEG_INFINITY } else { len },
                        if left { 0. } else { f32::INFINITY }) ||
                        !slab(&mut range,p.z-end.z,delta.z,-TRACK_UP,TRACK_UP) { continue; }
                    let q=(p-end).truncate(); let v=delta.truncate();
                    let aa=v.length_squared(); let bb=2.*q.dot(v);
                    let cc=q.length_squared()-TRACK_SIDE*TRACK_SIDE;
                    if aa<1e-12 {
                        if cc<=0. { cuts.push(range); }
                    } else {
                        let disc=bb*bb-4.*aa*cc;
                        if disc<0. { continue; }
                        let root=disc.sqrt();
                        range.0=range.0.max((-bb-root)/(2.*aa));
                        range.1=range.1.min((-bb+root)/(2.*aa));
                        if range.0<=range.1 { cuts.push(range); }
                    }
                }
            }
            cuts.sort_by(|a,b| a.0.total_cmp(&b.0));
            let mut merged: Vec<(f32,f32)>=Vec::new();
            for cut in cuts {
                if let Some(last)=merged.last_mut().filter(|r| cut.0<=r.1+1e-6) {
                    last.1=last.1.max(cut.1);
                } else { merged.push(cut); }
            }
            let mut cursor=0.;
            for (lo,hi) in merged.into_iter().chain(std::iter::once((1.,1.))) {
                if lo>cursor+1e-6 {
                    let a=p+delta*cursor; let b=p+delta*lo;
                    if piece.last().is_none_or(|q: &Vec3| q.distance(a)>1e-4) {
                        if piece.len()>=2 { out.push(std::mem::take(&mut piece)); } else { piece.clear(); }
                        piece.push(a);
                    }
                    piece.push(b);
                }
                if hi>lo && piece.len()>=2 { out.push(std::mem::take(&mut piece)); }
                cursor=cursor.max(hi);
            }
        }
        if piece.len()>=2 { out.push(piece); }
    }
    out
}

/// Finder lines this close to a track's rail line (sideways, up or down,
/// metres) are that rail's slot edges.
/// Top height tolerance when testing an existing solid under a rail (metres).
const COVERED: f32 = 0.015;

/// Stretches without solid GTA collision under the full rail width. Thin
/// faces and nearby walls are not coverage. Existing closed rail boxes stay.
pub fn uncovered(rails: &[Vec<Vec3>], gta: &[[Vec3; 3]]) -> Vec<Vec<Vec3>> {
    let grid = crate::grid::XyGrid::triangles(1.0, gta);
    uncovered_indexed(rails, gta, &grid)
}

fn uncovered_indexed(rails: &[Vec<Vec3>], gta: &[[Vec3; 3]], grid: &crate::grid::XyGrid) -> Vec<Vec<Vec3>> {
    uncovered_kept(rails, gta, grid, &[])
}

fn uncovered_kept(rails: &[Vec<Vec3>], gta: &[[Vec3; 3]], grid: &crate::grid::XyGrid, keep: &[bool]) -> Vec<Vec<Vec3>> {
    if rails.is_empty() {
        return Vec::new();
    }
    let solid = |p: Vec3, side: Vec3| {
        // At centre and both sides require an upward top at the line AND a
        // downward underside at least 6 cm below, plus outward exits in
        // every horizontal direction. Two disconnected sheets are not solid.
        [-0.99, 0.0, 0.99].into_iter().all(|s| {
            let q = p + side * s;
            let inside = q.z - crate::visrails::BOX_DEPTH * 0.5;
            let mut above = (f32::INFINITY, 0.0);
            let mut below = (f32::INFINITY, 0.0);
            for &i in grid.at(q.x, q.y) {
                if keep.get(i as usize) == Some(&false) { continue; }
                let t = gta[i as usize];
                let n = (t[1]-t[0]).cross(t[2]-t[0]);
                if let Some(z) = crate::grid::height(&t, q.x, q.y) {
                    let hit = if z >= inside { &mut above } else { &mut below };
                    let distance = (z-inside).abs();
                    if distance < hit.0 { *hit = (distance, n.z); }
                }
            }
            above.1 > 0.0 && (inside + above.0 - q.z).abs() <= COVERED
                && below.1 < 0.0 && q.z - (inside-below.0) >= crate::visrails::BOX_DEPTH - 0.001
                && [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y].into_iter().all(|direction| {
                    // A conservative finite ray: no exit within 8 m means we
                    // cannot prove a closed GTA solid, so install our box.
                    let origin = Vec3::new(q.x, q.y, inside);
                    let end = origin + direction * 8.0;
                    let (lo, hi) = (origin.min(end), origin.max(end));
                    let mut seen = rustc_hash::FxHashSet::default();
                    let mut nearest = (f32::INFINITY, 0.0);
                    for x in grid.cell_of(lo.x)..=grid.cell_of(hi.x) {
                        for y in grid.cell_of(lo.y)..=grid.cell_of(hi.y) {
                            for &i in grid.cell(x, y) {
                                if keep.get(i as usize) == Some(&false) { continue; }
                                if !seen.insert(i) { continue; }
                                let [a,b,c] = gta[i as usize];
                                let (ab,ac) = (b-a,c-a);
                                let h = direction.cross(ac);
                                let determinant = ab.dot(h);
                                if determinant.abs() < 1e-9 { continue; }
                                let offset = origin-a;
                                let u = offset.dot(h)/determinant;
                                let cross = offset.cross(ab);
                                let v = direction.dot(cross)/determinant;
                                let distance = ac.dot(cross)/determinant;
                                if u >= -1e-5 && v >= -1e-5 && u+v <= 1.0+1e-5
                                    && distance > 1e-6 && distance <= 8.0 && distance < nearest.0 {
                                    nearest = (distance, ab.cross(ac).dot(direction));
                                }
                            }
                        }
                    }
                    nearest.1 > 0.0
                })
        })
    };
    let mut out = Vec::new();
    for r in rails {
        let mut run: Vec<Vec3> = Vec::new();
        for w in r.windows(2) {
            let d = w[1]-w[0];
            let side = Vec3::new(-d.y, d.x, 0.0).normalize_or_zero()
                * crate::tubes::half_width(w[0], w[1]).max(crate::visrails::BOX_HALF);
            // Sample the entire stretch (not only the ends of long rails).
            let steps = (d.length() / 0.25).ceil().max(1.0) as usize;
            let bare = (0..=steps).any(|i| !solid(w[0] + d * (i as f32 / steps as f32), side));
            if bare {
                if run.is_empty() {
                    run.push(w[0]);
                }
                run.push(w[1]);
            } else if run.len() >= 2 {
                out.push(std::mem::take(&mut run));
            } else {
                run.clear();
            }
        }
        if run.len() >= 2 {
            out.push(run);
        }
    }
    out
}

/// The finder's lines within this of a visible rail are its stand-ins
/// (stair edges under a handrail, a wall's top beside it; metres).
const VISUAL_SIDE: f32 = 0.25;
const VISUAL_UP: f32 = 0.35;
/// GTA material id the visible rails' boxes sound as (metal).
const METAL_MATERIAL: u8 = 61;
const TRACK_SIDE: f32 = 0.3;
const TRACK_UP: f32 = 0.4;

/// `rails` without the stretches lying within `side` (horizontally) and
/// `up` (vertically) of any of `lines` (GTA metres).
pub fn off_lines(rails: Vec<Vec<Vec3>>, lines: &[Vec<Vec3>], side: f32, up: f32) -> Vec<Vec<Vec3>> {
    let mut grid: rustc_hash::FxHashMap<(i32, i32), Vec<(Vec3, Vec3)>> = Default::default();
    for l in lines {
        for w in l.windows(2) {
            let (lo, hi) = (w[0].min(w[1]) - Vec3::splat(side), w[0].max(w[1]) + Vec3::splat(side));
            for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
                for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                    grid.entry((x, y)).or_default().push((w[0], w[1]));
                }
            }
        }
    }
    let near = |p: Vec3| {
        grid.get(&(p.x.floor() as i32, p.y.floor() as i32)).is_some_and(|segs| {
            segs.iter().any(|&(a, b)| {
                let d = (b - a).truncate();
                let s = if d.length_squared() > 1e-12 { ((p - a).truncate().dot(d) / d.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                let q = a + (b - a) * s;
                q.truncate().distance(p.truncate()) <= side && (q.z - p.z).abs() <= up
            })
        })
    };
    let mut out = Vec::new();
    for rail in rails {
        let mut piece: Vec<Vec3> = Vec::new();
        for w in rail.windows(2) {
            if near(w[0]) && near(w[1]) && near((w[0] + w[1]) * 0.5) {
                if piece.len() >= 2 {
                    out.push(std::mem::take(&mut piece));
                }
                piece.clear();
            } else {
                if piece.is_empty() {
                    piece.push(w[0]);
                }
                piece.push(w[1]);
            }
        }
        if piece.len() >= 2 {
            out.push(piece);
        }
    }
    out
}

/// GTAxSkate RailTops (World page, default on): `onto_tops`.
pub static RAIL_TOPS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// A rail point moves onto collision above it within this sideways (m) ...
const TOP_REACH: f32 = 0.06;
/// ... and at most this much higher (m).
const TOP_RISE: f32 = 0.25;

/// Rails lifted onto the top of the geometry they run along: a line on a
/// tube's lower facet edge or on a box rail's side left the board under the
/// rail in a boardslide (Manatros 2026-10-20: "the board sometimes slips
/// under the rail"). A point climbs only over faces it touches (its own
/// rail's sides), step by step, never to other geometry merely nearby (a
/// bench beside a curb pulled the curb's line onto itself, 2026-10-20), and
/// at most `TOP_REACH` sideways and `TOP_RISE` up (GTA metres).
pub fn onto_tops(rails: Vec<Vec<Vec3>>, gta: &[[Vec3; 3]]) -> Vec<Vec<Vec3>> {
    let grid = crate::grid::XyGrid::triangles(1.0, gta);
    onto_tops_indexed(rails, gta, &grid)
}

fn onto_tops_indexed(rails: Vec<Vec<Vec3>>, gta: &[[Vec3; 3]], grid: &crate::grid::XyGrid) -> Vec<Vec<Vec3>> {
    if rails.is_empty() || !RAIL_TOPS.load(std::sync::atomic::Ordering::Relaxed) {
        return rails;
    }
    let climb = |p: Vec3| -> f32 {
        let mut q = p;
        for _ in 0..8 {
            let mut best = q;
            for &i in grid.at(q.x, q.y) {
                let t = gta[i as usize];
                if closest_on_triangle(q, &t).distance(q) > TOUCH {
                    continue; // not a face this point lies on
                }
                for k in 0..3 {
                    let (a, b) = (t[k], t[(k + 1) % 3]);
                    let d = (b - a).truncate();
                    let s = if d.length_squared() > 1e-12 { ((p - a).truncate().dot(d) / d.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                    let e = a + (b - a) * s;
                    if e.truncate().distance(p.truncate()) <= TOP_REACH && e.z > best.z + 1e-3 && e.z <= p.z + TOP_RISE {
                        best = e;
                    }
                }
            }
            if best == q {
                break;
            }
            q = best;
        }
        q.z
    };
    rails.into_iter().map(|r| {
        let lift: Vec<f32> = r.iter().map(|&p| climb(p) - p.z).collect();
        let lift = even_lifts(&lift);
        r.iter().zip(lift).map(|(p, l)| Vec3::new(p.x, p.y, p.z + l)).collect()
    }).collect()
}

/// Most a point's lift onto a top may stand out from its neighbours'
/// (metres).
const LIFT_STEP: f32 = 0.05;

/// Per-point lifts with lone peaks levelled: a wall line's end beside a
/// pillar climbed 21 cm onto the pillar while the wall stayed level (Codex
/// 2026-10-25, mark 7). An end takes its neighbour's lift, an inner point
/// the higher of its neighbours'. Lifts that rise together stay.
fn even_lifts(lift: &[f32]) -> Vec<f32> {
    let n = lift.len();
    if n < 2 { return lift.to_vec(); }
    (0..n).map(|i| {
        let near = match i {
            0 => lift[1],
            _ if i == n - 1 => lift[n - 2],
            _ => lift[i - 1].max(lift[i + 1]),
        };
        if lift[i] > near + LIFT_STEP { near } else { lift[i] }
    }).collect()
}

/// A rail point lies on a face when within this of it (metres).
const TOUCH: f32 = 0.02;

/// The point of triangle `t` closest to `p`.
pub(crate) fn closest_on_triangle(p: Vec3, t: &[Vec3; 3]) -> Vec3 {
    let [a, b, c] = *t;
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 { return a; }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 { return b; }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 { return a + ab * (d1 / (d1 - d3)); }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 { return c; }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 { return a + ac * (d2 / (d2 - d6)); }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 { return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6))); }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

/// Within this of a step triangle's edge a rail point lies on the step.
const ON_STEP: f32 = 0.03;
/// Shortest rail piece kept after the steps are cut out (rails.rs MIN_RAIL).
const MIN_PIECE: f32 = 24.0 * 0.0254;

/// Rails without the stretches that run along rebuilt stair steps
/// (`steps`, indices into `gta`): every step nose is a lip to the rail
/// finder, but a flight's steps are not for grinding. Handrails and ledges
/// above or beside the flight stay.
fn off_step_edges(rails: Vec<Vec<Vec3>>, gta: &[[Vec3; 3]], steps: &[usize]) -> Vec<Vec<Vec3>> {
    split_step_edges(rails, gta, steps).0
}

/// As [`off_step_edges`], returning the stretches along the steps too.
fn split_step_edges(rails: Vec<Vec<Vec3>>, gta: &[[Vec3; 3]], steps: &[usize]) -> (Vec<Vec<Vec3>>, Vec<Vec<Vec3>>) {
    if steps.is_empty() {
        return (rails, Vec::new());
    }
    let mut grid: rustc_hash::FxHashMap<(i32, i32), Vec<(Vec3, Vec3)>> = Default::default();
    for &i in steps {
        let t = gta[i];
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let (lo, hi) = (a.min(b) - Vec3::splat(ON_STEP), a.max(b) + Vec3::splat(ON_STEP));
            for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
                for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                    grid.entry((x, y)).or_default().push((a, b));
                }
            }
        }
    }
    let on_step = |p: Vec3| {
        grid.get(&(p.x.floor() as i32, p.y.floor() as i32)).is_some_and(|edges| {
            edges.iter().any(|&(a, b)| {
                let d = b - a;
                let s = if d.length_squared() > 1e-12 { ((p - a).dot(d) / d.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                (a + d * s).distance(p) <= ON_STEP
            })
        })
    };
    let (mut out, mut noses) = (Vec::new(), Vec::new());
    let flush = |piece: &mut Vec<Vec3>, into: &mut Vec<Vec<Vec3>>| {
        if piece.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>() >= MIN_PIECE {
            into.push(std::mem::take(piece));
        }
        piece.clear();
    };
    for rail in rails {
        let (mut piece, mut nose): (Vec<Vec3>, Vec<Vec3>) = (Vec::new(), Vec::new());
        for w in rail.windows(2) {
            let (this, other, into_this) = if on_step(w[0]) && on_step(w[1]) && on_step((w[0] + w[1]) * 0.5) {
                (&mut nose, &mut piece, false)
            } else {
                (&mut piece, &mut nose, true)
            };
            flush(other, if into_this { &mut noses } else { &mut out });
            if this.is_empty() {
                this.push(w[0]);
            }
            this.push(w[1]);
        }
        flush(&mut piece, &mut out);
        flush(&mut nose, &mut noses);
    }
    (out, noses)
}

/// Sorts triangles along a Z-order (Morton) curve of their centroids.
///
/// Skate groups imported triangles into 64-triangle query meshes in the order
/// given and builds its bounds hierarchy over those. GTA's soup arrives in
/// cache-tile order, so a chunk could span a whole tile and every probe
/// visited thousands of triangles (owner: 9..21 ms Skate ticks walking in
/// Rockford). Spatially coherent chunks keep the same triangles and tags and
/// let the existing hierarchy prune. GTA triangle order carries no meaning.
pub fn spatial_order<T>(tris: &mut Vec<([Vec3; 3], T)>) {
    if tris.is_empty() {
        return;
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for (t, _) in tris.iter() {
        let c = (t[0] + t[1] + t[2]) / 3.0;
        lo = lo.min(c);
        hi = hi.max(c);
    }
    let span = (hi - lo).max(Vec3::splat(1e-3));
    // 21 bits per axis (63-bit key).
    let spread = |v: u64| {
        let mut x = v & 0x1f_ffff;
        x = (x | x << 32) & 0x1f_0000_0000_ffff;
        x = (x | x << 16) & 0x1f_0000_ff00_00ff;
        x = (x | x << 8) & 0x100f_00f0_0f00_f00f;
        x = (x | x << 4) & 0x10c3_0c30_c30c_30c3;
        x = (x | x << 2) & 0x1249_2492_4924_9249;
        x
    };
    let key = |t: &[Vec3; 3]| {
        let c = ((t[0] + t[1] + t[2]) / 3.0 - lo) / span * 2_097_151.0;
        spread(c.x as u64) | spread(c.y as u64) << 1 | spread(c.z as u64) << 2
    };
    tris.sort_by_cached_key(|(t, _)| key(t));
}

/// What an area feeds Skate, checked against the invariants Skate relies on.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct AreaAudit {
    pub triangles: usize,
    /// Non-finite vertex (Skate rejects the whole build).
    pub non_finite: usize,
    /// Zero-area or zero-length-edge triangle in f32 Skate space (Skate's
    /// `WorldTriangle::from_vertices` / "Invalid SKATE collision triangle
    /// normal" reject the whole build).
    pub degenerate: usize,
    /// Packed wheel surface `(tag >> 7) & 31` of 16 or more: Skate's surface
    /// vote panics ("native surface histogram index exceeded").
    pub bad_surface: usize,
    /// Bits outside the packed surface and NO_GRIND_TAG.
    pub unknown_tag_bits: usize,
    pub tag_count_mismatch: bool,
    pub non_finite_rails: usize,
}

impl AreaAudit {
    pub fn ok(&self) -> bool {
        self.non_finite == 0
            && self.degenerate == 0
            && self.bad_surface == 0
            && self.unknown_tag_bits == 0
            && !self.tag_count_mismatch
            && self.non_finite_rails == 0
    }
}

impl std::fmt::Display for AreaAudit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "audit {}: {} triangles, {} non-finite, {} degenerate, {} bad surface, {} unknown tag bits, tags {}, {} non-finite rails",
            if self.ok() { "ok" } else { "FAILED" },
            self.triangles,
            self.non_finite,
            self.degenerate,
            self.bad_surface,
            self.unknown_tag_bits,
            if self.tag_count_mismatch { "MISMATCHED" } else { "matched" },
            self.non_finite_rails
        )
    }
}

/// Packed wheel surface Skate reads from a collision tag.
pub fn packed_surface(tag: u32) -> u32 {
    (tag >> 7) & 31
}

/// Checks a built area against what Skate requires of its input.
pub fn audit(area: &Area) -> AreaAudit {
    let mut a = AreaAudit {
        triangles: area.triangles.len(),
        tag_count_mismatch: area.tags.len() != area.triangles.len(),
        ..Default::default()
    };
    for t in &area.triangles {
        let p = t.map(Vec3::from_array);
        if !p.iter().all(|v| v.is_finite()) {
            a.non_finite += 1;
            continue;
        }
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        let edges = [p[1] - p[0], p[2] - p[1], p[0] - p[2]];
        if n.try_normalize().is_none() || edges.iter().any(|e| !(e.length() > 0.0)) {
            a.degenerate += 1;
        }
    }
    for &tag in &area.tags {
        if packed_surface(tag) >= 16 {
            a.bad_surface += 1;
        }
        if tag & !(skate_host::bridge::NO_GRIND_TAG | crate::live::PARKMESH_TAG | crate::live::PARKGTA_TAG | crate::live::PARKCUT_TAG | 0xf7ff) != 0 {
            a.unknown_tag_bits += 1;
        }
    }
    a.non_finite_rails = area
        .rails
        .iter()
        .filter(|r| r.iter().flatten().any(|v| !v.is_finite()))
        .count();
    a
}

pub fn needs_recentre(built: Vec3, skater: Vec3) -> bool {
    (skater - built).truncate().length() > RECENTRE
}

/// Builds are assumed to take this much longer than the recent average
/// before the next one is requested and placed.
const BUILD_LEAD: f32 = 1.3;
/// The next area's centre leads the skater by at most this much, so it
/// still covers the skater with room behind if they stop or turn.
const MAX_LEAD: f32 = RADIUS * 0.6;
/// A moving skater is rebuilt for while still this far inside the area.
const COVER: f32 = RADIUS * 0.85;

/// As `needs_recentre`, for a skater moving at `velocity` (GTA m/s) while
/// builds take `build_secs`: at speed the next area is requested while the
/// current one still covers the distance travelled during the build (a
/// Backwards Man launch at ~36 m/s ran off every area).
pub fn needs_recentre_moving(built: Vec3, skater: Vec3, velocity: Vec3, build_secs: f32) -> bool {
    let d = (skater - built).truncate().length();
    d > RECENTRE || d + velocity.truncate().length() * build_secs * BUILD_LEAD > COVER
}

/// Where to centre the next area: ahead of the skater by the distance they
/// will travel while it builds, at most `MAX_LEAD`.
pub fn lead_centre(skater: Vec3, velocity: Vec3, build_secs: f32) -> Vec3 {
    let ahead = Vec3::new(velocity.x, velocity.y, 0.) * (build_secs * BUILD_LEAD);
    skater + ahead.clamp_length_max(MAX_LEAD)
}

/// Whether a finished build centred at `built` should replace the installed
/// area at `installed` for a skater at `skater`: when it still covers them
/// well, or when it is nearer to them than what they are on now.
pub fn worth_installing(built: Vec3, installed: Vec3, skater: Vec3) -> bool {
    let to_new = (skater - built).truncate().length();
    to_new <= RADIUS * 0.75 || to_new < (skater - installed).truncate().length()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ledge x0..x1 (top at 0.5 m, its front face at y 0) on ground that runs on in front.
    fn ledge(x0: f32, x1: f32) -> Vec<[Vec3; 3]> {
        let v = Vec3::new;
        vec![
            [v(x0, -0.5, 0.5), v(x1, -0.5, 0.5), v(x1, 0., 0.5)], [v(x0, -0.5, 0.5), v(x1, 0., 0.5), v(x0, 0., 0.5)],
            [v(x0, 0., 0.5), v(x1, 0., 0.5), v(x1, 0., 0.)], [v(x0, 0., 0.5), v(x1, 0., 0.), v(x0, 0., 0.)],
            [v(x0, 0., 0.), v(x1, 0., 0.), v(x1, 2., 0.)], [v(x0, 0., 0.), v(x1, 2., 0.), v(x0, 2., 0.)],
        ]
    }

    /// Offline: why the edges near a point of baked tiles are or are not
    /// rails. `SKATEV_RAIL_DIAG="<tiles dir> x y z"`, then
    /// `cargo test -p skatev-runtime rail_diagnosis_at_a_point -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn rail_diagnosis_at_a_point() {
        let Ok(spec) = std::env::var("SKATEV_RAIL_DIAG") else { return };
        let parts: Vec<&str> = spec.split_whitespace().collect();
        let dir = std::path::Path::new(parts[0]);
        let at = Vec3::new(parts[1].parse().unwrap(), parts[2].parse().unwrap(), parts[3].parse().unwrap());
        let mut gta = Vec::new();
        let mut tags = Vec::new();
        // SKATEV_RAIL_DIAG_INPUT: a baker input dump (SKATEV_BAKE_DUMP) instead of the tiles.
        if let Ok(dump) = std::env::var("SKATEV_RAIL_DIAG_INPUT") {
            for line in std::fs::read_to_string(dump).unwrap().lines() {
                let v: Vec<f32> = line.split_whitespace().map(|x| x.parse().unwrap()).collect();
                gta.push([Vec3::new(v[0], v[1], v[2]), Vec3::new(v[3], v[4], v[5]), Vec3::new(v[6], v[7], v[8])]);
                tags.push(v[9] as u32);
            }
        }
        for c in crate::tiles::cells([at.x, at.y], 20.0, 64.0).into_iter().filter(|_| tags.is_empty()) {
            let Ok(f) = std::fs::File::open(crate::tiles::path(dir, c)) else { continue };
            let tile = crate::tiles::Tile::read(std::io::BufReader::new(f)).unwrap();
            gta.extend(tile.triangles.iter().filter(|(t, tag)| crate::live::shown(*tag)
                && t.iter().any(|p| (p[0] - at.x).abs() < 20.0 && (p[1] - at.y).abs() < 20.0)).map(|(t, _)| t.map(Vec3::from_array)));
        }
        crate::rails::set_diagnostics(Some(at));
        let (found, _) = crate::rails::find_metres(&gta);
        for r in crate::rails::take_rejected() {
            println!("rejected {:6.2} m at {:.2} {:.2} {:.2}: {}", r.length, r.mid.x, r.mid.y, r.mid.z, r.reason);
        }
        for r in found.iter().filter(|r| r.iter().any(|p| p.distance(at) < 3.0)) {
            println!("rail {:.2} m from {:?} to {:?}", r.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>(), r[0], r[r.len() - 1]);
        }
        crate::rails::set_diagnostics(None);
        // The whole area pipeline on the same faces; SKATEV_RAIL_DIAG_VISRAILS
        // names a sk8v.visrails to load the visible-model rails too.
        if let Ok(v) = std::env::var("SKATEV_RAIL_DIAG_VISRAILS") {
            println!("visrails: {:?}", crate::visrails::load(std::path::Path::new(&v)));
        }
        if tags.is_empty() { tags = vec![0; gta.len()]; }
        let area = build_area_tagged(at, gta.iter().zip(&tags).map(|(t, &tag)| (t.map(|p| p.to_array()), tag)));
        for r in area.rails.iter().map(|r| r.iter().map(|&p| coords::from_skate(Vec3::from_array(p))).collect::<Vec<_>>())
            .filter(|r| r.iter().any(|p| p.distance(at) < 3.0)) {
            println!("final {:.2} m from {:?} to {:?}", r.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>(), r[0], r[r.len() - 1]);
        }
    }

    #[test]
    fn park_edge_lines_come_back_on_a_chamfer_but_not_on_a_smooth_hump() {
        let v = Vec3::new;
        let s = |t: [Vec3; 3]| t.map(|p| coords::to_skate(p).to_array());
        // A curb 4 m long: top at 0.5, a 45 deg chamfer 4 cm wide, then its side.
        let chamfer = vec![
            s([v(0., -0.5, 0.5), v(4., -0.5, 0.5), v(4., -0.04, 0.5)]), s([v(0., -0.5, 0.5), v(4., -0.04, 0.5), v(0., -0.04, 0.5)]),
            s([v(0., -0.04, 0.5), v(4., -0.04, 0.5), v(4., 0., 0.46)]), s([v(0., -0.04, 0.5), v(4., 0., 0.46), v(0., 0., 0.46)]),
            s([v(0., 0., 0.46), v(4., 0., 0.46), v(4., 0., 0.)]), s([v(0., 0., 0.46), v(4., 0., 0.), v(0., 0., 0.)]),
        ];
        let mut area = Area { noses: Vec::new(), center: Vec3::ZERO, triangles: chamfer.clone(), tags: vec![0; chamfer.len()], rails: Vec::new(),
            census: Default::default(), bevels: Default::default(), note: String::new(), bounds: Vec::new(), live_required: false,
            changed: None, fixed: 0, options: 0, origins: Default::default() };
        // GTA's sharp curb found its top edge at y 0, z 0.5.
        let gta_line = vec![vec![coords::to_skate(v(0.2, 0., 0.5)).to_array(), coords::to_skate(v(3.8, 0., 0.5)).to_array()]];
        assert_eq!(park_edge_lines(&mut area, &gta_line), 1);
        // A GTA line running 2 m past the curb's end (no fold there) comes back only along the curb.
        let mut longer = Area { rails: Vec::new(), ..Area { noses: Vec::new(), center: Vec3::ZERO, triangles: chamfer.clone(), tags: vec![0; chamfer.len()], rails: Vec::new(),
            census: Default::default(), bevels: Default::default(), note: String::new(), bounds: Vec::new(), live_required: false,
            changed: None, fixed: 0, options: 0, origins: Default::default() } };
        let past = vec![vec![coords::to_skate(v(0.2, 0., 0.5)).to_array(), coords::to_skate(v(6.0, 0., 0.5)).to_array()]];
        assert_eq!(park_edge_lines(&mut longer, &past), 1);
        let end = longer.rails[0].iter().map(|&p| coords::from_skate(Vec3::from_array(p)).x).fold(f32::MIN, f32::max);
        assert!(end <= 4.05, "stops at the curb's end: {end}");
        // A smooth hump (folds of 10 deg): GTA's crease line stays out.
        let hump: Vec<_> = (0..6).map(|k| {
            let (y0, y1) = (k as f32 * 0.2 - 0.6, k as f32 * 0.2 - 0.4);
            let z = |y: f32| 0.5 - 0.15 * y * y;
            [s([v(0., y0, z(y0)), v(4., y0, z(y0)), v(4., y1, z(y1))]), s([v(0., y0, z(y0)), v(4., y1, z(y1)), v(0., y1, z(y1))])]
        }).flatten().collect();
        let mut area = Area { triangles: hump.clone(), tags: vec![0; hump.len()], rails: Vec::new(), ..area };
        let crease = vec![vec![coords::to_skate(v(0.2, 0., 0.5)).to_array(), coords::to_skate(v(3.8, 0., 0.5)).to_array()]];
        assert_eq!(park_edge_lines(&mut area, &crease), 0);
    }

    #[test]
    fn a_line_broken_along_one_clean_edge_joins_and_a_real_opening_stays() {
        let v = Vec3::new;
        let gta = ledge(0., 4.);
        let grid = crate::grid::XyGrid::triangles(1.0, &gta);
        let mut rails = vec![vec![v(0., 0., 0.5), v(1.5, 0., 0.5)], vec![v(2.2, 0., 0.5), v(4., 0., 0.5)]];
        join_along_edges(&mut rails, &gta, &grid);
        assert_eq!(rails.len(), 1, "one ledge, one line: {rails:?}");
        // Two ledges with 70 cm of nothing between them stay two lines.
        let mut gta = ledge(0., 1.5);
        gta.extend(ledge(2.2, 4.));
        let grid = crate::grid::XyGrid::triangles(1.0, &gta);
        let mut rails = vec![vec![v(0., 0., 0.5), v(1.5, 0., 0.5)], vec![v(2.2, 0., 0.5), v(4., 0., 0.5)]];
        join_along_edges(&mut rails, &gta, &grid);
        assert_eq!(rails.len(), 2);
    }

    #[test]
    fn long_segments_are_clipped_inside_track_slots_even_with_both_ends_outside() {
        let v=Vec3::new;
        let tracks=vec![vec![v(0.,0.,10.),v(20.,0.,10.)]];
        let crossing=vec![v(0.,-1.,10.),v(20.,1.,10.)];
        let kept=cut_track_slots(vec![crossing],&tracks);
        assert_eq!(kept.len(),2);
        assert!((kept[0].last().unwrap().y+TRACK_SIDE).abs()<1e-4);
        assert!((kept[1][0].y-TRACK_SIDE).abs()<1e-4);
        assert_eq!(kept[0][0],v(0.,-1.,10.));
        assert_eq!(*kept[1].last().unwrap(),v(20.,1.,10.));
        let above=vec![v(0.,0.,11.),v(20.,0.,12.)];
        assert_eq!(cut_track_slots(vec![above.clone()],&tracks),vec![above]);
        // A slot line extending past both finite ends loses its middle too.
        let beyond=cut_track_slots(vec![vec![v(-2.,0.,10.),v(22.,0.,10.)]],&tracks);
        assert_eq!(beyond.len(),2);
        assert!((beyond[0].last().unwrap().x+TRACK_SIDE).abs()<1e-4);
        assert!((beyond[1][0].x-(20.+TRACK_SIDE)).abs()<1e-4);
    }

    #[test]
    fn final_track_ownership_removes_tube_visual_and_lifted_slot_lines() {
        let v = Vec3::new;
        let steel = vec![v(0.,0.,18.211),v(20.,0.,18.211)];
        let rails = vec![
            vec![v(0.,0.16,18.22),v(20.,0.16,18.22)],
            vec![v(0.,-0.1,18.2),v(20.,-0.1,18.2)],
            vec![v(0.,2.,18.95),v(20.,2.,18.95)],
            vec![v(0.,3.,19.5),v(20.,3.,22.)],
        ];
        let result = authoritative_tracks(rails,vec![steel.clone()]);
        assert_eq!(result.len(),3);
        assert_eq!(result.last(),Some(&steel));
        assert!(result[..2].iter().all(|r|r[0].y>=2.));
    }

    #[test]
    fn reused_rail_processing_matches_fresh_after_geometry_changes_and_reordering() {
        let mut scene = flight_and_ledge();
        let mut cache = crate::reuse::AreaCache::default();
        for round in 0..4 {
            if round == 2 { scene.push(([[8., -3., 0.], [9., -3., 0.], [8., -2., 0.]], 0)); }
            if round == 3 { scene.reverse(); }
            let reused = build_area_reusing(Vec3::ZERO, scene.iter().copied(), &mut cache);
            let fresh = build_area_tagged(Vec3::ZERO, scene.iter().copied());
            assert_eq!(reused.triangles, fresh.triangles, "round {round}");
            assert_eq!(reused.tags, fresh.tags, "round {round}");
            assert_eq!(reused.rails, fresh.rails, "round {round}");
        }
    }

    #[test]
    fn capsule_fast_path_matches_original_cylinder_clipping() {
        let mut seed = 53u32;
        let mut random = || { seed = seed.wrapping_mul(1664525).wrapping_add(1013904223); (seed >> 8) as f32 / 16777216. * 6. - 3. };
        for _ in 0..1000 {
            let a = Vec3::new(random(), random(), random());
            let b = a + Vec3::new(random(), random(), random());
            let t = std::array::from_fn(|_| a + Vec3::new(random(), random(), random()));
            let guard = RailClearance::new(&[vec![a, b]]);
            assert_eq!(guard.touches(&t), RailClearance::clipped(a, &RailClearance::clip_planes(a, b), &t), "{a:?} {b:?} {t:?}");
        }
    }

    #[test]
    #[ignore = "manual full area timing"]
    fn area_200k_timing() {
        let mut tris = Vec::new();
        for x in 0..400 { for y in 0..250 {
            let a = Vec3::new(x as f32, y as f32, 0.);
            tris.extend([[a, a + Vec3::X, a + Vec3::ONE.with_z(0.)], [a, a + Vec3::ONE.with_z(0.), a + Vec3::Y]].map(|t| (t.map(|p| p.to_array()), 0)));
        } }
        let mut cache = crate::reuse::AreaCache::default();
        let start = std::time::Instant::now();
        let fresh = build_area_reusing(Vec3::ZERO, tris.iter().copied(), &mut cache);
        let cold = start.elapsed();
        let start = std::time::Instant::now();
        let warm = build_area_reusing(Vec3::ZERO, tris.iter().copied(), &mut cache);
        eprintln!("full area 200k: fresh {:?}, unchanged {:?}", cold, start.elapsed());
        assert_eq!(fresh.triangles, warm.triangles);
        assert_eq!(fresh.rails, warm.rails);
    }

    #[test]
    #[ignore = "manual 200k-triangle clearance timing"]
    fn clearance_200k_timing() {
        let rails: Vec<_> = (0..400).map(|i| vec![Vec3::new(i as f32, 0., 0.2), Vec3::new(i as f32 + 0.3, 500., 0.2)]).collect();
        let tris: Vec<_> = (0..200_000).map(|i| {
            let p = Vec3::new((i % 400) as f32 + 0.15, (i / 400) as f32, 0.);
            [p, p + Vec3::X * 0.4, p + Vec3::Y * 0.4]
        }).collect();
        let start = std::time::Instant::now();
        let clearance = RailClearance::new(&rails);
        let built = start.elapsed();
        let hits = tris.iter().filter(|t| clearance.touches(t)).count();
        eprintln!("clearance 200k: index {:?}, total {:?}, hits {}", built, start.elapsed(), hits);
        assert!(hits > 100_000);
    }

    #[test]
    fn rail_clearance_checks_interiors_and_both_distances() {
        let guard = RailClearance::new(&[vec![Vec3::new(-2.0, 0.0, 1.0), Vec3::new(2.0, 0.0, 1.0)]]);
        assert!(guard.touches(&[Vec3::new(-10.0, -10.0, 1.0), Vec3::new(10.0, -10.0, 1.0), Vec3::new(0.0, 10.0, 1.0)]));
        let face = |y: f32, z: f32| [Vec3::new(-1.0, y, z), Vec3::new(1.0, y, z), Vec3::new(0.0, y + 0.001, z)];
        assert!(guard.touches(&face(0.299, 1.299)));
        assert!(!guard.touches(&face(0.31, 1.0)));
        assert!(!guard.touches(&face(0.0, 1.31)));
        let offset = Vec3::new(-1374.0, -1396.0, 500.0);
        let translated = RailClearance::new(&[vec![Vec3::new(-2.0, 0.0, 1.0) + offset, Vec3::new(2.0, 0.0, 1.0) + offset]]);
        assert!(translated.touches(&face(0.299, 1.299).map(|p| p + offset)));
        assert!(!translated.touches(&face(0.31, 1.0).map(|p| p + offset)));
        let sloped = RailClearance::new(&[vec![Vec3::ZERO, Vec3::new(5.0, 0.0, 5.0)]]);
        assert!(sloped.touches(&face(0.29, 1.29)));
        assert!(!sloped.touches(&face(0.0, 4.0)));
    }

    #[test]
    fn rail_line_suppresses_bevels_and_preserves_their_original_collision() {
        let mut tris = Vec::new();
        for (x0, z0, x1, z1) in [(-3.0, 0.0, 0.0, 0.0), (0.0, 0.1, 1.0, 0.1 + 15f32.to_radians().tan())] {
            let (a, b, c, d) = (Vec3::new(x0, -2.0, z0), Vec3::new(x1, -2.0, z1),
                Vec3::new(x1, 2.0, z1), Vec3::new(x0, 2.0, z0));
            tris.extend([[a, b, c], [a, c, d]]);
        }
        let grid = crate::grid::XyGrid::triangles(curbs::CELL, &tris);
        assert!(!curbs::bevels_with_sources(&tris, &grid).triangles.is_empty());
        let guard = RailClearance::new(&[vec![Vec3::new(0.0, -2.0, 0.1), Vec3::new(0.0, 2.0, 0.1)]]);
        let result = curbs::bevels_reusing_filtered(&tris, &grid, &crate::reuse::Dirty::everything(),
            &mut curbs::CurbCache::default(), |t, _| guard.touches(t));
        assert!(result.triangles.is_empty() && result.buried.is_empty());
    }

    #[test]
    fn installed_rails_restore_no_grind_faces_without_losing_surface_bits() {
        let mut scene = flight_and_ledge();
        let tagged = [[6.5, -2.95, 0.39], [8.5, -2.95, 0.39], [7.5, -2.9, 0.39]];
        scene.push((tagged, skate_host::bridge::NO_GRIND_TAG | 3));
        let area = build_area_tagged(Vec3::ZERO, scene.into_iter());
        let i = area.triangles.iter().position(|t| t.map(|p| coords::from_skate(Vec3::from_array(p)).to_array()) == tagged).unwrap();
        assert_eq!(area.tags[i], 3);
        let installed = RailClearance::new(&area.rails.iter().map(|r| r.iter().map(|&p| coords::from_skate(Vec3::from_array(p))).collect()).collect::<Vec<_>>());
        for (t, tag) in area.triangles.iter().zip(&area.tags) {
            if tag & skate_host::bridge::NO_GRIND_TAG != 0 {
                assert!(!installed.touches(&t.map(|p| coords::from_skate(Vec3::from_array(p)))));
            }
        }
    }

    #[test]
    fn visual_boxes_stay_within_their_rail_clearance() {
        let rails = vec![vec![Vec3::new(-2.0, 0.0, 1.0), Vec3::new(2.0, 0.0, 1.0)]];
        let guard = RailClearance::new(&rails);
        let boxes = crate::visrails::boxes(&rails);
        assert_eq!(boxes.len(), 12);
        assert!(boxes.iter().all(|t| guard.touches(t)));
    }

    /// A 4-step flight (riser 0.18, tread 0.3, 2 m wide) rising along +y from
    /// flat ground, its faces tagged as rebuilt steps, and a 0.4 m ledge beside it.
    fn flight_and_ledge() -> Vec<([[f32; 3]; 3], u32)> {
        let step = skate_host::bridge::NO_GRIND_TAG;
        let mut out = Vec::new();
        let mut quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], tag: u32| {
            out.push(([a, b, c], tag));
            out.push(([a, c, d], tag));
        };
        for s in 0..4 {
            let (y0, y1, z0, z1) = (s as f32 * 0.3, (s + 1) as f32 * 0.3, s as f32 * 0.18, (s + 1) as f32 * 0.18);
            quad([2., y0, z0], [0., y0, z0], [0., y0, z1], [2., y0, z1], step); // riser, facing -y
            quad([0., y0, z1], [2., y0, z1], [2., y1, z1], [0., y1, z1], step); // tread
        }
        quad([-20., -20., 0.], [20., -20., 0.], [20., 0., 0.], [-20., 0., 0.], 0); // ground before the flight
        quad([0., 1.2, 0.72], [2., 1.2, 0.72], [2., 6., 0.72], [0., 6., 0.72], 0); // landing
        // Ledge 5 m away: top 0.4 m, front face toward -y.
        quad([6., -3., 0.4], [9., -3., 0.4], [9., -2., 0.4], [6., -2., 0.4], 0);
        quad([9., -3., 0.], [6., -3., 0.], [6., -3., 0.4], [9., -3., 0.4], 0);
        out
    }

    #[test]
    fn rebuilt_steps_keep_their_shape_and_give_no_rails_but_a_ledge_does() {
        let area = build_area_tagged(Vec3::ZERO, flight_and_ledge().into_iter());
        let rails: Vec<Vec<Vec3>> = area.rails.iter().map(|r| r.iter().map(|&p| coords::from_skate(Vec3::from_array(p))).collect()).collect();
        let on_flight = rails.iter().filter(|r| r.iter().all(|p| p.x > -0.05 && p.x < 2.05 && p.y > -0.05 && p.y < 1.25)).count();
        let on_ledge = rails.iter().filter(|r| r.iter().all(|p| p.x > 5.9 && (p.z - 0.4).abs() < 0.05)).count();
        assert_eq!(on_flight, 0, "no rails on the step noses: {rails:?}");
        assert!(on_ledge > 0, "the ledge keeps its rail: {rails:?}");
        // Every riser is still there (no bevel buried it).
        let risers = area.triangles.iter().filter(|t| {
            let p = t.map(|v| coords::from_skate(Vec3::from_array(v)));
            let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
            n.z.abs() < 0.1 && p.iter().all(|v| v.x >= 0. && v.x <= 2. && v.y <= 1.2 && v.y >= 0.)
        }).count();
        assert_eq!(risers, 8, "4 risers, 2 triangles each");
        // The same flight untagged (GTA's own step-shaped collision, as the
        // Vespucci bowl stairs): the noses form a flight and give no rails
        // either (rails::off_flight_noses).
        let plain = build_area_tagged(Vec3::ZERO, flight_and_ledge().into_iter().map(|(t, _)| (t, 0)));
        let noses = plain.rails.iter().filter(|r| r.iter().all(|&p| {
            let p = coords::from_skate(Vec3::from_array(p));
            p.x > -0.05 && p.x < 2.05 && p.y > -0.05 && p.y < 1.25
        })).count();
        assert_eq!(noses, 0, "untagged step noses are no rails either: {:?}", plain.rails);
    }
    #[test]
    fn an_alternative_step_flight_alone_gives_its_noses() {
        // The bake runs the other stair variant (steps rebuilt from an
        // unflagged flight) through the rules alone to get StepRails noses.
        let tag = skate_host::bridge::NO_GRIND_TAG | crate::live::STEP_TAG | crate::live::USTEP_TAG;
        let flight: Vec<_> = flight_and_ledge().into_iter().filter(|(_, t)| *t != 0).map(|(v, _)| (v, tag)).collect();
        let area = build_area_tagged(Vec3::ZERO, flight.into_iter());
        let lines: Vec<Vec<Vec3>> = area.rails.iter().chain(&area.noses)
            .map(|r| r.iter().map(|&p| coords::from_skate(Vec3::from_array(p))).collect()).collect();
        let noses = lines.iter().filter(|r| {
            let (a, b) = (r[0], r[r.len() - 1]);
            (a.y - b.y).abs() < 0.05 && (a.z - b.z).abs() < 0.05 && (a.x - b.x).abs() > 1.5
                && (0..4).any(|s| (a.y - s as f32 * 0.3).abs() < 0.05 && (a.z - (s + 1) as f32 * 0.18).abs() < 0.05)
        }).count();
        assert!(noses >= 3, "step noses from the flight alone: {lines:?}");
    }

    #[test]
    fn fast_skaters_get_their_next_area_ahead_and_in_time() {
        let v = Vec3::new(36.0, 0.0, 0.0);
        // Standing still: the fixed drift rule.
        assert!(!needs_recentre_moving(Vec3::ZERO, Vec3::new(30.0, 0.0, 0.0), Vec3::ZERO, 3.0));
        assert!(needs_recentre_moving(Vec3::ZERO, Vec3::new(65.0, 0.0, 0.0), Vec3::ZERO, 3.0));
        // 36 m/s with 2.5 s builds: requested long before the 64 m drift.
        assert!(needs_recentre_moving(Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0), v, 2.5));
        // The centre leads, capped so the skater stays well inside.
        let c = lead_centre(Vec3::ZERO, v, 2.5);
        assert!((c.x - MAX_LEAD).abs() < 1e-3 && c.y == 0.0);
        assert_eq!(lead_centre(Vec3::new(1.0, 2.0, 3.0), Vec3::ZERO, 2.5), Vec3::new(1.0, 2.0, 3.0));
        // Steady state at 36 m/s: each build lands before the skater leaves
        // the installed area.
        let (mut installed, mut at, build) = (Vec3::ZERO, Vec3::ZERO, 2.5);
        for _ in 0..20 {
            while !needs_recentre_moving(installed, at, v, build) {
                at += v / 60.0;
            }
            let centre = lead_centre(at, v, build);
            at += v * build;
            assert!((at - installed).truncate().length() < RADIUS, "ran off the area at {at}");
            assert!(worth_installing(centre, installed, at));
            installed = centre;
        }
    }

    #[test]
    fn a_late_build_nearer_than_the_installed_area_is_still_installed() {
        let installed = Vec3::ZERO;
        let skater = Vec3::new(250.0, 0.0, 0.0);
        assert!(worth_installing(Vec3::new(120.0, 0.0, 0.0), installed, skater));
        assert!(!worth_installing(Vec3::new(-20.0, 0.0, 0.0), installed, skater));
        assert!(worth_installing(Vec3::new(10.0, 0.0, 0.0), installed, Vec3::new(20.0, 0.0, 0.0)));
    }

    #[test]
    fn spatial_order_keeps_every_triangle_with_its_tag_and_groups_neighbours() {
        let tri = |x: f32, y: f32| [Vec3::new(x, y, 0.0), Vec3::new(x + 0.5, y, 0.0), Vec3::new(x, y + 0.5, 0.0)];
        // Alternate far-apart triangles: source order interleaves two clusters.
        let mut t: Vec<([Vec3; 3], u32)> = (0..128)
            .map(|i| if i % 2 == 0 { (tri(i as f32 * 0.01, 0.0), i) } else { (tri(500.0 + i as f32 * 0.01, 500.0), i) })
            .collect();
        let before: std::collections::BTreeSet<u32> = t.iter().map(|(_, s)| *s).collect();
        super::spatial_order(&mut t);
        let after: std::collections::BTreeSet<u32> = t.iter().map(|(_, s)| *s).collect();
        assert_eq!(before, after);
        for (p, s) in &t {
            let near = if s % 2 == 0 { p[0].x < 10.0 } else { p[0].x > 400.0 };
            assert!(near, "tag {s} still belongs to its triangle");
        }
        // Each 64-triangle chunk now holds one cluster.
        for chunk in t.chunks(64) {
            let first = chunk[0].0[0].x > 400.0;
            assert!(chunk.iter().all(|(p, _)| (p[0].x > 400.0) == first), "chunk mixes clusters");
        }
    }

    #[test]
    fn filtering_preserves_contact_material_alignment() {
        let floor = [[0.,0.,0.],[1.,0.,0.],[0.,1.,0.]];
        let area = build_area_materials(Vec3::ZERO, [(floor,55), ([[0.;3];3],1), (floor,174)].into_iter());
        let converted = floor.map(|p| coords::to_skate(Vec3::from_array(p)).to_array());
        let tags: Vec<_> = area.triangles.iter().zip(&area.tags).filter(|(t,_)| **t == converted).map(|(_,s)| *s).collect();
        assert_eq!(tags, vec![crate::materials::surface(55).packed(), crate::materials::surface(174).packed()]);
        assert!(audit(&area).ok());
    }

    #[test]
    fn no_grind_tag_keeps_surface_zero_and_audits_cleanly() {
        let mut area = build_area(Vec3::ZERO,
            [[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]].into_iter());
        area.tags[0] = skate_host::bridge::NO_GRIND_TAG;
        assert_eq!(packed_surface(area.tags[0]), 0);
        assert!(audit(&area).ok());
        area.tags[0] |= 16 << 7;
        let result = audit(&area);
        assert_eq!(result.bad_surface, 1);
        assert_eq!(result.unknown_tag_bits, 1);
        assert!(!result.ok());
    }

    #[test]
    fn area_is_converted_to_skate_space_and_drops_degenerates() {
        let floor = [[0.0, 0.0, 5.0], [1.0, 0.0, 5.0], [0.0, 1.0, 5.0]];
        let degenerate = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        let area = build_area(Vec3::ZERO, [floor, degenerate].into_iter());
        let expected = floor.map(|p| coords::to_skate(Vec3::from_array(p)).to_array());
        let t = area.triangles.iter().find(|t| **t == expected).expect("original floor retained").map(Vec3::from_array);
        assert!(area.triangles.iter().all(|t| {
            let [a,b,c] = t.map(Vec3::from_array);
            (b-a).cross(c-a).length_squared() > 1e-10
        }), "original and rail collision contain no degenerates");
        assert!(
            t.iter().all(|v| (v.y - 5.0).abs() < 1e-6),
            "GTA z becomes skate y"
        );
        assert!(
            (t[1] - t[0]).cross(t[2] - t[0]).y > 0.0,
            "upward floor stays upward"
        );
    }

    #[test]
    fn recentre_ignores_vertical_motion() {
        assert!(!needs_recentre(Vec3::ZERO, Vec3::new(0.0, 0.0, 500.0)));
        assert!(needs_recentre(
            Vec3::ZERO,
            Vec3::new(RECENTRE + 1.0, 0.0, 0.0)
        ));
    }
}

#[cfg(test)]
mod coverage_tests {
    use super::*;

    fn floor(x: f32) -> Area {
        build_area(Vec3::ZERO, std::iter::once([
            [x - 2., -2., 0.], [x + 2., -2., 0.], [x, 2., 0.],
        ]))
    }

    #[test]
    fn swap_checks_current_and_predicted_support_not_build_centre() {
        let old = floor(20.);
        let new = floor(0.);
        let supported = Vec3::new(20., 0., 1.);
        let unsupported = Vec3::new(10., 0., 1.);
        assert!(new.loses_floor(&old, supported, unsupported));
        assert!(new.loses_floor(&old, unsupported, supported));
        assert!(!old.loses_floor(&old, supported, unsupported));
        assert!(!new.loses_floor(&old, unsupported, unsupported));
        let airborne = Vec3::new(20., 0., 10.);
        assert!(!new.loses_floor(&old, airborne, airborne));
        assert!(!old.has_floor(Vec3::new(20., 0., 4.)), "same strict four-metre window as NO FLOOR");
    }
}

#[cfg(test)]
mod top_tests {
    use super::*;

    #[test]
    fn only_rails_without_gta_collision_get_a_box() {
        let v = Vec3::new;
        // GTA collision along the first 2 m of the rail (a thin top face), none after.
        let gta = vec![[v(0., -0.05, 1.0), v(2., -0.05, 1.0), v(2., 0.05, 1.0)], [v(0., -0.05, 1.0), v(2., 0.05, 1.0), v(0., 0.05, 1.0)]];
        let rail = vec![v(0., 0., 1.0), v(1., 0., 1.0), v(2., 0., 1.0), v(3., 0., 1.0), v(4., 0., 1.0)];
        let bare = uncovered(&[rail], &gta);
        assert_eq!(bare.len(), 1);
        assert_eq!(bare[0].len(), 5, "a thin top is not a solid: {bare:?}");
    }

    #[test]
    fn coverage_requires_thickness_width_and_support_along_the_whole_line() {
        let line = vec![Vec3::new(0.0, 0.0, 1.0), Vec3::new(2.0, 0.0, 1.0)];
        let gta = crate::visrails::boxes(&[vec![Vec3::new(-0.1, 0.0, 1.0), Vec3::new(2.1, 0.0, 1.0)]]);
        assert!(uncovered(&[line.clone()], &gta).is_empty(), "a real closed box is reused");
        let thin = gta.iter().copied().filter(|t| t.iter().all(|p| p.z > 0.99)).collect::<Vec<_>>();
        assert_eq!(uncovered(&[line.clone()], &thin).len(), 1, "a top alone is not enough");
        let sheets = gta.iter().copied().filter(|t| {
            let n = (t[1]-t[0]).cross(t[2]-t[0]);
            n.z.abs() > 1e-6
        }).collect::<Vec<_>>();
        assert_eq!(uncovered(&[line.clone()], &sheets).len(), 1, "top and bottom sheets without sides are not an enclosure");
        let no_end = gta.iter().copied().filter(|t| {
            let n = (t[1]-t[0]).cross(t[2]-t[0]);
            n.x < 1e-6
        }).collect::<Vec<_>>();
        assert_eq!(uncovered(&[line.clone()], &no_end).len(), 1, "a missing end cap is not a solid");
        let disconnected = sheets.iter().map(|t| t.map(|p| {
            if p.z < 0.99 { p - Vec3::Z*3.0 } else { p }
        })).collect::<Vec<_>>();
        assert_eq!(uncovered(&[line.clone()], &disconnected).len(), 1, "a distant underside is not coverage");
        let shifted = gta.iter().map(|t| t.map(|p| p + Vec3::Y*0.06)).collect::<Vec<_>>();
        assert_eq!(uncovered(&[line.clone()], &shifted).len(), 1, "an adjacent surface within 8 cm is not coverage");
        let ends = [(-0.1,0.1), (0.9,1.1), (1.9,2.1)].into_iter().flat_map(|(a,b)|
            crate::visrails::boxes(&[vec![Vec3::new(a,0.0,1.0), Vec3::new(b,0.0,1.0)]])).collect::<Vec<_>>();
        assert_eq!(uncovered(&[line], &ends).len(), 1, "coverage at ends/midpoint leaves holes");
    }

    #[test]
    fn sparse_track_nodes_project_onto_the_crown_between_nodes() {
        let y = crate::tracks::HALF_GAUGE;
        let mut gta = Vec::new();
        for (a, b) in [(0.0_f32,5.0_f32),(5.0,10.0)] {
            let point = |x: f32, side: f32| {
                let bend = (1.0-(x-5.0).abs()/5.0)*0.04;
                Vec3::new(x, y+bend+side, 0.17+bend)
            };
            let (p,q,r,t) = (point(a,-0.025), point(b,-0.025),point(b,0.025),point(a,0.025));
            gta.extend([[p,q,r],[p,r,t]]);
        }
        let raw = crate::tracks::rails_of(&[vec![Vec3::ZERO,Vec3::X*10.]], [5.,0.], 20.);
        let snapped = onto_rail_boxes(raw, &gta);
        // The rail at +y gives two edge lines; their middle follows the crown.
        let at: Vec<Vec3> = snapped.iter().flatten().filter(|p| (p.x-5.0).abs()<1e-5 && p.y>0.0).copied().collect();
        assert_eq!(at.len(), 2, "both edges of the rail head: {at:?}");
        let mid = (at[0] + at[1]) * 0.5;
        assert!((mid.y-(y+0.04)).abs()<0.011, "follows intermediate crown centre: {mid}");
        assert!(at.iter().all(|p| (p.z-(0.21+RAIL_CLEAR)).abs()<0.001), "sits on intermediate crown: {at:?}");
    }

    /// A box between two corners (x, y, z), as triangles.
    fn cuboid(lo: Vec3, hi: Vec3) -> Vec<[[f32; 3]; 3]> {
        let c = |i: u32| Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z }).to_array();
        [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]].iter()
            .flat_map(|q| [[c(q[0]), c(q[1]), c(q[2])], [c(q[0]), c(q[2]), c(q[3])]]).collect()
    }

    #[test]
    fn a_placed_rail_ends_flush_where_its_bend_box_pokes_up() {
        // GTA's prop_skate_rail collision: bar top 0.384 m to y 2.465; the
        // bend's rotated box has a corner at y 2.478, z 0.406 (2.2 cm over).
        let mut tris = cuboid(Vec3::new(-0.036, -2.465, 0.301), Vec3::new(0.036, 2.465, 0.384));
        let bend = [(-0.0346, 2.4285, 0.3168), (-0.0346, 2.4785, 0.4061), (-0.0346, 2.6694, 0.2991), (-0.0346, 2.6194, 0.2099)];
        // The bend as a whole rotated box: two side quads and four edge quads.
        let ring = |x: f32| bend.iter().map(|&(_, y, z)| Vec3::new(x, y, z)).collect::<Vec<_>>();
        let (l, r) = (ring(-0.0346), ring(0.0346));
        let centre = (l.iter().chain(&r).copied().sum::<Vec3>()) / 8.0;
        let mut quad = |q: [Vec3; 4]| {
            let n = (q[1] - q[0]).cross(q[2] - q[0]);
            let q = if n.dot((q[0] + q[2]) * 0.5 - centre) < 0.0 { [q[0], q[3], q[2], q[1]] } else { q };
            tris.push([q[0].to_array(), q[1].to_array(), q[2].to_array()]);
            tris.push([q[0].to_array(), q[2].to_array(), q[3].to_array()]);
        };
        quad([l[0], l[1], l[2], l[3]]);
        quad([r[0], r[1], r[2], r[3]]);
        for k in 0..4 {
            let j = (k + 1) % 4;
            quad([l[k], l[j], r[j], r[k]]);
        }
        let floor = cuboid(Vec3::new(-3., -4., -0.1), Vec3::new(3., 4., 0.0));
        let shape: Vec<_> = tris.into_iter().chain(floor).map(|t| (t, 0u32)).collect();
        let area = build_area_tagged(Vec3::ZERO, shape.iter().copied());
        let end = Vec3::new(0.0, 2.465, 0.384);
        let mut shape = shape;
        flush_rail_ends(&mut shape, &area.rails);
        let area = build_area_tagged(Vec3::ZERO, shape.into_iter());
        let near = |p: &Vec3| p.x.abs() < 0.1 && p.y > end.y - 0.3 && p.y < end.y + 0.3;
        let poke = area.triangles.iter().flatten().map(|p| coords::from_skate(Vec3::from_array(*p)))
            .filter(near).map(|p| p.z).fold(f32::MIN, f32::max);
        assert!(poke <= end.z + 1e-3, "no collision over the bar's top at its end: {poke}");
        let line = area.rails.iter().flatten().map(|p| coords::from_skate(Vec3::from_array(*p)))
            .filter(near).map(|p| p.z).fold(f32::MIN, f32::max);
        assert!(line > 0.3 && line <= end.z + 1e-3, "the grind line stays level to the end: {line}");
    }

    #[test]
    fn a_rising_rail_keeps_its_slope_to_the_end_and_stays_closed() {
        // A bar rising 10 cm per metre, in 0.25 m pieces with a long underside
        // sharing the end vertices (Codex review 2026-10-25).
        let mut tris = Vec::new();
        let z = |y: f32| 0.4 + 0.1 * y;
        for k in 0..20 {
            let (y0, y1) = (k as f32 * 0.25, (k + 1) as f32 * 0.25);
            let (a, b, c, d) = (Vec3::new(-0.03, y0, z(y0)), Vec3::new(0.03, y0, z(y0)), Vec3::new(0.03, y1, z(y1)), Vec3::new(-0.03, y1, z(y1)));
            tris.push([a.to_array(), b.to_array(), c.to_array()]);
            tris.push([a.to_array(), c.to_array(), d.to_array()]);
        }
        let mut shape: Vec<([[f32; 3]; 3], u32)> = tris.into_iter().map(|t| (t, 0)).collect();
        let before = shape.clone();
        let line = vec![(0..=20).map(|k| Vec3::new(0.0, k as f32 * 0.25, z(k as f32 * 0.25) + 0.01)).collect::<Vec<_>>()];
        flush_rail_ends_gta(&mut shape, &line);
        assert_eq!(shape, before, "a sloped rail with nothing poking is left as it is");
    }

    #[test]
    fn a_lone_lift_onto_a_pillar_is_levelled() {
        assert_eq!(even_lifts(&[0.0, 0.0, 0.21]), vec![0.0, 0.0, 0.0]);
        assert_eq!(even_lifts(&[0.21, 0.0]), vec![0.0, 0.0]);
        assert_eq!(even_lifts(&[0.0, 0.3, 0.0]), vec![0.0, 0.0, 0.0]);
        // A rail lifted 3 cm along its whole length keeps it.
        assert_eq!(even_lifts(&[0.03, 0.03, 0.04]), vec![0.03, 0.03, 0.04]);
    }

    #[test]
    fn a_lower_bar_under_a_top_bar_goes() {
        let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let top = vec![v(0., 0., 1.0), v(4., 0., 1.0)];
        let low = vec![v(0., 0.03, 0.6), v(4., 0.03, 0.6)];
        let beside = vec![v(0., 0.5, 0.6), v(4., 0.5, 0.6)];
        let kept = off_stacked(vec![top.clone(), low, beside.clone()]);
        assert_eq!(kept, vec![top.clone(), beside]);
        // A handrail crossing over a ledge line leaves the ledge whole.
        let ledge = vec![v(0., 0., 0.5), v(4., 0., 0.5)];
        let crossing = vec![v(2., -2., 1.0), v(2., 0.05, 1.0)];
        assert_eq!(off_stacked(vec![ledge.clone(), crossing.clone()]), vec![ledge, crossing]);
    }

    #[test]
    fn a_line_without_an_edge_goes_and_edges_stay() {
        let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let quad = |out: &mut Vec<[Vec3; 3]>, a: Vec3, b: Vec3, c: Vec3, d: Vec3| { out.push([a, b, c]); out.push([a, c, d]); };
        // Floor at z 0 for x -3..0; a 0.3 m drop (a ledge) at x 0, ground on
        // to x 3 at z -0.3; and a slope (a bowl wall) for y 4..8 rising 0.5
        // per metre in x.
        let mut gta = Vec::new();
        quad(&mut gta, v(-3., -2., 0.), v(0., -2., 0.), v(0., 2., 0.), v(-3., 2., 0.));
        quad(&mut gta, v(0., -2., -0.3), v(3., -2., -0.3), v(3., 2., -0.3), v(0., 2., -0.3));
        quad(&mut gta, v(-3., 4., -1.5), v(3., 4., 1.5), v(3., 8., 1.5), v(-3., 8., -1.5));
        let grid = crate::grid::XyGrid::triangles(1.0, &gta);
        // A line painted into the floor goes.
        assert!(off_flush(vec![vec![v(-1.5, -1.5, 0.), v(-1.5, 1.5, 0.)]], &gta, &grid).is_empty());
        // The ledge's edge stays.
        let edge = vec![vec![v(-0.01, -1.5, 0.), v(-0.01, 1.5, 0.)]];
        assert_eq!(off_flush(edge.clone(), &gta, &grid), edge);
        // A seam climbing the slope goes (mark 5, the bean bowl).
        assert!(off_flush(vec![vec![v(-1., 6., -0.5), v(1., 6., 0.5)]], &gta, &grid).is_empty());
        // A 10 cm drop beside a bank rising 0.5 per metre (Codex) stays.
        let mut bank = Vec::new();
        quad(&mut bank, v(-3., 10., -0.1), v(0., 10., -0.1), v(0., 14., -0.1), v(-3., 14., -0.1));
        quad(&mut bank, v(0., 10., 0.), v(3., 10., 1.5), v(3., 14., 1.5), v(0., 14., 0.));
        let bank_grid = crate::grid::XyGrid::triangles(1.0, &bank);
        let lip = vec![vec![v(0.01, 10.5, 0.), v(0.01, 13.5, 0.)]];
        assert_eq!(off_flush(lip.clone(), &bank, &bank_grid), lip);
        // A free-standing rail 1 m over the floor stays.
        let rail = vec![vec![v(-1.5, -1.5, 1.), v(-1.5, 1.5, 1.)]];
        assert_eq!(off_flush(rail.clone(), &gta, &grid), rail);
    }

    #[test]
    fn a_narrow_cap_cuts_the_rail_at_its_edges_only() {
        let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        // A cap from x 2.03 to 2.4, 60 cm over the rail (Codex test idea).
        let gta = vec![[v(2.03, -1., 1.6), v(2.4, -1., 1.6), v(2.4, 1., 1.6)], [v(2.03, -1., 1.6), v(2.4, 1., 1.6), v(2.03, 1., 1.6)]];
        let grid = crate::grid::XyGrid::triangles(1.0, &gta);
        let kept = off_buried(vec![vec![v(0., 0., 1.0), v(4., 0., 1.0)]], &gta, &grid);
        assert_eq!(kept.len(), 2, "both free parts stay: {kept:?}");
        let ends: Vec<f32> = kept.iter().flat_map(|r| [r[0].x, r[r.len() - 1].x]).collect();
        assert!(ends.iter().any(|x| (x - 2.03).abs() < 0.015) && ends.iter().any(|x| (x - 2.4).abs() < 0.015), "cut at the cap's edges: {ends:?}");
        // A clear line comes back as it is.
        let clear = vec![vec![v(5., 0., 1.0), v(5.3, 0., 1.0)]];
        assert_eq!(off_buried(clear.clone(), &gta, &grid), clear);
    }

    #[test]
    fn too_many_grind_segments_drop_the_farthest_lines() {
        let line = |x: f32, n: usize| (0..=n).map(|k| [x, 0.0, k as f32 * 0.1]).collect::<Vec<_>>();
        let mut rails = vec![line(50.0, 10), line(1.0, 10), line(20.0, 10)];
        let dropped = cap_rail_segments(&mut rails, Vec3::ZERO, 20);
        assert_eq!(dropped, 1);
        assert!(rails.iter().all(|r| r[0][0] < 30.0), "the far line went: {rails:?}");
        assert_eq!(cap_rail_segments(&mut rails, Vec3::ZERO, 20), 0, "under the limit nothing goes");
    }

    #[test]
    fn reassembly_keeps_close_track_edges_and_drops_copies() {
        let v = |x: f32, y: f32| Vec3::new(x, y, 1.0);
        let mut rails = vec![vec![v(0., 0.), v(10., 0.)], vec![v(0., 0.04), v(10., 0.04)], vec![v(2., 0.005), v(6., 0.005)]];
        drop_exact_copies(&mut rails, 0.02);
        assert_eq!(rails.len(), 2, "the two edge lines stay, the copy goes: {rails:?}");
        assert!(rails.iter().all(|r| r[0].x == 0.0), "{rails:?}");
    }

    #[test]
    fn a_rail_under_a_cap_is_dropped_where_covered() {
        let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        // A cap at z 1.6 over x 0..4 (60 cm above the rail at z 1.0).
        let gta = vec![[v(0., -1., 1.6), v(4., -1., 1.6), v(4., 1., 1.6)], [v(0., -1., 1.6), v(4., 1., 1.6), v(0., 1., 1.6)]];
        let grid = crate::grid::XyGrid::triangles(1.0, &gta);
        let kept = off_buried(vec![vec![v(-3., 0., 1.0), v(8., 0., 1.0)]], &gta, &grid);
        assert!(kept.iter().flatten().all(|p| p.x <= 0.01 || p.x >= 3.99), "nothing under the cap: {kept:?}");
        assert_eq!(kept.len(), 2, "the free parts either side stay: {kept:?}");
        // A ceiling 3 m up leaves the rail alone.
        let high = vec![[v(-5., -1., 4.0), v(9., -1., 4.0), v(9., 1., 4.0)], [v(-5., -1., 4.0), v(9., 1., 4.0), v(-5., 1., 4.0)]];
        let grid = crate::grid::XyGrid::triangles(1.0, &high);
        assert_eq!(off_buried(vec![vec![v(-3., 0., 1.0), v(8., 0., 1.0)]], &high, &grid).len(), 1);
    }

    #[test]
    fn only_interior_gaps_in_a_track_rail_get_a_box() {
        // Rail top at z 0.17 along x 5..10 and 14..20; bare 0..5 (end) and
        // 10..14 (a missing piece between found rail).
        let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let mut gta = Vec::new();
        for (a, b) in [(5.0, 10.0), (14.0, 20.0)] {
            gta.push([v(a, -0.05, 0.17), v(b, -0.05, 0.17), v(b, 0.05, 0.17)]);
            gta.push([v(a, -0.05, 0.17), v(b, 0.05, 0.17), v(a, 0.05, 0.17)]);
        }
        let grid = crate::grid::XyGrid::triangles(1.0, &gta);
        let line = vec![(0..=20).map(|x| v(x as f32, 0.0, 0.18)).collect::<Vec<_>>()];
        let boxes = track_gap_boxes(&line, &gta, &grid);
        assert!(!boxes.is_empty(), "the interior gap is boxed");
        assert!(boxes.iter().flatten().all(|p| p.x >= 9.99 && p.x <= 14.01), "only between 10 and 14: {:?}",
            boxes.iter().flatten().map(|p| p.x).fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(x), b.max(x))));
    }

    #[test]
    fn a_track_line_snaps_onto_the_rail_box_under_it() {
        // The metro's rail: a box 7 cm wide, top 0.169 m over the bed (z 0),
        // its centre 4 cm off where the default line runs.
        let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let top = 0.169;
        let gta = vec![[v(0., 0.005, top), v(20., 0.005, top), v(20., 0.075, top)], [v(0., 0.005, top), v(20., 0.075, top), v(0., 0.075, top)],
                       [v(-5., -2., 0.), v(25., -2., 0.), v(25., 2., 0.)], [v(-5., -2., 0.), v(25., 2., 0.), v(-5., 2., 0.)]];
        let line = vec![vec![v(2., 0., crate::tracks::RAIL_TOP), v(10., 0., crate::tracks::RAIL_TOP), v(18., 0., crate::tracks::RAIL_TOP)]];
        let snapped = onto_rail_boxes(line, &gta);
        // Two lines, one on each edge of the 7 cm head (Manatros 2026-10-25).
        assert_eq!(snapped.len(), 2);
        for (a, b) in snapped[0].iter().zip(&snapped[1]) {
            assert!((a.z - (top + RAIL_CLEAR)).abs() < 1e-4 && (b.z - (top + RAIL_CLEAR)).abs() < 1e-4, "on top: {a} {b}");
            assert!(((a.y + b.y) * 0.5 - 0.04).abs() < 0.011, "centred on the head: {a} {b}");
            assert!((a.y - b.y).abs() >= EDGE_MIN && (a.y - b.y).abs() <= 0.08, "on its edges: {a} {b}");
        }
    }

    #[test]
    fn slot_edges_along_a_track_rail_go_and_a_ledge_beside_it_stays() {
        let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let track = vec![vec![v(0., 0., 18.19), v(50., 0., 18.19)]];
        let rails = vec![
            vec![v(0., 0.05, 18.04), v(50., 0.05, 18.04)],  // slot edge
            vec![v(0., -0.06, 18.19), v(50., -0.06, 18.19)], // rail box edge
            vec![v(0., 2.0, 18.6), v(50., 2.0, 18.6)],      // platform edge 2 m away
        ];
        let kept = off_lines(rails, &track, TRACK_SIDE, TRACK_UP);
        assert_eq!(kept.len(), 1);
        assert!((kept[0][0].y - 2.0).abs() < 1e-5);
    }

    #[test]
    fn a_rail_line_on_a_box_rails_lower_edge_moves_onto_its_top() {
        // A box rail 5 cm wide, top 0.40 m, its line found on the side at 0.30 m.
        let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        // Its top, and the side face the line runs along (y = 0.025).
        let gta = vec![[v(0., -0.025, 0.4), v(3., -0.025, 0.4), v(3., 0.025, 0.4)], [v(0., -0.025, 0.4), v(3., 0.025, 0.4), v(0., 0.025, 0.4)],
                       [v(0., 0.025, 0.0), v(3., 0.025, 0.0), v(3., 0.025, 0.4)], [v(0., 0.025, 0.0), v(3., 0.025, 0.4), v(0., 0.025, 0.4)]];
        let rails = onto_tops(vec![vec![v(0.2, 0.03, 0.30), v(2.8, 0.03, 0.30)]], &gta);
        assert!(rails[0].iter().all(|p| (p.z - 0.4).abs() < 1e-5), "{rails:?}");
        // A bench seat 5 cm beside and 20 cm above a curb's line does not
        // pull it: the line does not touch the bench.
        let bench = vec![[v(0., 0.08, 0.5), v(3., 0.08, 0.5), v(3., 0.5, 0.5)]];
        let rails = onto_tops(vec![vec![v(0.2, 0.03, 0.30), v(2.8, 0.03, 0.30)]], &bench);
        assert!(rails[0].iter().all(|p| (p.z - 0.3).abs() < 1e-5), "{rails:?}");
        // A roof 1 m above a ledge's line stays out of reach.
        let roof = vec![[v(0., -1., 1.3), v(3., -1., 1.3), v(3., 1., 1.3)]];
        let rails = onto_tops(vec![vec![v(0.2, 0., 0.30), v(2.8, 0., 0.30)]], &roof);
        assert!(rails[0].iter().all(|p| (p.z - 0.3).abs() < 1e-5));
    }
}

#[cfg(test)]
mod baked_tile_tests {
    use super::*;
    use crate::tiles::{self, Tile};

    #[test]
    fn tile_assembly_restores_short_border_tails_and_preserves_parallel_edges() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../out/tests/assembled-rails-{}", std::process::id()));
        let dir = root.join("tiles");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tile-size.txt"), "64").unwrap();
        std::fs::write(dir.join("rules.txt"), tiles::rules_hash()).unwrap();
        tiles::write_live_cells(&dir, 64., &tiles::rules_hash(), &Default::default()).unwrap();
        let mut pieces = std::collections::BTreeMap::<_, Vec<_>>::new();
        for y in [10., 10.1] {
            let rail = [[62., y, 2.], [64.04, y, 2.]];
            for (owner, part) in tiles::split_rail(&rail, 64.) {
                pieces.entry(owner).or_default().push(part.clone());
                // A repeated source describes the same physical edge.
                pieces.entry(owner).or_default().push(part);
            }
        }
        for c in tiles::cells([32., 32.], RADIUS, 64.) {
            let tile = Tile { origins: Vec::new(), origin_names: Vec::new(), noses: Vec::new(), cell: c, size: 64., source: format!("{}\n{}",
                tiles::rules_hash(), tiles::live_policy(&Default::default())),
                triangles: vec![], rails: pieces.remove(&c).unwrap_or_default() };
            tile.write(std::fs::File::create(tiles::path(&dir, c)).unwrap()).unwrap();
        }
        let mut world = StaticWorld::open(&root.join("test.svwc"));
        let area = world.around_with_tiles(Vec3::new(32., 32., 1.), true).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(area.rails.len(), 2, "one continuous rail per physical edge: {:?}", area.rails);
        for rail in &area.rails {
            let gta: Vec<_> = rail.iter().map(|&p| coords::from_skate(Vec3::from_array(p))).collect();
            let (a,b) = (gta[0], *gta.last().unwrap());
            assert!((a.x.min(b.x)-62.).abs() < 1e-4 && (a.x.max(b.x)-64.04).abs() < 1e-4,
                "short tail at the border must survive: {gta:?}");
            assert!(gta.iter().all(|p| (p.y-a.y).abs()<1e-4 && (p.z-2.).abs()<1e-4),
                "normalization must preserve rail position: {gta:?}");
        }
    }

    #[test]
    #[ignore = "manual rail assembly throughput measurement"]
    fn assembled_rail_throughput() {
        for count in [500usize, 2000, 5000] {
            let mut pieces: Vec<_> = (0..count).map(|i| {
                let x = i as f32 * 0.5;
                vec![[x, 2., 0.], [x+0.5, 2., 0.]]
            }).collect();
            let start = std::time::Instant::now();
            normalize_assembled_rails(&mut pieces);
            eprintln!("assembled rail normalization: {count} fragments -> {} rails in {:?}",
                pieces.len(), start.elapsed());
            assert_eq!(pieces.len(), 1);
            assert_eq!(pieces[0].len(), 2);
        }
    }

    #[test]
    fn assembled_rails_retain_unique_tails_of_mostly_overlapping_sources() {
        let mut rails = vec![vec![[0.,2.,0.],[10.,2.,0.]],vec![[1.,2.,0.],[11.,2.,0.]]];
        normalize_assembled_rails(&mut rails);
        assert_eq!(rails.len(),1);
        let mut short = vec![vec![[0.,2.,0.],[2.,2.,0.]],vec![[1.,2.,0.],[2.04,2.,0.]]];
        normalize_assembled_rails(&mut short);
        assert_eq!(short.len(),1);
        assert!(short[0].iter().any(|p|(p[0]-2.04).abs()<1e-5),
            "even a 4 cm uncovered tail must survive: {short:?}");
        let r=&rails[0];
        assert!((r[0][0].min(r.last().unwrap()[0])-0.).abs()<1e-5);
        assert!((r[0][0].max(r.last().unwrap()[0])-11.).abs()<1e-5,
            "the 90-percent covered source still contributes a unique metre: {rails:?}");
    }

    #[test]
    fn assembled_rails_keep_real_bends_and_open_gaps() {
        let v = Vec3::new;
        let original = vec![
            vec![v(0.,0.,2.),v(2.,0.,2.)],
            vec![v(2.,0.,2.),v(4.,1.,2.)],
            vec![v(4.3,1.,2.),v(6.,1.,2.)],
        ];
        let mut rails: Vec<_> = original.iter().map(|r| r.iter()
            .map(|&p| coords::to_skate(p).to_array()).collect()).collect();
        normalize_assembled_rails(&mut rails);
        assert_eq!(rails.len(),2,"a real gap must not be bridged");
        assert!(rails.iter().any(|r| r.len()==3 && r[1]==coords::to_skate(v(2.,0.,2.)).to_array()),
            "a real bend must remain in the continuous rail: {rails:?}");
    }

    #[test]
    fn inactive_state_auxiliary_floors_are_cleared_including_crossing_faces() {
        let selection = tiles::Selection { size:64.,baked:Default::default(),missing:vec![(0,0)],live_only:vec![(0,0)],stale:0 };
        let crossing = svwc::Tri { v:[[-100.,1.,2.],[100.,1.,2.],[100.,2.,2.]],material:1 };
        assert!(touches_live_only(&crossing,&selection));
        let outside = svwc::Tri { v:[[70.,1.,2.],[71.,1.,2.],[70.,2.,2.]],material:1 };
        assert!(!touches_live_only(&outside,&selection));
    }

    #[test]
    fn baked_static_needs_no_live_level_and_off_or_missing_uses_live() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../out/tests/runtime-{}", std::process::id()));
        let dir = root.join("tiles");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tile-size.txt"),"64").unwrap();
        std::fs::write(dir.join("rules.txt"),tiles::rules_hash()).unwrap();
        tiles::write_live_cells(&dir,64.,&tiles::rules_hash(),&Default::default()).unwrap();
        let mut world = StaticWorld::open(&root.join("test.svwc"));
        let center = Vec3::new(32.,32.,1.);
        for c in tiles::cells([32.,32.],RADIUS,64.) {
            let x = c.0 as f32*64.; let y = c.1 as f32*64.;
            let tile = Tile { origins: Vec::new(), origin_names: Vec::new(), noses: Vec::new(), cell:c,size:64.,source:format!("{}\n{}",tiles::rules_hash(),tiles::live_policy(&Default::default())),triangles:vec![([[x,y,1.],[x+1.,y,1.],[x,y+1.,1.]],NO_GRIND_TILE_TAG)],rails:vec![vec![[x,y,2.],[x+1.,y,2.]]] };
            tile.write(std::fs::File::create(tiles::path(&dir,c)).unwrap()).unwrap();
        }
        assert!(world.around_with_tiles(center,false).is_err(),"off uses the live level");
        crate::ground_at::publish(crate::ground_at::GroundIndex::build(std::iter::once((
            [[20.,20.,1.],[60.,20.,1.],[20.,60.,1.]],18))));
        let probe = Vec3::new(32.,32.,1.05);
        assert!(crate::ground_at::at(probe,0.5).is_some());
        let area = world.around_with_tiles(center,true).unwrap();
        assert!(crate::ground_at::at(probe,0.5).is_none(), "without a raw auxiliary map, stale live ground is cleared");
        assert_eq!(area.triangles.len(),25);
        assert_eq!(area.rails.len(),25);
        assert!(area.tags.iter().all(|t| *t==NO_GRIND_TILE_TAG),"final tags bypass static rules");
        assert!(area.bounds.is_empty());
        assert!(!area.needs_live_watch(), "fully baked areas need no live stream scan");
        std::fs::remove_file(dir.join("live-cells.txt")).unwrap();
        let policy_error = world.around_with_tiles(center,true).err().unwrap();
        assert!(policy_error.contains("map-state policy"), "bad policy must not silently restore state-polluted fallback: {policy_error}");
        tiles::write_live_cells(&dir,64.,&tiles::rules_hash(),&Default::default()).unwrap();
        assert_eq!(area.triangles[0][0][1],1.,"GTA z becomes Skate y");
        std::fs::remove_file(tiles::path(&dir,(0,0))).unwrap();
        world.tile_cache = Default::default();
        assert!(world.around_with_tiles(center,true).is_err(),"a missing tile requires live collision");
        for c in tiles::cells([32.,32.],RADIUS,64.).into_iter().filter(|c| *c!=(0,0)) { std::fs::remove_file(tiles::path(&dir,c)).unwrap(); }
        std::fs::remove_file(dir.join("tile-size.txt")).unwrap();
        std::fs::remove_file(dir.join("rules.txt")).unwrap();
        std::fs::remove_file(dir.join("live-cells.txt")).unwrap();
        std::fs::remove_dir(dir).unwrap(); std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn mixed_selection_keeps_only_live_fragments_in_the_missing_tile() {
        let triangles = vec![[[-1.,0.,1.],[1.,0.,1.],[-1.,1.,1.]]];
        let area = Area { noses: Vec::new(), center:Vec3::ZERO,triangles:triangles.iter().map(|t| t.map(|p| coords::to_skate(Vec3::from_array(p)).to_array())).collect(),tags:vec![123],rails:vec![vec![[-1.,1.,0.],[1.,1.,0.]]],census:Default::default(),bevels:Default::default(),note:String::new(),bounds:Vec::new(),live_required:true,changed:None,fixed:0,options:0,origins:Default::default() };
        let selection = tiles::Selection { size:64.,baked:Default::default(),missing:vec![(0,0)],live_only:vec![(0,0)],stale:0 };
        let live = tiles::keep_live(area,&selection);
        assert!(live.needs_live_watch(), "an empty active state's later collision load must be observed");
        assert_eq!(live.triangles.len(),1);
        assert_eq!(live.tags,[123 | crate::live::LIVE_TAG], "live fragments are marked for the collision view");
        assert!(live.triangles.iter().flatten().all(|p| p[0]>=0.));
        assert_eq!(live.rails.len(),1);
        assert_eq!(live.rails[0],[[0.,1.,0.],[1.,1.,0.]]);
    }
}
