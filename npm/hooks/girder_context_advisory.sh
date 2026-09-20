#!/bin/sh
# Forward a structured hook payload to the pinned native Girder executable.
# The hook is fail-open: missing/failed binaries and all native errors are
# discarded, while valid native stdout is forwarded unchanged.

binary=${1:-girder}
"$binary" hook 2>/dev/null || true
exit 0
