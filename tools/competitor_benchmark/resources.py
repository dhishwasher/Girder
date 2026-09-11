"""Fail-closed resource checks for the low-memory campaign host."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class MemorySnapshot:
    total_bytes: int
    available_bytes: int
    swap_total_bytes: int
    swap_free_bytes: int


def read_meminfo(path: Path = Path("/proc/meminfo")) -> MemorySnapshot:
    values: dict[str, int] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        key, raw = line.split(":", 1)
        values[key] = int(raw.strip().split()[0]) * 1024
    required = ("MemTotal", "MemAvailable", "SwapTotal", "SwapFree")
    missing = [key for key in required if key not in values]
    if missing:
        raise ValueError(f"meminfo lacks required fields: {', '.join(missing)}")
    return MemorySnapshot(*(values[key] for key in required))


def resource_block_reason(snapshot: MemorySnapshot, minimum_available_bytes: int) -> str | None:
    if snapshot.available_bytes < minimum_available_bytes:
        return (
            f"MemAvailable={snapshot.available_bytes} below frozen minimum "
            f"{minimum_available_bytes}"
        )
    return None
