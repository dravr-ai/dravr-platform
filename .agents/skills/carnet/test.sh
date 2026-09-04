#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Tests for carnet.sh and its two hooks against a stub gh — every refusal path is made to fire
# ABOUTME: No network, no real tracker: run `skills/carnet/test.sh` from anywhere
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
carnet="$here/carnet.sh"
tmp=$(mktemp -d)
peer_pid=""
cleanup() { [ -z "$peer_pid" ] || kill "$peer_pid" 2>/dev/null || true; rm -rf "$tmp"; }
trap cleanup EXIT

# ------------------------------------------------------------------ stub gh
# Reads come from canned files under $CARNET_STUB; every call is appended to calls.log, with
# the content of any --body-file captured because carnet.sh deletes that temp file afterwards.
mkdir -p "$tmp/bin" "$tmp/stub"
cat > "$tmp/bin/gh" <<'EOF'
#!/usr/bin/env bash
S=$CARNET_STUB
body=""; prev=""
for a in "$@"; do [ "$prev" = "--body-file" ] && body=$(cat "$a"); prev=$a; done
{ printf 'CALL %s\n' "$*"; [ -z "$body" ] || printf 'BODY %s\n' "$body"; printf 'END\n'; } >> "$S/calls.log"
case "$1 $2" in
    "api user")                 printf 'tester\n' ;;
    "api repos/"*"/comments")   cat "$S/comments.txt" 2>/dev/null || true ;;
    "issue view")               cat "$S/issue.json" ;;
    "issue list")               cat "$S/list.txt" 2>/dev/null || true ;;
    "issue create")             printf 'https://github.com/dravr-ai/dravr-carnet/issues/321\n' ;;
    "repo view")                cat "$S/private.txt" 2>/dev/null || printf 'true\n' ;;
    *) : ;;
esac
exit 0
EOF
chmod +x "$tmp/bin/gh"
export PATH="$tmp/bin:$PATH"
export CARNET_STUB="$tmp/stub"
S=$CARNET_STUB

# ------------------------------------------------------------------ fake repo + session
git init -q "$tmp/repo"
git -C "$tmp/repo" -c user.name=t -c user.email=t@t.t commit -q --allow-empty -m init
git -C "$tmp/repo" branch -M main
git -C "$tmp/repo" remote add origin git@github.com:dravr-ai/dravr-test.git
printf 'tracker = "dravr-ai/dravr-carnet"\n' > "$tmp/repo/registre.toml"
cd "$tmp/repo"
head_sha=$(git rev-parse HEAD)

export CLAUDE_CONFIG_DIR="$tmp/cfg"
export CLAUDE_CODE_SESSION_ID="11111111-aaaa-bbbb-cccc-000000000001"
export CLAUDE_PID=$$
mkdir -p "$tmp/cfg/sessions"
printf '{"pid":%s,"sessionId":"%s","name":"TestSession"}\n' "$$" "$CLAUDE_CODE_SESSION_ID" > "$tmp/cfg/sessions/$$.json"
HOST=$(hostname -s 2>/dev/null || hostname)
ME=$CLAUDE_CODE_SESSION_ID
PEER="22222222-aaaa-bbbb-cccc-000000000002"
DEAD="33333333-aaaa-bbbb-cccc-000000000003"
ledger="$tmp/cfg/carnet-claims/$ME.jsonl"

# A live peer session on this host: a real process plus its sessions file.
sleep 600 & peer_pid=$!
printf '{"pid":%s,"sessionId":"%s","name":"PeerSession"}\n' "$peer_pid" "$PEER" > "$tmp/cfg/sessions/$peer_pid.json"

