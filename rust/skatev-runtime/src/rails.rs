//! Grind rails found in collision: the lips a skater can grind.
//!
//! Adapted from chasmlol/2010-rust-rewrite-mashup @ ab43b8a9,
//! `crates/render_anim/src/skate/rails.rs` (Apache-2.0). The algorithm and its
//! tuned thresholds are unchanged; they are expressed in the source's map
//! units (inches). `find_metres` converts GTA metres to inches and back so no
//! constant is re-tuned for GTA. Changes from the source: glam via bevy_math,
//! the convexity gate (valley creases between bank facets are not lips),
//! buried-lip trimming (a handrail flight's top edge running on inside the
//! landing's rail is cut where it goes under), the metre wrapper, and tests.
//!
//! A lip is found by probing the geometry around each edge of a walkable face:
//! the ground must fall away just past the edge and nothing may rise there.
//! Surviving edges are merged along their lines across seams and T-junctions,
//! then chained into polylines where they meet at a gentle turn.

// Port keeps the source's inline map types verbatim.
#![allow(clippy::type_complexity)]

use rustc_hash::FxHashMap as HashMap;

use bevy_math::Vec3;

const INCH: f32 = 0.0254;

/// A face this upward is walkable; its edges are rail candidates.
const UPWARD_Z: f32 = 0.65;
/// XY grid cell for probing, in map units (inches).
const CELL: f32 = 64.0;
/// How far past the lip the probes look.
const PROBE_OUT: f32 = 2.0;
/// Open space the ground must drop below the lip, just past it.
const MIN_DROP: f32 = 3.0;
/// Height above the lip the wall probe runs at.
const WALL_PROBE_UP: f32 = 3.0;
/// Shortest rail kept, after merging and chaining.
const MIN_RAIL: f32 = 24.0;
/// Steepest rail, as rise over length: stair handrails and ramp edges pass.
const MAX_SLOPE: f32 = 0.7;
/// Upward faces folding less than this (45 degrees) are one curved surface.
const SEAM_COS: f32 = 0.707_107;
/// The volume probe: below an open edge, how far down (just under the lip
/// for thin slabs and seats, and deeper) and how far in a face must stand
/// (inches).
const SUPPORT_DOWN: [f32; 2] = [1.0, 3.0];
const SUPPORT_IN: f32 = 8.0;
/// Chains continue through a joint turning less than about 35 degrees.
const MIN_TURN_COS: f32 = 0.82;
/// SkateV: a face across the edge rising more than about one degree above
/// the walkable face's plane makes the edge concave (sin 1 degree).
const CONCAVE_SIN: f32 = 0.017_452;
/// SkateV: lip stretches with collision this close above them are buried
/// (inside another solid) and trimmed; a board there hits that solid.
const BURIED_CLEARANCE: f32 = 18.0;
/// SkateV: spacing of the buried-lip samples along a lip.
const BURIED_STEP: f32 = 2.0;
/// SkateV: the buried test also looks this far to either side of the lip (a
/// truck's half width): a handrail tube's other top edge lies just beside it.
const BURIED_SIDE: f32 = 4.0;
/// SkateV: probes skip faces whose height span misses the probe's by more
/// than this (inches).
const HIT_Z_MARGIN: f32 = 1.0;
/// The skate engine counts rails in a u16.
const MAX_RAILS: usize = u16::MAX as usize;

#[derive(Debug, Default, Clone, Copy)]
pub struct RailCensus {
    pub candidates: usize,
    pub lips: usize,
    pub runs: usize,
    pub rails: usize,
    /// Rails dropped as duplicates of a longer rail on the same lip.
    pub duplicates: usize,
}

/// The lips each candidate edge gave last build (`reuse`), by edge identity:
/// its key and the face that defines it (the first upward face naming it).
#[derive(Default)]
pub struct RailCache {
    edges: HashMap<u64, (u32, u32)>,
    lips: Vec<(Vec3, Vec3)>,
}

/// The probes reach this far (metres) from an edge: past it, nothing a lip
/// test reads (`reuse::Dirty` pad).
const REUSE_PAD: f32 = 0.5;

/// Developer collision view only: candidate edges within `DIAG_RADIUS` of a
/// point (GTA space, metres) that did not become rails, with the reason, so a
/// bench or ledge that cannot be ground shows which gate dropped it.
static DIAG_AT: std::sync::Mutex<Option<Vec3>> = std::sync::Mutex::new(None);
static DIAG_OUT: std::sync::Mutex<Vec<Rejected>> = std::sync::Mutex::new(Vec::new());
const DIAG_RADIUS: f32 = 3.0;
const DIAG_MAX: usize = 24;

/// A candidate edge that did not become a rail: midpoint (GTA metres),
/// length (metres) and why.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rejected {
    pub mid: Vec3,
    pub length: f32,
    pub reason: &'static str,
}

/// Where to explain rejected edges (None: off, the normal case).
pub fn diagnosing() -> bool { DIAG_AT.lock().unwrap_or_else(|e| e.into_inner()).is_some() }

pub fn set_diagnostics(at: Option<Vec3>) {
    *DIAG_AT.lock().unwrap_or_else(|e| e.into_inner()) = at;
}

/// The rejected edges near the diagnostic point from the latest builds.
pub fn take_rejected() -> Vec<Rejected> {
    std::mem::take(&mut *DIAG_OUT.lock().unwrap_or_else(|e| e.into_inner()))
}

fn reject(focus: Option<(Vec3, f32)>, metres: f32, a: Vec3, b: Vec3, reason: &'static str) {
    let Some((at, radius)) = focus else { return };
    let mid = (a + b) * 0.5;
    if mid.distance(at) > radius {
        return;
    }
    let mut out = DIAG_OUT.lock().unwrap_or_else(|e| e.into_inner());
    if out.len() < DIAG_MAX {
        out.push(Rejected { mid: mid * metres, length: a.distance(b) * metres, reason });
    }
}

/// Rails over GTA-space triangles in metres (Z up), as polylines in metres.
pub fn find_metres(tris: &[[Vec3; 3]]) -> (Vec<Vec<Vec3>>, RailCensus) {
    find_metres_reusing(tris, &crate::reuse::Dirty::everything(), &mut RailCache::default())
}

/// As `find_metres`, keeping the lips of edges `dirty` leaves untouched from
/// `cache`, which then holds this build's.
pub fn find_metres_reusing(tris: &[[Vec3; 3]], dirty: &crate::reuse::Dirty, cache: &mut RailCache) -> (Vec<Vec<Vec3>>, RailCensus) {
    let inches: Vec<[Vec3; 3]> = tris.iter().map(|t| t.map(|v| v / INCH)).collect();
    let (rails, census) = find_reusing(&inches, dirty, INCH, cache);
    (
        rails
            .into_iter()
            .map(|r| r.into_iter().map(|v| v * INCH).collect())
            .collect(),
        census,
    )
}

/// Rails over `tris` (map units, z up), each a polyline of two or more points.
pub fn find(tris: &[[Vec3; 3]]) -> (Vec<Vec<Vec3>>, RailCensus) {
    find_reusing(tris, &crate::reuse::Dirty::everything(), 1.0, &mut RailCache::default())
}

/// As `find`; `metres` converts map units to `dirty`'s metres.
fn find_reusing(tris: &[[Vec3; 3]], dirty: &crate::reuse::Dirty, metres: f32, cache: &mut RailCache) -> (Vec<Vec<Vec3>>, RailCensus) {
    let grid = Grid::build(tris);
    let mut probe = Probe {
        tris,
        grid: &grid,
        stamp: vec![0; tris.len()],
        round: 0,
        scratch: Vec::new(),
    };

    // Every edge of a walkable face, once.
    let key = |v: Vec3| v.to_array().map(|x| (x * 8.).round() as i32);
    let edge_key = |a: Vec3, b: Vec3| {
        let (ka, kb) = (key(a), key(b));
        if ka < kb { (ka, kb) } else { (kb, ka) }
    };
    // Every edge of a walkable face (the first face to name it), in key
    // order so the rails come out the same on every run.
    let mut slot: HashMap<([i32; 3], [i32; 3]), u32> = HashMap::default();
    let mut edges: Vec<(([i32; 3], [i32; 3]), (Vec3, Vec3, Vec3, Vec3, u32))> = Vec::new();
    for (t, tri) in tris.iter().enumerate() {
        let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
        if normal.z <= UPWARD_Z {
            continue;
        }
        let centroid = (tri[0] + tri[1] + tri[2]) / 3.;
        for i in 0..3 {
            let (a, b) = (tri[i], tri[(i + 1) % 3]);
            let k = edge_key(a, b);
            slot.entry(k).or_insert_with(|| {
                edges.push((k, (a, b, normal, centroid, t as u32)));
                edges.len() as u32 - 1
            });
        }
    }
    edges.sort_unstable_by_key(|e| e.0);
    for (i, (k, _)) in edges.iter().enumerate() {
        slot.insert(*k, i as u32);
    }
    // SkateV: every face across each candidate edge and its far vertex, for
    // the convexity gate and the lip probes below (per edge, a list threaded
    // through `fars`).
    let mut head = vec![u32::MAX; edges.len()];
    let mut fars: Vec<(Vec3, u32, u32)> = Vec::new();
    for (t, tri) in tris.iter().enumerate() {
        for i in 0..3 {
            if let Some(&e) = slot.get(&edge_key(tri[i], tri[(i + 1) % 3])) {
                fars.push((tri[(i + 2) % 3], t as u32, head[e as usize]));
                head[e as usize] = fars.len() as u32 - 1;
            }
        }
    }
    let across = |e: usize| {
        std::iter::successors(Some(head[e]).filter(|&f| f != u32::MAX), |&f| Some(fars[f as usize].2).filter(|&n| n != u32::MAX))
            .map(|f| (fars[f as usize].0, fars[f as usize].1))
    };
    let mut faces = Vec::new();
    let candidates = edges.len();
    let focus = DIAG_AT.lock().unwrap_or_else(|e| e.into_inner()).map(|at| (at / metres, DIAG_RADIUS / metres));

    let mut lips = Vec::new();
    let mut next = RailCache::default();
    next.edges.reserve(edges.len());
    for (e, &(k, (a, b, normal, centroid, own_t))) in edges.iter().enumerate() {
        let id = k.0.iter().chain(&k.1).map(|&x| x as u32)
            .chain([a, b, normal, centroid].iter().flat_map(|v| v.to_array()).map(f32::to_bits))
            .fold(0u64, |h, x| crate::reuse::mix(h ^ x as u64));
        let start = lips.len();
        // Diagnosed edges are probed again so each one reports its reason.
        let diagnosed = focus.is_some_and(|(at, radius)| ((a + b) * 0.5).distance(at) <= radius);
        let reused = (!diagnosed && !dirty.touches(a.min(b) * metres, a.max(b) * metres, REUSE_PAD))
            .then(|| cache.edges.get(&id))
            .flatten()
            .map(|&(s, n)| lips.extend_from_slice(&cache.lips[s as usize..(s + n) as usize]))
            .is_some();
        'probe: {
            if reused {
                break 'probe;
            }
            let along = b - a;
            let len = along.length();
            // Weld: an edge without a face across it by vertex match may still
            // lie on another part's edge (model parts meeting a few mm apart,
            // or a T-junction). That face counts as across it, so a curved
            // surface's band seam is a seam and not an open lip (Manatros
            // 2026-10-25: grind lines at the bowl's model seams).
            let welded: Vec<(Vec3, u32)> = if across(e).all(|(_, t)| t == own_t) {
                weld_partners(tris, &grid, a, b, own_t)
            } else {
                Vec::new()
            };
            let across = |e: usize| across(e).chain(welded.iter().copied());
            if len < 1. || along.z.abs() > MAX_SLOPE * len {
                reject(focus, metres, a, b, if len < 1. { "edge under 1 inch" } else { "edge too steep" });
                break 'probe;
            }
            // SkateV: a lip is a convex edge. Where a face across the edge rises
            // above this face's plane the ground climbs (a quarter-pipe or bank
            // facet crease); the probes below run along the tilted face and can
            // miss that, so it is rejected here.
            let rises = across(e).any(|(far, t)| {
                let from = far - a;
                let off_edge = from - along * (from.dot(along) / (len * len));
                t != own_t && off_edge.length() > 1e-3 && from.dot(normal) > CONCAVE_SIN * off_edge.length()
            });
            if rises {
                reject(focus, metres, a, b, "not convex (ground rises across the edge)");
                break 'probe;
            }
            // A shallow fold shared by two upward faces lying on opposite sides
            // of the edge is a curved surface's facet seam: rounded wall tops,
            // bowl transitions, convex crests (Manatros 2026-10-22 21:42: only
            // edges steep enough to grind). Real lips meet a steep side.
            if across(e).any(|(far, t)| {
                let tri = tris[t as usize];
                let other = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
                t != own_t
                    && along.cross(far - a).dot(along.cross(centroid - a)) < 0.
                    && normal.z >= 0.35 && other.z >= 0.35
                    && normal.dot(other) > SEAM_COS
            }) {
                reject(focus, metres, a, b, "continuous surface facet seam or rounded edge (under 45 degrees)");
                break 'probe;
            }
            let mid = (a + b) * 0.5;
            let mut out = along.cross(normal).normalize_or_zero();
            if out.dot(centroid - mid) > 0. {
                out = -out;
            }
            // An open edge (no face across it: a mesh border, a hole outline,
            // a loose panel or wedge laid over the ground) is a lip only with
            // a face right beneath it, as a ledge's or curb's side (Manatros
            // 2026-10-22 21:42: grindable edges need volume geometry).
            if across(e).all(|(_, t)| t == own_t) && !probe.supported(mid, out) {
                reject(focus, metres, a, b, "open edge without a face beneath (mesh border, hole or seam)");
                break 'probe;
            }
            let samples: &[f32] = if len < 12. {
                &[0.5]
            } else {
                &[0.25, 0.5, 0.75]
            };
            faces.clear();
            faces.extend(across(e).map(|(_, t)| t));
            // Two thirds of the samples must pass: stop once that is decided.
            let need = (samples.len() * 2).div_ceil(3);
            let (mut passing, mut failing) = (0, 0);
            for &s in samples {
                if passing >= need || failing > samples.len() - need {
                    break;
                }
                if probe.is_lip(a + along * s, out, &faces) {
                    passing += 1;
                } else {
                    failing += 1;
                }
            }
            if passing >= need {
                let before = lips.len();
                lips.extend(probe.unburied(a, b));
                if lips.len() == before {
                    reject(focus, metres, a, b, "no clearance above (buried under a back/arm/overhang)");
                }
            } else {
                reject(focus, metres, a, b, "lip probes failed (support right outside, too small a drop, or a wall above)");
            }
        }
        next.edges.insert(id, (next.lips.len() as u32, (lips.len() - start) as u32));
        next.lips.extend_from_slice(&lips[start..]);
    }
    *cache = next;
    let lip_count = lips.len();

    let runs = merge_collinear(lips);
    let run_count = runs.len();
    let mut rails = chain(runs);
    rails.retain(|rail| {
        let long = polyline_len(rail) >= MIN_RAIL;
        if !long && rail.len() >= 2 {
            reject(focus, metres, rail[0], rail[rail.len() - 1], "rail shorter than 24 inches");
        }
        long
    });
    let duplicates = dedupe(&mut rails);
    if rails.len() > MAX_RAILS {
        rails.sort_by(|a, b| polyline_len(b).total_cmp(&polyline_len(a)));
        rails.truncate(MAX_RAILS);
    }
    let census = RailCensus {
        candidates,
        lips: lip_count,
        runs: run_count,
        rails: rails.len(),
        duplicates,
    };
    (rails, census)
}

