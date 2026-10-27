#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Exports a production iOS bundle of the mobile app and fails if the E2E-only seeded-session route is in it
# ABOUTME: The route signs the app in from a link (carnet#1201); metro.config.js must leave app/(auth)/e2e/ out of every release export

# WHY THIS EXISTS
# ---------------
# frontend-mobile/app/(auth)/e2e/ holds a route that signs the app in with a
# session the Maestro control process minted. Metro's block list keeps the
# directory out of every bundle but a local E2E development one, and
# __tests__/seededSessionBundling.test.ts pins the block list's decisions. This
# check proves the outcome instead of the decision: it runs the export a store,
# TestFlight or EAS update build runs, with every switch the route answers to
# turned on (the E2E flag, a loopback API URL), and reads the bundle.
#
# It fails closed: the export must exit 0, must have written a JavaScript
# bundle, and that bundle must hold the login screen (proof it is the app, not
# an empty file) before the absence of the route marker counts for anything.
#
# USAGE (from the repository root; frontend-mobile's dependencies installed)
#   ./scripts/ci/check-seeded-session-route-not-bundled.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MOBILE="$ROOT/frontend-mobile"
OUT="$(mktemp -d)"
trap 'rm -rf "$OUT"' EXIT

# The route's testID, and the screen every bundle must carry.
ROUTE_MARKER='e2e-seeded-session'
APP_MARKER='login-screen'

echo "Exporting a production iOS bundle with the E2E flag and a loopback API set..."
# NODE_ENV is unset so the export chooses it, as EAS does: `expo export` only
# fills it in when it is empty.
if ! export_log="$(cd "$MOBILE" && env -u NODE_ENV \
    EXPO_PUBLIC_E2E_SEEDED_SESSIONS=1 \
    EXPO_PUBLIC_API_URL=http://127.0.0.1:8081 \
    CI=1 \
    npx expo export --platform ios --no-bytecode --output-dir "$OUT" 2>&1)"; then
    echo "$export_log" | tail -40
    echo "FAIL: expo export did not complete, so the bundle could not be checked"
    exit 1
fi

bundles=()
while IFS= read -r -d '' file; do
    bundles+=("$file")
done < <(find "$OUT" -type f -name '*.js' -print0)
if [ "${#bundles[@]}" -eq 0 ]; then
    echo "$export_log" | tail -40
    echo "FAIL: expo export wrote no JavaScript bundle under $OUT"
    exit 1
fi

if ! grep -q -- "$APP_MARKER" "${bundles[@]}"; then
    echo "FAIL: the exported bundle does not hold '$APP_MARKER'; it is not the app, so its contents prove nothing"
    exit 1
fi

if grep -l -- "$ROUTE_MARKER" "${bundles[@]}"; then
    echo "FAIL: a production export carries the E2E seeded-session route ('$ROUTE_MARKER' found above)."
    echo "      metro.config.js must block frontend-mobile/app/(auth)/e2e/ in every release bundle."
    exit 1
fi

echo "OK: the production bundle (${#bundles[@]} file(s)) holds the app and no seeded-session route"
