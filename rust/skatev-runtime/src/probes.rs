//! Offline diagnostics over svwc caches (ignored tests; run with
//! `-- --ignored --nocapture` and the env vars each names).

#[cfg(test)]
mod tests {
    fn load() -> Vec<svwc::Tri> {
        svwc::Cache::open(std::path::Path::new(&std::env::var("SVWC").unwrap())).and_then(|mut c| c.read_all()).unwrap()
    }

    /// SVWC=<cache>: every 12 triangles as one box (primitive boxes come out
    /// that way): its extents and height span.
    #[test]
    #[ignore]
    fn boxes_of_a_primitive_cache() {
        let tris = load();
        for (i, b) in tris.chunks(12).enumerate() {
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for t in b { for p in t.v { for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); } } }
            println!("box {i:2}: x {:7.2}..{:7.2} y {:7.2}..{:7.2} z {:6.3}..{:6.3}  size {:.3} x {:.3} x {:.3}", lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], hi[0]-lo[0], hi[1]-lo[1], hi[2]-lo[2]);
        }
    }

    /// SVWC=<cache>, AT="x y" and DIR="dx dy": the surfaces' heights along a
    /// line across (perpendicular to DIR) through AT, every 2 cm, +-1.5 m.
    #[test]
    #[ignore]
    fn cross_section() {
        let tris = load();
        let at: Vec<f32> = std::env::var("AT").unwrap().split_whitespace().map(|s| s.parse().unwrap()).collect();
        let d: Vec<f32> = std::env::var("DIR").unwrap().split_whitespace().map(|s| s.parse().unwrap()).collect();
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
        let side = [-d[1] / l, d[0] / l];
        let mut off = -1.5f32;
        while off <= 1.5 {
            let (x, y) = (at[0] + side[0] * off, at[1] + side[1] * off);
            let mut zs: Vec<f32> = tris.iter().filter_map(|t| svwc::clean::floor_at(t, [x, y])).collect();
            zs.sort_by(f32::total_cmp);
            zs.dedup_by(|a, b| (*a - *b).abs() < 0.005);
            println!("{off:+.2}: {}", zs.iter().map(|z| format!("{z:.3}")).collect::<Vec<_>>().join(" "));
            off += 0.02;
        }
    }

    /// SVWC=<cache>: each box (12 triangles) as an oriented box: its three
    /// edge lengths, centre, long-axis direction and slope.
    #[test]
    #[ignore]
    fn oriented_boxes() {
        use bevy_math::Vec3;
        let tris = load();
        for (i, b) in tris.chunks(12).enumerate() {
            let mut v: Vec<Vec3> = Vec::new();
            for t in b { for p in t.v { let p = Vec3::from_array(p); if !v.iter().any(|q| q.distance(p) < 1e-4) { v.push(p); } } }
            if v.len() != 8 { println!("box {i}: {} corners", v.len()); continue; }
            let mut d: Vec<(f32, Vec3)> = v[1..].iter().map(|&q| (q.distance(v[0]), q - v[0])).collect();
            d.sort_by(|a, b| a.0.total_cmp(&b.0));
            let edges = [d[0], d[1], d[2]];
            let long = edges.iter().copied().fold(edges[0], |m, e| if e.0 > m.0 { e } else { m });
            let centre = v.iter().fold(Vec3::ZERO, |s, &p| s + p) / 8.0;
            let dir = long.1.normalize();
            let top = v.iter().map(|p| p.z).fold(f32::MIN, f32::max);
            println!("box {i:3}: {:.3} x {:.3} x {:.3} centre ({:.2}, {:.2}, {:.2}) top {:.2} long dir ({:.2}, {:.2}, {:.2}) slope {:.0} deg",
                edges[0].0, edges[1].0, edges[2].0, centre.x, centre.y, centre.z, top, dir.x, dir.y, dir.z, dir.z.abs().asin().to_degrees());
        }
    }

    #[test]
    #[ignore]
    fn cache_bounds() {
        let tris = load();
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for t in &tris { for p in t.v { for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); } } }
        println!("bounds {lo:?} .. {hi:?}; {} triangles", tris.len());
    }
}
