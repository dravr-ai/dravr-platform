#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Prints a short digest of the capability values SurfaceProfile::resolve hands out.
# ABOUTME: One implementation, read by both the generator and the staleness gate.

# WHY THIS EXISTS
# ---------------
# The generated capability catalogue carries the surface ids and block kinds as
# tokens, so a *renamed* or *added* surface is caught by comparing names. A
# flipped value is not: `scene_raster: false` reads exactly like
# `scene_raster: true` to a name comparison, and a client would keep generating
# code for an affordance the server stopped sending.
#
# So the generator stamps this digest into the file it writes and
# check-turn-envelope.sh recomputes it. The input is the two capability
# constructors' own bodies with comments and blank lines stripped: prose about
# a capability may be rewritten freely, the values may not.

set -uo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"

SOURCE="$PROJECT_ROOT/crates/pierre-chat-pipeline/src/surface_profile.rs"
GENERATED="$PROJECT_ROOT/packages/shared-constants/src/surface-capabilities.generated.ts"

# --content hashes the EMITTED table rather than the Rust constructors.
#
# The source digest catches "surface_profile.rs moved and nobody regenerated".
# It cannot catch the opposite: a value edited by hand in the generated file,
# because the messaging numbers are not literals here at all -- they come from
# canot's ChannelDescriptor at runtime. Two digests, two directions.
if [[ "${1:-}" == "--content" ]]; then
    if [[ ! -f "$GENERATED" ]]; then
        echo "generated catalogue not found at $GENERATED" >&2
        exit 1
    fi
    ROWS="$(grep -vE '^// capability-digest:|^// content-digest:' "$GENERATED" \
        | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//' \
        | grep -vE '^$')"
    if [[ -z "$ROWS" ]]; then
        echo "Extracted zero catalogue lines -- this fingerprint is stale." >&2
        exit 1
    fi
    if command -v sha256sum >/dev/null 2>&1; then
        printf '%s\n' "$ROWS" | sha256sum | cut -c1-16
    else
        printf '%s\n' "$ROWS" | shasum -a 256 | cut -c1-16
    fi
    exit 0
fi

if [[ ! -f "$SOURCE" ]]; then
    echo "surface_profile.rs not found at $SOURCE" >&2
    exit 1
fi

# The Rust constructors are half the fingerprint: they catch "the source moved
# and nobody regenerated". The emitted table is the other half — without it a
# hand-edit of the generated file (a value nudged, a row dropped) leaves the
# recorded digest matching and the drift invisible, which is the exact shape
# this gate exists to refuse.
BODIES="$(awk '/^const fn (in_app|messaging)_capabilities/,/^\}/' "$SOURCE" \
    | grep -vE '^[[:space:]]*//' \
    | grep -vE '^[[:space:]]*$' \
    | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//')"

if [[ -z "$BODIES" ]]; then
    echo "Extracted zero capability-constructor lines — this fingerprint is stale." >&2
    exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
    printf '%s\n' "$BODIES" | sha256sum | cut -c1-16
else
    printf '%s\n' "$BODIES" | shasum -a 256 | cut -c1-16
fi
