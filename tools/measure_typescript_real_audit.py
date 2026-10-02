"""Rerun the frozen 100-call TypeScript audit against fresh, pinned sources.

Development-only acquisition is strictly offline. Observations never overwrite
earlier results. The labels, scorer, inventories, and 900-second command limit
are unchanged from the published baseline.
"""
from __future__ import annotations

import argparse
import gzip
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import time

from tools import dispatch_audit_scorer_typescript as scorer
from tools.core_representative_benchmark import acquire_artifact, source_inventory, verify_inventory
from tools.dispatch_audit_reconciliation import BASE, acceptance, sha, write
from tools.dispatch_audit_site_selector_typescript import extract_archive_subset
from tools.prepare_typescript_audit_extension import CACHE, MANIFEST, OUT, ORIGINAL


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--name', required=True)
    parser.add_argument('--binary', type=Path, default=Path('/mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder'))
    args = parser.parse_args()
    if not args.name.replace('-', '').isalnum():
        raise ValueError('name must contain letters, digits, and hyphens only')
    dest = OUT.parent / 'lexical-bindings' / args.name
    work = Path('/tmp') / f'girder-ts-real-audit-{args.name}'
    dest.mkdir(exist_ok=False)
    work.mkdir(exist_ok=False)
    raw = work / 'raw'
    raw.mkdir()
    frozen = json.loads((OUT / 'freeze-manifest.json').read_text())
    labels = json.loads((OUT / 'label-manifest.json').read_text())
    record = {
        'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        'binary_sha256': sha(args.binary), 'runner_sha256': sha(Path(__file__)),
        'labels_sha256': sha(OUT / 'combined-labels.json'),
        'environment': {'platform': platform.platform(), 'RAYON_NUM_THREADS': os.environ.get('RAYON_NUM_THREADS'), 'offline': True},
        'inputs': [*frozen['inputs'], *labels['inputs']],
        'roots': {}, 'source_inventories': {}, 'commands': [], 'status': 'started',
    }
    write(dest / 'start.json', record)
    try:
        for item in record['inputs']:
            if sha(BASE / item['path']) != item['sha256']:
                raise ValueError(f"frozen input changed: {item['path']}")
        roots, paths = {}, {}
        for repo in json.loads(MANIFEST.read_text())['repositories']:
            rid = repo['id']
            archive = acquire_artifact(repo['artifact'], CACHE, offline=True, timeout_seconds=120)
            prefixes = ('src', 'LICENSE.txt', 'package.json') if rid == 'typescript-6.0.3' else ('',)
            root = extract_archive_subset(archive, work / rid, repo['artifact']['root'], prefixes)
            inventory = source_inventory(root, set(repo['sources']['extensions']))
            verify_inventory(repo, inventory)
            record['source_inventories'][rid] = {
                'archive_sha256': sha(archive), 'files': inventory.files, 'bytes': inventory.bytes,
                'physical_lines': inventory.physical_lines, 'manifest_sha256': inventory.manifest_sha256,
            }
            roots[rid] = root
            record['roots'][rid] = str(root)
            graph = None
            for command in ['analyze', 'inspect']:
                output = raw / f'{rid}-{command}.json'
                argv = [str(args.binary), command, str(root) if command == 'analyze' else graph, '--json']
                entry = {'argv': argv, 'stdout': output.name + '.gz'}
                record['commands'].append(entry)
                start = time.monotonic()
                with output.open('xb') as out, (dest / f'{rid}-{command}.stderr.txt').open('xb') as err:
                    proc = subprocess.run(argv, stdout=out, stderr=err, timeout=900)
                entry.update(exit_code=proc.returncode, seconds=round(time.monotonic()-start, 3), stdout_sha256=sha(output))
                with output.open('rb') as src, (dest / (output.name+'.gz')).open('xb') as dst:
                    with gzip.GzipFile(filename='', mode='wb', fileobj=dst, mtime=0) as gz:
                        shutil.copyfileobj(src, gz)
                if proc.returncode:
                    raise RuntimeError(f'{rid} {command}: exit {proc.returncode}')
                if command == 'analyze':
                    graph = json.loads(output.read_text())['graph_path']
            paths[rid] = output
        original = scorer.score(ORIGINAL, roots, paths)
        combined = scorer.score(OUT / 'combined-labels.json', roots, paths)
        write(dest / 'original-cohort.json', original)
        write(dest / 'combined-cohort.json', combined)
        previous_path = OUT / 'before/combined-cohort.json'
        previous = json.loads(previous_path.read_text())
        changes = [{'before': a, 'after': b} for a,b in zip(previous['results'], combined['results'], strict=True) if a != b]
        write(dest / 'changed-answers.json', {'previous_path': str(previous_path.relative_to(BASE)), 'previous_sha256': sha(previous_path), 'changes': changes})
        record.update(status='measured', original_cells=original['cell_counts'], combined_cells=combined['cell_counts'],
                      acceptance=acceptance(combined), changed_answers=len(changes))
    except BaseException as exc:
        record.update(status='interrupted' if isinstance(exc, KeyboardInterrupt) else 'unavailable', error=f'{type(exc).__name__}: {exc}')
        raise
    finally:
        write(dest / 'run.json', record)
        print(json.dumps({k:v for k,v in record.items() if k not in ['commands','inputs']}, indent=2))


if __name__ == '__main__':
    main()
