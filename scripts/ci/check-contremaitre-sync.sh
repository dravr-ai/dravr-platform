#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Compile-free static check that platform code stays in sync with the
# ABOUTME: dravr-contremaitre catalogues (locales, notify events, MCP tool list).

# WHY THIS EXISTS
# ---------------
# The integration tests that police platform<->contremaitre coupling
# (contremaitre_test, notify_catalogue_test, messaging_locale_test) live in
# pierre-server/tests. They are also wired into the per-push `contremaitre-sync`
# job in ci-backend.yml, but that job costs a Rust compile; this script is the
# grep-only pre-push tier that catches the same drift in seconds, before the
# push, with no compile at all.
#
# Covered:
#   1. Locale invariant — every messaging-string key ships all 5 locales
#      (fr/en/es/de/pt). Mirrors contremaitre_test
#      test_messaging_registry_seeds_all_compiled_locales (entry == keys * 5).
#   2. Notify catalogue — every info!(target:"notify", event="X") emitted in
#      platform src exists as a key in contremaitre's notify-events.yaml.
#      Mirrors notify_catalogue_test.
#   3. MCP tool list — the tool names in src match all three of their mirrors:
#      EXPECTED_TOOLS (contremaitre_test), the hardcoded count assertion
#      (configuration_mcp_integration_test), and the generated TS SDK types
#      (packages/mcp-types/src/tools.ts, whose staleness reds CI: TypeScript SDK).
#
# Known blind spot: an event or tool whose name is built at runtime rather than
# written as a literal is invisible to a static scan. Check 3 defends against
# that by asserting its own completeness (one extracted name per call site) and
# failing loudly if the assumption ever breaks. See AGENTS.md.

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"
cd "$PROJECT_ROOT"

echo -e "${BLUE}==== Contremaitre Coupling Sync (static) ====${NC}"

FAILED=false

# ---------------------------------------------------------------------------
# Check 1: messaging-string locale invariant (5 locales per key)
# ---------------------------------------------------------------------------
MS="crates/pierre-contremaitre/src/messaging_strings.rs"

if [[ ! -f "$MS" ]]; then
    echo -e "${RED}❌ messaging_strings.rs not found at $MS — this check is stale.${NC}"
    FAILED=true
else
    # Extract the COMPILED_IN (KEY, "locale", VALUE) table, collapse multi-line
    # tuples by joining on spaces, then pull (KEY, locale) pairs. The only quoted
    # literals inside the table are the locale codes, so this is unambiguous.
    BLOCK="$(awk '/const COMPILED_IN/{f=1} f{print} f&&/^\];/{exit}' "$MS")"
    PAIRS="$(printf '%s' "$BLOCK" | tr '\n' ' ' \
        | grep -oE '\(\s*KEY_[A-Z0-9_]+\s*,\s*"(fr|en|es|de|pt)"' \
        | sed -E 's/.*(KEY_[A-Z0-9_]+)[^"]*"(fr|en|es|de|pt)".*/\1 \2/' || true)"

    PAIR_COUNT="$(printf '%s\n' "$PAIRS" | grep -c . || true)"
    if [[ "$PAIR_COUNT" -eq 0 ]]; then
        echo -e "${RED}❌ Could not parse the COMPILED_IN locale table — structure changed; update this check.${NC}"
        FAILED=true
    else
        # Per-key locale audit: any key without exactly the 5 locales is drift.
        MISSING="$(printf '%s\n' "$PAIRS" | awk '
            { have[$1] = have[$1] " " $2 }
            END {
                split("fr en es de pt", want, " ")
                for (k in have)
                    for (i = 1; i <= 5; i++)
                        if (index(have[k], " " want[i]) == 0)
                            print k " missing:" want[i]
            }' | sort)"
        if [[ -n "$MISSING" ]]; then
            KEYS_AFFECTED="$(printf '%s\n' "$MISSING" | awk '{print $1}' | sort -u | grep -c .)"
            echo -e "${RED}❌ Messaging locale drift: ${KEYS_AFFECTED} key(s) do not ship all 5 locales (fr/en/es/de/pt):${NC}"
            printf '%s\n' "$MISSING" | sed 's/^/   /'
            echo -e "${YELLOW}   Add the missing locale const(s) in ${MS} so every key has fr/en/es/de/pt.${NC}"
            echo -e "${YELLOW}   (This is the recurrence behind commits d73eec36f and a60209307.)${NC}"
            FAILED=true
        else
            KEYS="$(printf '%s\n' "$PAIRS" | awk '{print $1}' | sort -u | grep -c . || true)"
            echo -e "${GREEN}✅ Locale invariant: ${KEYS} keys × 5 locales = ${PAIR_COUNT} entries, all complete.${NC}"
        fi
    fi
