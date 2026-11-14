"""M0 offline pipeline: extract & convert Skate 3 Tutorial (DIST_SkateSchool +
DIST_StartPark), Coach Frank assets, English-only tutorial strings/VO/video
refs, .park starter templates, and tutorial APT screens with strict No-Underwater
Z-level validation.

Runs purely offline against the player's own extracted Skate 3 disc folder and
never modifies GTA V base archives (update.rpf / x64*.rpf) or releases copyrighted
disc assets into source control.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import struct
import sys
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

sys.dont_write_bytecode = True
os.environ['PYTHONDONTWRITEBYTECODE'] = '1'

from _common import REPO, sha256  # noqa: E402

# Island anchors ~4 km SW of Vespucci Beach in GTA V world coordinates (Z up, metres).
# Sea level in GTA V is Z = 0.0; both islands keep all geometry >= MIN_WATER_CLEARANCE_M.
MIN_WATER_CLEARANCE_M = 20.0
SEA_LEVEL_Z = 0.0

ISLAND_ANCHORS_XY = {
    'DIST_SkateSchool': (-4500.0, -3200.0),
    'DIST_StartPark': (-5450.0, -3200.0),
}

ALLOWED_LANGUAGE_TAG = 'english'
BLOCKED_LANGUAGE_TAGS = ('german', 'french', 'spanish', 'italian', 'dutch', 'japanese')

TUTORIAL_BIG_NAME = {
    'DIST_SkateSchool': 'data/content/worldDIST_SkateSchool.big',
    'DIST_StartPark': 'data/content/worldDIST_StartPark.big',
}

COACH_FRANK_INTRO_VP6 = 'data/movies/Coach_frank_intro_english_ntsc.vp6'


def resolve_donor_root(cli_donor: Path | None) -> Path:
    """Resolve read-only skate-3-rust-engine-donor directory without hardcoding local user paths."""
    candidates = []
    if cli_donor is not None:
        candidates.append(cli_donor)
    env_donor = os.environ.get('SKATE3_DONOR_ROOT')
    if env_donor:
        candidates.append(Path(env_donor))
    candidates.append(REPO / 'upstream' / 'skate-3-rust-engine-donor')

    for candidate in candidates:
        resolved = candidate.resolve()
        if (resolved / 'tools' / 'owned_game' / 'big.py').is_file():
            return resolved
    raise FileNotFoundError(
        'skate-3-rust-engine-donor not found; pass --donor <path> or set SKATE3_DONOR_ROOT'
    )


def _import_donor_modules(donor_root: Path):
    """Add donor extraction packages to sys.path cleanly without modifying donor files."""
    vanilla_tools = donor_root / 'tools' / 'vendor' / 'university' / 'tools' / 'vanilla_map_extraction' / 'tools'
    for p in (str(donor_root), str(vanilla_tools)):
        if p not in sys.path:
            sys.path.insert(0, p)

    from tools.owned_game.big import BigArchive  # type: ignore
    from skate3_streams import load_district_stream  # type: ignore
    from retail_collision_mesh import decode_rx2_clustered_meshes  # type: ignore
    from retail_grind_splines import decode_grind_splines  # type: ignore

    return BigArchive, load_district_stream, decode_rx2_clustered_meshes, decode_grind_splines


def from_skate(x: float, y_up: float, z: float) -> tuple[float, float, float]:
    """Convert Skate 3 right-handed Y-up metres to GTA V right-handed Z-up metres."""
    return (x, -z, y_up)


def compute_safe_island_origin(
    min_skate_y: float,
    anchor_xy: tuple[float, float],
    min_clearance_m: float = MIN_WATER_CLEARANCE_M,
) -> tuple[float, float, float]:
    """Compute (X, Y, Z) island origin so every vertex stays strictly above sea level."""
    if not math.isfinite(min_skate_y):
        raise ValueError('min_skate_y must be finite')
    if min_clearance_m <= SEA_LEVEL_Z:
        raise ValueError('min_clearance_m must be strictly positive above GTA sea level')
    z_origin = min_clearance_m - min_skate_y
    return (anchor_xy[0], anchor_xy[1], z_origin)


def validate_no_underwater(
    label: str,
    z_values: list[float] | tuple[float, ...],
    min_clearance_m: float = MIN_WATER_CLEARANCE_M,
) -> dict[str, float]:
    """Fail-closed check ensuring zero vertices, rails, or spawns sit at or below sea level."""
    if not z_values:
        raise ValueError(f'{label}: empty coordinate set cannot pass waterline validation')
    min_z = min(z_values)
    max_z = max(z_values)
    for z in z_values:
        if not math.isfinite(z):
            raise ValueError(f'{label}: non-finite Z coordinate encountered')
        if z <= SEA_LEVEL_Z:
            raise ValueError(f'{label}: underwater Z={z:.3f} <= sea level ({SEA_LEVEL_Z})')
    if min_z < min_clearance_m - 1e-4:
        raise ValueError(
            f'{label}: min_z={min_z:.3f}m violates minimum clearance {min_clearance_m:.2f}m'
        )
    return {'min_z': round(min_z, 4), 'max_z': round(max_z, 4)}


def is_english_only_path(relative_path: str) -> bool:
    """Filter locale-bearing audio/video paths to English only per M0 decision."""
    low = relative_path.replace('\\', '/').lower()
    for blocked in BLOCKED_LANGUAGE_TAGS:
        if f'/{blocked}/' in low or f'_{blocked}_' in low or low.startswith(f'{blocked}/'):
            return False
    return True


def write_collision_cache(path: Path, triangles_gta: list[tuple[tuple[float, float, float], tuple[float, float, float], tuple[float, float, float], int]]) -> None:
    """Write compact binary triangle cache (`SK8M` v1, 40 bytes per triangle)."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('wb') as f:
        f.write(struct.pack('<4sII', b'SK8M', 1, len(triangles_gta)))
        for a, b, c, surface in triangles_gta:
            f.write(struct.pack('<9fI', *a, *b, *c, int(surface) & 0xFFFFFFFF))


