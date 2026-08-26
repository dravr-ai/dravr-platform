#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Compile-free gate on the converged turn envelope — one shape, one transport, two clients.
# ABOUTME: Fails when a change reintroduces channel identity, a half-rendered block, or a hand-rolled send.

# WHY THIS EXISTS
# ---------------
# Six phases converged the chat surfaces: `is_messaging` and `ChannelProfile`
# were replaced by declared render capabilities, every reply became one
# `TurnEnvelope` of ordered `ReplyBlock`s, and both clients came to paint the
# blocks the server declared through one `sendTurn` transport.
#
# Convergence achieved once is a snapshot. What makes it a framework is that
# adding a capability to one surface without the others fails at authoring
# time — and nothing did that, which is exactly why every gap in the original
# survey passed CI green. This script is that gate.
#
# MODES
#   check-turn-envelope.sh              — the standing invariants, exit non-zero
#                                         if any is already broken
#   check-turn-envelope.sh <BASE_REF>   — additionally FAIL on what the diff
#                                         against BASE_REF introduces
#
# Three of the four checks are whole-tree rather than diff-scoped, because the
# tree satisfies them today: a block kind rendered by one client and not the
# other, or a stale generated catalogue, is wrong however it got there. Only
# the hand-rolled-transport check is diff-scoped — `fetch(` is a normal thing
# for a client to do, and it is only the chat endpoint it must not aim at.

set -uo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"
cd "$PROJECT_ROOT" || exit 1

BASE_REF="${1:-}"
FAILED=false

WIRE_TYPES="packages/shared-types/src/turn.ts"
PIPELINE_ENVELOPE="crates/pierre-chat-pipeline/src/envelope.rs"
SURFACE_PROFILE="crates/pierre-chat-pipeline/src/surface_profile.rs"
GENERATED="packages/shared-constants/src/surface-capabilities.generated.ts"
NOTIFICATION_SCREENS="crates/pierre-core/src/models/notifications.rs"
WEB_RENDERER="frontend/src/components/chat/MessageItem.tsx"
MOBILE_RENDERER="frontend-mobile/src/screens/chat/MessageList.tsx"
REGENERATE="cd packages/shared-constants && bun run generate"

echo -e "${BLUE}==== Turn Envelope Convergence Gate ====${NC}"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

for required in "$WIRE_TYPES" "$PIPELINE_ENVELOPE" "$SURFACE_PROFILE" "$WEB_RENDERER" "$MOBILE_RENDERER" "$NOTIFICATION_SCREENS"; do
    if [[ ! -f "$required" ]]; then
        echo -e "${RED}❌ ${required} not found — this gate is stale.${NC}"
        FAILED=true
    fi
done
if [[ "$FAILED" == "true" ]]; then
    echo -e "${RED}❌ TURN ENVELOPE GATE FAILED${NC}"
    exit 1
fi

# ---------------------------------------------------------------------------
# Check 1: channel identity stays retired
# ---------------------------------------------------------------------------
# `is_messaging` was one boolean answering four unrelated questions at seven
# call sites; `ChannelProfile` was the struct that carried it. Both are gone.
# A new capability forks on what it needs, declared on RenderCapabilities —
# never on which product the turn came from.
IDENTITY_HITS="$(grep -rn --include='*.rs' --include='*.ts' --include='*.tsx' \
    -E '\bis_messaging\b|\bisMessaging\b|\bChannelProfile\b' \
    crates frontend/src frontend-mobile/src packages sdk/src 2>/dev/null \
    | grep -v "$(basename "$0")" || true)"

if [[ -n "$IDENTITY_HITS" ]]; then
    echo -e "${RED}❌ Channel identity is back:${NC}"
    printf '%s\n' "$IDENTITY_HITS" | sed 's/^/   /'
    echo -e "${YELLOW}   Declare the capability the code actually needs on RenderCapabilities${NC}"
    echo -e "${YELLOW}   and read that. One flag standing in for four capabilities is the${NC}"
    echo -e "${YELLOW}   pattern the convergence removed.${NC}"
    FAILED=true
else
    echo -e "${GREEN}✅ Channel identity: no is_messaging / ChannelProfile anywhere.${NC}"
fi

# ---------------------------------------------------------------------------
# Check 2: every reply block is rendered by BOTH clients
# ---------------------------------------------------------------------------
# The wire union in shared-types is the vocabulary; each client switches on
# `block.type`. A kind one client has an arm for and the other does not is a
# block an athlete sees on one platform and not the other — silently, because
# an unmatched switch arm renders nothing and throws nothing.
grep -oE "type: '[a-z_]+'" "$WIRE_TYPES" | sed -E "s/type: '([a-z_]+)'/\1/" \
    | sort -u > "$TMP/wire_kinds.txt"

# The pipeline's own vocabulary, from ReplyBlockKind::as_str.
awk '/impl ReplyBlockKind \{/,/^\}/' "$PIPELINE_ENVELOPE" \
    | grep -oE '=> "[a-z_]+",' | sed -E 's/=> "([a-z_]+)",/\1/' \
    | sort -u > "$TMP/rust_kinds.txt"