/// SkateV: a rail lying wholly within a wheel's diameter of a longer rail is
/// the same lip to a skater (stacked GTA faces, a deck skin over its ramp,
/// yield lips one to three centimetres apart; one wheel spans both, the grind
/// probes cannot tell them apart and the duplicates multiply the catches
/// along that lip). The longer rail is kept. Returns how many were dropped.
/// `dedupe` for rails in metres (the park files' rails, `parks.rs`).
/// Step noses of stair flights GTA models as real steps (metres, z up):
/// every nose is a lip to the finder, so a flight came out as a ladder of
/// short parallel rails (Vespucci bowl stairs, Manatros 2026-10-22 21:04
/// video 09:02: "the stairs here are all rails"). Level rails that climb in
/// a stair pattern, three or more in a row, are dropped; a single curb, two
/// ledge edges at one height and stacked scaffold bars are no flight.
pub fn off_flight_noses(rails: Vec<Vec<Vec3>>) -> Vec<Vec<Vec3>> {
    off_flight_noses_split(rails).0
}

/// As [`off_flight_noses`], returning the nose lines too (StepRails).
pub fn off_flight_noses_split(rails: Vec<Vec<Vec3>>) -> (Vec<Vec<Vec3>>, Vec<Vec<Vec3>>) {
    const RUN: std::ops::RangeInclusive<f32> = 0.18..=0.55;
    const RISE: std::ops::RangeInclusive<f32> = 0.08..=0.30;
    /// Rise over run of a flight: about 14 to 51 degrees.
    const PITCH: std::ops::RangeInclusive<f32> = 0.25..=1.25;
    const MIN_FLIGHT: usize = 3;
    struct Nose { a: Vec3, dir: Vec3, len: f32, mid: Vec3 }
    let noses: Vec<Option<Nose>> = rails.iter().map(|r| {
        let (mut a, mut b) = (*r.first()?, *r.last()?);
        // One direction for every nose, so "to the left of it" means the
        // same side for all of them.
        if b.x < a.x || (b.x == a.x && b.y < a.y) {
            std::mem::swap(&mut a, &mut b);
        }
        let flat = Vec3::new(b.x - a.x, b.y - a.y, 0.);
        let len = flat.length();
        let level = len >= 0.3 && (b.z - a.z).abs() <= 0.1 * len
            && r.iter().all(|p| (p.z - a.z).abs() <= 0.05 + 0.1 * len);
        level.then(|| Nose { a, dir: flat / len, len, mid: (a + b) * 0.5 })
    }).collect();
    // Noses one step apart, with the side the flight climbs towards.
    let mut steps: Vec<(usize, usize, bool)> = Vec::new();
    for i in 0..noses.len() {
        let Some(n) = &noses[i] else { continue };
        let left = Vec3::new(-n.dir.y, n.dir.x, 0.);
        for j in i + 1..noses.len() {
            let Some(m) = &noses[j] else { continue };
            if n.dir.dot(m.dir) < 0.985 {
                continue;
            }
            let rise = m.mid.z - n.mid.z;
            let side = (m.mid - n.mid).dot(left);
            if !RISE.contains(&rise.abs()) || !RUN.contains(&side.abs()) || !PITCH.contains(&(rise.abs() / side.abs())) {
                continue;
            }
            // Overlap along the nose by half the shorter one.
            let (s0, s1) = ((m.a - n.a).dot(n.dir), (m.a - n.a).dot(n.dir) + m.len);
            if s1.min(n.len) - s0.max(0.) < 0.5 * n.len.min(m.len) {
                continue;
            }
            steps.push((i, j, (rise > 0.) == (side > 0.)));
        }
    }
    let mut parent: Vec<usize> = (0..rails.len()).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i { p[i] = p[p[i]]; i = p[i]; }
        i
    }
    for &(i, j, _) in &steps {
        let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
        parent[ri] = rj;
    }
    // A flight climbs one way: a group with steps both ways (ledges up and
    // down again) is no flight.
    let mut climb: rustc_hash::FxHashMap<usize, Option<bool>> = Default::default();
    for &(i, _, up) in &steps {
        let r = root(&mut parent, i);
        let e = climb.entry(r).or_insert(Some(up));
        if *e != Some(up) { *e = None; }
    }
    let mut size = vec![0usize; rails.len()];
    for i in 0..rails.len() {
        if noses[i].is_some() {
            let r = root(&mut parent, i);
            size[r] += 1;
        }
    }
    let flight: Vec<bool> = (0..rails.len())
        .map(|i| {
            let r = root(&mut parent, i);
            noses[i].is_some() && size[r] >= MIN_FLIGHT && climb.get(&r).is_some_and(|c| c.is_some())
        })
        .collect();
    let (noses, kept): (Vec<_>, Vec<_>) = rails.into_iter().zip(flight).partition(|(_, f)| *f);
    (kept.into_iter().map(|(r, _)| r).collect(), noses.into_iter().map(|(r, _)| r).collect())
}

pub fn dedupe_metres(rails: &mut Vec<Vec<Vec3>>) -> usize {
    let mut inches: Vec<Vec<Vec3>> = rails.iter().map(|r| r.iter().map(|&p| p / INCH).collect()).collect();
    let dropped = dedupe(&mut inches);
    *rails = inches.into_iter().map(|r| r.into_iter().map(|p| p * INCH).collect()).collect();
    dropped
}

fn dedupe(rails: &mut Vec<Vec<Vec3>>) -> usize {
    // GTAxSkate: at least 9 cm (3.5 inches). A faceted tube or a narrow top
    // gave lines 7 cm apart that all survived (2026-10-19 Vespucci drops:
    // three lines per placed rail); trucks span them all.
    let radius = (2.0 * skate_core::physics::board::RETAIL_WHEEL_RADIUS / INCH).max(DUPLICATE_SPAN);
    let mut order: Vec<usize> = (0..rails.len()).collect();
    order.sort_by(|&a, &b| polyline_len(&rails[b]).total_cmp(&polyline_len(&rails[a])));
    // Kept segments by grid cell (every cell a segment's padded box touches).
    let mut cells: HashMap<(i32, i32), Vec<(Vec3, Vec3)>> = HashMap::default();
    let mut keep = vec![false; rails.len()];
    // Uncovered stretches of dropped lines (a unique tail of a mostly covered
    // line used to vanish with it; Codex 2026-10-25). Inches.
    let mut tails: Vec<Vec<Vec3>> = Vec::new();
    for i in order {
        let rail = &rails[i];
        let near = |p: Vec3| {
            cells.get(&(cell_of(p.x), cell_of(p.y))).is_some_and(|segs| {
                segs.iter().any(|&(a, b)| {
                    let ab = b - a;
                    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0., 1.);
                    p.distance(a + ab * t) <= radius
                })
            })
        };
        // Covered when nearly all of it lies on kept rails (ends of facet
        // lines differ by a few cm, so one sample past an end kept them all).
        let (mut samples, mut hits) = (0usize, 0usize);
        for w in rail.windows(2) {
            let steps = (w[0].distance(w[1]) / radius).ceil().max(1.) as usize;
            for k in 0..=steps {
                samples += 1;
                hits += usize::from(near(w[0].lerp(w[1], k as f32 / steps as f32)));
            }
        }
        let covered = samples > 0 && hits * 100 >= samples * COVERED_PERCENT;
        if covered {
            let mut run: Vec<Vec3> = Vec::new();
            let flush = |run: &mut Vec<Vec3>, tails: &mut Vec<Vec<Vec3>>| {
                if run.len() >= 2 && polyline_len(run) >= TAIL_MIN { tails.push(std::mem::take(run)); } else { run.clear(); }
            };
            for w in rail.windows(2) {
                let steps = (w[0].distance(w[1]) / radius).ceil().max(1.) as usize;
                for k in 0..=steps {
                    let p = w[0].lerp(w[1], k as f32 / steps as f32);
                    if near(p) { flush(&mut run, &mut tails); } else if run.last() != Some(&p) { run.push(p); }
                }
            }
            flush(&mut run, &mut tails);
            continue;
        }
        keep[i] = true;
        for w in rail.windows(2) {
            let (lo, hi) = (w[0].min(w[1]) - Vec3::splat(radius), w[0].max(w[1]) + Vec3::splat(radius));
            for x in cell_of(lo.x)..=cell_of(hi.x) {
                for y in cell_of(lo.y)..=cell_of(hi.y) {
                    cells.entry((x, y)).or_default().push((w[0], w[1]));
                }
            }
        }
    }
    // The kept (longest) rail may be a lower facet edge of a tube or a stack:
    // each of its points moves onto the top of the lips it stands for, centred
    // across the top band (2026-10-18: the board sank into rails and railings;
    // Codex: the longest, not the highest, duplicate was kept).
    let dropped: Vec<(Vec3, Vec3)> = rails.iter().zip(&keep).filter(|(_, k)| !**k)
        .flat_map(|(r, _)| r.windows(2).map(|w| (w[0], w[1])).collect::<Vec<_>>()).collect();
    if !dropped.is_empty() {
        let mut by_cell: HashMap<(i32, i32), Vec<u32>> = HashMap::default();
        for (i, &(a, b)) in dropped.iter().enumerate() {
            let (lo, hi) = (a.min(b) - Vec3::splat(radius), a.max(b) + Vec3::splat(radius));
            for x in cell_of(lo.x)..=cell_of(hi.x) {
                for y in cell_of(lo.y)..=cell_of(hi.y) {
                    by_cell.entry((x, y)).or_default().push(i as u32);
                }
            }
        }
        for (rail, _) in rails.iter_mut().zip(&keep).filter(|(_, k)| **k) {
            // The rail moves as one piece when its points agree on the shift
            // (facet lines end a few cm apart, so an end point may have no
            // partner); otherwise only points with partners move, and only
            // when every point has one (one raised vertex would kink it).
            let crowned: Vec<Option<Vec3>> = rail.iter()
                .map(|p| crown(*p, by_cell.get(&(cell_of(p.x), cell_of(p.y))).map_or(&[][..], |v| v), &dropped, radius))
                .collect();
            let shifts: Vec<Vec3> = rail.iter().zip(&crowned).filter_map(|(p, c)| c.map(|c| c - *p)).collect();
            if let Some(&first) = shifts.first() {
                let mean = shifts.iter().copied().sum::<Vec3>() / shifts.len() as f32;
                if shifts.iter().all(|s| s.distance(first) <= 0.4) {
                    for p in rail.iter_mut() {
                        *p += mean;
                    }
                } else if crowned.iter().all(Option::is_some) {
                    *rail = crowned.into_iter().flatten().collect();
                }
            }
        }
    }
    let before = rails.len();
    let mut k = keep.into_iter();
    rails.retain(|_| k.next().unwrap_or(true));
    let dropped = before - rails.len();
    rails.extend(tails);
    dropped
}