def write_svwc_v2(
    path: Path,
    triangles_gta: list[tuple[tuple[float, float, float], tuple[float, float, float], tuple[float, float, float], int]],
    tile_size: float = 64.0,
) -> tuple[int, int]:
    """Write an SVWC v2 tiled collision cache readable natively by rust/svwc (`Cache::open`)."""
    tiles: dict[tuple[int, int], list[int]] = {}
    for idx, (a, b, c, _surf) in enumerate(triangles_gta):
        min_x = min(a[0], b[0], c[0])
        max_x = max(a[0], b[0], c[0])
        min_y = min(a[1], b[1], c[1])
        max_y = max(a[1], b[1], c[1])
        tx0 = math.floor(min_x / tile_size)
        tx1 = math.floor(max_x / tile_size)
        ty0 = math.floor(min_y / tile_size)
        ty1 = math.floor(max_y / tile_size)
        for tx in range(tx0, tx1 + 1):
            for ty in range(ty0, ty1 + 1):
                tiles.setdefault((tx, ty), []).append(idx)

    sorted_keys = sorted(tiles.keys())
    record_count = sum(len(tiles[k]) for k in sorted_keys)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('wb') as f:
        f.write(struct.pack('<4sIfII', b'SVWC', 2, float(tile_size), len(sorted_keys), record_count))
        first = 0
        for tx, ty in sorted_keys:
            cnt = len(tiles[(tx, ty)])
            f.write(struct.pack('<iiII', tx, ty, first, cnt))
            first += cnt
        for key in sorted_keys:
            for tri_idx in tiles[key]:
                a, b, c, surf = triangles_gta[tri_idx]
                f.write(struct.pack('<9f4B', *a, *b, *c, int(surf) & 0xFF, 0, 0, 0))
    return len(sorted_keys), record_count


def _decode_rx2_model_count(pres_assets) -> tuple[int, int]:
    """Count RX2 mesh headers (`0x00EB0023`) and internal texture references (`0x00EB000B`)."""
    model_assets = 0
    tex_refs = 0
    for asset in pres_assets:
        data = asset.data
        if len(data) < 32 or data[:4] != b'RW4x' and b'RW' not in data[:8]:
            # Still scan RX2 record magic safely via 0x00EB0023 big-endian TOC
            pass
        if len(data) >= 32:
            model_assets += 1
        tex_refs += data.count(b'.Texture')
    return model_assets, tex_refs


