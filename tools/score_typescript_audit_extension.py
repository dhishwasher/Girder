"""Measure the frozen TypeScript extension before changing its resolver."""
from __future__ import annotations

import gzip
import json
from pathlib import Path
import shutil
import subprocess
import time

from tools import dispatch_audit_scorer_typescript as scorer
from tools.dispatch_audit_reconciliation import BASE, acceptance, sha, write
from tools.prepare_typescript_audit_extension import OUT, ORIGINAL, WORK


def main() -> None:
    frozen = json.loads((OUT / 'freeze-manifest.json').read_text())
    labels = json.loads((OUT / 'label-manifest.json').read_text())
    for item in [*frozen['inputs'], *labels['inputs']]:
        if sha(BASE / item['path']) != item['sha256']:
            raise ValueError(f"frozen input changed: {item['path']}")
    binary = Path(frozen['binary'])
    if sha(binary) != frozen['binary_sha256']:
        raise ValueError('candidate binary changed')
    dest = OUT / 'before'
    dest.mkdir(exist_ok=False)
    raw = WORK / 'raw-before'
    raw.mkdir(exist_ok=False)
    roots = {rid: Path(path) for rid, path in frozen['roots'].items()}
    paths = {}
    record = {'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
              'binary_sha256': sha(binary), 'commands': [], 'status': 'started'}
    try:
        for rid, root in roots.items():
            graph = None
            for command in ['analyze', 'inspect']:
                path = raw / f'{rid}-{command}.json'
                argv = [str(binary), command, str(root) if command == 'analyze' else graph, '--json']
                entry = {'argv': argv, 'stdout': path.name+'.gz'}
                record['commands'].append(entry)
                start = time.monotonic()
                with path.open('xb') as out, (dest / f'{rid}-{command}.stderr.txt').open('xb') as err:
                    proc = subprocess.run(argv, stdout=out, stderr=err, timeout=900)
                entry.update(exit_code=proc.returncode, seconds=round(time.monotonic()-start, 3), stdout_sha256=sha(path))
                with path.open('rb') as src, (dest / (path.name+'.gz')).open('xb') as dst:
                    with gzip.GzipFile(filename='', mode='wb', fileobj=dst, mtime=0) as gz:
                        shutil.copyfileobj(src, gz)
                if proc.returncode:
                    raise RuntimeError(f'{rid} {command}: exit {proc.returncode}')
                if command == 'analyze':
                    graph = json.loads(path.read_text())['graph_path']
            paths[rid] = path
        original = scorer.score(ORIGINAL, roots, paths)
        combined = scorer.score(OUT / 'combined-labels.json', roots, paths)
        write(dest / 'original-cohort.json', original)
        write(dest / 'combined-cohort.json', combined)
        previous_path = OUT.parent / 'audit-scored-results.json'
        previous = json.loads(previous_path.read_text())
        changes = [{'before': a, 'after': b} for a,b in zip(previous['results'], original['results'], strict=True) if a != b]
        write(dest / 'changed-original-answers.json', {'previous_path': str(previous_path.relative_to(BASE)),
                                                      'previous_sha256': sha(previous_path), 'changes': changes})
        record.update(status='measured', original_cells=original['cell_counts'], combined_cells=combined['cell_counts'],
                      acceptance=acceptance(combined), changed_original_answers=len(changes))
        print(json.dumps({k:record[k] for k in ['status','original_cells','combined_cells','acceptance','changed_original_answers']}))
    except BaseException as exc:
        record.update(status='interrupted' if isinstance(exc, KeyboardInterrupt) else 'unavailable', error=f'{type(exc).__name__}: {exc}')
        raise
    finally:
        write(dest / 'run.json', record)


if __name__ == '__main__':
    main()
