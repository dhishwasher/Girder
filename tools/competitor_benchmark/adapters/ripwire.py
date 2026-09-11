"""Ripwire 0.5.0 adapter using its official native CLI responses."""

from __future__ import annotations

import json
import os
import re
import time
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Sequence

from ..fixtures import apply_mutation as write_mutation
from ..process import SupervisedResult, run_supervised
from ..protocol import Adapter, NativeResult, Status, normalize_paths


@dataclass(frozen=True)
class CliCall:
    args: tuple[str, ...]
    process: SupervisedResult
    stdout_path: Path
    stderr_path: Path


class RipwireAdapter(Adapter):
    name = "ripwire"
    version = "0.5.0"
    commit = "bacfa3b7b3ad13648ce3892de06af05b6b55a2ac"

    def __init__(self, binary: Path, *, limits: Mapping[str, Any], private_home: Path,
                 initial_target: str) -> None:
        self.binary = binary.resolve()
        self.limits = limits
        self.private_home = private_home
        self.initial_target = initial_target
        self.fixture_root: Path | None = None
        self.artifact_root: Path | None = None
        self.call_index = 0
        self.env = _isolated_environment(private_home)
        self.cache = private_home / "ripwire-index"

    def prepare(self, fixture_root: Path, artifact_root: Path) -> NativeResult:
        self.fixture_root = fixture_root.resolve()
        self.artifact_root = artifact_root.resolve()
        self.artifact_root.mkdir(parents=True, exist_ok=True)
        call = self._invoke([f"--for={self.initial_target}", "--json", "--no-cache"],
                            timeout=self.limits["prepare_timeout"])
        return _native_from_calls([call], query_calls=0)

    def query_definition(self, target: str) -> NativeResult:
        first = self._invoke([f"--for={target}", "--json", self._cache_arg()])
        calls = [first]
        if first.process.status is not Status.PASS:
            return _native_from_calls([first])
        try:
            data = json.loads(first.stdout_path.read_text(encoding="utf-8"))
            rows = [row for row in data.get("sigs", []) if row.get("n") == target]
            answers = [f"{_path(row['p'])}::{row['n']}" for row in rows]
            if len(rows) != 1:
                return _native_from_calls([first], answer=answers)
            second = self._invoke([
                f"--expand={_path(rows[0]['p'])}:{target}", "--top-k=0", f"--cache={self.cache}"
            ])
            calls.append(second)
            if second.process.status is not Status.PASS:
                return _native_from_calls([first, second], answer=answers)
            root = ET.fromstring(second.stdout_path.read_text(encoding="utf-8"))
            bodies = [node for node in root.findall(".//b")
                      if node.attrib.get("n") == target and _path(node.attrib.get("p", "")) == _path(rows[0]["p"])]
            if len(bodies) != 1:
                raise ValueError(f"expanded bodies={len(bodies)}")
            source = bodies[0].text or ""
            return _native_from_calls(calls, answer=answers, source_text=source)
        except (ET.ParseError, json.JSONDecodeError, KeyError, TypeError, ValueError) as error:
            return _native_from_calls(calls, forced_status=Status.ERROR,
                                      detail=f"definition normalization failed: {error}")

    def query_callers(self, target: str) -> NativeResult:
        return self._json_relation(f"--callers={target}", "callers")

    def query_callees(self, target: str) -> NativeResult:
        return self._json_relation(f"--callees={target}", "callees")

    def query_impact(self, target: str) -> NativeResult:
        return self._json_relation(f"--impact={target}", "impact")

    def query_tests(self, target: str) -> NativeResult:
        call = self._invoke([f"--affected={target}", self._cache_arg()])
        if call.process.status is not Status.PASS:
            return _native_from_calls([call])
        try:
            root = ET.fromstring(call.stdout_path.read_text(encoding="utf-8"))
            answers = [f"{_path(node.attrib['p'])}::*" for node in root.findall(".//test")]
            return _native_from_calls([call], answer=answers)
        except (ET.ParseError, KeyError, TypeError, ValueError) as error:
            return _native_from_calls([call], forced_status=Status.ERROR,
                                      detail=f"test normalization failed: {error}")

    def apply_mutation(self, mutation: Mapping[str, Any]) -> NativeResult:
        assert self.fixture_root is not None
        started = time.monotonic()
        try:
            write_mutation(self.fixture_root, mutation)
            return NativeResult(status=Status.PASS, tool_calls=0,
                                metadata={"elapsed_seconds": time.monotonic() - started})
        except (OSError, ValueError) as error:
            return NativeResult(status=Status.ERROR, detail=f"mutation failed: {error}", tool_calls=0)

    def wait_until_ready(self, timeout_seconds: float) -> NativeResult:
        return NativeResult(status=Status.PASS, tool_calls=0, metadata={"timeout_seconds": timeout_seconds})

    def cleanup(self) -> None:
        # Ripwire is one process per call and run_supervised reaps every process group.
        return None

    def _json_relation(self, flag: str, field: str) -> NativeResult:
        calls: list[CliCall] = []
        answers: list[str] = []
        offset = 0
        while True:
            extra = [flag, "--json", *self._paged_common()]
            if offset:
                extra.append(f"--offset={offset}")
            call = self._invoke(extra)
            calls.append(call)
            if call.process.status is not Status.PASS:
                return _native_from_calls(calls, answer=answers)
            try:
                data = json.loads(call.stdout_path.read_text(encoding="utf-8"))
                rows = data.get(field, [])
                if not isinstance(rows, list):
                    raise ValueError(f"{field} is not a list")
                answers.extend(f"{_path(row['p'])}::{row['n']}" for row in rows)
                total = int(data.get("count" if field != "impact" else "reaches", len(rows)))
                offset += len(rows)
                if not rows or offset >= total:
                    return _native_from_calls(calls, answer=answers)
            except (json.JSONDecodeError, KeyError, TypeError, ValueError) as error:
                return _native_from_calls(calls, answer=answers, forced_status=Status.ERROR,
                                          detail=f"{field} normalization failed: {error}")

    def _cache_arg(self) -> str:
        return f"--cache={self.cache}"

    def _paged_common(self) -> list[str]:
        return [self._cache_arg(), "--limit=500"]

    def _invoke(self, extra: Sequence[str], *, timeout: float | None = None) -> CliCall:
        assert self.fixture_root is not None and self.artifact_root is not None
        self.call_index += 1
        prefix = f"{self.call_index:04d}"
        stdout = self.artifact_root / f"{prefix}.stdout"
        stderr = self.artifact_root / f"{prefix}.stderr"
        args = (str(self.binary), str(self.fixture_root), *extra)
        process = run_supervised(
            args, cwd=self.fixture_root, env=self.env, stdout_path=stdout, stderr_path=stderr,
            timeout_seconds=timeout or self.limits["query_timeout"],
            max_output_bytes=self.limits["max_output_bytes"],
            minimum_available_bytes=self.limits["minimum_available_bytes"],
            emergency_available_bytes=self.limits["emergency_available_bytes"],
            maximum_tree_rss_bytes=self.limits["maximum_tree_rss_bytes"],
        )
        return CliCall(tuple(args), process, stdout, stderr)


