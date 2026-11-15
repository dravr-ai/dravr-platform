#!/usr/bin/env bash
# ABOUTME: Fixture test for check-actions-lock.sh — a stub gh whose extension is missing, stale, crashes or reports
# ABOUTME: findings, so every way the gate can verify nothing is shown to fail, offline, in about a second
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The gate must fail on a finding and pass a clean report, but it must also
# fail whenever it could not have looked: no gh, no extension, another
# extension version, a crash, no report, a report that disagrees with the exit
# status, a report covering fewer workflows than exist, no lockfile, no
# workflow, no tracked file. Each of those passes silently in a gate that greps
# for a finding.
# The real extension needs the network and judges the real tree; the CI lane
# and pre-push run it. This pins the gate's reading of what it says.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-actions-lock.sh}"
WORK="$(mktemp -d)"
OUT="$WORK/out"
trap 'rm -rf "$WORK"' EXIT

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

PINNED="$("$UNDER_TEST" --pinned-version)"

# A stub gh: `actions-lock --version` answers from $STUB/version (absent = the
# extension is not installed), and any other `actions-lock` call records its
# arguments, prints $STUB/report if present and exits with $STUB/exit.
STUB="$WORK/stub"
mkdir -p "$STUB" "$WORK/bin"
cat >"$WORK/bin/gh" <<'EOF'
#!/bin/sh
[ "$1" = "actions-lock" ] || { echo "stub gh: unexpected: $*" >&2; exit 99; }
if [ "$2" = "--version" ]; then
  [ -f "$STUB/version" ] || { echo 'unknown command "actions-lock" for "gh"' >&2; exit 1; }
  echo "actions-lock version $(cat "$STUB/version")"
  exit 0
fi
echo "$*" >"$STUB/args"
echo "Scanning workflows" >&2
[ -f "$STUB/report" ] && cat "$STUB/report"
exit "$(cat "$STUB/exit")"
EOF
chmod +x "$WORK/bin/gh"
export STUB

