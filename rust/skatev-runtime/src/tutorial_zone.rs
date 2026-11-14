//! Optional offshore collision; never consults mainland tile policy.
use crate::{
    coords,
    world::{self, Area, RADIUS},
};
use bevy_math::Vec3;
use std::path::{Path, PathBuf};

pub(crate) fn district(center: Vec3) -> Option<&'static str> {
    if (center.y + 3200.0).abs() > 400.0 || !center.is_finite() {
        return None;
    }
    if (center.x + 4500.0).abs() <= 400.0 {
        Some("DIST_SkateSchool")
    } else if (center.x + 5450.0).abs() <= 400.0 {
        Some("DIST_StartPark")
    } else {
        None
    }
}

pub(crate) struct TutorialZone {
    root: PathBuf,
}

impl TutorialZone {
    pub(crate) fn open(cache_root: &Path) -> Option<Self> {
        let root = cache_root.join("addons/skate_school");
        root.is_dir().then_some(Self { root })
    }

    pub(crate) fn around(&self, center: Vec3) -> Option<Result<Area, String>> {
        district(center).map(|name| {
            self.build(center, name)
                .map_err(|e| format!("tutorial {name}: {e}"))
        })
    }

    fn build(&self, center: Vec3, name: &str) -> Result<Area, String> {
        // Accept flat packs as well as the manifest's collision/splines layout.
        let locate = |folder: &str, file: String| {
            let nested = self.root.join(folder).join(&file);
            if nested.is_file() {
                nested
            } else {
                self.root.join(file)
            }
        };
        let mut cache = svwc::Cache::open(&locate("collision", format!("{name}.svwc")))
            .map_err(|e| e.to_string())?;
        let raw = cache
            .query([center.x, center.y], RADIUS)
            .map_err(|e| e.to_string())?;
        let json = std::fs::read(locate("splines", format!("{name}_rails.json")))
            .map_err(|e| e.to_string())?;
        let records: Vec<serde_json::Value> =
            serde_json::from_slice(&json).map_err(|e| e.to_string())?;
        let mut rails = Vec::new();
        for record in records {
            let mut points: Vec<[f32; 3]> =
                serde_json::from_value(record["points_gta"].clone()).map_err(|e| e.to_string())?;
            if points.iter().flatten().any(|v| !v.is_finite()) {
                return Err("non-finite rail point".into());
            }
            if record["closed"].as_bool() == Some(true)
                && points.len() > 2
                && points.first() != points.last()
            {
                points.push(points[0]);
            }
            // Segment bounds include rails crossing the query with both ends outside.
            if points.windows(2).any(|s| {
                (0..2).all(|a| {
                    s[0][a].min(s[1][a]) <= center[a] + RADIUS
                        && s[0][a].max(s[1][a]) >= center[a] - RADIUS
                })
            }) {
                rails.push(
                    points
                        .into_iter()
                        .map(|p| coords::to_skate(Vec3::from_array(p)).to_array())
                        .collect(),
                );
            }
        }
        world::normalize_assembled_rails(&mut rails);
        world::cap_rail_segments(
            &mut rails,
            coords::to_skate(center),
            world::STATIC_RAIL_SEGMENTS,
        );
        crate::ground_at::publish(crate::ground_at::GroundIndex::build(
            raw.iter().map(|t| (t.v, t.material)),
        ));
        let tags = raw
            .iter()
            .map(|t| crate::materials::surface(t.material).packed())
            .collect();
        let triangles = raw
            .into_iter()
            .map(|t| {
                t.v.map(|p| coords::to_skate(Vec3::from_array(p)).to_array())
            })
            .collect();
        Ok(Area {
            center,
            triangles,
            tags,
            rails,
            noses: Vec::new(),
            census: Default::default(),
            bevels: Default::default(),
            note: format!("tutorial {name}"),
            bounds: Vec::new(),
            live_required: false,
            changed: None,
            fixed: 0,
            options: world::options_version(),
            origins: Default::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_pack_bypasses_mainland_and_converts_coordinates() {
        let dir = std::env::temp_dir().join(format!(
            "skatev-tutorial-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(TutorialZone::open(&dir).is_none());
        let root = dir.join("addons/skate_school");
        std::fs::create_dir_all(root.join("collision")).unwrap();
        std::fs::create_dir_all(root.join("splines")).unwrap();
        for (name, x, z) in [
            ("DIST_SkateSchool", -4500., 93.),
            ("DIST_StartPark", -5450., 20.),
        ] {
            let tri = svwc::Tri {
                v: [[x, -3200., z], [x + 8., -3200., z], [x, -3192., z]],
                material: 0,
            };
            svwc::write(
                &mut std::fs::File::create(root.join(format!("collision/{name}.svwc"))).unwrap(),
                &[tri],
                64.,
            )
            .unwrap();
            let rails = serde_json::json!([{"points_gta": [[x,-3200.,z+1.],[x+8.,-3200.,z+1.]], "closed": false}]);
            std::fs::write(
                root.join(format!("splines/{name}_rails.json")),
                rails.to_string(),
            )
            .unwrap();
        }
        // An incompatible mainland policy and no live GTA level must not matter.
        std::fs::create_dir_all(dir.join("tiles")).unwrap();
        std::fs::write(dir.join("tiles/rules.txt"), "invalid policy").unwrap();
        let zone = TutorialZone::open(&dir).unwrap();
        assert!(zone.around(Vec3::ZERO).is_none());
        assert!(zone.around(Vec3::new(f32::NAN, -3200., 93.)).is_none());
        let mut world = world::StaticWorld::open(&dir.join("world.svwc"));
        for (x, z) in [(-4500., 93.), (-5450., 20.)] {
            let area = world.around(Vec3::new(x, -3200., z)).unwrap();
            assert_eq!(area.triangles.len(), 1);
            assert_eq!(area.tags.len(), 1);
            assert_eq!(area.triangles[0][0], [x, z, 3200.]);
            assert!(!area.rails.is_empty());
            assert!(!area.live_required);
        }
        assert!(world.around(Vec3::ZERO).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
