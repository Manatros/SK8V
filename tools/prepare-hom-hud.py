"""Prepare Skate 3's Hall of Meat HUD (the APT `homscoring` movie) from the
user's own game files into the converted data root.

    python tools/prepare-hom-hud.py --game Skate_3 --assets local/skate-data/assets
    python tools/prepare-hom-hud.py --game Skate_3 --assets local/skate-data/assets \
        --movie chyron --hud-ytd <hud.ytd>

`--movie trickanalyser` prepares the Trick Analyzer (the gesture of the last
trick) into `private/hud-analyser`, with the gesture_item control merged in.
`--movie hudcentral` prepares Skate's centre-screen message (collectible popups)
into `private/hud-central`.
`--movie chyron` prepares Skate's TRAX now-playing banner (GTA's radio on
the board) into `private/hud-chyron`; with `--hud-ytd` (GTA's HUD atlas
`update.rpf/.../scaleform_generic.rpf/hud.ytd`, extracted by tools/prepare.py)
it also exports GTA's radio station logos into `private/hud-chyron/logos`,
which replace the EA logo per station. Logos that cannot be read leave the
EA logos in place.

Writes `<assets>/private/hud-hom/runtime/homscoring.json` plus the RGBA
textures it references, in the same `skate3-scoring-hud` v1 form as the trick
HUD (`private/hud`). The decoding pipeline is the donor's vendored UI
extractor and HUD preparer, imported in place from
upstream/skate-3-rust-engine-donor/tools (nothing is copied). Output contains
copyrighted assets and stays private (never commit or publish it).

One adaptation: the donor VM has no ToString (APT opcode 0x4B). Each one is
rewritten as `push ""` + `add` at the same offset, which converts the value
the same way (ActionScript string concatenation). Trace (0x26, debug output) becomes
Pop, which drops its argument the same way.
Get member by name (0xA5, hudcentral) becomes `push name` + GetMember.
"""
from pathlib import Path
import argparse
import json
import re
import shutil
import sys
import tempfile

from _common import sha256

ROOT = Path(__file__).resolve().parent.parent
DONOR_TOOLS = ROOT / 'upstream' / 'skate-3-rust-engine-donor' / 'tools'
sys.path.insert(0, str(DONOR_TOOLS))

from vendor.skate3_ui.big import BigArchive  # noqa: E402
from vendor.skate3_ui.project import extract_project  # noqa: E402
from vendor.skate3_ui.rw4 import decode_rgba, parse_fetch_constant, read_textures  # noqa: E402
from vendor.skate3_ui.scene_graph import AssetCache, SceneFlattener  # noqa: E402
from vendor.skate3_ui.actions import Actions  # noqa: E402
from prepare_hud import font_mapping  # noqa: E402

BUNDLE = 'data/fe/source/screens/hud2/homscoring'
MANIFEST = 'runtime/homscoring.json'
# --movie: bundle, manifest, destination under <assets>/private. `chyron` is
# the TRAX now-playing banner, shown for GTA's radio on the board.
MOVIES = {
    'homscoring': (BUNDLE, MANIFEST, 'hud-hom'),
    'chyron': ('data/fe/source/screens/main/chyron', 'runtime/chyron.json', 'hud-chyron'),
    'trickanalyser': ('data/fe/source/screens/hud2/trickanalyser', 'runtime/trickanalyser.json', 'hud-analyser'),
    'hudcentral': ('data/fe/source/screens/hud2/hudcentral', 'runtime/hudcentral.json', 'hud-central'),
    'replay_editor2': ('data/fe/source/screens/replay2/replay_editor2', 'runtime/replayeditor.json', 'hud-replay'),
}

REPLAY_BUNDLE = 'data/fe/source/screens/replay2/replay_editor2'
BUTTON_MENU_BUNDLE = 'data/fe/source/controls/button_menu'
BUTTON_ITEM_BUNDLE = 'data/fe/source/controls/button_item2'
BUTTON_RX2 = 'data/fe/source/images/buttons/xbox360/buttons.rx2'
BM_SHIFT, BM_TARGET, BM_SRC, BM_ITEM_IMPORT = 2000, 15, 7, 3
BI_SHIFT, BI_TARGET, BI_SRC, BI_RENDER = 3000, 24, 3, 2
REPLAY_STOP_ACTION = 33092
SOLID_WHITE_SHAPE = 3999

