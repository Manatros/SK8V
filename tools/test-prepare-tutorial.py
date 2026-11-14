"""Unit tests for M0 Skate School / StartPark conversion helpers and waterline guardrails."""

from __future__ import annotations

import importlib.util
import json
import struct
import tempfile
import unittest
from pathlib import Path

TOOLS_DIR = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('prepare_tutorial', TOOLS_DIR / 'prepare-tutorial.py')
assert SPEC and SPEC.loader
MOD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MOD)


class PrepareTutorialTests(unittest.TestCase):
    def test_from_skate_maps_y_up_to_z_up_and_preserves_right_handed_basis(self):
        # Skate (+X, +Y_up, +Z) -> GTA (+X, -Z, +Y_up)
        self.assertEqual(MOD.from_skate(10.0, 25.0, -4.0), (10.0, 4.0, 25.0))
        self.assertEqual(MOD.from_skate(0.0, 1.0, 0.0), (0.0, 0.0, 1.0))

    def test_compute_safe_island_origin_keeps_lowest_skirt_above_water(self):
        # DIST_SkateSchool lowest Skate Y is ~ -73.03; origin Z lifts min Z to +20.0m
        ox, oy, oz = MOD.compute_safe_island_origin(-73.03, (-4500.0, -3200.0), min_clearance_m=20.0)
        self.assertEqual((ox, oy), (-4500.0, -3200.0))
        self.assertAlmostEqual(oz + (-73.03), 20.0, places=5)

    def test_validate_no_underwater_rejects_sea_level_or_submerged_points(self):
        with self.assertRaises(ValueError):
            MOD.validate_no_underwater('submerged', [25.0, -0.05, 30.0])
        with self.assertRaises(ValueError):
            MOD.validate_no_underwater('too-low-clearance', [12.0, 25.0], min_clearance_m=20.0)
        with self.assertRaises(ValueError):
            MOD.validate_no_underwater('non-finite', [22.0, float('nan')])

        ok = MOD.validate_no_underwater('valid-island', [20.0, 95.5, 174.0], min_clearance_m=20.0)
        self.assertEqual(ok['min_z'], 20.0)
        self.assertEqual(ok['max_z'], 174.0)

    def test_english_only_language_filter_excludes_german_and_other_locales(self):
        self.assertTrue(MOD.is_english_only_path('data/audio/english/dialog_en.sbs'))
        self.assertTrue(MOD.is_english_only_path('data/movies/Coach_frank_intro_english_ntsc.vp6'))
        self.assertFalse(MOD.is_english_only_path('data/audio/german/dialog_de.sbs'))
        self.assertFalse(MOD.is_english_only_path('data/movies/Coach_frank_intro_german_pal.vp6'))

    def test_write_collision_cache_binary_layout(self):
        tris = [((1.0, 2.0, 25.0), (2.0, 2.0, 25.0), (1.0, 3.0, 25.0), 7)]
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / 'test.sk8m'
            MOD.write_collision_cache(out, tris)
            raw = out.read_bytes()
        self.assertEqual(len(raw), 12 + 40)
        magic, ver, count = struct.unpack_from('<4sII', raw, 0)
        self.assertEqual((magic, ver, count), (b'SK8M', 1, 1))
        coords_surf = struct.unpack_from('<9fI', raw, 12)
        self.assertEqual(coords_surf[-1], 7)
        self.assertAlmostEqual(coords_surf[2], 25.0)

    def test_decode_cubic_spline_segment_payload_keeps_z_above_sea_level(self):
        # Verify cubic segment payload math: d=(1, 5, 2) start, a=b=0, c=(2, 1, 0) -> end=(3, 6, 2)
        seg = bytearray(120)
        struct.pack_into('>3f', seg, 32, 2.0, 1.0, 0.0)   # c (delta)
        struct.pack_into('>3f', seg, 48, 1.0, 5.0, 2.0)   # d (start)
        a = struct.unpack_from('>3f', seg, 0)
        b = struct.unpack_from('>3f', seg, 16)
        c = struct.unpack_from('>3f', seg, 32)
        d = struct.unpack_from('>3f', seg, 48)
        end = tuple(a[k] + b[k] + c[k] + d[k] for k in range(3))
        self.assertEqual(end, (3.0, 6.0, 2.0))
        gx, gy, gz = MOD.from_skate(*end)
        self.assertEqual((gx, gy, gz), (3.0, -2.0, 6.0))

    def test_verify_manifest_checks_sk8m_and_rejects_submerged_cache(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            col = root / 'collision' / 'DIST_Mock.sk8m'
            rails = root / 'splines' / 'DIST_Mock_rails.json'
            rails.parent.mkdir(parents=True, exist_ok=True)
            MOD.write_collision_cache(col, [((0.0, 0.0, 25.0), (1.0, 0.0, 25.0), (0.0, 1.0, 25.0), 1)])
            rails.write_text(json.dumps([{'index': 0, 'points_gta': [[0.0, 0.0, 26.0]]}]), encoding='utf-8')
            mpath = root / 'm0-tutorial-manifest.json'
            mpath.write_text(json.dumps({
                'format': 'gtaxskate-m0-tutorial-v1',
                'waterline_guardrail': 'NO_UNDERWATER_ASSETS',
                'tutorial_metadata': {'language': 'english', 'english_only_enforced': True},
                'districts': [{
                    'district': 'DIST_Mock',
                    'outputs': {'collision_cache': 'collision/DIST_Mock.sk8m', 'rails_json': 'splines/DIST_Mock_rails.json'},
                }],
            }), encoding='utf-8')
            ok = MOD.verify_manifest(mpath)
            self.assertEqual(ok['DIST_Mock']['triangles'], 1)

            # Corrupt triangle below sea level and verify fail-closed rejection
            MOD.write_collision_cache(col, [((0.0, 0.0, -2.0), (1.0, 0.0, 25.0), (0.0, 1.0, 25.0), 1)])
            with self.assertRaises(ValueError):
                MOD.verify_manifest(mpath)

    def test_write_svwc_v2_tiled_header_and_multi_tile_triangle(self):
        # Triangle spanning tiles (0,0) and (1,0) at tile_size=64.0
        tris = [((60.0, 10.0, 22.0), (70.0, 10.0, 22.0), (60.0, 20.0, 22.0), 4)]
        with tempfile.TemporaryDirectory() as tmp:
            svwc = Path(tmp) / 'test.svwc'
            tile_cnt, rec_cnt = MOD.write_svwc_v2(svwc, tris, tile_size=64.0)
            self.assertEqual((tile_cnt, rec_cnt), (2, 2))
            raw = svwc.read_bytes()
        magic, ver, tile_size, t_count, r_count = struct.unpack_from('<4sIfII', raw, 0)
        self.assertEqual((magic, ver, tile_size, t_count, r_count), (b'SVWC', 2, 64.0, 2, 2))
        self.assertEqual(len(raw), 20 + 2 * 16 + 2 * 40)


if __name__ == '__main__':
    unittest.main()
