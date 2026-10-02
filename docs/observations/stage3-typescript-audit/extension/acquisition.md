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
