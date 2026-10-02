"""Freeze an independent TypeScript reserve without observing graph answers."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import random
import re
import tempfile
import urllib.request

from tools.core_representative_benchmark import acquire_artifact, ensure_cache_directory, RejectRedirects
from tools.dispatch_audit_reconciliation import BASE, combine, identity, sha, write
from tools import dispatch_audit_site_selector_typescript as selector

OUT = BASE / 'docs/observations/stage3-typescript-audit/extension'
ORIGINAL = OUT.parent / 'audit-sites-labeled.json'
MANIFEST = BASE / 'docs/stage3-typescript-corpus.json'
CACHE = BASE / '.benchmark-cache/stage3-typescript-v1'
WORK = Path('/tmp/girder-typescript-extension-20261002')


def acquire_tag(artifact: dict) -> Path:
    """Bound the actual response body even when codeload omits Content-Length."""
    ensure_cache_directory(CACHE)
    destination = CACHE / f"{artifact['sha256']}.tar.gz"
    if destination.exists():
        return acquire_artifact(artifact, CACHE, offline=True, timeout_seconds=120)
    request = urllib.request.Request(artifact['url'], headers={'Accept-Encoding': 'identity'})
    opener = urllib.request.build_opener(RejectRedirects())
    descriptor, name = tempfile.mkstemp(prefix='.acquire-', dir=CACHE)
    temporary = Path(name)
    received = 0
    try:
        with os.fdopen(descriptor, 'wb') as output, opener.open(request, timeout=120) as response:
            if response.status != 200 or response.geturl() != artifact['url']:
                raise RuntimeError('unexpected archive response')
            if response.headers.get('Content-Encoding') not in {None, '', 'identity'}:
                raise RuntimeError('unexpected archive encoding')
            length = response.headers.get('Content-Length')
            if length is not None and int(length) != artifact['bytes']:
                raise RuntimeError('archive declared size mismatch')
            while chunk := response.read(64 * 1024):
                received += len(chunk)
                if received > artifact['bytes']:
                    raise RuntimeError('archive exceeded pinned size')
                output.write(chunk)
            output.flush()
            os.fsync(output.fileno())
        if received != artifact['bytes'] or sha(temporary) != artifact['sha256']:
            raise RuntimeError('archive actual size or SHA mismatch')
        os.replace(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)
    return destination


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('step', choices=['acquire', 'prepare', 'combine'])
    parser.add_argument('--binary', type=Path)
    args = parser.parse_args()
    repos = json.loads(MANIFEST.read_text())['repositories']
    if args.step == 'acquire':
        for repo in repos:
            artifact = dict(repo['artifact'])
            match = re.fullmatch(r'https://github.com/([^/]+)/([^/]+)/archive/refs/tags/([^/]+)\.tar\.gz', artifact['url'])
            if match is None:
                raise ValueError('expected a pinned GitHub tag archive')
            owner, name, tag = match.groups()
            # Request GitHub's archive endpoint directly, rejecting redirects
            # and enforcing the original body size/SHA.
            artifact['url'] = f'https://codeload.github.com/{owner}/{name}/tar.gz/refs/tags/{tag}'
            path = acquire_tag(artifact)
            print(repo['id'], sha(path), flush=True)
        return
    original = json.loads(ORIGINAL.read_text())['sites']
    if args.step == 'combine':
        reserve = json.loads((OUT / 'reserve.json').read_text())['sites']
        labels = json.loads((OUT / 'extension-labels.json').read_text())['sites']
        for label in labels:
            if label['true_class'] == 'must' and not label.get('true_target'):
                raise ValueError('Must requires a checked target')
        write(OUT / 'combined-labels.json', {'sites': combine(original, reserve, labels)})
        return
    if args.binary is None:
        raise ValueError('--binary required when preparing')
    used = {identity(s) for s in original}
    frozen = {'seed': 20261002, 'roots': {}, 'artifacts': {}, 'inputs': [],
              'binary': str(args.binary), 'binary_sha256': sha(args.binary), 'pool_counts': {}}
    pool = []
    for repo in repos:
        rid = repo['id']
        archive = acquire_artifact(repo['artifact'], CACHE, offline=True, timeout_seconds=120)
        prefixes = ('src', 'LICENSE.txt', 'package.json') if rid == 'typescript-6.0.3' else ('',)
        root = selector.extract_archive_subset(archive, WORK / rid, repo['artifact']['root'], prefixes)
        sites = [s for s in selector.collect_sites(root, rid) if identity(s) not in used]
        pool.extend(sites)
        frozen['roots'][rid] = str(root)
        frozen['artifacts'][rid] = repo['artifact']
        frozen['pool_counts'][rid] = len(sites)
    reserve = selector.stratified_sample(pool, 64, 20261002)
    random.Random(20261002).shuffle(reserve)
    write(OUT / 'reserve.json', {'sites': reserve})
    inputs = [ORIGINAL, MANIFEST, OUT / 'policy.md', OUT / 'reserve.json', Path(__file__),
              BASE / 'tools/dispatch_audit_site_selector_typescript.py',
              BASE / 'tools/dispatch_audit_scorer_typescript.py']
    inputs += [OUT.parent / name for name in ['labeling-rubric.md', 'labeling-rubric-addendum.md', 'labeling-rubric-addendum-2.md']]
    frozen['inputs'] = [{'path': str(p.relative_to(BASE)), 'sha256': sha(p)} for p in inputs]
    write(OUT / 'freeze-manifest.json', frozen)
    print(f'froze {len(reserve)} reserve entries without graph output')


if __name__ == '__main__':
    main()
