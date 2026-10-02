"""Finish the interrupted Python audit while preserving completed extracts."""
from __future__ import annotations

import gzip
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import time

from tools import dispatch_audit_scorer_python as scorer
from tools.core_representative_benchmark import acquire_artifact
from tools.dispatch_audit_reconciliation import BASE, OUT, ORIGINALS, REPOS, acceptance, sha, write


def main() -> None:
    frozen = json.loads((OUT / 'freeze-manifest.json').read_text())
    candidate = json.loads((OUT / 'operator-candidate.json').read_text())
    prior_dir = OUT / 'operator-correction'
    prior = json.loads((prior_dir / 'run.json').read_text())
    assert prior['candidate'] == candidate
    binary = Path(candidate['binary'])
    assert sha(binary) == candidate['binary_sha256'], 'binary changed'
    for item in json.loads((OUT / 'label-manifest.json').read_text())['files']:
        assert sha(BASE / item['path']) == item['sha256'], item['path']
    for item in frozen['originals'].values():
        assert sha(BASE / item['path']) == item['sha256'], item['path']
    roots = {rid: Path(frozen['roots'][rid]) for rid in REPOS['python']}
    # A pause must not allow changed source to be scored against old labels.
    for rid, root in roots.items():
        artifact = frozen['artifacts'][rid]
        archive = acquire_artifact(artifact, BASE / '.benchmark-cache/core-representative-v1', offline=True, timeout_seconds=60)
        with tarfile.open(archive) as tar:
            for member in tar.getmembers():
                if member.isfile():
                    relative = Path(member.name).relative_to(artifact['root'])
                    assert (root / relative).read_bytes() == tar.extractfile(member).read(), str(relative)
    dest = OUT / 'operator-resumption'
    dest.mkdir(exist_ok=False)
    work = Path('/tmp/girder-audit-reconciliation-20261001/operator-resumption')
    work.mkdir(exist_ok=False)
    record = {'candidate': candidate, 'prior_run_sha256': sha(prior_dir / 'run.json'),
              'runner_sha256': sha(Path(__file__)), 'commands': [], 'reused': [], 'status': 'started'}
    paths = {}
    try:
        for rid, root in roots.items():
            completed = next((c for c in prior['commands'] if c['argv'][1] == 'inspect'
                              and rid in c['argv'][2] and c.get('exit_code') == 0), None)
            if completed:
                path = work / f'{rid}-inspect.json'
                source = prior_dir / completed['stdout']
                with gzip.open(source, 'rb') as src, path.open('xb') as dst:
                    shutil.copyfileobj(src, dst)
                assert sha(path) == completed['stdout_sha256'], 'completed output changed'
                record['reused'].append({'path': str(source.relative_to(BASE)), 'stdout_sha256': sha(path)})
                paths[rid] = path
                continue
            graph = None
            for command in ['analyze', 'inspect']:
                path = work / f'{rid}-{command}.json'
                argv = [str(binary), command, str(root) if command == 'analyze' else graph, '--json']
                entry = {'argv': argv, 'stdout': path.name + '.gz'}
                record['commands'].append(entry)
                start = time.monotonic()
                with path.open('xb') as out, (dest / f'{rid}-{command}.stderr.txt').open('xb') as err:
                    proc = subprocess.run(argv, stdout=out, stderr=err, timeout=600)
                entry.update(exit_code=proc.returncode, seconds=round(time.monotonic()-start, 3), stdout_sha256=sha(path))
                with path.open('rb') as src, (dest / (path.name + '.gz')).open('xb') as raw:
                    with gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=0) as gz:
                        shutil.copyfileobj(src, gz)
                if proc.returncode:
                    raise RuntimeError(f'{rid} {command}: exit {proc.returncode}')
                if command == 'analyze':
                    graph = json.loads(path.read_text())['graph_path']
            paths[rid] = path
        combined = scorer.score(OUT / 'python-combined-labels.json', roots, paths)
        write(dest / 'python-combined-cohort.json', combined)
        write(dest / 'python-original-cohort.json', scorer.score(ORIGINALS['python'], roots, paths))
        before = json.loads((OUT / 'observation/python-combined-cohort.json').read_text())
        changes = [{'before': old, 'after': new} for old, new in zip(before['results'], combined['results'], strict=True) if old != new]
        write(dest / 'python-changed-answers.json', {'changes': changes})
        record.update(status='measured', cells=combined['cell_counts'], acceptance=acceptance(combined), changed_answers=len(changes))
        print(json.dumps({k: record[k] for k in ['status', 'cells', 'acceptance', 'changed_answers']}))
    except BaseException as exc:
        record.update(status='interrupted' if isinstance(exc, KeyboardInterrupt) else 'unavailable', error=f'{type(exc).__name__}: {exc}')
        raise
    finally:
        write(dest / 'run.json', record)


if __name__ == '__main__':
    main()
