#!/usr/bin/env bash
# ABOUTME: Fixture test for .claude/skills/lib/worktree.sh — the worktree facts three skills share
# ABOUTME: Pins that main and current are told apart, since reading one as the other removes the wrong tree
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# `git rev-parse --show-toplevel` used to be spelled in all three scripts and
# meant the FEATURE worktree in two of them and MAIN in the third. A helper that
# blurred them would let merge-and-cleanup run from a feature tree and remove
# the wrong one, so both are exercised from inside a real second worktree.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/../../.claude/skills/lib/worktree.sh}"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

expect() { # $1 = label, $2 = actual, $3 = expected
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1"
    echo "      got:      $2"
    echo "      expected: $3"
  fi
}

echo "worktree.sh fixture tests"

# A repo with a main worktree and one feature worktree beside it.
root="$(mktemp -d)"
main="$root/dravr-platform"
mkdir -p "$main"
git -C "$main" init -q
git -C "$main" config user.email test@example.com
git -C "$main" config user.name test
mkdir -p "$main/.claude/skills"
echo seed >"$main/seed.txt"
git -C "$main" add -A
git -C "$main" commit -qm base
feature="$root/pierre_mcp_server-feature-demo"
git -C "$main" worktree add -q -b feature/demo "$feature"

# macOS puts mktemp dirs under /var, a symlink to /private/var; git reports the
# resolved path, so compare against the resolved one.
resolved_main="$(cd "$main" && pwd -P)"
resolved_feature="$(cd "$feature" && pwd -P)"

run_in() { # $1 = dir, $2 = function to call, $3... = args
  local dir="$1"
  shift
  ( cd "$dir" && . "$UNDER_TEST" && "$@" )
}

expect "main_worktree_root from the main worktree" \
  "$(run_in "$main" main_worktree_root)" "$resolved_main"
expect "main_worktree_root from a feature worktree still names main" \
  "$(run_in "$feature" main_worktree_root)" "$resolved_main"

expect "current_worktree_root from the main worktree" \
  "$(run_in "$main" current_worktree_root)" "$resolved_main"
expect "current_worktree_root from a feature worktree names that worktree" \
  "$(run_in "$feature" current_worktree_root)" "$resolved_feature"

expect "feature_worktree_path slugifies the branch" \
  "$(run_in "$main" feature_worktree_path feature/single-source-sweep)" \
  "$(dirname "$resolved_main")/pierre_mcp_server-feature-single-source-sweep"
expect "feature_worktree_path resolves the same from either worktree" \
  "$(run_in "$feature" feature_worktree_path feature/demo)" \
  "$(dirname "$resolved_main")/pierre_mcp_server-feature-demo"

expect "last_branch_file lives in the main worktree, wherever it is called from" \
  "$(run_in "$feature" last_branch_file)" \
  "$resolved_main/.claude/skills/.last-feature-branch"

rm -rf "$root"

echo ""
if [ "$failures" -ne 0 ]; then
  echo "❌ $failures worktree-lib case(s) failed"
  exit 1
fi
echo "✅ all worktree-lib cases passed"
