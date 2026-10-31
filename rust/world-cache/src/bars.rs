//! Rails from GTA's visible models (Manatros 2026-10-20: handrails "do not
//! have collision boxes and grinding isn't quite right"). GTA leaves many
//! handrails, railings and coping bars out of every collision copy (the
//! Portola Drive station's stair handrails exist only in its drawable
//! `bh1_rd2_portola_subwaybits`); their mesh is the only record of them.
//!
//! A rail is a thin tube or bar: a connected piece of the high-detail mesh
//! that, cut into short slices along its length, stays tube-thin. The rail
//! line runs along the top of those slices, through bends and landings.

type V = [f32; 3];

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn scale(a: V, s: f32) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn norm(a: V) -> V {
    let l = dot(a, a).sqrt().max(1e-9);
    scale(a, 1.0 / l)
}

/// Widest a tube's side triangle is across its long edge (metres).
const THIN: f32 = 0.12;
/// Thinnest a bar is across, either way (metres).
const MIN_ROUND: f32 = 0.012;
/// Widest angle a bar's cross-section leaves open, seen from its centre:
/// a 3-sided bar (GTA's low-poly handrails) leaves 120 deg, a one-sided
/// L-shaped trim about 143 deg.
const MAX_GAP: f32 = 2.3;
/// Fewest mesh edges around a tube's cross-section (a box rail has 4).
const MIN_SIDES: usize = 3;
/// Spacing of the points sampled along a mesh's edges (metres).
const EDGE_STEP: f32 = 0.04;
/// Slice length along a piece (metres).
const SLICE: f32 = 0.12;
/// Widest a tube-slice spreads from its centre (metres): handrails 2-5 cm,
/// coping pipes up to 8 cm.
const MAX_SPREAD: f32 = 0.09;
/// Shortest rail kept (metres).
const MIN_LENGTH: f32 = 0.6;
/// Steepest rail segment, rise over length (rails.rs MAX_SLOPE).
const MAX_SLOPE: f32 = 0.7;

/// Main axis of `pts` about `c` (power iteration on the covariance).
fn axis(pts: &[V], c: V) -> V {
    let mut m = [[0f32; 3]; 3];
    for p in pts {
        let d = sub(*p, c);
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += d[i] * d[j];
            }
        }
    }
    let mut v = [0.57, 0.58, 0.59];
    for _ in 0..60 {
        v = norm([dot(m[0], v), dot(m[1], v), dot(m[2], v)]);
    }
    v
}

/// The rails in one mesh: `positions` and its triangle `indices`, model space.
pub fn rails(positions: &[V], indices: &[u32]) -> Vec<Vec<V>> {
    // Weld by position (1 mm): a bar split at its UV seams is one piece.
    let mut weld: std::collections::BTreeMap<(i64, i64, i64), usize> = std::collections::BTreeMap::new();
    let id: Vec<usize> = positions
        .iter()
        .map(|p| {
            let k = ((p[0] * 1000.0).round() as i64, (p[1] * 1000.0).round() as i64, (p[2] * 1000.0).round() as i64);
            let n = weld.len();
            *weld.entry(k).or_insert(n)
        })
        .collect();
    let mut parent: Vec<usize> = (0..weld.len()).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for t in indices.chunks_exact(3) {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| id.get(i as usize).copied());
        let (Some(a), Some(b), Some(c)) = (a, b, c) else { continue };
        let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
        parent[ra] = rb;
        let (rb, rc) = (root(&mut parent, b), root(&mut parent, c));
        parent[rb] = rc;
    }
    // Each piece's tube surface as points, each with the mesh edge it lies
    // on: points every `EDGE_STEP` along every edge of a thin triangle (a
    // tube's or bar's side: long and narrow). GTA's tubes are low-poly (a
    // straight 3 m stretch has vertices only at its ends), so slices through
    // the middle need the edges' points; wide triangles (walls, steps, floors)
    // are no tube and give none.
    let mut pieces: std::collections::BTreeMap<usize, Vec<(V, u32)>> = std::collections::BTreeMap::new();
    let mut seen: std::collections::BTreeSet<(usize, usize)> = std::collections::BTreeSet::new();
    let mut edge_id = 0u32;
    for t in indices.chunks_exact(3) {
        let ids = [t[0], t[1], t[2]].map(|i| i as usize);
        if ids.iter().any(|&i| i >= positions.len()) {
            continue;
        }
        let tri = ids.map(|i| positions[i]);
        if !thin(&tri) {
            continue;
        }
        for k in 0..3 {
            let (i, j) = (ids[k], ids[(k + 1) % 3]);
            let (wi, wj) = (id[i], id[j]);
            if !seen.insert((wi.min(wj), wi.max(wj))) {
                continue;
            }
            let (p, q) = (positions[i], positions[j]);
            if !(p.iter().chain(q.iter()).all(|x| x.is_finite())) {
                continue;
            }
            let r = root(&mut parent, wi);
            let d = sub(q, p);
            let steps = ((dot(d, d).sqrt() / EDGE_STEP).ceil() as usize).clamp(1, 4000);
            let out = pieces.entry(r).or_default();
            edge_id += 1;
            for s in 0..=steps {
                let f = s as f32 / steps as f32;
                out.push(([p[0] + d[0] * f, p[1] + d[1] * f, p[2] + d[2] * f], edge_id));
            }
        }
    }
    // Ordered maps throughout: the output order must not depend on a hash
    // seed (std's HashMap reordered the rails each run, and order decides
    // which duplicate survives downstream; 2026-10-24).
    let mut out = Vec::new();
    for pts in pieces.values() {
        if pts.len() < 8 {
            continue;
        }
        out.extend(rails_of_piece(pts));
    }
    out
}

