"""Serial offline measurement of the frozen lexical-binding proof contract."""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time

from tools.dispatch_audit_scorer_typescript import extract_call_claims, resolve_node_id

BASE = Path(__file__).resolve().parents[1]
CORPUS = BASE / 'fixtures/typescript-binding-corpus/v1'
OBSERVATIONS = BASE / 'docs/observations/stage3-typescript-audit/lexical-bindings'


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--name', required=True)
    parser.add_argument('--binary', type=Path, default=Path('/mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder'))
    args = parser.parse_args()
    if not args.name.replace('-', '').isalnum():
        raise ValueError('name must be letters, digits, and hyphens')
    dest = OBSERVATIONS / args.name
    work = Path('/tmp') / f'girder-typescript-bindings-{args.name}'
    dest.mkdir(exist_ok=False)
    work.mkdir(exist_ok=False)
    manifest = json.loads((CORPUS / 'manifest.json').read_text())
    record = {
        'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        'binary_sha256': sha(args.binary), 'manifest_sha256': sha(CORPUS / 'manifest.json'),
        'runner_sha256': sha(Path(__file__)), 'runtime_probe_sha256': sha(BASE / 'tools/probe_typescript_binding.mjs'),
        'node_version': subprocess.check_output(['node', '--version'], text=True).strip(),
        'commands': [], 'cases': [], 'status': 'started',
    }

    def run(argv: list[str], name: str, timeout: int = 120) -> Path:
        output = work / f'{name}.json'
        item = {'argv': argv, 'stdout': output.name + '.gz'}
        record['commands'].append(item)
        start = time.monotonic()
        with output.open('xb') as out, (dest / f'{name}.stderr.log').open('xb') as err:
            proc = subprocess.run(argv, stdout=out, stderr=err, timeout=timeout)
        item.update(exit_status=proc.returncode, seconds=round(time.monotonic()-start, 3), stdout_sha256=sha(output))
        with output.open('rb') as src, (dest / (output.name + '.gz')).open('xb') as dst:
            with gzip.GzipFile(filename='', mode='wb', fileobj=dst, mtime=0) as archive:
                shutil.copyfileobj(src, archive)
        if proc.returncode:
            raise RuntimeError(f'{name}: exit {proc.returncode}')
        return output

    try:
        for case in manifest['cases']:
            source = CORPUS / case['file']
            if sha(source) != case['sha256']:
                raise ValueError(f"changed frozen source: {case['file']}")
            root = work / case['name']
            root.mkdir()
            shutil.copyfile(source, root / case['file'])
            row = {'name': case['name'], 'expected_class': case['expected_class'], 'source_sha256': sha(source)}
            record['cases'].append(row)
            try:
                result = run([str(args.binary), 'analyze', str(root), '--json'], case['name']+'-analyze')
                graph = json.loads(result.read_text())['graph_path']
                result = run([str(args.binary), 'inspect', graph, '--json'], case['name']+'-inspect')
                doc = json.loads(result.read_text())
                text = source.read_text()
                marker = text.index(manifest['marker']) + len(manifest['marker'])
                offset = len(text[:marker].encode()) + len(text[marker:].encode()) - len(text[marker:].lstrip().encode())
                claims = [c for c in extract_call_claims(result) if c['start_byte'] == offset]
                row['marked_claims'] = claims
                expected_start = len(text[:text.index(case['target_marker'])].encode())
                targets = [n for n in doc['nodes'] if n['kind'] == 'Function' and n['span']['start_byte'] == expected_start]
                ids = {n['id'] for n in targets}
                target_ids = {resolve_node_id(t) for c in claims for t in c['targets']}
                row['target_matches_original_declaration'] = len(ids) == 1 and target_ids == ids
                row['exact_contract'] = len(claims) == 1 and claims[0]['class'] == case['expected_class'] and (
                    row['target_matches_original_declaration'] if case['expected_class'] == 'must' else not target_ids)
                if case['runtime_mode'] in ['module', 'script']:
                    result = run(['node', str(BASE / 'tools/probe_typescript_binding.mjs'), str(source), case['runtime_mode']], case['name']+'-runtime', 10)
                    row['runtime_observed'] = json.loads(result.read_text())
                    row['runtime_matches'] = row['runtime_observed'] == case['runtime_expected']
                    row['runtime_disproves_original_must'] = (
                        case['runtime_exercises_claim'] and row['runtime_matches']
                        and row['runtime_observed'] != 'original'
                        and any(c['class'] == 'must' for c in claims)
                        and row['target_matches_original_declaration'])
                else:
                    row['runtime_skipped'] = case['runtime_mode']
            except Exception as exc:
                row.update(exact_contract=False, error=f'{type(exc).__name__}: {exc}')
        record.update(
            status='measured',
            exact_contracts=sum(c.get('exact_contract', False) for c in record['cases']),
            runtime_passed=sum(c.get('runtime_matches', False) for c in record['cases']),
            runtime_skipped=sum('runtime_skipped' in c for c in record['cases']),
            runtime_disproved_must=[c['name'] for c in record['cases'] if c.get('runtime_disproves_original_must')],
        )
        record['criterion_met'] = record['exact_contracts'] == 22 and record['runtime_passed'] == 19 and record['runtime_skipped'] == 3
    except BaseException as exc:
        record.update(status='failed', error=f'{type(exc).__name__}: {exc}')
        raise
    finally:
        with (dest / 'run.json').open('x') as stream:
            json.dump(record, stream, indent=2)
            stream.write('\n')
        print(json.dumps({k:v for k,v in record.items() if k not in ['commands','cases']}, indent=2))


if __name__ == '__main__':
    main()