fi

# ---------------------------------------------------------------------------
# Check 2: notify events emitted in src must be catalogued in notify-events.yaml
# ---------------------------------------------------------------------------
# Resolve the catalogue from the pinned dravr-contremaitre rev (Cargo.lock) via
# cargo metadata; fall back to a sibling checkout; warn-skip if unresolvable so
# an offline machine is not blocked from pushing.
YAML=""
MANIFEST="$(cargo metadata --format-version 1 2>/dev/null \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); print(next((p["manifest_path"] for p in d["packages"] if p["name"]=="dravr-contremaitre"), ""))' 2>/dev/null || true)"
if [[ -n "$MANIFEST" && -f "${MANIFEST%Cargo.toml}schemas/notify-events.yaml" ]]; then
    YAML="${MANIFEST%Cargo.toml}schemas/notify-events.yaml"
elif [[ -f "../dravr-contremaitre/schemas/notify-events.yaml" ]]; then
    YAML="../dravr-contremaitre/schemas/notify-events.yaml"
fi

if [[ -z "$YAML" ]]; then
    echo -e "${YELLOW}⚠️  notify-events.yaml could not be resolved (offline?) — skipping the notify check.${NC}"
else
    CATALOGUED="$(grep -E '^\s*- name:' "$YAML" | sed -E 's/.*- name:[[:space:]]*//' | tr -d ' "' | sort -u)"
    # Emitted events: each .rs under crates/*/src (not tests) that targets
    # "notify"; grab event = "literal" within a window around the target line.
    # Dynamically-built (non-literal) event names are a known blind spot.
    EMITTED="$(grep -rl 'target: "notify"' crates/*/src 2>/dev/null | grep -v '/tests/' \
        | while IFS= read -r f; do
            grep -B2 -A10 'target: "notify"' "$f" \
                | grep -oE 'event[[:space:]]*=[[:space:]]*"[^"]+"' \
                | sed -E 's/.*"([^"]+)".*/\1/'
          done | sort -u || true)"
    DRIFT="$(comm -23 <(printf '%s\n' "$EMITTED" | grep -v '^$') <(printf '%s\n' "$CATALOGUED" | grep -v '^$') || true)"
    if [[ -n "$DRIFT" ]]; then
        echo -e "${RED}❌ Notify catalogue drift: event(s) emitted in src but absent from notify-events.yaml:${NC}"
        printf '%s\n' "$DRIFT" | sed 's/^/   /'
        echo -e "${YELLOW}   Register the event in dravr-contremaitre notify-events.yaml (or reuse an existing one),${NC}"
        echo -e "${YELLOW}   then bump the contremaitre rev — else notify_catalogue_test reds main post-merge.${NC}"
        FAILED=true
    else
        EMIT_COUNT="$(printf '%s\n' "$EMITTED" | grep -c . || true)"
        echo -e "${GREEN}✅ Notify catalogue: all ${EMIT_COUNT} emitted events are catalogued.${NC}"
    fi
fi

# ---------------------------------------------------------------------------
# Check 3: MCP tool list vs its three mirrors
# ---------------------------------------------------------------------------
# Every McpTool::definition() builds its Tool through
# tool_definition("<name>", ...) (pierre-tool-runtime/src/conversions.rs), so the
# names ARE literals at a greppable call site and the full set is enumerable
# without compiling. The scan proves its own completeness before comparing.
EXPECTED_FILE="crates/pierre-server/tests/contremaitre_test.rs"
COUNT_FILE="crates/pierre-server/tests/configuration_mcp_integration_test.rs"
TS_FILE="packages/mcp-types/src/tools.ts"

TOOL_FILES="$(grep -rl 'tool_definition(' crates --include='*.rs' 2>/dev/null | grep '/src/' || true)"

if [[ -z "$TOOL_FILES" ]]; then
    echo -e "${RED}❌ No tool_definition( call sites found in crates/*/src — this check is stale.${NC}"
    FAILED=true
