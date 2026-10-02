"""Offline, one-shot CLI observation for the precommitted registration repair.

Run from the repository root: python3 -m tools.measure_typescript_registration_identity
The two real files are analyzed in isolation: this measures extraction/ownership,
not whole-project resolution or the 100-site language acceptance criterion.
"""
from __future__ import annotations

import gzip
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time

from tools import dispatch_audit_scorer_typescript as scorer

BASE = Path(__file__).resolve().parents[1]
AUDIT = BASE / 'docs/observations/stage3-typescript-audit'
DEST = AUDIT / 'identity-repair/cli-observation-1'
BINARY = Path('/mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder')
WORK = Path('/tmp/girder-ts-identity-measurement-67d65f6')


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path: Path, value: object) -> None:
    with path.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')


def packed(path: Path, value: object) -> None:
    with path.open('xb') as stream:
        with gzip.GzipFile(filename='', mode='wb', fileobj=stream, mtime=0) as archive:
            archive.write((json.dumps(value, indent=2) + '\n').encode())


def tests(doc: dict) -> list[dict]:
    return [n for n in doc['nodes'] if dict(n['attributes']).get('is_test') == 'true']


def main() -> None:
    DEST.mkdir(exist_ok=False)
    WORK.mkdir(exist_ok=False)
    labels = AUDIT / 'extension/combined-labels.json'
    frozen = json.loads((AUDIT / 'extension/freeze-manifest.json').read_text())
    sites = json.loads(labels.read_text())['sites']
    record = {
        'candidate': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        'binary_sha256': sha(BINARY), 'labels_sha256': sha(labels),
        'scope': 'Isolated source-file extraction and call ownership; not whole-project dispatch acceptance.',
        'commands': [], 'cases': [], 'status': 'started',
    }
    try:
        if record['binary_sha256'] != '13976d976157318878512ae6eb5458f0cf5ca4f19b5c642a9bf16bbde54381f7':
            raise ValueError('unexpected rebuilt binary')
        fixture = AUDIT / 'collision-repro/typescript-repro'
        cases = [
            ('original', fixture / 'sample.ts', 'sample.ts', fixture / 'inspect-baseline.json', None),
            ('harder', AUDIT / 'identity-repair/harder.test.ts', 'harder.test.ts', None, None),
        ]
        for index in [23, 91]:
            site = sites[index]
            source = Path(frozen['roots'][site['package']]) / site['file']
            before = Path(frozen['roots'][site['package']]).parent / 'raw-before' / f"{site['package']}-inspect.json"
            cases.append((f'site-{index}', source, site['file'], before, index))
        for name, source, relative, before_path, index in cases:
            root = WORK / name
            target = root / relative
            target.parent.mkdir(parents=True)
            shutil.copyfile(source, target)
            case = {'name': name, 'source': str(source), 'file': relative, 'source_sha256': sha(source)}
            record['cases'].append(case)
            before = None
            if before_path:
                complete = json.loads(before_path.read_text())
                nodes = [n for n in complete['nodes'] if n['file'] == relative]
                modules = [n for n in nodes if n['kind'] == 'Module']
                if len(modules) != 1 or modules[0]['source_sha256'] != sha(source):
                    raise ValueError(f'{name}: baseline source fingerprint mismatch')
                before = {'nodes': nodes}
                case.update(before_path=str(before_path), before_sha256=sha(before_path), before_tests=len(tests(before)))
                packed(DEST / f'{name}-before-nodes.json.gz', before)
            graph = None
            for command in ['analyze', 'inspect']:
                argv = [str(BINARY), command, str(root) if command == 'analyze' else graph, '--json']
                output = WORK / f'{name}-{command}.json'
                entry = {'argv': argv}
                record['commands'].append(entry)
                start = time.monotonic()
                with output.open('xb') as out, (DEST / f'{name}-{command}.stderr.log').open('xb') as err:
                    proc = subprocess.run(argv, stdout=out, stderr=err, timeout=900)
                entry.update(exit_status=proc.returncode, seconds=round(time.monotonic()-start, 3), stdout_sha256=sha(output))
                with output.open('rb') as src, (DEST / f'{name}-{command}.json.gz').open('xb') as dst:
                    with gzip.GzipFile(filename='', mode='wb', fileobj=dst, mtime=0) as archive:
                        shutil.copyfileobj(src, archive)
                if proc.returncode:
                    raise RuntimeError(f'{name} {command} failed: {proc.returncode}')
                doc = json.loads(output.read_text())
                if command == 'analyze':
                    graph = doc['graph_path']
            case['after_tests'] = len(tests(doc))
            claims = scorer.extract_call_claims(output)
            case['duplicate_path_boundaries'] = sum(c['reason'] == 'duplicate-semantic-path' for c in claims)
            if case['duplicate_path_boundaries']:
                raise AssertionError(f'{name}: duplicate identities remain')
            if before:
                old = {n['span']['start_byte'] for n in tests(before)}
                new = {n['span']['start_byte'] for n in tests(doc)}
                case['lost_registration_offsets'] = sorted(old - new)
                case['recovered_registration_offsets'] = sorted(new - old)
                if old - new or not new - old:
                    raise AssertionError(f'{name}: lost registrations or none recovered')
            if index is not None:
                selected = WORK / f'{name}-label.json'
                write(selected, {'sites': [sites[index]]})
                result = scorer.score(selected, {sites[index]['package']: root}, {sites[index]['package']: output})
                case.update(audit_index=index, answer=result['results'][0])
                write(DEST / f'{name}-score.json', result)
                answer = case['answer']
                owner = next((n for n in doc['nodes'] if n['path'] == answer.get('observed_caller')), None)
                case['site_owned_by_test'] = bool(owner and dict(owner['attributes']).get('is_test') == 'true')
                if not case['site_owned_by_test'] or answer.get('cell') not in ['exact', 'conservative']:
                    raise AssertionError(f'{name}: missing/corrupt call ownership or unsound answer')
            elif case['after_tests'] != (4 if name == 'original' else 5):
                raise AssertionError(f'{name}: incorrect test inventory')
        record['status'] = 'measured'
    except BaseException as exc:
        record.update(status='failed', error=f'{type(exc).__name__}: {exc}')
        raise
    finally:
        write(DEST / 'run.json', record)
        print(json.dumps(record, indent=2))


if __name__ == '__main__':
    main()