/// Shortest uncovered tail (inches, 30 cm) a dropped duplicate leaves behind.
const TAIL_MIN: f32 = 11.8;
/// Duplicate lines closer than this (inches) are one rail (`dedupe`).
const DUPLICATE_SPAN: f32 = 3.5;
/// Share of a line (percent) on a kept rail that makes it a duplicate.
const COVERED_PERCENT: usize = 75;

/// Inches below the highest duplicate that still count as the top band.
const TOP_BAND: f32 = 0.25;

/// `p` on the top of the duplicate lips around it (`near` indexes `segs`):
/// the highest within `radius` across and up/down, centred across the points
/// within `TOP_BAND` of that height. None when no duplicate lies more than
/// `TOP_BAND` above `p` (`p` is already on top).
fn crown(p: Vec3, near: &[u32], segs: &[(Vec3, Vec3)], radius: f32) -> Option<Vec3> {
    let mut points = Vec::new();
    for &i in near {
        let (a, b) = segs[i as usize];
        let ab = (b - a).truncate();
        let t = ((p.truncate() - a.truncate()).dot(ab) / ab.length_squared().max(1e-12)).clamp(0., 1.);
        let q = a.lerp(b, t);
        if q.truncate().distance(p.truncate()) <= radius && (q.z - p.z).abs() <= radius {
            points.push(q);
        }
    }
    if points.is_empty() {
        return None;
    }
    // Duplicates at the same height (a pipe's two top facet edges, a narrow
    // cap's two lips) average to the middle, as a rail's crown line in Skate
    // (Manatros 2026-10-24: the placed rail's line sat on one side; "merged
    // rails should lie in between"). Higher duplicates lift a lower flank
    // onto the top band, centred across it.
    let top = points.iter().map(|q| q.z).fold(p.z, f32::max);
    let mut band: Vec<Vec3> = points.into_iter().filter(|q| q.z >= top - TOP_BAND).collect();
    if p.z >= top - TOP_BAND {
        band.push(p);
    }
    let xy = band.iter().map(|q| q.truncate()).sum::<bevy_math::Vec2>() / band.len() as f32;
    Some(Vec3::new(xy.x, xy.y, top))
}

/// The old SkateGTA plugin's rail rule (gta/skate-gta/src/parks.rs), used on
/// an object's own shape (dynamic.rs): an edge two triangles share where the
/// surface folds down by more than 50 degrees (coping, a ledge's lip, a
/// ramp's top bar), with a walkable face on one side and running within 44
/// degrees of level; edges meeting in line are joined, and only rails at
/// least 0.8 m long are kept. Metres.
pub fn fold_rails(triangles: &[[Vec3; 3]]) -> Vec<Vec<Vec3>> {
    const MIN_FOLD: f32 = 0.64; // cos 50
    const MAX_SLOPE: f32 = 0.7; // sin 44, as the finder (the old plugin had 25)
    const MIN_LENGTH: f32 = 0.8;
    let key = |p: Vec3| ((p.x * 50.).round() as i64, (p.y * 50.).round() as i64, (p.z * 50.).round() as i64);
    let mut edges: std::collections::BTreeMap<((i64, i64, i64), (i64, i64, i64)), Vec<usize>> = std::collections::BTreeMap::new();
    let normals: Vec<Vec3> = triangles.iter().map(|t| (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero()).collect();
    for (i, t) in triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (key(t[k]), key(t[(k + 1) % 3]));
            edges.entry(if a < b { (a, b) } else { (b, a) }).or_default().push(i);
        }
    }
    let mut pieces: Vec<(Vec3, Vec3)> = Vec::new();
    for (&(ka, kb), sharing) in &edges {
        let [i, j] = sharing[..] else { continue };
        let (ni, nj) = (normals[i], normals[j]);
        if ni.dot(nj) > MIN_FOLD || ni.z.max(nj.z) < 0.7 {
            continue;
        }
        let point = |k: (i64, i64, i64)| Vec3::new(k.0 as f32, k.1 as f32, k.2 as f32) / 50.;
        let (a, b) = (point(ka), point(kb));
        let along = b - a;
        if along.length() < 0.05 || (along.z / along.length()).abs() > MAX_SLOPE {
            continue;
        }
        // Convex: each triangle lies below the other's plane.
        let middle = (a + b) * 0.5;
        let centre = |t: usize| (triangles[t][0] + triangles[t][1] + triangles[t][2]) / 3.;
        if (centre(j) - middle).dot(ni) > 0.01 || (centre(i) - middle).dot(nj) > 0.01 {
            continue;
        }
        pieces.push((a, b));
    }
    // Join pieces end to end while they run on in line.
    let mut rails: Vec<Vec<Vec3>> = Vec::new();
    let mut used = vec![false; pieces.len()];
    let near = |p: Vec3, q: Vec3| p.distance(q) < 0.03;
    for start in 0..pieces.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut rail = vec![pieces[start].0, pieces[start].1];
        for forward in [true, false] {
            loop {
                let (end, before) = if forward { (rail[rail.len() - 1], rail[rail.len() - 2]) } else { (rail[0], rail[1]) };
                let heading = (end - before).normalize_or_zero();
                let next = (0..pieces.len()).find(|&k| {
                    !used[k] && {
                        let (a, b) = pieces[k];
                        let other = if near(a, end) { Some(b) } else if near(b, end) { Some(a) } else { None };
                        other.is_some_and(|o| (o - end).normalize_or_zero().dot(heading) > 0.9)
                    }
                });
                let Some(k) = next else { break };
                used[k] = true;
                let (a, b) = pieces[k];
                let other = if near(a, end) { b } else { a };
                if forward {
                    rail.push(other);
                } else {
                    rail.insert(0, other);
                }
            }
        }
        let length: f32 = rail.windows(2).map(|w| w[0].distance(w[1])).sum();
        if length >= MIN_LENGTH {
            rails.push(rail);
        }
    }
    rails
}

#[cfg(test)]
mod fold_regressions {
    use super::*;

    #[test]
    fn lower_and_sloping_copings_survive_taller_unrelated_geometry() {
        for rise in [0., 0.6] {
            let a = Vec3::new(0., 0., 1.);
            let b = Vec3::new(2., 0., 1. + rise);
            let tris = [
                [a, b, Vec3::new(0., 1., 1.)],
                [b, a, Vec3::new(0., 0., 0.)],
                [Vec3::new(10., 10., 4.), Vec3::new(11., 10., 4.), Vec3::new(10., 11., 4.)],
            ];
            let rails = fold_rails(&tris);
            assert!(rails.iter().any(|r| r.iter().any(|p| p.distance(a) < 0.03)
                && r.iter().any(|p| p.distance(b) < 0.03)), "rise {rise}: {rails:?}");
        }
    }
}

/// Rail pieces that meet end to end are one rail (2026-10-20 test: "the
/// rails do not connect; there are gaps you grind through; make them
/// continuous"): ends within `JOIN_GAP` whose directions continue within
/// `JOIN_TURN` are joined into one polyline, from any source (finder, tube,
/// track, visual). Returns how many joins were made.
pub fn join_ends(rails: &mut Vec<Vec<Vec3>>) -> usize {
    join_ends_within(rails, 0.22)
}

/// Gap a visible-model rail's bend may leave (metres): the finder takes the
/// straight tubes but not the curved piece between them (Manatros
/// 2026-10-25 mark 9: a metro handrail broke 1.1 m between its landing piece
/// and the next flight).
pub const BEND_GAP: f32 = 1.3;
/// Least turn (radians) across a bridged gap wider than the usual one.
const BEND_TURN: f32 = 8.0 * std::f32::consts::PI / 180.0;

/// How far a piece may start behind the previous one's end and still join
/// (m), and how far to the side (m): track edge lines lie 3 cm or more apart
/// (A49) and must never fuse.
const OVERLAP_JOIN: f32 = 0.12;
const OVERLAP_SIDE: f32 = 0.02;

/// As [`join_ends`], with ends up to `gap` apart (connector and both ends
/// within 35 degrees of one direction).
pub fn join_ends_within(rails: &mut Vec<Vec<Vec3>>, gap: f32) -> usize {
    join_ends_bridging(rails, gap, &|_, _| false)
}

