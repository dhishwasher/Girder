# GitNexus setup result

GitNexus 1.6.11 could not be reproducibly installed within this benchmark's frozen Chromebook limits. This is a host-specific `RESOURCE_BLOCKED` setup result. It is not a Girder win, and GitNexus receives no correctness, query-cost, freshness, or memory score.

The committed npm lock pins `gitnexus@1.6.11` with integrity `sha512-+UdZPqmRSIc0dwZea43uHVOPhtnfBk/3iRj7WDzfqTPGPpQ8EcqcsBKbA07uySOjS3um/zLJIGOn8KfM0ZFh4g==`. Acquisition used Node v22.23.1 and npm 10.9.8 with this exact command:

```sh
GITNEXUS_SKIP_OPTIONAL_GRAMMARS=1 \
CARGO_BUILD_JOBS=1 CMAKE_BUILD_PARALLEL_LEVEL=1 MAKEFLAGS=-j1 \
RAYON_NUM_THREADS=1 npm_config_jobs=1 \
npm ci --omit=dev --no-audit --no-fund
```

The npm registry was needed for package acquisition. No API key or product account was required. No compiler completion was observed, and none is inferred.

## Retained attempts

| Attempt | Placement | Result | Time (s) | Peak RSS | Reason |
| --- | --- | --- | ---: | ---: | --- |
| v1 | ChromeOS removable 9p | `ERROR` | 414.886 | 61,153,280 bytes | npm could not create the required `node_modules/.bin/arrow2csv` symlink: `EACCES` |
| v2 | Fresh local temporary directory | invalidated false `RESOURCE_BLOCKED` | 123.779 | impossible 7,949,430,816,768-byte reading | The benchmark split `/proc/<pid>/stat` on spaces, so a process name shifted the RSS field |
| v3 | Fresh local temporary directory after revision 11 | `RESOURCE_BLOCKED` | 38.085 | 1,076,068,352 bytes | Corrected process-tree RSS exceeded the frozen 1,073,741,824-byte cap |

Revision 11 fixed the harness parser and added a process-name regression test before v3. The v3 reading exceeded the cap by 2,326,528 bytes while the supervisor kept the system above its emergency headroom floor. The benchmark therefore stopped as required. It did not increase the memory limit, add swap, remove more dependencies, or run an uncommitted alternative installation.

The [structured observation](gitnexus-install-observation.json), [artifact manifest](gitnexus-preflight-manifest.json), and [raw install artifacts](preflight-artifacts/gitnexus) preserve all three attempts. Partial temporary dependency trees were removed after their raw command, output, hashes, status, timing, and peak measurements were retained.

This result establishes only that the pinned installation did not complete under this host and protocol. A machine with more memory could install the same lock and run the already frozen native-query design, but no behavior beyond installation is claimed here.