# The Trick Analyzer imports `Gesture Item` (its character 6) from this
# control, whose script attachMovie()s the trick's `g_<gesture>` clip into its
# placeholder. The HUD VM has no attachMovie, so the import is merged in:
# Gesture Item's characters move up by GESTURE_SHIFT, character 6 becomes its
# copy, and the empty clip its SetGestureType creates (`gesture`, depth 2, at
# the item's origin; the placeholder stays hidden) is a stopped sprite with
# one labelled frame per g_ clip, which the runtime's SetGestureType seeks,
# and one more per clip mirrored (`<name>_mirrored`): the script sets
# `_xscale = -100` while the rider stands goofy, which the VM does not read.
GESTURES = 'data/fe/source/controls/gesture_item'
GESTURE_SHIFT = 1000
GESTURE_ITEM, GESTURE_IMPORT, PICKER = 135, 6, 2000
STOP_ACTION = 3428  # trickanalyser's own `stop()` block


def shifted(c):
    c = json.loads(json.dumps(c))
    c['id'] += GESTURE_SHIFT
    if c.get('text'):
        c['text']['font_id'] += GESTURE_SHIFT
    for f in c.get('frames', []):
        f['controls'] = [k for k in f['controls'] if k['type_name'] not in ('do_action', 'do_init_action')]
        for k in f['controls']:
            if k['type_name'].startswith('place_object') and k['flags'] & 2:
                k['character_id'] += GESTURE_SHIFT
    return c


