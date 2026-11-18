"""Build the optional skate_school DLC from the owner's Skate 3 disc and M0 manifest.

--xml-only exports drawables, DDS, Menyoo and DLC metadata without rage.exe.
--dry-run performs the same decode and safety checks and records the build commands,
but never runs rage.exe. Retail-derived output belongs under ignored local/ only.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import math
import os
import re
import struct
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('tutorial_m0', Path(__file__).with_name('prepare-tutorial.py'))
m0 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m0)
MAX_VERTICES = 60_000
SK8V_ROOT = Path(os.environ.get('SK8V_ROOT', str(REPO)))
DEFAULT_RAGE = Path(os.environ.get('SK8V_RAGE', str(SK8V_ROOT / 'build/tools/rage.exe')))


def transform(point, origin):
    return tuple(a + b for a, b in zip(m0.from_skate(*point), origin))


def validate_geometry(vertices):
    if any(not math.isfinite(float(v)) for p in vertices for v in p):
        raise ValueError('NO_UNDERWATER: non-finite geometry')
    report = m0.validate_no_underwater('NO_UNDERWATER', [p[2] for p in vertices])
    # Packing uses the strict limit, even though M0 permits a rounding tolerance.
    if min(p[2] for p in vertices) < 20.0:
        raise ValueError('NO_UNDERWATER: min_z < 20.0')
    return report


def chunk_mesh(vertices, faces, normals=None, uvs=None, limit=MAX_VERTICES):
    """Remap whole triangles; each drawable has at most 60,000 unique vertices."""
    if not 3 <= limit <= MAX_VERTICES:
        raise ValueError('chunk limit must be between 3 and 60000')
    mapping, rows, indices = {}, [], []
    for face in faces:
        if len(face) != 3:
            raise ValueError('expected triangle faces')
        face = tuple(int(i) for i in face)
        if any(i < 0 or i >= len(vertices) for i in face):
            raise ValueError('triangle index outside vertex buffer')
        if len(rows) + len(set(face) - mapping.keys()) > limit:
            yield rows, indices
            mapping, rows, indices = {}, [], []
        if normals is None:
            a, b, c = (vertices[i] for i in face)
            ab, ac = [b[k] - a[k] for k in range(3)], [c[k] - a[k] for k in range(3)]
            n = (ab[1]*ac[2]-ab[2]*ac[1], ab[2]*ac[0]-ab[0]*ac[2], ab[0]*ac[1]-ab[1]*ac[0])
            length = math.sqrt(sum(x*x for x in n))
            n = tuple(x / length for x in n) if length else (0., 0., 1.)
        for i in face:
            if i not in mapping:
                mapping[i] = len(rows)
                normal = normals[i] if normals is not None else n
                uv = uvs[i] if uvs is not None else (0., 0.)
                row = (*vertices[i], *normal, *uv)
                if len(row) != 8 or not all(math.isfinite(float(v)) for v in row):
                    raise ValueError('invalid vertex attributes')
                rows.append(row)
            indices.append(mapping[i])
    if rows:
        yield rows, indices


def sub(root, tag, text=None, **attrs):
    node = ET.SubElement(root, tag, {k: str(v) for k, v in attrs.items()})
    if text is not None:
        node.text = str(text)
    return node


def vector(root, tag, values):
    return sub(root, tag, **dict(zip('xyz', values)))


def write_xml(path, root):
    path.parent.mkdir(parents=True, exist_ok=True)
    ET.indent(root)
    ET.ElementTree(root).write(path, encoding='utf-8', xml_declaration=True)


def drawable_xml(name, rows, indices, texture):
    if not rows or len(rows) > MAX_VERTICES or not indices or len(indices) % 3:
        raise ValueError('invalid drawable chunk')
    if any(i < 0 or i >= len(rows) for i in indices):
        raise ValueError('invalid drawable indices')
    lo = tuple(min(r[k] for r in rows) for k in range(3))
    hi = tuple(max(r[k] for r in rows) for k in range(3))
    center = tuple((a+b)/2 for a, b in zip(lo, hi))
    radius = math.dist(lo, hi)/2
    root = ET.Element('Drawable')
    sub(root, 'Name', name)
    vector(root, 'BoundingSphereCenter', center)
    sub(root, 'BoundingSphereRadius', value=radius)
    vector(root, 'BoundingBoxMin', lo)
    vector(root, 'BoundingBoxMax', hi)
    for lod in ('High', 'Med', 'Low', 'Vlow'):
        sub(root, 'LodDist'+lod, value=1200)
        sub(root, 'Flags'+lod, value=1 if lod == 'High' else 0)
    group = sub(root, 'ShaderGroup')
    sub(group, 'TextureDictionary')
    shader = sub(sub(group, 'Shaders'), 'Item')
    sub(shader, 'Name', 'default')
    sub(shader, 'FileName', 'default.sps')
    sub(shader, 'RenderBucket', value=0)
    params = sub(shader, 'Parameters')
    sub(sub(params, 'Item', name='DiffuseSampler', type='Texture'), 'Name', texture)
    model = sub(sub(root, 'DrawableModelsHigh'), 'Item')
    for tag, value in [('RenderMask', 255), ('Flags', 0), ('HasSkin', 0), ('BoneIndex', 0), ('Unknown1', 0)]:
        sub(model, tag, value=value)
    geom = sub(sub(model, 'Geometries'), 'Item')
    sub(geom, 'ShaderIndex', value=0)
    vector(geom, 'BoundingBoxMin', lo)
    vector(geom, 'BoundingBoxMax', hi)
    vb = sub(geom, 'VertexBuffer')
    sub(vb, 'Flags', value=0)
    layout = sub(vb, 'Layout', type='GTAV1')
    for tag in ('Position', 'Normal', 'Colour0', 'TexCoord0'):
        sub(layout, tag)
    sub(vb, 'Data', '\n' + '\n'.join(
        ' '.join(format(float(v), '.9g') for v in r[:6]) + ' 255 255 255 255 ' +
        ' '.join(format(float(v), '.9g') for v in r[6:]) for r in rows) + '\n')
    sub(sub(geom, 'IndexBuffer'), 'Data', ' '.join(map(str, indices)))
    return root


def joaat(name):
    value = 0
    for byte in name.lower().encode('ascii'):
        value = (value + byte) & 0xFFFFFFFF
        value = (value + (value << 10)) & 0xFFFFFFFF
        value ^= value >> 6
    value = (value + (value << 3)) & 0xFFFFFFFF
    value ^= value >> 11
    return (value + (value << 15)) & 0xFFFFFFFF


def menyoo_xml(placements):
    root = ET.Element('SpoonerPlacements')
    for name, position in placements:
        item = sub(root, 'Placement')
        sub(item, 'ModelHash', f'0x{joaat(name):08x}')
        sub(item, 'HashName', name)
        sub(item, 'Type', 3)
        sub(item, 'Dynamic', 'false')
        sub(item, 'FrozenPos', 'true')
        sub(item, 'LodDistance', 1200)
        pr = sub(item, 'PositionRotation')
        for tag, value in zip(('X', 'Y', 'Z', 'Pitch', 'Roll', 'Yaw'), (*position, 0, 0, 0)):
            sub(pr, tag, value)
    return root


def dlc_metadata(staging):
    device = 'dlc_skate_school'
    archive = f'{device}:/%PLATFORM%/levels/gta5/skateschool.rpf'
    typ = f'{device}:/%PLATFORM%/levels/gta5/skateschool.ityp'
    root = ET.Element('CDataFileMgr__ContentsOfDataFileXml')
    sub(root, 'disabledFiles')
    sub(root, 'includedXmlFiles')
    sub(root, 'includedDataFiles')
    files = sub(root, 'dataFiles')
    for path, kind, disabled, persistent in [
        (archive, 'RPF_FILE', 'true', 'false'),
        (typ, 'DLC_ITYP_REQUEST', 'true', 'true'),
    ]:
        item = sub(files, 'Item')
        sub(item, 'filename', path)
        sub(item, 'fileType', kind)
        if kind == 'RPF_FILE':
            sub(item, 'contents', 'CONTENTS_DLC_MAP_DATA')
        sub(item, 'overlay', value='false')
        sub(item, 'disabled', value=disabled)
        sub(item, 'persistent', value=persistent)
    changesets = sub(root, 'contentChangeSets')
    init_cs = sub(changesets, 'Item')
    sub(init_cs, 'changeSetName', 'CCS_SKATE_SCHOOL_INIT')
    for tag in ('mapChangeSetData', 'filesToEnable', 'filesToDisable', 'txdToLoad',
                'txdToUnload', 'residentResources', 'unregisterResources'):
        sub(init_cs, tag)
    stream_cs = sub(changesets, 'Item')
    sub(stream_cs, 'changeSetName', 'CCS_SKATE_SCHOOL_STREAMING_MAP')
    sub(stream_cs, 'mapChangeSetData')
    enable = sub(stream_cs, 'filesToEnable')
    for path in (archive, typ):
        sub(enable, 'Item', path)
    for tag in ('filesToDisable', 'txdToLoad', 'txdToUnload', 'residentResources', 'unregisterResources'):
        sub(stream_cs, tag)
    conds = sub(stream_cs, 'executionConditions')
    sub(conds, 'activeChangesetConditions')
    sub(conds, 'genericConditions', '$level=MO_JIM_L11')
    write_xml(staging / 'content.xml', root)
    root = ET.Element('SSetupData')
    for tag, value in [('deviceName', device), ('datFile', 'content.xml'), ('nameHash', 'skate_school')]:
        sub(root, tag, value)
    for tag, value in [('type', 'EXTRACONTENT_LEVEL_PACK'), ('order', 40), ('minorOrder', 0)]:
        sub(root, tag, value=value) if tag != 'type' else sub(root, tag, value)
    groups = sub(root, 'contentChangeSetGroups')
    for group_name, cs_name in [
        ('GROUP_EARLY_ON', 'CCS_SKATE_SCHOOL_INIT'),
        ('GROUP_UPDATE_STREAMING', 'CCS_SKATE_SCHOOL_STREAMING_MAP'),
    ]:
        group = sub(groups, 'Item')
        sub(group, 'NameHash', group_name)
        sub(sub(group, 'ContentChangeSets'), 'Item', cs_name)
    sub(root, 'isLevelPack', value='true')
    sub(root, 'dependencyPackHash')
    sub(root, 'requiredVersion')
    write_xml(staging / 'setup2.xml', root)


def write_dds(path, width, height, rgba):
    """Lossless legacy RGBA DDS, understood by rage textures build; no encoder required."""
    if width <= 0 or height <= 0 or len(rgba) != width * height * 4:
        raise ValueError('invalid decoded RGBA texture')
    header = [124, 0x100F, height, width, width*4, 0, 0, *([0]*11),
              32, 0x41, 0, 32, 0xFF, 0xFF00, 0xFF0000, 0xFF000000,
              0x1000, 0, 0, 0, 0]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b'DDS ' + struct.pack('<31I', *header) + bytes(rgba))


def build_commands(rage, out, drawables):
    staging, stream = out / 'staging', out / 'resources'
    commands = [[rage, 'textures', 'build', out / 'textures', '-o', stream / 'skateschool.ytd']]
    for xml in drawables:
        commands.append([rage, 'resource', 'build', xml, '-o', stream / xml.name.removesuffix('.xml')])
    commands.extend([
        [rage, 'ytyp', 'from-drawables', stream, '-o', stream / 'skateschool.ytyp', '--flags', '131104', '--txd', 'skateschool', '--lod-dist', '1200'],
        [rage, 'ymap', 'from-menyoo', out / 'skateschool.menyoo.xml', '-o', stream / 'skateschool.ymap', '--ytyp', stream / 'skateschool.ytyp', '--lod-dist', '1200'],
        [rage, 'manifest', 'generate', stream, '-o', stream / '_manifest.ymf'],
        [rage, 'create', '-e', 'open', '-o', staging / 'x64/levels/gta5/skateschool.rpf', stream],
        [rage, 'create', '-e', 'open', '-o', out / 'dlc.rpf', staging],
    ])
    return [[str(arg) for arg in command] for command in commands]


def district_origin(entry):
    name = entry['district']
    origin = tuple(float(v) for v in entry['island_origin_gta'])
    if len(origin) != 3 or not all(math.isfinite(v) for v in origin):
        raise ValueError(f'{name}: invalid M0 origin')
    expected = m0.compute_safe_island_origin(entry['skate3_y_bounds'][0], m0.ISLAND_ANCHORS_XY[name])
    if any(abs(a-b) > 0.002 for a, b in zip(origin, expected)):
        raise ValueError(f'{name}: M0 origin does not match safe island placement')
    return origin


def load_donor(donor):
    vanilla = donor / 'tools/vendor/university/tools/vanilla_map_extraction/tools'
    for path in (donor, vanilla, donor / 'tools/vendor/utt'):
        sys.path.insert(0, str(path))
    from tools.owned_game.big import BigArchive
    from skate3_streams import load_district_stream, ASSET_TYPE_MODEL, ASSET_TYPE_TEXTURE
    import rx2_parser
    from mdl_parser import parse_rx2
    return BigArchive, load_district_stream, ASSET_TYPE_MODEL, ASSET_TYPE_TEXTURE, rx2_parser, parse_rx2


def export(args):
    manifest = json.loads(args.manifest.read_text(encoding='utf-8'))
    entries = {e['district']: e for e in manifest['districts']}
    origins = {name: district_origin(entries[name]) for name in m0.TUTORIAL_BIG_NAME}
    donor = m0.resolve_donor_root(args.donor)
    BigArchive, load_stream, MODEL, TEXTURE, rx2, parse_model = load_donor(donor)
    out = args.out.resolve()
    # Avoid accidentally packaging stale resources from a previous export.
    if out.exists() and any(out.iterdir()):
        raise ValueError(f'output must be empty: {out}')
    for folder in ('xml', 'textures', 'resources', 'staging/x64/levels/gta5'):
        (out / folder).mkdir(parents=True, exist_ok=True)
    write_dds(out / 'textures/school_white.dds', 1, 1, b'\xff'*4)
    placements, drawables, reports = [], [], []
    for district, relative in m0.TUTORIAL_BIG_NAME.items():
        archive = BigArchive(args.disc / relative)
        with tempfile.TemporaryDirectory() as tmp:
            for entry in archive.entries:
                path = Path(tmp) / Path(entry.path).name
                if path.exists():
                    raise ValueError(f'{district}: duplicate stream filename {path.name}')
                path.write_bytes(archive.read(entry))
            assets = load_stream(Path(tmp), 'Pres', district)
        textures, models = {}, []
        tag = district.removeprefix('DIST_').lower()
        for asset in assets:
            if not m0.is_english_only_path(str(asset.source_path)):
                continue
            if asset.record.asset_type == TEXTURE:
                parsed = rx2.parse_rx2(asset.data)
                if not parsed.textures:
                    raise ValueError(f'{district}: texture {asset.record.asset_id:x} decoded empty')
                match = re.search(rb'(0x[0-9a-fA-F]{16})\.Texture', asset.data)
                key = match.group(1).decode().lower() if match else f'0x{asset.record.asset_id:016x}'
                names = []
                for i, tex in enumerate(parsed.textures):
                    name = f'school_{tag}_{key}_{i}'
                    write_dds(out / 'textures' / (name+'.dds'), tex.width, tex.height, tex.rgba)
                    names.append(name)
                textures[key] = names[0]
            elif asset.record.asset_type == MODEL:
                models.append(asset)
        district_count = 0
        for asset in models:
            model = parse_model(asset.data, strict=True)
            if not model.meshes:
                raise ValueError(f'{district}: model {asset.record.asset_id:x} decoded empty')
            for mi, mesh in enumerate(model.meshes):
                if not m0.is_english_only_path(mesh.name):
                    continue
                world = [transform(p, origins[district]) for p in mesh.vertices]
                report = validate_geometry(world)
                report.update(district=district, asset=f'{asset.record.asset_id:x}', mesh=mi)
                reports.append(report)
                normals = [m0.from_skate(*n) for n in mesh.normals] if mesh.normals is not None else None
                match = re.search(r'0x[0-9a-fA-F]{16}$', mesh.material_name or '')
                texture = textures.get(match.group().lower(), 'school_white') if match else 'school_white'
                for ci, (rows, indices) in enumerate(chunk_mesh(world, mesh.faces, normals, mesh.uvs)):
                    name = f'school_{district.removeprefix("DIST_").lower()}_{asset.record.asset_id:x}_{mi}_{ci}'
                    # Keep local Z nonnegative: float32 drawable quantization cannot
                    # round a negative local offset below the validated waterline.
                    center = (*( (min(r[k] for r in rows)+max(r[k] for r in rows))/2
                                 for k in range(2)), min(r[2] for r in rows))
                    local = [(*(r[k]-center[k] for k in range(3)), *r[3:]) for r in rows]
                    xml = out / 'xml' / (name+'.ydr.xml')
                    write_xml(xml, drawable_xml(name, local, indices, texture))
                    drawables.append(xml)
                    placements.append((name, center))
                    district_count += 1
        if not district_count:
            raise ValueError(f'{district}: no drawable chunks exported')
    write_xml(out / 'skateschool.menyoo.xml', menyoo_xml(placements))
    dlc_metadata(out / 'staging')
    commands = build_commands(args.rage, out, drawables)
    result = {'NO_UNDERWATER': True, 'min_z': min(r['min_z'] for r in reports),
              'chunks': len(drawables), 'geometry': reports, 'commands': commands,
              'mode': 'xml-only' if args.xml_only else 'dry-run' if args.dry_run else 'ready'}
    report_path = out / 'build-report.json'
    report_path.write_text(json.dumps(result, indent=2, default=float), encoding='utf-8')
    if not (args.xml_only or args.dry_run):
        for command in commands:
            subprocess.run(command, check=True)
        result['mode'] = 'packed'
        report_path.write_text(json.dumps(result, indent=2, default=float), encoding='utf-8')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--disc', type=Path, required=True, help='extracted Skate 3 disc root')
    parser.add_argument('--manifest', type=Path, default=REPO / 'local/m0-tutorial/m0-tutorial-manifest.json')
    default_donor = Path(os.environ.get('SKATE3_DONOR_ROOT', str(SK8V_ROOT / 'upstream/skate-3-rust-engine-donor')))
    parser.add_argument('--donor', type=Path, default=default_donor)
    parser.add_argument('--rage', type=Path, default=DEFAULT_RAGE)
    parser.add_argument('--out', type=Path, default=REPO / 'local/m1-tutorial-dlc')
    parser.add_argument('--dry-run', action='store_true')
    parser.add_argument('--xml-only', action='store_true')
    args = parser.parse_args()
    try:
        result = export(args)
    except (OSError, ValueError, ImportError, subprocess.CalledProcessError) as exc:
        parser.exit(1, f'tutorial DLC: {exc}\n')
    print(f'{result["mode"]}: {result["chunks"]} chunks; NO_UNDERWATER min_z={result["min_z"]:.4f}')


if __name__ == '__main__':
    main()
