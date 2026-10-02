# Pinned archive acquisition

The first acquisition attempt stopped at the TypeScript archive's HTTP 302:
the shared benchmark downloader deliberately rejects redirects. Its traceback
is preserved in `acquisition-attempt-1.txt`; no site selection or measurement
ran on that attempt.

The acquisition helper now requests the exact GitHub codeload tag endpoint
directly, retaining redirect rejection and the original manifest's byte count
and SHA-256 checks. Repository versions, archive identity and scoring policy
are unchanged. The benchmark downloader itself was not edited. Acquisition is
a development preparation step; selection and measured workloads remain offline.

The direct request also failed the shared helper's mandatory Content-Length
check: GitHub's response omitted that header. `acquisition-attempt-2.txt` retains
the failure. The TypeScript-only acquisition routine now accepts an absent
header while bounding bytes during streaming and requiring the exact pinned
final size and SHA before installing an archive in the cache. A present,
incorrect header still fails. Tests exercise exact, truncated, oversized, and
same-size/wrong-hash bodies. No shared downloader or archive pin was changed.
