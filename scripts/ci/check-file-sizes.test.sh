#!/usr/bin/env bash
# ABOUTME: Fixture test for check-file-sizes.sh — builds throwaway git repos and
# ABOUTME: asserts each ratchet branch, so the gate cannot silently pass everything.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# An unverified gate is worse than no gate: it reports ✅ forever and everyone
# believes the budget is enforced. Each case below builds a base commit, makes a
# change on top, and asserts the exit code the ratchet must produce.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="$SCRIPT_DIR/check-file-sizes.sh"
MAX=1200

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

# Build a repo with a base commit, then apply $2 and commit it as HEAD.
# $1 = base setup function, $2 = change function
make_repo() {
  local dir
  dir="$(mktemp -d)"
  git -C "$dir" init -q
  git -C "$dir" config user.email test@example.com
  git -C "$dir" config user.name test
  mkdir -p "$dir/crates/demo/src"
  printf '%s\n' "$dir"
}

gen_lines() { # $1 = count, $2 = path
  : >"$2"
  local i=1
  while [ "$i" -le "$1" ]; do
    printf 'let x%d = %d;\n' "$i" "$i" >>"$2"
    i=$((i + 1))
  done
}

commit_all() {
  git -C "$1" add -A
  git -C "$1" commit -qm "$2"
}

# Runs the gate against HEAD~1 and echoes the exit code.
run_gate() {
  local dir="$1" code=0
  ( cd "$dir" && GATE_BASE_REF='' "$UNDER_TEST" HEAD~1 >/tmp/file-sizes-test.out 2>&1 ) || code=$?
  echo "$code"
}

# Runs the gate with NO argument — the shape architectural-validation.sh uses —
# with $GATE_BASE_REF optionally supplied as $2. This is the CI call path.
run_gate_default() {
  local dir="$1" base="${2:-}" code=0
  ( cd "$dir" && GATE_BASE_REF="$base" "$UNDER_TEST" >/tmp/file-sizes-test.out 2>&1 ) || code=$?
  echo "$code"
}

expect() { # $1 = label, $2 = actual, $3 = expected
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, expected $3)"
    sed 's/^/      /' /tmp/file-sizes-test.out
  fi
}

echo "check-file-sizes.sh"

# 1. A new file under the ceiling passes.
d="$(make_repo)"
gen_lines 10 "$d/crates/demo/src/lib.rs"
commit_all "$d" base
gen_lines 900 "$d/crates/demo/src/small.rs"
commit_all "$d" change
expect "new file under the ceiling passes" "$(run_gate "$d")" 0
rm -rf "$d"

# 2. A new file over the ceiling fails — no grandfathering for new files.
d="$(make_repo)"
gen_lines 10 "$d/crates/demo/src/lib.rs"
commit_all "$d" base
gen_lines $((MAX + 50)) "$d/crates/demo/src/big.rs"
commit_all "$d" change
expect "new file over the ceiling fails" "$(run_gate "$d")" 1
rm -rf "$d"

# 3. An already-over-ceiling file that GROWS fails. This is the ratchet.
d="$(make_repo)"
gen_lines $((MAX + 500)) "$d/crates/demo/src/legacy.rs"
commit_all "$d" base
gen_lines $((MAX + 501)) "$d/crates/demo/src/legacy.rs"
commit_all "$d" change
expect "grandfathered file may not grow by even one line" "$(run_gate "$d")" 1
rm -rf "$d"

# 4. An already-over-ceiling file that SHRINKS passes, even though it is still
#    over the ceiling. Without this, the gate would be unlandable.
d="$(make_repo)"
gen_lines $((MAX + 500)) "$d/crates/demo/src/legacy.rs"
commit_all "$d" base
gen_lines $((MAX + 200)) "$d/crates/demo/src/legacy.rs"
commit_all "$d" change
expect "grandfathered file may shrink while still over the ceiling" "$(run_gate "$d")" 0
rm -rf "$d"

# 5. An under-ceiling file that crosses the ceiling fails.
d="$(make_repo)"
gen_lines $((MAX - 100)) "$d/crates/demo/src/growing.rs"
commit_all "$d" base
gen_lines $((MAX + 1)) "$d/crates/demo/src/growing.rs"
commit_all "$d" change
expect "file crossing the ceiling fails" "$(run_gate "$d")" 1
rm -rf "$d"

