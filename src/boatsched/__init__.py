"""boatsched -- boat assignment optimizer.

Assign kids to boats over several days under level constraints, with a strict
lexicographic objective: minimize mixed boats on restricted days (A), then
maximize level diversity on diverse days (B), then minimize repeat
co-boatings across the whole period (C).
"""
from .solver import (
    Config,
    SolveError,
    ValidationError,
    build_model,
    staged_solve,
    extract_schedule,
    solve,
)
from .roster import load_roster, roster_from_counts, parse_roster_bytes

__all__ = [
    "Config", "SolveError", "ValidationError",
    "build_model", "staged_solve", "extract_schedule", "solve",
    "load_roster", "roster_from_counts", "parse_roster_bytes",
]
__version__ = "0.1.0"
