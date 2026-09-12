#!/usr/bin/env python3
"""Generate the final cross-language benchmark aggregate without rescoring runs."""

from __future__ import annotations

import argparse
import csv
import json
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Mapping, Sequence

from .reporting import QUERY_KINDS, summarize_result


FIXTURES = ("modest-rust", "modest-python", "modest-typescript-tsx", "modest-go")
RUNNABLE_PRODUCTS = ("girder", "girder-watch", "ripwire", "codebase-memory-mcp", "code-review-graph")


def _ratio(numerator: int, denominator: int) -> float | None:
    return numerator / denominator if denominator else None


def _score(rows: Sequence[Mapping[str, Any]], kind: str) -> dict[str, float | int | None]:
    tp = sum(int(row["base_score_by_kind"][kind]["true_positive"]) for row in rows)
    fp = sum(int(row["base_score_by_kind"][kind]["false_positive"]) for row in rows)
    fn = sum(int(row["base_score_by_kind"][kind]["false_negative"]) for row in rows)
    return {
        "precision": _ratio(tp, tp + fp), "recall": _ratio(tp, tp + fn),
        "true_positive": tp, "false_positive": fp, "false_negative": fn,
    }


def validate_matrix(campaigns: Sequence[Mapping[str, Any]]) -> None:
    seen = Counter((row["product"], row["fixture"]) for row in campaigns)
    expected = {(product, fixture) for product in RUNNABLE_PRODUCTS for fixture in FIXTURES}
    if set(seen) != expected:
        missing = sorted(expected - set(seen))
        extra = sorted(set(seen) - expected)
        raise ValueError(f"final matrix mismatch; missing={missing}, extra={extra}")
    duplicates = sorted(key for key, count in seen.items() if count != 1)
    if duplicates:
        raise ValueError(f"duplicate final matrix rows: {duplicates}")
    incomplete = sorted(
        (row["product"], row["fixture"]) for row in campaigns if row["campaign_state"] != "COMPLETE"
    )
    if incomplete:
        raise ValueError(f"incomplete campaigns cannot enter final matrix: {incomplete}")


