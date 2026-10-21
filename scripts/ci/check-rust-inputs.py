#!/usr/bin/env python3
# ABOUTME: Fails when Rust code under crates/ reads a repo file outside crates/ that ci-backend's `rust` filter misses
# ABOUTME: A missed input skips every compile and test job on a push that changes it — a green nobody earned
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
"""Coverage check for ci-backend.yml's Rust-input gate (carnet#1114).

ci-backend.yml runs its compile and test jobs only when a push touches a path
in the ``rust`` filter of ``.github/actions/ci-backend-changes/action.yml``.
Rust code also reads files outside ``crates/``: locale catalogues through
``include_str!``, migrations through ``sqlx::migrate!``, Terraform and
frontend sources that tests hold the code to. A push changing one of those
and nothing else would skip the very tests that read it. This fails when a
file or directory a crate reads is not covered by the filter.

What counts as a read, in every ``.rs`` file under ``crates/``:

* a string literal holding a ``../`` path, resolved against the file's own
  directory (``include_str!``) and against its crate's directory
  (``CARGO_MANIFEST_DIR``, ``sqlx::migrate!``); a candidate counts when it
  exists and lies outside ``crates/``;
* a literal naming a path from the repository root (``"docker/images/..."``)
  that exists, read through a repo-root helper;
* ``.join("<dir>")`` of a top-level directory (``workspace_root.join("sdk")``).

A literal that resolves to nothing on disk is not a read and is skipped, so a
path in an assertion message or a traversal-attack fixture cannot fail it.

The filter only judges a push that started a run, so every glob in it must
also be reachable from ci-backend.yml's own ``on.push.paths`` and
``on.pull_request.paths``: a representative path for each glob (``**`` and
``*`` replaced by a name) must match one of them.

Exit status: 0 covered, 1 an uncovered read or an unreachable glob, 2 when
the filter or the triggers cannot be read or the scan found no read at all,
since a scan that found nothing verified nothing.
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

import yaml

ACTION = Path(".github/actions/ci-backend-changes/action.yml")
WORKFLOW = Path(".github/workflows/ci-backend.yml")
SKIP_DIRS = {"target", "node_modules", ".git"}
LITERAL = re.compile(r'"((?:\\.|[^"\\])*)"')
# A chain of .join("segment") calls, which rustfmt spreads over lines.
JOIN_CHAIN = re.compile(r'(?:\.join\(\s*"[A-Za-z0-9_.-]+"\s*\)\s*)+')
JOIN_SEGMENT = re.compile(r'\.join\(\s*"([A-Za-z0-9_.-]+)"\s*\)')
PATHLIKE = re.compile(r"^/?[A-Za-z0-9_./*-]+$")


class CheckError(Exception):
    """The check could not read what it judges; a pass would verify nothing."""


def rust_globs(root: Path) -> list[str]:
    try:
        doc = yaml.safe_load((root / ACTION).read_text(encoding="utf-8"))
        step = next(s for s in doc["runs"]["steps"] if s.get("id") == "filter")
        filters = yaml.safe_load(step["with"]["filters"])
        globs = [str(g) for g in filters["rust"]]
    except (OSError, yaml.YAMLError, KeyError, StopIteration, TypeError) as exc:
        raise CheckError(f"cannot read the `rust` filter from {ACTION}: {exc}") from exc
    if not globs:
        raise CheckError(f"{ACTION}: the `rust` filter is empty")
    return globs


def trigger_paths(root: Path) -> dict[str, list[str]]:
    """ci-backend.yml's push and pull_request `paths`, by event."""
    try:
        doc = yaml.safe_load((root / WORKFLOW).read_text(encoding="utf-8"))
        # PyYAML reads the bare key `on` as the boolean True.
        on = doc.get("on", doc.get(True))
        paths = {event: [str(p) for p in on[event]["paths"]] for event in ("push", "pull_request")}
    except (OSError, yaml.YAMLError, KeyError, TypeError, AttributeError) as exc:
        raise CheckError(f"cannot read the push and pull_request paths of {WORKFLOW}: {exc}") from exc
    return paths


def representative(glob: str) -> str:
    """A path the glob matches, to test against another glob list."""
    return glob.replace("**", "x").replace("*", "x").replace("?", "x")


def glob_regex(glob: str) -> re.Pattern[str]:
    """picomatch's semantics as paths-filter applies them: ** spans '/', * does not."""
    out, i = [], 0
    while i < len(glob):
        if glob.startswith("**/", i):
            out.append("(?:.*/)?")
            i += 3
        elif glob.startswith("**", i):
            out.append(".*")
            i += 2
        elif glob[i] == "*":
            out.append("[^/]*")
            i += 1
        elif glob[i] == "?":
            out.append("[^/]")
            i += 1
        else:
            out.append(re.escape(glob[i]))
            i += 1
    return re.compile("^" + "".join(out) + "$")