# ------------------------------------------------------------------ fixtures
issue() { # <state> <labels-json> <assignees-json>
    printf '{"number":42,"title":"[test] Thing","url":"https://github.com/dravr-ai/dravr-carnet/issues/42","state":"%s","labels":%s,"assignees":%s}\n' \
        "$1" "$2" "$3" > "$S/issue.json"
}
issue_open()   { issue OPEN '[{"name":"dravr-test"}]' '[]'; }
issue_held()   { issue OPEN '[{"name":"dravr-test"},{"name":"in-progress"}]' '[{"login":"tester"}]'; }
issue_closed() { issue CLOSED '[]' '[]'; }

claim_marker() { # <session> <name> <user> <host> <pid>
    printf '<!-- carnet-claim {"v":1,"session":"%s","name":"%s","user":"%s","host":"%s","pid":%s,"repo":"dravr-test","branch":"main","at":"2026-09-02T10:00:00Z"} -->\n🔒 Claimed by @%s\n' \
        "$1" "$2" "$3" "$4" "$5" "$3"
}
release_marker() { # <session>
    printf '<!-- carnet-release {"v":1,"session":"%s","name":"x","user":"x","host":"x","pid":1,"repo":"dravr-test","branch":"main","at":"2026-09-02T11:00:00Z","reason":"done"} -->\n🔓 Released\n' "$1"
}
comments() { cat > "$S/comments.txt"; }
reset() { : > "$S/calls.log"; : > "$S/comments.txt"; rm -rf "$tmp/cfg/carnet-claims"; rm -f "$S/private.txt" "$S/list.txt"; issue_open; }

