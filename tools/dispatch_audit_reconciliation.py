"""Prospective audit-size repair; old sites/results are never rewritten."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from tools import dispatch_audit_site_selector as rust
from tools import dispatch_audit_site_selector_python as python
from tools.core_representative_benchmark import acquire_artifact, extract_archive

BASE = Path(__file__).resolve().parents[1]
OUT = BASE / 'docs/observations/stage3-audit-reconciliation'
ORIGINALS = {
    'rust': BASE / 'docs/observations/stage3-rust-audit/audit-sites-labeled-v2.json',
    'python': BASE / 'docs/observations/stage3-python-audit/audit-sites-labeled.json',
}
REPOS = {'rust': rust.RUST_CRATE_IDS, 'python': python.PYTHON_REPO_IDS}


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def write(path: Path, doc: object) -> None:
    # Observations cannot silently replace earlier results.
    with path.open('x') as f:
        json.dump(doc, f, indent=2)
        f.write('\n')


def identity(site: dict) -> tuple:
    return (site.get('crate', site.get('package')), site['file'], site['line'])


def combine(original: list, reserve: list, labels: list, minimum: int = 100) -> list:
    if not labels or len(labels) > len(reserve):
        raise ValueError('labels must consume a nonempty prefix of the frozen reserve')
    seen = {identity(s) for s in original}
    real = sum(s['true_class'] != 'not_a_call_site' for s in original)
    for i, label in enumerate(labels):
        if real >= minimum:
            raise ValueError('labeling continued past the precommitted stopping point')
        if any(label.get(k) != v for k, v in reserve[i].items()):
            raise ValueError('reserve was skipped, reordered, or changed')
        if identity(label) in seen:
            raise ValueError('duplicate source site')
        seen.add(identity(label))
        if label.get('true_class') not in {'must', 'may', 'unknown', 'not_a_call_site'}:
            raise ValueError('missing or invalid ground truth')
        if not label.get('rationale') or label.get('confidence') not in {'high', 'medium', 'low'}:
            raise ValueError('ground truth must have a rationale and confidence')
        real += label['true_class'] != 'not_a_call_site'
    if real < minimum:
        raise ValueError(f'only {real} real sites; minimum is {minimum}')
    return original + labels


def acceptance(result: dict, minimum: int = 100) -> dict:
    rows = result['results']
    scored = [r for r in rows if r['status'] == 'scored']
    failures = [r for r in scored if r['cell'] in {'overclaim', 'unsafe_exclusion'}]
    must = [r for r in scored if r['observed_class'] == 'must']
    # Matching the class is insufficient when a scorer has identified the
    # wrong concrete target (for example, another overload).
    correct_must = sum(r['true_class'] == 'must' and r['cell'] == 'exact' for r in must)
    relocated = all(r['status'] in {'scored', 'not_a_call_site'} for r in rows)
    return {
        'minimum_real_sites': minimum, 'scored': len(scored),
        'not_a_call_site': sum(r['status'] == 'not_a_call_site' for r in rows),
        'all_sites_accounted_for': relocated, 'unsound': len(failures),
        'must_numerator': correct_must, 'must_denominator': len(must),
        'must_precision': correct_must / len(must) if must else None,
        'passed': len(scored) >= minimum and relocated and not failures
            and bool(must) and correct_must == len(must),
    }


def prepare(work: Path, binary: Path) -> None:
    repos = {r['id']: r for r in json.loads((BASE / 'docs/core-representative-corpus.json').read_text())['repositories']}
    manifest = {'seed': 20261001, 'binary': str(binary), 'binary_sha256': sha(binary),
                'roots': {}, 'artifacts': {}, 'originals': {}, 'reserves': {}}
    for language, ids in REPOS.items():
        pool = []
        original = json.loads(ORIGINALS[language].read_text())['sites']
        used = {identity(s) for s in original}
        for rid in ids:
            artifact = repos[rid]['artifact']
            archive = acquire_artifact(artifact, BASE / '.benchmark-cache/core-representative-v1', offline=True, timeout_seconds=60)
            root = extract_archive(archive, work / rid, artifact['root'])
            manifest['roots'][rid] = str(root)
            manifest['artifacts'][rid] = artifact
            if language == 'rust':
                pool.extend(rust.collect_sites(root, rid))
            else:
                errors = []
                pool.extend(python.collect_sites(root, rid, errors))
                if errors:
                    raise ValueError(f'masking failed: {rid}: {errors}')
        pool = [s for s in pool if identity(s) not in used]
        selector = rust if language == 'rust' else python
        reserve = selector.stratified_sample(pool, 256 if language == 'rust' else 128, 20261001)
        path = OUT / f'{language}-reserve.json'
        write(path, {'sites': reserve})
        manifest['reserves'][language] = {'path': str(path.relative_to(BASE)), 'sha256': sha(path), 'count': len(reserve)}
        manifest['originals'][language] = {'path': str(ORIGINALS[language].relative_to(BASE)), 'sha256': sha(ORIGINALS[language])}
    write(OUT / 'freeze-manifest.json', manifest)


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--prepare', action='store_true')
    p.add_argument('--work-root', type=Path)
    p.add_argument('--binary', type=Path)
    p.add_argument('--language', choices=list(REPOS))
    args = p.parse_args()
    if args.prepare:
        prepare(args.work_root, args.binary)
    else:
        lang = args.language
        original = json.loads(ORIGINALS[lang].read_text())['sites']
        reserve = json.loads((OUT / f'{lang}-reserve.json').read_text())['sites']
        labels = json.loads((OUT / f'{lang}-extension-labels.json').read_text())['sites']
        combined = combine(original, reserve, labels)
        write(OUT / f'{lang}-combined-labels.json', {'sites': combined})
        print(f'{lang}: {len(combined)} entries, 100 actual sites')


if __name__ == '__main__':
    main()
