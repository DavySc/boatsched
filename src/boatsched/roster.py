"""Roster input: load (name, level) pairs from CSV files, bytes, or counts."""
from __future__ import annotations
import csv
import io
from typing import List, Tuple

Kid = Tuple[str, str]   # (name, level)


class RosterError(ValueError):
    """Raised when roster input is malformed."""


def _read_dictreader(reader: csv.DictReader) -> List[Kid]:
    if not reader.fieldnames:
        raise RosterError("empty CSV")
    cols = {c.lower().strip(): c for c in reader.fieldnames}
    if "name" not in cols or "level" not in cols:
        raise RosterError("CSV must have columns: name, level")
    kids: List[Kid] = []
    for row in reader:
        name = (row.get(cols["name"]) or "").strip()
        level = (row.get(cols["level"]) or "").strip()
        if name:
            kids.append((name, level))
    if not kids:
        raise RosterError("no data rows")
    return kids


def load_roster(path: str) -> List[Kid]:
    """Load a roster from a CSV file with columns name, level."""
    with open(path, newline="", encoding="utf-8-sig") as f:
        return _read_dictreader(csv.DictReader(f))


def parse_roster_bytes(raw: bytes) -> List[Kid]:
    """Load a roster from raw CSV bytes (e.g. an HTTP upload)."""
    text = raw.decode("utf-8-sig", errors="replace")
    return _read_dictreader(csv.DictReader(io.StringIO(text)))


def roster_from_counts(counts: dict) -> List[Kid]:
    """Build a synthetic roster from {level: n} counts."""
    kids: List[Kid] = []
    for level, n in counts.items():
        for j in range(int(n)):
            kids.append((f"{level}{j}", level))
    return kids