/// A tube's or bar's side: a triangle no wider, across its longest edge,
/// than a rail is round (`MAX_SPREAD` x 2 x pi / 3 sides at least).
fn thin(t: &[V; 3]) -> bool {
    let e = [sub(t[1], t[0]), sub(t[2], t[1]), sub(t[0], t[2])];
    let (k, long) = e.iter().enumerate().map(|(k, d)| (k, dot(*d, *d))).fold((0, 0.0f32), |m, x| if x.1 > m.1 { x } else { m });
    if long < 1e-8 {
        return false;
    }
    let other = sub(t[(k + 2) % 3], t[k]);
    let along = dot(other, e[k]) / long;
    let h = sub(other, scale(e[k], along));
    dot(h, h).sqrt() <= THIN
}

/// Points of one slice within this of each other are one cross-section (metres).
const GROUP: f32 = 0.05;
/// A chain continues to a cross-section at most this far away (metres).
const LINK: f32 = 0.35;

/// Rails along one connected piece. It is cut into slices along its main
/// axis; inside a slice the points fall into cross-sections (a double
/// handrail's top bar and lower bar, a post), each thin one is a candidate,
/// and candidates in following slices link into chains: one per bar. A post
/// crosses only a slice or two and stays too short.
fn rails_of_piece(samples: &[(V, u32)]) -> Vec<Vec<V>> {
    let pts: Vec<V> = samples.iter().map(|s| s.0).collect();
    let n = pts.len() as f32;
    let c = scale(pts.iter().fold([0.0; 3], |s, p| [s[0] + p[0], s[1] + p[1], s[2] + p[2]]), 1.0 / n);
    let a = axis(&pts, c);
    let t: Vec<f32> = pts.iter().map(|p| dot(sub(*p, c), a)).collect();
    let (lo, hi) = t.iter().fold((f32::MAX, f32::MIN), |(l, h), &x| (l.min(x), h.max(x)));
    if hi - lo < MIN_LENGTH {
        return Vec::new();
    }
    let slices = (((hi - lo) / SLICE).ceil() as usize).max(1);
    let mut bins: Vec<Vec<(V, u32)>> = vec![Vec::new(); slices];
    for (s, &ti) in samples.iter().zip(&t) {
        bins[(((ti - lo) / SLICE) as usize).min(slices - 1)].push(*s);
    }
    // Cross-sections per slice: (centre, top point).
    let sections: Vec<Vec<V>> = bins.iter().map(|b| cross_sections(b, a)).collect();
    // Chains: (points, last slice index).
    let mut chains: Vec<(Vec<V>, usize)> = Vec::new();
    for (k, secs) in sections.iter().enumerate() {
        let mut taken = vec![false; chains.len()];
        for s in secs {
            let best = chains
                .iter()
                .enumerate()
                .filter(|(i, (ch, last))| !taken[*i] && k - last <= 3 && {
                    let d = sub(*s, *ch.last().unwrap());
                    let flat = (d[0] * d[0] + d[1] * d[1]).sqrt();
                    dot(d, d).sqrt() <= LINK && d[2].abs() <= MAX_SLOPE * (flat + d[2].abs()).max(1e-6)
                })
                .min_by(|(_, (x, _)), (_, (y, _))| {
                    let dx = sub(*s, *x.last().unwrap());
                    let dy = sub(*s, *y.last().unwrap());
                    dot(dx, dx).total_cmp(&dot(dy, dy))
                })
                .map(|(i, _)| i);
            match best {
                Some(i) => {
                    taken[i] = true;
                    chains[i].0.push(*s);
                    chains[i].1 = k;
                }
                None => {
                    chains.push((vec![*s], k));
                    taken.push(true);
                }
            }
        }
    }
    chains
        .into_iter()
        .map(|(ch, _)| ch)
        .filter(|ch| ch.len() >= 2 && ch.windows(2).map(|w| dot(sub(w[1], w[0]), sub(w[1], w[0])).sqrt()).sum::<f32>() >= MIN_LENGTH)
        .map(simplify)
        .collect()
}

