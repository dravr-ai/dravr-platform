#!/usr/bin/env bash
# ABOUTME: Fixture test for check-server-features.sh — copies the four feature sites and drifts them one at a time
# ABOUTME: Pins that the real tree passes, a drifted site fails by name, and a missing site fails as verified-nothing
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-server-features.sh}"
WORK="$(mktemp -d)"
OUT="$WORK/out"
trap 'rm -rf "$WORK"' EXIT

DOCKERFILE="docker/images/server/Dockerfile"
ACTION=".github/actions/server-binaries/action.yml"
WORKFLOW=".github/workflows/ci-backend.yml"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

# A fresh copy of the three files under $WORK/tree.
fixture() {
  rm -rf "$WORK/tree"
  local f
  for f in "$DOCKERFILE" "$ACTION" "$WORKFLOW"; do
    mkdir -p "$WORK/tree/$(dirname "$f")"
    cp "$REPO_ROOT/$f" "$WORK/tree/$f"
  done
}

run_check() { # $1 = repo root; prints the exit status, output lands in $OUT
  local code=0
  "$UNDER_TEST" "$1" >"$OUT" 2>&1 || code=$?
  echo "$code"
}

expect() { # $1 = label, $2 = actual exit, $3 = expected exit
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, expected $3)"
    sed 's/^/      /' "$OUT"
  fi
}

expect_out() { # $1 = label, $2 = fixed string the output must contain
  if grep -qF -- "$2" "$OUT"; then pass "$1"; else
    fail "$1"
    sed 's/^/      /' "$OUT"
  fi
}

# Rewrites the first line of $1 matching $2 with sed expression $3.
edit_first() {
  local file="$1" match="$2" expr="$3" line
  line=$(grep -n -m1 -- "$match" "$file" | cut -d: -f1)
  if [ -z "$line" ]; then
    echo "fixture: '$match' not found in $file" >&2
    exit 1
  fi
  sed "${line}s/${expr}/" "$file" >"$file.new" && mv "$file.new" "$file"
}

echo "check-server-features.sh fixture tests"

expect "the real tree passes" "$(run_check "$REPO_ROOT")" 0
expect_out "the pass names four sites" "4 sites build"

fixture
edit_first "$WORK/tree/$ACTION" '--features "' 'gcp-kms"/gcp-kms,extra"'
expect "a feature added to the action alone fails" "$(run_check "$WORK/tree")" 1
expect_out "the drifted site is named" "$ACTION:"
expect_out "the drifted value is shown" "gcp-kms,extra"

fixture
edit_first "$WORK/tree/$DOCKERFILE" 'chef cook --profile' 'telemetry,/'
expect "a feature dropped from the Dockerfile cook line fails" "$(run_check "$WORK/tree")" 1
expect_out "the Dockerfile site is named" "$DOCKERFILE:"

fixture
edit_first "$WORK/tree/$WORKFLOW" "features: 'server-production" "gcp-kms'/gcp-kms,toon'"
expect "a drifted clippy matrix entry fails" "$(run_check "$WORK/tree")" 1
expect_out "the workflow site is named" "$WORKFLOW:"

fixture
edit_first "$WORK/tree/$WORKFLOW" '- name: production' 'production/prod'
expect "a missing matrix site fails" "$(run_check "$WORK/tree")" 1
expect_out "it says the check verified nothing" "found 3 of the 4"

fixture
rm "$WORK/tree/$ACTION"
expect "a missing file fails" "$(run_check "$WORK/tree")" 1
expect_out "the missing file is named" "$ACTION not found"

if [ "$failures" -gt 0 ]; then
  echo "❌ $failures check-server-features.sh case(s) failed"
  exit 1
fi
echo "✅ check-server-features.sh fixture tests passed"
