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
real_caps() { printf '%s' "$1" | jq '[.caps[]] | length'; }

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
    git -C "$root/work" fetch -q origin 2>/dev/null   # FETCH_HEAD, or every case reads as stale
    printf '%s' "$root/work"
}

# SessionStart runs this at t=0 against whatever state the checkout is in, so every test that
# measures a change has to establish the starting point first — otherwise the first bilan run
# creates the baseline itself and correctly treats the change as inherited.
baseline_now() { ( cd "$1" && CLAUDE_CONFIG_DIR="$CFG" CLAUDE_CODE_SESSION_ID="$SID" \
    bash "$BILAN" baseline >/dev/null 2>&1 ); }

run() { # <repo> [args...]
    local repo=$1; shift
    ( cd "$repo" && CLAUDE_CONFIG_DIR="$CFG" CLAUDE_CODE_SESSION_ID="$SID" \
        bash "$BILAN" --cheap --json "$@" 2>/dev/null )
}

CFG=$(mktemp -d -t bilan-cfg)
SID="00000000-0000-0000-0000-00000000test"
trap 'rm -rf "$CFG"' EXIT

# Every fixture repo starts with nothing committed by the "session", so check_measurable would
# fire on every case. A completed todo makes the harness measurable; the unmeasured case has its
# own block below, where it is the thing under test.
measurable() {
    mkdir -p "$CFG/tasks/$SID"
    printf '{"status":"completed","subject":"harness"}\n' > "$CFG/tasks/$SID/0.json"
}

printf '\nbilan tests\n\n'
measurable

# ---- clean repo scores 10
R=$(new_repo)
baseline_now "$R"
out=$(run "$R")
check "clean repo has no real cap" 0 "$(real_caps "$out")"
# --cheap used to carry a standing cap at 9 because it skipped CI. CI no longer scores at all,
# so both paths give the same number and that cap only penalised the cheap one.
check "--cheap and the full run agree on the number" 10 "$(printf '%s' "$out" | jq -r .score)"
check "no standing cap for skipping CI" 0 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.evidence | test("CI"))] | length')"

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

# ---- a commit made after the baseline is this session's, and caps at 8
baseline_now "$R"
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
baseline_now "$R"                       # observing from here: the commit below is this session's
echo peer-commit > "$R/peer.txt"
git -C "$R" add peer.txt && git -C "$R" commit -qm "a peer's cherry-pick"
check "a commit made after the baseline caps at 8" 8 "$(run "$R" | jq -r .score)"
baseline_now "$R"                       # now re-observe: the same commit is pre-existing
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

# ---- the deliverable, not just the repo. A session doing research or writing a document
# commits nothing and holds no issue, so every repo check comes back clean; its own todo list is
# the only thing that knows it is not finished.
TD="$CFG/tasks/$SID"
mkdir -p "$TD"
printf '{"status":"completed","subject":"read the synthesis"}\n'      > "$TD/1.json"
printf '{"status":"in_progress","subject":"publish the artifact"}\n'  > "$TD/2.json"
out=$(run "$R")
check "an open todo caps at 7" 1 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.cap==7) | select(.evidence | test("todo"))] | length')"
check "it names the unfinished one, not the done one" 1 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.evidence | test("publish the artifact")) | select(.evidence | test("read the synthesis") | not)] | length')"
printf '{"status":"completed","subject":"publish the artifact"}\n'    > "$TD/2.json"
check "all todos done means no cap" 0 \
    "$(run "$R" | jq '[.caps[] | select(.evidence | test("todo"))] | length')"
rm -rf "$CFG/tasks"

# ---- a session that checked nothing must not report a verdict. This is the case that scored
# 10/10 with its artifact unwritten: research and writing touch no commit, no issue and no CI,
# so every check came back clean because every check came back empty.
rm -rf "$CFG/tasks"; rm -f "$CFG/bilan/"*.baseline*
baseline_now "$R"
out=$(run "$R")
check "no commit and no todo means unmeasured, not 10" 9 "$(printf '%s' "$out" | jq -r .score)"
check "and it says why" 1 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.evidence | test("nothing measurable"))] | length')"
mkdir -p "$CFG/tasks/$SID"
printf '{"status":"completed","subject":"published the artifact"}\n' > "$CFG/tasks/$SID/1.json"
check "a declared todo makes the session measurable" 0 \
    "$(run "$R" | jq '[.caps[] | select(.evidence | test("nothing measurable"))] | length')"
rm -rf "$CFG/tasks"
echo measurable > "$R/m.txt" && git -C "$R" add m.txt && git -C "$R" commit -qm "a commit"
check "a commit makes the session measurable" 0 \
    "$(run "$R" | jq '[.caps[] | select(.evidence | test("nothing measurable"))] | length')"
git -C "$R" push -q origin HEAD:refs/heads/main
rm -f "$CFG/bilan/"*.baseline*

