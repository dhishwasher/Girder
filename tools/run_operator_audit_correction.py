"""Measure the operator correction without replacing the first audit result."""
from __future__ import annotations

import gzip
import json
from pathlib import Path
import shutil
import subprocess
import time

from tools import dispatch_audit_scorer as rust
from tools import dispatch_audit_scorer_python as python
from tools.dispatch_audit_reconciliation import BASE, OUT, ORIGINALS, REPOS, acceptance, sha, write


def main() -> None:
    frozen = json.loads((OUT / 'freeze-manifest.json').read_text())
    candidate = json.loads((OUT / 'operator-candidate.json').read_text())
    for entry in json.loads((OUT / 'label-manifest.json').read_text())['files']:
        if sha(BASE / entry['path']) != entry['sha256']:
            raise ValueError(f"frozen input changed: {entry['path']}")
    for entry in [*frozen['originals'].values(), *frozen['reserves'].values()]:
        if sha(BASE / entry['path']) != entry['sha256']:
            raise ValueError(f"frozen sample changed: {entry['path']}")
    binary = Path(candidate['binary'])
    if sha(binary) != candidate['binary_sha256']:
        raise ValueError('candidate binary changed')
    roots = {rid: Path(root) for rid, root in frozen['roots'].items()}
    dest = OUT / 'operator-correction'
    dest.mkdir(exist_ok=False)
    work = Path('/tmp/girder-audit-reconciliation-20261001/operator-raw')
    work.mkdir(exist_ok=False)
    record = {'candidate': candidate, 'commands': [], 'languages': {}}
    try:
        for lang, ids in REPOS.items():
            paths = {}
            for rid in ids:
                graph = None
                for command in ['analyze', 'inspect']:
                    path = work / f'{rid}-{command}.json'
                    argv = [str(binary), command, str(roots[rid]) if command == 'analyze' else graph, '--json']
                    entry = {'argv': argv, 'stdout': path.name + '.gz'}
                    record['commands'].append(entry)
                    start = time.monotonic()
                    with path.open('xb') as out, (dest / f'{rid}-{command}.stderr.txt').open('xb') as err:
                        proc = subprocess.run(argv, stdout=out, stderr=err, timeout=600)
                    entry.update(exit_code=proc.returncode, seconds=round(time.monotonic()-start, 3), stdout_sha256=sha(path))
                    with path.open('rb') as source, (dest / (path.name + '.gz')).open('xb') as raw:
                        with gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=0) as gz:
                            shutil.copyfileobj(source, gz)
                    if proc.returncode:
                        raise RuntimeError(f'{rid} {command}: exit {proc.returncode}')
                    if command == 'analyze':
                        graph = json.loads(path.read_text())['graph_path']
                paths[rid] = path
            scorer = rust if lang == 'rust' else python
            result = scorer.score(OUT / f'{lang}-combined-labels.json', roots, paths)
            write(dest / f'{lang}-combined-cohort.json', result)
            write(dest / f'{lang}-original-cohort.json', scorer.score(ORIGINALS[lang], roots, paths))
            before = json.loads((OUT / 'observation' / f'{lang}-combined-cohort.json').read_text())
            changes = []
            for old, new in zip(before['results'], result['results'], strict=True):
                if old != new:
                    changes.append({'before': old, 'after': new})
            write(dest / f'{lang}-changed-answers.json', {'changes': changes})
            record['languages'][lang] = {'cells': result['cell_counts'], 'acceptance': acceptance(result), 'changed_answers': len(changes)}
            print(lang, json.dumps(record['languages'][lang]), flush=True)
        record['status'] = 'measured'
    except Exception as exc:
        record.update(status='unavailable', error=f'{type(exc).__name__}: {exc}')
        raise
    finally:
        write(dest / 'run.json', record)


if __name__ == '__main__':
    main()
