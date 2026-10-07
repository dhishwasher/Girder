"""Measure the frozen ESM import contract, preserving every refusal and failure.

This measures cold CLI graphs only. Incremental, watched MCP, ingestion-route,
dispatch-corpus and real-repository acceptance require separate observations.
Runtime truth is pinned by the existing preimplementation checks, not rerun here.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

from tools.dispatch_audit_scorer_typescript import extract_call_claims, resolve_node_id

BASE = Path(__file__).resolve().parents[1]
CORPUS = BASE / 'fixtures/typescript-esm-import-proof/v1'
OBS = BASE / 'docs/observations/stage3-typescript-audit/esm-import-proof'
MANIFEST_SHA = 'daa7310248f8e2adc3bdbbae340a3301dcca6b18983ca29ef3c6738bb97b94cd'


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pins(root: Path) -> dict:
    result = {}
    for directory, dirs, files in os.walk(root, followlinks=False):
        for name in dirs + files:
            path = Path(directory) / name
            if path.is_symlink():
                result[path.relative_to(root).as_posix()] = {'symlink': os.readlink(path)}
            elif path.is_file():
                result[path.relative_to(root).as_posix()] = sha(path)
    return result


def score(case: dict, manifest: dict, root: Path, inspect: Path) -> dict:
    def file_identity(file: str) -> Path:
        path = Path(file)
        return path if path.is_absolute() else root / path

    source = (root / case['importer']).read_bytes()
    marker = manifest['marker'].encode()
    if source.count(marker) != 1:
        raise ValueError('expected exactly one call marker')
    tail = source.split(marker, 1)[1]
    offset = len(source) - len(tail.lstrip())
    claims = [c for c in extract_call_claims(inspect)
              if file_identity(c['file']) == root / case['importer']
              and c['start_byte'] == offset]
    doc = json.loads(inspect.read_text())
    result = {'marked_claims': claims, 'exact_contract': False}
    if len(claims) != 1:
        result['failure'] = f'expected one marked claim, got {len(claims)}'
        return result
    claim = claims[0]
    targets = [resolve_node_id(t) for t in claim['targets']]
    if case['expected_class'] == 'unknown':
        result['exact_contract'] = claim['class'] == 'unknown' and targets == []
        return result
    expected = case['expected_target']
    target_source = (root / expected['file']).read_bytes()
    target_marker = expected['marker'].encode()
    if target_source.count(target_marker) != 1:
        raise ValueError('expected exactly one target marker')
    start = target_source.index(target_marker)
    function_start = start + target_marker.index(b'function')
    nodes = [n for n in doc['nodes'] if n['kind'] == 'Function'
             and n['path'] == expected['predicted_path']
             and n.get('file') and file_identity(n['file']) == root / expected['file']
             and start <= n['span']['start_byte'] <= function_start < n['span']['end_byte']]
    owners = [n for n in doc['nodes'] if n['path'] == claim['caller']
              and n.get('file') == claim['file']]
    evidence = [v for n in owners for k, v in n.get('attributes', []) if k == 'call_evidence_v1']
    assumptions = []
    if len(evidence) == 1:
        matches = re.findall(r'assumptions:\[([^\]]*)\]', evidence[0])
        if len(matches) == 1:
            assumptions = json.loads('[' + matches[0] + ']')
    result['assumptions'] = assumptions
    result['expected_target_nodes'] = [n['id'] for n in nodes]
    result['exact_contract'] = (
        claim['class'] == 'must' and not claim['coverage_gap']
        and claim['reason'] == 'proven-typescript-relative-esm-named-import'
        and len(nodes) == 1 and targets == [nodes[0]['id']]
        and set(manifest['must_claim_assumptions']).issubset(assumptions)
    )
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--name', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--binary-revision', required=True)
    args = parser.parse_args()
    if not re.fullmatch(r'[a-zA-Z0-9-]+', args.name):
        parser.error('name must contain only letters, digits and hyphens')
    dest = OBS / args.name
    dest.mkdir(exist_ok=False)
    record = {'status': 'started', 'scope': 'cold CLI proof contract only',
              'candidate': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=BASE, text=True).strip(),
              'binary_revision': args.binary_revision, 'commands': [], 'cases': [],
              'must_interpretation': 'conditional on the frozen execution assumptions'}

    def save():
        temporary = dest / 'run.json.tmp'
        temporary.write_text(json.dumps(record, indent=2) + '\n')
        temporary.replace(dest / 'run.json')

    save()
    try:
        binary = args.binary.resolve(strict=True)
        record.update(binary_sha256=sha(binary), runner_sha256=sha(Path(__file__)),
                      manifest_sha256=sha(CORPUS / 'manifest.json'))
        if record['manifest_sha256'] != MANIFEST_SHA:
            raise ValueError('frozen manifest changed')
        manifest = json.loads((CORPUS / 'manifest.json').read_text())
        for path, digest in manifest['frozen_inputs'].items():
            if sha(BASE / path) != digest:
                raise ValueError(f'frozen input changed: {path}')
        for case in manifest['cases']:
            if pins(CORPUS / case['id']) != case['files']:
                raise ValueError(f'frozen case changed: {case["id"]}')
        with tempfile.TemporaryDirectory(prefix=f'girder-esm-{args.name}-') as workdir:
            work = Path(workdir)
            record['scratch_root'] = str(work)

            def command(argv, stem):
                output = work / (stem + '.json')
                item = {'argv': argv, 'stdout': stem + '.json.gz', 'stderr': stem + '.stderr.log'}
                record['commands'].append(item)
                save()
                started = time.monotonic()
                try:
                    with output.open('xb') as out, (dest / item['stderr']).open('xb') as err:
                        proc = subprocess.run(argv, stdout=out, stderr=err, timeout=120)
                    item['exit_status'] = proc.returncode
                    if proc.returncode:
                        raise RuntimeError(f'command exit {proc.returncode}')
                except subprocess.TimeoutExpired:
                    item['timed_out'] = True
                    raise
                finally:
                    item['seconds'] = round(time.monotonic() - started, 3)
                    if output.exists():
                        item['stdout_sha256'] = sha(output)
                        (dest / item['stdout']).write_bytes(gzip.compress(output.read_bytes(), mtime=0))
                    save()
                return output

            for case in manifest['cases']:
                row = {'id': case['id'], 'expected_class': case['expected_class'], 'exact_contract': False}
                record['cases'].append(row)
                try:
                    root = work / case['id']
                    shutil.copytree(CORPUS / case['id'], root, symlinks=True)
                    analysis = command([str(binary), 'analyze', str(root), '--json'], case['id'] + '-analyze')
                    graph = json.loads(analysis.read_text())['graph_path']
                    inspection = command([str(binary), 'inspect', graph, '--json'], case['id'] + '-inspect')
                    row.update(score(case, manifest, root, inspection))
                except Exception as error:
                    row['error'] = f'{type(error).__name__}: {error}'
                save()
        record.update(status='measured', total=len(record['cases']),
                      exact_contracts=sum(c['exact_contract'] for c in record['cases']),
                      errors=sum('error' in c for c in record['cases']))
        record['cold_contract_met'] = record['total'] == 73 and record['exact_contracts'] == 73
        return 0 if record['cold_contract_met'] else 1
    except BaseException as error:
        record.update(status='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        save()
        print(json.dumps({k: v for k, v in record.items() if k not in ('cases', 'commands')}, indent=2))


if __name__ == '__main__':
    raise SystemExit(main())
