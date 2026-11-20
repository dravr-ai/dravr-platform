#!/usr/bin/env bash
# ABOUTME: Fixture test for remote-clippy-verdict.sh: tree pinning, record refusal, and every check mismatch
# ABOUTME: Pins that only a workspace CLEAN for HEAD's exact tree passes, and that tree never touches the index
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Tier 1e skips its local compile on `check` exiting 0, so a check that passes
# wrongly silently drops a lint from the gate. Each case builds a throwaway repo
# with an origin/main, so record's merge-base resolves the way it does in a
# real checkout.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/remote-clippy-verdict.sh}"
ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum | cut -c1-64; else shasum -a 256 | cut -c1-64; fi; }

# A repo whose origin/main is its first commit, with one commit of work on top.
make_repo() {
  local dir origin
  origin="$(mktemp -d "$ROOT/origin-XXXXXX")"
  git -C "$origin" init -q --bare
  dir="$(mktemp -d "$ROOT/case-XXXXXX")"
  git -C "$dir" init -q -b main
  git -C "$dir" config user.email test@example.com
  git -C "$dir" config user.name test
  printf 'base\n' >"$dir/README"
  git -C "$dir" add -A
  git -C "$dir" commit -qm base
  git -C "$dir" remote add origin "$origin"
  git -C "$dir" push -q origin main
  printf 'work\n' >"$dir/work.txt"
  git -C "$dir" add -A
  git -C "$dir" commit -qm work
  printf '%s\n' "$dir"
}

v() { local dir="$1"; shift; ( cd "$dir" && "$UNDER_TEST" "$@" ); }
patch_sha() { v "$1" patch "$(git -C "$1" merge-base HEAD origin/main)" | sha256; }
# What the skill does: prepare notes the scope it sent with the patch, then record follows a CLEAN.
send_and_record() { local sha; sha="$(patch_sha "$1")"; v "$1" sent "$sha" "$2"; v "$1" record "$sha" "$2"; }

expect_check() { # $1 label, $2 dir, $3 expected exit, $4 literal the output must carry
  local code=0 out
  out="$(v "$2" check 2>&1)" || code=$?
  if [ "$code" != "$3" ]; then fail "$1 (check exit $code, expected $3)"
  elif [[ "$out" != *"$4"* ]]; then fail "$1 (output '$out' is missing: $4)"
  else pass "$1"; fi
}

echo "remote-clippy-verdict.sh"

echo "  case 1: no record, no skip"
dir="$(make_repo)"
expect_check "check fails with no record" "$dir" 1 "no cloud clippy verdict recorded"

echo "  case 2: a workspace CLEAN for a clean tree covers HEAD"
dir="$(make_repo)"
send_and_record "$dir" workspace >/dev/null
expect_check "check passes on the recorded tree" "$dir" 0 \
  "cloud clippy CLEAN (workspace) covers HEAD's tree $(git -C "$dir" rev-parse 'HEAD^{tree}')"

echo "  case 3: an untracked file the run saw, then committed, still matches"
dir="$(make_repo)"
printf 'new test\n' >"$dir/new_test.rs"
send_and_record "$dir" workspace >/dev/null
git -C "$dir" add -A
git -C "$dir" commit -qm "commit the file the run saw"
expect_check "committing exactly what was linted keeps the verdict" "$dir" 0 "cloud clippy CLEAN (workspace)"

echo "  case 4: an edit after the run invalidates it"
dir="$(make_repo)"
send_and_record "$dir" workspace >/dev/null
printf 'edit after the run\n' >>"$dir/work.txt"
git -C "$dir" commit -qam "edit after the run"
expect_check "a changed tree falls back to the local compile" "$dir" 1 "is not HEAD's tree"

echo "  case 5: a package-scoped CLEAN does not cover Tier 1e"
dir="$(make_repo)"
send_and_record "$dir" pierre-core >/dev/null
expect_check "scope other than workspace is refused" "$dir" 1 "recorded scope is 'pierre-core', not workspace"

echo "  case 6: record refuses a report for another patch"
dir="$(make_repo)"
code=0
out="$(v "$dir" record "0000000000000000000000000000000000000000000000000000000000000000" workspace 2>&1)" || code=$?
if [ "$code" = 2 ]; then pass "a foreign patch sha is refused"; else fail "a foreign patch sha is refused (exit $code)"; fi
if [[ "$out" == *"is not this checkout's"*"Nothing recorded."* ]]; then pass "the refusal says why and that nothing was recorded"
else fail "the refusal says why and that nothing was recorded (output '$out')"; fi
expect_check "nothing was recorded" "$dir" 1 "no cloud clippy verdict recorded"

echo "  case 6b: a CLEAN from a narrower run cannot be recorded as workspace"
dir="$(make_repo)"
sha="$(patch_sha "$dir")"
v "$dir" sent "$sha" "cargo check --workspace"
code=0
msg="$(v "$dir" record "$sha" workspace 2>&1)" || code=$?
if [ "$code" = 2 ] && printf '%s' "$msg" | grep -qF "no run with scope 'workspace' was prepared"; then
  pass "a scope never sent with this patch is refused"
else fail "a scope never sent with this patch is refused (exit $code: $msg)"; fi
expect_check "nothing was recorded for the narrower run" "$dir" 1 "no cloud clippy verdict recorded"

echo "  case 7: a non-CLEAN record never passes"
dir="$(make_repo)"
printf 'verdict=FINDINGS\nscope=workspace\ntree=%s\n' "$(git -C "$dir" rev-parse 'HEAD^{tree}')" \
  >"$(git -C "$dir" rev-parse --absolute-git-dir)/remote-clippy-verdict"
expect_check "FINDINGS is not CLEAN" "$dir" 1 "recorded verdict is 'FINDINGS', not CLEAN"

echo "  case 8: tree leaves the real index untouched"
dir="$(make_repo)"
printf 'staged\n' >"$dir/staged.txt"
git -C "$dir" add staged.txt
printf 'untracked\n' >"$dir/untracked.txt"
before="$(git -C "$dir" diff --cached --name-only | tr '\n' ' ')"
v "$dir" tree >/dev/null
after="$(git -C "$dir" diff --cached --name-only | tr '\n' ' ')"
if [ "$before" = "$after" ]; then pass "the staged set is unchanged"; else fail "the staged set changed: '$before' -> '$after'"; fi
if git -C "$dir" ls-files --error-unmatch untracked.txt >/dev/null 2>&1; then
  fail "the untracked file was staged"
else pass "the untracked file stays untracked"; fi

echo ""
if [ "$failures" -ne 0 ]; then
  echo "❌ remote-clippy-verdict.test.sh: $failures case(s) failed"
  exit 1
fi
echo "✅ remote-clippy-verdict.test.sh: every case passed"