def _path(value: str) -> str:
    value = value.replace("\\", "/").removeprefix("./")
    return re.sub(r":\d+(?:-\d+)?$", "", value)


def _native_from_calls(calls: Sequence[CliCall], *, answer: Sequence[str] = (),
                       source_text: str | None = None, forced_status: Status | None = None,
                       detail: str = "", query_calls: int | None = None) -> NativeResult:
    failed = next((call for call in calls if call.process.status is not Status.PASS), None)
    status = forced_status or (failed.process.status if failed else Status.PASS)
    if not detail and failed:
        detail = failed.process.detail
    metadata: dict[str, Any] = {
        "elapsed_seconds": sum(call.process.wall_seconds for call in calls),
        "stdout_bytes": sum(call.process.stdout_bytes for call in calls),
        "stderr_bytes": sum(call.process.stderr_bytes for call in calls),
        "stdout_artifacts": [str(call.stdout_path) for call in calls],
        "stderr_artifacts": [str(call.stderr_path) for call in calls],
        "peak_rss_bytes": max((call.process.peak_rss_bytes or 0 for call in calls), default=0),
        "methods": [{"argv": list(call.args)} for call in calls],
        "returncodes": [call.process.returncode for call in calls],
    }
    if source_text is not None:
        metadata["source_text"] = source_text
    return NativeResult(answer=normalize_paths(answer), status=status, detail=detail,
                        tool_calls=len(calls) if query_calls is None else query_calls,
                        metadata=metadata)


def _isolated_environment(private_home: Path) -> dict[str, str]:
    private_home.mkdir(parents=True, exist_ok=True)
    return dict(
        os.environ, HOME=str(private_home), XDG_CACHE_HOME=str(private_home / ".cache"),
        CARGO_BUILD_JOBS="1", CMAKE_BUILD_PARALLEL_LEVEL="1", MAKEFLAGS="-j1",
        RAYON_NUM_THREADS="1", npm_config_jobs="1",
    )