/// As [`join_ends_within`]; a gap wider than the usual one without a bend
/// also joins when `bridge(end, start)` says the geometry continues along it.
pub fn join_ends_bridging(rails: &mut Vec<Vec<Vec3>>, gap: f32, bridge: &dyn Fn(Vec3, Vec3) -> bool) -> usize {
    let join_gap = gap;
    let min_dot = 35.0f32.to_radians().cos();
    let bucket = gap.max(0.25);
    let reach = (gap / bucket).ceil() as i32;
    let key = |p: Vec3| ((p.x/bucket).floor() as i32, (p.y/bucket).floor() as i32, (p.z/bucket).floor() as i32);
    let tip = |r: &[Vec3], side: usize| if side==0 { r[0] } else { r[r.len()-1] };
    let out = |r: &[Vec3], side: usize| if side==0 {
        (r[0]-r[1]).normalize_or_zero()
    } else { (r[r.len()-1]-r[r.len()-2]).normalize_or_zero() };
    let mut ends: rustc_hash::FxHashMap<_,Vec<(usize,usize)>> = Default::default();
    for (i,r) in rails.iter().enumerate().filter(|(_,r)|r.len()>=2) {
        for side in 0..2 { ends.entry(key(tip(r,side))).or_default().push((i,side)); }
    }
    let mut candidates = Vec::new();
    for (i,r) in rails.iter().enumerate().filter(|(_,r)|r.len()>=2) {
        for side in 0..2 {
            let p=tip(r,side); let direction=out(r,side);
            let (x,y,z)=key(p);
            for dx in -reach..=reach { for dy in -reach..=reach { for dz in -reach..=reach {
                let Some(list)=ends.get(&(x+dx,y+dy,z+dz)) else {continue};
                for &(j,other) in list {
                    if j<=i {continue;}
                    let q=tip(&rails[j],other); let distance=p.distance(q);
                    let incoming=-out(&rails[j],other);
                    let alignment=direction.dot(incoming);
                    if distance>join_gap || alignment<min_dot {continue;}
                    // Pieces that overlap a little (the next one starts up to
                    // 12 cm behind this end, within 2 cm sideways) are one
                    // rail too (2026-10-26 mark 4: a curved ledge in ten
                    // pieces, each starting 4 cm back). Both pieces must be
                    // longer than twice the overlap.
                    // Measured along both pieces' mean direction: on a curve
                    // the next piece starts a little to the inside.
                    let mean=(direction+incoming).normalize_or_zero();
                    let delta=q-p; let along=delta.dot(mean);
                    if along<0.0 {
                        let lateral=(delta-mean*along).length();
                        let lengths=polyline_len(r).min(polyline_len(&rails[j]));
                        if -along<=OVERLAP_JOIN && lateral<=OVERLAP_SIDE && lengths>-2.0*along {
                            candidates.push((distance,-alignment,i,side,j,other));
                        }
                        continue;
                    }
                    // Past the usual gap only a bend: the direction changes
                    // (8 degrees or more). A straight railing's opening stays
                    // open (Codex peer review 2026-10-25).
                    if distance>0.22 && alignment>BEND_TURN.cos() && !bridge(p,q) {continue;}
                    // A short lateral/backward connector creates a sharp kink
                    // even when both original rails point the same way. Under
                    // 3 cm it is a seam (Codex 2026-10-25, mark 12: track
                    // edges 12 mm apart across a tile border, the connector
                    // along the border).
                    if distance>=0.03 {
                        let connector=(q-p)/distance;
                        if connector.dot(direction)<min_dot || connector.dot(incoming)<min_dot {continue;}
                    }
                    candidates.push((distance,-alignment,i,side,j,other));
                }
            }}}
        }
    }
    candidates.sort_unstable_by(|a,b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))
        .then((a.2,a.3,a.4,a.5).cmp(&(b.2,b.3,b.4,b.5))));
    let mut links=vec![[None;2];rails.len()];
    for (_,_,i,side,j,other) in candidates {
        if links[i][side].is_none() && links[j][other].is_none() {
            links[i][side]=Some((j,other)); links[j][other]=Some((i,side));
        }
    }
    let mut used=vec![false;rails.len()];
    let mut joined=Vec::with_capacity(rails.len());
    // Open chains first, then closed rings. Every original rail is copied once;
    // the old loop rebuilt all endpoint buckets after every individual join.
    for closed in [false,true] {
        for first in 0..rails.len() {
            if used[first] {continue;}
            if rails[first].len()<2 {used[first]=true;joined.push(rails[first].clone());continue;}
            let side=if links[first][0].is_none(){0}else if links[first][1].is_none(){1}else if closed{0}else{continue};
            let (mut i,mut entry)=(first,side); let mut points=Vec::new();
            loop {
                used[i]=true;
                let r=&rails[i];
                for k in 0..r.len() {
                    let p=if entry==0 {r[k]}else{r[r.len()-1-k]};
                    if k==0 && points.last().is_some_and(|q: &Vec3|q.distance(p)<0.01){continue;}
                    // An overlapping start: both ends meet at their middle
                    // (no backward kink).
                    if k==0 && points.len()>=2 {
                        let (a,b)=(points[points.len()-2],points[points.len()-1]);
                        if (p-b).dot((b-a).normalize_or_zero())< -1e-4 {
                            *points.last_mut().unwrap()=(b+p)*0.5;
                            continue;
                        }
                    }
                    points.push(p);
                }
                let Some((next,next_entry))=links[i][1-entry] else {break};
                if used[next] {
                    if next==first && next_entry==side && points.last()!=points.first() {
                        points.push(points[0]);
                    }
                    break;
                }
                (i,entry)=(next,next_entry);
            }
            joined.push(points);
        }
    }
    let joins=rails.len()-joined.len();
    *rails=joined;
    joins
}

/// Remove only overlapping portions of nearly collinear rails, retaining every
/// uncovered tail and real bend. Whole-line coverage alone leaves duplicates
/// whenever two sources cover different extents of the same physical edge.
/// Metres, Z up; lines within 2 cm (and 8 degrees) count as the same edge, so
/// neighbouring physical edges further apart are left alone. A tail's cut end
/// moves onto the line that covered the overlap, so `join_ends` can connect
/// them (a 15 mm sideways offset left two pieces, Codex 2026-10-25).
pub fn trim_overlaps(rails: &mut Vec<Vec<Vec3>>) {
    const EPS: f32=0.02;
    let mut order:Vec<_>=(0..rails.len()).collect();
    order.sort_by(|&a,&b|polyline_len(&rails[b]).total_cmp(&polyline_len(&rails[a])).then(a.cmp(&b)));
    let mut grid=crate::grid::XyGrid::new(4.0);
    let mut kept:Vec<(Vec3,Vec3)>=Vec::new(); let mut result=Vec::new();
    for i in order {
        let before=result.len(); let mut run:Vec<Vec3>=Vec::new();
        for w in rails[i].windows(2) {
            let (a,b)=(w[0],w[1]); let ab=b-a; let len2=ab.length_squared();
            if len2<1e-12 {continue;}
            let mut nearby=rustc_hash::FxHashSet::default();
            let lo=a.min(b)-Vec3::splat(EPS);let hi=a.max(b)+Vec3::splat(EPS);
            for x in grid.cell_of(lo.x)..=grid.cell_of(hi.x) {
                for y in grid.cell_of(lo.y)..=grid.cell_of(hi.y) {
                    nearby.extend(grid.cell(x,y).iter().copied());
                }
            }
            let mut covered=Vec::new(); let mut covering:Vec<(Vec3,Vec3)>=Vec::new();
            for id in nearby {
                let (c,d)=kept[id as usize]; let cd=d-c;
                if ab.normalize_or_zero().dot(cd.normalize_or_zero()).abs()<0.99 {continue;}
                let (u,v)=((c-a).dot(ab)/len2,(d-a).dot(ab)/len2);
                if c.distance(a+ab*u)>EPS || d.distance(a+ab*v)>EPS {continue;}
                let (lo,hi)=(u.min(v).max(0.),u.max(v).min(1.));
                if hi>lo+1e-6 {covered.push((lo,hi)); covering.push((c,d));}
            }
            let onto=|p:Vec3|covering.iter().map(|&(c,d)|{let cd=d-c;let t=((p-c).dot(cd)/cd.length_squared().max(1e-12)).clamp(0.,1.);c+cd*t})
                .min_by(|x,y|x.distance(p).total_cmp(&y.distance(p))).filter(|q|q.distance(p)<=EPS).unwrap_or(p);
            covered.sort_unstable_by(|a,b|a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
            let mut cursor=0.; let mut spans=Vec::new();
            for (lo,hi) in covered {
                if lo>cursor+1e-6 {spans.push((cursor,lo));}
                cursor=cursor.max(hi);
            }
            if cursor<1.-1e-6 {spans.push((cursor,1.));}
            for (lo,hi) in spans {
                let start=if lo>1e-6 {onto(a+ab*lo)} else {a+ab*lo};
                let end=if hi<1.-1e-6 {onto(a+ab*hi)} else {a+ab*hi};
                if !run.is_empty() && run.last().unwrap().distance(start)>1e-5 {result.push(std::mem::take(&mut run));}
                if run.is_empty(){run.push(start);}
                run.push(end);
            }
            if run.last().is_some_and(|p|p.distance(b)>1e-5) {result.push(std::mem::take(&mut run));}
        }
        if !run.is_empty(){result.push(run);}
        for r in &result[before..] {for w in r.windows(2) {
            let id=kept.len() as u32; kept.push((w[0],w[1]));
            grid.insert(id,w[0].min(w[1])-Vec3::splat(EPS),w[0].max(w[1])+Vec3::splat(EPS));
        }}
    }
    *rails=result;
}

/// Straight rails get straight grind lines (2026-10-20 test: "straight rails
/// get wavy grind rails"): each line keeps only the points that leave the
/// straight path between their neighbours by more than `tolerance`
/// (Douglas-Peucker), so the small kinks the correction steps leave go and
/// real bends stay.
pub fn straighten(rails: &mut [Vec<Vec3>], tolerance: f32) {
    fn keep(points: &[Vec3], tolerance: f32, out: &mut Vec<Vec3>) {
        let (a, b) = (points[0], points[points.len() - 1]);
        let ab = b - a;
        let len2 = ab.length_squared();
        let (mut worst, mut at) = (0.0f32, 0);
        for (i, &p) in points.iter().enumerate().take(points.len() - 1).skip(1) {
            let s = if len2 > 1e-12 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let d = p.distance(a + ab * s);
            if d > worst {
                (worst, at) = (d, i);
            }
        }
        if worst > tolerance {
            keep(&points[..=at], tolerance, out);
            keep(&points[at..], tolerance, out);
        } else {
            if out.last() != Some(&a) {
                out.push(a);
            }
            out.push(b);
        }
    }
    for r in rails.iter_mut() {
        if r.len() > 2 {
            let mut out = Vec::with_capacity(r.len());
            keep(r, tolerance, &mut out);
            out.dedup();
            *r = out;
        }
    }
}

fn polyline_len(points: &[Vec3]) -> f32 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

struct Grid {
    xy: crate::grid::XyGrid,
    /// Each triangle's height span (lowest, highest vertex z).
    span: Vec<(f32, f32)>,
}

fn cell_of(v: f32) -> i32 {
    (v / CELL).floor() as i32
}

/// Inches: how close another triangle's edge must lie to count as the same edge.
const WELD: f32 = 0.5;

/// Faces of other triangles whose edge lies on `a`-`b` (within `WELD`, the
/// two overlapping by half the shorter length), with each one's far vertex.
fn weld_partners(tris: &[[Vec3; 3]], grid: &Grid, a: Vec3, b: Vec3, own: u32) -> Vec<(Vec3, u32)> {
    let along = b - a;
    let len2 = along.length_squared();
    if len2 < 1e-6 {
        return Vec::new();
    }
    let off_line = |p: Vec3| {
        let t = (p - a).dot(along) / len2;
        ((p - (a + along * t)).length(), t)
    };
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    // Every cell the edge's box (widened by WELD) touches: a partner across a
    // cell border was missed when only a, b and the midpoint were looked up.
    let (lo, hi) = (a.min(b) - Vec3::splat(WELD), a.max(b) + Vec3::splat(WELD));
    let cells: Vec<(i32, i32)> = (grid.xy.cell_of(lo.x)..=grid.xy.cell_of(hi.x))
        .flat_map(|x| (grid.xy.cell_of(lo.y)..=grid.xy.cell_of(hi.y)).map(move |y| (x, y))).collect();
    for (x, y) in cells {
        for &t in grid.xy.cell(x, y) {
            if t == own || !seen.insert(t) {
                continue;
            }
            let tri = tris[t as usize];
            for i in 0..3 {
                let (p0, p1, far) = (tri[i], tri[(i + 1) % 3], tri[(i + 2) % 3]);
                let ((d0, t0), (d1, t1)) = (off_line(p0), off_line(p1));
                if d0 > WELD || d1 > WELD {
                    continue;
                }
                let (lo, hi) = (t0.min(t1).max(0.), t0.max(t1).min(1.));
                let shorter = (p1 - p0).length().min(len2.sqrt());
                if (hi - lo) * len2.sqrt() >= 0.5 * shorter && shorter > 0. {
                    out.push((far, t));
                    break;
                }
            }
        }
    }
    out
}

impl Grid {
    fn build(tris: &[[Vec3; 3]]) -> Self {
        let span = tris
            .iter()
            .map(|t| (t[0].z.min(t[1].z).min(t[2].z), t[0].z.max(t[1].z).max(t[2].z)))
            .collect();
        Self { xy: crate::grid::XyGrid::triangles(CELL, tris), span }
    }
}

struct Probe<'a> {
    tris: &'a [[Vec3; 3]],
    grid: &'a Grid,
    stamp: Vec<u32>,
    round: u32,
    scratch: Vec<u32>,
}