def aggregate_campaigns(campaigns: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    validate_matrix(campaigns)
    grouped: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
    for row in campaigns:
        grouped[row["product"]].append(row)
    aggregates = []
    for product in RUNNABLE_PRODUCTS:
        rows = sorted(grouped[product], key=lambda row: FIXTURES.index(row["fixture"]))
        base = Counter()
        terminal_queries = Counter()
        mutations = Counter()
        for row in rows:
            base.update(row["base_task_status"])
            terminal_queries.update(row["freshness_terminal_query_status"])
            mutations.update(row["mutation_terminal_status"])
        latency_count = sum(row["update_to_all_correct_count"] for row in rows)
        latency_weighted_sum = sum(
            row["update_to_all_correct_seconds_mean"] * row["update_to_all_correct_count"]
            for row in rows if row["update_to_all_correct_seconds_mean"] is not None
        )
        aggregates.append({
            "product": product,
            "version": rows[0]["version"],
            "commit": rows[0]["commit"],
            "install_status": "PASS",
            "campaigns_complete": len(rows),
            "cold_setup_seconds_mean": sum(row["cold_setup_seconds"] for row in rows) / len(rows),
            "warm_five_query_seconds_mean": sum(row["warm_query_seconds_total"] for row in rows) / len(rows),
            "base_task_status": dict(sorted(base.items())),
            "definition_passes": sum(row["base_status_by_kind"]["definition"] == "PASS" for row in rows),
            "base_score_by_kind": {kind: _score(rows, kind) for kind in QUERY_KINDS[1:]},
            "freshness_terminal_query_status": dict(sorted(terminal_queries.items())),
            "mutation_terminal_status": dict(sorted(mutations.items())),
            "update_to_all_correct_count": latency_count,
            "update_to_all_correct_seconds_mean": (
                latency_weighted_sum / latency_count if latency_count else None
            ),
            "query_response_bytes": sum(row["query_response_bytes"] for row in rows),
            "query_tool_calls": sum(row["query_tool_calls"] for row in rows),
            "query_record_count": sum(row["query_record_count"] for row in rows),
            "calls_per_query_record": (
                sum(row["query_tool_calls"] for row in rows) /
                sum(row["query_record_count"] for row in rows)
            ),
            "peak_rss_bytes": max(row["peak_rss_bytes"] for row in rows if row["peak_rss_bytes"] is not None),
            "policy_ids": sorted({row["policy_id"] for row in rows}),
            "harness_commits": sorted({row["harness_commit"] for row in rows}),
        })
    return aggregates


def matched_all_pass_cost(
    campaigns: Sequence[Mapping[str, Any]], left: str, right: str, kind: str
) -> dict[str, Any]:
    by_key = {(row["product"], row["fixture"]): row for row in campaigns}
    pairs = []
    for fixture in FIXTURES:
        left_row = by_key[(left, fixture)]
        right_row = by_key[(right, fixture)]
        left_cost = left_row["query_cost_by_kind"][kind]
        right_cost = right_row["query_cost_by_kind"][kind]
        if set(left_cost["status"]) == {"PASS"} and set(right_cost["status"]) == {"PASS"}:
            pairs.append((fixture, left_cost, right_cost))
    return {
        "left": left, "right": right, "query_kind": kind,
        "fixtures": [pair[0] for pair in pairs], "comparable_pairs": len(pairs),
        "left_response_bytes": sum(pair[1]["response_bytes"] for pair in pairs),
        "right_response_bytes": sum(pair[2]["response_bytes"] for pair in pairs),
        "left_tool_calls": sum(pair[1]["tool_calls"] for pair in pairs),
        "right_tool_calls": sum(pair[2]["tool_calls"] for pair in pairs),
        "left_query_records": sum(pair[1]["query_records"] for pair in pairs),
        "right_query_records": sum(pair[2]["query_records"] for pair in pairs),
    }


def _fmt(value: float | None) -> str:
    return "unavailable" if value is None else f"{value:.3f}"


def _counts(values: Mapping[str, int]) -> str:
    return ", ".join(f"{key} {value}" for key, value in sorted(values.items())) or "none"


def _pair(score: Mapping[str, Any]) -> str:
    return f"{_fmt(score['precision'])}/{_fmt(score['recall'])}"


def render_markdown(
    campaigns: Sequence[Mapping[str, Any]], aggregates: Sequence[Mapping[str, Any]],
    blocked: Mapping[str, Any], setup: Mapping[str, Any], matched: Mapping[str, Any],
) -> str:
    aggregate_by_product = {row["product"]: row for row in aggregates}
    campaign_by_key = {(row["product"], row["fixture"]): row for row in campaigns}
    girder = aggregate_by_product["girder"]
    watch = aggregate_by_product["girder-watch"]
    ripwire = aggregate_by_product["ripwire"]
    codebase = aggregate_by_product["codebase-memory-mcp"]
    girder_tsx_tests = campaign_by_key[("girder", "modest-typescript-tsx")]["base_score_by_kind"]["tests"]
    ripwire_tsx_tests = campaign_by_key[("ripwire", "modest-typescript-tsx")]["base_score_by_kind"]["tests"]
    overall = [
        "| Product | Install | Cold mean (s) | Warm five-query mean (s) | Base exact PASS/20 | Definition PASS/4 | Callers P/R | Callees P/R | Impact P/R | Tests P/R | Final mutation queries | Mutation summaries | Update to all-correct | Query bytes | Calls/query record | Peak RSS (bytes) |",
        "|---|---|---:|---:|---:|---:|---|---|---|---|---|---|---:|---:|---:|---:|",
    ]
    for row in aggregates:
        overall.append(
            f"| {row['product']} {row['version']} | {row['install_status']} | "
            f"{row['cold_setup_seconds_mean']:.3f} | {row['warm_five_query_seconds_mean']:.3f} | "
            f"{row['base_task_status'].get('PASS', 0)}/20 | {row['definition_passes']}/4 | "
            f"{_pair(row['base_score_by_kind']['callers'])} | {_pair(row['base_score_by_kind']['callees'])} | "
            f"{_pair(row['base_score_by_kind']['impact'])} | {_pair(row['base_score_by_kind']['tests'])} | "
            f"{_counts(row['freshness_terminal_query_status'])} | {_counts(row['mutation_terminal_status'])} | "
            f"{_fmt(row['update_to_all_correct_seconds_mean'])} | {row['query_response_bytes']} | "
            f"{row['calls_per_query_record']:.3f} | {row['peak_rss_bytes']} |"
        )
    overall.append(
        f"| {blocked['product']} {blocked['version']} | {blocked['final_status']} | unavailable | unavailable | "
        "unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | "
        f"unavailable | unavailable | unavailable | unavailable | {blocked['attempts'][-1]['peak_rss_bytes']} |"
    )

    language = [
        "| Product | Fixture | Definition | Callers P/R | Callees P/R | Impact P/R | Tests P/R | Base status | Final mutation queries | Mutation summaries | Query bytes | Calls | Peak RSS |",
        "|---|---|---|---|---|---|---|---|---|---|---:|---:|---:|",
    ]
    for row in sorted(campaigns, key=lambda item: (RUNNABLE_PRODUCTS.index(item["product"]), FIXTURES.index(item["fixture"]))):
        language.append(
            f"| {row['product']} | {row['fixture']} | {row['base_status_by_kind']['definition']} | "
            f"{_pair(row['base_score_by_kind']['callers'])} | {_pair(row['base_score_by_kind']['callees'])} | "
            f"{_pair(row['base_score_by_kind']['impact'])} | {_pair(row['base_score_by_kind']['tests'])} | "
            f"{_counts(row['base_task_status'])} | {_counts(row['freshness_terminal_query_status'])} | "
            f"{_counts(row['mutation_terminal_status'])} | {row['query_response_bytes']} | "
            f"{row['query_tool_calls']} | {row['peak_rss_bytes']} |"
        )

    setup_table = [
        "| Product | Outcome | Distribution/build | Network/account | Measured friction |",
        "|---|---|---|---|---|",
    ]
    for item in setup["products"]:
        setup_table.append(
            f"| {item['product']} {item['version']} | {item['outcome']} | {item['distribution']} | "
            f"{item['network_and_account']} | {item['measured_friction']} |"
        )

    reduction = 1 - matched["left_response_bytes"] / matched["right_response_bytes"]
    return "\n".join([
        "# Modest cross-language competitive benchmark", "",
        "This is the final aggregate of 20 complete modest-fixture campaigns: twelve external-product campaigns "
        "under frozen revision 11 and eight corrected Girder campaigns under revision 12. The eight Girder "
        "revision 11 campaigns are preserved and excluded. Tiny-fixture adapter gates are reported separately.", "",
        "`COMPLETE` means the campaign executed to the end. It does not mean its answers were correct. Base results "
        "count 20 distinct tasks per runnable product: five query kinds across four languages. Final mutation-query "
        "status counts use the last probe for 120 distinct tasks per product: five query kinds after each of six "
        "mutations across four languages. Precision/recall is micro-aggregated from the four base tasks for that "
        "query kind. Operational totals include every warmup, measured query, retry, and error response. Response "
        "sizes are UTF-8 bytes; no tokenizer was run.", "",
        "## Overall comparison", "", *overall, "",
        "No product reached a fully correct answer after any mutation. Update-to-all-correct latency is therefore "
        "unavailable for every runnable product. Codebase-memory-mcp's COMPLETE campaigns include terminal native "
        "errors after rename; those errors remain separate from wrong parseable answers.", "",
        "## Per-language results", "", *language, "",
        "## Setup friction", "", *setup_table, "",
        f"GitNexus is a host-specific `{blocked['final_status']}` setup result. Its corrected exact-lock installation "
        f"peaked at {blocked['attempts'][-1]['peak_rss_bytes']} bytes, above the frozen "
        f"{blocked['attempts'][-1]['limit_bytes']} byte cap, and was stopped. It receives no correctness, cost, "
        "freshness, or comparative score and is not counted as a Girder win.", "",
        "## Where Girder Lost", "",
        f"- Girder normal and watch each passed {girder['base_task_status'].get('PASS', 0)} of 20 warmed base "
        f"exact-set tasks. Ripwire passed {ripwire['base_task_status'].get('PASS', 0)} of 20 and "
        f"codebase-memory-mcp passed {codebase['base_task_status'].get('PASS', 0)} of 20.",
        "- On Go, Girder's native response returned the correct declaration source but only the root identity "
        "`crate::calculateTotal`; without a file component, the frozen adapter could not produce the required "
        "file-qualified identity. The Go definition was therefore WRONG. Ripwire, codebase-memory-mcp, and "
        "code-review-graph passed that base definition task.",
        f"- Girder's TypeScript/TSX base test answer had precision/recall {_pair(girder_tsx_tests)}. Ripwire's "
        f"was {_pair(ripwire_tsx_tests)} "
        "because its whole-file selection found both relevant tests plus one unrelated test.",
        f"- Girder normal used {girder['query_tool_calls']} calls across its recorded query attempts, versus "
        f"Ripwire's {ripwire['query_tool_calls']}. Watch mode used {watch['query_tool_calls']} calls and "
        "more response bytes than normal mode.",
        "- Neither Girder mode produced an all-correct mutation summary. The frozen function-parameter callback "
        "remained absent from callers, reverse impact, and test selection.", "",
        "## Where Girder Won", "",
        f"- For exact-definition attempts where both Girder normal and Ripwire were correct throughout, the matched "
        f"Rust, Python, and TypeScript/TSX pairs contained {matched['left_query_records']} query records per product. "
        f"Girder returned {matched['left_response_bytes']} bytes versus Ripwire's {matched['right_response_bytes']} "
        f"({reduction:.1%} fewer), with {matched['left_tool_calls']} calls each. The Go pair is excluded because "
        "Girder's file-qualified identity was wrong.",
        f"- Girder normal's measured peak process-tree RSS was {girder['peak_rss_bytes']} bytes across the four "
        "campaigns, lower "
        "than every successfully tested external product in this matrix. This is host- and adapter-specific.",
        f"- Girder and Girder watch returned parseable results after every mutation. Codebase-memory-mcp ended "
        f"{codebase['freshness_terminal_query_status'].get('ERROR', 0)} of 120 terminal mutation queries with "
        f"ERROR. Parseable does not imply correct: Girder still had "
        f"{girder['freshness_terminal_query_status'].get('WRONG', 0)} WRONG terminal queries.", "",
        "## What This Benchmark Does NOT Establish", "",
        "- It does not establish an overall best product. No runnable product achieved complete base or mutation correctness.",
        "- It does not establish production-repository behavior. The corpus contains deterministic modest fixtures on one low-resource Chromebook container.",
        "- It does not establish token savings or full coding-agent session cost. Only delivered UTF-8 tool-response bytes, calls, and wall time were measured.",
        "- It does not establish that GitNexus would fail on a machine with more memory; GitNexus never reached semantic measurement here.",
        "- It does not establish that Girder failed to retrieve the Go declaration source. It failed the stricter file-qualified identity requirement through this interface.",
        "- It does not establish complete caller, test, or blast-radius correctness. The frozen dynamic-dispatch case defeated every tested product in at least one required answer.",
        "- Total campaign bytes and times are operational counters. They are not standalone efficiency wins when correctness, retries, or error termination differ.", "",
        "## Integrity and provenance", "",
        "The inclusion manifest names all 20 included campaign archives, source-result hashes, policy IDs, freeze-manifest hashes, and harness commits. "
        "Every raw artifact byte count was checked before aggregation. All archives were created from local file-by-file copies, avoiding the removable 9p mount's observed direct-tar corruption. "
        "The twelve external rows retain revision 11 because their adapters and results were unaffected; Girder rows use revision 12 after the Rust identity correction. "
        "The preserved invalid archive and preflight log disclose the reason for exclusion.", "",
        "## Reproduction", "",
        "Run the commands in `../../README.md` one product at a time with fresh work and output paths. Then invoke "
        "`python3 -m tools.competitor_benchmark.matrix_reporting` with the 20 `--result` and matching "
        "`--artifact-root` arguments recorded in `reproduce-report.sh`. `artifacts.sha256` verifies each raw archive. "
        "Extracting those archives and running `reproduce-report.sh` regenerates `summary.json`, `summary.csv`, "
        "`aggregate.csv`, `inclusion-manifest.json`, and this report byte-for-byte.", "",
    ])


def write_outputs(
    campaigns: Sequence[Mapping[str, Any]], blocked: Mapping[str, Any], setup: Mapping[str, Any], output: Path
) -> None:
    aggregates = aggregate_campaigns(campaigns)
    matched = matched_all_pass_cost(campaigns, "girder", "ripwire", "definition")
    output.mkdir(parents=True, exist_ok=True)
    summary = {
        "schema_version": 2,
        "scope": "modest cross-language final aggregate",
        "campaigns": list(campaigns),
        "aggregates": aggregates,
        "blocked": [blocked],
        "matched_all_pass_cost": [matched],
    }
    (output / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")

    fields = [
        "product", "version", "commit", "fixture", "campaign_state", "cold_setup_seconds",
        "warm_query_seconds_total", "base_pass", "base_wrong", "definition_status", "callers_precision",
        "callers_recall", "callees_precision", "callees_recall", "impact_precision", "impact_recall",
        "tests_precision", "tests_recall", "terminal_query_status", "mutation_terminal_status",
        "update_to_all_correct_seconds_mean", "query_response_bytes", "query_tool_calls", "query_record_count",
        "calls_per_query_record", "peak_rss_bytes", "source_result_sha256", "raw_archive", "policy_id",
        "policy_sha256", "freeze_manifest_sha256", "harness_commit",
    ]
    with (output / "summary.csv").open("w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        for row in campaigns:
            flat = {key: row.get(key) for key in fields}
            flat.update({
                "base_pass": row["base_task_status"].get("PASS", 0),
                "base_wrong": row["base_task_status"].get("WRONG", 0),
                "definition_status": row["base_status_by_kind"]["definition"],
                "terminal_query_status": json.dumps(row["freshness_terminal_query_status"], sort_keys=True),
                "mutation_terminal_status": json.dumps(row["mutation_terminal_status"], sort_keys=True),
            })
            for kind in QUERY_KINDS[1:]:
                flat[f"{kind}_precision"] = row["base_score_by_kind"][kind]["precision"]
                flat[f"{kind}_recall"] = row["base_score_by_kind"][kind]["recall"]
            writer.writerow(flat)

    aggregate_fields = [
        "product", "version", "commit", "install_status", "campaigns_complete", "cold_setup_seconds_mean",
        "warm_five_query_seconds_mean", "base_task_status", "definition_passes", "callers_precision",
        "callers_recall", "callees_precision", "callees_recall", "impact_precision", "impact_recall",
        "tests_precision", "tests_recall", "freshness_terminal_query_status", "mutation_terminal_status",
        "update_to_all_correct_seconds_mean", "query_response_bytes", "query_tool_calls", "query_record_count",
        "calls_per_query_record", "peak_rss_bytes", "policy_ids", "harness_commits",
    ]
    with (output / "aggregate.csv").open("w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=aggregate_fields, lineterminator="\n")
        writer.writeheader()
        for row in aggregates:
            flat = {key: row.get(key) for key in aggregate_fields}
            for key in ("base_task_status", "freshness_terminal_query_status", "mutation_terminal_status", "policy_ids", "harness_commits"):
                flat[key] = json.dumps(flat[key], sort_keys=True)
            for kind in QUERY_KINDS[1:]:
                flat[f"{kind}_precision"] = row["base_score_by_kind"][kind]["precision"]
                flat[f"{kind}_recall"] = row["base_score_by_kind"][kind]["recall"]
            writer.writerow(flat)

    included = [{
        key: row[key] for key in (
            "product", "version", "commit", "fixture", "raw_archive", "source_result_sha256", "policy_id",
            "policy_sha256", "freeze_manifest_sha256", "harness_commit",
        )
    } for row in campaigns]
    inclusion = {
        "schema_version": 1,
        "included_campaigns": included,
        "blocked_setup_only": [{
            "product": blocked["product"], "version": blocked["version"],
            "status": blocked["final_status"], "comparative_result": False,
            "observation": "../../gitnexus-install-observation.json",
        }],
        "excluded": [
            {"scope": "tiny-python campaigns", "reason": "adapter gates, excluded by the frozen corpus", "evidence": "../tiny"},
            {"scope": "eight Girder revision 11 modest campaigns", "reason": "Rust crate-path adapter normalization defect", "evidence": "../../invalid-campaign-artifacts/modest-girder-revision11-normalization.tar.gz"},
            {"scope": "all invalid, interrupted, and preflight-only attempts", "reason": "not comparative results", "evidence": "../../preflight-log.json"},
        ],
    }
    (output / "inclusion-manifest.json").write_text(json.dumps(inclusion, indent=2, sort_keys=True) + "\n")
    (output / "report.md").write_text(render_markdown(campaigns, aggregates, blocked, setup, matched))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--result", action="append", required=True, metavar="ARCHIVE=RESULT_JSON")
    parser.add_argument("--artifact-root", action="append", default=[], metavar="ARCHIVE=ROOT")
    parser.add_argument("--blocked-observation", type=Path, required=True)
    parser.add_argument("--setup-friction", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    roots = {}
    for spec in args.artifact_root:
        archive, separator, root = spec.partition("=")
        if not separator:
            parser.error("--artifact-root must be ARCHIVE=ROOT")
        roots[archive] = Path(root)
    campaigns = []
    for spec in args.result:
        archive, separator, result = spec.partition("=")
        if not separator:
            parser.error("--result must be ARCHIVE=RESULT_JSON")
        campaigns.append(summarize_result(Path(result), archive, roots.get(archive)))
    blocked = json.loads(args.blocked_observation.read_text())
    setup = json.loads(args.setup_friction.read_text())
    write_outputs(campaigns, blocked, setup, args.output)


if __name__ == "__main__":
    main()
