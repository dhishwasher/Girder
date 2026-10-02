"""Run the frozen audit extension serially, retaining raw output and failures."""
from __future__ import annotations

import gzip
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

from tools import dispatch_audit_scorer as rust
from tools import dispatch_audit_scorer_python as python
from tools.dispatch_audit_reconciliation import BASE, OUT, ORIGINALS, REPOS, acceptance, sha, write

PREVIOUS = {
    'rust': BASE / 'docs/observations/stage3-rust-audit/after-method-call-fix/correction-2/audit-after.json',
    'python': BASE / 'docs/observations/stage3-python-audit/after-transformed-scope-fix/correction-1/audit-scored-results.json',
}


def main() -> None:
    frozen = json.loads((OUT / 'freeze-manifest.json').read_text())
    labels = json.loads((OUT / 'label-manifest.json').read_text())
    binary = Path(frozen['binary'])
    if sha(binary) != frozen['binary_sha256']:
        raise ValueError('candidate binary changed')
    for entry in [*frozen['originals'].values(), *frozen['reserves'].values(), *labels['files']]:
        if sha(BASE / entry['path']) != entry['sha256']:
            raise ValueError(f"frozen input changed: {entry['path']}")
    destination = OUT / 'observation'
    destination.mkdir(exist_ok=False)
    work = Path('/tmp/girder-audit-reconciliation-20261001/raw')
    work.mkdir(exist_ok=False)
    record = {
        'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        'binary_sha256': sha(binary), 'commands': [], 'languages': {},
        'policy_sha256': sha(OUT / 'policy.md'),
        'label_manifest_sha256': sha(OUT / 'label-manifest.json'),
        'environment': {k: os.environ.get(k) for k in ['CARGO_BUILD_JOBS', 'CARGO_INCREMENTAL', 'CARGO_TARGET_DIR']},
    }
    roots = {rid: Path(root) for rid, root in frozen['roots'].items()}
    try:
        for language, ids in REPOS.items():
            paths = {}
            for rid in ids:
                analyze = work / f'{rid}-analyze.json'
                inspect = work / f'{rid}-inspect.json'
                graph = None
                for command, output in [('analyze', analyze), ('inspect', inspect)]:
                    args = [str(binary), command, str(roots[rid]) if command == 'analyze' else graph, '--json']
                    start = time.monotonic()
                    stderr = destination / f'{rid}-{command}.stderr.txt'
                    entry = {'argv': args, 'stdout': output.name, 'stderr': stderr.name}
                    record['commands'].append(entry)
                    with output.open('xb') as out, stderr.open('xb') as err:
                        proc = subprocess.run(args, stdout=out, stderr=err, timeout=600)
                    entry.update(exit_code=proc.returncode, seconds=round(time.monotonic()-start, 3), stdout_sha256=sha(output))
                    # Stream compression: keep peak RAM below the inspect parser's needs.
                    with output.open('rb') as source, (destination / (output.name + '.gz')).open('xb') as raw:
                        with gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=0) as gz:
                            shutil.copyfileobj(source, gz)
                    if proc.returncode:
                        raise RuntimeError(f'{rid} {command}: exit {proc.returncode}')
                    if command == 'analyze':
                        graph = json.loads(output.read_text())['graph_path']
                paths[rid] = inspect
            scorer = rust if language == 'rust' else python
            old = scorer.score(ORIGINALS[language], roots, paths)
            combined = scorer.score(OUT / f'{language}-combined-labels.json', roots, paths)
            write(destination / f'{language}-original-cohort.json', old)
            write(destination / f'{language}-combined-cohort.json', combined)
            previous = json.loads(PREVIOUS[language].read_text())
            keys = ('status', 'observed_class', 'observed_reason', 'observed_caller', 'cell')
            changed = []
            for before, after in zip(previous['results'], old['results'], strict=True):
                assert (before['file'], before['line']) == (after['file'], after['line'])
                if any(before.get(k) != after.get(k) for k in keys):
                    changed.append({'before': before, 'after': after})
            write(destination / f'{language}-changed-original-answers.json', {
                'previous_path': str(PREVIOUS[language].relative_to(BASE)),
                'previous_sha256': sha(PREVIOUS[language]), 'changes': changed,
            })
            record['languages'][language] = {
                'original_cells': old['cell_counts'], 'combined_cells': combined['cell_counts'],
                'changed_original_answers': len(changed), 'acceptance': acceptance(combined),
            }
            print(language, json.dumps(record['languages'][language]), flush=True)
            if language == 'rust':
                doc = json.loads(paths['serde_json-1.0.150'].read_text())
                nodes = [n for n in doc['nodes'] if 'serialize_element' in n['path']]
                write(destination / 'rust-serialize-element-identities.json', {'nodes': nodes})
                del doc, nodes
        record['status'] = 'measured'
    except Exception as exc:
        record.update(status='unavailable', error=f'{type(exc).__name__}: {exc}')
        raise
    finally:
        write(destination / 'run.json', record)


if __name__ == '__main__':
    main()
