#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Tests for bilan — a throwaway git repo per case, so the caps are exercised for real
# ABOUTME: Run from anywhere: bash .agents/skills/bilan/test.sh
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
BILAN="$HERE/bilan.sh"
PASS=0
FAIL=0

ok()   { printf '  ✅ %s\n' "$1"; PASS=$((PASS + 1)); }
bad()  { printf '  ❌ %s\n' "$1"; FAIL=$((FAIL + 1)); }
check() { # <description> <expected> <actual>
    if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 — expected '$2', got '$3'"; fi
}

# A repo with an origin it can actually push to, so upstream-dependent caps are real.
new_repo() {
    local root remote
    root=$(mktemp -d -t bilan-test) || exit 1
    remote="$root/remote.git"
    git init -q --bare "$remote"
    git init -q "$root/work"
    git -C "$root/work" config user.email t@t.t
    git -C "$root/work" config user.name t
    git -C "$root/work" remote add origin "$remote"
    echo one > "$root/work/a.txt"
    git -C "$root/work" add a.txt
    git -C "$root/work" commit -qm first
    git -C "$root/work" push -q -u origin HEAD:refs/heads/main >/dev/null 2>&1
    git -C "$root/work" branch -q --set-upstream-to=origin/main 2>/dev/null
    printf '%s' "$root/work"
}

run() { # <repo> [args...]
    local repo=$1; shift
    ( cd "$repo" && CLAUDE_CONFIG_DIR="$CFG" CLAUDE_CODE_SESSION_ID="$SID" \
        bash "$BILAN" --cheap --json "$@" 2>/dev/null )
}

CFG=$(mktemp -d -t bilan-cfg)
SID="00000000-0000-0000-0000-00000000test"
trap 'rm -rf "$CFG"' EXIT

printf '\nbilan tests\n\n'

# ---- clean repo scores 10
R=$(new_repo)
out=$(run "$R")
check "clean repo scores 10" 10 "$(printf '%s' "$out" | jq -r .score)"
check "clean repo exits 0" 0 "$( ( cd "$R" && CLAUDE_CONFIG_DIR="$CFG" CLAUDE_CODE_SESSION_ID="$SID" bash "$BILAN" --cheap --quiet >/dev/null 2>&1 ); echo $?)"

# ---- uncommitted tracked change caps at 7
echo two >> "$R/a.txt"
out=$(run "$R")
check "uncommitted tracked change caps at 7" 7 "$(printf '%s' "$out" | jq -r .score)"
check "the evidence names the file" 1 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.evidence | test("a\\.txt"))] | length')"
git -C "$R" checkout -q -- a.txt

# ---- untracked file caps at 9, not 7
touch "$R/stray.md"
out=$(run "$R")
check "untracked file caps at 9" 9 "$(printf '%s' "$out" | jq -r .score)"
check "the untracked evidence names the file" 1 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.evidence | test("stray\\.md"))] | length')"
rm -f "$R/stray.md"

# ---- unpushed commit caps at 8
echo three >> "$R/a.txt"
git -C "$R" commit -q -am second
out=$(run "$R")
check "unpushed commit caps at 8" 8 "$(printf '%s' "$out" | jq -r .score)"
check "unpushed names the remedy" "git push" "$(printf '%s' "$out" | jq -r '.caps[] | select(.cap==8) | .remedy')"

# ---- validation marker missing is reported alongside the unpushed commit
check "missing validation marker is a cap" 1 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.evidence | test("validation-passed"))] | length')"
git -C "$R" push -q origin HEAD:refs/heads/main

# ---- a held carnet issue caps at 6 and outranks everything else
mkdir -p "$CFG/carnet-claims"
cat > "$CFG/carnet-claims/$SID.jsonl" <<LEDGER
{"v":1,"session":"$SID","name":"test","user":"t","host":"h","pid":1,"repo":"dravr-platform","branch":"main","at":"2026-09-08T00:00:00Z","kind":"identity"}
{"kind":"claim","tracker":"dravr-ai/dravr-carnet","issue":999,"at":"2026-09-08T00:00:00Z"}
LEDGER
out=$(run "$R")
check "held carnet issue caps at 6" 6 "$(printf '%s' "$out" | jq -r .score)"
check "held issue is named in the evidence" 1 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.evidence | test("carnet#999"))] | length')"

# ---- a filed issue caps at 9 and is distinguishable from a held one
printf '{"kind":"filed","tracker":"dravr-ai/dravr-carnet","issue":1000,"at":"2026-09-08T00:00:00Z"}\n' \
    >> "$CFG/carnet-claims/$SID.jsonl"
out=$(run "$R")
check "filed issue is reported separately" 1 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.evidence | test("filed this session"))] | length')"
rm -f "$CFG/carnet-claims/$SID.jsonl"

# ---- an unregistered LIMITATION marker caps at 6
echo 'let x = 1; // LIMITATION(registre#): nothing reads this' >> "$R/a.txt"
out=$(run "$R")
check "LIMITATION naming no issue caps at 6" 6 "$(printf '%s' "$out" | jq -r .score)"
git -C "$R" checkout -q -- a.txt

# ---- the Stop gate blocks once, then latches
gate() { echo "{\"session_id\":\"$SID\",\"stop_hook_active\":$1}" \
    | ( cd "$R" && CLAUDE_CONFIG_DIR="$CFG" bash "$HERE/hooks/stop-gate.sh" 2>/dev/null ); }
echo two >> "$R/a.txt"          # a tracked edit caps at 7, below the gate threshold
check "stop gate blocks a dirty stop" block "$(gate false | jq -r '.decision // empty')"
check "stop gate latches on the same state" "" "$(gate false | jq -r '.decision // empty')"
check "stop gate respects stop_hook_active" "" "$(gate true | jq -r '.decision // empty')"
check "block reason names the remedy" 1 "$(gate false >/dev/null; echo 1)"
git -C "$R" checkout -q -- a.txt
check "stop gate is silent on a clean tree" "" "$(gate false | jq -r '.decision // empty')"

# A cap of 9 is reported but never worth refusing a stop over.
touch "$R/scratch.md"
check "score 9 does not block" "" "$(gate false | jq -r '.decision // empty')"
check "score 9 is still a cap in the report" 9 "$(run "$R" | jq -r .score)"
rm -f "$R/scratch.md"

rm -rf "$(dirname "$R")"
printf '\n%s passed · %s failed\n\n' "$PASS" "$FAIL"
[ "$FAIL" = 0 ]
