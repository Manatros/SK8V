"""Skate 3's replay editor texts for the replay (design 2026-11-09-skate-replay-editor).

Reads the English language table from the player's own Skate 3 copy
(`data/big/fedata.big`) and writes every ID_REPLAY* entry as
`<assets>/private/replay-text.txt`: one `ID<TAB>text` per line, UTF-8, Skate's
`^` line breaks kept. The host loads it once; without it the replay uses its
own short English. Nothing is shipped: the file is made at setup.

    prepare-replay-text.py --game <extracted Skate 3 folder> --assets <out>/assets
"""
import argparse
import json
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / 'upstream' / 'skate-3-rust-engine-donor' / 'tools'))
from vendor.skate3_ui.project import extract_project  # noqa: E402

PREFIXES = ('ID_REPLAY_EDITOR', 'ID_REPLAYEDITOR')


def texts(english_json: Path) -> dict:
    data = json.loads(english_json.read_text(encoding='utf-8'))
    out = {}
    for e in data['entries']:
        label, value = e.get('label') or '', e.get('value') or ''
        if label.startswith(PREFIXES) and value:
            # Skate's own bytes are Latin-1; tabs and newlines would break the line format.
            out[label] = value.replace('\t', ' ').replace('\r', '').replace('\n', '^')
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--game', type=Path, required=True)
    ap.add_argument('--assets', type=Path, required=True)
    args = ap.parse_args()
    with tempfile.TemporaryDirectory(prefix='svreplaytext-') as tmp:
        work = Path(tmp)
        extract_project(args.game.resolve(), work, prefixes=('data/fe/source/screens/replay2',), update=True)
        english = work / 'metadata' / 'languages' / 'english_global.json'
        if not english.exists():
            sys.exit('the English language table was not found in fedata.big')
        found = texts(english)
    if len(found) < 20:
        sys.exit(f'only {len(found)} replay editor texts found')
    target = args.assets.resolve() / 'private' / 'replay-text.txt'
    target.parent.mkdir(parents=True, exist_ok=True)
    tmp_target = target.with_suffix('.tmp')
    tmp_target.write_text(''.join(f'{k}\t{v}\n' for k, v in sorted(found.items())), encoding='utf-8')
    tmp_target.replace(target)
    print(f'replay editor texts ready: {len(found)} in {target}')


if __name__ == '__main__':
    main()