WIRE_N=$(grep -c . < "$TMP/wire_kinds.txt" || true)
RUST_N=$(grep -c . < "$TMP/rust_kinds.txt" || true)

if [[ "$WIRE_N" -eq 0 || "$RUST_N" -eq 0 ]]; then
    echo -e "${RED}❌ Parsed zero reply-block kinds — this check is stale.${NC}"
    FAILED=true
else
    DRIFT="$(comm -3 "$TMP/wire_kinds.txt" "$TMP/rust_kinds.txt" | tr -d '\t' | tr '\n' ' ' | sed 's/ *$//')"
    if [[ -n "$DRIFT" ]]; then
        echo -e "${RED}❌ Reply-block vocabulary drift between Rust and the wire types: ${DRIFT}${NC}"
        echo -e "${YELLOW}   ReplyBlockKind (${PIPELINE_ENVELOPE}) and the ReplyBlock union${NC}"
        echo -e "${YELLOW}   (${WIRE_TYPES}) must name the same kinds.${NC}"
        FAILED=true
    fi

    MISSING=""
    while read -r kind; do
        [[ -z "$kind" ]] && continue
        WEB_ARM=$(grep -c "case '${kind}':" "$WEB_RENDERER" || true)
        MOBILE_ARM=$(grep -c "case '${kind}':" "$MOBILE_RENDERER" || true)
        if [[ "$WEB_ARM" -eq 0 && "$MOBILE_ARM" -eq 0 ]]; then
            MISSING+="   ${kind}: neither client renders it\n"
        elif [[ "$WEB_ARM" -eq 0 ]]; then
            MISSING+="   ${kind}: mobile renders it, web has no arm (${WEB_RENDERER})\n"
        elif [[ "$MOBILE_ARM" -eq 0 ]]; then
            MISSING+="   ${kind}: web renders it, mobile has no arm (${MOBILE_RENDERER})\n"
        fi
    done < "$TMP/wire_kinds.txt"

    if [[ -n "$MISSING" ]]; then
        echo -e "${RED}❌ Reply blocks rendered by only one client:${NC}"
        printf '%b' "$MISSING"
        echo -e "${YELLOW}   A block kind ships to both in-app clients or to neither. Add the${NC}"
        echo -e "${YELLOW}   missing arm in this same change — the server already decided the${NC}"
        echo -e "${YELLOW}   block exists, so the other client is simply dropping it.${NC}"
        FAILED=true
    else
        echo -e "${GREEN}✅ Reply blocks: all ${WIRE_N} kinds have an arm in both client renderers.${NC}"
    fi
fi

# ---------------------------------------------------------------------------
# Check 3: the generated capability catalogue is current
# ---------------------------------------------------------------------------
# The catalogue is generated from the server's own SurfaceProfile::resolve
# table. Names are compared by name; values are compared by the digest the
# generator stamped in, because `scene_raster: false` reads exactly like
# `scene_raster: true` to a name comparison.
if [[ ! -f "$GENERATED" ]]; then
    echo -e "${RED}❌ ${GENERATED} is missing.${NC}"
    echo -e "${YELLOW}   Regenerate it: ${REGENERATE}${NC}"
    FAILED=true