# ------------------------------------------------------------------ assertions
pass=0; fail=0
ok()  { pass=$((pass + 1)); printf '  ✓ %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf '  ✗ %s\n' "$1"; }
assert_eq()   { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 — expected '$3', got '$2'"; fi; }
assert_grep() { # <desc> <regex> <file>
    if grep -qE -- "$2" "$3"; then ok "$1"; else bad "$1 — no /$2/ in $(basename "$3")"; sed 's/^/      | /' "$3" | head -20; fi
}
assert_no_grep() { if grep -qE -- "$2" "$3"; then bad "$1 — found /$2/ in $(basename "$3")"; else ok "$1"; fi; }
count_calls() { grep -cE "^CALL $1" "$S/calls.log" || true; }
WRITES='issue (edit|comment|close|create)'

run_carnet() { # args... ; sets rc, writes $tmp/out and $tmp/err
    rc=0
    bash "$carnet" "$@" > "$tmp/out" 2> "$tmp/err" || rc=$?
}

section() { printf '\n%s\n' "$1"; }

# ================================================================== syntax
section "syntax"
for f in "$carnet" "$here"/hooks/*.sh "$here/test.sh"; do
    if bash -n "$f"; then ok "bash -n $(basename "$f")"; else bad "bash -n $(basename "$f")"; fi
done
if command -v shellcheck >/dev/null 2>&1; then
    if shellcheck -S warning "$carnet" "$here"/hooks/*.sh; then ok "shellcheck"; else bad "shellcheck"; fi
fi

# ================================================================== claim
section "claim"
reset
run_carnet claim 42
assert_eq "claim on an unclaimed issue exits 0" "$rc" 0
assert_grep "assigns me and adds the label" 'issue edit 42 -R dravr-ai/dravr-carnet --add-assignee tester --add-label in-progress' "$S/calls.log"
assert_grep "posts a claim marker" '^BODY <!-- carnet-claim \{"v":1,"session":"11111111' "$S/calls.log"
assert_grep "marker carries the session name" '"name":"TestSession"' "$S/calls.log"
assert_grep "marker carries repo and branch" '"repo":"dravr-test","branch":"main"' "$S/calls.log"
assert_grep "ledger records the claim" '"kind":"claim","tracker":"dravr-ai/dravr-carnet","issue":42' "$ledger"
assert_grep "ledger starts with the identity" '"kind":"identity"' "$ledger"

reset
comments < <(claim_marker "$ME" TestSession tester "$HOST" "$$")
run_carnet claim 42
assert_eq "claim on my own claim is a no-op" "$rc" 0
assert_grep "says so" "already held by this session" "$tmp/out"
assert_eq "no tracker write" "$(count_calls 'issue edit')" 0

reset
comments < <(claim_marker "$PEER" PeerSession peer "$HOST" "$peer_pid")
run_carnet claim 42
assert_eq "refuses a claim held by a live session on this host" "$rc" 2
assert_grep "names the holder" "held by @peer · session PeerSession \(22222222\) on $HOST, still running" "$tmp/err"
assert_eq "no tracker write when refused" "$(count_calls 'issue edit')" 0
run_carnet claim 42 --steal
assert_eq "--steal overrides" "$rc" 0
assert_grep "warns locally" "stealing carnet#42 from live session PeerSession" "$tmp/err"
assert_grep "the comment says it was stolen from a live session" "Stolen from live session \*\*PeerSession\*\*" "$S/calls.log"
assert_grep "the displaced human is unassigned" 'issue edit 42 -R dravr-ai/dravr-carnet --remove-assignee peer' "$S/calls.log"

reset
comments < <(claim_marker "$DEAD" GoneSession peer "$HOST" 999999)
run_carnet claim 42
assert_eq "takes over a claim whose session ended on this host" "$rc" 0
assert_grep "the comment says it took over" "Took over from ended session \*\*GoneSession\*\*" "$S/calls.log"

reset
comments < <(claim_marker "$PEER" RemoteSession phil elsewhere 4242)
run_carnet claim 42
assert_eq "refuses a claim held on another host" "$rc" 2
assert_grep "explains liveness is unknowable" "on host elsewhere since .* liveness cannot be checked from $HOST" "$tmp/err"
run_carnet claim 42 --steal
assert_eq "--steal takes it" "$rc" 0
assert_grep "the comment warns the displaced session" "Stolen from session \*\*RemoteSession\*\* \(\`22222222\`\) of @phil on elsewhere" "$S/calls.log"

reset
comments < <(claim_marker "$PEER" PeerSession peer "$HOST" "$peer_pid"; release_marker "$PEER")
run_carnet claim 42
assert_eq "a released claim no longer holds" "$rc" 0
assert_no_grep "no takeover text after a release" "Took over|Stolen" "$S/calls.log"

reset
issue_closed
run_carnet claim 42
assert_eq "cannot claim a closed issue" "$rc" 1
assert_grep "says closed" "is closed" "$tmp/err"

reset
run_carnet claim 42 --dry-run
assert_eq "dry-run exits 0" "$rc" 0
assert_eq "dry-run writes nothing" "$(count_calls "$WRITES")" 0
assert_grep "dry-run prints the edit" '\[dry-run\] gh issue edit 42' "$tmp/err"
[ -f "$ledger" ] && bad "dry-run must not touch the ledger" || ok "dry-run leaves no ledger"

# ================================================================== release
section "release"
reset
comments < <(claim_marker "$ME" TestSession tester "$HOST" "$$")
run_carnet claim 42 >/dev/null 2>&1 || true   # not held per marker? it is — no-op, but seed the ledger by hand
mkdir -p "$tmp/cfg/carnet-claims"
printf '{"kind":"identity","v":1,"session":"%s","name":"TestSession","user":"tester","host":"%s","pid":%s}\n{"kind":"claim","tracker":"dravr-ai/dravr-carnet","issue":42,"at":"2026-09-02T10:00:00Z"}\n' "$ME" "$HOST" "$$" > "$ledger"
: > "$S/calls.log"
run_carnet release --all --reason session-ended
assert_eq "release --all exits 0" "$rc" 0
assert_grep "removes label and assignee" 'issue edit 42 -R dravr-ai/dravr-carnet --remove-label in-progress --remove-assignee tester' "$S/calls.log"
assert_grep "posts a release marker with the reason" '^BODY <!-- carnet-release \{.*"reason":"session-ended"' "$S/calls.log"
[ -f "$ledger" ] && bad "ledger removed once empty" || ok "ledger removed once empty"

reset
run_carnet release 42
assert_eq "releasing something not held fails" "$rc" 1
assert_eq "without writing" "$(count_calls 'issue edit')" 0

reset
comments < <(claim_marker "$PEER" PeerSession peer "$HOST" "$peer_pid")
run_carnet release 42
assert_eq "cannot release another session's claim" "$rc" 2
assert_grep "points at claim --steal" "claim 42 --steal" "$tmp/err"

# ================================================================== close
section "close"
reset
run_carnet close 42
assert_eq "close without --why fails" "$rc" 1
assert_grep "says why" "close needs --why" "$tmp/err"
assert_eq "and writes nothing" "$(count_calls "$WRITES")" 0

reset
run_carnet close 42 --why "fixed in the seeder" --commit "$head_sha"
assert_eq "close with a reason exits 0" "$rc" 0
assert_grep "the comment carries the reason" '\*\*Why:\*\* fixed in the seeder' "$S/calls.log"
assert_grep "and the commit URL on the code repo" "https://github.com/dravr-ai/dravr-test/commit/$head_sha" "$S/calls.log"
assert_grep "the comment is also a release marker" '^BODY <!-- carnet-release \{.*"reason":"closed"' "$S/calls.log"
assert_grep "then closes" '^CALL issue close 42 -R dravr-ai/dravr-carnet' "$S/calls.log"

reset
run_carnet close 42 --why x --commit deadbeef
assert_eq "an unknown commit is refused" "$rc" 1
assert_grep "names the sha" "commit deadbeef is not in this repository" "$tmp/err"

reset
issue_held
comments < <(claim_marker "$PEER" PeerSession peer "$HOST" "$peer_pid")
run_carnet close 42 --why x
assert_eq "cannot close an issue another session holds" "$rc" 2

reset
issue_held
comments < <(claim_marker "$ME" TestSession tester "$HOST" "$$")
run_carnet close 42 --why "done"
assert_eq "closing my own held issue works" "$rc" 0
assert_grep "and drops label + assignee first" '--remove-label in-progress --remove-assignee tester' "$S/calls.log"

# ================================================================== create
section "create"
reset
run_carnet create --title "Thing is wrong" --label limitation --body "where / what / fix"
assert_eq "create exits 0" "$rc" 0
assert_grep "title gets the project prefix" 'issue create -R dravr-ai/dravr-carnet --title \[test\] Thing is wrong' "$S/calls.log"
assert_grep "project label always, extra labels after" '--label dravr-test --label limitation' "$S/calls.log"
assert_grep "prints the URL" "issues/321" "$tmp/out"
assert_grep "prints the marker hint for a limitation" 'LIMITATION\(registre#321\)' "$tmp/out"
assert_grep "checked the tracker is private" '^CALL repo view dravr-ai/dravr-carnet --json isPrivate' "$S/calls.log"

reset
run_carnet create --title "[platform] Already prefixed" --body b
assert_grep "an existing prefix is kept" '--title \[platform\] Already prefixed --body-file' "$S/calls.log"

reset
printf 'false\n' > "$S/private.txt"
run_carnet create --title T --body b
assert_eq "a public tracker is refused" "$rc" 1
assert_eq "and nothing is filed" "$(count_calls 'issue create')" 0

reset
run_carnet create --title T < /dev/null
assert_eq "an empty body is refused" "$rc" 1

reset
run_carnet create --title "With claim" --body b --claim
assert_eq "create --claim exits 0" "$rc" 0
assert_grep "and claims the new number" 'issue edit 321 -R dravr-ai/dravr-carnet --add-assignee tester --add-label in-progress' "$S/calls.log"

# ================================================================== label
section "label"
reset
run_carnet label 42 +critical -bug
assert_eq "label exits 0" "$rc" 0
assert_grep "adds and removes" 'issue edit 42 -R dravr-ai/dravr-carnet --add-label critical --remove-label bug' "$S/calls.log"

# ================================================================== status
section "status"
reset
run_carnet status 42 --short
assert_grep "unclaimed" '^carnet#42 · open · unclaimed · \[test\] Thing' "$tmp/out"
issue_held
run_carnet status 42 --short
assert_grep "a label without a marker is called stale" 'stale in-progress label' "$tmp/out"
comments < <(claim_marker "$ME" TestSession tester "$HOST" "$$")
run_carnet status 42 --short
assert_grep "my own claim" 'held by THIS session \(TestSession\)' "$tmp/out"
comments < <(claim_marker "$PEER" PeerSession peer "$HOST" "$peer_pid")
run_carnet status 42 --short
assert_grep "a live peer" "held by @peer · session PeerSession \(22222222\) on $HOST \[running\] · main" "$tmp/out"
comments < <(claim_marker "$DEAD" GoneSession peer "$HOST" 999999)
run_carnet status 42 --short
assert_grep "an ended peer" '\[session ended' "$tmp/out"
comments < <(claim_marker "$PEER" R phil elsewhere 1)
run_carnet status 42 --short
assert_grep "another host" '\[other host\]' "$tmp/out"
issue_closed
run_carnet status 42 --short
assert_grep "closed" '^carnet#42 · closed' "$tmp/out"

reset
printf '42\n' > "$S/list.txt"
run_carnet status
assert_grep "status with no number lists in-progress issues" '^carnet#42' "$tmp/out"
: > "$S/list.txt"
run_carnet status
assert_grep "and says when nothing is" 'no issue in dravr-ai/dravr-carnet is in progress' "$tmp/out"

# ================================================================== mine
section "mine"
reset
run_carnet mine
assert_grep "empty ledger" 'holds nothing' "$tmp/out"
run_carnet claim 42 >/dev/null 2>&1
run_carnet mine
assert_grep "lists the held issue without an API call" 'carnet#42 · since' "$tmp/out"

# A RESUMED session gets a new id and a new, empty ledger. The previous
# incarnation's claims stay in its own file, so `mine` used to answer "holds
# nothing" — a false all-clear, and exactly when someone is auditing.
reset
prior="$tmp/cfg/carnet-claims/$DEAD.jsonl"
mkdir -p "$tmp/cfg/carnet-claims"
printf '{"kind":"identity","v":1,"session":"%s","name":"EarlierMe","user":"tester","host":"%s","pid":1,"repo":"r","branch":"main","at":"2026-01-01T00:00:00Z"}\n' "$DEAD" "$HOST" > "$prior"
printf '{"kind":"claim","tracker":"dravr-ai/dravr-carnet","issue":91,"at":"2026-01-01T00:00:00Z"}\n' >> "$prior"
run_carnet mine
assert_grep "an empty ledger still says so" 'holds nothing' "$tmp/out"
assert_grep "but a prior session id's claims are surfaced" 'other session ids on this machine still list claims' "$tmp/out"
assert_grep "named, with the issue" 'EarlierMe \(33333333\): 91' "$tmp/out"
assert_grep "and not asserted as mine" 'NOT necessarily yours' "$tmp/out"
assert_eq "surfacing them costs no API call" "$(count_calls 'issue view')" 0

# A ledger belonging to someone else, or another machine, is not a candidate for
# "an earlier me" — adopting a peer's claim would be worse than the false
# all-clear this fixes.
reset
foreign="$tmp/cfg/carnet-claims/$PEER.jsonl"
mkdir -p "$tmp/cfg/carnet-claims"
printf '{"kind":"identity","v":1,"session":"%s","name":"SomeoneElse","user":"other","host":"%s","pid":1,"repo":"r","branch":"main","at":"2026-01-01T00:00:00Z"}\n' "$PEER" "$HOST" > "$foreign"
printf '{"kind":"claim","tracker":"dravr-ai/dravr-carnet","issue":92,"at":"2026-01-01T00:00:00Z"}\n' >> "$foreign"
run_carnet mine
assert_no_grep "another user's ledger is not surfaced" 'SomeoneElse' "$tmp/out"

reset
elsewhere="$tmp/cfg/carnet-claims/$PEER.jsonl"
mkdir -p "$tmp/cfg/carnet-claims"
printf '{"kind":"identity","v":1,"session":"%s","name":"OtherBox","user":"tester","host":"not-this-host","pid":1,"repo":"r","branch":"main","at":"2026-01-01T00:00:00Z"}\n' "$PEER" > "$elsewhere"
printf '{"kind":"claim","tracker":"dravr-ai/dravr-carnet","issue":93,"at":"2026-01-01T00:00:00Z"}\n' >> "$elsewhere"
run_carnet mine
assert_no_grep "another machine's ledger is not surfaced" 'OtherBox' "$tmp/out"

# ================================================================== hooks
section "hooks"
reset
hook_out=$(printf '{"prompt":"look at carnet#42, then registre#42 and https://github.com/dravr-ai/dravr-carnet/issues/7","session_id":"%s"}' "$ME" \
    | bash "$here/hooks/prompt-status.sh")
printf '%s\n' "$hook_out" > "$tmp/hook"
assert_eq "prompt hook: one line per distinct issue" "$(grep -c '^carnet#' "$tmp/hook")" 2
assert_grep "prompt hook: resolves carnet#42" '^carnet#42 · open · unclaimed' "$tmp/hook"
assert_grep "prompt hook: resolves the URL form" '^carnet#7 ' "$tmp/hook"
views_before=$(count_calls 'issue view')
printf '{"prompt":"carnet#42 again"}' | bash "$here/hooks/prompt-status.sh" > "$tmp/hook2"
assert_eq "prompt hook: a repeat within a minute is served from cache" "$(count_calls 'issue view')" "$views_before"
assert_grep "prompt hook: cached line still printed" '^carnet#42' "$tmp/hook2"
printf '{"prompt":"nothing about the register"}' | bash "$here/hooks/prompt-status.sh" > "$tmp/hook3"
assert_eq "prompt hook: silent when no issue is named" "$(wc -c < "$tmp/hook3" | tr -d ' ')" 0

reset
mkdir -p "$tmp/cfg/carnet-claims"
printf '{"kind":"identity","v":1,"session":"%s","name":"Ender","user":"ender","host":"%s","pid":1}\n{"kind":"claim","tracker":"dravr-ai/dravr-carnet","issue":42,"at":"2026-09-02T10:00:00Z"}\n' "$PEER" "$HOST" > "$tmp/cfg/carnet-claims/$PEER.jsonl"
comments < <(claim_marker "$PEER" Ender ender "$HOST" 1)
printf '{"session_id":"%s","reason":"exit"}' "$PEER" | bash "$here/hooks/session-end-release.sh" > "$tmp/hook4"
assert_grep "session-end hook: reports the release" '^🔓 carnet#42 released \(session-ended\)' "$tmp/hook4"
assert_grep "session-end hook: releases with the ledger's identity" 'issue edit 42 -R dravr-ai/dravr-carnet --remove-label in-progress --remove-assignee ender' "$S/calls.log"
assert_grep "session-end hook: marker names the ended session" "^BODY <!-- carnet-release \{.*\"session\":\"$PEER\".*\"reason\":\"session-ended\"" "$S/calls.log"
[ -f "$tmp/cfg/carnet-claims/$PEER.jsonl" ] && bad "session-end hook: ledger removed" || ok "session-end hook: ledger removed"
: > "$S/calls.log"
printf '{"session_id":"%s"}' "$DEAD" | bash "$here/hooks/session-end-release.sh"
assert_eq "session-end hook: no ledger, no call" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0

# ================================================================== auto-claim
section "auto-claim (PreToolUse)"

auto_claim() { # <payload> ; sets rc, $tmp/ac.out, $tmp/ac.err
    rc=0
    printf '%s' "$1" | bash "$here/hooks/auto-claim.sh" > "$tmp/ac.out" 2> "$tmp/ac.err" || rc=$?
}
pending_dir="$tmp/cfg/carnet-claims/pending"
set_pending() { mkdir -p "$pending_dir"; printf '%s\n' "$@" > "$pending_dir/$ME.txt"; }
edit_payload() { printf '{"tool_name":"Edit","session_id":"%s","tool_input":{"file_path":"/x"}}' "$ME"; }
bash_payload() { printf '{"tool_name":"Bash","session_id":"%s","tool_input":{"command":"%s"}}' "$ME" "$1"; }

# Nothing pending is the common case and must cost nothing at all.
reset; rm -rf "$pending_dir"
auto_claim "$(edit_payload)"
assert_eq "no pending list: allows the tool" "$rc" 0
assert_eq "no pending list: makes no call" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0

# The prompt hook is what fills the list.
reset; rm -rf "$pending_dir"
printf '{"prompt":"work carnet#42 please","session_id":"%s"}' "$ME" | bash "$here/hooks/prompt-status.sh" >/dev/null
assert_grep "prompt hook records the issue as pending" '^42$' "$pending_dir/$ME.txt"
printf '{"prompt":"nothing about the register","session_id":"%s"}' "$ME" | bash "$here/hooks/prompt-status.sh" >/dev/null
assert_grep "a prompt naming none leaves the list alone" '^42$' "$pending_dir/$ME.txt"

# An edit claims it, without anyone asking.
reset; set_pending 42
auto_claim "$(edit_payload)"
assert_eq "an edit claims the pending issue" "$rc" 0
assert_grep "and says so" '^🔒 carnet auto-claimed: 42' "$tmp/ac.out"
assert_grep "the claim reached the tracker" 'issue edit 42 -R dravr-ai/dravr-carnet --add-assignee tester --add-label in-progress' "$S/calls.log"
assert_grep "the marker names this session" "^BODY <!-- carnet-claim \{.*\"session\":\"$ME\"" "$S/calls.log"

# Consumed once: a second edit is free.
: > "$S/calls.log"
auto_claim "$(edit_payload)"
assert_eq "the list is consumed, so a later edit makes no call" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0

# Reading is not working.
reset; set_pending 42
auto_claim "$(printf '{"tool_name":"Read","session_id":"%s"}' "$ME")"
assert_eq "a read tool claims nothing" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0
auto_claim "$(bash_payload 'grep -rn TODO src/')"
assert_eq "a read-shaped Bash claims nothing" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0
auto_claim "$(bash_payload 'git status --short')"
assert_eq "git status claims nothing" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0

# Suppressing stderr is the commonest idiom in a read, and `2>/dev/null` contains
# a `>`. Classifying it as a write made every quiet read claim — four issues were
# taken that way in a day, one of them a peer's, off a message that only NAMED it.
auto_claim "$(bash_payload 'git log --oneline -1 abc123 2>/dev/null')"
assert_eq "a read that suppresses stderr claims nothing" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0
auto_claim "$(bash_payload 'gh run list --json status -q ".[]" 2>/dev/null')"
assert_eq "gh with 2>/dev/null claims nothing" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0
auto_claim "$(bash_payload 'ls -la >/dev/null')"
assert_eq "stdout to /dev/null claims nothing" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0
auto_claim "$(bash_payload 'make check &>/dev/null')"
assert_eq "&>/dev/null claims nothing" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0

# A git verb needs a terminator, or `merge` matches inside `git merge-base` —
# the standard "is this commit on main?" query — and a pure read claims. That
# took carnet#323 off a peer who was mid-investigation and stood down for it.
auto_claim "$(bash_payload 'git merge-base --is-ancestor abc123 origin/main')"
assert_eq "git merge-base claims nothing" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0
auto_claim "$(bash_payload 'git merge-base --fork-point main')"
assert_eq "git merge-base --fork-point claims nothing" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0
auto_claim "$(bash_payload 'git merge-base --is-ancestor abc origin/main 2>/dev/null && echo yes')"
assert_eq "the exact command that mis-claimed #323 claims nothing" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0

# Reporting-only flags write nothing, whatever the verb.
auto_claim "$(bash_payload 'git add --dry-run .')"
assert_eq "git add --dry-run claims nothing" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0
auto_claim "$(bash_payload 'git apply --check my.patch')"
assert_eq "git apply --check claims nothing" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0

# ...but a redirect into a real file is still an edit, /dev/null nearby or not.
reset; set_pending 42
auto_claim "$(bash_payload 'grep -rn TODO src/ 2>/dev/null > findings.txt')"
assert_grep "a real redirect still claims, even beside 2>/dev/null" 'issue edit 42 .*--add-label in-progress' "$S/calls.log"

# A write-shaped Bash command is an edit — this session edits through bash.
reset; set_pending 42
auto_claim "$(bash_payload "sed -i '' s/a/b/ f.txt")"
assert_grep "sed -i claims" 'issue edit 42 .*--add-label in-progress' "$S/calls.log"
reset; set_pending 42
auto_claim "$(bash_payload 'cat > note.txt <<EOT')"
assert_grep "a redirect into a file claims" 'issue edit 42 .*--add-label in-progress' "$S/calls.log"
reset; set_pending 42
auto_claim "$(bash_payload 'git commit -m wip')"
assert_grep "git commit claims" 'issue edit 42 .*--add-label in-progress' "$S/calls.log"
reset; set_pending 42
auto_claim "$(bash_payload 'git merge origin/main')"
assert_grep "a real git merge still claims" 'issue edit 42 .*--add-label in-progress' "$S/calls.log"
reset; set_pending 42
auto_claim "$(bash_payload 'git add -A')"
assert_grep "git add still claims" 'issue edit 42 .*--add-label in-progress' "$S/calls.log"
reset; set_pending 42
auto_claim "$(bash_payload 'git apply my.patch')"
assert_grep "git apply without --check still claims" 'issue edit 42 .*--add-label in-progress' "$S/calls.log"
reset; set_pending 42
auto_claim "$(bash_payload 'git reset --hard origin/main')"
assert_grep "git reset still claims" 'issue edit 42 .*--add-label in-progress' "$S/calls.log"

# A live peer holding it blocks the edit once, and names them.
reset; set_pending 42
comments < <(claim_marker "$PEER" PeerSession peer "$HOST" "$peer_pid")
auto_claim "$(edit_payload)"
assert_eq "a live peer's claim blocks the edit" "$rc" 2
assert_grep "the block names the holder" 'PeerSession' "$tmp/ac.err"
assert_grep "the block says not to duplicate" 'Do not do this work twice' "$tmp/ac.err"
assert_no_grep "and steals nothing" 'add-label in-progress' "$S/calls.log"
set_pending 42
auto_claim "$(edit_payload)"
assert_eq "having said it once, it stops blocking" "$rc" 0

# A list nobody acted on goes stale rather than claiming much later.
reset; set_pending 42
touch -t 202001010000 "$pending_dir/$ME.txt"
auto_claim "$(edit_payload)"
assert_eq "an hour-old list is dropped, not claimed" "$(wc -c < "$S/calls.log" | tr -d ' ')" 0
[ -f "$pending_dir/$ME.txt" ] && bad "stale list removed" || ok "stale list removed"

# ================================================================== summary
printf '\n%s passed, %s failed\n' "$pass" "$fail"
[ "$fail" = 0 ]