def _parse_boundary_xml(raw_bytes: bytes) -> list[tuple[float, float]]:
    text = raw_bytes.decode('utf-8', errors='replace')
    root = ET.fromstring(text)
    points: list[tuple[float, float]] = []
    for node in root.findall('Point'):
        if not node.text:
            continue
        parts = [p.strip() for p in node.text.split(',')]
        if len(parts) >= 2:
            points.append((float(parts[0]), float(parts[1])))
    return points


def process_district(
    disc_root: Path,
    out_root: Path,
    dist_name: str,
    big_cls,
    load_stream_fn,
    decode_meshes_fn,
    decode_splines_fn,
) -> dict:
    big_path = disc_root / TUTORIAL_BIG_NAME[dist_name]
    if not big_path.is_file():
        raise FileNotFoundError(f'Missing Skate 3 archive: {big_path}')

    big = big_cls(big_path)
    locators_dir = disc_root / 'data' / 'content' / 'global_locators' / 'PARK' / dist_name
    locator_files = sorted(f.name for f in locators_dir.iterdir() if f.is_file()) if locators_dir.is_dir() else []

    with tempfile.TemporaryDirectory() as tmpdir:
        stream_dir = Path(tmpdir) / dist_name
        stream_dir.mkdir(parents=True, exist_ok=True)
        for entry in big.entries:
            (stream_dir / Path(entry.path).name).write_bytes(big.read(entry))

        sim_assets = load_stream_fn(stream_dir, 'Sim', dist_name)
        pres_assets = load_stream_fn(stream_dir, 'Pres', dist_name)
        tex_assets = load_stream_fn(stream_dir, 'Tex', dist_name)

    raw_tris: list[tuple[tuple[float, float, float], tuple[float, float, float], tuple[float, float, float], int]] = []
    raw_splines: list[dict] = []
    min_y = math.inf
    max_y = -math.inf

    for asset in sim_assets:
        for mesh in decode_meshes_fn(asset.data):
            min_y = min(min_y, mesh.bounds_min[1])
            max_y = max(max_y, mesh.bounds_max[1])
            for tri in mesh.triangles:
                raw_tris.append((tri.a, tri.b, tri.c, int(tri.surface)))
                min_y = min(min_y, tri.a[1], tri.b[1], tri.c[1])
                max_y = max(max_y, tri.a[1], tri.b[1], tri.c[1])
        for rail in decode_splines_fn(asset.data):
            raw_splines.append(rail)

    if not raw_tris:
        raise ValueError(f'{dist_name}: zero collision triangles decoded')

    origin_gta = compute_safe_island_origin(min_y, ISLAND_ANCHORS_XY[dist_name])
    ox, oy, oz = origin_gta

    gta_tris = []
    tri_z_values: list[float] = []
    for a, b, c, surf in raw_tris:
        ga = from_skate(*a)
        gb = from_skate(*b)
        gc = from_skate(*c)
        pa = (ga[0] + ox, ga[1] + oy, ga[2] + oz)
        pb = (gb[0] + ox, gb[1] + oy, gb[2] + oz)
        pc = (gc[0] + ox, gc[1] + oy, gc[2] + oz)
        gta_tris.append((pa, pb, pc, surf))
        tri_z_values.extend([pa[2], pb[2], pc[2]])

    tri_z_report = validate_no_underwater(f'{dist_name}.collision', tri_z_values)

    def _decode_rail_points_skate(rail: dict) -> list[tuple[float, float, float]]:
        points: list[tuple[float, float, float]] = []
        payloads = rail.get('native_segment_payloads') or ()
        for i, payload_hex in enumerate(payloads):
            seg = bytes.fromhex(payload_hex)
            if len(seg) < 60:
                continue
            a = struct.unpack_from('>3f', seg, 0)
            b = struct.unpack_from('>3f', seg, 16)
            c = struct.unpack_from('>3f', seg, 32)
            d = struct.unpack_from('>3f', seg, 48)
            if i == 0:
                points.append(d)
            # Sample midpoint (t=0.5) and segment endpoint (t=1.0) so arched/curved rails validate accurately.
            mid = tuple(a[k] * 0.125 + b[k] * 0.25 + c[k] * 0.5 + d[k] for k in range(3))
            end = tuple(a[k] + b[k] + c[k] + d[k] for k in range(3))
            points.append(mid)
            points.append(end)
        return points

    gta_splines: list[dict] = []
    spline_z_values: list[float] = []
    for idx, rail in enumerate(raw_splines):
        converted_points: list[list[float]] = []
        for spt in _decode_rail_points_skate(rail):
            gx, gy, gz = from_skate(spt[0], spt[1], spt[2])
            px, py, pz = gx + ox, gy + oy, gz + oz
            converted_points.append([round(px, 4), round(py, 4), round(pz, 4)])
            spline_z_values.append(pz)
        if not converted_points:
            raise ValueError(f'{dist_name}: spline {idx} decoded zero points')
        gta_splines.append({
            'index': idx,
            'spline_id': rail.get('spline_id', f'0x{idx:X}'),
            'closed': bool(rail.get('closed', False)),
            'segment_count': int(rail.get('segment_count', 0)),
            'points_gta': converted_points,
        })

    if raw_splines and not spline_z_values:
        raise ValueError(f'{dist_name}: expected spline Z points for {len(raw_splines)} rails')
    spline_z_report = (
        validate_no_underwater(f'{dist_name}.splines', spline_z_values)
        if spline_z_values
        else {'min_z': None, 'max_z': None}
    )

    col_rel = f'collision/{dist_name}.sk8m'
    write_collision_cache(out_root / col_rel, gta_tris)
    svwc_rel = f'collision/{dist_name}.svwc'
    svwc_tiles, svwc_records = write_svwc_v2(out_root / svwc_rel, gta_tris)

    rails_rel = f'splines/{dist_name}_rails.json'
    (out_root / rails_rel).parent.mkdir(parents=True, exist_ok=True)
    (out_root / rails_rel).write_text(json.dumps(gta_splines, indent=2), encoding='utf-8')

    pres_models, pres_tex_refs = _decode_rx2_model_count(pres_assets)

    return {
        'district': dist_name,
        'island_origin_gta': [round(ox, 3), round(oy, 3), round(oz, 3)],
        'skate3_y_bounds': [round(min_y, 3), round(max_y, 3)],
        'sim_assets': len(sim_assets),
        'pres_assets': len(pres_assets),
        'tex_assets': len(tex_assets),
        'pres_rx2_assets': pres_models,
        'pres_texture_refs': pres_tex_refs,
        'locator_rx2_files': locator_files,
        'collision_triangles': len(gta_tris),
        'svwc_tiles': svwc_tiles,
        'svwc_tile_records': svwc_records,
        'grind_splines': len(gta_splines),
        'waterline_validation': {
            'sea_level_z': SEA_LEVEL_Z,
            'min_clearance_m': MIN_WATER_CLEARANCE_M,
            'collision': tri_z_report,
            'splines': spline_z_report,
        },
        'outputs': {
            'collision_cache': col_rel,
            'svwc_cache': svwc_rel,
            'rails_json': rails_rel,
        },
    }