# ---- the opening ask is carried into the report, so completion is claimed against the request
mkdir -p "$CFG/projects/fixture"
TX="$CFG/projects/fixture/$SID.jsonl"
printf '%s\n' '{"type":"user","message":{"content":"<system-reminder>ignore me</system-reminder>"}}' > "$TX"
printf '%s\n' '{"type":"user","message":{"content":[{"type":"text","text":"Build the thing that measures completion"}]}}' >> "$TX"
check "the report carries the opening ask" 1 \
    "$( ( cd "$R" && CLAUDE_CONFIG_DIR="$CFG" CLAUDE_CODE_SESSION_ID="$SID" bash "$BILAN" --cheap 2>/dev/null ) | grep -c 'asked: Build the thing')"
check "and skips the system-reminder that precedes it" 0 \
    "$( ( cd "$R" && CLAUDE_CONFIG_DIR="$CFG" CLAUDE_CODE_SESSION_ID="$SID" bash "$BILAN" --cheap 2>/dev/null ) | grep -c 'ignore me')"
rm -rf "$CFG/projects"

# ---- a peer's dev stack in the shared checkout is not this session's to stop. Third instance
# of the same hazard: pid files are the CHECKOUT's, and dev_owned only asks whether the process
# is alive, never who started it. ack had no channel for it, so the baseline gets one.
mkdir -p "$R/logs" "$R/bin"
# Mirrors the real library, including its self-resolving default — without that default a
# stub silently answers about the wrong directory and every dev-stack case passes vacuously.
cat > "$R/bin/dev-processes.sh" <<'LIB'
DEV_PROJECT_ROOT="${DEV_PROJECT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)}"
dev_pid_file() { echo "$DEV_PROJECT_ROOT/logs/$1.pid"; }
dev_owned() { [ -r "$(dev_pid_file "$1")" ] && head -1 "$(dev_pid_file "$1")"; }
LIB
printf '%s\n' "$$" > "$R/logs/peer-server.pid"      # already running when the session opens
rm -f "$CFG/bilan/"*.baseline*
baseline_now "$R"
out=$(run "$R")
check "a stack running before the session does not cap" 0 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.evidence | test("dev stack"))] | length')"
check "it is stated as a peer's instead" 1 \
    "$(printf '%s' "$out" | jq '[.notes[] | select(test("since before this session opened"))] | length')"
printf '%s\n' "$$" > "$R/logs/mine-server.pid"      # started after the baseline
out=$(run "$R")
check "a stack this session started does cap at 9" 1 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.cap==9) | select(.evidence | test("this session started"))] | length')"
check "and names only the new one" 0 \
    "$(printf '%s' "$out" | jq '[.caps[] | select(.evidence | test("this session started")) | select(.evidence | test("peer-server"))] | length')"
rm -rf "$R/logs" "$R/bin"; rm -f "$CFG/bilan/"*.baseline*
baseline_now "$R"        # leave a clean starting point: with none, the next case's own edit
                         # is created before the baseline and correctly reads as inherited

# ---- the sweep must not report a dead session's claim on an issue that is already closed.
# carnet#236 was closed on 2026-09-03 and MCPNext's ledger still named it, so every session
# start since reported an abandoned issue that no longer existed — a recurring false alarm
# trains the reader to skip the one line the sweep exists to print.
sweep_ledger="$CFG/carnet-claims/11111111-1111-1111-1111-111111111111.jsonl"
mkdir -p "$(dirname "$sweep_ledger")"
cat > "$sweep_ledger" <<LEDGER
{"v":1,"session":"11111111-1111-1111-1111-111111111111","name":"Dead","user":"t","host":"h","pid":999999,"repo":"r","branch":"main","at":"2026-09-03T11:07:02Z","kind":"identity"}
{"kind":"claim","tracker":"dravr-ai/dravr-carnet","issue":236,"at":"2026-09-03T11:07:02Z"}
LEDGER
# No gh in the fixture, so the tracker cannot be consulted: the claim must still be REPORTED
# rather than silently dropped — absence of a verdict is not a closed issue.
sweep_out=$( ( cd "$R" && CLAUDE_CONFIG_DIR="$CFG" PATH=/usr/bin:/bin bash "$BILAN" sweep 2>/dev/null ) )
check "an unverifiable claim is still reported, not dropped" 1 \
    "$(printf '%s' "$sweep_out" | grep -c 'carnet#236')"
check "and the ledger is left intact when it cannot be checked" 1 \
    "$(grep -c '"issue":236' "$sweep_ledger")"
rm -f "$sweep_ledger"

# A worktree someone is still working in is not abandoned work. A session's own cwd is not the
# signal — every session here sits in the main checkout and reaches a worktree by path — so
# liveness is a live process inside it, or a file edited recently.
live_dir=$(mktemp -d -t bilan-live)
touch "$live_dir/just-edited.txt"
check "a directory edited moments ago reads as in use" 0 \
    "$( ( cd "$R" && CLAUDE_CONFIG_DIR="$CFG" bash -c "source <(sed -n '/^worktree_is_live/,/^}/p' \"$BILAN\"); worktree_is_live \"$live_dir\"" ); echo $?)"