# A fixture git repo with two workflows and a lockfile. The symlink check reads
# the index, so every change to it is staged with `track`.
FIX="$WORK/repo"
fixture() {
  rm -rf "$FIX"
  mkdir -p "$FIX/.github/workflows"
  printf 'name: a\non: push\njobs: {}\n' >"$FIX/.github/workflows/a.yml"
  printf 'name: b\non: push\njobs: {}\n' >"$FIX/.github/workflows/b.yml"
  printf "version: 'v0.0.3'\n" >"$FIX/.github/workflows/actions.lock"
  git -C "$FIX" init -q
  track
}
track() {
  case "$FIX" in "$WORK"/*) ;; *) echo "❌ not a fixture repo: $FIX" >&2; exit 1 ;; esac
  git -C "$FIX" add -A
}

# stub <version|-> <exit> <report|->: "-" leaves the file absent.
stub() {
  rm -f "$STUB/version" "$STUB/report" "$STUB/args"
  [ "$1" = "-" ] || printf '%s' "$1" >"$STUB/version"
  printf '%s' "$2" >"$STUB/exit"
  [ "$3" = "-" ] || printf '%s' "$3" >"$STUB/report"
}

LAST=0
run_gate() { LAST=0; GH="${GH_BIN:-$WORK/bin/gh}" "$UNDER_TEST" "$FIX" >"$OUT" 2>&1 || LAST=$?; }
expect_fail() { # $1 label, $2 literal the output must carry
  if [ "$LAST" -ne 0 ] && grep -qF -- "$2" "$OUT"; then pass "$1"; else
    fail "$1 (exit $LAST, wanted non-zero and: $2)"; sed 's/^/      /' "$OUT"
  fi
}
expect_pass() { # $1 label, $2 literal the output must carry
  if [ "$LAST" -eq 0 ] && grep -qF -- "$2" "$OUT"; then pass "$1"; else
    fail "$1 (exit $LAST, wanted 0 and: $2)"; sed 's/^/      /' "$OUT"
  fi
}

W2='[{"path":".github/workflows/a.yml","valid":true,"findings":[],"dependencies":[]},{"path":".github/workflows/b.yml","valid":true,"findings":[],"dependencies":[]}]'
W1='[{"path":".github/workflows/a.yml","valid":true,"findings":[],"dependencies":[]}]'
CLEAN="{\"valid\":true,\"findings\":[],\"workflows\":$W2}"
NOT_PINNED="{\"valid\":false,\"findings\":[{\"workflow\":\".github/workflows/a.yml\",\"category\":\"not-pinned\",\"severity\":\"error\",\"dependency\":\"actions/setup-go@v5\",\"detail\":\"used in workflow but not pinned in lockfile (actions/setup-go@v5)\",\"remediation\":\"pin with gh actions-lock\"}],\"workflows\":$W2}"
IMAGE="{\"valid\":true,\"findings\":[{\"workflow\":\".github/workflows/b.yml\",\"category\":\"container-image\",\"severity\":\"warning\",\"detail\":\"container image postgres:16-alpine is not pinned by digest\"}],\"workflows\":$W2}"
MOVED="{\"valid\":true,\"findings\":[{\"workflow\":\".github/workflows/a.yml\",\"category\":\"ref-moved\",\"severity\":\"warning\",\"dependency\":\"actions/checkout@v6\",\"detail\":\"v6 moved past the locked commit\"}],\"workflows\":$W2}"

echo "check-actions-lock.sh"

if [[ "$PINNED" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then pass "the pin is an exact release ($PINNED)"; else
  fail "the pin is not an exact release: '$PINNED'"
fi

fixture
stub "$PINNED" 0 "$CLEAN"; run_gate
expect_pass "a clean report passes" "2 workflows verified"
if grep -qF -- "--verify" "$STUB/args" && grep -qF -- "--json=valid,findings,workflows" "$STUB/args"; then
  pass "it runs the frozen --verify check with a JSON report"
else fail "it did not run --verify --json=valid,findings,workflows (args: $(cat "$STUB/args" 2>/dev/null))"; fi

GH_BIN="$WORK/no-such-gh" run_gate
expect_fail "no gh on PATH fails" "gh is not on PATH"

stub - 0 "$CLEAN"; run_gate
expect_fail "a missing extension fails" "extension is not installed"
expect_fail "it says how to install the pinned version" "--pin ${PINNED}"

stub v0.0.1 0 "$CLEAN"; run_gate
expect_fail "another extension version fails" "gh-actions-lock ${PINNED} is required"

stub "$PINNED" 2 -; run_gate
expect_fail "a crash fails" "it crashed or refused to run"

stub "$PINNED" 0 -; run_gate
expect_fail "an exit 0 with no report fails" "printed no JSON report"

stub "$PINNED" 0 "{\"valid\":false,\"findings\":[],\"workflows\":$W2}"; run_gate
expect_fail "a report disagreeing with exit 0 fails" "but the extension exited 0"

stub "$PINNED" 1 "$CLEAN"; run_gate
expect_fail "a valid report with exit 1 fails" "but the extension exited 1"

stub "$PINNED" 1 "{\"valid\":false,\"findings\":[],\"workflows\":$W2}"; run_gate
expect_fail "an invalid report naming no error fails" "names no error finding"

stub "$PINNED" 1 "$NOT_PINNED"; run_gate
expect_fail "a not-pinned finding fails" "not-pinned: .github/workflows/a.yml actions/setup-go@v5"
expect_fail "it prints the relock command" "gh actions-lock --no-narrow"

stub "$PINNED" 0 "$IMAGE"; run_gate
expect_fail "an undigested job image fails although the extension exits 0" "container-image: .github/workflows/b.yml"

stub "$PINNED" 0 "{\"valid\":true,\"findings\":[],\"workflows\":$W1}"; run_gate
expect_fail "a report covering fewer workflows than exist fails" "covers 1 workflows but .github/workflows holds 2"

stub "$PINNED" 0 '{"valid":true}'; run_gate
expect_fail "a report without workflows or findings fails" "lacks a boolean"

stub "$PINNED" 0 "$MOVED"; run_gate
expect_pass "a moved floating ref passes" "1 warning(s) above"
expect_pass "it prints the moved ref" "ref-moved: .github/workflows/a.yml actions/checkout@v6"

stub "$PINNED" 0 "$CLEAN"
mkdir -p "$FIX/.claude" "$FIX/.agents"
ln -s ../.savoir/platform/skills "$FIX/.claude/skills"; track; run_gate
expect_fail "a symlink dangling in the archive fails" "symlink .claude/skills -> ../.savoir/platform/skills dangles"
expect_fail "it names the export-ignore fix" "export-ignore"

printf '.claude/skills export-ignore\n' >"$FIX/.gitattributes"; track; run_gate
expect_pass "an export-ignored dangling symlink passes" "2 workflows verified"

ln -s ../.claude/skills "$FIX/.agents/skills"; track; run_gate
expect_fail "a symlink reaching an export-ignored one fails" "symlink .agents/skills -> ../.claude/skills dangles"

rm "$FIX/.agents/skills"; ln -s ../.github/workflows "$FIX/.agents/workflows"; track; run_gate
expect_pass "a symlink to a tracked directory passes" "2 workflows verified"

# git archive drops everything under an export-ignored directory, though
# check-attr on one of its files answers unspecified, in both rule spellings.
mkdir -p "$FIX/docs"; echo x >"$FIX/docs/f.md"; ln -s ../docs/f.md "$FIX/.agents/doc"
printf '.claude/skills export-ignore\ndocs/ export-ignore\n' >"$FIX/.gitattributes"; track; run_gate
expect_fail "a symlink into an export-ignored directory/ fails" "symlink .agents/doc -> ../docs/f.md dangles"
printf '.claude/skills export-ignore\ndocs export-ignore\n' >"$FIX/.gitattributes"; track; run_gate
expect_fail "a symlink into an export-ignored directory fails" "symlink .agents/doc -> ../docs/f.md dangles"
rm "$FIX/.agents/doc"

# The archive is cut from what is committed, so a rule only the work tree holds
# keeps nothing out of it.
ln -s ../.savoir/platform/rules "$FIX/.claude/rules"; track
printf '.claude/skills export-ignore\n.claude/rules export-ignore\n' >"$FIX/.gitattributes"; run_gate
expect_fail "an export-ignore line the index does not hold does not count" "symlink .claude/rules -> ../.savoir/platform/rules dangles"
track; run_gate
expect_pass "the same line once staged passes" "2 workflows verified"

rm -rf "$FIX/.git"; git -C "$FIX" init -q; run_gate
expect_fail "an index that tracks no file fails" "tracks no file, so the archive check judged nothing"

rm -rf "$FIX/.git"; run_gate
expect_fail "a root that is not a git work tree fails" "is not a git work tree"
fixture

rm "$FIX/.github/workflows/actions.lock"
stub "$PINNED" 0 "$CLEAN"; run_gate
expect_fail "a missing lockfile fails" "actions.lock is missing"

rm -f "$FIX"/.github/workflows/*
run_gate
expect_fail "no workflow at all fails" "the check verified nothing"

# A parity check between two artefacts nothing at runtime joins: the version
# the lane installs and the version the gate pins and judges against.
WORKFLOW="$REPO_ROOT/.github/workflows/actions-lock.yml"
if grep -qF -- "gh extension install github/gh-actions-lock --pin ${PINNED}" "$WORKFLOW"; then
  pass "actions-lock.yml installs gh-actions-lock ${PINNED}, the version the gate pins"
else
  fail "actions-lock.yml does not install gh-actions-lock ${PINNED} (found: $(grep -o 'gh-actions-lock --pin [^ ]*' "$WORKFLOW" | head -1))"
fi

echo ""
if [ "$failures" -gt 0 ]; then
  echo "❌ $failures case(s) failed"
  exit 1
fi
echo "✅ all check-actions-lock.sh cases passed"
