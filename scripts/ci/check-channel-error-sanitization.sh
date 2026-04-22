#!/usr/bin/env bash
# ABOUTME: Forbids raw AppError interpolation into messaging channel replies
# ABOUTME: Every error-to-channel-reply site must route through ChannelErrorReply::to_channel_reply
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# REST responses go through `impl IntoResponse for AppError` which is a
# single type-enforced choke point and already calls `sanitized_message()`.
# Messaging channels (Telegram, Slack, Discord, WhatsApp, Messenger) don't
# have that shape — a reply is a `String` inside `MessageContent::Text`, so
# a handler can write `format!("DB: {e}")` and leak raw schema detail to
# the end user.
#
# The `ChannelErrorReply` trait on `AppError` in
# `crates/pierre-server/src/services/channel_error_reply.rs` is the single
# approved funnel. This gate forbids the two patterns that would bypass it:
#
#   1. `format!("...{e}..."`  or  `format!("...{err}..."`  appearing within
#      20 lines of a `MessageContent::Text { body:` literal in the same file.
#
#   2. The string `Command failed:` appearing anywhere without a neighboring
#      `to_channel_reply` call — that literal is the prefix the original
#      leak (DRAVR ticket, 2026-04-22) used, so we pin it as a canary.
#
# Run from repo root. Exits non-zero on the first match.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

STATUS=0

# Files exempt from the gate — for example the sanitizer itself.
ALLOWED=(
    "crates/pierre-server/src/services/channel_error_reply.rs"
)

is_allowed() {
    local path="$1"
    for ok in "${ALLOWED[@]}"; do
        if [[ "$path" == "$ok" ]]; then
            return 0
        fi
    done
    return 1
}

# Pattern 1: `format!("...{e}...")` near `MessageContent::Text`. Use ripgrep
# with multiline mode to catch constructions that span several lines, which
# is the common shape when the body is built in-place.
if command -v rg >/dev/null 2>&1; then
    MATCHES=$(rg --multiline --no-heading -n \
        -g 'crates/pierre-server/src/**/*.rs' \
        -e 'MessageContent::Text\s*\{\s*body:\s*format!\([^)]*\{e[!?]?\}' \
        -e 'MessageContent::Text\s*\{\s*body:\s*format!\([^)]*\{err[!?]?\}' \
        2>/dev/null || true)
    while IFS= read -r line; do
        [[ -z "$line" ]] && continue
        path=$(echo "$line" | cut -d: -f1)
        if is_allowed "$path"; then continue; fi
        echo "FAIL: raw error interpolation into MessageContent::Text at $line"
        echo "      Route the error through ChannelErrorReply::to_channel_reply instead."
        STATUS=1
    done <<< "$MATCHES"
fi

# Pattern 2: the literal "Command failed:" outside the sanitizer. This
# catches the original leak shape even when the error variable isn't named
# `e` or `err`.
MATCHES=$(grep -rn --include='*.rs' \
    -F 'Command failed:' \
    crates/pierre-server/src/ 2>/dev/null || true)
while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    path=$(echo "$line" | cut -d: -f1)
    if is_allowed "$path"; then continue; fi
    # Comment-only lines are fine.
    rest=$(echo "$line" | cut -d: -f3-)
    if [[ "$rest" =~ ^[[:space:]]*(//|#|\*) ]]; then
        continue
    fi
    echo "FAIL: literal 'Command failed:' at $line"
    echo "      Use ChannelErrorReply::to_channel_reply — the generic message"
    echo "      + correlation id it returns is locale-aware."
    STATUS=1
done <<< "$MATCHES"

if [[ "$STATUS" -eq 0 ]]; then
    echo "OK: no raw error leaks into channel replies detected."
fi

exit "$STATUS"