else
    STALE=""

    # Scoped to SurfaceId::as_str — the only place a surface names itself.
    # Unscoped, `call_type`'s own arms would read as two extra surfaces.
    awk '/pub const fn as_str\(self\) -> &.static str \{/,/^    \}/' "$SURFACE_PROFILE" \
        | grep -oE '=> "[a-z_]+",' | sed -E 's/=> "([a-z_]+)",/\1/' \
        | sort -u > "$TMP/rust_surfaces.txt"
    awk '/export const SURFACE_CAPABILITY_IDS/,/\] as const;/' "$GENERATED" \
        | grep -oE "'[a-z_]+'" | tr -d "'" | sort -u > "$TMP/gen_surfaces.txt"
    if [[ "$(grep -c . < "$TMP/rust_surfaces.txt" || true)" -eq 0 ]]; then
        echo -e "${RED}❌ Parsed zero surface ids from ${SURFACE_PROFILE} — this check is stale.${NC}"
        FAILED=true
    elif ! diff -q "$TMP/rust_surfaces.txt" "$TMP/gen_surfaces.txt" >/dev/null 2>&1; then
        STALE+="   surfaces: $(comm -3 "$TMP/rust_surfaces.txt" "$TMP/gen_surfaces.txt" | tr -d '\t' | tr '\n' ' ')\n"
    fi

    awk '/export const REPLY_BLOCK_KINDS/,/\] as const;/' "$GENERATED" \
        | grep -oE "'[a-z_]+'" | tr -d "'" | sort -u > "$TMP/gen_kinds.txt"
    if ! diff -q "$TMP/rust_kinds.txt" "$TMP/gen_kinds.txt" >/dev/null 2>&1; then
        STALE+="   block kinds: $(comm -3 "$TMP/rust_kinds.txt" "$TMP/gen_kinds.txt" | tr -d '\t' | tr '\n' ' ')\n"
    fi

    # Scoped to NotificationScreen::as_str: the `surface()` arms in the same
    # impl block return surface ids, which are not screen tokens.
    awk '/impl NotificationScreen \{/,/^\}/' "$NOTIFICATION_SCREENS" \
        | awk '/pub const fn as_str\(self\) -> &.static str \{/,/^    \}/' \
        | grep -oE '=> "[a-z_]+",' | sed -E 's/=> "([a-z_]+)",/\1/' \
        | sort -u > "$TMP/rust_screen_tokens.txt"
    awk '/export const NOTIFICATION_SCREEN_SURFACES/,/\} as const;/' "$GENERATED" \
        | grep -oE "^  '[a-z_]+':" | tr -d " ':" | sort -u > "$TMP/gen_screens.txt"
    if [[ "$(grep -c . < "$TMP/rust_screen_tokens.txt" || true)" -eq 0 ]]; then
        echo -e "${RED}❌ Parsed zero notification screens from ${NOTIFICATION_SCREENS} — this check is stale.${NC}"
        FAILED=true
    elif ! diff -q "$TMP/rust_screen_tokens.txt" "$TMP/gen_screens.txt" >/dev/null 2>&1; then
        STALE+="   notification screens: $(comm -3 "$TMP/rust_screen_tokens.txt" "$TMP/gen_screens.txt" | tr -d '\t' | tr '\n' ' ')\n"
    fi

    DIGEST_NOW="$("$SCRIPT_DIR/surface-capabilities-fingerprint.sh" 2>/dev/null || echo "unreadable")"
    DIGEST_FILE="$(grep -oE '^// capability-digest: [0-9a-f]+' "$GENERATED" | awk '{print $3}')"
    if [[ -z "$DIGEST_FILE" ]]; then
        STALE+="   capability digest: the generated file carries none\n"
    elif [[ "$DIGEST_NOW" != "$DIGEST_FILE" ]]; then
        STALE+="   capability values changed (digest ${DIGEST_FILE} → ${DIGEST_NOW})\n"
    fi

    CONTENT_NOW="$("$SCRIPT_DIR/surface-capabilities-fingerprint.sh" --content 2>/dev/null || echo "unreadable")"
    CONTENT_FILE="$(grep -oE '^// content-digest: [0-9a-f]+' "$GENERATED" | awk '{print $3}')"
    if [[ -z "$CONTENT_FILE" ]]; then
        STALE+="   content digest: the generated file carries none\n"
    elif [[ "$CONTENT_NOW" != "$CONTENT_FILE" ]]; then
        STALE+="   the generated catalogue was edited by hand (content ${CONTENT_FILE} → ${CONTENT_NOW})\n"
    fi

    if [[ -n "$STALE" ]]; then
        echo -e "${RED}❌ The generated capability catalogue is stale:${NC}"
        printf '%b' "$STALE"
        echo -e "${YELLOW}   Regenerate it against a running server:${NC}"
        echo -e "${YELLOW}     ${REGENERATE}${NC}"
        echo -e "${YELLOW}   Both clients read that file. Until it is regenerated they are${NC}"
        echo -e "${YELLOW}   generating code for a surface table the server no longer has.${NC}"
        FAILED=true
    else
        echo -e "${GREEN}✅ Generated catalogue: surfaces, block kinds, screens current; neither side hand-edited.${NC}"
    fi
fi

# ---------------------------------------------------------------------------
# Gate: what THIS change introduces
# ---------------------------------------------------------------------------
if [[ -n "$BASE_REF" ]]; then
    echo ""
    echo -e "${BLUE}---- New in this change (vs ${BASE_REF}) ----${NC}"

    ADDED="$(git diff -U0 "${BASE_REF}...HEAD" -- \
        'frontend/src' 'frontend-mobile/src' 'packages/*/src' 'sdk/src' 2>/dev/null \
        | grep '^+' | grep -v '^+++' || true)"

    # A client that builds its own request to the chat endpoint is a second
    # transport: it re-decides framing, auth, SSE and error mapping, and the
    # two drift. `sendTurn` is the one way a message reaches the server.
    HAND_ROLLED="$(printf '%s\n' "$ADDED" | grep -E 'fetch[[:space:]]*\(' | grep -E '/api/chat' || true)"
    if [[ -n "$HAND_ROLLED" ]]; then
        echo -e "${RED}❌ A hand-rolled transport to /api/chat is back:${NC}"
        printf '%s\n' "$HAND_ROLLED" | sed 's/^+/   /'
        echo -e "${YELLOW}   Send through sendTurn (@pierre/api-client). It is the one place${NC}"
        echo -e "${YELLOW}   that knows the envelope, the SSE framing and the error shape.${NC}"
        FAILED=true
    else
        echo -e "${GREEN}✅ Transport: this change adds no direct fetch() to /api/chat.${NC}"
    fi
fi

if [[ "$FAILED" == "true" ]]; then
    echo ""
    echo -e "${RED}❌ TURN ENVELOPE GATE FAILED${NC}"
    exit 1
fi
exit 0
