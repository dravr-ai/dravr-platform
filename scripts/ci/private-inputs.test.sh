#!/usr/bin/env bash
# ABOUTME: Fixture test for private-inputs.sh — runs a copy of it inside throwaway trees shaped like the repository
# ABOUTME: Pins the usage and missing-file refusals, and that a stage copies the var-file byte for byte at mode 600
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The script finds the repository from its own location, so every case runs a
# copy placed at infra/private-inputs.sh of a tree made here: nothing is read
# from or written to the checkout this test lives in.
#
# Usage: infra/private-inputs.test.sh — bash and coreutils, no network, no terraform.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/../../infra/private-inputs.sh}"
# Every fixture lives under one directory made here, at top level: tree_with
# runs inside $(...), a subshell, so it could not record what it made.
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
OUT="$WORK/out"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

# A tree shaped like the repository, holding a copy of the script under test.
# $1 = environment directories to make (space-separated, may be empty),
# $2 = environments to give a private var-file in the tree's own .savoir.
tree_with() {
  local dir env
  dir="$(mktemp -d "$WORK/tree.XXXXXX")"
  mkdir -p "$dir/infra/environments" "$dir/.savoir/platform/infra"
  cp "$UNDER_TEST" "$dir/infra/private-inputs.sh"
  for env in $1; do
    mkdir -p "$dir/infra/environments/$env"
  done
  for env in $2; do
    printf 'llm_chain = "gemini=some-model"\nseat_cap  = 4\n# %s\n' "$env" \
      >"$dir/.savoir/platform/infra/$env.private.tfvars"
  done
  printf '%s\n' "$dir"
}

run_stage() { # $1 = tree, then the script's own arguments
  local tree="$1" code=0
  shift
  bash "$tree/infra/private-inputs.sh" "$@" >"$OUT" 2>&1 || code=$?
  echo "$code"
}

expect() { # $1 = label, $2 = actual exit, $3 = expected exit
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, expected $3)"
    sed 's/^/      /' "$OUT"
  fi
}

expect_output() { # $1 = label, $2 = text the last run must have printed
  if grep -qF -- "$2" "$OUT"; then pass "$1"; else
    fail "$1 (output lacks: $2)"
    sed 's/^/      /' "$OUT"
  fi
}

# The permission bits of a file, as octal digits, on GNU and BSD stat alike.
mode_of() {
  stat -c '%a' "$1" 2>/dev/null || stat -f '%Lp' "$1"
}

staged_nothing() { # $1 = label, $2 = tree
  local found
  found="$(find "$2/infra/environments" -name 'private.auto.tfvars' | head -n 1)"
  if [ -z "$found" ]; then pass "$1"; else fail "$1 (found $found)"; fi
}

echo "private-inputs.sh"

# ── Usage ────────────────────────────────────────────────────────────────────
tree="$(tree_with 'dev prod' 'dev prod')"
expect "no environment name is a usage error" "$(run_stage "$tree")" 2
expect_output "and prints the usage line" "usage:"
expect "an unknown environment name is a usage error" "$(run_stage "$tree" staging)" 2
expect_output "and prints the usage line" "<dev|prod>"
staged_nothing "a usage error stages nothing" "$tree"

# ── Missing source file ──────────────────────────────────────────────────────
tree="$(tree_with 'dev' '')"
expect "a missing private var-file fails" "$(run_stage "$tree" dev)" 1
expect_output "naming the file it looked for" \
  "error: $tree/.savoir/platform/infra/dev.private.tfvars not found."
expect_output "and how to get it" "savoir.sh sync"
staged_nothing "a missing source stages nothing" "$tree"

# ── Missing environment directory ────────────────────────────────────────────
tree="$(tree_with 'dev' 'dev prod')"
expect "an environment with no directory fails" "$(run_stage "$tree" prod)" 1
expect_output "naming the directory" \
  "error: $tree/infra/environments/prod is not a terraform environment"
staged_nothing "a missing environment stages nothing" "$tree"

# ── A stage ──────────────────────────────────────────────────────────────────
tree="$(tree_with 'dev prod' 'dev prod')"
source_file="$tree/.savoir/platform/infra/dev.private.tfvars"
staged="$tree/infra/environments/dev/private.auto.tfvars"
expect "a stage succeeds" "$(run_stage "$tree" dev)" 0
expect_output "and counts the assignments it staged" "staged private inputs for dev: 2 assignments"
if [ -f "$staged" ]; then pass "the staged file is private.auto.tfvars in the environment"; else
  fail "the staged file is private.auto.tfvars in the environment (no $staged)"
fi
if cmp -s "$source_file" "$staged"; then pass "its content is the var-file's, byte for byte"; else
  fail "its content is the var-file's, byte for byte"
fi
staged_mode="$(mode_of "$staged")"
if [ "$staged_mode" = "600" ]; then pass "its mode is 600"; else
  fail "its mode is 600 (got $staged_mode)"
fi
if [ ! -e "$tree/infra/environments/prod/private.auto.tfvars" ]; then
  pass "the other environment is left alone"
else
  fail "the other environment is left alone"
fi

# ── A second run overwrites ──────────────────────────────────────────────────
printf 'llm_chain = "anthropic=another-model"\n' >"$source_file"
chmod 644 "$staged"
expect "a second stage succeeds" "$(run_stage "$tree" dev)" 0
if cmp -s "$source_file" "$staged"; then pass "it replaces the staged copy with the new content"; else
  fail "it replaces the staged copy with the new content"
fi
staged_mode="$(mode_of "$staged")"
if [ "$staged_mode" = "600" ]; then pass "and puts its mode back to 600"; else
  fail "and puts its mode back to 600 (got $staged_mode)"
fi

# ── A checkout named on the command line ─────────────────────────────────────
tree="$(tree_with 'dev' '')"
elsewhere="$(mktemp -d "$WORK/savoir.XXXXXX")"
mkdir -p "$elsewhere/platform/infra"
printf 'llm_chain = "gemini=from-elsewhere"\n' >"$elsewhere/platform/infra/dev.private.tfvars"
expect "a checkout passed as the second argument is read" "$(run_stage "$tree" dev "$elsewhere")" 0
if cmp -s "$elsewhere/platform/infra/dev.private.tfvars" \
  "$tree/infra/environments/dev/private.auto.tfvars"; then
  pass "and its var-file is the one staged"
else
  fail "and its var-file is the one staged"
fi

if [ "$failures" -gt 0 ]; then
  echo "❌ private-inputs.test.sh: $failures case(s) failed"
  exit 1
fi
echo "✅ private-inputs.test.sh: every case passed"
