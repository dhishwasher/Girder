"""Common adapter contract and auditable execution-record types."""

from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import asdict, dataclass, field
from enum import Enum
from pathlib import Path
from typing import Any, Mapping, Sequence


class Status(str, Enum):
    PASS = "PASS"
    WRONG = "WRONG"
    STALE = "STALE"
    UNSUPPORTED = "UNSUPPORTED"
    TIMEOUT = "TIMEOUT"
    RESOURCE_BLOCKED = "RESOURCE_BLOCKED"
    INSTALL_FAILED = "INSTALL_FAILED"
    ERROR = "ERROR"


class QueryKind(str, Enum):
    DEFINITION = "definition"
    CALLERS = "callers"
    CALLEES = "callees"
    IMPACT = "impact"
    TESTS = "tests"


@dataclass(frozen=True)
class NativeResult:
    """One native response before scoring; raw streams live in artifact files."""

    answer: tuple[str, ...] = ()
    status: Status | None = None
    detail: str = ""
    tool_calls: int = 1
    metadata: Mapping[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class ExecutionRecord:
    schema_version: int
    competitor: str
    version: str
    commit: str
    task_id: str
    fixture_id: str
    command: tuple[str, ...]
    started_at: str
    ended_at: str
    elapsed_seconds: float
    exit_status: int | None
    timed_out: bool
    resource_blocked: bool
    stdout_bytes: int
    stderr_bytes: int
    stdout_artifact: str
    stderr_artifact: str
    normalized_answer: tuple[str, ...]
    expected_answer: tuple[str, ...]
    prior_expected_answer: tuple[str, ...] = ()
    status: Status = Status.ERROR
    score: Mapping[str, float | int | None] = field(default_factory=dict)
    tool_calls: int = 0
    peak_rss_bytes: int | None = None
    detail: str = ""

    def to_dict(self) -> dict[str, Any]:
        value = asdict(self)
        value["status"] = self.status.value
        return value


class Adapter(ABC):
    """The frozen API every competitor integration must implement."""

    name: str
    version: str
    commit: str

    @abstractmethod
    def prepare(self, fixture_root: Path, artifact_root: Path) -> NativeResult:
        raise NotImplementedError

    @abstractmethod
    def query_definition(self, target: str) -> NativeResult:
        raise NotImplementedError

    @abstractmethod
    def query_callers(self, target: str) -> NativeResult:
        raise NotImplementedError

    @abstractmethod
    def query_callees(self, target: str) -> NativeResult:
        raise NotImplementedError

    @abstractmethod
    def query_impact(self, target: str) -> NativeResult:
        raise NotImplementedError

    @abstractmethod
    def query_tests(self, target: str) -> NativeResult:
        raise NotImplementedError

    @abstractmethod
    def apply_mutation(self, mutation: Mapping[str, Any]) -> NativeResult:
        raise NotImplementedError

    @abstractmethod
    def wait_until_ready(self, timeout_seconds: float) -> NativeResult:
        raise NotImplementedError

    @abstractmethod
    def cleanup(self) -> None:
        raise NotImplementedError


def normalize_paths(values: Sequence[str]) -> tuple[str, ...]:
    """Canonicalize relative semantic identities without hiding duplicates."""

    normalized = []
    for raw in values:
        value = raw.strip().replace("\\", "/")
        while value.startswith("./"):
            value = value[2:]
        if value:
            normalized.append(value)
    return tuple(sorted(set(normalized)))
