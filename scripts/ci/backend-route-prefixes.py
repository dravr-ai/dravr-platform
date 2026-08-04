#!/usr/bin/env python3
# ABOUTME: Extracts the backend's top-level HTTP path prefixes, resolving .nest() mounts
# ABOUTME: Feeds the nginx routing drift guard so a new route family cannot go unproxied
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
"""Print every top-level path prefix the backend serves, one per line.

Why this is not a grep
----------------------
The naive version — take the first segment of every ``.route("/x/…")`` string —
is wrong, because routers are mounted two different ways:

    app.merge(build_store_router())                  # routes stay as written
    app.nest("/api/admin", build_coaches_admin_router())   # routes gain a prefix

``build_coaches_admin_router`` declares ``.route("/coaches", …)`` but serves
``/api/admin/coaches``. A first-segment grep reports ``/coaches`` as an unrouted
top-level family and cries wolf; a first draft of this guard produced 14 such
false positives, which is worse than no guard at all — people learn to ignore it
and then it misses the real one.

So: find which builder functions are ``.nest``ed and under what prefix, attribute
every ``.route()`` to the function that contains it, and prepend the mount prefix
before taking the first segment.

Limitation, stated rather than hidden: this reads source text, not a compiled
router. A route mounted through a construct this does not model (a nest whose
router argument is a local variable, say) would be attributed to its literal
path. That fails *safe* — the prefix is reported as top-level and the guard asks
a human to classify it — never silently dropped.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

# `.route("/path"` — the leading segment is what nginx matches on.
ROUTE = re.compile(r'\.route\(\s*"(/[^"]*)"')
# `.nest("/prefix", <arg>` — the argument is either a builder call or a local
# variable holding one. Both forms appear in multitenant.rs, so both are
# resolved; matching only the call form left /api/admin/config unresolved and
# reported five of its routes as bogus top-level families.
NEST = re.compile(r'\.nest\(\s*"(/[^"]*)"\s*,\s*(?:[\w:]*::)?(\w+)')
# `let some_routes = … admin_config_router(…)` — ties a nested variable to the
# builder it was assigned from. Matches only `*_router` / `*_routes` calls: the
# binding expression often wraps the builder in `map_or_else`/`unwrap_or_else`,
# so taking the first call in the expression captured `as_ref` instead.
LET_BINDS = re.compile(
    r'let\s+(\w+)\s*=(?P<body>[^;]*?);', re.S)
BUILDER_CALL = re.compile(r'(?:[\w:]*::)?(\w+(?:_router|_routes))\s*(?:::<[^>]*>)?\s*\(')
# `fn name(` / `pub fn name(` — attributes routes to their enclosing function.
FN = re.compile(r'^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)')


def nested_builders(files: list[Path]) -> dict[str, str]:
    """Map builder-function name -> mount prefix, for every ``.nest`` site.

    Handles both ``.nest("/p", builder())`` and ``.nest("/p", var)`` where
    ``var`` was bound from a builder call earlier in the same file.
    """
    mounts: dict[str, str] = {}
    for path in files:
        text = path.read_text(errors="ignore")
        # variable name -> builder function it was assigned from
        bindings = {}
        for match in LET_BINDS.finditer(text):
            builders = BUILDER_CALL.findall(match.group("body"))
            if builders:
                bindings[match.group(1)] = builders[-1]
        for prefix, arg in NEST.findall(text):
            # `arg` is either the builder itself or a variable holding one.
            mounts[bindings.get(arg, arg)] = prefix
    return mounts


def routes_by_function(path: Path) -> list[tuple[str, str]]:
    """Return (enclosing_fn, route_path) for every ``.route()`` in a file."""
    found: list[tuple[str, str]] = []
    current = ""
    for line in path.read_text(errors="ignore").splitlines():
        fn_match = FN.match(line)
        if fn_match:
            current = fn_match.group(1)
        for route in ROUTE.findall(line):
            found.append((current, route))
    return found


def first_segment(path: str) -> str:
    parts = [p for p in path.split("/") if p]
    return parts[0] if parts else ""


def main() -> int:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    # Production source only: `tests/` fixtures and dev-only fixture binaries do
    # not serve traffic, and reporting them would be another false positive.
    files = [
        p
        for p in root.glob("crates/*/src/**/*.rs")
        if "dev-fixture" not in str(p) and "/tests/" not in str(p)
    ]

    mounts = nested_builders(files)
    prefixes: set[str] = set()
    for path in files:
        for fn, route in routes_by_function(path):
            full = mounts.get(fn, "") + route
            segment = first_segment(full)
            # `{param}` first segments come from routers mounted elsewhere; the
            # mount prefix is what matters and is captured on that side.
            if segment and not segment.startswith("{"):
                prefixes.add(segment)

    for prefix in sorted(prefixes):
        print(prefix)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