impl Probe<'_> {
    /// Past `p` along `out` the ground falls away and nothing rises. The
    /// faces `across` the edge are tried first: on open ground one of them
    /// takes the drop probe, and the grid would find it too.
    /// SkateV: whether a face stands right beneath the edge point `p`: a
    /// level ray from just outside and below it back under the edge hits one.
    fn supported(&mut self, p: Vec3, out: Vec3) -> bool {
        let flat = Vec3::new(out.x, out.y, 0.).normalize_or_zero();
        if flat == Vec3::ZERO {
            return true;
        }
        SUPPORT_DOWN.iter().any(|&down| self.hit(p + flat * PROBE_OUT - Vec3::Z * down, -flat, PROBE_OUT + SUPPORT_IN))
    }

    fn is_lip(&mut self, p: Vec3, out: Vec3, across: &[u32]) -> bool {
        let down_from = p + out * PROBE_OUT + Vec3::Z;
        let tris = self.tris;
        if across.iter().any(|&i| ray_triangle(down_from, -Vec3::Z, 1. + MIN_DROP, &tris[i as usize]))
            || self.hit(down_from, -Vec3::Z, 1. + MIN_DROP)
        {
            return false;
        }
        let across_from = p - out + Vec3::Z * WALL_PROBE_UP;
        !self.hit(across_from, out, 2. + PROBE_OUT)
    }

    /// SkateV: the triangles any buried-lip ray of lip `a`..`b` could hit:
    /// those whose box meets the prism the rays sweep (the lip padded by the
    /// truck half width, from just above it to the clearance). Testing the
    /// rays against this short list gives the same answers as probing the
    /// grid's full-height columns per sample (2-3 s per area:
    /// collision fell behind the skater).
    fn burial_candidates(&mut self, a: Vec3, b: Vec3) -> Vec<u32> {
        // Inch coordinates run to ~1e5 (float steps ~0.01): pad generously.
        let pad = Vec3::new(BURIED_SIDE, BURIED_SIDE, 0.) + Vec3::splat(0.25);
        let lo = a.min(b) - pad + Vec3::Z * 0.5;
        let hi = a.max(b) + pad + Vec3::Z * (0.5 + BURIED_CLEARANCE);
        self.round = self.round.wrapping_add(1);
        if self.round == 0 {
            self.stamp.fill(0);
            self.round = 1;
        }
        let mut out = Vec::new();
        for x in cell_of(lo.x)..=cell_of(hi.x) {
            for y in cell_of(lo.y)..=cell_of(hi.y) {
                for &index in self.grid.xy.cell(x, y) {
                    let (zl, zh) = self.grid.span[index as usize];
                    if zh < lo.z || zl > hi.z {
                        continue;
                    }
                    let seen = &mut self.stamp[index as usize];
                    if *seen == self.round {
                        continue;
                    }
                    *seen = self.round;
                    let t = &self.tris[index as usize];
                    let (tlo, thi) = (t[0].min(t[1]).min(t[2]), t[0].max(t[1]).max(t[2]));
                    if tlo.cmple(hi).all() && thi.cmpge(lo).all() {
                        out.push(index);
                    }
                }
            }
        }
        out
    }

    /// SkateV: the stretches of lip `a`..`b` that are not buried, with each
    /// cut refined to a fraction of an inch. A point is buried when solid
    /// geometry lies just above it or a truck's half width to either side
    /// (a stair handrail's sloped top edge continuing under the landing
    /// rail's top: bails at the bottom or half-way
    /// point of a handrail).
    fn unburied(&mut self, a: Vec3, b: Vec3) -> Vec<(Vec3, Vec3)> {
        let candidates = self.burial_candidates(a, b);
        if candidates.is_empty() {
            return vec![(a, b)];
        }
        let tris = self.tris;
        let steps = ((b - a).length() / BURIED_STEP).ceil().max(1.) as usize;
        let side = Vec3::new(a.y - b.y, b.x - a.x, 0.).normalize_or_zero() * BURIED_SIDE;
        let offsets = [Vec3::ZERO, side, -side];
        // Per candidate and ray line (the lip, and the lip moved a truck's
        // half width to either side): the stretch of lip fractions over the
        // triangle's plan, padded a quarter inch. A vertical ray outside it
        // cannot meet the triangle; one inside is tested as before. Faces
        // the vertical rays cannot hit (Möller–Trumbore's determinant, as
        // `ray_triangle` computes it) are dropped.
        let d = b - a;
        let pad = 0.25 / Vec3::new(d.x, d.y, 0.).length().max(1e-3);
        let mut spans: Vec<(u32, [(f32, f32); 3])> = Vec::with_capacity(candidates.len());
        for &i in &candidates {
            let t = &tris[i as usize];
            let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
            if e1.dot(Vec3::Z.cross(e2)).abs() < 1e-8 {
                continue;
            }
            let lines = offsets.map(|off| plan_span(a + off, d, t, pad));
            if lines.iter().any(|(t0, t1)| t0 <= t1) {
                spans.push((i, lines));
            }
        }
        if spans.is_empty() {
            return vec![(a, b)];
        }
        let buried = |p: Vec3, f: f32| {
            offsets.iter().enumerate().any(|(k, &off)| {
                let origin = p + off + Vec3::Z * 0.5;
                // Faces wholly below or above the ray are skipped: nearly
                // parallel to it (a wall face ending at the lip) Möller–Trumbore
                // can report them hit.
                spans.iter().any(|(i, lines)| {
                    let (t0, t1) = lines[k];
                    let t = &tris[*i as usize];
                    f >= t0
                        && f <= t1
                        && t[0].z.max(t[1].z).max(t[2].z) >= origin.z
                        && t[0].z.min(t[1].z).min(t[2].z) <= origin.z + BURIED_CLEARANCE
                        && ray_triangle(origin, Vec3::Z, BURIED_CLEARANCE, t)
                })
            })
        };
        let at = |i: usize| i as f32 / steps as f32;
        let clear: Vec<bool> = (0..=steps).map(|i| !buried(a.lerp(b, at(i)), at(i))).collect();
        if clear.iter().all(|c| *c) {
            return vec![(a, b)];
        }
        // The last clear fraction between a clear sample and a buried one.
        let edge = |mut inside: f32, mut outside: f32| {
            for _ in 0..8 {
                let mid = (inside + outside) * 0.5;
                if buried(a.lerp(b, mid), mid) {
                    outside = mid;
                } else {
                    inside = mid;
                }
            }
            inside
        };
        let mut spans = Vec::new();
        let mut i = 0;
        while i <= steps {
            if !clear[i] {
                i += 1;
                continue;
            }
            let first = i;
            while i < steps && clear[i + 1] {
                i += 1;
            }
            let t0 = if first == 0 { 0. } else { edge(at(first), at(first - 1)) };
            let t1 = if i == steps { 1. } else { edge(at(i), at(i + 1)) };
            if (t1 - t0) * (b - a).length() >= 1. {
                spans.push((a.lerp(b, t0), a.lerp(b, t1)));
            }
            i += 1;
        }
        spans
    }

    /// Whether the segment from `origin` along unit `dir` for `len` crosses
    /// any triangle, either side facing.
    fn hit(&mut self, origin: Vec3, dir: Vec3, len: f32) -> bool {
        let end = origin + dir * len;
        let (min, max) = (origin.min(end), origin.max(end));
        let (zlo, zhi) = (min.z - HIT_Z_MARGIN, max.z + HIT_Z_MARGIN);
        self.round = self.round.wrapping_add(1);
        if self.round == 0 {
            self.stamp.fill(0);
            self.round = 1;
        }
        self.scratch.clear();
        for x in cell_of(min.x)..=cell_of(max.x) {
            for y in cell_of(min.y)..=cell_of(max.y) {
                for &index in self.grid.xy.cell(x, y) {
                    let (lo, hi) = self.grid.span[index as usize];
                    if hi < zlo || lo > zhi {
                        continue;
                    }
                    let seen = &mut self.stamp[index as usize];
                    if *seen != self.round {
                        *seen = self.round;
                        self.scratch.push(index);
                    }
                }
            }
        }
        self.scratch
            .iter()
            .any(|&index| ray_triangle(origin, dir, len, &self.tris[index as usize]))
    }
}

/// The fractions `t` in 0..=1 where point `from + along * t` lies over
/// triangle `tri` in plan (XY), widened by `pad`; empty (t0 > t1) if none.
fn plan_span(from: Vec3, along: Vec3, tri: &[Vec3; 3], pad: f32) -> (f32, f32) {
    let (mut t0, mut t1) = (0f32, 1f32);
    // Counterclockwise in plan: inside is left of every edge.
    let ccw = (tri[1].x - tri[0].x) * (tri[2].y - tri[0].y) - (tri[1].y - tri[0].y) * (tri[2].x - tri[0].x) > 0.;
    for i in 0..3 {
        let (p, q) = if ccw { (tri[i], tri[(i + 1) % 3]) } else { (tri[(i + 1) % 3], tri[i]) };
        let (ex, ey) = (q.x - p.x, q.y - p.y);
        let len = (ex * ex + ey * ey).sqrt();
        if len < 1e-9 {
            continue;
        }
        // Signed distance left of the edge, linear in t: s0 + ds * t >= -pad_dist.
        let s0 = (ex * (from.y - p.y) - ey * (from.x - p.x)) / len;
        let ds = (ex * along.y - ey * along.x) / len;
        let slack = s0 + pad * (along.x * along.x + along.y * along.y).sqrt() + 0.25;
        if ds.abs() < 1e-12 {
            if slack < 0. {
                return (1., 0.);
            }
        } else if ds > 0. {
            t0 = t0.max(-slack / ds);
        } else {
            t1 = t1.min(-slack / ds);
        }
    }
    (t0 - pad, t1 + pad)
}

/// Möller–Trumbore, both faces, hits within `(0, len]`.
fn ray_triangle(origin: Vec3, dir: Vec3, len: f32, tri: &[Vec3; 3]) -> bool {
    let e1 = tri[1] - tri[0];
    let e2 = tri[2] - tri[0];
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-8 {
        return false;
    }
    let inv = 1. / det;
    let s = origin - tri[0];
    let u = s.dot(p) * inv;
    if !(0. ..=1.).contains(&u) {
        return false;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0. || u + v > 1. {
        return false;
    }
    let t = e2.dot(q) * inv;
    t > 1e-4 && t <= len
}

/// Lips on one line, merged into maximal runs where they overlap or touch.
fn merge_collinear(lips: Vec<(Vec3, Vec3)>) -> Vec<(Vec3, Vec3)> {
    let mut lines: HashMap<([i32; 3], [i32; 3]), (Vec3, Vec3, Vec<(f32, f32)>)> = HashMap::default();
    for (mut a, mut b) in lips {
        let mut d = (b - a).normalize_or_zero();
        if d == Vec3::ZERO {
            continue;
        }
        let flip =
            d.x < -1e-4 || (d.x.abs() <= 1e-4 && (d.y < -1e-4 || (d.y.abs() <= 1e-4 && d.z < 0.)));
        if flip {
            d = -d;
            std::mem::swap(&mut a, &mut b);
        }
        let o = a - d * a.dot(d);
        let k = (
            (d * 64.).to_array().map(|x| x.round() as i32),
            (o * 2.).to_array().map(|x| x.round() as i32),
        );
        let line = lines.entry(k).or_insert((d, o, Vec::new()));
        line.2.push((a.dot(line.0), b.dot(line.0)));
    }
    let mut runs = Vec::new();
    let mut lines: Vec<_> = lines.into_iter().collect();
    lines.sort_unstable_by_key(|(k, _)| *k);
    for (_, (d, o, mut spans)) in lines {
        spans.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut current = spans[0];
        for &(t0, t1) in &spans[1..] {
            if t0 <= current.1 + 1.5 {
                current.1 = current.1.max(t1);
            } else {
                runs.push((o + d * current.0, o + d * current.1));
                current = (t0, t1);
            }
        }
        runs.push((o + d * current.0, o + d * current.1));
    }
    runs
}

/// Runs joined end to end into polylines through joints where exactly two
/// runs meet at a gentle turn.
fn chain(runs: Vec<(Vec3, Vec3)>) -> Vec<Vec<Vec3>> {
    let node = |v: Vec3| v.to_array().map(|x| x.round() as i32);
    let mut at: HashMap<[i32; 3], Vec<(usize, bool)>> = HashMap::default();
    for (i, (a, b)) in runs.iter().enumerate() {
        at.entry(node(*a)).or_default().push((i, false));
        at.entry(node(*b)).or_default().push((i, true));
    }
    let mut used = vec![false; runs.len()];
    let far = |i: usize, from_end: bool| if from_end { runs[i].0 } else { runs[i].1 };
    let near = |i: usize, from_end: bool| if from_end { runs[i].1 } else { runs[i].0 };
    // The run continuing a polyline at `joint`, arriving along `heading`.
    let next = |joint: Vec3, heading: Vec3, used: &[bool]| -> Option<(usize, bool)> {
        let there = at.get(&node(joint))?;
        if there.len() != 2 {
            return None;
        }
        let &(i, at_end) = there.iter().find(|(i, _)| !used[*i])?;
        let leave = (far(i, at_end) - near(i, at_end)).normalize_or_zero();
        (heading.dot(leave) >= MIN_TURN_COS).then_some((i, at_end))
    };
    let mut rails = Vec::new();
    for start in 0..runs.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let (a, b) = runs[start];
        let mut points = std::collections::VecDeque::from([a, b]);
        // Forward from b, then backward from a.
        let mut tip = b;
        let mut heading = (b - a).normalize_or_zero();
        while let Some((i, at_end)) = next(tip, heading, &used) {
            used[i] = true;
            let to = far(i, at_end);
            heading = (to - tip).normalize_or_zero();
            tip = to;
            points.push_back(to);
        }
        let mut tip = a;
        let mut heading = (a - b).normalize_or_zero();
        while let Some((i, at_end)) = next(tip, heading, &used) {
            used[i] = true;
            let to = far(i, at_end);
            heading = (to - tip).normalize_or_zero();
            tip = to;
            points.push_front(to);
        }
        rails.push(points.into_iter().collect());
    }
    rails
}

