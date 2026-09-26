#!/usr/bin/env python3
# ABOUTME: Extracts every HTTP route the backend serves, resolving .nest() mounts and multi-line .route( calls
# ABOUTME: One scanner for the nginx prefix snapshot and the phantom-surface /api/ route gate
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
"""Print the routes the backend serves.

    backend-routes.py prefixes [ROOT]   top-level path prefixes, one per line
                                        (nginx-routing-check.sh's snapshot)
    backend-routes.py routes [ROOT]     every full route as PATH<TAB>FILE:LINE,
                                        one line per declaration
                                        (check-phantom-surfaces.sh Scan 4)

Why this is not a grep
----------------------
Three things a line grep gets wrong, and each hid real routes:

1. Routers are mounted two ways:

       app.merge(build_store_router())                        # routes stay as written
       app.nest("/api/admin", build_agents_admin_router())    # routes gain a prefix

   ``build_agents_admin_router`` declares ``.route("/agents/{id}", …)`` but
   serves ``/api/admin/agents/{id}``. So every ``.nest`` site is found, its
   router argument is resolved to the builder function that declares the routes
   — directly, or through the ``let`` binding the argument names — and that
   function's routes get the mount prefix.

2. rustfmt splits a long registration across lines:

       .route(
           "/api/agents/{id}/versions",
           get(versions::handle_list_versions::<C>),
       )

   A line grep reads none of these. When this scanner was line-based it missed
   more ``/api`` routes than it read (160 against 71), and the phantom-surface
   gate passed routes no client called because it never saw them.

3. A path is the whole template. Cutting ``/api/agents/{id}/fork`` at its first
   parameter leaves ``/api/agents``, which a client mentions, so a route nothing
   calls read as called.

The scan asserts its own premises instead of trusting them, because every one of
them fails silent — a route the scan cannot read is a route no gate checks:

* in a file that uses axum, every ``.route(`` and ``.nest(`` takes a string
  literal path (a computed path would be invisible);
* every ``.nest`` resolves to exactly one builder function, defined once, that
  declares at least one route (an unresolved mount would report its routes
  without their prefix);
* no mounted builder itself nests (the prefix would need composing);
* the scan finds at least one route.

Any violation exits 2 with the sites named, rather than printing a partial list.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

# `.route("/path"` — `\s*` spans the newline rustfmt puts after `.route(`.
ROUTE = re.compile(r'\.route\(\s*"(/[^"]*)"')
ROUTE_CALL = re.compile(r"\.route\(")
# `.nest("/prefix", <arg>` — the argument is a builder call (optionally
# path-qualified) or a local variable holding one.
NEST = re.compile(r'\.nest\(\s*"(/[^"]*)"\s*,\s*(?:[\w:]*::)?(\w+)')
NEST_CALL = re.compile(r"\.nest\(")
# Every call in a `let` statement, so a nested variable resolves to the builder
# it was bound from however that call is wrapped (`map_or_else(|| …, |x| builder(x))`).
CALL = re.compile(r"(\w+)\s*(?:::<[^>]*>)?\s*\(")
# `fn name(` / `pub fn name(` — attributes routes to their enclosing function.
FN = re.compile(
    r"^[ \t]*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?fn\s+(\w+)", re.M
)


class ScanError(RuntimeError):
    """The scan could not stand behind its own result."""


def _line_of(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def _statement_end(text: str, start: int) -> int:
    """Offset of the `;` that ends the statement beginning at `start`.

    Depth-counted over (), [] and {} with string literals skipped, so the `;`
    inside a closure body (`|| { tracing::warn!(…); Router::new() }`) does not
    end the `let` early — a `[^;]*;` body stopped there and left the
    /api/admin/config mount unresolved.
    """
    depth = 0
    i = start
    in_str = False
    while i < len(text):
        c = text[i]
        if in_str:
            if c == "\\":
                i += 1
            elif c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
            if depth < 0:
                return i
        elif c == ";" and depth == 0:
            return i
        i += 1
    return len(text)


def _enclosing(fns: list[tuple[int, str]], offset: int) -> str:
    name = ""
    for start, fn in fns:
        if start > offset:
            break
        name = fn
    return name


def source_files(root: Path) -> list[Path]:
    # Production source only: `tests/` fixtures and dev-only fixture binaries do
    # not serve traffic.
    return sorted(
        p
        for p in root.glob("crates/*/src/**/*.rs")
        if "dev-fixture" not in str(p) and "/tests/" not in str(p)
    )


def scan(root: Path) -> list[tuple[str, str, int]]:
    """Every route the backend serves, as (full_path, file, line).

    Raises ScanError when a premise in the module docstring does not hold.
    """
    problems: list[str] = []
    # fn name -> [(route_path, file, line)]
    routes_by_fn: dict[str, list[tuple[str, str, int]]] = {}
    # fn name -> files defining it
    fn_files: dict[str, set[str]] = {}
    # (prefix, arg, file, line, text, offset, enclosing fn)
    nests: list[tuple[str, str, str, int, str, int, str]] = []

    for path in source_files(root):
        text = path.read_text(errors="ignore")
        rel = str(path.relative_to(root))
        fns = [(m.start(), m.group(1)) for m in FN.finditer(text)]
        for _, fn in fns:
            fn_files.setdefault(fn, set()).add(rel)

        literal_routes = list(ROUTE.finditer(text))
        literal_nests = list(NEST.finditer(text))
        if "axum" in text:
            literal_at = {m.start() for m in literal_routes}
            for m in ROUTE_CALL.finditer(text):
                if m.start() not in literal_at:
                    problems.append(
                        f"{rel}:{_line_of(text, m.start())}: .route( without a "
                        "string-literal path"
                    )
            nest_at = {m.start() for m in literal_nests}
            for m in NEST_CALL.finditer(text):
                if m.start() not in nest_at:
                    problems.append(
                        f"{rel}:{_line_of(text, m.start())}: .nest( without a "
                        "string-literal prefix"
                    )

        for m in literal_routes:
            fn = _enclosing(fns, m.start())
            routes_by_fn.setdefault(fn, []).append(
                (m.group(1), rel, _line_of(text, m.start()))
            )
        for m in literal_nests:
            nests.append(
                (
                    m.group(1),
                    m.group(2),
                    rel,
                    _line_of(text, m.start()),
                    text,
                    m.start(),
                    _enclosing(fns, m.start()),
                )
            )

    mounts: dict[str, str] = {}
    for prefix, arg, rel, line, text, offset, _host_fn in nests:
        builder = arg if arg in routes_by_fn else ""
        if not builder:
            # A variable: the nearest `let arg =` before the mount site.
            binds = list(re.finditer(rf"\blet\s+{re.escape(arg)}\s*=", text[:offset]))
            if binds:
                body = text[binds[-1].end() : _statement_end(text, binds[-1].end())]
                called = {c for c in CALL.findall(body) if c in routes_by_fn}
                if len(called) == 1:
                    builder = called.pop()
                elif called:
                    problems.append(
                        f"{rel}:{line}: .nest({prefix!r}, {arg}) binds several "
                        f"route builders: {sorted(called)}"
                    )
                    continue
        if not builder:
            problems.append(
                f"{rel}:{line}: .nest({prefix!r}, {arg}) resolves to no function "
                "that declares a route"
            )
            continue
        defined_in = len(fn_files.get(builder, ()))
        if defined_in != 1:
            problems.append(
                f"{rel}:{line}: .nest({prefix!r}, …) mounts {builder}, which is "
                f"defined in {defined_in} files — the prefix cannot be attributed"
            )
            continue
        mounts[builder] = prefix

    for _prefix, _arg, rel, line, _text, _offset, host_fn in nests:
        if host_fn in mounts:
            problems.append(
                f"{rel}:{line}: {host_fn} is mounted under {mounts[host_fn]!r} and "
                "nests again — composed prefixes are not modelled"
            )

    if problems:
        raise ScanError("\n  ".join(["route scan premise violated:"] + problems))

    found: list[tuple[str, str, int]] = []
    for fn, routes in routes_by_fn.items():
        prefix = mounts.get(fn, "")
        for route, rel, line in routes:
            full = prefix if (prefix and route == "/") else prefix + route
            found.append((full, rel, line))
    if not found:
        raise ScanError(
            f"no .route( declarations found under {root}/crates — refusing to pass"
        )
    return sorted(found)


def first_segment(path: str) -> str:
    parts = [p for p in path.split("/") if p]
    return parts[0] if parts else ""


def main(argv: list[str]) -> int:
    if len(argv) < 2 or argv[1] not in ("prefixes", "routes"):
        print(__doc__, file=sys.stderr)
        return 64
    root = Path(argv[2] if len(argv) > 2 else ".")
    try:
        routes = scan(root)
    except ScanError as err:
        print(f"backend-routes.py: {err}", file=sys.stderr)
        return 2

    if argv[1] == "routes":
        for full, rel, line in routes:
            print(f"{full}\t{rel}:{line}")
        return 0

    prefixes: set[str] = set()
    for full, _rel, _line in routes:
        segment = first_segment(full)
        # A `{param}` first segment would come from a router mounted elsewhere;
        # the mount prefix is what matters and is captured on that side.
        if segment and not segment.startswith("{"):
            prefixes.add(segment)
    for prefix in sorted(prefixes):
        print(prefix)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