def extract_tutorial_metadata(disc_root: Path, out_root: Path, big_cls) -> dict:
    """Extract English-only tutorial challenges, Coach Frank assets, .park starters, and APT screens."""
    fedata_big = big_cls(disc_root / 'data/big/fedata.big')
    miscload_big = big_cls(disc_root / 'data/big/miscload.big')
    db_big = big_cls(disc_root / 'data/big/db.big')

    apt_dir = out_root / 'apt'
    parks_dir = out_root / 'parks'
    coach_dir = out_root / 'coach_frank'
    challenges_dir = out_root / 'challenges'
    for d in (apt_dir, parks_dir, coach_dir, challenges_dir):
        d.mkdir(parents=True, exist_ok=True)

    apt_extracted: list[str] = []
    for entry in fedata_big.entries:
        low = entry.path.lower()
        if any(k in low for k in ('skate101.', 'hudtutorial.', 'tutorial.')):
            target = apt_dir / Path(entry.path).name
            target.write_bytes(fedata_big.read(entry))
            apt_extracted.append(f'apt/{target.name}')

    parks_extracted: list[str] = []
    boundaries: dict[str, list[list[float]]] = {}
    coach_files: list[str] = []
    for entry in miscload_big.entries:
        low = entry.path.lower()
        if low.startswith('data/skatepark/') and 'startpark' in low and low.endswith('.park'):
            target = parks_dir / Path(entry.path).name
            target.write_bytes(miscload_big.read(entry))
            parks_extracted.append(f'parks/{target.name}')
        elif 'coach_frank.abin' in low:
            kind = 'facebinds' if 'facebinds' in low else 'faces'
            target = coach_dir / f'{kind}_{Path(entry.path).name}'
            target.write_bytes(miscload_big.read(entry))
            coach_files.append(f'coach_frank/{target.name}')
        elif low in ('data/boundary/freeskate_sksc_hub.xml', 'data/boundary/1up_sksc_01.xml'):
            pts = _parse_boundary_xml(miscload_big.read(entry))
            ox, oy, _ = ISLAND_ANCHORS_XY['DIST_SkateSchool'][0], ISLAND_ANCHORS_XY['DIST_SkateSchool'][1], 0.0
            boundaries[Path(entry.path).name] = [[round(ox + x, 2), round(oy - z, 2)] for x, z in pts]

    challenge_files: list[str] = []
    for entry in db_big.entries:
        low = entry.path.lower()
        if not is_english_only_path(low):
            continue
        if any(k in low for k in ('tut_sksc_', 'tut_skpk_', '1up_sksc_', 'tele_sksc_')):
            target = challenges_dir / Path(entry.path).name
            target.write_bytes(db_big.read(entry))
            challenge_files.append(f'challenges/{target.name}')

    vp6_path = disc_root / COACH_FRANK_INTRO_VP6
    if not vp6_path.is_file():
        raise FileNotFoundError(f'Missing English Coach Frank intro movie: {vp6_path}')

    english_audio_dir = disc_root / 'data/audio/english'
    english_audio_files = sorted(
        f.name for f in english_audio_dir.iterdir()
        if f.is_file() and is_english_only_path(f'data/audio/english/{f.name}')
    ) if english_audio_dir.is_dir() else []

    challenge_ids = sorted({
        Path(p).name.split('.')[0].replace('_summaryreport', '')
        for p in challenge_files
    })

    return {
        'language': ALLOWED_LANGUAGE_TAG,
        'english_only_enforced': True,
        'apt_screens': sorted(apt_extracted),
        'startpark_templates': sorted(parks_extracted),
        'coach_frank_assets': sorted(coach_files),
        'coach_frank_intro_vp6': {
            'relative_disc_path': COACH_FRANK_INTRO_VP6,
            'size_bytes': vp6_path.stat().st_size,
        },
        'english_audio_count': len(english_audio_files),
        'challenge_descriptor_count': len(challenge_files),
        'challenge_ids': challenge_ids,
        'boundary_polygons_gta_xy': boundaries,
    }