#[cfg(test)]
mod tests {

    #[test]
    fn overlapping_pieces_of_a_curved_ledge_join_and_parallel_lines_do_not() {
        // 2026-10-26 mark 4: each piece starts 4 cm behind the last one's end,
        // turning about 10 degrees.
        let v = |x: f32, y: f32| Vec3::new(x, y, 3.67);
        let mut rails = vec![
            vec![v(-1362.64, -1427.57), v(-1362.07, -1427.14)],
            vec![v(-1362.11, -1427.16), v(-1361.31, -1426.33)],
            vec![v(-1361.35, -1426.41), v(-1360.62, -1425.09)],
        ];
        assert_eq!(join_ends(&mut rails), 2);
        assert_eq!(rails.len(), 1);
        let r = &rails[0];
        assert!(r.windows(2).all(|w| (w[1] - w[0]).length() > 0.05), "no backward kink: {r:?}");
        // A parallel line 5 cm to the side overlapping by 10 cm stays apart.
        let mut rails = vec![vec![v(0., 0.), v(1., 0.)], vec![v(0.9, 0.05), v(2., 0.05)]];
        assert_eq!(join_ends(&mut rails), 0);
    }

    #[test]
    fn track_edges_join_across_a_tile_seam_offset_sideways() {
        let v = |x: f32, y: f32| Vec3::new(x, y, 18.211);
        // Mark 12: the same edge 12 mm apart along the border x = -640.
        let mut rails = vec![vec![v(-617.908, -128.0), v(-640.0, -115.001)], vec![v(-640.0, -114.990), v(-704.0, -86.114)]];
        assert_eq!(join_ends(&mut rails), 1);
        assert_eq!(rails.len(), 1);
    }

    #[test]
    fn a_handrail_bend_is_bridged_but_side_by_side_ends_are_not() {
        let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        // Mark 9: landing piece at 10 degrees, then 1.1 m on the next flight at 33.
        let mut rails = vec![
            vec![v(-807.84, -103.79, 34.19), v(-806.42, -103.06, 34.47)],
            vec![v(-805.51, -102.6, 35.13), v(-804.01, -101.78, 36.26)],
        ];
        assert_eq!(join_ends_within(&mut rails, BEND_GAP), 1);
        assert_eq!(rails.len(), 1);
        // A straight railing with a 1 m opening is no bend.
        let mut rails = vec![vec![v(0., 0., 1.), v(3., 0., 1.)], vec![v(4., 0., 1.), v(7., 0., 1.)]];
        assert_eq!(join_ends_within(&mut rails, BEND_GAP), 0);
        // Two parallel rails 1 m apart are no bend.
        let mut rails = vec![vec![v(0., 0., 1.), v(3., 0., 1.)], vec![v(3., 1., 1.), v(6., 1., 1.)]];
        assert_eq!(join_ends_within(&mut rails, BEND_GAP), 0);
    }
    use super::*;

    /// Quads along x from 0 to `len`, between two profile points (y, z).
    fn strip(len: f32, steps: usize, p: (f32, f32), q: (f32, f32)) -> Vec<[Vec3; 3]> {
        let mut out = Vec::new();
        for i in 0..steps {
            let (x0, x1) = (len * i as f32 / steps as f32, len * (i + 1) as f32 / steps as f32);
            let v = |x: f32, (y, z): (f32, f32)| Vec3::new(x, y, z);
            let (a, b, c, d) = (v(x0, p), v(x1, p), v(x1, q), v(x0, q));
            out.push([a, b, c]);
            out.push([a, c, d]);
        }
        out
    }

    #[test]
    fn a_line_offset_15_mm_and_overlapping_joins_into_one_rail() {
        // Codex 2026-10-25: the tail kept its 15 mm offset and join_ends
        // refused the sideways connector, leaving two pieces.
        let v = |x: f32, y: f32| Vec3::new(x, y, 0.0);
        let mut rails = vec![vec![v(0., 0.), v(10., 0.)], vec![v(5., 0.015), v(12., 0.015)]];
        trim_overlaps(&mut rails);
        join_ends(&mut rails);
        dedupe_metres(&mut rails);
        join_ends(&mut rails);
        assert_eq!(rails.len(), 1, "one continuous rail: {rails:?}");
        let x_max = rails[0].iter().map(|p| p.x).fold(f32::MIN, f32::max);
        assert!((x_max - 12.0).abs() < 1e-3, "reaches x 12: {rails:?}");
    }

    #[test]
    fn a_duplicates_unique_tail_survives() {
        // A 10 m line and a 4 cm offset copy that runs 1 m further.
        let v = |x: f32, y: f32| Vec3::new(x, y, 0.0);
        let mut rails = vec![vec![v(0., 0.), v(10., 0.)], vec![v(0.5, 0.04), v(11., 0.04)]];
        dedupe_metres(&mut rails);
        let x_max = rails.iter().flatten().map(|p| p.x).fold(f32::MIN, f32::max);
        assert!(x_max > 10.8, "the 1 m extension stays: {rails:?}");
    }

    #[test]
    fn unwelded_bands_of_a_curved_wall_are_a_seam_but_an_unwelded_ledge_is_a_lip() {
        // A bowl transition in two bands meeting at y 1, z 0.3: the lower
        // band in 3 pieces, the upper in 2 (T-junctions); the upper band's
        // edge overlaps 3 mm over the lower one, 2 mm above it, as GTA's
        // park models (Vespucci bowl ring seams, 2026-10-25).
        let mut bowl = strip(6.0, 3, (0.0, 0.0), (1.0, 0.3));
        bowl.extend(strip(6.0, 2, (0.997, 0.301), (1.8, 0.9)));
        let (rails, _) = find_metres(&bowl);
        let seam = rails.iter().filter(|r| r.iter().all(|p| (p.y - 1.0).abs() < 0.05 && (p.z - 0.3).abs() < 0.05)).count();
        assert_eq!(seam, 0, "no rail on the band seam: {rails:?}");
        // A ledge: top at z 0.5, its front face a separate unwelded part.
        let mut ledge = strip(6.0, 3, (0.0, 0.5), (1.0, 0.5));
        ledge.extend(strip(6.0, 2, (0.0, 0.0), (0.0, 0.498)));
        ledge.extend(strip(6.0, 1, (-2.0, 0.0), (0.0, 0.0)));
        let (rails, _) = find_metres(&ledge);
        assert!(rails.iter().any(|r| r.iter().all(|p| p.y.abs() < 0.05 && (p.z - 0.5).abs() < 0.05)), "the ledge keeps its lip: {rails:?}");
    }
    #[test]
    fn a_wavy_line_along_a_straight_rail_becomes_straight_and_bends_stay() {
        let v = Vec3::new;
        let mut rails = vec![
            (0..21).map(|i| v(i as f32 * 0.25, if i % 2 == 0 { 0.012 } else { -0.012 }, 1.0 + if i % 3 == 0 { 0.01 } else { 0.0 })).collect::<Vec<_>>(),
            vec![v(0., 0., 1.), v(2., 0., 1.), v(2., 2., 1.)],
        ];
        straighten(&mut rails, 0.03);
        assert_eq!(rails[0].len(), 2, "{:?}", rails[0]);
        assert_eq!(rails[1].len(), 3);
    }

    #[test]
    fn partial_collinear_copies_keep_uncovered_tails_without_duplicate_intervals() {
        let v=Vec3::new;
        let mut rails=vec![vec![v(0.,0.,1.),v(2.,0.,1.)],vec![v(1.,0.,1.),v(3.,0.,1.)]];
        trim_overlaps(&mut rails);
        assert!((rails.iter().map(|r|polyline_len(r)).sum::<f32>()-3.).abs()<1e-5);
        join_ends(&mut rails);
        assert_eq!(rails.len(),1,"the retained tail must continue its source edge");
        assert!((polyline_len(&rails[0])-3.).abs()<1e-5);
    }

    #[test]
    fn overlap_cleanup_preserves_curves_crossings_parallel_edges_and_real_gaps() {
        let v=Vec3::new;
        let mut rails=vec![
            vec![v(0.,0.,1.),v(2.,0.,1.),v(3.,1.,1.)],
            vec![v(1.,0.,1.),v(2.,0.,1.),v(4.,2.,1.)],
        ];
        trim_overlaps(&mut rails);
        assert!((rails.iter().map(|r|polyline_len(r)).sum::<f32>()-(2.+8.0f32.sqrt())).abs()<1e-4,
            "shared curved spans are present exactly once: {rails:?}");
        let mut distinct=vec![
            vec![v(0.,0.,1.),v(2.,0.,1.)],
            vec![v(1.,0.1,1.),v(3.,0.1,1.)],
            vec![v(1.,-1.,1.),v(1.,1.,1.)],
            vec![v(3.3,0.1,1.),v(4.,0.1,1.)],
        ];
        let length=distinct.iter().map(|r|polyline_len(r)).sum::<f32>();
        trim_overlaps(&mut distinct);
        join_ends(&mut distinct);
        assert_eq!(distinct.len(),4,"nearby edges, crossings and open gaps remain distinct");
        assert!((distinct.iter().map(|r|polyline_len(r)).sum::<f32>()-length).abs()<1e-5);
    }

    #[test]
    fn overlap_cleanup_keeps_disjoint_source_support_without_bridging_a_void() {
        let v=Vec3::new;
        let mut rails=vec![
            vec![v(0.,0.,1.),v(1.,0.,1.)],vec![v(2.,0.,1.),v(3.,0.,1.)],
            vec![v(0.5,0.,1.),v(2.5,0.,1.)],
        ];
        trim_overlaps(&mut rails);join_ends(&mut rails);
        assert_eq!(rails.len(),1);
        assert!((polyline_len(&rails[0])-3.).abs()<1e-5);
        let mut gap=vec![vec![v(0.,0.,1.),v(1.,0.,1.)],vec![v(1.3,0.,1.),v(2.,0.,1.)]];
        trim_overlaps(&mut gap);join_ends(&mut gap);
        assert_eq!(gap.len(),2,"no source covers this gap");
    }