touch -t 202601010000 "$live_dir/just-edited.txt"
check "and one untouched for hours does not" 1 \
    "$( ( cd "$R" && CLAUDE_CONFIG_DIR="$CFG" bash -c "source <(sed -n '/^worktree_is_live/,/^}/p' \"$BILAN\"); worktree_is_live \"$live_dir\"" ); echo $?)"
rm -rf "$live_dir"

# ---- the status line has room for one phrase and it must name the thing to act on.
# ".agents/skills/b" identified nothing, and the --cheap notice filled the line with a sentence
# that says only "this is not a verdict".
score_line() { cut -f3 "$CFG/bilan/$(printf '%s' "$SID" | tr -c 'a-zA-Z0-9._-' '_').score"; }
echo edited >> "$R/a.txt"
run "$R" >/dev/null
check "the published line names the file, not a cut path" 1 \
    "$(score_line | grep -c 'a\.txt')"
for n in alpha bravo charlie delta echo foxtrot golf hotel; do echo x > "$R/$n.txt"; git -C "$R" add "$n.txt"; done
run "$R" >/dev/null
check "a long list is cut on a word boundary, with an ellipsis" 1 \
    "$(score_line | grep -cE '[a-z]…$')"
check "and stays within the width the line has" 1 \
    "$([ "$(score_line | wc -c)" -le 60 ] && echo 1 || echo 0)"
git -C "$R" reset -q HEAD -- . ; rm -f "$R"/{alpha,bravo,charlie,delta,echo,foxtrot,golf,hotel}.txt
git -C "$R" checkout -q -- a.txt

# ---- CI on a head this session did not create is not this session's verdict. In the shared
# main worktree every session sits on the same tip, so one peer's red capped all ten at 5 and
# the gate blocked every one of them over a commit none of them made.
ci_head() { # <baseline-head> -> 1 when the session is graded on it, 0 when it is only stated
    [ -z "$1" ] && { printf 0; return; }
    [ "$1" = "deadbeef" ] && printf 0 || printf 1
}
check "a head unchanged since the session opened is not scored" 0 "$(ci_head deadbeef)"
check "a head this session moved is scored" 1 "$(ci_head abc1234)"
check "no baseline means bilan was not watching, so not scored" 0 "$(ci_head '')"

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

# A session must never be trapped. The per-state latch bounds repetition only while the state
# holds still; in a shared checkout a peer editing beside you makes a new signature every few
# minutes. Three tellings is the ceiling, then the gate stands down for good.
rm -f "$CFG/bilan/"*.json "$CFG/bilan/"*.baseline*
baseline_now "$R"
: > "$CFG/blocks.txt"
for i in 1 2 3 4 5; do
    # A different file each round, so each is a genuinely new signature — churning one file
    # keeps the same path set and the per-state latch alone would silence it.
    echo "churn" > "$R/churn-$i.txt" && git -C "$R" add "churn-$i.txt"
    d=$(gate false | jq -r '.decision // empty'); printf '%s\n' "${d:--}" >> "$CFG/blocks.txt"
done
# One block per session, ever. A block re-invokes the model on the whole conversation, so a
# second telling costs a full turn's tokens and adds nothing the first did not say.
check "the gate blocks once, then holds its cooldown" 1 \
    "$(grep -c '^block$' "$CFG/blocks.txt")"
check "and is silent for every attempt inside it" 4 \
    "$(grep -c '^-$' "$CFG/blocks.txt")"
# ...but it must NOT stand down forever: a session held carnet#384 for nineteen hours after its
# one and only block, and ChefFamille had to find it by hand.
st="$CFG/bilan/$(printf '%s' "$SID" | tr -c 'a-zA-Z0-9._-' '_').json"
check "the block was recorded with a timestamp" 1 "$([ -f "$st" ] && jq -e 'has("blockedAt")' "$st" >/dev/null && echo 1 || echo 0)"
old=$(python3 -c "import datetime;print((datetime.datetime.now(datetime.timezone.utc)-datetime.timedelta(hours=1)).strftime('%Y-%m-%dT%H:%M:%SZ'))")
jq --arg a "$old" '.blockedAt = $a' "$st" > "$st.tmp" && mv "$st.tmp" "$st"
check "after the cooldown expires it blocks again" block "$(gate false | jq -r '.decision // empty')"
git -C "$R" reset -q HEAD -- . 2>/dev/null; rm -f "$R"/churn-*.txt

# A cap of 9 is reported but never worth refusing a stop over.
touch "$R/scratch.md"
check "score 9 does not block" "" "$(gate false | jq -r '.decision // empty')"
check "score 9 is still a cap in the report" 9 "$(run "$R" | jq -r .score)"
rm -f "$R/scratch.md"

rm -rf "$(dirname "$R")"
printf '\n%s passed · %s failed\n\n' "$PASS" "$FAIL"
[ "$FAIL" = 0 ]
