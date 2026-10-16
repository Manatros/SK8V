"""No game/assets required; exercise metadata preservation and QA rejection."""
import copy
import importlib.util
from pathlib import Path
import unittest
import xml.etree.ElementTree as ET

spec = importlib.util.spec_from_file_location('repair', Path(__file__).with_name('repair-board-skin-flags.py'))
repair = importlib.util.module_from_spec(spec)
spec.loader.exec_module(repair)


class RepairTests(unittest.TestCase):
    def test_skin_bit_preserves_other_flags_and_rigid_models_across_lods(self):
        root = ET.fromstring('<Drawable><DrawableModelsHigh><Item><Flags value="16"/><HasSkin value="1"/></Item>'
                             '<Item><Flags value="2"/><HasSkin value="0"/></Item></DrawableModelsHigh>'
                             '<DrawableModelsLow><Item><Flags value="1"/><HasSkin value="1"/></Item>'
                             '</DrawableModelsLow></Drawable>')
        self.assertEqual(repair.repair_flags(root), 1)
        self.assertEqual([e.get('value') for e in root.iter('Flags')], ['17', '2', '1'])
        self.assertEqual(repair.repair_flags(root), 0)

    def test_roundtrip_guard_rejects_unrelated_geometry_or_material_changes(self):
        original = ET.fromstring('<Drawable><Bone tag="3"/><Shader>normal_spec</Shader><Position>1 2 3</Position></Drawable>')
        for mutate in (lambda x: x.find('Bone').set('tag', '4'),
                       lambda x: setattr(x.find('Shader'), 'text', 'default'),
                       lambda x: setattr(x.find('Position'), 'text', '1 2 3.01')):
            changed = copy.deepcopy(original)
            mutate(changed)
            self.assertFalse(repair.same_tree(original, changed))

    def test_roundtrip_guard_accepts_float_format_and_whitespace_only(self):
        self.assertTrue(repair.same_tree(ET.fromstring('<P a="1">2 3</P>'), ET.fromstring('<P a="1.0"> 2.0000\n3 </P>')))
        self.assertFalse(repair.same_text('0', 'nan'))


if __name__ == '__main__':
    unittest.main()