    #[test]
    fn aligned_rails_do_not_gain_a_short_sideways_or_backward_connector() {
        let v=Vec3::new;
        for (x,y) in [(1.,0.1),(0.95,0.05)] {
            let mut rails=vec![vec![v(0.,0.,1.),v(1.,0.,1.)],vec![v(x,y,1.),v(2.,y,1.)]];
            assert_eq!(join_ends(&mut rails),0,"a connector must follow both rail tangents");
        }
        let mut rails=vec![vec![v(0.,0.,1.),v(1.,0.,1.)],vec![v(1.05,0.,1.),v(2.,0.,1.)]];
        assert_eq!(join_ends(&mut rails),1,"a small in-line seam remains connected");
    }

    #[test]
    fn pieces_of_one_rail_are_joined_and_corners_are_not() {
        let v = Vec3::new;
        // One straight rail in three pieces with 5 cm and 10 cm gaps, one reversed.
        let mut rails = vec![
            vec![v(0., 0., 1.), v(2., 0., 1.)],
            vec![v(4., 0., 1.), v(2.05, 0., 1.)],
            vec![v(4.1, 0., 1.), v(6., 0., 1.)],
            // A rail meeting at a right angle stays separate.
            vec![v(6.05, 0., 1.), v(6.05, 3., 1.)],
        ];
        assert_eq!(join_ends(&mut rails), 2);
        assert_eq!(rails.len(), 2);
        let long = rails.iter().find(|r| polyline_len(r) > 5.0).unwrap();
        assert!(long.first().unwrap().x.min(long.last().unwrap().x) < 0.01);
        assert!(long.first().unwrap().x.max(long.last().unwrap().x) > 5.99);
    }

    use super::*;

    /// Two counterclockwise (upward) triangles covering an axis-aligned quad.
    fn quad(min: Vec3, max: Vec3, z: f32, out: &mut Vec<[Vec3; 3]>) {
        let (a, b) = (Vec3::new(min.x, min.y, z), Vec3::new(max.x, min.y, z));
        let (c, d) = (Vec3::new(max.x, max.y, z), Vec3::new(min.x, max.y, z));
        out.push([a, b, c]);
        out.push([a, c, d]);
    }

    /// A vertical wall face along y = `y` from x0..x1, z0..z1.
    fn wall_y(y: f32, x0: f32, x1: f32, z0: f32, z1: f32, out: &mut Vec<[Vec3; 3]>) {
        let (a, b) = (Vec3::new(x0, y, z0), Vec3::new(x1, y, z0));
        let (c, d) = (Vec3::new(x1, y, z1), Vec3::new(x0, y, z1));
        out.push([a, b, c]);
        out.push([a, c, d]);
    }

    /// A 0.4 m high ledge, 3 m long, 0.5 m deep, standing on open ground.
    fn ledge() -> Vec<[Vec3; 3]> {
        let mut t = Vec::new();
        quad(
            Vec3::new(-10.0, -10.0, 0.0),
            Vec3::new(10.0, 0.0, 0.0),
            0.0,
            &mut t,
        );
        quad(
            Vec3::new(-10.0, 0.5, 0.0),
            Vec3::new(10.0, 10.0, 0.0),
            0.0,
            &mut t,
        );
        quad(
            Vec3::new(-1.5, 0.0, 0.0),
            Vec3::new(1.5, 0.5, 0.0),
            0.4,
            &mut t,
        );
        wall_y(0.0, -1.5, 1.5, 0.0, 0.4, &mut t);
        wall_y(0.5, -1.5, 1.5, 0.0, 0.4, &mut t);
        t
    }

    #[test]
    fn shallow_convex_crest_is_a_surface_seam_but_a_steep_lip_stays() {
        let mut tris = Vec::new();
        // Twenty-two degree convex fold: unlike the old concavity gate this
        // must reject the crest too, not only the bottom of a bowl.
        let v = |x, y, z| Vec3::new(x, y, z);
        for [a, b, c, d] in [
            [v(-3., -3., 0.4), v(3., -3., 0.4), v(3., 0., 1.), v(-3., 0., 1.)],
            [v(-3., 0., 1.), v(3., 0., 1.), v(3., 3., 0.4), v(-3., 3., 0.4)],
        ] { tris.extend([[a, b, c], [a, c, d]]); }
        let (found, _) = find_metres(&tris);
        assert!(!found.iter().flat_map(|r| r.windows(2)).any(|w| w.iter().all(|p| p.y.abs() < 0.01 && (p.z - 1.).abs() < 0.01)), "{found:?}");
        assert!(!find_metres(&ledge()).0.is_empty(), "vertical drop remains grindable");
    }

    #[test]
    fn diagnostics_explain_why_a_short_seat_is_not_a_rail() {
        // A 0.4 m long, 0.4 m high block: its lips are real but too short.
        // Far from the other tests' geometry: the diagnostics focus is
        // process-wide and tests run in parallel.
        let o = Vec3::new(5000.0, 5000.0, 0.0);
        let mut t = Vec::new();
        quad(o + Vec3::new(-10.0, -10.0, 0.0), o + Vec3::new(10.0, 10.0, 0.0), 0.0, &mut t);
        quad(o + Vec3::new(-0.2, 0.0, 0.0), o + Vec3::new(0.2, 0.5, 0.0), 0.4, &mut t);
        wall_y(o.y, o.x - 0.2, o.x + 0.2, 0.0, 0.4, &mut t);
        set_diagnostics(Some(o + Vec3::new(0.0, 0.25, 0.4)));
        let (rails, _) = find_metres(&t);
        set_diagnostics(None);
        let rejected = take_rejected();
        assert!(rails.iter().all(|r| polyline_len(r) >= MIN_RAIL * INCH - 1e-4));
        assert!(!rejected.is_empty(), "the seat's edges report a reason");
        assert!(rejected.iter().all(|r| r.mid.distance(o + Vec3::new(0.0, 0.25, 0.4)) <= DIAG_RADIUS + 1e-3));
        assert!(rejected.iter().any(|r| r.reason == "rail shorter than 24 inches"), "{rejected:?}");
    }

    #[test]
    fn facet_lines_a_few_centimetres_apart_are_one_rail() {
        // A placed skate rail (2026-10-19): three lines 7 cm apart, ends a few cm off.
        let line = |y: f32, x0: f32, x1: f32| vec![Vec3::new(x0, y, 40.0), Vec3::new(x1, y, 40.0)];
        let mut rails = vec![line(0.0, 0.0, 180.0), line(2.8, 2.6, 182.5), line(1.4, -1.5, 179.0)];
        assert_eq!(dedupe(&mut rails), 2);
        // Two lips a hand apart (10 cm) stay two rails.
        let mut rails = vec![line(0.0, 0.0, 180.0), line(4.0, 0.0, 180.0)];
        assert_eq!(dedupe(&mut rails), 0);
    }

    #[test]
    fn a_tube_rail_sits_on_its_crown() {
        // A faceted handrail: a longer lower flank edge and the two top edges
        // of its flat top facet, 0.4 inch higher, 0.6 inch either side.
        let line = |y: f32, z: f32, x0: f32, x1: f32| vec![Vec3::new(x0, y, z), Vec3::new(x1, y, z)];
        let mut rails = vec![line(-0.6, 40.4, 0.2, 99.8), line(0.6, 40.4, 0.2, 99.8), line(0.9, 40.0, 0., 100.)];
        assert_eq!(dedupe(&mut rails), 2);
        assert_eq!(rails.len(), 1);
        for p in &rails[0] {
            assert!((p.z - 40.4).abs() < 1e-4, "on top: {p}");
            assert!(p.y.abs() < 1e-4, "centred over the top band: {p}");
        }
        // Two equal lips under 9 cm apart are one rail in their middle
        // (Manatros 2026-10-24; before, the line stayed on one lip).
        let mut rails = vec![line(0., 30., 0., 100.), line(1.9, 30., 1., 99.)];
        assert_eq!(dedupe(&mut rails), 1);
        assert!(rails[0].iter().all(|p| (p.y - 0.95).abs() < 1e-4 && (p.z - 30.).abs() < 1e-4), "{rails:?}");
    }

    #[test]
    fn rails_on_the_same_lip_collapse_to_the_longest() {
        let r = skate_core::physics::board::RETAIL_WHEEL_RADIUS / INCH;
        let line = |y: f32, x0: f32, x1: f32| vec![Vec3::new(x0, y, 10.), Vec3::new(x1, y, 10.)];
        // A lip, a shorter copy just over a wheel radius off it, and a
        // parallel lip three wheel radii away (a separate edge).
        let mut rails = vec![line(1.1 * r, 5., 95.), line(0., 0., 100.), line(3. * r, 0., 100.)];
        assert_eq!(dedupe(&mut rails), 1);
        assert_eq!(rails, vec![line(0., 0., 100.), line(3. * r, 0., 100.)]);
        // A copy only partly on the lip stays.
        let mut rails = vec![line(0., 0., 100.), line(r / 3., 50., 150.)];
        assert_eq!(dedupe(&mut rails), 0);
    }

    #[test]
    fn ledge_lips_become_rails_in_metres() {
        let (rails, census) = find_metres(&ledge());
        assert!(census.rails >= 2, "{census:?}");
        let on_top = |r: &&Vec<Vec3>| r.iter().all(|p| (p.z - 0.4).abs() < 1e-3);
        let lips: Vec<_> = rails
            .iter()
            .filter(on_top)
            .filter(|r| polyline_len(r) > 2.9)
            .collect();
        assert!(lips.len() >= 2, "front and back ledge lips: {rails:?}");
        for y in [0.0, 0.5] {
            assert!(
                lips.iter()
                    .any(|r| r.iter().all(|p| (p.y - y).abs() < 1e-3)),
                "lip along y={y}: {lips:?}"
            );
        }
        // Where the ground meets the ledge wall nothing drops away.
        let seam = rails.iter().any(|r| {
            r.iter().all(|p| {
                p.z.abs() < 1e-3
                    && p.x.abs() < 1.6
                    && (p.y.abs() < 1e-3 || (p.y - 0.5).abs() < 1e-3)
            })
        });
        assert!(!seam, "ground/ledge seam must not be a rail: {rails:?}");
    }

    #[test]
    fn flat_ground_interior_edges_are_not_rails() {
        let mut t = Vec::new();
        for i in -3..3 {
            for j in -3..3 {
                let min = Vec3::new(i as f32 * 2.0, j as f32 * 2.0, 0.0);
                quad(min, min + Vec3::new(2.0, 2.0, 0.0), 0.0, &mut t);
            }
        }
        // Surround the plaza with ground so only interior seams are candidates.
        quad(
            Vec3::new(-50.0, -50.0, 0.0),
            Vec3::new(50.0, -6.0, 0.0),
            0.0,
            &mut t,
        );
        quad(
            Vec3::new(-50.0, 6.0, 0.0),
            Vec3::new(50.0, 50.0, 0.0),
            0.0,
            &mut t,
        );
        quad(
            Vec3::new(-50.0, -6.0, 0.0),
            Vec3::new(-6.0, 6.0, 0.0),
            0.0,
            &mut t,
        );
        quad(
            Vec3::new(6.0, -6.0, 0.0),
            Vec3::new(50.0, 6.0, 0.0),
            0.0,
            &mut t,
        );
        let (rails, census) = find_metres(&t);
        let interior = rails
            .iter()
            .any(|r| r.iter().all(|p| p.x.abs() < 5.9 && p.y.abs() < 5.9));
        assert!(!interior, "{census:?} {rails:?}");
    }

