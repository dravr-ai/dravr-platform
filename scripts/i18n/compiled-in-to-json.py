#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# ABOUTME: Moves messaging_strings.rs COMPILED_IN tuples into the five nested translation.json catalogue files
# ABOUTME: Run once per branch that still carries Rust string consts; the JSON files are the catalogue afterwards
"""
Usage:
    scripts/i18n/compiled-in-to-json.py [--source PATH] [--locales-dir DIR] [--dry-run]

Reads every `const XX_NAME: &str = "..."` locale constant and the
`COMPILED_IN` `(KEY_*, "locale", XX_*)` table from `messaging_strings.rs`,
resolves each tuple to `(dotted key, locale, text)`, and inserts the text at
the dotted path of `<locales-dir>/<locale>/translation.json`.

The script refuses to overwrite an existing leaf with a different value and
refuses to turn a leaf into a subtree (or the reverse) — both are reported and
the run exits non-zero without writing anything.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

LOCALES = ("fr", "en", "es", "de", "pt")

KEY_CONST = re.compile(r'^pub const (KEY_[A-Z0-9_]+): &str = "([^"]+)";', re.M)
LOCALE_CONST = re.compile(
    r'^(?:pub(?:\(crate\))? )?const ((?:FR|EN|ES|DE|PT)_[A-Z0-9_]+): &str =\s*"((?:[^"\\]|\\.)*)"\s*;',
    re.M | re.S,
)
# rustfmt leaves a trailing comma inside a tuple it had to break across lines.
TUPLE = re.compile(r'\(\s*(KEY_[A-Z0-9_]+)\s*,\s*"(fr|en|es|de|pt)"\s*,\s*((?:FR|EN|ES|DE|PT)_[A-Z0-9_]+)\s*,?\s*\)')
UNICODE_ESCAPE = re.compile(r"\\u\{([0-9a-fA-F]{1,6})\}")
SIMPLE_ESCAPES = {"n": "\n", "t": "\t", "r": "\r", '"': '"', "'": "'", "\\": "\\", "0": "\0"}


def unescape(literal: str) -> str:
    """Decode the subset of Rust string escapes the catalogue uses."""
    out: list[str] = []
    i = 0
    while i < len(literal):
        ch = literal[i]
        if ch != "\\":
            out.append(ch)
            i += 1
            continue
        nxt = literal[i + 1]
        if nxt == "u":
            m = UNICODE_ESCAPE.match(literal, i)
            if m is None:
                raise ValueError(f"malformed unicode escape at {i}: {literal[i:i+12]!r}")
            out.append(chr(int(m.group(1), 16)))
            i = m.end()
            continue
        if nxt == "\n":
            # Rust line continuation: drop the newline and the leading whitespace.
            i += 2
            while i < len(literal) and literal[i] in " \t":
                i += 1
            continue
        if nxt not in SIMPLE_ESCAPES:
            raise ValueError(f"unsupported escape \\{nxt} in {literal[:60]!r}")
        out.append(SIMPLE_ESCAPES[nxt])
        i += 2
    return "".join(out)


def parse_table(source: str) -> list[tuple[str, str, str]]:
    keys = dict(KEY_CONST.findall(source))
    consts = {name: unescape(raw) for name, raw in LOCALE_CONST.findall(source)}
    start = source.find("const COMPILED_IN")
    if start < 0:
        raise SystemExit("no COMPILED_IN table in the source — nothing to convert")
    end = source.find("\n];", start)
    block = source[start:end]
    entries: list[tuple[str, str, str]] = []
    for key_const, locale, value_const in TUPLE.findall(block):
        if key_const not in keys:
            raise SystemExit(f"{key_const} is used in COMPILED_IN but has no `pub const` declaration")
        if value_const not in consts:
            raise SystemExit(f"{value_const} is used in COMPILED_IN but has no string constant")
        entries.append((keys[key_const], locale, consts[value_const]))
    return entries


def insert(tree: dict, dotted: str, value: str, conflicts: list[str], locale: str) -> None:
    parts = dotted.split(".")
    node = tree
    for part in parts[:-1]:
        child = node.get(part)
        if child is None:
            child = node[part] = {}
        elif not isinstance(child, dict):
            conflicts.append(f"{locale}: {dotted} — '{part}' is already a leaf")
            return
        node = child
    leaf = parts[-1]
    existing = node.get(leaf)
    if isinstance(existing, dict):
        conflicts.append(f"{locale}: {dotted} is already a subtree")
    elif existing is not None and existing != value:
        conflicts.append(f"{locale}: {dotted} already holds a different value")
    else:
        node[leaf] = value


def is_sorted(tree: dict) -> bool:
    return list(tree.keys()) == sorted(tree.keys()) and all(
        is_sorted(v) for v in tree.values() if isinstance(v, dict)
    )


def sort_tree(tree: dict) -> dict:
    return {k: sort_tree(v) if isinstance(v, dict) else v for k, v in sorted(tree.items())}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--source", default="crates/pierre-contremaitre/src/messaging_strings.rs")
    ap.add_argument("--locales-dir", default="packages/i18n/src/locales")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    entries = parse_table(Path(args.source).read_text(encoding="utf-8"))
    by_locale: dict[str, list[tuple[str, str]]] = {l: [] for l in LOCALES}
    for key, locale, value in entries:
        by_locale[locale].append((key, value))
    key_set = {key for key, _, _ in entries}
    print(f"parsed {len(entries)} entries, {len(key_set)} keys")
    for locale in LOCALES:
        if len(by_locale[locale]) != len(key_set):
            print(f"  {locale}: {len(by_locale[locale])} entries (expected {len(key_set)})")

    conflicts: list[str] = []
    outputs: dict[Path, str] = {}
    for locale in LOCALES:
        path = Path(args.locales_dir) / locale / "translation.json"
        raw = path.read_text(encoding="utf-8")
        tree = json.loads(raw)
        sorted_before = is_sorted(tree)
        for key, value in by_locale[locale]:
            insert(tree, key, value, conflicts, locale)
        if sorted_before:
            tree = sort_tree(tree)
        text = json.dumps(tree, ensure_ascii=False, indent=2) + "\n"
        outputs[path] = text
        added = len(by_locale[locale])
        print(f"  {locale}: +{added} entries -> {path} ({'sorted' if sorted_before else 'append order'})")

    if conflicts:
        print("refusing to write — conflicts:", file=sys.stderr)
        for c in conflicts:
            print(f"  {c}", file=sys.stderr)
        return 1
    if args.dry_run:
        print("dry run: nothing written")
        return 0
    for path, text in outputs.items():
        path.write_text(text, encoding="utf-8")
    print("written")
    return 0


if __name__ == "__main__":
    sys.exit(main())
