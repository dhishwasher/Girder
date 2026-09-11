"""Materialize the committed corpus without following paths outside its root."""

from __future__ import annotations

import json
import shutil
from pathlib import Path, PurePosixPath
from typing import Any, Mapping


def safe_relative_path(value: str) -> PurePosixPath:
    path = PurePosixPath(value)
    if path.is_absolute() or not path.parts or any(part in ("", ".", "..") for part in path.parts):
        raise ValueError(f"unsafe fixture path: {value!r}")
    return path


def load_corpus(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema_version") != 1:
        raise ValueError("unsupported corpus schema")
    return data


def materialize(fixture: Mapping[str, Any], destination: Path) -> None:
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True)
    _write_files(destination, fixture["base_files"])


def apply_mutation(destination: Path, mutation: Mapping[str, Any]) -> None:
    for raw in mutation.get("delete", []):
        target = _contained_target(destination, raw)
        if target.is_dir():
            raise ValueError("fixture mutations may delete files only")
        target.unlink(missing_ok=True)
    _write_files(destination, mutation.get("write", {}))


def _write_files(destination: Path, files: Mapping[str, str]) -> None:
    for raw, content in files.items():
        target = _contained_target(destination, raw)
        target.parent.mkdir(parents=True, exist_ok=True)
        target = _contained_target(destination, raw)
        target.write_text(content, encoding="utf-8")


def _contained_target(destination: Path, raw: str) -> Path:
    root = destination.resolve()
    target = destination.joinpath(*safe_relative_path(raw).parts)
    resolved = target.resolve(strict=False)
    if resolved == root or root not in resolved.parents:
        raise ValueError(f"fixture path escapes root: {raw!r}")
    return target