    #[test]
    fn edge_against_a_rising_wall_is_not_a_lip() {
        let mut t = Vec::new();
        quad(
            Vec3::new(-2.0, -2.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            0.0,
            &mut t,
        );
        wall_y(0.0, -2.0, 2.0, 0.0, 3.0, &mut t);
        let (rails, _) = find_metres(&t);
        assert!(
            !rails
                .iter()
                .any(|r| r.iter().all(|p| p.y.abs() < 1e-3 && p.z.abs() < 1e-3)),
            "floor/wall seam must not be grindable: {rails:?}"
        );
    }

    #[test]
    fn bank_facet_crease_is_not_a_lip() {
        // Floor, a 30 degree facet, then a steeper 57 degree facet: the valley
        // between the two facets rises on both sides and is never grindable.
        let p = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let (z1, z2) = (0.5 * 30f32.to_radians().tan(), 0.5 * 30f32.to_radians().tan() + 0.5 * 57f32.to_radians().tan());
        let mut t = Vec::new();
        quad(p(-3., -3., 0.), p(3., 0., 0.), 0., &mut t);
        for (y0, y1, za, zb) in [(0., 0.5, 0., z1), (0.5, 1.0, z1, z2)] {
            t.push([p(-3., y0, za), p(3., y0, za), p(3., y1, zb)]);
            t.push([p(-3., y0, za), p(3., y1, zb), p(-3., y1, zb)]);
        }
        let (rails, _) = find_metres(&t);
        assert!(
            !rails.iter().any(|r| r.iter().all(|v| (v.y - 0.5).abs() < 1e-3)),
            "facet crease must not be a rail: {rails:?}"
        );
    }

    /// A box (all six faces) from `min` to `max`, tilted so its top rises by
    /// `rise` from min.x to max.x.
    fn sloped_box(min: Vec3, max: Vec3, rise: f32, out: &mut Vec<[Vec3; 3]>) {
        let p = |x: f32, y: f32, z: f32| Vec3::new(x, y, z + rise * (x - min.x) / (max.x - min.x));
        let c = [
            p(min.x, min.y, min.z), p(max.x, min.y, min.z), p(max.x, max.y, min.z), p(min.x, max.y, min.z),
            p(min.x, min.y, max.z), p(max.x, min.y, max.z), p(max.x, max.y, max.z), p(min.x, max.y, max.z),
        ];
        for [a, b, cc, d] in [[4, 5, 6, 7], [3, 2, 1, 0], [0, 1, 5, 4], [2, 3, 7, 6], [1, 2, 6, 5], [3, 0, 4, 7]] {
            out.push([c[a], c[b], c[cc]]);
            out.push([c[a], c[cc], c[d]]);
        }
    }

    #[test]
    fn handrail_flight_is_cut_where_it_runs_under_the_landing_rail() {
        // A 1 m high landing rail from x 0..2 (top 1.0) and a flight rail
        // coming down from x -3 that runs on 0.4 m into the landing rail,
        // its top edge going 0.24 m under the landing rail's top.
        let mut t = Vec::new();
        quad(Vec3::new(-10.0, -10.0, -5.0), Vec3::new(10.0, 10.0, -5.0), -5.0, &mut t);
        sloped_box(Vec3::new(0.0, 0.0, 0.9), Vec3::new(2.0, 0.3, 1.0), 0.0, &mut t);
        // Top from 3.0 at x -3 down to 1.0 at x 0 (slope -2/3), on to x 0.4.
        sloped_box(Vec3::new(-3.0, 0.0, 2.9), Vec3::new(0.4, 0.3, 3.0), -2.0 * 3.4 / 3.0, &mut t);
        let (rails, _) = find_metres(&t);
        let flight: Vec<_> = rails
            .iter()
            .filter(|r| r.iter().any(|p| p.x < -2.5 && p.z > 2.5))
            .collect();
        assert!(!flight.is_empty(), "the flight's lip is a rail: {rails:?}");
        for r in &flight {
            let end = r.iter().map(|p| p.x).fold(f32::MIN, f32::max);
            assert!(end < 0.02, "flight rail must stop where it goes under the landing rail, ends at x {end}: {r:?}");
            assert!(end > -0.05, "flight rail keeps its open part, ends at x {end}");
        }
        // The landing rail itself is untouched.
        assert!(rails.iter().any(|r| r.iter().all(|p| (p.z - 1.0).abs() < 1e-3) && polyline_len(r) > 1.9), "{rails:?}");
    }

    #[test]
    fn non_walkable_faces_are_not_candidates() {
        let mut t = Vec::new();
        wall_y(0.0, -2.0, 2.0, 0.0, 1.0, &mut t);
        let (_, census) = find_metres(&t);
        assert_eq!(census.candidates, 0);
    }

    #[test]
    fn short_lips_are_dropped() {
        let mut t = Vec::new();
        quad(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.3, 0.3, 0.0),
            1.0,
            &mut t,
        );
        let (rails, _) = find_metres(&t);
        assert!(
            rails
                .iter()
                .all(|r| polyline_len(r) >= MIN_RAIL * INCH - 1e-4)
        );
        assert!(
            rails.is_empty(),
            "0.3 m edges are under the 24 in minimum: {rails:?}"
        );
    }

    #[test]
    fn open_boundary_on_flat_ground_is_not_a_rail() {
        let mut t = Vec::new();
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(5.0, 0.0, 0.0);
        let c = Vec3::new(5.0, 5.0, 0.0);
        let d = Vec3::new(0.0, 5.0, 0.0);
        t.push([a, b, c]);
        t.push([a, c, d]);
        let (rails, _) = find_metres(&t);
        assert_eq!(rails.len(), 0, "flat open ground boundary must not generate grind rails");
    }

    /// Two triangles of a level quad (metres).
    fn level(x0: f32, y0: f32, x1: f32, y1: f32, z: f32) -> [[Vec3; 3]; 2] {
        let v = |x: f32, y: f32| Vec3::new(x, y, z);
        [[v(x0, y0), v(x1, y0), v(x1, y1)], [v(x0, y0), v(x1, y1), v(x0, y1)]]
    }

    /// A vertical wall quad along x at `y`, from z0 to z1 (metres).
    fn upright(x0: f32, x1: f32, y: f32, z0: f32, z1: f32) -> [[Vec3; 3]; 2] {
        let v = |x: f32, z: f32| Vec3::new(x, y, z);
        [[v(x0, z0), v(x1, z0), v(x1, z1)], [v(x0, z0), v(x1, z1), v(x0, z1)]]
    }

    #[test]
    fn a_loose_panel_over_the_ground_has_no_rails() {
        // A wedge or overlay piece laid over the floor, its edges open: no
        // face stands under them (the Vespucci bowl seam, 2026-10-23 16:32).
        let mut t: Vec<[Vec3; 3]> = level(-5., -5., 5., 5., 0.).to_vec();
        t.extend(level(-1., -1., 1., 1., 0.6));
        let (rails, _) = find_metres(&t);
        assert!(rails.iter().all(|r| r.iter().all(|p| p.z < 0.3)), "{rails:?}");
    }

    #[test]
    fn a_cap_over_its_wall_keeps_both_lips() {
        // A ledge whose top is its own mesh and whose sides stand 3 cm in.
        let mut t: Vec<[Vec3; 3]> = level(-5., -5., 5., 5., 0.).to_vec();
        t.extend(level(-1.5, 0., 1.5, 0.5, 0.4));
        t.extend(upright(-1.5, 1.5, 0.03, 0., 0.4));
        t.extend(upright(-1.5, 1.5, 0.47, 0., 0.4));
        let (rails, _) = find_metres(&t);
        for y in [0.0, 0.5] {
            assert!(rails.iter().any(|r| polyline_len(r) > 2.5
                && r.iter().all(|p| (p.y - y).abs() < 1e-2 && (p.z - 0.4).abs() < 1e-2)),
                "lip along y={y}: {rails:?}");
        }
    }

    #[test]
    fn a_thin_seat_whose_sides_meet_it_at_t_junctions_keeps_its_lips() {
        // A 4 cm bench slab 0.45 m up on legs: its long sides are split into
        // two faces each, so no side shares the top's full edge.
        let mut t: Vec<[Vec3; 3]> = level(-5., -5., 5., 5., 0.).to_vec();
        t.extend(level(-1., 0., 1., 0.4, 0.45));
        for y in [0.0, 0.4] {
            t.extend(upright(-1., 0., y, 0.41, 0.45));
            t.extend(upright(0., 1., y, 0.41, 0.45));
        }
        let (rails, _) = find_metres(&t);
        for y in [0.0, 0.4] {
            assert!(rails.iter().any(|r| polyline_len(r) > 1.5
                && r.iter().all(|p| (p.y - y).abs() < 1e-2 && (p.z - 0.45).abs() < 1e-2)),
                "seat lip along y={y}: {rails:?}");
        }
    }

    #[test]
    fn stair_noses_are_no_rails_but_curbs_ledges_and_bars_stay() {
        let v = Vec3::new;
        let mut rails: Vec<Vec<Vec3>> = (0..6)
            .map(|i| vec![v(0., i as f32 * 0.3, 0.17 * i as f32), v(3., i as f32 * 0.3, 0.17 * i as f32)])
            .collect();
        // The flight's side ledge (sloped), a curb, two ledge edges at one
        // height and two scaffold bars stacked 0.5 m apart.
        rails.push(vec![v(-0.2, 0., 0.3), v(-0.2, 1.5, 1.15)]);
        rails.push(vec![v(10., 0., 0.15), v(14., 0., 0.15)]);
        rails.push(vec![v(20., 0., 0.4), v(23., 0., 0.4)]);
        rails.push(vec![v(20., 0.5, 0.4), v(23., 0.5, 0.4)]);
        rails.push(vec![v(30., 0., 1.0), v(33., 0., 1.0)]);
        rails.push(vec![v(30., 0., 1.5), v(33., 0., 1.5)]);
        let kept = off_flight_noses(rails);
        assert_eq!(kept.len(), 6, "{kept:?}");
        assert!(kept.iter().all(|r| r[0].x != 0.), "every nose dropped: {kept:?}");
    }

    #[test]
    fn ledges_up_and_down_again_are_no_flight_but_a_descending_flight_is() {
        let v = Vec3::new;
        // Codex review 2026-10-24: (y, z) = (0, 0), (0.3, 0.17), (0.6, 0).
        let zigzag = vec![
            vec![v(0., 0., 0.), v(3., 0., 0.)],
            vec![v(0., 0.3, 0.17), v(3., 0.3, 0.17)],
            vec![v(0., 0.6, 0.), v(3., 0.6, 0.)],
        ];
        assert_eq!(off_flight_noses(zigzag).len(), 3);
        // A flight drawn the other way round (rails pointing -x, going down in y).
        let flight: Vec<Vec<Vec3>> = (0..4)
            .map(|i| vec![v(3., -0.3 * i as f32, 0.17 * i as f32), v(0., -0.3 * i as f32, 0.17 * i as f32)])
            .collect();
        assert!(off_flight_noses(flight).is_empty());
        // Stacked bars with stair-like spacing but too steep (0.3 up per 0.2 across) are no flight.
        let steep: Vec<Vec<Vec3>> = (0..4).map(|i| vec![v(0., 0.2 * i as f32, 0.3 * i as f32), v(3., 0.2 * i as f32, 0.3 * i as f32)]).collect();
        assert_eq!(off_flight_noses(steep).len(), 4);
    }

    #[test]
    fn two_steps_are_a_curb_pair_not_a_flight() {
        let v = Vec3::new;
        let rails = vec![vec![v(0., 0., 0.15), v(3., 0., 0.15)], vec![v(0., 0.3, 0.3), v(3., 0.3, 0.3)]];
        assert_eq!(off_flight_noses(rails).len(), 2);
    }


    #[test]
    fn two_lips_of_one_pipe_merge_into_the_middle() {
        let v = Vec3::new;
        // A placed rail's two top facet edges, 5 cm apart at one height.
        let mut rails = vec![vec![v(0., 0., 0.5), v(3., 0., 0.5)], vec![v(0., 0.05, 0.5), v(2.9, 0.05, 0.5)]];
        dedupe_metres(&mut rails);
        assert_eq!(rails.len(), 1, "{rails:?}");
        assert!(rails[0].iter().all(|p| (p.y - 0.025).abs() < 2e-3 && (p.z - 0.5).abs() < 1e-3), "{rails:?}");
        // Two ledge edges 30 cm apart stay two rails where they are.
        let mut ledge = vec![vec![v(0., 0., 0.4), v(3., 0., 0.4)], vec![v(0., 0.3, 0.4), v(3., 0.3, 0.4)]];
        dedupe_metres(&mut ledge);
        assert_eq!(ledge.len(), 2);
        assert!(ledge.iter().any(|r| r.iter().all(|p| p.y.abs() < 1e-4)));
    }

}
