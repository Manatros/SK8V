"""Disc-free contract tests for the optional tutorial DLC builder."""
import importlib.util
import json
import struct
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path
from types import SimpleNamespace as NS
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('tutorial_dlc', Path(__file__).with_name('prepare-tutorial-dlc.py'))
dlc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dlc)


class TutorialDlcTests(unittest.TestCase):
    def test_chunk_over_60000_preserves_all_triangles(self):
        vertices = [(float(i), 0., 20.) for i in range(60003)]
        faces = [(i, i+1, i+2) for i in range(0, len(vertices), 3)]
        chunks = list(dlc.chunk_mesh(vertices, faces))
        self.assertEqual([len(rows) for rows, _ in chunks], [60000, 3])
        reconstructed = [tuple(rows[i][:3] for i in ids[j:j+3])
                         for rows, ids in chunks for j in range(0, len(ids), 3)]
        self.assertEqual(reconstructed, [tuple(vertices[i] for i in face) for face in faces])

    def test_shared_vertices_remapped_across_boundary(self):
        vertices = [(0., 0., 20.), (1., 0., 20.), (0., 1., 20.), (1., 1., 20.)]
        chunks = list(dlc.chunk_mesh(vertices, [(0, 1, 2), (2, 1, 3)], limit=3))
        self.assertEqual(len(chunks), 2)
        self.assertEqual(chunks[1][0][0][:3], vertices[2])
        self.assertEqual(chunks[1][1], [0, 1, 2])

    def test_transform_matches_m0_origin(self):
        origin = dlc.district_origin({'district': 'DIST_SkateSchool',
                                     'skate3_y_bounds': [-73.027, 79.014],
                                     'island_origin_gta': [-4500., -3200., 93.027]})
        point = dlc.transform((10., -73.027, 5.), origin)
        self.assertEqual(point[:2], (-4490., -3205.))
        self.assertAlmostEqual(point[2], 20.)

    def test_origin_mismatch_rejected(self):
        with self.assertRaisesRegex(ValueError, 'M0 origin'):
            dlc.district_origin({'district': 'DIST_StartPark', 'skate3_y_bounds': [0., 1.],
                                 'island_origin_gta': [-5450., -3200., 21.]})

    def test_underwater_rejection_is_strict(self):
        for z in (0., -1., 19.99999, float('nan'), float('inf')):
            with self.subTest(z=z), self.assertRaises(ValueError):
                dlc.validate_geometry([(0., 0., z)])
        self.assertEqual(dlc.validate_geometry([(0., 0., 20.)])['min_z'], 20.)

    def test_invalid_geometry_rejected(self):
        with self.assertRaises(ValueError):
            dlc.validate_geometry([])
        with self.assertRaisesRegex(ValueError, 'index'):
            list(dlc.chunk_mesh([(0., 0., 20.)], [(0, 1, 2)]))
        with self.assertRaises(ValueError):
            list(dlc.chunk_mesh([(0., 0., 20.)]*3, [(0, 1, 2)], limit=60001))

    def test_drawable_and_menyoo_round_trip(self):
        rows = [(0., 0., 0., 0., 0., 1., 0., 0.), (1., 0., 0., 0., 0., 1., 1., 0.),
                (0., 1., 0., 0., 0., 1., 0., 1.)]
        root = ET.fromstring(ET.tostring(dlc.drawable_xml('school_test', rows, [0, 1, 2], 'school_white')))
        self.assertEqual(root.tag, 'Drawable')
        self.assertEqual(root.findtext('./ShaderGroup/Shaders/Item/Parameters/Item/Name'), 'school_white')
        data = root.findtext('.//VertexBuffer/Data').strip().splitlines()
        self.assertEqual([len(line.split()) for line in data], [12]*3)
        self.assertEqual(root.findtext('.//IndexBuffer/Data'), '0 1 2')
        placements = dlc.menyoo_xml([('school_test', (-4500., -3200., 20.))])
        self.assertEqual(placements.findtext('./Placement/HashName'), 'school_test')
        self.assertEqual(int(placements.findtext('./Placement/ModelHash'), 16), dlc.joaat('school_test'))
        self.assertEqual(float(placements.findtext('./Placement/PositionRotation/Z')), 20.)

    def test_joaat_known_gta_model_hash(self):
        self.assertEqual(dlc.joaat('adder'), 0xB779A091)
        self.assertEqual(dlc.joaat('ADDER'), 0xB779A091)

    def test_dlc_metadata_registers_optional_pack(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)
            dlc.dlc_metadata(path)
            content = ET.parse(path / 'content.xml').getroot()
            setup = ET.parse(path / 'setup2.xml').getroot()
            self.assertEqual(setup.findtext('deviceName'), 'dlc_skate_school')
            self.assertEqual(setup.findtext('datFile'), 'content.xml')
            self.assertEqual(setup.findtext('nameHash'), 'skate_school')
            self.assertEqual([n.text for n in content.findall('./dataFiles/Item/fileType')],
                             ['RPF_FILE', 'DLC_ITYP_REQUEST'])
            self.assertEqual(content.findtext('./contentChangeSets/Item/changeSetName'),
                             setup.findtext('./contentChangeSetGroups/Item/ContentChangeSets/Item'))
            self.assertEqual(len(content.findall('./contentChangeSets/Item/filesToEnable/Item')), 2)

    def test_direct_dds_header_and_pixels(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'test.dds'
            pixels = bytes([255, 0, 0, 255, 0, 255, 0, 255])
            dlc.write_dds(path, 2, 1, pixels)
            data = path.read_bytes()
            self.assertEqual(data[:4], b'DDS ')
            self.assertEqual(struct.unpack_from('<4I', data, 4), (124, 0x100F, 1, 2))
            self.assertEqual(data[128:], pixels)

    def fake_export(self, root, *, dry_run=False, xml_only=False, height=0.):
        manifest = root / 'm0.json'
        manifest.write_text(json.dumps({'districts': [
            {'district': name, 'skate3_y_bounds': [0., 1.], 'island_origin_gta': [*xy, 20.]}
            for name, xy in dlc.m0.ISLAND_ANCHORS_XY.items()]}))
        class Archive:
            def __init__(self, path):
                self.entries = [NS(path='stream.bin')]
            def read(self, entry):
                return b'stream'
        def stream(path, kind, district):
            self.assertEqual(kind, 'Pres')
            self.assertEqual((path / 'stream.bin').read_bytes(), b'stream')
            return [NS(record=NS(asset_type=4, asset_id=1), source_path=Path('english/model'), data=b'model'),
                    NS(record=NS(asset_type=4096, asset_id=2), source_path=Path('english/texture'),
                       data=b'0x0000000000000002.Texture')]
        mesh = NS(name='english_ground', vertices=[(0., height, 0.), (1., height, 0.), (0., height, 1.)],
                  faces=[(0, 1, 2)], normals=None, uvs=None, material_name='material_0x0000000000000002')
        texture = NS(width=1, height=1, rgba=b'\xff'*4)
        donor = (Archive, stream, 4, 4096, NS(parse_rx2=lambda data: NS(textures=[texture])),
                 lambda data, strict: NS(meshes=[mesh]))
        args = NS(manifest=manifest, donor=root, out=root / 'out', disc=root / 'disc',
                  rage=Path('custom/rage.exe'), dry_run=dry_run, xml_only=xml_only)
        with patch.object(dlc.m0, 'resolve_donor_root', return_value=root), \
             patch.object(dlc, 'load_donor', return_value=donor):
            return dlc.export(args)

    def test_mocked_rage_build_commands(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(dlc.subprocess, 'run') as run:
            result = self.fake_export(Path(tmp))
            self.assertEqual(result['chunks'], 2)
            self.assertEqual(run.call_count, 8)
            commands = [call.args[0] for call in run.call_args_list]
            self.assertTrue(all(cmd[0] == str(Path('custom/rage.exe')) for cmd in commands))
            self.assertTrue(all(call.kwargs == {'check': True} for call in run.call_args_list))
            self.assertEqual(commands[0][1:3], ['textures', 'build'])
            self.assertEqual(commands[3][1:3], ['ytyp', 'from-drawables'])
            self.assertEqual(commands[3][commands[3].index('--flags')+1], '131104')
            self.assertIn('--txd', commands[3])
            self.assertEqual(commands[4][1:3], ['ymap', 'from-menyoo'])
            self.assertEqual(commands[5][1:3], ['manifest', 'generate'])
            self.assertEqual(commands[-1][1:4], ['create', '-e', 'open'])
            self.assertEqual(Path(commands[-1][-2]).name, 'dlc.rpf')

    def test_offline_modes_never_launch_rage(self):
        for mode in ('dry_run', 'xml_only'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as tmp, \
                 patch.object(dlc.subprocess, 'run') as run:
                result = self.fake_export(Path(tmp), **{mode: True})
                run.assert_not_called()
                self.assertTrue(result['NO_UNDERWATER'])
                self.assertEqual(result['min_z'], 20.)
                root = Path(tmp) / 'out'
                self.assertEqual(len(list((root / 'xml').glob('*.ydr.xml'))), 2)
                self.assertTrue((root / 'textures/school_skateschool_0x0000000000000002_0.dds').is_file())
                ET.parse(root / 'skateschool.menyoo.xml')

    def test_unsafe_export_never_packs(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(dlc.subprocess, 'run') as run:
            with self.assertRaises(ValueError):
                self.fake_export(Path(tmp), height=-1.)
            run.assert_not_called()

    def test_rage_failure_stops_before_pack(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(dlc.subprocess, 'run') as run:
            run.side_effect = dlc.subprocess.CalledProcessError(2, ['rage'])
            with self.assertRaises(dlc.subprocess.CalledProcessError):
                self.fake_export(Path(tmp))
            self.assertEqual(run.call_count, 1)


if __name__ == '__main__':
    unittest.main()
