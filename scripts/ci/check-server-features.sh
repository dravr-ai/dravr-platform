#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Fails when the shipped server feature set differs between the Dockerfile, server-binaries and CI clippy
# ABOUTME: Reads the four literal `--features` sites and fails closed when any is missing or they disagree
#
# Usage: check-server-features.sh [REPO_ROOT]
#
# The production feature string is spelled out in four places nothing at
# runtime joins (carnet#928):
#   docker/images/server/Dockerfile        builder stage, `cargo chef cook` and `cargo build`
#                                          (BINARY_SOURCE=builder: docker-compose, local builds)
#   .github/actions/server-binaries/action.yml  the binaries CI sizes and the dev image ships
#   .github/workflows/ci-backend.yml       the `production` clippy matrix entry
# A feature added to one and not the others ships an image that differs from the
# one CI linted, or a compose image that differs from dev. This reads all four
# and fails when they differ, naming each site. Finding fewer than four sites
# fails too: a scan that matched nothing verified nothing.

set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
DOCKERFILE="docker/images/server/Dockerfile"
ACTION=".github/actions/server-binaries/action.yml"
WORKFLOW=".github/workflows/ci-backend.yml"
EXPECTED_SITES=4

for f in "$DOCKERFILE" "$ACTION" "$WORKFLOW"; do
    if [[ ! -f "$ROOT/$f" ]]; then
        echo "❌ check-server-features: $f not found under $ROOT; the check verified nothing"
        exit 1
    fi
done

# One `file:line<TAB>features` row per site.
cargo_sites() { # $1 = file: every `cargo chef cook` / `cargo build` line's --features value
    awk -v F="$1" '
        /^[ \t]*#/ { next }
        /cargo (chef cook|build)/ { want = 1 }
        want && match($0, /--features "[^"]*"/) {
            v = substr($0, RSTART + 12, RLENGTH - 13)
            printf "%s:%d\t%s\n", F, FNR, v
            want = 0
        }
        # A backslash continues the invocation onto the next line; anything
        # else ends it.
        want && $0 !~ /\\[ \t]*$/ && !/cargo (chef cook|build)/ { want = 0 }
    ' "$ROOT/$1"
}

matrix_site() { # $1 = file: the `production` matrix entry's features value
    # Q carries the single quote: \x27 is not POSIX awk, and the runners' is mawk.
    awk -v F="$1" -v Q="'" '
        /^[ \t]*- name: production[ \t]*$/ { want = 1; next }
        want && /^[ \t]*features:/ {
            v = $0
            sub(/^[ \t]*features:[ \t]*/, "", v)
            gsub("^[\"" Q "]|[\"" Q "][ \t]*$", "", v)
            printf "%s:%d\t%s\n", F, FNR, v
            want = 0
        }
        want && /^[ \t]*- name:/ { want = 0 }
    ' "$ROOT/$1"
}

SITES="$(cargo_sites "$DOCKERFILE"; cargo_sites "$ACTION"; matrix_site "$WORKFLOW")"
COUNT=$(awk 'NF { n++ } END { print n + 0 }' <<< "$SITES")

if [[ "$COUNT" -ne "$EXPECTED_SITES" ]]; then
    echo "❌ check-server-features: found $COUNT of the $EXPECTED_SITES production --features sites; the check verified nothing"
    [[ -n "$SITES" ]] && printf '%s\n' "$SITES" | sed 's/^/   /'
    echo "   Expected: 2 in $DOCKERFILE (cook + build), 1 in $ACTION, 1 in $WORKFLOW (production matrix)."
    exit 1
fi

DISTINCT=$(cut -f2 <<< "$SITES" | sort -u | awk 'NF { n++ } END { print n + 0 }')
if [[ "$DISTINCT" -ne 1 ]]; then
    echo "❌ check-server-features: the shipped server feature set differs between its sites:"
    printf '%s\n' "$SITES" | sed 's/^/   /'
    echo "   Make every site build the same features, or the image differs from what CI linted."
    exit 1
fi

echo "✅ check-server-features: $COUNT sites build \"$(head -1 <<< "$SITES" | cut -f2)\""
