#!/usr/bin/env bash
set -euo pipefail

result_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd "$result_dir/../../../.." && pwd)
replay_root=${1:-/tmp/girder-competitor-benchmark-modest-final-replay}

(cd "$result_dir" && sha256sum -c artifacts.sha256)
rm -rf "$replay_root"
mkdir -p "$replay_root/extracted" "$replay_root/generated"

args=()
for archive_path in "$result_dir"/raw/*.tar.gz; do
  archive=$(basename "$archive_path")
  campaign=${archive%.tar.gz}
  tar -xzf "$archive_path" -C "$replay_root/extracted"
  root="$replay_root/extracted/$campaign"
  args+=(--result "raw/$archive=$root/result.json")
  args+=(--artifact-root "raw/$archive=$root")
done

cd "$repo_root"
PYTHONPATH=. python3 -m tools.competitor_benchmark.matrix_reporting \
  "${args[@]}" \
  --blocked-observation docs/competitor-benchmark/gitnexus-install-observation.json \
  --setup-friction docs/competitor-benchmark/setup-friction.json \
  --output "$replay_root/generated"

if [[ ${GENERATE_ONLY:-0} != 1 ]]; then
  for generated in summary.json summary.csv aggregate.csv inclusion-manifest.json report.md; do
    cmp "$result_dir/$generated" "$replay_root/generated/$generated"
  done
  echo "Replay matched all committed generated outputs."
fi
