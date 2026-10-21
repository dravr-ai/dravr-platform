#!/usr/bin/env python3
# ABOUTME: Reports the Actions cache against its 10 GB quota: size per family, and the families the push lanes need
# ABOUTME: Warns when a priority family is missing, the total is over quota, or an entry is for a retired toolchain
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
"""Actions cache budget report (carnet#1114).

GitHub keeps 10 GB of Actions cache per repository and evicts the least
recently used entries above it. On 2026-10-21 the repository held 10.85 GB:
the two mobile-server caches took 65% of it, a Rust 1.98.1 pair dead since the
toolchain bump held 2.47 GB, and the caches the push lanes restore —
postgres-tests, test-dev, leaf-crate-tests, release — were absent. A miss costs
minutes on every job that restores them, and nothing said so.

So this reads every entry, groups them by family, and writes one table to the
step summary. It emits a ``::warning::`` for each family the push lanes depend
on that has no entry, when the total is over the quota, and for a cargo entry
(``<OS>-cargo-<family>-<version>-...``) whose key names a Rust toolchain other
than ``rust-toolchain.toml``'s. A version in any other key (a Python or Node
cache) is not a Rust toolchain and is never judged. It reports;
it never fails on what it finds, because the cache refills on its own schedule.

A family is the cache's purpose, not one key:

* ``rust:<shared-key>`` — Swatinem/rust-cache entries, ``v0-rust-<key>-<OS>-<arch>-...``
* ``mobile-server:<OS>`` — the full target/ of ``cargo build --release`` that
  mobile-e2e-*.yml save and integration-tests.yml and mcp-compliance.yml restore
* anything else is grouped by the key's text before its first hash.

Exit status: 0 with a report, 1 when the entries could not be read, and 2 when
the read returned no entry at all, since an empty answer from a repository
that saves caches daily means the read verified nothing.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

QUOTA_BYTES = 10 * 10**9

# What the push lanes restore: the plan's ranking (Q15), with the Linux
# mobile-server entry added before the macOS one. The plan listed it to fold or
# drop; it is kept because integration-tests, mcp-compliance and the Android
# lane all restore it, and dropping it would build thin release cold on most
# branch pushes (carnet#1114).
PRIORITY_FAMILIES = (
    "rust:postgres-tests",
    "rust:test-dev",
    "rust:clippy",
    "rust:leaf-crate-tests",
    "rust:release",
    "mobile-server:Linux",
    "mobile-server:macOS",
)

# rust-cache appends Node's os.type() and os.arch(): Linux, Darwin or
# Windows_NT, then x64 or arm64 (its src/config.ts), never RUNNER_OS's spelling.
RUST_CACHE_KEY = re.compile(r"^v0-rust-(?P<shared>.+?)-(?:Linux|Darwin|Windows_NT)-(?:x64|arm64)-")
MOBILE_SERVER_KEY = re.compile(r"^(?P<os>[A-Za-z]+)-cargo-mobile-server-(?P<toolchain>\d+\.\d+\.\d+)-")
# The toolchain a hand-keyed cargo cache names, e.g. Linux-cargo-mobile-server-1.99.1-<hash>.
TOOLCHAIN_IN_KEY = re.compile(r"(?:^|-)cargo-(?:[A-Za-z0-9]+-)*?(\d+\.\d+\.\d+)-")


class BudgetError(Exception):
    """The cache entries could not be read; the report would verify nothing."""


@dataclass
class Family:
    name: str
    entries: int = 0
    size: int = 0
    last_accessed: str = ""
    keys: list[str] = field(default_factory=list)


def family_of(key: str) -> str:
    """The family a cache key belongs to."""
    if m := RUST_CACHE_KEY.match(key):
        return f"rust:{m.group('shared')}"
    if m := MOBILE_SERVER_KEY.match(key):
        return f"mobile-server:{m.group('os')}"
    # A key's variable tail is a hash (hex, or bun's base64): keep the
    # segments before the first one that looks like one.
    kept = []
    for part in key.split("-"):
        if re.fullmatch(r"[0-9a-f]{8,}", part) or len(part) >= 20:
            break
        kept.append(part)
    return "-".join(kept) or key


def toolchain(repo_root: Path) -> str:
    text = (repo_root / "rust-toolchain.toml").read_text(encoding="utf-8")
    m = re.search(r'^channel\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if not m:
        raise BudgetError("rust-toolchain.toml names no channel")
    return m.group(1)


def read_entries(repo: str, from_json: Path | None) -> list[dict]:
    """Every cache entry, from the API or from a saved listing."""
    if from_json is not None:
        raw = from_json.read_text(encoding="utf-8")
    else:
        cmd = ["gh", "api", "--paginate", f"repos/{repo}/actions/caches?per_page=100", "--jq", ".actions_caches[]"]
        proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
        if proc.returncode != 0:
            raise BudgetError(f"gh api failed ({proc.returncode}): {proc.stderr.strip()}")
        raw = proc.stdout
    entries = []
    for line in raw.splitlines():
        line = line.strip()
        if line:
            entries.append(json.loads(line))
    return entries


def summarise(entries: list[dict]) -> dict[str, Family]:
    families: dict[str, Family] = {}
    for e in entries:
        name = family_of(e["key"])
        fam = families.setdefault(name, Family(name))
        fam.entries += 1
        fam.size += int(e["size_in_bytes"])
        fam.last_accessed = max(fam.last_accessed, e.get("last_accessed_at") or "")
        fam.keys.append(e["key"])
    return families


def gb(n: int) -> str:
    return f"{n / 10**9:.2f}"


def report(families: dict[str, Family], current_toolchain: str) -> tuple[list[str], list[str]]:
    """The summary's markdown lines, and the warnings."""
    total = sum(f.size for f in families.values())
    warnings = []
    lines = [
        "## Actions cache budget",
        "",
        f"{gb(total)} of {gb(QUOTA_BYTES)} GB in {sum(f.entries for f in families.values())} entries.",
        "",
        "| Family | Priority | Entries | GB | Last accessed |",
        "|---|---|---|---|---|",
    ]
    for fam in sorted(families.values(), key=lambda f: f.size, reverse=True):
        rank = PRIORITY_FAMILIES.index(fam.name) + 1 if fam.name in PRIORITY_FAMILIES else ""
        lines.append(f"| `{fam.name}` | {rank} | {fam.entries} | {gb(fam.size)} | {fam.last_accessed[:16]} |")
    missing = [name for name in PRIORITY_FAMILIES if name not in families]
    if missing:
        lines += ["", "Missing: " + ", ".join(f"`{m}`" for m in missing)]
        for name in missing:
            warnings.append(
                f"cache family {name} has no entry: every job that restores it compiles cold until its saver runs"
            )
    if total > QUOTA_BYTES:
        warnings.append(
            f"Actions cache holds {gb(total)} GB, over the {gb(QUOTA_BYTES)} GB quota: GitHub is evicting the least recently used entries"
        )
    for fam in families.values():
        for key in fam.keys:
            m = TOOLCHAIN_IN_KEY.search(key)
            if m and m.group(1) != current_toolchain and re.match(r"\d+\.\d+\.\d+$", current_toolchain):
                warnings.append(
                    f"cache entry {key} is for Rust {m.group(1)}, not {current_toolchain}: nothing restores it usefully; delete it"
                )
    return lines, warnings


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", "dravr-ai/dravr-platform"))
    parser.add_argument("--from-json", type=Path, help="read entries (one JSON object per line) instead of the API")
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args(argv)
    try:
        entries = read_entries(args.repo, args.from_json)
        current = toolchain(args.repo_root)
    except (BudgetError, OSError, json.JSONDecodeError) as exc:
        print(f"::error::cache-budget: {exc}")
        return 1
    if not entries:
        print("::error::cache-budget: the listing returned no cache entry; the report would verify nothing")
        return 2
    lines, warnings = report(summarise(entries), current)
    text = "\n".join(lines) + "\n"
    print(text)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as fh:
            fh.write(text)
    for w in warnings:
        print(f"::warning title=Actions cache budget::{w}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