else
    # Join lines: a call site may wrap between the paren and the name literal.
    BLOB="$(printf '%s\n' "$TOOL_FILES" | xargs cat 2>/dev/null | tr '\n' ' ')"
    CALLS="$(printf '%s' "$BLOB" | grep -oE 'tool_definition\(' | grep -c . || true)"
    HELPER="$(printf '%s' "$BLOB" | grep -oE 'fn[[:space:]]+tool_definition\(' | grep -c . || true)"
    CALL_SITES=$(( CALLS - HELPER ))
    SRC_TOOLS="$(printf '%s' "$BLOB" \
        | grep -oE 'tool_definition\([[:space:]]*"[a-z0-9_]+"' \
        | grep -oE '"[a-z0-9_]+"' | tr -d '"' | sort -u || true)"
    SRC_COUNT="$(printf '%s\n' "$SRC_TOOLS" | grep -c . || true)"

    if [[ "$SRC_COUNT" -ne "$CALL_SITES" ]]; then
        # The scan's own assumption broke. Fail loudly rather than compare a
        # silently-truncated set, which would read as "all tools in sync".
        echo -e "${RED}❌ Tool scan incomplete: ${CALL_SITES} tool_definition( call site(s) but ${SRC_COUNT} literal name(s).${NC}"
        echo -e "${YELLOW}   A tool is registered with a non-literal name, so this static scan can no longer see${NC}"
        echo -e "${YELLOW}   every tool. Give the tool a literal name, or extend this check — never ignore it.${NC}"
        FAILED=true
    else
        TOOL_DRIFT=false

        # 3a. EXPECTED_TOOLS in contremaitre_test.rs
        EXPECTED_LIST="$(awk '/EXPECTED_TOOLS/{f=1} f{print} f&&/^\];/{exit}' "$EXPECTED_FILE" 2>/dev/null \
            | grep -oE '"[a-z0-9_]+"' | tr -d '"' | sort -u || true)"
        if [[ -z "$EXPECTED_LIST" ]]; then
            echo -e "${RED}❌ Could not parse EXPECTED_TOOLS in ${EXPECTED_FILE} — structure changed; update this check.${NC}"
            TOOL_DRIFT=true
        else
            UNLISTED="$(comm -23 <(printf '%s\n' "$SRC_TOOLS") <(printf '%s\n' "$EXPECTED_LIST") || true)"
            PHANTOM="$(comm -13 <(printf '%s\n' "$SRC_TOOLS") <(printf '%s\n' "$EXPECTED_LIST") || true)"
            if [[ -n "$UNLISTED" ]]; then
                echo -e "${RED}❌ Tool drift: registered in src but absent from EXPECTED_TOOLS:${NC}"
                printf '%s\n' "$UNLISTED" | sed 's/^/   /'
                TOOL_DRIFT=true
            fi
            if [[ -n "$PHANTOM" ]]; then
                echo -e "${RED}❌ Tool drift: listed in EXPECTED_TOOLS but no longer registered in src:${NC}"
                printf '%s\n' "$PHANTOM" | sed 's/^/   /'
                TOOL_DRIFT=true
            fi
        fi

        # 3b. hardcoded total in configuration_mcp_integration_test.rs
        ASSERTED="$(grep -oE 'assert_eq!\([[:space:]]*tools\.len\(\),[[:space:]]*[0-9]+' "$COUNT_FILE" 2>/dev/null \
            | grep -oE '[0-9]+$' | sort -u || true)"
        ASSERT_LINES="$(printf '%s\n' "$ASSERTED" | grep -c . || true)"
        if [[ "$ASSERT_LINES" -ne 1 ]]; then
            echo -e "${RED}❌ Expected exactly one tools.len() assertion in ${COUNT_FILE}, found ${ASSERT_LINES} distinct value(s) — update this check.${NC}"
            TOOL_DRIFT=true
        elif [[ "$ASSERTED" -ne "$SRC_COUNT" ]]; then
            echo -e "${RED}❌ Tool count drift: src registers ${SRC_COUNT} tools, ${COUNT_FILE} asserts ${ASSERTED}.${NC}"
            TOOL_DRIFT=true
        fi

        # 3c. generated TS SDK types — stale types red CI: TypeScript SDK
        if [[ ! -f "$TS_FILE" ]]; then
            echo -e "${RED}❌ ${TS_FILE} not found — this check is stale.${NC}"
            TOOL_DRIFT=true
        else
            TS_TOOLS="$(grep -oE '"[a-z0-9_]+"' "$TS_FILE" | tr -d '"' | sort -u || true)"
            TS_HEADER="$(grep -oE '^// Tool count: [0-9]+' "$TS_FILE" | grep -oE '[0-9]+' || true)"
            TS_DIFF="$(comm -3 <(printf '%s\n' "$SRC_TOOLS") <(printf '%s\n' "$TS_TOOLS") || true)"
            if [[ -n "$TS_DIFF" || "$TS_HEADER" != "$SRC_COUNT" ]]; then
                echo -e "${RED}❌ SDK type drift: ${TS_FILE} does not match the ${SRC_COUNT} tools in src (header says '${TS_HEADER}').${NC}"
                [[ -n "$TS_DIFF" ]] && printf '%s\n' "$TS_DIFF" | sed 's/^/   /'
                echo -e "${YELLOW}   Regenerate against a running server: cd packages/mcp-types && bun run generate${NC}"
                TOOL_DRIFT=true
            fi
        fi

        if [[ "$TOOL_DRIFT" == "true" ]]; then
            echo -e "${YELLOW}   A new McpTool must land with EXPECTED_TOOLS (kept sorted), the count assertion,${NC}"
            echo -e "${YELLOW}   and regenerated SDK types in the SAME change — else main reds post-merge.${NC}"
            FAILED=true
        else
            echo -e "${GREEN}✅ MCP tool list: ${SRC_COUNT} tools match EXPECTED_TOOLS, the count assertion, and the SDK types.${NC}"
        fi
    fi
