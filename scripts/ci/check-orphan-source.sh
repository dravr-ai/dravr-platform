#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Fails CI when a crate src/ .rs file is not reachable from any `mod <name>;` declaration
# ABOUTME: Catches vestigial modules (e.g. the unwired websocket.rs transport) that compile-out silently

# A .rs file under crates/*/src that is never named by a `mod <name>;` /
# `pub mod <name>;` / `pub(crate) mod <name>;` declaration anywhere in its
# own crate is dead weight: the compiler never sees it, so clippy and the
# test suite skip it entirely. The vestigial /ws transport (websocket.rs)
# lingered exactly this way — file present on disk, zero mod edge pointing
# at it. This gate makes that state a hard failure.
#
# Exemptions (these are crate roots / build entrypoints, not mod-declared):
#   - lib.rs, main.rs, mod.rs, build.rs
#   - anything under a bin/ directory ([[bin]] targets are declared in
#     Cargo.toml or auto-discovered by cargo, never via `mod`)

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"
cd "$PROJECT_ROOT"

echo -e "${BLUE}==== Orphan Source Detection (unreachable modules) ====${NC}"

ORPHANS=""
ORPHAN_COUNT=0

for crate in crates/*/; do
    [ -d "${crate}src" ] || continue
    while IFS= read -r f; do
        base=$(basename "$f" .rs)
        case "$base" in
            lib|main|mod|build) continue ;;
        esac
        # Is this file's stem named by a mod declaration anywhere in the crate?
        # Matches: mod foo; / pub mod foo; / pub(crate) mod foo; / pub(super) mod foo;
        if ! rg -q "\bmod[[:space:]]+${base}[[:space:]]*;" "${crate}src" 2>/dev/null; then
            ORPHANS="${ORPHANS}${f}\n"
            ORPHAN_COUNT=$((ORPHAN_COUNT + 1))
        fi
    done < <(find "${crate}src" -name "*.rs" -not -path "*/bin/*")
done

if [ "$ORPHAN_COUNT" -gt 0 ]; then
    echo -e "${RED}❌ Found $ORPHAN_COUNT orphaned source file(s) — not reachable from any 'mod <name>;':${NC}"
    echo -e "$ORPHANS" | sed '/^$/d' | sed 's/^/    /'
    echo -e "${RED}A file the compiler never sees is dead code. Wire it with a mod${NC}"
    echo -e "${RED}declaration in the crate, or delete it.${NC}"
    exit 1
fi

echo -e "${GREEN}✅ All crate src/ modules are reachable from a mod declaration${NC}"
exit 0
