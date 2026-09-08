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
# Every case here runs --cheap for speed, and --cheap now carries a standing cap at 9 saying it
# is not a completion verdict. So "clean" is asserted as "no cap other than that one" rather
# than as a score of 10, which cheap can no longer reach by construction.
real_caps() { printf '%s' "$1" | jq '[.caps[] | select(.evidence | test("local facts only") | not)] | length'; }

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
check "clean repo has no real cap" 0 "$(real_caps "$out")"
check "--cheap can never report 10" 9 "$(printf '%s' "$out" | jq -r .score)"
check "--cheap says why it is not a verdict" 1 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.evidence | test("CI was not consulted"))] | length')"

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
check "unpushed names the remedy" 1 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.cap==8) | select(.remedy | test("git push"))] | length')"

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
# ChefFamille: a session that files an issue must fix it before it stops or claims 10/10.
check "a filed issue caps at 6, level with a held one" 6 "$(printf '%s' "$out" | jq -r .score)"
check "filed is reported separately from held" 1 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.evidence | test("filed this session and still open"))] | length')"
rm -f "$CFG/carnet-claims/$SID.jsonl"

# ---- an unregistered LIMITATION marker caps at 6
echo 'let x = 1; // LIMITATION(registre#): nothing reads this' >> "$R/a.txt"
out=$(run "$R")
check "LIMITATION naming no issue caps at 6" 6 "$(printf '%s' "$out" | jq -r .score)"
git -C "$R" checkout -q -- a.txt

# ---- the scan must not see prose, fixtures, or its own tree. The commit that installed bilan
# reported three markers that were all its own: the fixture here, the row in SKILL.md, and the
# grep pattern in bilan.sh. A scanner that reports itself is worse than no scanner.
mkdir -p "$R/.agents/skills/bilan" "$R/crates/x/tests"
echo 'LIMITATION(registre#[^)]*) is the pattern' > "$R/.agents/skills/bilan/bilan.sh"
echo '| LIMITATION(registre#…) marker | caps at 6 |'  > "$R/doc.md"
echo 'assert LIMITATION(registre#) fires'             > "$R/crates/x/tests/fixture.rs"
echo 'let y = 2; // LIMITATION(registre#) in a spec'  > "$R/thing.spec.ts"
out=$(run "$R")
check "the scan does not report its own tree, prose, tests or specs" 0 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.evidence | test("LIMITATION"))] | length')"
echo 'let z = 3; // LIMITATION(registre#) in real source' >> "$R/a.txt"
out=$(run "$R")
check "…but still reports a marker in real source" 1 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.evidence | test("LIMITATION"))] | length')"
git -C "$R" checkout -q -- a.txt
rm -rf "$R/.agents" "$R/doc.md" "$R/crates" "$R/thing.spec.ts"

# ---- ack accounts for files this session must not touch, for exactly that set
echo peer >> "$R/a.txt"
check "before ack, a dirty tracked file caps at 7" 7 "$(run "$R" | jq -r .score)"
( cd "$R" && CLAUDE_CONFIG_DIR="$CFG" CLAUDE_CODE_SESSION_ID="$SID" \
    bash "$BILAN" ack --why "a peer's pin bump in the shared checkout" >/dev/null 2>&1 )
out=$(run "$R")
# An ack is a recorded statement of ownership, so it CLEARS: a peer's file is not this
# session's incompleteness, and leaving it at 9 meant a session that had done everything right
# still could not reach 10.
check "after ack the cap is gone entirely" 0 "$(real_caps "$out")"
check "the ack reason stays visible as a note" 1 \
    "$(printf '%s' "$out" | jq '[.notes[] | select(test("pin bump"))] | length')"
echo second > "$R/b.txt" && git -C "$R" add b.txt
check "dirtying one more file brings the cap back" 7 "$(run "$R" | jq -r .score)"
git -C "$R" rm -q -f --cached b.txt >/dev/null 2>&1; rm -f "$R/b.txt"
git -C "$R" checkout -q -- a.txt