fi

# ---------------------------------------------------------------------------
# Check 4: the shipped contremaitre catalogues carry no retired ACWR framing
# ---------------------------------------------------------------------------
# Tool descriptions and persona prompts reach the model through the runtime
# sync, not through include_str!, so no Rust test can see them: a test that
# reads registry_builtin::get_tools() inspects the compiled-in fallback that
# ToolRegistry::build_schema overwrites. This is the only gate that reads what
# actually ships. It bans the framings, not the framework name — "ACWR →
# Gabbett" as an attribution is fine, and the coach prompt legitimately explains
# what "the retired injury-prediction use" was.
CM_ROOT=""
if [[ -n "$MANIFEST" && -d "${MANIFEST%Cargo.toml}tools" ]]; then
    CM_ROOT="${MANIFEST%Cargo.toml}"
elif [[ -d "../dravr-contremaitre/tools" ]]; then
    CM_ROOT="../dravr-contremaitre/"
fi

if [[ -z "$CM_ROOT" ]]; then
    echo -e "${YELLOW}⚠️  contremaitre corpus could not be resolved (offline?) — skipping the framing check.${NC}"
else
    # Couple the injury token to a load token before flagging: a strength coach
    # citing Lauersen on resistance training reducing injury risk, and a mobility
    # coach on warm-ups, are unrelated to the ratio. Lines that *forbid* the
    # framing ("never present it as an injury risk") are excluded too.
    FRAMING_HITS="$(python3 - "$CM_ROOT" <<'PYCHECK'
import pathlib, re, sys
root = pathlib.Path(sys.argv[1])
INJURY = re.compile(r"injury risk|injury probabilit|risque de blessure|probabilit\w* de blessure", re.I)
LOAD = re.compile(r"acwr|load spike|load increase|acute:chronic|charge aigu|pic de charge|hausses soudaines", re.I)
NEGATED = re.compile(r"never|not present|jamais|retired|retir\u00e9|\bpas\b", re.I)
GREEN = re.compile(r"green band|bande verte", re.I)
ABS_TSB = re.compile(r"TSB\s*[<>]\s*[-\u2212+]?\d", re.I)
hits = []
for sub in ("tools", "prompts/personas", "prompts/coaches"):
    d = root / sub
    if not d.is_dir():
        continue
    for f in sorted(d.rglob("*")):
        if f.suffix not in (".md", ".yaml", ".yml"):
            continue
        for n, line in enumerate(f.read_text(encoding="utf-8").splitlines(), 1):
            why = None
            if INJURY.search(line) and LOAD.search(line) and not NEGATED.search(line):
                why = "ACWR presented as injury risk"
            elif GREEN.search(line):
                why = "red/green safety verdict"
            elif ABS_TSB.search(line):
                why = "absolute TSB band (use % of CTL)"
            if why:
                hits.append(f"{f}:{n}: [{why}] {line.strip()[:160]}")
print("\n".join(hits))
PYCHECK
)"

    if [[ -n "$FRAMING_HITS" ]]; then
        echo -e "${RED}❌ Retired ACWR/TSB framing in the shipped contremaitre catalogues:${NC}"
        echo -e "$FRAMING_HITS"
        echo -e "${YELLOW}   Present ACWR as magnitude against the 28-day baseline, never injury risk or a${NC}"
        echo -e "${YELLOW}   red/green verdict, and band TSB as a share of CTL (registre#26).${NC}"
        FAILED=true
    else
        echo -e "${GREEN}✅ Contremaitre catalogues carry no retired ACWR/TSB framing.${NC}"
    fi
fi

if [[ "$FAILED" == "true" ]]; then
    echo -e "${RED}❌ CONTREMAITRE COUPLING CHECK FAILED${NC}"
    echo -e "${RED}Fix the drift above before pushing — these reds otherwise land on main post-merge.${NC}"
    exit 1
fi
echo -e "${GREEN}✅ Contremaitre coupling in sync.${NC}"
exit 0
