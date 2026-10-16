"""Stage the missing model skin render bit in existing owned board YDRs.

Never writes source assets or installs anything. Existing geometry, skeleton,
materials, other flags and embedded DDS data are verified after rebuilding.
"""
import argparse
import concurrent.futures
import hashlib
import json
import math
from pathlib import Path
import subprocess
import tempfile
import xml.etree.ElementTree as ET


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def same_text(a, b):
    a, b = (a or '').split(), (b or '').split()
    if len(a) != len(b):
        return False
    for x, y in zip(a, b):
        if x == y:
            continue
        try:
            xf, yf = float(x), float(y)
            if not math.isfinite(xf) or not math.isfinite(yf) or not math.isclose(xf, yf, rel_tol=1e-6, abs_tol=1e-7):
                return False
        except ValueError:
            return False
    return True


def same_tree(a, b):
    return (a.tag == b.tag and a.attrib.keys() == b.attrib.keys()
            and all(same_text(a.attrib[k], b.attrib[k]) for k in a.attrib)
            and same_text(a.text, b.text) and len(a) == len(b)
            and all(same_tree(x, y) for x, y in zip(a, b)))


def repair_flags(root):
    count = 0
    for level in ('High', 'Med', 'Low', 'Vlow'):
        for model in root.findall('./DrawableModels' + level + '/Item'):
            skin = model.find('HasSkin')
            if skin is not None and int(skin.get('value')) != 0:
                flags = model.find('Flags')
                if flags is None:
                    raise ValueError('Skinned model without Flags')
                value = int(flags.get('value'))
                if not value & 1:
                    flags.set('value', str(value | 1))
                    count += 1
    return count


def run(rage, *args):
    result = subprocess.run([str(rage), '--no-update-check', 'resource', *map(str, args)],
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if result.returncode:
        raise RuntimeError(result.stderr[-2000:] or result.stdout[-2000:])


def stage_one(source, assets, out, rage):
    relative = source.relative_to(assets)
    before = sha(source)
    with tempfile.TemporaryDirectory(prefix='skin-', dir=out / 'work') as temp:
        temp = Path(temp)
        original, check = temp / 'original', temp / 'check'
        original.mkdir()
        check.mkdir()
        xml = original / 'model.ydr.xml'
        run(rage, 'dump', source, '-o', xml)
        tree = ET.parse(xml)
        count = repair_flags(tree.getroot())
        if not count:
            return {'name': relative.as_posix(), 'changedModels': 0, 'originalSha256': before, 'sha256': before}
        tree.write(xml, encoding='utf-8', xml_declaration=True)
        target = out / 'payload' / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        run(rage, 'build', xml, '-o', target, '--strict')
        checked_xml = check / 'model.ydr.xml'
        run(rage, 'dump', target, '-o', checked_xml)
        if not same_tree(tree.getroot(), ET.parse(checked_xml).getroot()):
            raise ValueError(f'{relative}: non-flag XML change after rebuild')
        textures = lambda folder: {p.relative_to(folder).as_posix(): sha(p) for p in folder.rglob('*.dds')}
        if textures(original) != textures(check):
            raise ValueError(f'{relative}: embedded texture data changed')
        if sha(source) != before:
            raise ValueError(f'{relative}: source changed during staging')
        return {'name': relative.as_posix(), 'changedModels': count, 'originalSha256': before, 'sha256': sha(target)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--rage', required=True, type=Path)
    parser.add_argument('--only', nargs='*', help='Specific asset-relative YDR paths for isolated verification')
    parser.add_argument('--workers', type=int, default=4)
    args = parser.parse_args()
    assets, out, rage = args.assets.resolve(), args.out.resolve(), args.rage.resolve()
    if out.exists() or out == assets or assets in out.parents:
        raise ValueError('Use a new output folder outside source assets')
    sources = ([assets / name for name in args.only] if args.only else
               sorted((assets / 'private' / 'board-native').glob('skatev_board.ydr')) +
               sorted((assets / 'private' / 'board-parts').rglob('skatev_*.ydr')))
    if not sources:
        raise ValueError('No existing board assets found')
    for path in sources:
        resolved = path.resolve()
        relative = resolved.relative_to(assets)
        if (not path.is_file() or path.is_symlink() or relative.parts[:2] not in
                (('private', 'board-native'), ('private', 'board-parts')) or path.suffix != '.ydr'):
            raise ValueError('Unexpected source asset path')
    (out / 'work').mkdir(parents=True)
    # Create shared ancestors once on the main thread. Concurrent recursive
    # mkdir on Windows can fail transiently while siblings create the parent.
    for directory in sorted({(out / 'payload' / p.relative_to(assets)).parent for p in sources}):
        directory.mkdir(parents=True, exist_ok=True)
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, min(args.workers, 8))) as pool:
        jobs = [pool.submit(stage_one, p, assets, out, rage) for p in sources]
        files = [job.result() for job in jobs]
    manifest = {'format': 'gtaxskate-board-skin-fix', 'version': 1, 'assets': str(assets),
                'rageSha256': sha(rage), 'files': files,
                'checked': len(files), 'changed': sum(bool(e['changedModels']) for e in files),
                'verification': 'All XML fields semantically equal except requested skin bit; embedded DDS hashes exact; source unchanged',
                'gameplay': 'Not tested; corrects contradictory Flags=0 / HasSkin=1 metadata'}
    (out / 'manifest.json').write_text(json.dumps(manifest, indent=2), encoding='utf-8')
    print(json.dumps({'output': str(out), 'checked': manifest['checked'], 'changed': manifest['changed']}))


if __name__ == '__main__':
    main()