/// The thin cross-sections among one slice's points (`a`: the slicing axis):
/// each as its centre with the top's height.
fn cross_sections(samples: &[(V, u32)], a: V) -> Vec<V> {
    if samples.is_empty() {
        return Vec::new();
    }
    let b: Vec<V> = samples.iter().map(|s| s.0).collect();
    let mut group: Vec<usize> = (0..b.len()).collect();
    fn root(g: &mut [usize], mut i: usize) -> usize {
        while g[i] != i {
            g[i] = g[g[i]];
            i = g[i];
        }
        i
    }
    for i in 0..b.len() {
        for j in i + 1..b.len() {
            let d = sub(b[i], b[j]);
            if dot(d, d) <= GROUP * GROUP {
                let (ri, rj) = (root(&mut group, i), root(&mut group, j));
                group[ri] = rj;
            }
        }
    }
    let mut members: std::collections::BTreeMap<usize, Vec<V>> = std::collections::BTreeMap::new();
    let mut sides: std::collections::BTreeMap<usize, std::collections::BTreeSet<u32>> = std::collections::BTreeMap::new();
    for i in 0..b.len() {
        let r = root(&mut group, i);
        members.entry(r).or_default().push(b[i]);
        sides.entry(r).or_default().insert(samples[i].1);
    }
    members
        .iter()
        .filter(|(r, m)| m.len() >= 3 && sides[r].len() >= MIN_SIDES)
        .map(|(_, m)| m)
        .filter_map(|m| {
            // A rail joined to its posts or brackets: the slice through a
            // joint holds the post too. Its topmost bar is the part within
            // a rail's width of its highest point.
            let top = m.iter().copied().fold([0.0, 0.0, f32::MIN], |t, p| if p[2] > t[2] { p } else { t });
            let m: Vec<V> = m.iter().copied().filter(|p| dot(sub(*p, top), sub(*p, top)) <= (2.0 * MAX_SPREAD) * (2.0 * MAX_SPREAD)).collect();
            let m = &m[..];
            let centre = scale(m.iter().fold([0.0; 3], |s, p| [s[0] + p[0], s[1] + p[1], s[2] + p[2]]), 1.0 / m.len() as f32);
            let spread = m
                .iter()
                .map(|p| {
                    let d = sub(*p, centre);
                    let q = sub(d, scale(a, dot(d, a)));
                    dot(q, q).sqrt()
                })
                .fold(0.0f32, f32::max);
            (spread <= MAX_SPREAD && closed(m, centre, a)).then(|| [centre[0], centre[1], m.iter().map(|p| p[2]).fold(f32::MIN, f32::max)])
        })
        .collect()
}

