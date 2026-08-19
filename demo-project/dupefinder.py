"""Find duplicate files in a folder by comparing content hashes."""

import hashlib
import os
import sys


def hash_file(path, chunk_size=65536):
    """Return the SHA-256 hex digest of a file's contents."""
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(chunk_size), b""):
            digest.update(chunk)
    return digest.hexdigest()


def find_duplicates(folder):
    """Map each content hash to the sorted paths sharing it, duplicates only."""
    by_hash = {}
    for root, _dirs, names in os.walk(folder):
        for name in names:
            path = os.path.join(root, name)
            try:
                by_hash.setdefault(hash_file(path), []).append(path)
            except OSError:
                continue
    return {h: sorted(p) for h, p in by_hash.items() if len(p) > 1}


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if not argv:
        print("usage: python3 dupefinder.py <folder>")
        return 1
    duplicates = find_duplicates(argv[0])
    if not duplicates:
        print("no duplicates found")
        return 0
    for digest, paths in sorted(duplicates.items()):
        print(digest[:12] + "  " + str(len(paths)) + " copies")
        for path in paths:
            print("    " + path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
