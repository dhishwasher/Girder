#!/usr/bin/env python3
"""Generate auditable JSON, CSV, and Markdown campaign summaries."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import re
from collections import Counter
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence


QUERY_KINDS = ("definition", "callers", "callees", "impact", "tests")
SCORED_STATUSES = {"PASS", "WRONG", "STALE"}
_PROBE_TASK = re.compile(r"^state(?P<state>\d+):probe(?P<probe>\d+):(?P<kind>[^:]+)$")


def _micro_score(records: Iterable[Mapping[str, Any]]) -> dict[str, float | int | None]:
    rows = [row for row in records if row["status"] in SCORED_STATUSES]
    true_positive = sum(int(row.get("score", {}).get("true_positive", 0)) for row in rows)
    false_positive = sum(int(row.get("score", {}).get("false_positive", 0)) for row in rows)
    false_negative = sum(int(row.get("score", {}).get("false_negative", 0)) for row in rows)
    precision = true_positive / (true_positive + false_positive) if true_positive + false_positive else None
    recall = true_positive / (true_positive + false_negative) if true_positive + false_negative else None
    return {
        "precision": precision,
        "recall": recall,
        "true_positive": true_positive,
        "false_positive": false_positive,
        "false_negative": false_negative,
    }


def _last_mutation_probes(records: Sequence[Mapping[str, Any]]) -> list[Mapping[str, Any]]:
    latest: dict[tuple[int, str], tuple[int, Mapping[str, Any]]] = {}
    for row in records:
        match = _PROBE_TASK.match(row["task_id"])
        if not match:
            continue
        key = (int(match.group("state")), match.group("kind"))
        probe = int(match.group("probe"))
        if key not in latest or probe > latest[key][0]:
            latest[key] = (probe, row)
    return [value[1] for _, value in sorted(latest.items())]


def _resolved_artifact(recorded: str, artifact_root: Path | None) -> Path:
    if artifact_root is None:
        return Path(recorded)
    parts = Path(recorded).parts
    try:
        raw_index = parts.index("raw")
    except ValueError as error:
        raise ValueError(f"recorded artifact has no raw path component: {recorded}") from error
    candidate = artifact_root.joinpath(*parts[raw_index:]).resolve()
    root = artifact_root.resolve()
    if not candidate.is_relative_to(root):
        raise ValueError(f"artifact escapes explicit root: {recorded}")
    return candidate


def _assert_raw_accounting(result: Mapping[str, Any], artifact_root: Path | None = None) -> None:
    seen: set[str] = set()
    for row in result["records"]:
        for stream in ("stdout", "stderr"):
            total = 0
            for raw in row[f"{stream}_artifacts"]:
                if raw in seen:
                    raise ValueError(f"raw artifact is referenced twice: {raw}")
                seen.add(raw)
                path = _resolved_artifact(raw, artifact_root)
                if not path.is_file():
                    raise ValueError(f"raw artifact is missing: {raw}")
                total += path.stat().st_size
            if total != row[f"{stream}_bytes"]:
                raise ValueError(
                    f"raw byte mismatch for {row['task_id']} {stream}: "
                    f"recorded {row[f'{stream}_bytes']}, actual {total}"
                )


def summarize_result(path: Path, archive: str, artifact_root: Path | None = None) -> dict[str, Any]:
    raw = path.read_bytes()
    result = json.loads(raw)
    _assert_raw_accounting(result, artifact_root)
    records = result["records"]
    prepare = [row for row in records if row["query_kind"] == "prepare"]
    if len(prepare) != 1:
        raise ValueError(f"expected one prepare record in {path}")
    warm = [row for row in records if row["task_id"].startswith("state0:warm_query:")]
    if sorted(row["query_kind"] for row in warm) != sorted(QUERY_KINDS):
        raise ValueError(f"warm query set is incomplete in {path}")
    queries = [row for row in records if row["query_kind"] in QUERY_KINDS]
    last_probes = _last_mutation_probes(records)
    rss = [row["peak_rss_bytes"] for row in records if isinstance(row.get("peak_rss_bytes"), int)]
    successful_latencies = [
        row["update_to_correct_seconds"] for row in result.get("mutation_summaries", [])
        if row.get("update_to_correct_seconds") is not None
    ]
    base_by_kind = {kind: [row for row in warm if row["query_kind"] == kind] for kind in QUERY_KINDS}
    freshness_by_kind = {
        kind: dict(sorted(Counter(row["status"] for row in last_probes if row["query_kind"] == kind).items()))
        for kind in QUERY_KINDS
    }
    return {
        "product": result["product"],
        "version": prepare[0]["version"],
        "commit": prepare[0]["commit"],
        "fixture": result["fixture"],
        "campaign_state": result["campaign_state"],
        "prepare_success": prepare[0]["status"] == "PASS",
        "cold_setup_seconds": prepare[0]["elapsed_seconds"],
        "campaign_seconds": result["elapsed_seconds"],
        "warm_query_seconds_total": sum(row["elapsed_seconds"] for row in warm),
        "warm_query_seconds_mean": sum(row["elapsed_seconds"] for row in warm) / len(warm),
        "base_status_by_kind": {kind: base_by_kind[kind][0]["status"] for kind in QUERY_KINDS},
        "base_score_by_kind": {kind: _micro_score(base_by_kind[kind]) for kind in QUERY_KINDS},
        "freshness_status_by_kind": freshness_by_kind,
        "mutation_terminal_status": dict(sorted(Counter(
            row["terminal_status"] for row in result.get("mutation_summaries", [])
        ).items())),
        "update_to_all_correct_seconds_mean": (
            sum(successful_latencies) / len(successful_latencies) if successful_latencies else None
        ),
        "query_status": dict(sorted(Counter(row["status"] for row in queries).items())),
        "query_response_bytes": sum(row["stdout_bytes"] + row["stderr_bytes"] for row in queries),
        "query_tool_calls": sum(row["tool_calls"] for row in queries),
        "query_record_count": len(queries),
        "calls_per_query_record": sum(row["tool_calls"] for row in queries) / len(queries),
        "peak_rss_bytes": max(rss) if rss else None,
        "source_result_sha256": hashlib.sha256(raw).hexdigest(),
        "raw_archive": archive,
        "policy_id": result["policy_id"],
        "policy_sha256": result["policy_sha256"],
        "freeze_manifest_sha256": result["freeze_manifest_sha256"],
        "harness_commit": result["harness_commit"],
    }


def _ratio(value: float | None) -> str:
    return "unavailable" if value is None else f"{value:.3f}"


def _seconds(value: float | None) -> str:
    return "unavailable" if value is None else f"{value:.3f}"


def _base_pair(row: Mapping[str, Any], kind: str) -> str:
    score = row["base_score_by_kind"][kind]
    return f"{row['base_status_by_kind'][kind]}; {_ratio(score['precision'])}/{_ratio(score['recall'])}"


def _counts(values: Mapping[str, int]) -> str:
    return ", ".join(f"{key} {value}" for key, value in sorted(values.items())) or "none"


def render_markdown(summary: Mapping[str, Any], title: str, scope_note: str) -> str:
    rows = summary["campaigns"]
    harness_commits = sorted({row["harness_commit"] for row in rows})
    table = [
        "| Product | Cold setup (s) | Warm five-query total (s) | Definition | Callers P/R | Callees P/R | Impact P/R | Tests P/R | Mutation terminal | Query bytes | Calls | Peak RSS (bytes) |",
        "|---|---:|---:|---|---|---|---|---|---|---:|---:|---:|",
    ]
    for row in rows:
        mutation = ", ".join(f"{key} {value}" for key, value in row["mutation_terminal_status"].items()) or "none"
        table.append(
            f"| {row['product']} {row['version']} | {_seconds(row['cold_setup_seconds'])} | "
            f"{_seconds(row['warm_query_seconds_total'])} | {row['base_status_by_kind']['definition']} | "
            f"{_base_pair(row, 'callers')} | {_base_pair(row, 'callees')} | "
            f"{_base_pair(row, 'impact')} | {_base_pair(row, 'tests')} | {mutation} | "
            f"{row['query_response_bytes']} | {row['query_tool_calls']} | {row['peak_rss_bytes'] or 'unavailable'} |"
        )
    girder = next((row for row in rows if row["product"] == "girder"), None)
    watch = next((row for row in rows if row["product"] == "girder-watch"), None)
    ripwire = next((row for row in rows if row["product"] == "ripwire"), None)
    losses = []
    wins = []
    if girder and ripwire:
        losses.append(
            f"Ripwire's base test selection recalled {_ratio(ripwire['base_score_by_kind']['tests']['recall'])} "
            f"versus Girder's {_ratio(girder['base_score_by_kind']['tests']['recall'])}. Ripwire selected the whole "
            "test file, so that recall came with an unrelated false positive."
        )
        losses.append(
            f"Girder normal used {girder['query_tool_calls']} tool calls across the recorded query attempts; "
            f"Ripwire used {ripwire['query_tool_calls']}."
        )
        reduction = 1 - girder["query_response_bytes"] / ripwire["query_response_bytes"]
        correctness = (
            f"Both had the same {_counts(girder['query_status'])} query-status counts."
            if girder["query_status"] == ripwire["query_status"]
            else f"Their query-status counts differed: Girder {_counts(girder['query_status'])}; "
                 f"Ripwire {_counts(ripwire['query_status'])}."
        )
        wins.append(
            f"Girder normal returned {girder['query_response_bytes']} query-response bytes versus "
            f"Ripwire's {ripwire['query_response_bytes']} ({reduction:.1%} fewer). {correctness}"
        )
        wins.append(
            f"Girder's base test answer had precision {_ratio(girder['base_score_by_kind']['tests']['precision'])}; "
            f"Ripwire's was {_ratio(ripwire['base_score_by_kind']['tests']['precision'])}."
        )
    if watch and girder:
        losses.append(
            f"Girder watch took {watch['campaign_seconds']:.3f} seconds for the campaign versus "
            f"{girder['campaign_seconds']:.3f} seconds for normal Girder on this tiny fixture."
        )
    campaign_states = ", ".join(f"{row['product']}={row['campaign_state']}" for row in rows)
    mutation_count = sum(sum(row["mutation_terminal_status"].values()) for row in rows)
    mutation_terminals = Counter()
    for row in rows:
        mutation_terminals.update(row["mutation_terminal_status"])
    status_sentence = f"Campaign states: {campaign_states}."
    if rows and all(row["base_status_by_kind"]["definition"] == "PASS" for row in rows):
        status_sentence += " Every base warmed definition query passed."
    if rows and all(row["base_status_by_kind"]["callees"] == "PASS" for row in rows):
        status_sentence += " Every base warmed direct-callee query passed."
    mutation_sentence = (
        f"The {mutation_count} mutation summaries ended with {_counts(mutation_terminals)}. "
        "Update-to-all-correct latency is unavailable wherever the terminal status was not PASS."
    )
    boundaries = []
    fixtures = {row["fixture"] for row in rows}
    if fixtures == {"tiny-python"}:
        boundaries.extend([
            "The tiny Python fixture is an adapter and scoring gate. Its corpus entry excludes it from the final competitive aggregate.",
            "These results do not establish behavior on the committed modest Rust, Python, TypeScript/TSX, or Go fixtures, or on large repositories.",
        ])
    else:
        boundaries.append("These results do not establish behavior on fixtures or repository scales outside the supplied campaigns.")
    exceptional = Counter()
    for row in rows:
        exceptional.update({
            status: count for status, count in row["query_status"].items()
            if status in {"UNSUPPORTED", "TIMEOUT", "RESOURCE_BLOCKED", "INSTALL_FAILED", "ERROR"}
        })
    exceptional_sentence = (
        f"The supplied runs recorded {_counts(exceptional)} exceptional query statuses; none is converted into a win."
        if exceptional else
        "The result does not turn `UNSUPPORTED`, `TIMEOUT`, `RESOURCE_BLOCKED`, `INSTALL_FAILED`, or `ERROR` into a win. None occurred in the supplied runs."
    )
    return "\n".join([
        f"# {title}", "", scope_note, "",
        "Every precision/recall pair is shown as `precision/recall`. Base correctness uses exactly one warmed "
        "query of each kind. Operational byte and call totals include warmup, the measured warm query, and every "
        "freshness probe; failed probes remain in those totals. No tokenizer was run.", "",
        *table, "",
        status_sentence, "", mutation_sentence, "",
        "## Where Girder Lost", "", *(f"- {item}" for item in (losses or ["No Girder comparison pair was supplied."])), "",
        "## Where Girder Won", "", *(f"- {item}" for item in (wins or ["No Girder comparison pair was supplied."])), "",
        "## What This Benchmark Does NOT Establish", "",
        *(f"- {item}" for item in boundaries),
        "- They do not establish universal speed, memory, context cost, or semantic coverage. They measure one low-resource Chromebook container.",
        "- Response sizes are UTF-8 bytes actually delivered by native tools. They are not token counts or complete coding-agent session costs.",
        "- A matching PASS/WRONG count does not mean the products returned the same wrong answers; the raw records retain each answer and oracle comparison.",
        f"- {exceptional_sentence}", "",
        "## Reproduction", "",
        "[`../../README.md`](../../README.md) and the current [`../../policy.json`](../../policy.json) describe the "
        "protocol and its revision history. Each campaign row in `summary.json` pins its product version and "
        "commit, harness commit, policy hashes, result SHA-256, and raw archive. Recover the exact policy used by "
        f"these inputs with `git show {harness_commits[0]}:docs/competitor-benchmark/policy.json` and verify the "
        "recorded policy SHA-256." if len(harness_commits) == 1 else
        "The supplied rows use multiple harness commits; recover each exact policy with `git show <harness_commit>:docs/competitor-benchmark/policy.json` and verify its recorded SHA-256.", "",
    ])


def write_outputs(campaigns: Sequence[Mapping[str, Any]], output: Path, title: str, scope_note: str) -> None:
    output.mkdir(parents=True, exist_ok=True)
    summary = {"schema_version": 1, "scope": title, "campaigns": list(campaigns)}
    (output / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    fields = [
        "product", "version", "commit", "fixture", "campaign_state", "prepare_success",
        "cold_setup_seconds", "campaign_seconds", "warm_query_seconds_total", "warm_query_seconds_mean",
        "definition_status", "callers_precision", "callers_recall", "callees_precision", "callees_recall",
        "impact_precision", "impact_recall", "tests_precision", "tests_recall", "mutation_terminal_status",
        "update_to_all_correct_seconds_mean", "query_response_bytes", "query_tool_calls", "query_record_count",
        "calls_per_query_record", "peak_rss_bytes", "source_result_sha256", "raw_archive", "policy_id",
        "policy_sha256", "freeze_manifest_sha256", "harness_commit",
    ]
    with (output / "summary.csv").open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        for row in campaigns:
            flat = {key: row.get(key) for key in fields}
            flat.update({
                "definition_status": row["base_status_by_kind"]["definition"],
                "mutation_terminal_status": json.dumps(row["mutation_terminal_status"], sort_keys=True),
            })
            for kind in ("callers", "callees", "impact", "tests"):
                flat[f"{kind}_precision"] = row["base_score_by_kind"][kind]["precision"]
                flat[f"{kind}_recall"] = row["base_score_by_kind"][kind]["recall"]
            writer.writerow(flat)
    (output / "report.md").write_text(render_markdown(summary, title, scope_note), encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--result", action="append", required=True, metavar="ARCHIVE=RESULT_JSON")
    parser.add_argument(
        "--artifact-root", action="append", default=[], metavar="ARCHIVE=EXTRACTED_CAMPAIGN_ROOT",
        help="resolve recorded raw paths under an extracted campaign root instead of their original absolute paths",
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--title", required=True)
    parser.add_argument("--scope-note", required=True)
    args = parser.parse_args()
    artifact_roots = {}
    for spec in args.artifact_root:
        archive, separator, root = spec.partition("=")
        if not separator:
            parser.error("--artifact-root must be ARCHIVE=EXTRACTED_CAMPAIGN_ROOT")
        artifact_roots[archive] = Path(root)
    campaigns = []
    for spec in args.result:
        archive, separator, source = spec.partition("=")
        if not separator:
            parser.error("--result must be ARCHIVE=RESULT_JSON")
        campaigns.append(summarize_result(Path(source), archive, artifact_roots.get(archive)))
    write_outputs(campaigns, args.output, args.title, args.scope_note)


if __name__ == "__main__":
    main()
