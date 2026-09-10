#!/usr/bin/env bash
# ABOUTME: Fails when a tool rejects on a parameter name its own schema never declares
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
# Attribution comes from scripts/ci/tool_schema_properties.py, shared with
# check-contremaitre-sync.sh Check 9, which asks the opposite question: Check 9
# is "declared in the yaml overlay, not in the schema"; this is "refused for
# something never offered". The two do not overlap — measured against
# registre#420's four cases, this one would have caught detect_patterns alone,
# because it fires only where a handler REJECTS and not where one merely reads.
#
# This shipped module-scoped (carnet#418's sibling, 9874e071b) and gained
# per-tool attribution in carnet#424. Where a rejection sits in a helper file
# with no tool_definition of its own — recipes/inner.rs holds handlers whose
# schemas are in recipes/mod.rs — the tool cannot be resolved statically and the
# check falls back to that module's combined declarations. Those are reported as
# module-scoped so the weaker check is visible rather than assumed.

set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; NC='\033[0m'
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$PROJECT_ROOT"

if [[ ! -f scripts/ci/tool_schema_properties.py ]]; then
    echo -e "${RED}❌ shared attribution helper missing: scripts/ci/tool_schema_properties.py${NC}"
    exit 1
fi

# Exit status is judged on its own, and the pass is a success marker rather
# than the absence of a finding — a crashed scanner prints neither.
if ! OUT="$(python3 - <<'PYCHECK'
import sys

sys.path.insert(0, "scripts/ci")
import tool_schema_properties as tsp

try:
    tools, stats = tsp.scan()
except tsp.ScanError as exc:
    print(f"scan cannot stand behind its result: {exc}")
    raise SystemExit(1)

findings = []

# Per tool: precise. The rejection sits in the handler that follows the tool's
# own tool_definition literal, so the declarations it is checked against are
# that tool's and no other's.
for name in sorted(tools):
    entry = tools[name]
    declared = entry["properties"] | entry["required"] | tsp.UNIVERSAL
    for param in sorted(entry["rejections"]):
        if param not in declared:
            findings.append(
                (f"tool {name}", param, entry["rejections"][param], "")
            )

# Per module: the fallback, for rejections in a helper file that names no tool.
by_module = {}
for name, entry in tools.items():
    bucket = by_module.setdefault(entry["module"], set())
    bucket |= entry["properties"] | entry["required"]

for module, params in sorted(stats["orphan_rejections"].items()):
    declared = by_module.get(module, set()) | tsp.UNIVERSAL
    for param in sorted(params):
        if param not in declared:
            findings.append(
                (f"module {module}", param, params[param], " (module-scoped)")
            )

if findings:
    print(f"Rejection names a parameter that is never declared "
          f"({len(findings)} across {stats['tools']} tools):\n")
    for owner, param, sites, note in findings:
        print(f"  {owner}: rejects on '{param}', which its schema does not declare{note}")
        for s in sites:
            print(f"      {s}")
    print("\n  Either declare the parameter in the tool's schema, or correct the")
    print("  message to name the key the handler actually reads.")
    raise SystemExit(1)

print(f"PARAMETER_SCAN_OK {stats['tools']} tools {stats['files']} files")
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

echo -e "${GREEN}✅ Every tool rejection names a parameter its schema declares.${NC} (${OUT#PARAMETER_SCAN_OK })"