# 6. The escape hatch exempts a file that is size-by-data.
d="$(make_repo)"
gen_lines 10 "$d/crates/demo/src/lib.rs"
commit_all "$d" base
{ echo "// file-size-ok: generated locale table"; } >"$d/crates/demo/src/table.rs"
gen_lines $((MAX + 900)) /tmp/file-sizes-body.rs
cat /tmp/file-sizes-body.rs >>"$d/crates/demo/src/table.rs"
commit_all "$d" change
expect "'// file-size-ok:' exempts the file" "$(run_gate "$d")" 0
rm -rf "$d"

# 7. Files outside crates/*/src are out of scope.
d="$(make_repo)"
gen_lines 10 "$d/crates/demo/src/lib.rs"
commit_all "$d" base
mkdir -p "$d/crates/demo/tests"
gen_lines $((MAX + 900)) "$d/crates/demo/tests/huge_test.rs"
commit_all "$d" change
expect "test files are out of scope" "$(run_gate "$d")" 0
rm -rf "$d"

# 8. NESTED source paths are in scope. Most of this repo's source is nested
#    (crates/x/src/routes/chat/send_message.rs); a scope pattern that only
#    matched the top level would silently exempt nearly everything and the
#    gate would report ✅ forever.
d="$(make_repo)"
gen_lines 10 "$d/crates/demo/src/lib.rs"
commit_all "$d" base
mkdir -p "$d/crates/demo/src/routes/chat"
gen_lines $((MAX + 5)) "$d/crates/demo/src/routes/chat/deep.rs"
commit_all "$d" change
expect "nested src/ paths are in scope" "$(run_gate "$d")" 1
rm -rf "$d"

# 9. The CI checkout shape: origin/main points at HEAD, and the gate is called
#    with no argument. `origin/main...HEAD` is then empty and the gate used to
#    print its green "no changed files" line having read nothing. It must fall
#    back to the tip commit and still catch the violation in it.
d="$(make_repo)"
gen_lines 10 "$d/crates/demo/src/lib.rs"
commit_all "$d" base
gen_lines $((MAX + 50)) "$d/crates/demo/src/big.rs"
commit_all "$d" change
git -C "$d" update-ref refs/remotes/origin/main HEAD
expect "origin/main == HEAD still inspects the tip commit" "$(run_gate_default "$d")" 1
rm -rf "$d"

# 10. Same checkout shape, clean tip: the fallback must not manufacture a
#     failure out of a commit that introduced nothing over budget.
d="$(make_repo)"
gen_lines 10 "$d/crates/demo/src/lib.rs"
commit_all "$d" base
gen_lines 900 "$d/crates/demo/src/small.rs"
commit_all "$d" change
git -C "$d" update-ref refs/remotes/origin/main HEAD
expect "origin/main == HEAD passes a clean tip commit" "$(run_gate_default "$d")" 0
rm -rf "$d"

# 11. $GATE_BASE_REF is what CI sets to the push's before-sha, and it must be
#     honoured when the gate is called with no argument. Three commits, with the
#     violation in the middle one: only a base spanning the whole push sees it.
d="$(make_repo)"
gen_lines 10 "$d/crates/demo/src/lib.rs"
commit_all "$d" base
BEFORE_SHA="$(git -C "$d" rev-parse HEAD)"
gen_lines $((MAX + 50)) "$d/crates/demo/src/big.rs"
commit_all "$d" middle
gen_lines 20 "$d/crates/demo/src/other.rs"
commit_all "$d" tip
git -C "$d" update-ref refs/remotes/origin/main HEAD
expect "GATE_BASE_REF spans the whole push" "$(run_gate_default "$d" "$BEFORE_SHA")" 1
rm -rf "$d"

echo ""
if [ "$failures" -gt 0 ]; then
  echo "❌ check-file-sizes.test.sh: $failures case(s) failed."
  exit 1
fi
echo "✅ check-file-sizes.test.sh: all 11 cases passed."
exit 0