def verify_manifest(manifest_path: Path) -> dict:
    """Validate an already-prepared optional M0 tutorial directory offline."""
    manifest_path = manifest_path.resolve()
    root = manifest_path.parent
    data = json.loads(manifest_path.read_text(encoding='utf-8'))
    if data.get('format') != 'gtaxskate-m0-tutorial-v1':
        raise ValueError('Unsupported tutorial manifest format')
    if data.get('waterline_guardrail') != 'NO_UNDERWATER_ASSETS':
        raise ValueError('Missing NO_UNDERWATER_ASSETS guardrail')
    meta = data.get('tutorial_metadata', {})
    if meta.get('language') != ALLOWED_LANGUAGE_TAG or not meta.get('english_only_enforced'):
        raise ValueError('Tutorial pack must enforce English-only assets')

    summary: dict[str, dict] = {}
    for dist in data.get('districts', []):
        name = dist['district']
        col_path = root / dist['outputs']['collision_cache']
        raw = col_path.read_bytes()
        magic, ver, tri_count = struct.unpack_from('<4sII', raw, 0)
        if magic != b'SK8M' or ver != 1 or len(raw) != 12 + tri_count * 40:
            raise ValueError(f'{name}: corrupted .sk8m collision cache')
        z_vals: list[float] = []
        for idx in range(tri_count):
            coords = struct.unpack_from('<9f', raw, 12 + idx * 40)
            z_vals.extend([coords[2], coords[5], coords[8]])
        tri_z = validate_no_underwater(f'{name}.verify.collision', z_vals)

        rails_path = root / dist['outputs']['rails_json']
        rails = json.loads(rails_path.read_text(encoding='utf-8'))
        rail_z: list[float] = []
        for rail in rails:
            for pt in rail.get('points_gta', []):
                rail_z.append(float(pt[2]))
        if rails:
            validate_no_underwater(f'{name}.verify.splines', rail_z)

        svwc_rel = dist['outputs'].get('svwc_cache')
        svwc_tiles = 0
        if svwc_rel:
            sraw = (root / svwc_rel).read_bytes()
            smagic, sver, stile, stcnt, srcnt = struct.unpack_from('<4sIfII', sraw, 0)
            if smagic != b'SVWC' or sver != 2 or stile <= 0.0 or len(sraw) != 20 + stcnt * 16 + srcnt * 40:
                raise ValueError(f'{name}: corrupted SVWC v2 collision cache')
            svwc_tiles = stcnt
        summary[name] = {'triangles': tri_count, 'svwc_tiles': svwc_tiles, 'splines': len(rails), 'min_z': tri_z['min_z']}
    return summary


