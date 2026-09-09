#!/usr/bin/env bash
# ABOUTME: Fails when a tool rejects on a parameter name its own module never declares
# ABOUTME: Compile-free; catches a renamed schema key that left its error message behind
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A tool's rejection message reaches the model verbatim, so a message naming a
# parameter the schema does not have is an instruction to retry with a key that
# will never be accepted. Two ways that happens:
#
#   1. The handler hard-requires a parameter the schema never declares. Every
#      schema-following caller then fails on every call — detect_patterns did
#      exactly this, requiring `pattern_type` while declaring nothing.
#   2. A rename moves the schema key and the handler's `.get()` with it, and
#      leaves the message behind. After the coach -> agent rename, twelve sites
#      read `agent_id` and told the caller to supply `coach_id`.
#
# Neither is visible to the type system, and neither reds a test: the handler
# compiles, and a test that passes the right key never sees the message.
#
# Scope is the module, not the file: a tool's schema usually lives in `mod.rs`
# while its handler lives in `inner.rs`, so a per-file scan reports the whole
# recipes module as broken. Grouping by module is what makes this quiet enough
# to gate on.

set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; NC='\033[0m'
ROOT="${1:-crates/pierre-tool-runtime/src/implementations}"

if [[ ! -d "$ROOT" ]]; then
    echo -e "${RED}❌ tool implementation root not found: $ROOT${NC}"
    exit 1
fi

# Exit status is judged on its own, and the pass is a success marker rather
# than the absence of a finding — a crashed scanner prints neither.
if ! OUT="$(python3 - "$ROOT" <<'PYCHECK'
import re, sys, pathlib

root = pathlib.Path(sys.argv[1])

# How a handler says "you did not give me X".
REJECT = [
    re.compile(r'Missing required parameter:\s*([A-Za-z_][A-Za-z0-9_]*)'),
    re.compile(r'missing_parameter\(\s*[^,]+,\s*"([A-Za-z_][A-Za-z0-9_]*)"'),
    re.compile(r'require_string_field\(\s*&?args\s*,\s*"([A-Za-z_][A-Za-z0-9_]*)"'),
]

# A module is a directory under the root, or a top-level .rs file.
groups = {}
for f in sorted(root.rglob('*.rs')):
    rel = f.relative_to(root)
    key = rel.parts[0] if len(rel.parts) > 1 else rel.name
    groups.setdefault(key, []).append(f)

if not groups:
    print(f"scan resolved no Rust files under {root} — refusing to pass")
    sys.exit(1)

scanned = 0
findings = []
for key, files in sorted(groups.items()):
    rejected, declared = {}, set()
    for f in files:
        src = f.read_text(encoding='utf-8')
        scanned += 1
        for rx in REJECT:
            for m in rx.finditer(src):
                rejected.setdefault(m.group(1), []).append(
                    f"{f}:{src[:m.start()].count(chr(10)) + 1}")
        # Anything named in a `required` vec, or declared as a property.
        for blob in re.findall(r'Some\(vec!\[(.*?)\]\)', src, re.S):
            declared |= set(re.findall(r'"([A-Za-z_][A-Za-z0-9_]*)"', blob))
        declared |= set(re.findall(
            r'properties\.insert\(\s*"([A-Za-z_][A-Za-z0-9_]*)"', src))
        declared |= set(re.findall(
            r'\(\s*"([A-Za-z_][A-Za-z0-9_]*)"\.to_owned\(\)\s*,\s*PropertySchema', src))
    for param, sites in sorted(rejected.items()):
        if param not in declared:
            findings.append((key, param, sites))

if scanned == 0:
    print("scan read zero files — refusing to pass")
    sys.exit(1)

if findings:
    print(f"Rejection names a parameter its module never declares "
          f"({len(findings)} across {scanned} files):\n")
    for key, param, sites in findings:
        print(f"  module {key}: rejects on '{param}', which no schema there declares")
        for s in sites:
            print(f"      {s}")
    print("\n  Either declare the parameter in the tool's schema, or correct the")
    print("  message to name the key the handler actually reads.")
    sys.exit(1)

print(f"PARAMETER_SCAN_OK {scanned} files")
PYCHECK
)"; then
    echo -e "${RED}❌ Tool rejection messages name undeclared parameters.${NC}"
    echo "$OUT"
    exit 1
fi

# A zero exit is not enough: the scanner must say it actually scanned.
if ! grep -q '^PARAMETER_SCAN_OK [1-9]' <<<"$OUT"; then
    echo -e "${RED}❌ Parameter scan produced no success marker — it verified nothing.${NC}"
    echo "$OUT"
    exit 1
fi

echo -e "${GREEN}✅ Every tool rejection names a parameter its module declares.${NC} ($(grep -c . <<<"$OUT") line, $(awk '/^PARAMETER_SCAN_OK/{print $2}' <<<"$OUT") files scanned)"
