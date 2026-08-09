#!/usr/bin/env python3
"""Reproducible six-repository indexing and semantic-graph observation."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import platform
import re
import shutil
import stat
import struct
import sys
import tarfile
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Any, BinaryIO, Mapping, Sequence

try:
    from tools.harness_support import run_bounded
except ModuleNotFoundError:  # Direct execution via `python tools/<script>.py`.
    from harness_support import run_bounded


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = REPO_ROOT / "docs" / "core-representative-corpus.json"
DEFAULT_POLICY = REPO_ROOT / "docs" / "core-representative-beta-policy.json"
DEFAULT_CACHE = REPO_ROOT / ".benchmark-cache" / "core-representative-v1"
MEASURE_PROCESS = REPO_ROOT / "tools" / "measure_process.py"
SOURCE_MANIFEST_DOMAIN = b"BITCODE-SOURCE-MANIFEST-V1\0"
DEFAULT_EXCLUDED_COMPONENTS = {
    ".git",
    ".aether-cache",
    "target",
    "node_modules",
    "__pycache__",
    ".venv",
    "venv",
}
MAX_ARCHIVE_BYTES = 16 * 1024 * 1024
MAX_ARCHIVE_MEMBERS = 50_000
MAX_MEMBER_BYTES = 128 * 1024 * 1024
MAX_UNCOMPRESSED_BYTES = 1024 * 1024 * 1024
MAX_PATH_BYTES = 4096
MAX_PATH_DEPTH = 64
MAX_SOURCE_FILES = 100_000
MAX_SOURCE_BYTES = 1024 * 1024 * 1024
MAX_GRAPH_OUTPUT_BYTES = 128 * 1024 * 1024
HEX_256 = re.compile(r"^[0-9a-f]{64}$")
HEX_160 = re.compile(r"^[0-9a-f]{40}$")


@dataclass(frozen=True)
class SourceInventory:
    files: int
    bytes: int
    physical_lines: int
    manifest_sha256: str
    records: tuple[tuple[str, int, str], ...]


class RejectRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, file_pointer, code, message, headers, new_url):
        raise urllib.error.HTTPError(
            request.full_url,
            code,
            f"redirect rejected: {new_url}",
            headers,
            file_pointer,
        )


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def validate_manifest(data: Mapping[str, Any]) -> list[Mapping[str, Any]]:
    if type(data.get("schema_version")) is not int or data["schema_version"] != 1:
        raise RuntimeError("corpus manifest schema_version must equal 1")
    if not isinstance(data.get("suite_id"), str) or not data["suite_id"]:
        raise RuntimeError("corpus manifest must declare a suite_id")
    repositories = data.get("repositories")
    if not isinstance(repositories, list) or len(repositories) != 6:
        raise RuntimeError("corpus manifest must contain exactly six repositories")
    languages: dict[str, int] = {}
    identifiers: set[str] = set()
    for repository in repositories:
        if not isinstance(repository, dict):
            raise RuntimeError("every corpus repository must be an object")
        identifier = repository.get("id")
        language = repository.get("language")
        if not isinstance(identifier, str) or not identifier or identifier in identifiers:
            raise RuntimeError(f"invalid or duplicate corpus id: {identifier!r}")
        if language not in {"rust", "python"}:
            raise RuntimeError(f"unsupported primary language for {identifier}: {language!r}")
        identifiers.add(identifier)
        languages[language] = languages.get(language, 0) + 1
        artifact = repository.get("artifact")
        sources = repository.get("sources")
        cases = repository.get("semantic_cases")
        license_metadata = repository.get("license")
        if not all(
            isinstance(value, dict)
            for value in (artifact, sources, license_metadata)
        ):
            raise RuntimeError(f"{identifier} is missing artifact, source, or license metadata")
        parsed = urllib.parse.urlparse(str(artifact.get("url", "")))
        if (
            parsed.scheme != "https"
            or not parsed.hostname
            or parsed.username
            or parsed.password
            or parsed.query
            or parsed.fragment
        ):
            raise RuntimeError(f"{identifier} artifact URL must be credential-free HTTPS")
        if artifact.get("format") != "tar.gz":
            raise RuntimeError(f"{identifier} artifact format must be tar.gz")
        if not HEX_256.fullmatch(str(artifact.get("sha256", ""))):
            raise RuntimeError(f"{identifier} artifact SHA-256 is invalid")
        archive_bytes = artifact.get("bytes")
        if type(archive_bytes) is not int or not (1 <= archive_bytes <= MAX_ARCHIVE_BYTES):
            raise RuntimeError(f"{identifier} artifact byte count is invalid")
        root = artifact.get("root")
        if not isinstance(root, str) or not safe_archive_name(root) or "/" in root:
            raise RuntimeError(f"{identifier} archive root is invalid")
        if "upstream_commit" in repository and not HEX_160.fullmatch(
            str(repository["upstream_commit"])
        ):
            raise RuntimeError(f"{identifier} upstream commit is invalid")
        extensions = sources.get("extensions")
        expected_extension = ".rs" if language == "rust" else ".py"
        if extensions != [expected_extension]:
            raise RuntimeError(
                f"{identifier} source extensions must equal [{expected_extension!r}]"
            )
        for field, maximum in (
            ("files", MAX_SOURCE_FILES),
            ("bytes", MAX_SOURCE_BYTES),
            ("physical_lines", MAX_SOURCE_BYTES),
        ):
            value = sources.get(field)
            if type(value) is not int or not (1 <= value <= maximum):
                raise RuntimeError(f"{identifier} source {field} is invalid")
        if not HEX_256.fullmatch(str(sources.get("manifest_sha256", ""))):
            raise RuntimeError(f"{identifier} source manifest SHA-256 is invalid")
        license_files = license_metadata.get("files")
        if (
            not isinstance(license_metadata.get("spdx"), str)
            or not license_metadata["spdx"]
            or not isinstance(license_files, list)
            or not license_files
            or any(
                not isinstance(path, str) or not safe_archive_name(path)
                for path in license_files
            )
            or len(set(license_files)) != len(license_files)
        ):
            raise RuntimeError(f"{identifier} license metadata is invalid")
        if not isinstance(cases, list) or not cases:
            raise RuntimeError(f"{identifier} must declare semantic cases before execution")
        case_ids: set[str] = set()
        for case in cases:
            case_id = case.get("id") if isinstance(case, dict) else None
            if not isinstance(case_id, str) or not case_id or case_id in case_ids:
                raise RuntimeError(f"{identifier} has an invalid or duplicate semantic case")
            if not isinstance(case.get("expected_present"), bool):
                raise RuntimeError(f"{identifier}/{case_id} must declare expected_present")
            if case.get("kind") not in {
                "Calls",
                "Inherits",
                "DataFlow",
                "Contains",
                "SemanticSimilar",
                "Impacts",
                "Contributes",
            }:
                raise RuntimeError(f"{identifier}/{case_id} has an invalid edge kind")
            for field in ("source", "target", "file", "rationale"):
                if not isinstance(case.get(field), str) or not case[field]:
                    raise RuntimeError(f"{identifier}/{case_id} is missing {field}")
            if not safe_archive_name(case["file"]):
                raise RuntimeError(f"{identifier}/{case_id} has an invalid file path")
            case_ids.add(case_id)
    if languages != {"rust": 3, "python": 3}:
        raise RuntimeError(f"corpus must contain three Rust and three Python repos: {languages}")
    return repositories


def validate_policy(
    data: Mapping[str, Any], repository_ids: set[str]
) -> Mapping[str, Any]:
    if type(data.get("schema_version")) is not int or data["schema_version"] != 1:
        raise RuntimeError("beta policy schema_version must equal 1")
    if not isinstance(data.get("policy_id"), str) or not data["policy_id"]:
        raise RuntimeError("beta policy must declare a policy_id")
    eligibility = data.get("eligibility")
    semantic = data.get("semantic")
    determinism = data.get("determinism")
    performance = data.get("performance")
    if not all(
        isinstance(value, dict)
        for value in (eligibility, semantic, determinism, performance)
    ):
        raise RuntimeError("beta policy sections must be objects")
    for field in ("runs_per_repository", "rayon_threads"):
        value = eligibility.get(field)
        if type(value) is not int or value <= 0:
            raise RuntimeError(f"beta policy eligibility {field} is invalid")
    for field in ("false_negatives_max", "false_positives_max"):
        value = semantic.get(field)
        if type(value) is not int or value < 0:
            raise RuntimeError(f"beta policy semantic {field} is invalid")
    for field in (
        "macro_precision_min",
        "macro_recall_min",
        "micro_precision_min",
        "micro_recall_min",
    ):
        value = semantic.get(field)
        if not isinstance(value, (int, float)) or isinstance(value, bool) or not 0 <= value <= 1:
            raise RuntimeError(f"beta policy semantic {field} is invalid")
    for field in (
        "artifact_unique_digests_max",
        "semantic_unique_digests_max",
        "unique_node_edge_count_pairs_max",
    ):
        value = determinism.get(field)
        if type(value) is not int or value < 1:
            raise RuntimeError(f"beta policy determinism {field} is invalid")
    default = performance.get("default")
    overrides = performance.get("repository_overrides")
    if not isinstance(default, dict) or not isinstance(overrides, dict):
        raise RuntimeError("beta policy performance thresholds are invalid")
    if not set(overrides).issubset(repository_ids):
        raise RuntimeError("beta policy overrides name an unknown repository")
    for thresholds in [default, *overrides.values()]:
        if not isinstance(thresholds, dict):
            raise RuntimeError("beta policy repository thresholds must be objects")
        for field in ("analyze_max_ms", "analyze_median_max_ms", "rss_max_kib"):
            value = thresholds.get(field)
            if type(value) is not int or value <= 0:
                raise RuntimeError(f"beta policy performance {field} is invalid")
    for field in (
        "graph_artifact_max_bytes",
        "inspect_max_ms",
        "sum_of_repository_medians_max_ms",
    ):
        value = performance.get(field)
        if type(value) is not int or value <= 0:
            raise RuntimeError(f"beta policy performance {field} is invalid")
    return data


def safe_archive_name(name: str) -> bool:
    if not name or "\\" in name or name.startswith("/") or "\x00" in name:
        return False
    normalized = name.rstrip("/")
    parts = normalized.split("/")
    return bool(parts) and all(
        part not in {"", ".", ".."} and ":" not in part
        for part in parts
    ) and len(name.encode("utf-8")) <= MAX_PATH_BYTES and len(parts) <= MAX_PATH_DEPTH


def ensure_cache_directory(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    metadata = path.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        raise RuntimeError(f"cache path is not a real directory: {path}")


def verified_regular_file(path: Path, expected_bytes: int, expected_sha256: str) -> bool:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return False
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise RuntimeError(f"cached artifact is not a regular file: {path}")
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags)
    try:
        opened = os.fstat(descriptor)
        if not stat.S_ISREG(opened.st_mode) or opened.st_size != expected_bytes:
            return False
        digest = hashlib.sha256()
        while chunk := os.read(descriptor, 1024 * 1024):
            digest.update(chunk)
        return digest.hexdigest() == expected_sha256
    finally:
        os.close(descriptor)


def acquire_artifact(
    artifact: Mapping[str, Any],
    cache_directory: Path,
    *,
    offline: bool,
    timeout_seconds: float,
) -> Path:
    ensure_cache_directory(cache_directory)
    expected_sha256 = artifact["sha256"]
    expected_bytes = artifact["bytes"]
    destination = cache_directory / f"{expected_sha256}.tar.gz"
    lock_path = cache_directory / f"{expected_sha256}.lock"
    with lock_path.open("a+b") as lock:
        os.chmod(lock_path, 0o600)
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        if verified_regular_file(destination, expected_bytes, expected_sha256):
            return destination
        if destination.exists():
            raise RuntimeError(f"cached artifact failed integrity verification: {destination}")
        if offline:
            raise RuntimeError(f"verified artifact unavailable in offline mode: {destination}")
        download_artifact(
            artifact["url"],
            destination,
            expected_bytes=expected_bytes,
            expected_sha256=expected_sha256,
            timeout_seconds=timeout_seconds,
        )
        if not verified_regular_file(destination, expected_bytes, expected_sha256):
            raise RuntimeError(f"downloaded artifact failed post-write verification: {destination}")
    return destination


def download_artifact(
    url: str,
    destination: Path,
    *,
    expected_bytes: int,
    expected_sha256: str,
    timeout_seconds: float,
) -> None:
    request = urllib.request.Request(
        url,
        headers={"Accept-Encoding": "identity", "User-Agent": "bitcode-core-benchmark/1"},
    )
    opener = urllib.request.build_opener(RejectRedirects())
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{destination.name}.", suffix=".tmp", dir=destination.parent
    )
    temporary = Path(temporary_name)
    digest = hashlib.sha256()
    received = 0
    try:
        with os.fdopen(descriptor, "wb") as output, opener.open(
            request, timeout=timeout_seconds
        ) as response:
            if response.status != 200 or response.geturl() != url:
                raise RuntimeError(f"unexpected artifact response for {url}")
            encoding = response.headers.get("Content-Encoding")
            if encoding not in {None, "", "identity"}:
                raise RuntimeError(f"unsupported artifact content encoding: {encoding}")
            length = response.headers.get("Content-Length")
            if length is None or int(length) != expected_bytes:
                raise RuntimeError(
                    f"artifact Content-Length mismatch: expected {expected_bytes}, got {length}"
                )
            while chunk := response.read(64 * 1024):
                received += len(chunk)
                if received > expected_bytes:
                    raise RuntimeError("artifact exceeded its checked byte count")
                digest.update(chunk)
                output.write(chunk)
            output.flush()
            os.fsync(output.fileno())
        if received != expected_bytes or digest.hexdigest() != expected_sha256:
            raise RuntimeError("artifact bytes or SHA-256 did not match the checked manifest")
        os.chmod(temporary, 0o600)
        os.replace(temporary, destination)
        fsync_directory(destination.parent)
    finally:
        temporary.unlink(missing_ok=True)


def extract_archive(archive_path: Path, destination: Path, expected_root: str) -> Path:
    if destination.exists():
        raise RuntimeError(f"refusing to merge extraction into existing path: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(tempfile.mkdtemp(prefix=f".{destination.name}.", dir=destination.parent))
    os.chmod(temporary, 0o700)
    try:
        with tarfile.open(archive_path, mode="r:gz") as archive:
            members = preflight_archive(archive, expected_root)
            for member, parts in members:
                output = temporary.joinpath(*parts)
                if member.isdir():
                    output.mkdir(parents=True, exist_ok=True, mode=0o700)
                    os.chmod(output, 0o700)
                    continue
                output.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                source = archive.extractfile(member)
                if source is None:
                    raise RuntimeError(f"archive member had no readable payload: {member.name}")
                copy_exact_member(source, output, member.size)
        extracted_root = temporary / expected_root
        if not extracted_root.is_dir():
            raise RuntimeError(f"archive omitted exact root directory {expected_root}")
        os.replace(temporary, destination)
        fsync_directory(destination.parent)
        return destination / expected_root
    except Exception:
        shutil.rmtree(temporary, ignore_errors=True)
        raise


def preflight_archive(
    archive: tarfile.TarFile, expected_root: str
) -> list[tuple[tarfile.TarInfo, tuple[str, ...]]]:
    checked: list[tuple[tarfile.TarInfo, tuple[str, ...]]] = []
    seen: dict[str, str] = {}
    casefolded: set[str] = set()
    total_bytes = 0
    for member in archive:
        if len(checked) >= MAX_ARCHIVE_MEMBERS:
            raise RuntimeError("archive member-count limit exceeded")
        if not safe_archive_name(member.name):
            raise RuntimeError(f"unsafe archive member path: {member.name!r}")
        normalized = member.name.rstrip("/")
        parts = tuple(PurePosixPath(normalized).parts)
        if parts[0] != expected_root:
            raise RuntimeError(
                f"archive member is outside exact root {expected_root!r}: {member.name!r}"
            )
        if not (member.isdir() or member.isreg()):
            raise RuntimeError(f"unsupported archive member type: {member.name!r}")
        kind = "directory" if member.isdir() else "file"
        if normalized in seen:
            raise RuntimeError(f"duplicate archive member path: {normalized!r}")
        folded = normalized.casefold()
        if folded in casefolded:
            raise RuntimeError(f"case-colliding archive member path: {normalized!r}")
        for index in range(1, len(parts)):
            ancestor = "/".join(parts[:index])
            if seen.get(ancestor) == "file":
                raise RuntimeError(f"archive file is a parent of {normalized!r}")
        if kind == "file" and any(path.startswith(f"{normalized}/") for path in seen):
            raise RuntimeError(f"archive file conflicts with child paths: {normalized!r}")
        if member.size < 0 or member.size > MAX_MEMBER_BYTES:
            raise RuntimeError(f"archive member size limit exceeded: {member.name!r}")
        total_bytes += member.size
        if total_bytes > MAX_UNCOMPRESSED_BYTES:
            raise RuntimeError("archive uncompressed-byte limit exceeded")
        seen[normalized] = kind
        casefolded.add(folded)
        checked.append((member, parts))
    if not checked:
        raise RuntimeError("archive is empty")
    return checked


def copy_exact_member(source: BinaryIO, destination: Path, expected_bytes: int) -> None:
    written = 0
    with destination.open("xb") as output:
        os.chmod(destination, 0o600)
        while written < expected_bytes:
            chunk = source.read(min(64 * 1024, expected_bytes - written))
            if not chunk:
                raise RuntimeError(f"truncated archive member: {destination}")
            output.write(chunk)
            written += len(chunk)
        if source.read(1):
            raise RuntimeError(f"archive member exceeded declared size: {destination}")


def source_inventory(root: Path, extensions: set[str]) -> SourceInventory:
    records: list[tuple[str, bytes]] = []
    stack = [root]
    while stack:
        directory = stack.pop()
        entries = sorted(os.scandir(directory), key=lambda entry: entry.name)
        for entry in entries:
            metadata = entry.stat(follow_symlinks=False)
            if stat.S_ISLNK(metadata.st_mode):
                raise RuntimeError(f"source tree contains a symlink: {entry.path}")
            if stat.S_ISDIR(metadata.st_mode):
                relative = Path(entry.path).relative_to(root)
                if not is_excluded(relative):
                    stack.append(Path(entry.path))
            elif stat.S_ISREG(metadata.st_mode):
                relative = Path(entry.path).relative_to(root)
                if is_excluded(relative) or relative.suffix not in extensions:
                    continue
                if metadata.st_size > MAX_MEMBER_BYTES:
                    raise RuntimeError(f"source file exceeds byte limit: {relative}")
                content = Path(entry.path).read_bytes()
                try:
                    content.decode("utf-8")
                except UnicodeDecodeError as error:
                    raise RuntimeError(f"source file is not UTF-8: {relative}: {error}") from error
                records.append((relative.as_posix(), content))
            else:
                raise RuntimeError(f"source tree contains a special file: {entry.path}")
    records.sort(key=lambda record: record[0])
    if len(records) > MAX_SOURCE_FILES:
        raise RuntimeError("source file-count limit exceeded")
    total_bytes = sum(len(content) for _, content in records)
    if total_bytes > MAX_SOURCE_BYTES:
        raise RuntimeError("source aggregate-byte limit exceeded")
    digest = hashlib.sha256(SOURCE_MANIFEST_DOMAIN)
    public_records = []
    physical_lines = 0
    for relative, content in records:
        path_bytes = relative.encode("utf-8")
        content_sha = hashlib.sha256(content).digest()
        digest.update(struct.pack(">Q", len(path_bytes)))
        digest.update(path_bytes)
        digest.update(struct.pack(">Q", len(content)))
        digest.update(content_sha)
        public_records.append((relative, len(content), content_sha.hex()))
        physical_lines += content.count(b"\n")
        if content and not content.endswith(b"\n"):
            physical_lines += 1
    return SourceInventory(
        files=len(records),
        bytes=total_bytes,
        physical_lines=physical_lines,
        manifest_sha256=digest.hexdigest(),
        records=tuple(public_records),
    )


def is_excluded(relative: Path) -> bool:
    return any(
        component in DEFAULT_EXCLUDED_COMPONENTS or component.startswith(".")
        for component in relative.parts
    )


def verify_inventory(repository: Mapping[str, Any], observed: SourceInventory) -> None:
    expected = repository["sources"]
    actual = {
        "files": observed.files,
        "bytes": observed.bytes,
        "physical_lines": observed.physical_lines,
        "manifest_sha256": observed.manifest_sha256,
    }
    required = {key: expected[key] for key in actual}
    if actual != required:
        raise RuntimeError(
            f"{repository['id']} source inventory mismatch:\n"
            f"expected {json.dumps(required, sort_keys=True)}\n"
            f"actual   {json.dumps(actual, sort_keys=True)}"
        )


def parse_peak_rss(path: Path) -> int:
    usage = json.loads(path.read_text(encoding="utf-8"))
    peak = usage.get("peak_rss_kib")
    if usage.get("schema_version") != 1 or not isinstance(peak, int) or peak < 0:
        raise RuntimeError(f"wait4 output omitted an exact peak RSS value: {path}")
    return peak


def run_index(
    bitcode: Path,
    project: Path,
    repository: Mapping[str, Any],
    inventory: SourceInventory,
    ordinal: int,
    *,
    timeout_seconds: float,
    rayon_threads: int,
) -> Mapping[str, Any]:
    graph_path = project / "project.aether"
    graph_path.unlink(missing_ok=True)
    time_output = project.parent / f"time-{repository['id']}-{ordinal}.txt"
    time_output.unlink(missing_ok=True)
    analyze_command = (
        sys.executable,
        str(MEASURE_PROCESS),
        str(time_output),
        "--",
        str(bitcode),
        "analyze",
        str(project),
        "--json",
    )
    analyzed = run_bounded(
        analyze_command,
        cwd=REPO_ROOT,
        env={**os.environ, "RAYON_NUM_THREADS": str(rayon_threads)},
        timeout_seconds=timeout_seconds,
        max_output_bytes=4 * 1024 * 1024,
    )
    summary = json.loads(analyzed.stdout)
    if summary.get("schema_version") != 1:
        raise RuntimeError(f"{repository['id']} analyze emitted an unknown JSON schema")
    if summary.get("source_files") != inventory.files:
        raise RuntimeError(
            f"{repository['id']} Bit Code loaded {summary.get('source_files')} sources; "
            f"inventory requires {inventory.files}"
        )
    phase_ms = {}
    for field in ("build_ms", "similarity_ms", "save_ms"):
        value = summary.get(field)
        if not isinstance(value, int) or value < 0:
            raise RuntimeError(f"{repository['id']} analyze omitted numeric {field}")
        phase_ms[field.removesuffix("_ms")] = value
    if not graph_path.is_file():
        raise RuntimeError(f"{repository['id']} analyze did not create a graph artifact")

    inspected = run_bounded(
        (str(bitcode), "inspect", str(graph_path), "--json"),
        cwd=REPO_ROOT,
        env={**os.environ, "RAYON_NUM_THREADS": str(rayon_threads)},
        timeout_seconds=timeout_seconds,
        max_output_bytes=MAX_GRAPH_OUTPUT_BYTES,
    )
    exported = json.loads(inspected.stdout)
    if exported.get("schema_version") != 1:
        raise RuntimeError(f"{repository['id']} inspect emitted an unknown JSON schema")
    nodes = exported.get("nodes")
    edges = exported.get("edges")
    if not isinstance(nodes, list) or not isinstance(edges, list):
        raise RuntimeError(f"{repository['id']} graph export omitted nodes or edges")
    if summary.get("nodes") != len(nodes) or summary.get("edges") != len(edges):
        raise RuntimeError(f"{repository['id']} analyze/export graph counts disagree")

    semantic = evaluate_semantic_cases(repository, nodes, edges)
    canonical = json.dumps(
        exported, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    after = source_inventory(project, set(repository["sources"]["extensions"]))
    if after != inventory:
        raise RuntimeError(f"{repository['id']} source tree changed during indexing")
    edge_counts: dict[str, int] = {}
    for edge in edges:
        kind = edge.get("kind")
        if not isinstance(kind, str):
            raise RuntimeError(f"{repository['id']} graph export contains an invalid edge")
        edge_counts[kind] = edge_counts.get(kind, 0) + 1
    return {
        "ordinal": ordinal,
        "wall_ms": round(analyzed.wall_seconds * 1000),
        "phase_ms": phase_ms,
        "peak_rss_kib": parse_peak_rss(time_output),
        "analyze_stdout_sha256": analyzed.stdout_sha256,
        "analyze_stderr_sha256": analyzed.stderr_sha256,
        "inspect_wall_ms": round(inspected.wall_seconds * 1000),
        "source_manifest_after_sha256": after.manifest_sha256,
        "graph": {
            "nodes": len(nodes),
            "edges": len(edges),
            "edges_by_kind": dict(sorted(edge_counts.items())),
            "artifact_bytes": graph_path.stat().st_size,
            "artifact_sha256": sha256_file(graph_path),
            "canonical_semantic_sha256": hashlib.sha256(canonical).hexdigest(),
        },
        "semantic_cases": semantic,
    }


def evaluate_semantic_cases(
    repository: Mapping[str, Any],
    nodes: Sequence[Mapping[str, Any]],
    edges: Sequence[Mapping[str, Any]],
) -> list[Mapping[str, Any]]:
    node_paths = {node.get("path") for node in nodes}
    edge_set = {
        (edge.get("source"), edge.get("target"), edge.get("kind")) for edge in edges
    }
    results = []
    for case in repository["semantic_cases"]:
        source = case["source"]
        target = case["target"]
        if source not in node_paths or target not in node_paths:
            raise RuntimeError(
                f"{repository['id']}/{case['id']} endpoint missing: "
                f"source={source in node_paths}, target={target in node_paths}"
            )
        observed = (source, target, case["kind"]) in edge_set
        expected = case["expected_present"]
        classification = (
            "tp" if expected and observed else
            "fn" if expected else
            "fp" if observed else
            "tn"
        )
        results.append(
            {
                "id": case["id"],
                "kind": case["kind"],
                "source": source,
                "target": target,
                "expected_present": expected,
                "observed_present": observed,
                "classification": classification,
            }
        )
    return results


def summarize_repository_runs(runs: Sequence[Mapping[str, Any]]) -> Mapping[str, Any]:
    walls = sorted(run["wall_ms"] for run in runs)
    rss = sorted(run["peak_rss_kib"] for run in runs)
    artifacts = {run["graph"]["artifact_sha256"] for run in runs}
    semantics = {run["graph"]["canonical_semantic_sha256"] for run in runs}
    counts = {(run["graph"]["nodes"], run["graph"]["edges"]) for run in runs}
    cases = runs[0]["semantic_cases"]
    for run in runs[1:]:
        if run["semantic_cases"] != cases:
            raise RuntimeError("semantic case outcomes changed across identical runs")
    totals = {label: 0 for label in ("tp", "fp", "fn", "tn")}
    for case in cases:
        totals[case["classification"]] += 1
    selected = totals["tp"] + totals["fp"]
    actual = totals["tp"] + totals["fn"]
    return {
        "wall_median_ms": median(walls),
        "wall_max_ms": max(walls),
        "phase_median_ms": {
            phase: median(sorted(run["phase_ms"][phase] for run in runs))
            for phase in ("build", "similarity", "save")
        },
        "peak_rss_median_kib": median(rss),
        "peak_rss_max_kib": max(rss),
        "artifact_unique_digests": len(artifacts),
        "semantic_unique_digests": len(semantics),
        "unique_node_edge_count_pairs": len(counts),
        "semantic": {
            **totals,
            "precision": round(totals["tp"] / selected, 6) if selected else 1.0,
            "recall": round(totals["tp"] / actual, 6) if actual else 1.0,
        },
    }


def summarize_semantics(
    repositories: Sequence[Mapping[str, Any]],
) -> Mapping[str, Any]:
    def summarize(selected: Sequence[Mapping[str, Any]]) -> Mapping[str, Any]:
        totals = {label: 0 for label in ("tp", "fp", "fn", "tn")}
        precisions = []
        recalls = []
        for repository in selected:
            semantic = repository["summary"]["semantic"]
            for label in totals:
                totals[label] += semantic[label]
            precisions.append(semantic["precision"])
            recalls.append(semantic["recall"])
        predicted = totals["tp"] + totals["fp"]
        actual = totals["tp"] + totals["fn"]
        return {
            **totals,
            "micro_precision": round(totals["tp"] / predicted, 6) if predicted else 1.0,
            "micro_recall": round(totals["tp"] / actual, 6) if actual else 1.0,
            "macro_precision": round(sum(precisions) / len(precisions), 6),
            "macro_recall": round(sum(recalls) / len(recalls), 6),
        }

    return {
        "aggregate": summarize(repositories),
        "languages": {
            language: summarize(
                [
                    repository
                    for repository in repositories
                    if repository["language"] == language
                ]
            )
            for language in ("rust", "python")
        },
    }


def median(values: Sequence[int]) -> float | int:
    middle = len(values) // 2
    if len(values) % 2:
        return values[middle]
    return (values[middle - 1] + values[middle]) / 2


def evaluate_policy(
    repositories: Sequence[Mapping[str, Any]],
    semantic_summary: Mapping[str, Any],
    policy: Mapping[str, Any],
    *,
    runs: int,
    rayon_threads: int,
    source_worktree_clean: bool,
) -> Mapping[str, Any]:
    checks = []

    def maximum(check_id: str, actual: int | float, limit: int | float) -> None:
        checks.append(
            {"id": check_id, "passed": actual <= limit, "actual": actual, "maximum": limit}
        )

    def minimum(check_id: str, actual: int | float, limit: int | float) -> None:
        checks.append(
            {"id": check_id, "passed": actual >= limit, "actual": actual, "minimum": limit}
        )

    eligibility = policy["eligibility"]
    checks.append(
        {
            "id": "eligibility.runs_per_repository",
            "passed": runs == eligibility["runs_per_repository"],
            "actual": runs,
            "required": eligibility["runs_per_repository"],
        }
    )
    checks.append(
        {
            "id": "eligibility.rayon_threads",
            "passed": rayon_threads == eligibility["rayon_threads"],
            "actual": rayon_threads,
            "required": eligibility["rayon_threads"],
        }
    )
    checks.append(
        {
            "id": "eligibility.source_worktree_clean",
            "passed": source_worktree_clean,
            "actual": source_worktree_clean,
            "required": True,
        }
    )

    semantic_policy = policy["semantic"]
    for scope, semantic in [
        ("aggregate", semantic_summary["aggregate"]),
        *semantic_summary["languages"].items(),
    ]:
        maximum(
            f"semantic.{scope}.false_positives",
            semantic["fp"],
            semantic_policy["false_positives_max"],
        )
        maximum(
            f"semantic.{scope}.false_negatives",
            semantic["fn"],
            semantic_policy["false_negatives_max"],
        )
        for metric in (
            "micro_precision",
            "micro_recall",
            "macro_precision",
            "macro_recall",
        ):
            minimum(
                f"semantic.{scope}.{metric}",
                semantic[metric],
                semantic_policy[f"{metric}_min"],
            )

    determinism = policy["determinism"]
    performance = policy["performance"]
    sum_of_medians = 0
    for repository in repositories:
        identifier = repository["id"]
        summary = repository["summary"]
        checks.append(
            {
                "id": f"eligibility.{identifier}.observed_runs",
                "passed": len(repository["runs"])
                == eligibility["runs_per_repository"],
                "actual": len(repository["runs"]),
                "required": eligibility["runs_per_repository"],
            }
        )
        for field in (
            "artifact_unique_digests",
            "semantic_unique_digests",
            "unique_node_edge_count_pairs",
        ):
            maximum(
                f"determinism.{identifier}.{field}",
                summary[field],
                determinism[f"{field}_max"],
            )
        thresholds = performance["repository_overrides"].get(
            identifier, performance["default"]
        )
        maximum(
            f"performance.{identifier}.analyze_median_ms",
            summary["wall_median_ms"],
            thresholds["analyze_median_max_ms"],
        )
        maximum(
            f"performance.{identifier}.analyze_max_ms",
            summary["wall_max_ms"],
            thresholds["analyze_max_ms"],
        )
        maximum(
            f"performance.{identifier}.rss_max_kib",
            summary["peak_rss_max_kib"],
            thresholds["rss_max_kib"],
        )
        maximum(
            f"performance.{identifier}.inspect_max_ms",
            max(run["inspect_wall_ms"] for run in repository["runs"]),
            performance["inspect_max_ms"],
        )
        maximum(
            f"performance.{identifier}.graph_artifact_max_bytes",
            max(run["graph"]["artifact_bytes"] for run in repository["runs"]),
            performance["graph_artifact_max_bytes"],
        )
        sum_of_medians += summary["wall_median_ms"]
    maximum(
        "performance.sum_of_repository_medians_ms",
        sum_of_medians,
        performance["sum_of_repository_medians_max_ms"],
    )
    return {
        "passed": all(check["passed"] for check in checks),
        "checks": checks,
        "failed_check_ids": [check["id"] for check in checks if not check["passed"]],
    }


def cpu_model() -> str | None:
    try:
        for line in Path("/proc/cpuinfo").read_text(encoding="utf-8").splitlines():
            if line.startswith("model name"):
                return line.partition(":")[2].strip() or None
    except OSError:
        pass
    return None


def filesystem_metadata(path: Path) -> Mapping[str, Any] | None:
    try:
        resolved = path.resolve()
        best = None
        for line in Path("/proc/self/mountinfo").read_text(encoding="utf-8").splitlines():
            left, separator, right = line.partition(" - ")
            if not separator:
                continue
            fields = left.split()
            target = fields[4].replace("\\040", " ").replace("\\011", "\t")
            try:
                resolved.relative_to(target)
            except ValueError:
                continue
            candidate = (len(Path(target).parts), target, right.split())
            if best is None or candidate[0] > best[0]:
                best = candidate
        if best is None or len(best[2]) < 2:
            return None
        return {
            "mount_point": best[1],
            "filesystem_type": best[2][0],
            "source": best[2][1],
        }
    except OSError:
        return None


def host_metadata(execution_root: Path, rayon_threads: int) -> Mapping[str, Any]:
    memory_bytes = None
    try:
        memory_bytes = os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES")
    except (ValueError, OSError, AttributeError):
        pass
    return {
        "os": platform.system(),
        "kernel": platform.release(),
        "arch": platform.machine(),
        "python": platform.python_version(),
        "logical_cpus": os.cpu_count(),
        "cpu_model": cpu_model(),
        "memory_bytes": memory_bytes,
        "execution_filesystem": filesystem_metadata(execution_root),
        "rayon_threads": rayon_threads,
        "load_policy": "uncontrolled",
        "rss_measurement": "posix-wait4-ru_maxrss-kib",
    }


def atomic_write_json(path: Path, value: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "wb") as output:
            output.write(payload)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        fsync_directory(path.parent)
    finally:
        temporary.unlink(missing_ok=True)


def fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def benchmark(args: argparse.Namespace) -> Mapping[str, Any]:
    manifest_bytes = args.manifest.read_bytes()
    manifest = json.loads(manifest_bytes)
    repositories = validate_manifest(manifest)
    policy_bytes = args.policy.read_bytes()
    policy = validate_policy(
        json.loads(policy_bytes), {repository["id"] for repository in repositories}
    )
    rayon_threads = policy["eligibility"]["rayon_threads"]
    if not args.bitcode.is_file():
        raise RuntimeError(f"Bit Code binary does not exist: {args.bitcode}")
    if os.name != "posix" or not hasattr(os, "wait4") or not MEASURE_PROCESS.is_file():
        raise RuntimeError("representative indexing requires the checked POSIX wait4 wrapper")

    cache = args.cache_dir.resolve()
    ensure_cache_directory(cache)
    with tempfile.TemporaryDirectory(prefix="bitcode-core-representative-") as temporary_name:
        temporary = Path(temporary_name)
        private_bitcode = temporary / "bitcode"
        shutil.copyfile(args.bitcode, private_bitcode)
        os.chmod(private_bitcode, 0o500)
        bitcode_sha256 = sha256_file(private_bitcode)
        git = run_bounded(
            ("git", "rev-parse", "HEAD"),
            cwd=REPO_ROOT,
            timeout_seconds=30,
            max_output_bytes=64 * 1024,
        )
        source_commit = git.stdout.strip()
        git_status = run_bounded(
            ("git", "status", "--porcelain=v1", "--untracked-files=all"),
            cwd=REPO_ROOT,
            timeout_seconds=30,
            max_output_bytes=1024 * 1024,
        )
        source_worktree_clean = not git_status.stdout
        measured_host = host_metadata(temporary, rayon_threads)

        observed_repositories = []
        for repository in repositories:
            artifact = acquire_artifact(
                repository["artifact"],
                cache,
                offline=args.offline,
                timeout_seconds=args.download_timeout_seconds,
            )
            extraction = temporary / repository["id"]
            project = extract_archive(artifact, extraction, repository["artifact"]["root"])
            for license_file in repository["license"]["files"]:
                if not (project / license_file).is_file():
                    raise RuntimeError(
                        f"{repository['id']} omitted checked license file {license_file}"
                    )
            inventory = source_inventory(project, set(repository["sources"]["extensions"]))
            verify_inventory(repository, inventory)
            runs = [
                run_index(
                    private_bitcode,
                    project,
                    repository,
                    inventory,
                    ordinal,
                    timeout_seconds=args.index_timeout_seconds,
                    rayon_threads=rayon_threads,
                )
                for ordinal in range(1, args.runs + 1)
            ]
            observed_repositories.append(
                {
                    "id": repository["id"],
                    "language": repository["language"],
                    "artifact_sha256": repository["artifact"]["sha256"],
                    "source": {
                        "files": inventory.files,
                        "bytes": inventory.bytes,
                        "physical_lines": inventory.physical_lines,
                        "manifest_sha256": inventory.manifest_sha256,
                    },
                    "runs": runs,
                    "summary": summarize_repository_runs(runs),
                }
            )
        if sha256_file(private_bitcode) != bitcode_sha256:
            raise RuntimeError("measured Bit Code binary changed during the benchmark")

    semantic_summary = summarize_semantics(observed_repositories)
    policy_evaluation = evaluate_policy(
        observed_repositories,
        semantic_summary,
        policy,
        runs=args.runs,
        rayon_threads=rayon_threads,
        source_worktree_clean=source_worktree_clean,
    )
    result = {
        "schema_version": 1,
        "kind": "representative-corpus-observation",
        "suite_id": manifest["suite_id"],
        "corpus_manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(),
        "tool": {
            "bitcode_sha256": bitcode_sha256,
            "source_commit": source_commit,
            "source_worktree_clean": source_worktree_clean,
            "harness_sha256": sha256_file(Path(__file__)),
            "harness_support_sha256": sha256_file(REPO_ROOT / "tools" / "harness_support.py"),
            "measure_process_sha256": sha256_file(MEASURE_PROCESS),
            "analyze_argv": ["bitcode", "analyze", "<checkout>", "--json"],
        },
        "host": measured_host,
        "run_definition": "fresh-graph-fresh-process-os-cache-uncontrolled",
        "repositories": observed_repositories,
        "semantic_summary": semantic_summary,
        "assessment": {
            "classification": (
                "beta_policy_evaluation"
                if args.evaluate_policy
                else "provisional_policy_comparison"
            ),
            "beta_policy_id": policy["policy_id"],
            "beta_policy_sha256": hashlib.sha256(policy_bytes).hexdigest(),
            "beta_pass": policy_evaluation["passed"],
            "checks": policy_evaluation["checks"],
            "failed_check_ids": policy_evaluation["failed_check_ids"],
        },
    }
    return result


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bitcode", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--cache-dir", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--evaluate-policy", action="store_true")
    parser.add_argument("--download-timeout-seconds", type=float, default=120)
    parser.add_argument("--index-timeout-seconds", type=float, default=300)
    args = parser.parse_args(argv)
    if not 1 <= args.runs <= 20:
        parser.error("--runs must be between 1 and 20")
    if args.download_timeout_seconds <= 0 or args.index_timeout_seconds <= 0:
        parser.error("timeout values must be positive")
    return args


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    result = benchmark(args)
    if args.output:
        atomic_write_json(args.output, result)
        print(f"wrote representative observation: {args.output}")
    else:
        print(json.dumps(result, indent=2, sort_keys=True))
    if args.evaluate_policy and not result["assessment"]["beta_pass"]:
        print(
            "beta policy failed: "
            + ", ".join(result["assessment"]["failed_check_ids"]),
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, ValueError, json.JSONDecodeError, tarfile.TarError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