def covered(rel: str, is_dir: bool, patterns: list[re.Pattern[str]]) -> bool:
    # A directory is covered when anything under it would match.
    probe = f"{rel}/any-file" if is_dir else rel
    return any(p.match(probe) for p in patterns)


def crate_dir(path: Path, root: Path) -> Path | None:
    for parent in path.parents:
        if (parent / "Cargo.toml").is_file() and parent != root:
            return parent
        if parent == root:
            break
    return None


def rust_files(root: Path):
    for base, dirs, files in os.walk(root / "crates"):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        for name in files:
            if name.endswith(".rs"):
                yield Path(base) / name


def inside(path: Path, root: Path) -> str | None:
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        return None


def reads_of(path: Path, root: Path, top_dirs: set[str]):
    """(repo-relative target, is_dir, line number) for each read in one file."""
    text = path.read_text(encoding="utf-8", errors="replace")
    crate = crate_dir(path, root)
    for m in JOIN_CHAIN.finditer(text):
        segments = JOIN_SEGMENT.findall(m.group(0))
        if segments[0] not in top_dirs:
            continue
        # The deepest prefix of the chain that exists is what is read.
        rel = segments[0]
        for seg in segments[1:]:
            if not (root / rel / seg).exists():
                break
            rel = f"{rel}/{seg}"
        yield rel, (root / rel).is_dir(), text.count("\n", 0, m.start()) + 1
    for lineno, line in enumerate(text.splitlines(), 1):
        for m in LITERAL.finditer(line):
            lit = m.group(1)
            if not lit or not PATHLIKE.match(lit) or "*" in lit:
                continue
            if "../" in lit:
                rel_lit = lit.lstrip("/")
                bases = [path.parent] + ([crate] if crate else [])
                for base in bases:
                    target = base / rel_lit
                    rel = inside(target, root)
                    # The repo root itself is a base a helper joins from
                    # (`repo_root()`), not a read; what it joins is caught
                    # as a repo-root literal or a join chain.
                    if rel and rel != "." and target.exists() and not rel.startswith("crates/") and rel != "crates":
                        yield rel, target.is_dir(), lineno
                        break
            elif lit.split("/", 1)[0] in top_dirs and "/" in lit:
                target = root / lit
                if target.exists():
                    yield lit.rstrip("/"), target.is_dir(), lineno


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("root", nargs="?", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args(argv)
    root = args.root.resolve()
    try:
        globs = rust_globs(root)
        triggers = trigger_paths(root)
    except CheckError as exc:
        print(f"❌ check-rust-inputs: {exc}")
        return 2
    patterns = [glob_regex(g) for g in globs]
    unreachable = []
    for event, paths in triggers.items():
        event_patterns = [glob_regex(p) for p in paths]
        for g in globs:
            if not any(p.match(representative(g)) for p in event_patterns):
                unreachable.append(f"{g} ({event})")
    if unreachable:
        print(f"❌ check-rust-inputs: {len(unreachable)} `rust` glob(s) in {ACTION} that {WORKFLOW}'s paths never start a run for:")
        for u in unreachable:
            print(f"   {u}")
        print("   A push touching only one of them runs no ci-backend job at all; add it to both path lists.")
        return 1
    top_dirs = {p.name for p in root.iterdir() if p.is_dir() and p.name not in SKIP_DIRS | {"crates"}}
    reads = 0
    misses: dict[str, list[str]] = {}
    for path in sorted(rust_files(root)):
        for rel, is_dir, lineno in reads_of(path, root, top_dirs):
            reads += 1
            if not covered(rel, is_dir, patterns):
                misses.setdefault(rel + ("/" if is_dir else ""), []).append(f"{path.relative_to(root)}:{lineno}")
    if reads == 0:
        print("❌ check-rust-inputs: found no read of a file outside crates/; the scan verified nothing")
        return 2
    if misses:
        print(f"❌ check-rust-inputs: {len(misses)} path(s) Rust code reads are missing from the `rust` filter in {ACTION}.")
        print("   A push touching only them would skip every compile and test job that reads them.")
        for rel, sites in sorted(misses.items()):
            print(f"   {rel}  (read at {', '.join(sites[:3])}{', …' if len(sites) > 3 else ''})")
        return 1
    print(f"✅ check-rust-inputs: {reads} reads of files outside crates/, every one covered by the `rust` filter")
    return 0


if __name__ == "__main__":
    sys.exit(main())