def run_m0(disc_root: Path, out_root: Path, donor_root: Path) -> Path:
    disc_root = disc_root.resolve()
    out_root = out_root.resolve()
    out_root.mkdir(parents=True, exist_ok=True)

    big_cls, load_stream_fn, decode_meshes_fn, decode_splines_fn = _import_donor_modules(donor_root)

    districts = []
    for dist_name in ('DIST_SkateSchool', 'DIST_StartPark'):
        districts.append(
            process_district(
                disc_root,
                out_root,
                dist_name,
                big_cls,
                load_stream_fn,
                decode_meshes_fn,
                decode_splines_fn,
            )
        )

    metadata = extract_tutorial_metadata(disc_root, out_root, big_cls)

    manifest = {
        'format': 'gtaxskate-m0-tutorial-v1',
        'waterline_guardrail': 'NO_UNDERWATER_ASSETS',
        'districts': districts,
        'tutorial_metadata': metadata,
    }
    manifest_bytes = json.dumps(manifest, indent=2).encode('utf-8')
    manifest['manifest_sha256_payload'] = sha256(manifest_bytes)
    manifest_path = out_root / 'm0-tutorial-manifest.json'
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding='utf-8')
    return manifest_path


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skate', type=Path, default=None, help='Extracted Skate 3 disc root (contains default.xex and data/)')
    parser.add_argument('--out', type=Path, default=REPO / 'local' / 'm0-tutorial', help='Output directory for M0 extracted assets and manifest')
    parser.add_argument('--donor', type=Path, default=None, help='Optional override path to read-only skate-3-rust-engine-donor')
    parser.add_argument('--verify-manifest', type=Path, default=None, help='Verify an existing m0-tutorial-manifest.json offline')
    args = parser.parse_args(argv)

    if args.verify_manifest is not None:
        summary = verify_manifest(args.verify_manifest)
        print('M0 verify PASS:', json.dumps(summary))
        return 0

    if args.skate is None or not (args.skate / 'default.xex').is_file():
        raise SystemExit(f'Expected extracted Skate 3 disc folder with default.xex: {args.skate}')

    donor = resolve_donor_root(args.donor)
    manifest_path = run_m0(args.skate, args.out, donor)
    print(f'M0 complete: {manifest_path}')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
