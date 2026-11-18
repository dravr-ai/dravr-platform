#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: AccountCaller::host_cli, the caller with every power and no admin session, is built only by pierre-cli
# ABOUTME: Any other production crate naming it fails; tests and trailing test modules are not production code
#
# `AccountCaller::host_cli` is `pierre-cli` writing the database on the host:
# it passes the rank rule on any account and is written against no admin's
# session (carnet#1589). It stands for whoever holds the host shell, so a
# server route, a Slack handler or a scheduled job that built one would hand
# every caller of that surface every power. This gate keeps it to the CLI:
# production code under crates/*/src that names it outside
# crates/pierre-cli/src fails. Its definition in pierre-services is the one
# allowed mention there. Integration tests (crates/*/tests) are not scanned,
# and a file's trailing #[cfg(test)] module is not production code (ADR-027),
# dropped by prod-src-scan.sh.
#
# Usage: check-host-cli-caller.sh [ROOT]   (ROOT defaults to the repo root)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${1:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
SCAN="${PROD_SRC_SCAN:-$SCRIPT_DIR/prod-src-scan.sh}"
DEFINITION="crates/pierre-services/src/admin_ops/caller.rs"
ALLOWED="crates/pierre-cli/src/"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}==== AccountCaller::host_cli built only by pierre-cli ====${NC}"

cd "$ROOT"
shopt -s nullglob
SRC_DIRS=(crates/*/src)
if [ "${#SRC_DIRS[@]}" -eq 0 ]; then
    echo -e "${RED}❌ No crates/*/src under $ROOT: the scan would pass on nothing${NC}" >&2
    exit 2
fi
if [ ! -f "$DEFINITION" ] || ! grep -q 'pub fn host_cli(' "$DEFINITION"; then
    echo -e "${RED}❌ $DEFINITION no longer defines host_cli: the caller moved without this gate${NC}" >&2
    exit 2
fi

# A path to it (AccountCaller::host_cli, Self::host_cli, a re-export) or a
# `use` that brings the name in bare; the definition line is not a use.
PATTERN='::host_cli\b|\bhost_cli\s*[,}]|\bhost_cli\s+as\b'

# prod-src-scan.sh exits 2 on a crash; set -e propagates it, never an empty list.
hits="$(bash "$SCAN" "$PATTERN" "${SRC_DIRS[@]}")"
outside="$(printf '%s\n' "$hits" | grep -v "^$ALLOWED" | sed '/^$/d' || true)"
inside="$(printf '%s\n' "$hits" | grep -c "^$ALLOWED" || true)"

if [ -n "$outside" ]; then
    echo -e "${RED}❌ AccountCaller::host_cli named outside $ALLOWED:${NC}"
    printf '%s\n' "$outside" | sed 's/^/    /'
    echo "   host_cli is the host shell's caller: it takes every account and no admin session"
    echo "   is behind it. A server surface builds its caller from what authenticated the"
    echo "   request (AccountCaller::of_token, slack_room, sign_up_rule, account_holder)."
    exit 1
fi

# The success marker: pierre-cli builds it, so a scan that finds no site there
# matched nothing anywhere (a broken pattern, a moved CLI) and verified nothing.
if [ "$inside" -eq 0 ]; then
    echo -e "${RED}❌ The scan found no host_cli site under $ALLOWED: it verified nothing${NC}" >&2
    exit 2
fi

echo -e "${GREEN}✅ AccountCaller::host_cli is built only under $ALLOWED ($inside site(s))${NC}"