/// A bar's cross-section goes all round its centre (a tube, a box bar):
/// seen from the centre, across the slicing axis `a`, its points leave no
/// gap wider than `MAX_GAP` and it is at least `MIN_ROUND` thick both ways.
/// A trim strip, a step's nosing or a wall's corner is one-sided.
fn closed(m: &[V], centre: V, a: V) -> bool {
    let helper = if a[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
    let u = norm([a[1] * helper[2] - a[2] * helper[1], a[2] * helper[0] - a[0] * helper[2], a[0] * helper[1] - a[1] * helper[0]]);
    let w = [a[1] * u[2] - a[2] * u[1], a[2] * u[0] - a[0] * u[2], a[0] * u[1] - a[1] * u[0]];
    let mut angles = Vec::with_capacity(m.len());
    let (mut ulo, mut uhi, mut wlo, mut whi) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for p in m {
        let d = sub(*p, centre);
        let (x, y) = (dot(d, u), dot(d, w));
        (ulo, uhi, wlo, whi) = (ulo.min(x), uhi.max(x), wlo.min(y), whi.max(y));
        if x * x + y * y > 1e-8 {
            angles.push(y.atan2(x));
        }
    }
    if uhi - ulo < MIN_ROUND || whi - wlo < MIN_ROUND || angles.len() < 4 {
        return false;
    }
    angles.sort_by(f32::total_cmp);
    let wrap = angles[0] + std::f32::consts::TAU - angles[angles.len() - 1];
    let gap = angles.windows(2).map(|w| w[1] - w[0]).fold(wrap, f32::max);
    gap <= MAX_GAP
}

/// Drops points that lie on the line between their neighbours (2 mm).
fn simplify(line: Vec<V>) -> Vec<V> {
    if line.len() < 3 {
        return line;
    }
    let mut out = vec![line[0]];
    for i in 1..line.len() - 1 {
        let (a, b, c) = (*out.last().unwrap(), line[i], line[i + 1]);
        let ac = sub(c, a);
        let l = dot(ac, ac);
        let s = if l > 1e-12 { (dot(sub(b, a), ac) / l).clamp(0.0, 1.0) } else { 0.0 };
        let q = sub(b, [a[0] + ac[0] * s, a[1] + ac[1] * s, a[2] + ac[2] * s]);
        if dot(q, q).sqrt() > 0.002 {
            out.push(b);
        }
    }
    out.push(*line.last().unwrap());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An 8-sided tube of radius `r` from `a` to `b`, `rings` rings.
    fn tube(a: V, b: V, r: f32, rings: usize, pos: &mut Vec<V>, idx: &mut Vec<u32>) {
        let d = norm(sub(b, a));
        let up = if d[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
        let s = norm([d[1] * up[2] - d[2] * up[1], d[2] * up[0] - d[0] * up[2], d[0] * up[1] - d[1] * up[0]]);
        let u = [d[1] * s[2] - d[2] * s[1], d[2] * s[0] - d[0] * s[2], d[0] * s[1] - d[1] * s[0]];
        let base = pos.len() as u32;
        for k in 0..rings {
            let t = k as f32 / (rings - 1) as f32;
            let c = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
            for j in 0..8 {
                let ang = j as f32 * std::f32::consts::TAU / 8.0;
                let (cs, sn) = (ang.cos() * r, ang.sin() * r);
                pos.push([c[0] + s[0] * cs + u[0] * sn, c[1] + s[1] * cs + u[1] * sn, c[2] + s[2] * cs + u[2] * sn]);
            }
        }
        for k in 0..rings as u32 - 1 {
            for j in 0..8u32 {
                let (p, q) = (base + k * 8 + j, base + k * 8 + (j + 1) % 8);
                idx.extend([p, q, p + 8, q, q + 8, p + 8]);
            }
        }
    }

    #[test]
    fn a_stair_handrail_with_a_landing_is_one_rail_on_its_top() {
        let (mut pos, mut idx) = (Vec::new(), Vec::new());
        // Down a flight (34 deg), along a landing, down again; one welded tube.
        tube([0.0, 0.0, 3.0], [3.0, 0.0, 1.0], 0.025, 30, &mut pos, &mut idx);
        tube([3.0, 0.0, 1.0], [4.5, 0.0, 1.0], 0.025, 15, &mut pos, &mut idx);
        tube([4.5, 0.0, 1.0], [7.5, 0.0, -1.0], 0.025, 30, &mut pos, &mut idx);
        // A post under it (vertical, separate piece).
        tube([1.5, 0.0, 1.0], [1.5, 0.0, 2.0], 0.02, 10, &mut pos, &mut idx);
        let rails = rails(&pos, &idx);
        assert!(!rails.is_empty(), "the handrail is a rail");
        let total: f32 = rails.iter().map(|r| r.windows(2).map(|w| dot(sub(w[1], w[0]), sub(w[1], w[0])).sqrt()).sum::<f32>()).sum();
        assert!(total > 7.0, "most of its length: {total}");
        for r in &rails {
            for p in r {
                // On the top of the tube: the centre line plus the radius (one slice tolerance).
                let centre = if p[0] < 3.0 { 3.0 - p[0] * 2.0 / 3.0 } else if p[0] < 4.5 { 1.0 } else { 1.0 - (p[0] - 4.5) * 2.0 / 3.0 };
                assert!((p[2] - (centre + 0.025)).abs() < 0.09, "{p:?}");
                assert!(p[1].abs() < 0.03, "{p:?}");
            }
        }
        // The post is no rail.
        assert!(rails.iter().all(|r| r.iter().all(|p| (p[0] - 1.5).abs() > 0.05 || p[2] > 1.9)));
    }

    #[test]
    fn a_double_handrail_joined_by_posts_gives_two_rails() {
        let (mut pos, mut idx) = (Vec::new(), Vec::new());
        tube([0.0, 0.0, 1.0], [6.0, 0.0, 1.0], 0.025, 40, &mut pos, &mut idx);
        tube([0.0, 0.0, 0.5], [6.0, 0.0, 0.5], 0.02, 40, &mut pos, &mut idx);
        // Posts joining them (separate pieces here; welded posts: the next test).
        for x in [0.5, 3.0, 5.5] {
            tube([x, 0.0, 0.5], [x, 0.0, 1.0], 0.02, 6, &mut pos, &mut idx);
        }
        let rails = rails(&pos, &idx);
        let long: Vec<_> = rails.iter().filter(|r| r.windows(2).map(|w| dot(sub(w[1], w[0]), sub(w[1], w[0])).sqrt()).sum::<f32>() > 4.0).collect();
        assert_eq!(long.len(), 2, "{rails:?}");
    }

    #[test]
    fn a_handrail_welded_to_its_posts_is_one_rail_over_them() {
        let (mut pos, mut idx) = (Vec::new(), Vec::new());
        tube([0.0, 0.0, 2.0], [6.0, 0.0, -2.0], 0.025, 60, &mut pos, &mut idx);
        for k in 1..6 {
            let x = k as f32;
            let z = 2.0 - x * 4.0 / 6.0;
            tube([x, 0.0, z - 1.0], [x, 0.0, z], 0.03, 8, &mut pos, &mut idx);
        }
        // Weld: one big piece (the posts' top rings touch the rail's).
        let n = pos.len() as u32;
        idx.extend([0, n - 1, 1]);
        let rails = rails(&pos, &idx);
        let long: Vec<_> = rails.iter().filter(|r| r.windows(2).map(|w| dot(sub(w[1], w[0]), sub(w[1], w[0])).sqrt()).sum::<f32>() > 5.0).collect();
        assert_eq!(long.len(), 1, "{rails:?}");
        for p in long[0] {
            assert!((p[2] - (2.0 - p[0] * 4.0 / 6.0 + 0.025)).abs() < 0.1, "{p:?}");
        }
    }

    #[test]
    fn a_one_sided_trim_is_no_rail() {
        // A strip folded at a step's nosing: two thin faces meeting at a corner.
        let (mut pos, mut idx) = (Vec::new(), Vec::new());
        for k in 0..20u32 {
            let x = k as f32 * 0.2;
            pos.extend([[x, 0.0, 0.0], [x, 0.04, 0.0], [x, 0.0, -0.04]]);
            if k > 0 {
                let (p, q) = ((k - 1) * 3, k * 3);
                idx.extend([p, q, p + 1, q, q + 1, p + 1, p, q, p + 2, q, q + 2, p + 2]);
            }
        }
        assert!(rails(&pos, &idx).is_empty());
    }

    #[test]
    fn a_wall_is_no_rail() {
        // A 3 m x 1 m slab: wide across, never tube-thin.
        let pos = vec![[0.0, 0.0, 0.0], [3.0, 0.0, 0.0], [3.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.2, 0.0], [3.0, 0.2, 0.0], [3.0, 0.2, 1.0], [0.0, 0.2, 1.0]];
        let idx = vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4];
        assert!(rails(&pos, &idx).is_empty());
    }
}
