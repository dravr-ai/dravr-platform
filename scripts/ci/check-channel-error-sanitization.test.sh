#!/usr/bin/env bash
# ABOUTME: Pins check-channel-error-sanitization.sh — a raw error in a channel reply fails, a clean tree passes,
# ABOUTME: with stdin a pipe as in a hook or CI, where ripgrep given no path searched stdin and passed vacuously
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-channel-error-sanitization.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

scaffold() {
    rm -rf "$WORK/probe"
    mkdir -p "$WORK/probe/crates/pierre-server/src/services"
    printf 'pub fn reply() -> MessageContent {\n    MessageContent::Text { body: e.to_channel_reply() }\n}\n' \
        > "$WORK/probe/crates/pierre-server/src/services/reply.rs"
}

# Each run feeds the gate an empty pipe on stdin, as a hook or a CI step does.
expect() {
    local label="$1" want_status="$2" needle="$3"
    OUT="$(printf '' | "$CHECK" "$WORK/probe" 2>&1)"; STATUS=$?
    if [ "$STATUS" -eq "$want_status" ] && { [ -z "$needle" ] || echo "$OUT" | grep -qF -- "$needle"; }; then
        pass "$label"
    else
        fail "$label (status $STATUS, want $want_status)"
        echo "        ${OUT//$'\n'/$'\n'        }"
    fi
}

# 1. Clean: the reply goes through the sanitizer.
scaffold
expect "a reply routed through to_channel_reply passes" 0 "OK: no raw error leaks"

# 2. Catch: a raw error formatted into a channel reply, stdin a pipe.
scaffold
printf 'pub fn leak() {\n    MessageContent::Text { body: format!("DB: {e}") };\n}\n' \
    >> "$WORK/probe/crates/pierre-server/src/services/reply.rs"
expect "a raw {e} in MessageContent::Text fails" 1 "services/reply.rs:5"

# 3. Catch: across lines, with {err}.
scaffold
printf 'pub fn leak() {\n    MessageContent::Text {\n        body: format!(\n            "failed: {err}"),\n    };\n}\n' \
    >> "$WORK/probe/crates/pierre-server/src/services/reply.rs"
expect "a multiline raw {err} fails" 1 "raw error interpolation"

# 4. Catch: the canary literal.
scaffold
printf 'pub fn leak() -> String {\n    "Command failed: boom".to_owned()\n}\n' \
    >> "$WORK/probe/crates/pierre-server/src/services/reply.rs"
expect "the 'Command failed:' literal fails" 1 "literal 'Command failed:'"

# 5. The sanitizer itself is exempt.
scaffold
printf 'pub fn s() { MessageContent::Text { body: format!("{e}") }; }\n' \
    > "$WORK/probe/crates/pierre-server/src/services/channel_error_reply.rs"
expect "the sanitizer's own file is exempt" 0 "OK: no raw error leaks"

# 6. Fail closed: no tree to scan.
rm -rf "$WORK/probe" && mkdir -p "$WORK/probe"
expect "a missing source tree fails" 2 "not found"

if [ "$FAILURES" -ne 0 ]; then
    echo "check-channel-error-sanitization.test.sh: $FAILURES failure(s)"
    exit 1
fi
echo "check-channel-error-sanitization.test.sh: all cases pass"
