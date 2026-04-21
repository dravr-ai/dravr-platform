#!/usr/bin/env bash
# ABOUTME: Forbids mid-chain ConversationTurnId regeneration so per-turn observability stays intact
# ABOUTME: A turn id is generated at the inbound boundary and propagated; downstream sites must reuse it
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A conversation turn is threaded from the inbound boundary (webhook,
# chat route, messaging ingress, MCP tool handler) through every
# downstream call that participates in the turn. If any site along the
# chain generates a fresh `Uuid::new_v4()` into a turn_id / ConversationTurnId
# field, per-turn queries to `GET /internal/conversation-turn/{id}` will
# return partial traces.
#
# This gate scans the pierre-server sources for forbidden patterns:
#   - `turn_id: Uuid::new_v4()`  -> always wrong
#   - `turn_id: ConversationTurnId::new()` inside callers that already
#     receive a turn id (identified by presence of a DispatchResult /
#     TurnInput / PendingDispatch parameter on the same function).
#
# Run it from the repository root. Exits non-zero on the first match.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

STATUS=0

# Permitted boundary files — these generate the initial turn id.
# Keep this list minimal; adding a path is the same as claiming a new
# inbound boundary exists, which always deserves code review.
ALLOWED_GENERATORS=(
    "crates/pierre-server/src/routes/chat.rs"
    "crates/pierre-server/src/services/messaging_ingress.rs"
    "crates/pierre-server/src/mcp/tool_handlers.rs"
)

# 1. `turn_id:` assigned directly from `Uuid::new_v4()` is never correct.
#    Even at the boundary we go through `ConversationTurnId::new()` so
#    the type stays opaque.
MATCHES=$(grep -rn --include='*.rs' \
    -E 'turn_id:\s*Uuid::new_v4' \
    crates/ 2>/dev/null || true)
if [[ -n "$MATCHES" ]]; then
    echo "FAIL: direct Uuid::new_v4() used as turn_id. Use ConversationTurnId::new()."
    echo "$MATCHES"
    STATUS=1
fi

# 2. `ConversationTurnId::new()` outside allowed boundary files.
#    Tests may generate freely; production sites must be in the
#    boundary allow-list so adding one requires a reviewed diff here.
while IFS=: read -r path line rest; do
    # Accept docstring / comment hits (# or //) silently
    if [[ "$rest" =~ ^[[:space:]]*(//|#|\*) ]]; then
        continue
    fi
    allowed=0
    for ok in "${ALLOWED_GENERATORS[@]}"; do
        if [[ "$path" == "$ok" ]]; then
            allowed=1
            break
        fi
    done
    if [[ "$allowed" -eq 0 ]]; then
        echo "FAIL: $path:$line generates a ConversationTurnId outside an allowed boundary."
        echo "      $rest"
        echo "      If this is a new inbound boundary, add the path to ALLOWED_GENERATORS"
        echo "      in scripts/ci/check-conversation-turn-id.sh."
        STATUS=1
    fi
done < <(grep -rn --include='*.rs' \
    -E '\bConversationTurnId::new\(\)' \
    crates/ 2>/dev/null | grep -v '/tests/' || true)

# 3. `CanotTurnId::new()` — same policy: only allowed in the messaging
#    ingress / outbound reply handlers, which sit at the messaging
#    boundary.
while IFS=: read -r path line rest; do
    if [[ "$rest" =~ ^[[:space:]]*(//|#|\*) ]]; then
        continue
    fi
    allowed=0
    for ok in "${ALLOWED_GENERATORS[@]}"; do
        if [[ "$path" == "$ok" ]]; then
            allowed=1
            break
        fi
    done
    if [[ "$allowed" -eq 0 ]]; then
        echo "FAIL: $path:$line generates a CanotTurnId outside an allowed boundary."
        echo "      $rest"
        STATUS=1
    fi
done < <(grep -rn --include='*.rs' \
    -E '\bCanotTurnId::new\(\)' \
    crates/ 2>/dev/null | grep -v '/tests/' || true)

if [[ "$STATUS" -eq 0 ]]; then
    echo "OK: no mid-chain turn id regeneration detected."
fi

exit "$STATUS"