def merge_gestures(bundle, gestures):
    """Returns {character id: (bundle, id there)} for the merged characters."""
    origin = {}
    for c in gestures['characters'].values():
        if c['id'] == 0:  # gesture_item's own root
            continue
        m = shifted(c)
        origin[m['id']] = (GESTURES, c['id'])
        bundle['characters'][m['id']] = m
    # Two placements carry component parameters as clip actions
    # (`_type = "trickanalyzer"`, `mGestureImage = "ollie"`), which only the
    # replaced gesture_item script read; the VM has no clip actions.
    for c in bundle['characters'].values():
        for f in c.get('frames', []):
            for k in f['controls']:
                if k['type_name'].startswith('place_object') and k['flags'] & 0x80:
                    k['flags'] &= ~0x80
                    k['actions_offset'] = 0
    item = shifted(gestures['characters'][GESTURE_ITEM])
    item['id'] = GESTURE_IMPORT
    item['frames'][0]['controls'].append({
        'type_name': 'place_object2', 'flags': 0x26, 'depth': 2, 'character_id': PICKER,
        'matrix': [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 'name': 'gesture'})
    bundle['characters'][GESTURE_IMPORT] = item
    stop = {'type_name': 'do_action', 'actions_offset': STOP_ACTION}
    frames = [{'controls': [{'type_name': 'frame_label', 'label': 'none'}, stop]}]
    for name, cid in gestures['exports'].items():
        if not name.startswith('g_'):
            continue
        for label, sx in ((name, 1.0), (name + '_mirrored', -1.0)):
            frames.append({'controls': [
                {'type_name': 'frame_label', 'label': label},
                {'type_name': 'place_object2', 'flags': 6, 'depth': 1, 'character_id': cid + GESTURE_SHIFT,
                 'matrix': [sx, 0.0, 0.0, 1.0, 0.0, 0.0]},
                stop]})
    bundle['characters'][PICKER] = {'id': PICKER, 'type_name': 'sprite', 'frames': frames}
    return origin


def rewrite_to_string(code):
    """Replace ToString (0x4B) with push "" + add, and Trace (0x26) with Pop,
    keeping offsets (jump targets resolve to the first instruction at an
    offset)."""
    out = []
    for row in code:
        if row.get('body'):
            row = dict(row, body=rewrite_to_string(row['body']))
        if row['opcode'] == 0x4B:
            out.append({'offset': row['offset'], 'opcode': 0xA1, 'operand': '', 'next': row['offset']})
            out.append({'offset': row['offset'], 'opcode': 0x47, 'next': row['next']})
        elif row['opcode'] == 0xA5:  # get member by name (hudcentral's `_global.Audio`)
            out.append({'offset': row['offset'], 'opcode': 0xA1, 'operand': row['operand'], 'next': row['offset']})
            out.append({'offset': row['offset'], 'opcode': 0x4E, 'next': row['next']})
        elif row['opcode'] == 0x26:  # the Trick Analyzer's registration trace
            out.append(dict(row, opcode=0x17))
        else:
            out.append(row)
    return out


def quad_primitive(character_id: int, rgba_rel: str, width: int, height: int,
                   x0: float, y0: float, x1: float, y1: float) -> list:
    return [{
        'draw_order': 0,
        'path': '/',
        'bundle': REPLAY_BUNDLE,
        'character_id': character_id,
        'unit': 0,
        'render_type': 'texture_clamped',
        'matrix': [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        'color': [1.0, 1.0, 1.0, 1.0],
        'texture': {'rgba': rgba_rel, 'width': width, 'height': height},
        'triangles': [
            [{'position': [x0, y1], 'uv': [0.0, 1.0]},
             {'position': [x0, y0], 'uv': [0.0, 0.0]},
             {'position': [x1, y0], 'uv': [1.0, 0.0]}],
            [{'position': [x1, y0], 'uv': [1.0, 0.0]},
             {'position': [x1, y1], 'uv': [1.0, 1.0]},
             {'position': [x0, y1], 'uv': [0.0, 1.0]}],
        ],
    }]


def strip_clip_actions(bundle: dict):
    for c in bundle['characters'].values():
        for f in c.get('frames', []):
            for k in f['controls']:
                if k['type_name'].startswith('place_object') and k['flags'] & 0x80:
                    k['flags'] &= ~0x80
                    k['actions_offset'] = 0


def export_buttons(game: Path, work: Path) -> list:
    big = BigArchive(game / 'data/big/fedynamic.big')
    entry = next((e for e in big.entries if e.path == BUTTON_RX2), None)
    if entry is None:
        raise ValueError(f'{BUTTON_RX2} missing from fedynamic.big')
    rx2 = work / 'buttons.rx2'
    rx2.write_bytes(big.read(entry))
    _, textures = read_textures(rx2)
    out_dir = work / 'buttons'
    out_dir.mkdir(parents=True, exist_ok=True)
    out_dir_resolved = out_dir.resolve()
    buttons = []
    for tex in textures:
        name = tex.name
        if name.startswith('button_') and name.endswith('.Texture'):
            name = name[len('button_'):-len('.Texture')]
        safe = re.sub(r'[^A-Za-z0-9._-]', '_', Path(name).name).strip('.')
        if not safe or '..' in safe:
            raise ValueError(f'Unsafe button texture name: {tex.name!r}')
        fetch = parse_fetch_constant(bytes.fromhex(tex.metadata['fetch_constant_hex']))
        rgba = decode_rgba(tex.data, fetch)
        rel = f'buttons/{safe}.rgba'
        out_path = (work / rel).resolve()
        if not out_path.is_relative_to(out_dir_resolved):
            raise ValueError(f'Button texture escaped output directory: {tex.name!r}')
        out_path.write_bytes(rgba)
        buttons.append((safe, rel, fetch.width, fetch.height))
    white_rel = 'buttons/solid_white.rgba'
    (work / white_rel).write_bytes(b'\xff\xff\xff\xff')
    return buttons, white_rel


def merge_control_bundle(bundle: dict, src: dict, shift: int, root_target: int, root_src: int,
                         remap: dict, src_actions: Actions, offset_base: int, blocks: dict):
    """Merge a control's characters and init actions into `bundle`."""
    origin = {}
    root_actions = []
    for c in src['characters'].values():
        if c['id'] == 0:
            for f in c.get('frames', []):
                for k in f['controls']:
                    off = k.get('actions_offset')
                    if k['type_name'] == 'do_init_action' and off:
                        new_off = offset_base + off
                        blocks[str(new_off)] = rewrite_to_string(src_actions.stream(off))
                        root_actions.append({'type_name': 'do_init_action', 'actions_offset': new_off})
            continue
        m = json.loads(json.dumps(c))
        cid = root_target if c['id'] == root_src else c['id'] + shift
        m['id'] = cid
        if m.get('text'):
            m['text']['font_id'] += shift
        for f in m.get('frames', []):
            kept = []
            for k in f['controls']:
                if k['type_name'] in ('do_action', 'do_init_action'):
                    continue
                if k['type_name'].startswith('place_object') and k['flags'] & 2:
                    k['character_id'] = remap.get(k['character_id'], k['character_id'] + shift)
                    if k['flags'] & 0x80:
                        k['flags'] &= ~0x80
                        k['actions_offset'] = 0
                kept.append(k)
            f['controls'] = kept
        bundle['characters'][cid] = m
        origin[cid] = (c['id'], m)
    bundle['characters'][0]['frames'][0]['controls'][:0] = root_actions
    return origin


def merge_replay(game: Path, work: Path, cache: AssetCache, bundle: dict, blocks: dict):
    """Merge button_menu, button_item2, and Xbox 360 button textures into replay_editor2."""
    strip_clip_actions(bundle)
    bm = cache.load_bundle(BUTTON_MENU_BUNDLE)
    bi = cache.load_bundle(BUTTON_ITEM_BUNDLE)
    bm_apt = work / 'raw' / (BUTTON_MENU_BUNDLE + '.apt')
    bm_act = Actions(bm_apt.read_bytes(), bm_apt.with_suffix('.const').read_bytes())
    bi_apt = work / 'raw' / (BUTTON_ITEM_BUNDLE + '.apt')
    bi_act = Actions(bi_apt.read_bytes(), bi_apt.with_suffix('.const').read_bytes())
    origin = {}
    bm_orig = merge_control_bundle(
        bundle, bm, BM_SHIFT, BM_TARGET, BM_SRC,
        {BM_ITEM_IMPORT: BI_TARGET}, bm_act, 2_000_000, blocks,
    )
    for cid, (orig_id, _) in bm_orig.items():
        origin[cid] = (BUTTON_MENU_BUNDLE, orig_id)
    bi_orig = merge_control_bundle(
        bundle, bi, BI_SHIFT, BI_TARGET, BI_SRC,
        {}, bi_act, 3_000_000, blocks,
    )
    for cid, (orig_id, _) in bi_orig.items():
        origin[cid] = (BUTTON_ITEM_BUNDLE, orig_id)

    buttons, white_rel = export_buttons(game, work)
    stop = {'type_name': 'do_action', 'actions_offset': REPLAY_STOP_ACTION}
    render_frames = [{'controls': [{'type_name': 'frame_label', 'label': 'none'}, stop]}]
    synthetic_shapes = {str(SOLID_WHITE_SHAPE): quad_primitive(SOLID_WHITE_SHAPE, white_rel, 1, 1, 0.0, 0.0, 1.0, 1.0)}
    bundle['characters'][SOLID_WHITE_SHAPE] = {'id': SOLID_WHITE_SHAPE, 'type_name': 'shape', 'frames': []}
    for idx, (btn_name, rel, w, h) in enumerate(buttons):
        shape_id = 3100 + idx
        render_frames.append({'controls': [
            {'type_name': 'frame_label', 'label': btn_name},
            {'type_name': 'place_object2', 'flags': 6, 'depth': 1, 'character_id': shape_id,
             'matrix': [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]},
            stop,
        ]})
        bundle['characters'][shape_id] = {'id': shape_id, 'type_name': 'shape', 'frames': []}
        synthetic_shapes[str(shape_id)] = quad_primitive(shape_id, rel, w, h, -16.5, -16.5, 15.5, 15.5)
    bundle['characters'][BI_SHIFT + BI_RENDER] = {
        'id': BI_SHIFT + BI_RENDER, 'type_name': 'sprite', 'frames': render_frames,
    }
    return origin, synthetic_shapes


def prepare(game: Path, work: Path, collections: Path, bundle: str = BUNDLE, manifest: str = MANIFEST) -> Path:
    BUNDLE, MANIFEST = bundle, manifest
    analyser = BUNDLE == MOVIES['trickanalyser'][0]
    replay = BUNDLE == REPLAY_BUNDLE
    prefixes = (BUNDLE, GESTURES) if analyser else (BUNDLE, BUTTON_MENU_BUNDLE, BUTTON_ITEM_BUNDLE) if replay else (BUNDLE,)
    extract_project(game, work, prefixes=prefixes, update=True)
    cache = AssetCache(work)
    mappings = font_mapping(collections, cache)
    bundle = cache.load_bundle(BUNDLE)
    blocks = {}
    synthetic_shapes = {}
    if analyser:
        origin = merge_gestures(bundle, cache.load_bundle(GESTURES))
    elif replay:
        origin, synthetic_shapes = merge_replay(game, work, cache, bundle, blocks)
    else:
        origin = {}
    # A character placed but defined in another movie (hudcentral's team logo
    # loader, character 23, an import the HUD VM cannot follow) becomes an
    # empty clip, so placing it is harmless; only online team messages used it.
    placed = {k['character_id'] for c in bundle['characters'].values() for f in c.get('frames', [])
              for k in f['controls'] if k['type_name'].startswith('place_object') and k['flags'] & 2}
    for cid in sorted(placed - set(bundle['characters'])):
        print(f'Character {cid} is not in {BUNDLE}: placed as an empty clip')
        bundle['characters'][cid] = {'id': cid, 'type_name': 'sprite', 'frames': [{'controls': []}]}
    apt_path = work / 'raw' / (BUNDLE + '.apt')
    const_path = apt_path.with_suffix('.const')
    actions = Actions(apt_path.read_bytes(), const_path.read_bytes())
    for c in bundle['characters'].values():
        for f in c.get('frames', []):
            for control in f['controls']:
                if control['type_name'] in ('do_action', 'do_init_action') and control.get('actions_offset')                         and str(control['actions_offset']) not in blocks:
                    offset = control['actions_offset']
                    blocks[str(offset)] = rewrite_to_string(actions.stream(offset))
    shapes, fonts = dict(synthetic_shapes), {}
    for c in bundle['characters'].values():
        if c['type_name'] == 'shape':
            if str(c['id']) in synthetic_shapes:
                continue
            scene = SceneFlattener(cache, lambda *_: {}).flatten(*origin.get(c['id'], (BUNDLE, c['id'])))
            if scene['unresolved']:
                raise ValueError(f"Unresolved HoM HUD shape: {scene['unresolved']}")
            # Untextured `line` primitives are authoring guides (chyron's
            # orange safe-area frame); the HUD renderer draws textures only.
            shapes[str(c['id'])] = [p for p in scene['primitives'] if p['texture']]
        elif c['type_name'] == 'font':
            family = c['font']['name']
            fonts[family] = cache.font_asset(family)
    language = json.loads((work / 'metadata/languages/english_global.json').read_text(encoding='utf-8'))
    result = {
        'format': 'skate3-scoring-hud', 'version': 1,
        'source': {'bundle': BUNDLE,
                   'apt_sha256': sha256(apt_path.read_bytes()),
                   'const_sha256': sha256(const_path.read_bytes()),
                   'collections_sha256': sha256(collections.read_bytes())},
        'characters': list(bundle['characters'].values()),
        'shapes': shapes, 'fonts': fonts, 'actions': blocks,
        'font_mappings': mappings,
        'language': {row['label'].strip(): row['value'] for row in language['entries']},
        'unresolved_fonts': [family for family, asset in fonts.items() if asset is None],
    }
    if result['unresolved_fonts']:
        raise ValueError('Unresolved HoM HUD fonts: ' + ', '.join(result['unresolved_fonts']))
    target = work / MANIFEST
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(result, separators=(',', ':')) + '\n', encoding='utf-8')
    print(f'Prepared {len(shapes)} HoM HUD shapes and {len(blocks)} action blocks')
    return target


# GTA's radio station logos in hud.ytd: texture, cell (x, y) of 128 px ->
# GET_RADIO_STATION_NAME id. White on black; luminance becomes alpha. The
# atlas cells were matched by eye: the mast is Soulwax FM, the
# eagle Blaine County Radio (the two base stations left).
STATION_LOGOS = {
    'gtav_radio_stations_texture_512': {
        (0, 0): 'RADIO_06_COUNTRY', (1, 0): 'RADIO_08_MEXICAN', (2, 0): 'RADIO_04_PUNK',
        (3, 0): 'RADIO_01_CLASS_ROCK', (0, 1): 'RADIO_02_POP', (1, 1): 'RADIO_07_DANCE_01',
        (2, 1): 'RADIO_11_TALK_02', (3, 1): 'RADIO_16_SILVERLAKE', (0, 2): 'RADIO_09_HIPHOP_OLD',
        (1, 2): 'RADIO_18_90S_ROCK', (2, 2): 'RADIO_03_HIPHOP_NEW', (3, 2): 'RADIO_12_REGGAE',
        (0, 3): 'RADIO_13_JAZZ', (1, 3): 'RADIO_14_DANCE_02', (2, 3): 'RADIO_15_MOTOWN', (3, 3): 'OFF',
    },
    'gta_radio_stations_texture02_512': {(0, 0): 'RADIO_17_FUNK', (1, 0): 'RADIO_05_TALK_01'},
    'gta_radio_stations_texture03_128': {(0, 0): 'RADIO_19_USER'},
    'gta_radio_stations_texture04_128': {(0, 0): 'RADIO_20_THELAB'},
    'gta_radio_stations_texture05_128': {(0, 0): 'RADIO_21_DLC_XM17'},
    'gta_radio_stations_texture06_128': {(0, 0): 'RADIO_22_DLC_BATTLE_MIX1_RADIO'},
    'gta_radio_stations_texture07_128': {(0, 0): 'RADIO_23_DLC_XM19_RADIO'},
    'gta_radio_stations_texture08_128': {(0, 0): 'RADIO_35_DLC_HEI4_MLR'},
    'gta_radio_stations_texture09_128': {(0, 0): 'RADIO_27_DLC_PRHEI4'},
    'gta_radio_stations_texture10_128': {(0, 0): 'RADIO_34_DLC_HEI4_KULT'},
    'gta_radio_stations_texture11_128': {(0, 0): 'RADIO_36_AUDIOPLAYER'},
    'gta_radio_stations_texture12_128': {(0, 0): 'RADIO_37_MOTOMAMI'},
}


def station_logos(ytd: Path, work: Path, destination: Path) -> int:
    import subprocess
    from PIL import Image
    rage = ROOT / 'build' / 'tools' / 'rage.exe'
    if not rage.exists():
        raise ValueError(f'{rage} missing: run tools/build-rage-cli.ps1')
    subprocess.run([str(rage), '--no-update-check', 'textures', str(ytd), '-o', str(work / 'gta-hud')], check=True)
    out = destination / 'logos'
    out.mkdir(parents=True, exist_ok=True)
    rows = []
    for texture, cells in STATION_LOGOS.items():
        image = Image.open(work / 'gta-hud' / f'{texture}.png').convert('L')
        for (x, y), station in cells.items():
            cell = image.crop((x * 128, y * 128, x * 128 + 128, y * 128 + 128))
            rgba = bytearray()
            for value in cell.getdata():
                rgba += bytes((255, 255, 255, value))
            name = station.lower() + '.rgba'
            (out / name).write_bytes(bytes(rgba))
            rows.append({'id': station, 'file': name, 'width': 128, 'height': 128})
    (out / 'logos.json').write_text(json.dumps(rows, indent=1) + '\n', encoding='utf-8')
    return len(rows)


def textures(manifest: dict):
    for shape in manifest['shapes'].values():
        for primitive in shape:
            t = primitive['texture']
            yield t['rgba'], t['width'], t['height']
    for font in manifest['fonts'].values():
        size = font['definition']['textures'][0]
        yield font['texture'], size['width'], size['height']


def contained(root: Path, relative: str) -> Path:
    path = (root / relative).resolve()
    if not path.is_relative_to(root.resolve()) or path == root.resolve():
        raise ValueError(f'HUD path escapes its root: {relative}')
    return path


def install(work: Path, destination: Path, name: str = MANIFEST) -> int:
    manifest = json.loads((work / name).read_text(encoding='utf-8'))
    files = [name]
    for relative, width, height in textures(manifest):
        source = contained(work, relative)
        if width <= 0 or height <= 0 or source.stat().st_size != width * height * 4:
            raise ValueError(f'Invalid RGBA size: {source}')
        files.append(relative)
    staged = destination.with_name(destination.name + '.partial')
    if staged.exists():
        shutil.rmtree(staged)
    for relative in sorted(set(files)):
        target = contained(staged, relative)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(contained(work, relative), target)
    if destination.exists():
        shutil.rmtree(destination)
    staged.rename(destination)
    return len(set(files))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--game', type=Path, required=True, help='extracted Skate 3 folder (contains data/)')
    parser.add_argument('--assets', type=Path, required=True, help='converted data root (<out>/assets)')
    parser.add_argument('--movie', choices=sorted(MOVIES), default='homscoring')
    parser.add_argument('--hud-ytd', type=Path, help="GTA's hud.ytd: export its radio logos (--movie chyron)")
    args = parser.parse_args()
    bundle, manifest, folder = MOVIES[args.movie]
    assets = args.assets.resolve()
    collections = assets / 'private/stock/skater-collections.json'
    if not collections.exists():
        parser.error(f'{collections} missing: run tools/convert-skate-data.py first')
    # Retail names plus the workspace prefix can exceed legacy path limits.
    with tempfile.TemporaryDirectory(prefix='svhom-') as temporary:
        work = Path(temporary)
        prepare(args.game.resolve(), work, collections, bundle, manifest)
        count = install(work, assets / 'private' / folder, manifest)
        if args.movie == 'chyron' and args.hud_ytd:
            try:
                print(f'{station_logos(args.hud_ytd.resolve(), work, assets / "private" / folder)} station logos')
            except Exception as e:  # the banner still works with the EA logos
                print(f'station logos skipped: {e}')
    print(f'{args.movie} HUD ready: {count} files in {assets / "private" / folder}')


if __name__ == '__main__':
    main()