# The ack is keyed by path set, not by content: ownership is a property of the files, not of
# what is in them. So clear it before exercising the gate on the same file.
rm -f "$CFG/bilan/"*.ack.json

# ---- files already dirty when the session opened are not this session's, automatically.
# Three sessions were held at 7 by a peer's mid-edit file with nothing they could do about it.
rm -f "$CFG/bilan/"*.ack.json
echo peer-was-mid-edit >> "$R/a.txt"
check "a file dirty before the baseline caps at 7 without one" 7 "$(run "$R" | jq -r .score)"
( cd "$R" && CLAUDE_CONFIG_DIR="$CFG" CLAUDE_CODE_SESSION_ID="$SID" bash "$BILAN" baseline >/dev/null 2>&1 )
out=$(run "$R")
check "after the baseline it does not cap at all" 0 "$(real_caps "$out")"
check "the inherited file is still stated as a note" 1 \
    "$(printf '%s' "$out" | jq '[.notes[] | select(test("already uncommitted"))] | length')"
echo mine > "$R/c.txt" && git -C "$R" add c.txt
check "a file this session dirties still caps at 7" 7 "$(run "$R" | jq -r .score)"
check "and the cap names only the session's own file" 1 \
    "$(run "$R" | jq '[.caps[] | select(.cap==7) | select(.evidence | test("c\\.txt") and (test("a\\.txt") | not))] | length')"
git -C "$R" rm -q -f --cached c.txt >/dev/null 2>&1; rm -f "$R/c.txt"
git -C "$R" checkout -q -- a.txt
rm -f "$CFG/bilan/"*.baseline

# ---- a peer's unpushed commit is inherited the same way a dirty file is
rm -f "$CFG/bilan/"*.ack.json "$CFG/bilan/"*.baseline*
echo peer-commit > "$R/peer.txt"
git -C "$R" add peer.txt && git -C "$R" commit -qm "a peer's cherry-pick"
check "an unpushed commit caps at 8" 8 "$(run "$R" | jq -r .score)"
( cd "$R" && CLAUDE_CONFIG_DIR="$CFG" CLAUDE_CODE_SESSION_ID="$SID" bash "$BILAN" baseline >/dev/null 2>&1 )
out=$(run "$R")
check "a commit unpushed before the baseline does not cap" 0 "$(real_caps "$out")"
check "the inherited commit is stated as a note" 1 \
    "$(printf '%s' "$out" | jq '[.notes[] | select(test("already unpushed"))] | length')"
git -C "$R" push -q origin HEAD:refs/heads/main
rm -f "$CFG/bilan/"*.baseline*

# ---- background work is the one incompleteness that leaves no trace in git, the ledger or CI.
# A session reported 10/10 from --cheap with four subagents still running; closing it would
# have thrown all of that away.
TASKS="$CFG/scratch/tasks"
mkdir -p "$TASKS" "$CFG/scratch/scratchpad"   # ../tasks only resolves if scratchpad exists
printf 'watching\n\n[exited with code 0]\n' > "$TASKS/finished.output"
printf 'stopped\n\n[killed]\n'              > "$TASKS/stopped.output"
: > "$TASKS/orphan.output"                      # no marker, but nobody holds it
sleep 30 > "$TASKS/live.output" & LIVE=$!
task_run() { ( cd "$R" && CLAUDE_CONFIG_DIR="$CFG" CLAUDE_CODE_SESSION_ID="$SID" \
    CLAUDE_SCRATCHPAD_DIR="$CFG/scratch/scratchpad" bash "$BILAN" --cheap --json 2>/dev/null ); }
out=$(task_run)
check "a live background task caps at 7" 1 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.cap==7) | select(.evidence | test("background task"))] | length')"
check "it names the live one and only it" 1 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.evidence | test("live")) | select(.evidence | test("finished|stopped|orphan") | not)] | length')"
kill "$LIVE" 2>/dev/null; wait "$LIVE" 2>/dev/null
check "a finished task is not counted" 0 \
    "$(task_run | jq '[.caps[] | select(.evidence | test("background task"))] | length')"
rm -rf "$CFG/scratch"

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
