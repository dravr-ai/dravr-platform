#!/usr/bin/env python3
# ABOUTME: Says which workspace crates a change reaches: the crates it edits and every crate depending on them, from Cargo.lock
# ABOUTME: One classifier for the scoped CI lanes (scope) and for server-suites.sh's changed-targets (classify-server)
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
"""Which workspace crates a change reaches (carnet#1282).

A test's outcome changes when something it builds from changes: its crate's
sources, a crate it depends on, a package Cargo.lock resolves for any of them,
or a file outside crates/ that Rust code reads. So a change to crate X can
change the tests of X and of every crate that depends on X, through
dependencies of any kind (a dev-dependency builds into the test binaries
too), and of no other crate.

Commands:

  scope --base REV [--head REV]
      For a CI lane: the changed paths between REV and HEAD, judged with the
      lockfile and root manifest of both sides. Prints for $GITHUB_OUTPUT:
        mode=full|scoped
        linked=<crates whose linked code changed, with their dependents>
        tested=<linked, plus crates whose own test sources changed>
      (package names, space-separated, sorted). The reasons go to stderr.
      Any failure answers mode=full: scoping saves minutes, so a classifier
      that cannot answer must give back the whole run, never shrink it.

  classify-server
      For server-suites.sh: changed paths on stdin, no base. Prints one line
      per path, "<full|tests|none>\t<path>\t<why>": full when the path reaches
      pierre_mcp_server's build, tests for a server test source, none
      otherwise. With no base, Cargo.lock and the root Cargo.toml cannot be
      diffed, so they answer full.

  check-graph
      Compares the crate graph read from Cargo.lock with `cargo metadata
      --no-deps` and fails on any difference: the lockfile is read because it
      needs no toolchain, and this keeps it honest where one is installed.

How a path is judged, in order:
  1. Inside a crate: its tests/, benches/ or examples/ .rs sources reach only
     that crate's tests (pierre-server's tests/ sources are named targets of
     its own; any other file under its tests/ is a fixture any target may
     read); its top-level *.md reaches nothing; anything else reaches the
     crate and its dependents.
  2. Cargo.lock: the packages whose entry differs between the two sides, and
     every crate depending on them. The root Cargo.toml: a change confined to
     [workspace.dependencies] reaches the crates naming those entries (their
     lock changes are read too); any other change reaches every crate.
  3. The toolchain pin, .cargo/ and clippy.toml reach every crate.
  4. A file a crate reads from outside crates/ (check-rust-inputs.py's scan,
     plus symlinks under crates/ such as pierre-database's migrations) reaches
     the crates reading it. So does a file of one member that another embeds
     through a `../` literal (pierre-dev-fixture embeds pierre-server's
     test fixtures), on top of what rule 1 gives it.
  5. The trees Rust code is known to read (migrations, templates, sdk, ...)
     but that the scan above did not attribute reach every crate: unknown
     fails toward running more.
  6. Docs, clients, scripts and CI configuration reach no crate. For `scope`,
     a change to the definitions the scoped lanes run (their workflows,
     actions and scripts) makes the run full, so the change is exercised.
  7. Anything else reaches every crate.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import subprocess
import sys
import tomllib
from collections.abc import Iterable
from dataclasses import dataclass, field
from pathlib import Path

SERVER = "pierre_mcp_server"

WORKSPACE_WIDE = ("rust-toolchain", ".cargo/", "clippy.toml")
# Trees the server builds from as a whole (embedded migrations, the command
# catalogue, the SDK its build script builds): a path here that the read scan
# did not attribute to a crate reaches every crate. The client packages and
# infra/ are not here: Rust reads single files there, which the scan names one
# by one, so an unread file beside them (a .ts source, variables.tf) reaches
# nothing.
KNOWN_INPUTS = ("migrations/", "migrations_pg/", "templates/", "commands/", "sdk/", "test_data/", "build.rs")
NONE_PREFIXES = (
    "docs/", "frontend/", "frontend-mobile/", "packages/", ".github/", "scripts/",
    "bin/", "load-tests/", "docker/", "infra/", "examples/", "benches/",
    ".claude/", ".agents/", ".build", ".savoir/", ".pi/", ".vscode/",
)
NONE_FILES = {
    "bun.lock", "package.json", ".gitignore", ".gitmodules", ".dockerignore",
    ".trivyignore", ".env.postgres.example", ".envrc.example", ".mcp.json",
    ".nvmrc", "deny.toml", "docker-compose.yml", "feature-phases.yaml",
    "registre.toml", "satellites.toml", "validation-patterns.local.toml",
}
# What the scoped lanes execute: a change here runs them in full, so the
# change itself is exercised before it decides any scope.
CI_DEFINITIONS = (
    ".github/workflows/ci-backend.yml", ".github/workflows/ci-postgres.yml",
    ".github/actions/", "scripts/ci/affected-crates.py", "scripts/ci/scope-base.sh",
    "scripts/ci/server-suites.sh", "scripts/ci/test-binaries.sh",
    "scripts/ci/check-leaf-test-targets.sh", "scripts/ci/proven-on-branch.sh",
    "scripts/ci/check-rust-inputs.py",
)
SERVER_TEST_SOURCE = re.compile(r"tests/([^/]+|(it|helpers)/.+)\.rs")
OWN_TARGET_SOURCE = re.compile(r"(tests|benches|examples)/.+\.rs")


class ScopeError(Exception):
    """The classifier could not read what it judges."""


@dataclass
class Verdict:
    """What one path reaches."""

    kind: str  # full | linked | tested | server-tests | none | ci
    crates: set[str] = field(default_factory=set)
    why: str = ""


@dataclass
class Workspace:
    root: Path
    members: dict[str, str]  # member dir, relative to the root -> package name
    dependents: dict[str, set[str]]  # package -> workspace packages depending on it
    reads: list[tuple[str, bool, str]]  # (path read, is a dir, reading package)

    def closure(self, names: Iterable[str]) -> set[str]:
        """The named members and every member depending on them, transitively."""
        seen: set[str] = set()
        todo = [n for n in names if n in self.members.values()]
        while todo:
            name = todo.pop()
            if name in seen:
                continue
            seen.add(name)
            todo.extend(self.dependents.get(name, ()))
        return seen

    def everything(self) -> set[str]:
        return set(self.members.values())


def git(root: Path, *args: str) -> str:
    out = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True)
    if out.returncode != 0:
        raise ScopeError(f"git {' '.join(args)}: {out.stderr.strip()[:300]}")
    return out.stdout


def git_show(root: Path, rev: str, path: str) -> str | None:
    """The file at rev, or None when rev does not have it."""
    out = subprocess.run(["git", "-C", str(root), "show", f"{rev}:{path}"], capture_output=True, text=True)
    return out.stdout if out.returncode == 0 else None


def member_dirs(root: Path) -> dict[str, str]:
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    ws = manifest.get("workspace", {})
    excluded = {e.rstrip("/") for e in ws.get("exclude", [])}
    members: dict[str, str] = {}
    for pattern in ws.get("members", []):
        for cand in sorted(root.glob(pattern)):
            rel = cand.relative_to(root).as_posix()
            if rel in excluded or not (cand / "Cargo.toml").is_file():
                continue
            pkg = tomllib.loads((cand / "Cargo.toml").read_text(encoding="utf-8")).get("package", {})
            if "name" not in pkg:
                raise ScopeError(f"{rel}/Cargo.toml names no package")
            members[rel] = pkg["name"]
    if not members:
        raise ScopeError("the root Cargo.toml lists no workspace member")
    return members


def lock_entries(text: str) -> dict[tuple[str, str, str], tuple[str, ...]]:
    """(name, version, source) -> its dependency list, for every lock entry."""
    try:
        packages = tomllib.loads(text).get("package", [])
    except tomllib.TOMLDecodeError as exc:
        raise ScopeError(f"Cargo.lock does not parse: {exc}") from exc
    return {
        (p["name"], p.get("version", ""), p.get("source", "")): tuple(sorted(p.get("dependencies", [])))
        for p in packages
    }


def reverse_edges(*entry_sets: dict[tuple[str, str, str], tuple[str, ...]]) -> dict[str, set[str]]:
    """Package name -> names of the packages depending on it, over every side given.

    By name, not by version: two versions of one package are one node, which can
    only add dependents, never lose one.
    """
    rev: dict[str, set[str]] = {}
    for entries in entry_sets:
        for (name, _, _), deps in entries.items():
            for dep in deps:
                rev.setdefault(dep.split(" ", 1)[0], set()).add(name)
    return rev


def load_reads(root: Path, members: dict[str, str]) -> list[tuple[str, bool, str]]:
    """Every file a crate reads outside its own directory: check-rust-inputs.py's scan, another member's files, symlinks."""
    spec = importlib.util.spec_from_file_location("check_rust_inputs", Path(__file__).resolve().parent / "check-rust-inputs.py")
    if spec is None or spec.loader is None:
        raise ScopeError("cannot load scripts/ci/check-rust-inputs.py")
    scan = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(scan)

    def owner(path: Path) -> str | None:
        rel = path.relative_to(root).as_posix()
        best = max((d for d in members if rel.startswith(d + "/")), key=len, default=None)
        return members[best] if best else None

    top_dirs = {p.name for p in root.iterdir() if p.is_dir() and p.name not in scan.SKIP_DIRS | {"crates"}}
    reads: list[tuple[str, bool, str]] = []
    for path in scan.rust_files(root):
        pkg = owner(path)
        if pkg is None:
            continue
        for rel, is_dir, _ in scan.reads_of(path, root, top_dirs):
            reads.append((rel, is_dir, pkg))
    # A `../` literal resolving into another member's directory: the scan
    # above keeps only reads outside crates/, but pierre-dev-fixture embeds
    # pierre-server's test fixtures, and a reader that does not depend on the
    # file's crate is outside that crate's dependency closure.
    for path in scan.rust_files(root):
        pkg = owner(path)
        if pkg is None:
            continue
        crate = scan.crate_dir(path, root)
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        for m in (m for line in lines for m in scan.LITERAL.finditer(line)):
            lit = m.group(1)
            if "../" not in lit or not scan.PATHLIKE.match(lit) or "*" in lit:
                continue
            for base in [path.parent] + ([crate] if crate else []):
                target = base / lit.lstrip("/")
                rel = scan.inside(target, root)
                if rel and rel.startswith("crates/") and target.exists():
                    if owner(root / rel) not in (None, pkg):
                        reads.append((rel.rstrip("/"), target.is_dir(), pkg))
                    break
    for base, dirs, files in os.walk(root / "crates"):
        dirs[:] = [d for d in dirs if d not in scan.SKIP_DIRS]
        for name in dirs + files:
            link = Path(base) / name
            if not link.is_symlink():
                continue
            target = scan.inside(link, root)
            pkg = owner(link)
            if target and pkg and not target.startswith("crates/"):
                reads.append((target, link.is_dir(), pkg))
    return reads


def load_workspace(root: Path) -> Workspace:
    members = member_dirs(root)
    lock = root / "Cargo.lock"
    if not lock.is_file():
        raise ScopeError("no Cargo.lock at the workspace root")
    entries = lock_entries(lock.read_text(encoding="utf-8"))
    names = set(members.values())
    rev = reverse_edges(entries)
    dependents = {n: rev.get(n, set()) & names for n in names}
    return Workspace(root, members, dependents, load_reads(root, members))


def root_manifest_reach(ws: Workspace, base_text: str | None, head_text: str | None) -> Verdict:
    """What a change to the root Cargo.toml reaches."""
    if base_text is None or head_text is None:
        return Verdict("full", why="the root Cargo.toml was added or removed")
    try:
        base, head = tomllib.loads(base_text), tomllib.loads(head_text)
    except tomllib.TOMLDecodeError as exc:
        return Verdict("full", why=f"the root Cargo.toml does not parse: {exc}")
    base_deps = base.get("workspace", {}).pop("dependencies", {})
    head_deps = head.get("workspace", {}).pop("dependencies", {})
    if base != head:
        return Verdict("full", why="the root Cargo.toml changed outside [workspace.dependencies]")
    keys = {k for k in base_deps.keys() | head_deps.keys() if base_deps.get(k) != head_deps.get(k)}
    users: set[str] = set()
    for rel, pkg in ws.members.items():
        manifest = tomllib.loads((ws.root / rel / "Cargo.toml").read_text(encoding="utf-8"))
        tables = [manifest.get(t, {}) for t in ("dependencies", "dev-dependencies", "build-dependencies")]
        for target in manifest.get("target", {}).values():
            tables += [target.get(t, {}) for t in ("dependencies", "dev-dependencies", "build-dependencies")]
        if any(k in table for table in tables for k in keys):
            users.add(pkg)
    return Verdict("linked", users, f"[workspace.dependencies] {', '.join(sorted(keys))}")


def lock_reach(ws: Workspace, base_text: str | None, head_text: str | None) -> Verdict:
    """What a change to Cargo.lock reaches: the members depending on any entry that differs."""
    if base_text is None or head_text is None:
        return Verdict("full", why="Cargo.lock was added or removed")
    try:
        base, head = lock_entries(base_text), lock_entries(head_text)
    except ScopeError as exc:
        return Verdict("full", why=str(exc))
    changed = {k[0] for k in base.keys() ^ head.keys()} | {k[0] for k in base.keys() & head.keys() if base[k] != head[k]}
    rev = reverse_edges(base, head)
    seen: set[str] = set()
    todo = list(changed)
    while todo:
        name = todo.pop()
        if name not in seen:
            seen.add(name)
            todo.extend(rev.get(name, ()))
    reached = seen & ws.everything()
    shown = ", ".join(sorted(changed)[:6]) + (", …" if len(changed) > 6 else "")
    return Verdict("linked", reached, f"Cargo.lock entries changed: {shown}")


def member_file(pkg: str, rest: str) -> Verdict:
    """What a file of member pkg reaches, rest being its path inside the member."""
    if pkg == SERVER and rest.startswith("tests/"):
        if SERVER_TEST_SOURCE.fullmatch(rest):
            return Verdict("server-tests", {pkg}, "a server test source")
        return Verdict("linked", {pkg}, "a server test-tree file no target is named for")
    if "/" not in rest and rest.endswith(".md"):
        return Verdict("none", why="crate documentation")
    if OWN_TARGET_SOURCE.fullmatch(rest):
        return Verdict("tested", {pkg}, f"a target of {pkg} itself")
    return Verdict("linked", {pkg}, f"{pkg}'s own files")


def classify(ws: Workspace, path: str, base: str | None, head: str | None) -> Verdict:
    """What one changed path reaches. base/head are revs, or None when no base is known."""
    readers = {pkg for rel, is_dir, pkg in ws.reads if path == rel or (is_dir and path.startswith(rel + "/"))}
    owner = max((d for d in ws.members if path.startswith(d + "/")), key=len, default=None)
    if owner is not None:
        v = member_file(ws.members[owner], path[len(owner) + 1:])
        # Another member embedding this file builds from it as well.
        foreign = readers - {ws.members[owner]}
        if foreign:
            return Verdict("linked", v.crates | foreign, f"{v.why}; read by {', '.join(sorted(foreign))}")
        return v
    if path.startswith("crates/"):
        return Verdict("full", why="under crates/ but in no workspace member")
    if path.startswith(WORKSPACE_WIDE):
        return Verdict("full", why="the toolchain or cargo configuration")
    if path in ("Cargo.lock", "Cargo.toml"):
        if base is None:
            return Verdict("full", why=f"{path}, with no base to diff it against")
        reach = lock_reach if path == "Cargo.lock" else root_manifest_reach
        v = reach(ws, git_show(ws.root, base, path), git_show(ws.root, head or "HEAD", path))
        # A test that reads the file itself (pierre-server reads Cargo.lock) is reached too.
        if v.kind == "linked" and readers:
            v.crates |= readers
            v.why += f"; read by {', '.join(sorted(readers))}"
        return v
    if readers:
        return Verdict("linked", readers, f"read by {', '.join(sorted(readers))}")
    if path.startswith(KNOWN_INPUTS):
        return Verdict("full", why="a tree Rust code reads, with no crate found reading this path")
    if path.startswith(CI_DEFINITIONS):
        return Verdict("ci", why="a definition the scoped lanes run")
    if (path.startswith(NONE_PREFIXES) or path in NONE_FILES or path.startswith("LICENSE")
            or ("/" not in path and path.endswith(".md"))):
        return Verdict("none", why="docs, clients, scripts or CI configuration")
    return Verdict("full", why="unclassified")


def scope(ws: Workspace, paths: list[str], base: str, head: str) -> tuple[str, set[str], set[str], list[str]]:
    """(mode, linked, tested, reasons) for a diff."""
    linked: set[str] = set()
    tested: set[str] = set()
    reasons: list[str] = []
    full = False
    for path in paths:
        v = classify(ws, path, base, head)
        if v.kind in ("full", "ci"):
            full = True
            reasons.append(f"every crate: {path} ({v.why})")
        elif v.kind == "linked":
            linked |= v.crates
            reasons.append(f"{path}: {', '.join(sorted(v.crates)) or 'no crate'} ({v.why})")
        elif v.kind in ("tested", "server-tests"):
            tested |= v.crates
            reasons.append(f"{path}: tests of {', '.join(sorted(v.crates))} ({v.why})")
    if full:
        everything = ws.everything()
        return "full", everything, everything, reasons
    linked = ws.closure(linked)
    return "scoped", linked, linked | tested, reasons


def cmd_scope(root: Path, base: str, head: str) -> int:
    try:
        ws = load_workspace(root)
        paths = [p for p in git(root, "diff", "--name-only", "--no-renames", base, head).splitlines() if p]
        mode, linked, tested, reasons = scope(ws, paths, base, head)
    except (ScopeError, OSError, KeyError, tomllib.TOMLDecodeError) as exc:
        print(f"::warning title=Scope unknown::affected-crates.py could not judge {base}..{head}: {exc}; testing every crate", file=sys.stderr)
        mode, linked, tested, reasons = "full", set(), set(), []
    for line in reasons[:12]:
        print(f"affected-crates: {line}", file=sys.stderr)
    if len(reasons) > 12:
        print(f"affected-crates: … and {len(reasons) - 12} more paths", file=sys.stderr)
    print(f"affected-crates: mode={mode}, {len(tested)} crate(s) to test", file=sys.stderr)
    print(f"mode={mode}")
    print(f"linked={' '.join(sorted(linked))}")
    print(f"tested={' '.join(sorted(tested))}")
    return 0


def cmd_classify_server(root: Path) -> int:
    paths = [line.rstrip("\n") for line in sys.stdin if line.strip()]
    try:
        ws = load_workspace(root)
    except (ScopeError, OSError, KeyError, tomllib.TOMLDecodeError) as exc:
        for path in paths:
            print(f"full\t{path}\tthe crate graph is unknown ({exc})")
        return 0
    for path in paths:
        v = classify(ws, path, None, None)
        if v.kind == "full":
            print(f"full\t{path}\t{v.why}")
        elif v.kind == "server-tests":
            print(f"tests\t{path}\t{v.why}")
        elif v.kind == "linked" and SERVER in ws.closure(v.crates):
            print(f"full\t{path}\t{v.why}, in the {SERVER} dependency closure")
        elif v.kind == "linked":
            print(f"none\t{path}\t{', '.join(sorted(v.crates))} outside the {SERVER} dependency closure")
        else:
            print(f"none\t{path}\t{v.why}")
    return 0


def cmd_check_graph(root: Path) -> int:
    ws = load_workspace(root)
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--offline"],
        cwd=root, capture_output=True, text=True,
    )
    if out.returncode != 0:
        print(f"❌ affected-crates check-graph: cargo metadata failed: {out.stderr.strip()[:300]}")
        return 2
    meta = json.loads(out.stdout)
    names = {p["name"] for p in meta["packages"]}
    problems = []
    if names != ws.everything():
        problems.append(f"members differ: {sorted(names ^ ws.everything())}")
    for pkg in meta["packages"]:
        want = {d["name"] for d in pkg["dependencies"] if d.get("path")}
        have = {n for n, users in ws.dependents.items() if pkg["name"] in users}
        if want != have:
            problems.append(f"{pkg['name']}: cargo metadata {sorted(want - have)} vs Cargo.lock {sorted(have - want)}")
    if problems:
        print("❌ affected-crates check-graph: the Cargo.lock crate graph differs from cargo metadata:")
        for p in problems:
            print(f"   {p}")
        return 1
    print(f"✅ affected-crates check-graph: {len(names)} crates, the Cargo.lock graph matches cargo metadata")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    sub = parser.add_subparsers(dest="command", required=True)
    p_scope = sub.add_parser("scope")
    p_scope.add_argument("--base", required=True)
    p_scope.add_argument("--head", default="HEAD")
    sub.add_parser("classify-server")
    sub.add_parser("check-graph")
    args = parser.parse_args(argv)
    root = args.root.resolve()
    if args.command == "scope":
        return cmd_scope(root, args.base, args.head)
    if args.command == "classify-server":
        return cmd_classify_server(root)
    return cmd_check_graph(root)


if __name__ == "__main__":
    sys.exit(main())
