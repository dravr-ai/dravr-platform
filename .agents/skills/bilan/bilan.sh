#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: bilan — the session's balance sheet: what this session left undone, measured not narrated
# ABOUTME: Every completion number this repo reports is this script's number, computed from git, the carnet ledger and CI
#
# The 0-10 completion number used to come from the model's story of the session, so a session
# that believed it was done said 10 while it still held open carnet issues. Every fact behind
# that number is machine-checkable, and this script checks it.
#
# The score is min() over caps. A cap is a fact that makes "done" false; each one prints its own
# evidence and its own remedy, so the number is never a verdict without a reason.
#
# Portability: shared through the repo like carnet.sh — macOS bash 3.2 (no associative arrays,
# no mapfile), BSD sed, jq, git. `gh` is used only in full mode; --cheap touches no network,
# because the Stop hook runs on the hot path of every turn and hosts kill a hook at ~10s.
set -uo pipefail

CHEAP=0
JSON=0
QUIET=0

usage() {
    cat <<'EOF'
bilan — what this session left undone

  bilan.sh [--cheap] [--json] [--session <uuid>]   score this session in this checkout
  bilan.sh sweep                                    what dead sessions left across every worktree
  bilan.sh ack --why "<whose and why>"              declare uncommitted files not this session's
  bilan.sh baseline                                 record what was already dirty (SessionStart)

  --cheap     local facts only: no gh, no network (what the Stop hook runs)
  --json      machine form: {"score":N,"caps":[…],"friction":{…}}
  --quiet     print nothing when the score is 10

Exit: 0 = 10/10 · 1 = incomplete · 2 = error
EOF
}

# ------------------------------------------------------------------ output helpers
say()  { [ "$JSON" = 1 ] || printf '%s\n' "$*"; }
die()  { printf '❌ %s\n' "$1" >&2; exit 2; }
now()  { date -u +%Y-%m-%dT%H:%M:%SZ; }

command -v git >/dev/null 2>&1 || die "git is required"
command -v jq  >/dev/null 2>&1 || die "jq is required"

# ------------------------------------------------------------------ context
CFG=${CLAUDE_CONFIG_DIR:-$HOME/.claude}
LEDGER_DIR="$CFG/carnet-claims"
SESSION_ID=${CLAUDE_CODE_SESSION_ID:-}
SESSION_PID=${CLAUDE_PID:-}

REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || die "run this inside a git checkout"
GIT_DIR=$(git rev-parse --git-dir 2>/dev/null)
BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
HEAD_SHA=$(git rev-parse HEAD 2>/dev/null || echo "")

# One line per cap: <cap>\t<icon>\t<evidence>\t<remedy>. A temp file rather than an array so
# the checks can run in subshells and bash 3.2 stays happy.
CAPS=$(mktemp -t bilan) || die "mktemp failed"
trap 'rm -f "$CAPS" "$NOTES"' EXIT

cap() { # <cap> <icon> <evidence> <remedy>
    printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" >> "$CAPS"
}

# A note is a fact worth printing that is NOT this session's incompleteness, so it never
# reaches the score. Without this channel the only way to mention something was to cap on it,
# which is how a peer's mid-edit file came to hold three sessions at 7 with nothing they could
# do about it.
NOTES=$(mktemp -t bilan-notes) || die "mktemp failed"
say_note() { printf '%s\n' "$1" >> "$NOTES"; }

# ------------------------------------------------------------------ session facts
ledger_file() { [ -n "$SESSION_ID" ] && printf '%s' "$LEDGER_DIR/$SESSION_ID.jsonl"; }

session_started_epoch() {
    local f
    f=$(ledger_file) || return 1
    [ -s "$f" ] || return 1
    # -u: the ledger stamp is UTC, and BSD date otherwise reads it as local time, which put
    # the session's start hours in the future and made every stash look older than the session.
    date -j -u -f '%Y-%m-%dT%H:%M:%SZ' "$(head -1 "$f" | jq -r '.at // empty')" +%s 2>/dev/null \
        || date -u -d "$(head -1 "$f" | jq -r '.at // empty')" +%s 2>/dev/null
}

transcript_path() {
    local slug
    [ -n "$SESSION_ID" ] || return 1
    slug=$(printf '%s' "$REPO_ROOT" | sed 's#[/.]#-#g')
    for d in "$CFG/projects/$slug" "$CFG/projects"/*; do
        [ -f "$d/$SESSION_ID.jsonl" ] && { printf '%s' "$d/$SESSION_ID.jsonl"; return 0; }
    done
    return 1
}

# ------------------------------------------------------------------ acknowledgement
# Ownership of an uncommitted file is NOT machine-decidable here, and that was tested rather
# than assumed: Claude Code records the paths a session touched in the transcript's
# `file-history-snapshot.trackedFileBackups`, but only for the Edit/Write tools. Sessions in
# this repo work Bash-first, so that map came back EMPTY for a session that had just written
# nine files — attribution would have called its own work a peer's and stopped blocking, which
# is the worst direction to be wrong in.
#
# So the judgement stays with the session, and `ack` is what makes it cost one line instead of
# a paragraph on every run. It is keyed to the exact set of files: dirty one more and the cap
# comes back. It never applies to anything but the shared-checkout case, because every other
# cap is about state this session can actually change.
ack_file() { [ -n "$SESSION_ID" ] && printf '%s' "$CFG/bilan/$(printf '%s' "$SESSION_ID" | tr -c 'a-zA-Z0-9._-' '_').ack.json"; }

signature_of() { printf '%s' "$1" | sort | shasum 2>/dev/null | cut -d' ' -f1; }

ack_reason_for() { # <field> <signature>
    local f
    f=$(ack_file) || return 1
    [ -f "$f" ] || return 1
    jq -r --arg k "$1" --arg s "$2" 'select(.[$k] == $s) | .why // empty' "$f" 2>/dev/null | grep . || return 1
}

# Written by the SessionStart hook. A path already dirty when the session opened belongs to
# whoever dirtied it, which was not this session — the one piece of ownership that IS decidable.
# Paths, not content: a peer who keeps editing the same file is still that peer, and treating a
# changed hash as "now mine" would hand their work back to the cap it was meant to escape.
baseline_file() { [ -n "$SESSION_ID" ] && printf '%s' "$CFG/bilan/$(printf '%s' "$SESSION_ID" | tr -c 'a-zA-Z0-9._-' '_').baseline"; }

# A session with NO baseline cannot attribute anything, and defaulting to "it is all yours" is
# how one unpushed commit in the shared checkout came to block every other session in the fleet
# over work none of them had done — a session doing design-only research was told to push two
# commits it had never made. When the baseline is missing entirely, bilan has not been watching
# this session, so it starts watching now: the state it finds on its first run is the state it
# inherited. It can only ever measure change from the moment it began observing, and claiming
# otherwise is what did the damage.
ensure_baseline() {
    local f
    f=$(baseline_file) || return 0
    [ -f "$f" ] && return 0
    cmd_baseline >/dev/null 2>&1
    say_note "no baseline existed for this session — everything already uncommitted or unpushed is treated as inherited, and only changes from here are this session's"
}

cmd_baseline() {
    local f u up
    f=$(baseline_file) || return 0
    mkdir -p "$(dirname "$f")" 2>/dev/null || return 0
    git status --porcelain 2>/dev/null | grep -v '^??' | sed 's/^...//' > "$f"
    # Commits carry the same inheritance as files: a peer's cherry-pick sitting unpushed in the
    # shared checkout when this session opened is not this session's to push.
    up=$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || true)
    if [ -n "$up" ]; then
        git rev-list "$up..HEAD" 2>/dev/null > "${f}.commits"
    else
        : > "${f}.commits"
    fi
    # HEAD at session start, so "did this session commit anything at all" is answerable.
    git rev-parse HEAD 2>/dev/null > "${f}.head" || : > "${f}.head"
    u=$(grep -c . "${f}.commits" 2>/dev/null); u=${u:-0}
    local d; d=$(grep -c . "$f" 2>/dev/null); d=${d:-0}
    say "baseline: $d file(s) dirty and $u commit(s) unpushed at session start"
    return 0
}

# Split a newline-separated path list into what this session must answer for and what it
# inherited. A session that opened into someone else's mid-edit answers for neither.
not_mine() { # <paths>  -> prints the inherited subset
    local f
    f=$(baseline_file) || return 0
    [ -s "$f" ] || return 0
    printf '%s\n' "$1" | grep -Fxf "$f" 2>/dev/null || true
}

not_mine_commits() { # <shas> -> prints the inherited subset
    local f
    f=$(baseline_file) || return 0
    [ -s "${f}.commits" ] || return 0
    printf '%s\n' "$1" | grep -Fxf "${f}.commits" 2>/dev/null || true
}

cmd_ack() { # <why>
    local why=$1 f tracked commits up sig csig
    [ -n "$why" ] || die "ack needs --why: say whose work this is and why you are leaving it"
    f=$(ack_file) || die "not inside a Claude Code session"
    tracked=$(git status --porcelain 2>/dev/null | grep -v '^??' | sed 's/^...//')
    up=$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || true)
    commits=""
    [ -n "$up" ] && commits=$(git rev-list "$up..HEAD" 2>/dev/null || true)
    [ -n "$tracked$commits" ] || { say "nothing uncommitted or unpushed to account for"; return 0; }
    sig=$(signature_of "$tracked"); csig=$(signature_of "$commits")
    mkdir -p "$(dirname "$f")"
    jq -n --arg s "$sig" --arg c "$csig" --arg w "$why" --arg at "$(now)" \
       --arg files "$(printf '%s' "$tracked" | tr '\n' ' ')" \
       '{signature:$s, commit_signature:$c, why:$w, at:$at, files:$files}' > "$f"
    say "📌 accounted for: $(printf '%s\n' "$tracked" | grep -c .) file(s), $(printf '%s\n' "$commits" | grep -c .) commit(s) — $why"
    say "   this covers exactly that set; dirty one more file or make one more commit and the cap returns."
    return 0
}

# ------------------------------------------------------------------ checks · git
# Several sessions share the main worktree, so "5 tracked files modified" is not enough to act
# on: the first live block this gate ever issued was for a peer's embacle pin bump, and the
# count alone gave no way to see that. Name the files. Ownership is not decidable from here —
# most edits in this repo go through Bash, so the transcript's file_path arguments see only some
# of them, and mtime is not authorship — so the gate names what it found, blocks once, and
# leaves the judgement to the session.
name_files() { # <max> <newline-separated paths>
    local max=$1 list names count
    list=$(printf '%s\n' "$2" | grep -v '^$')
    count=$(printf '%s\n' "$list" | wc -l | tr -d ' ')
    names=$(printf '%s\n' "$list" | head -"$max" | tr '\n' ' ')
    if [ "$count" -gt "$max" ]; then
        printf '%s and %s more' "$names" "$((count - max))"
    else
        printf '%s' "${names% }"
    fi
}

check_worktree() {
    local porcelain tracked untracked inherited owned t_n u_n i_n why
    porcelain=$(git status --porcelain 2>/dev/null)
    [ -n "$porcelain" ] || return 0
    tracked=$(printf '%s\n' "$porcelain" | grep -v '^??' | sed 's/^...//')
    untracked=$(printf '%s\n' "$porcelain" | grep '^??' | sed 's/^...//')

    inherited=$(not_mine "$tracked")
    if [ -n "$inherited" ]; then
        owned=$(printf '%s\n' "$tracked" | grep -Fxv -f <(printf '%s\n' "$inherited") 2>/dev/null || true)
    else
        owned=$tracked
    fi
    i_n=$(printf '%s\n' "$inherited" | grep -cv '^$')
    t_n=$(printf '%s\n' "$owned" | grep -cv '^$')
    u_n=$(printf '%s\n' "$untracked" | grep -cv '^$')

    # Inherited dirt is stated, never scored. It is not this session's completion.
    [ "${i_n:-0}" -gt 0 ] && say_note "$i_n file(s) were already uncommitted when this session opened — not its work: $(name_files 5 "$inherited")"

    if [ "${t_n:-0}" -gt 0 ]; then
        # An ack is a recorded statement of ownership, so it CLEARS rather than softens: a
        # peer's file is not this session's incompleteness, and leaving it at 9 meant a session
        # that had done everything right still could not reach 10. It stays visible in every
        # later report, is keyed to the exact path set, and returns the moment one more file
        # goes dirty.
        if why=$(ack_reason_for signature "$(signature_of "$owned")"); then
            say_note "$t_n uncommitted file(s) declared not this session's: $why"
        else
            cap 7 "❌" "$t_n tracked file(s) modified and uncommitted: $(name_files 8 "$owned")" \
                  "commit them — or, if they are a peer's in this shared checkout, bilan.sh ack --why '…'"
        fi
    fi
    if [ "${u_n:-0}" -gt 0 ]; then
        cap 9 "⚠️" "$u_n untracked file(s): $(name_files 8 "$untracked")" \
              "add them, delete them, or move them to the scratchpad"
    fi
}

# Prints this session's own unpushed shas: everything ahead of upstream, minus what was already
# unpushed when the session opened, minus what an ack has declared. Empty means this session has
# nothing of its own waiting to go out — whatever else is sitting in the shared checkout.
owned_unpushed() {
    local upstream shas inherited owned
    upstream=$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || true)
    [ -n "$upstream" ] || return 0
    # The full run can settle it; the cheap run has to say it cannot.
    if [ "$CHEAP" = 0 ]; then
        git fetch -q origin 2>/dev/null || true
    elif [ "$(fetch_age)" -gt 300 ]; then
        stale=1
    fi
    shas=$(git rev-list "$upstream..HEAD" 2>/dev/null)
    [ -n "$shas" ] || return 0
    ack_reason_for commit_signature "$(signature_of "$shas")" >/dev/null 2>&1 && return 0
    inherited=$(not_mine_commits "$shas")
    if [ -n "$inherited" ]; then
        owned=$(printf '%s\n' "$shas" | grep -Fxv -f <(printf '%s\n' "$inherited") 2>/dev/null || true)
    else
        owned=$shas
    fi
    printf '%s' "$owned"
}

# Seconds since the last fetch, or a large number when there has never been one.
fetch_age() {
    local f="$GIT_DIR/FETCH_HEAD"
    [ -f "$f" ] || { printf '%s' 999999; return 0; }
    printf '%s' "$(( $(date +%s) - $(stat -f %m "$f" 2>/dev/null || stat -c %Y "$f" 2>/dev/null || echo 0) ))"
}

check_unpushed() {
    local upstream ahead shas inherited owned n why stale=0
    upstream=$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || true)
    if [ -z "$upstream" ]; then
        [ "$BRANCH" = main ] && return 0
        git rev-parse --verify -q origin/main >/dev/null 2>&1 || return 0
        ahead=$(git rev-list --count origin/main..HEAD 2>/dev/null || echo 0)
        [ "$ahead" -gt 0 ] || return 0
        cap 8 "❌" "branch $BRANCH has $ahead commit(s) and no upstream — never pushed" \
              "git push -u origin $BRANCH"
        return 0
    fi
    # The full run can settle it; the cheap run has to say it cannot.
    if [ "$CHEAP" = 0 ]; then
        git fetch -q origin 2>/dev/null || true
    elif [ "$(fetch_age)" -gt 300 ]; then
        stale=1
    fi
    shas=$(git rev-list "$upstream..HEAD" 2>/dev/null)
    [ -n "$shas" ] || return 0

    # A peer's commit already sitting unpushed when this session opened is not this session's
    # to push — the same inheritance the dirty-file baseline records, one level up. A peer's
    # cherry-pick landing mid-session held this session at 8 until ack covered it.
    inherited=$(not_mine_commits "$shas")
    if [ -n "$inherited" ]; then
        owned=$(printf '%s\n' "$shas" | grep -Fxv -f <(printf '%s\n' "$inherited") 2>/dev/null || true)
        say_note "$(printf '%s\n' "$inherited" | grep -c .) commit(s) were already unpushed when this session opened — not its work"
    else
        owned=$shas
    fi
    n=$(printf '%s\n' "$owned" | grep -cv '^$')
    [ "${n:-0}" -gt 0 ] || return 0

    if why=$(ack_reason_for commit_signature "$(signature_of "$shas")"); then
        say_note "$n unpushed commit(s) declared not this session's: $why"
        return 0
    fi
    if [ "$stale" = 1 ]; then
        cap 9 "⚠️" "$n commit(s) look unpushed, but origin/main was last fetched $(( $(fetch_age) / 60 ))m ago — they may already be on the remote" \
              "run bilan.sh without --cheap, which fetches first, before acting on this"
        return 0
    fi
    cap 8 "❌" "$n commit(s) not pushed to $upstream ($(printf '%s\n' "$owned" | cut -c1-8 | tr '\n' ' '))" \
          "git push — or, if they are a peer's in this shared checkout, bilan.sh ack --why '…'"
}

check_stash() {
    local start line ts count=0
    start=$(session_started_epoch 2>/dev/null) || return 0
    [ -n "$start" ] || return 0
    while IFS= read -r line; do
        ts=${line%% *}
        [ -n "$ts" ] || continue
        [ "$ts" -ge "$start" ] 2>/dev/null && count=$((count + 1))
    done <<< "$(git log -g --format='%ct %gs' refs/stash 2>/dev/null)"
    [ "$count" -gt 0 ] || return 0
    cap 9 "⚠️" "$count stash entr(y|ies) created during this session" \
          "apply or drop them — a stash left behind is invisible work"
}

# The reliable squash-merge tell: `git push origin --delete` is what clears the upstream, so a
# local branch whose upstream is gone is one the cleanup half-finished. A squash rewrites the
# sha, so `git branch --merged` cannot see it and is not used here.
check_branch_cleanup() {
    local gone
    gone=$(git branch -vv 2>/dev/null | grep ': gone\]' | awk '{print $1}' | tr -d '*' | tr '\n' ' ')
    [ -n "${gone// /}" ] || return 0
    cap 9 "⚠️" "local branch(es) whose upstream is deleted: ${gone% }" \
          "git branch -D ${gone% }"
}

check_validation_marker() {
    local marker epoch sha age
    # Only this session's own commits. A peer's inherited cherry-pick is not something this
    # session can or should validate, and capping on it was the third place the same category
    # error turned up.
    [ -n "$(owned_unpushed)" ] || return 0
    marker="$GIT_DIR/validation-passed"
    if [ ! -f "$marker" ]; then
        cap 9 "⚠️" "commits to push but no .git/validation-passed marker" \
              "./scripts/ci/pre-push-validate.sh"
        return 0
    fi
    read -r epoch sha < "$marker"
    if [ "$sha" != "$HEAD_SHA" ]; then
        cap 9 "⚠️" "validation marker is for ${sha:0:8}, HEAD is ${HEAD_SHA:0:8}" \
              "./scripts/ci/pre-push-validate.sh (it pins the sha)"
        return 0
    fi
    age=$(( $(date +%s) - epoch ))
    [ "$age" -le 900 ] || cap 9 "⚠️" "validation marker is $((age / 60))m old (TTL 15m)" \
                                "./scripts/ci/pre-push-validate.sh"
}

# ------------------------------------------------------------------ checks · carnet
check_carnet_held() {
    local f n list=""
    f=$(ledger_file) || return 0
    [ -s "$f" ] || return 0
    for n in $(jq -r 'select(.kind == "claim") | .issue' "$f" 2>/dev/null); do
        list="$list carnet#$n"
    done
    [ -n "${list// /}" ] || return 0
    cap 6 "❌" "still holding${list} — claimed by this session, neither closed nor released" \
          "carnet.sh close <n> --why … --commit <sha>, or release <n> --reason …"
}

# `create` writes a "filed" line and `close` removes it, so what remains is what this session
# opened and did not fix. That caps at 6 — level with an issue still held, and below the Stop
# gate's threshold, so the session cannot quietly walk away from issues it opened.
#
# The standing rule is fix first and file only the residue; this is what makes the second half
# of it visible. If something genuinely cannot be fixed here, that is a decision to put in front
# of ChefFamille, not a cap to slip past.
check_carnet_filed() {
    local f n list="" state
    f=$(ledger_file) || return 0
    [ -s "$f" ] || return 0
    for n in $(jq -r 'select(.kind == "filed") | .issue' "$f" 2>/dev/null); do
        # Someone else may have closed it. Only the full run can tell; --cheap keeps the cap,
        # which is the safe direction for a rule about not walking away from your own issues.
        if [ "$CHEAP" = 0 ] && command -v gh >/dev/null 2>&1; then
            state=$(gh issue view "$n" -R "${REGISTRE_TRACKER:-dravr-ai/dravr-carnet}" \
                    --json state -q .state 2>/dev/null || echo OPEN)
            [ "$state" = CLOSED ] && continue
        fi
        list="$list carnet#$n"
    done
    [ -n "${list// /}" ] || return 0
    cap 6 "❌" "filed this session and still open:${list}" \
          "fix them and close with carnet.sh close <n> --why … --commit <sha> — a session does not file its way out of work"
}

# A LIMITATION marker is the sanctioned way to ship a gap, but only when it names a live issue.
# A marker registers a gap at the declaration of the limited item, in source. It never lives in
# prose or in a fixture — so those paths are not scanned, and the first thing excluded is this
# skill's own tree. The commit that installed bilan made it report three markers that were all
# its own artifacts: the fixture in test.sh, the table row in SKILL.md, and the grep pattern on
# the line below. A scanner that cannot see itself is the whole point; the same self-match cost
# the friction counter its accuracy an hour earlier.
#
# The loose `[^)]*` is deliberate and is why the exclusions have to carry the weight: the
# repo's own gate matches `registre#[0-9]+` and therefore cannot see a marker that names no
# issue at all, which is exactly the case worth failing on.
LIMITATION_SKIP=':(exclude).agents/skills/bilan/**
:(exclude)*.md
:(exclude)**/tests/**
:(exclude)**/__tests__/**
:(exclude)*.test.*
:(exclude)*.spec.*'

limitation_diff() { # <range-or-empty>
    local skip=()
    while IFS= read -r p; do [ -n "$p" ] && skip+=("$p"); done <<< "$LIMITATION_SKIP"
    if [ -n "$1" ]; then
        git diff -U0 "$1" -- . "${skip[@]}" 2>/dev/null
    else
        git diff -U0 HEAD -- . "${skip[@]}" 2>/dev/null
    fi
}

check_limitation_markers() {
    local upstream added marker n bad=""
    upstream=$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || echo origin/main)
    # Both halves of the session's work: committed but unpushed, and still in the tree.
    added=$( { limitation_diff "$upstream..HEAD"; limitation_diff ""; } \
             | grep '^+' | grep -o 'LIMITATION(registre#[^)]*)' | sort -u )
    [ -n "$added" ] || return 0
    while IFS= read -r marker; do
        [ -n "$marker" ] || continue
        n=$(printf '%s' "$marker" | sed 's/[^0-9]//g')
        if [ -z "$n" ] || [ "$n" = 0 ]; then
            bad="$bad $marker"
        elif [ "$CHEAP" = 0 ] && command -v gh >/dev/null 2>&1; then
            gh issue view "$n" -R "${REGISTRE_TRACKER:-dravr-ai/dravr-carnet}" --json number \
                >/dev/null 2>&1 || bad="$bad #$n(no such issue)"
        fi
    done <<< "$added"
    [ -n "${bad// /}" ] || return 0
    cap 6 "❌" "LIMITATION marker(s) naming no live issue:${bad}" \
          "run the register-limitation skill — an unregistered gap is invisible debt"
}

# ------------------------------------------------------------------ checks · in-flight work
# The failure that prompted this: a session reported 10/10 from --cheap while four subagents and
# a CI watcher were still running. Closing it would have thrown all of that away. Background
# work is the one kind of incompleteness that leaves NO trace in git, the ledger or CI — the
# session is the only thing that knows, and it is exactly what a session forgets when it thinks
# it is finished.
#
# Claude Code writes each background task's stream to <scratchpad>/tasks/<id>.output and closes
# it with "[exited with code N]" or "[killed]". No marker means it never ended.
task_dir() {
    local c
    for c in "${CLAUDE_SCRATCHPAD_DIR:-}/../tasks" \
             "/private/tmp/claude-$(id -u)/$(printf '%s' "$REPO_ROOT" | sed 's#[/.]#-#g')/$SESSION_ID/tasks" \
             "/tmp/claude-$(id -u)/$(printf '%s' "$REPO_ROOT" | sed 's#[/.]#-#g')/$SESSION_ID/tasks"; do
        [ -d "$c" ] && { printf '%s' "$c"; return 0; }
    done
    return 1
}

# Every pid from this shell up to init. bilan runs inside one of those task streams itself, and
# its own output file is open and unmarked exactly like a live task's — the third self-match
# this file has had to answer for. A file held only by this process tree is this invocation.
my_pids() {
    local p=$$ out=""
    while [ -n "$p" ] && [ "$p" != 0 ] && [ "$p" != 1 ]; do
        out="$out $p"
        p=$(ps -o ppid= -p "$p" 2>/dev/null | tr -d ' ')
    done
    printf '%s' "$out"
}

check_running_tasks() {
    local dir f pids p mine running=0 names="" id
    command -v lsof >/dev/null 2>&1 || return 0
    dir=$(task_dir) || return 0
    mine=$(my_pids)
    for f in "$dir"/*.output; do
        [ -f "$f" ] || continue
        grep -q '\[exited with code\|\[killed\]' "$f" 2>/dev/null && continue
        pids=$(lsof -t -- "$f" 2>/dev/null) || continue
        [ -n "$pids" ] || continue        # nobody holds it: ended without writing a marker
        # ANY holder in this process's ancestry means the stream is this very invocation —
        # bilan is itself running inside one. Testing for a holder that is *not* mine reads
        # backwards: the shell's own subshells are descendants, so they never match the
        # ancestry walk and every run reported itself as a live task.
        local is_mine=0
        for p in $pids; do
            printf '%s' " $mine " | grep -q " $p " && { is_mine=1; break; }
        done
        [ "$is_mine" = 1 ] && continue
        id=$(basename "$f" .output)
        running=$((running + 1)); names="$names $id"
    done
    [ "$running" -gt 0 ] || return 0
    cap 7 "❌" "$running background task(s) still running:${names}" \
          "wait for them, or TaskStop them deliberately — closing the session loses the work"
}

# The deepest blind spot, named by a session that scored 10/10 with its artifact unwritten:
# "bilan measures repo state, and this session deliberately touches none of it — the 10 is
# honest about the repo and silent about the deliverable." A session whose work is research, a
# document, or a published artifact commits nothing, holds no issue and triggers no CI, so every
# check came back clean and the number said done.
#
# The session's own todo list is the missing measurement. It is the session declaring what it
# set out to do, Claude Code keeps it per session under tasks/<session-id>/, and an item still
# pending or in progress is the session's own statement that it is not finished.
check_open_todos() {
    local dir n subjects
    dir="$CFG/tasks/$SESSION_ID"
    [ -d "$dir" ] || return 0
    n=$(jq -r 'select(.status == "pending" or .status == "in_progress") | .subject // .description // "?"' \
        "$dir"/*.json 2>/dev/null | grep -c . ) || return 0
    [ "${n:-0}" -gt 0 ] || return 0
    subjects=$(jq -r 'select(.status == "pending" or .status == "in_progress") | .subject // .description // "?"' \
        "$dir"/*.json 2>/dev/null | head -3 | cut -c1-60 | tr '\n' '·' | sed 's/·/ · /g; s/ · $//')
    cap 7 "❌" "$n todo(s) still open: $subjects" \
          "finish them, or drop the ones you are not doing — an open todo is this session saying it is not done"
}

# A score of 10 means "everything I checked is done". When nothing was checkable, 10 means
# nothing at all — and that is how a session scored 10/10 with its artifact unwritten. bilan
# reads the repo, the register and CI; a session whose work is research, a design, a document or
# a published artifact touches none of the three, and every check came back clean because every
# check came back empty.
#
# So it says so. A session that made no commit and declared no todo is unmeasured, not complete,
# and cannot reach 10 — the same rule as --cheap, for the same reason. It caps at 9 rather than
# blocking, because answering a question really is a complete session; what it must not do is
# produce a verdict it never earned.
check_measurable() {
    local f start_head todos=0
    [ -d "$CFG/tasks/$SESSION_ID" ] && \
        todos=$(command ls "$CFG/tasks/$SESSION_ID"/*.json 2>/dev/null | grep -c . )
    [ "${todos:-0}" -gt 0 ] && return 0                 # the session declared what it set out to do
    f=$(baseline_file) || return 0
    start_head=$(cat "${f}.head" 2>/dev/null || true)
    [ -n "$start_head" ] || return 0                    # no baseline: cannot tell, do not claim
    [ "$start_head" = "$HEAD_SHA" ] || return 0         # it committed something: that is measurable
    cap 9 "⚠️" "nothing measurable: this session made no commit and declared no todo" \
          "say plainly whether the work is done — bilan checked the repo, the register and CI, and this session touched none of them"
}

# The opening ask, printed with every report. bilan cannot judge whether the work satisfies it —
# that would be narration again, which is the disease it treats — but it can refuse to let a
# session claim completion without the request in front of it. Extracted the way the session
# index does it: a session often opens with pasted output, which names it worse than nothing.
opening_ask() {
    local tx
    tx=$(transcript_path 2>/dev/null) || return 0
    [ -f "$tx" ] || return 0
    jq -r 'select(.type == "user")
           | (if (.message.content | type) == "string" then .message.content
              else ([.message.content[]? | select(.type == "text") | .text] | join("\n")) end)
           | select(test("<command-name>|<local-command|<system-reminder>|<task-notification>|<cross-session-message")|not)
           | select(test("^\\s*$")|not)' "$tx" 2>/dev/null \
      | grep -vE '^\s*$' | grep -vE '^\s*[│┌└├─╭╰|+=#]' | grep -vE '│.*│' \
      | head -1 | tr '\t\n' '  ' | sed -e 's/  */ /g' -e 's/^ //' | cut -c1-150
}

# ------------------------------------------------------------------ checks · dev stack
check_dev_stack() {
    local lib="$REPO_ROOT/bin/dev-processes.sh" f name up=""
    [ -f "$lib" ] || return 0
    # shellcheck disable=SC1090
    DEV_PROJECT_ROOT="$REPO_ROOT" . "$lib" >/dev/null 2>&1 || return 0
    for f in "$REPO_ROOT"/logs/*.pid; do
        [ -f "$f" ] || continue
        name=$(basename "$f" .pid)
        dev_owned "$name" >/dev/null 2>&1 && up="$up $name"
    done
    [ -n "${up// /}" ] || return 0
    cap 9 "⚠️" "dev stack from this checkout still up:${up}" \
          "./bin/stop-server.sh — a running stack holds 8081/8082/5173 against the next session"
}

# ------------------------------------------------------------------ checks · CI (network)
# `gh run list --commit` returns zero rows on this org even when runs exist, so rows are filtered
# by headSha out of a wide branch window. Absence is its own outcome: a sha with no row is NOT
# green, because "nothing pending" and "not present" are indistinguishable in this query.
check_ci() {
    local upstream ahead slug runs total running bad cancelled age
    command -v gh >/dev/null 2>&1 || return 0
    upstream=$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || true)
    [ -n "$upstream" ] || return 0
    ahead=$(git rev-list --count "$upstream..HEAD" 2>/dev/null || echo 0)
    [ "$ahead" -gt 0 ] && return 0   # unpushed already caps at 8; CI cannot have run
    slug=$(git remote get-url origin 2>/dev/null \
           | sed -E 's#^(git@github\.com:|https://github\.com/|ssh://git@github\.com/)##; s#\.git$##; s#/$##')
    [ -n "$slug" ] || return 0

    # Addressed by sha, not by a branch window. `gh run list --branch main --limit N` loses a
    # row the moment peers push past it, and "zero pending" is indistinguishable from "fell out
    # of the window" — which reads as green. That mistake has been made twice here. This
    # endpoint answers for exactly one commit and cannot be crowded out.
    runs=$(gh api "repos/$slug/commits/$HEAD_SHA/check-runs" 2>/dev/null) || return 0
    total=$(printf '%s' "$runs" | jq -r '.total_count // 0' 2>/dev/null)

    if [ "${total:-0}" = 0 ]; then
        # GitHub itself says no check exists for this commit. Right after a push that means the
        # checks have not registered yet; once the commit has had time, it means no workflow's
        # path filter matched it and none ever will — a tooling- or docs-only commit. Capping
        # forever on a row that cannot arrive is how a finished session never reaches 10.
        age=$(( $(date +%s) - $(git log -1 --format=%ct HEAD 2>/dev/null || date +%s) ))
        if [ "$age" -lt 300 ]; then
            cap 9 "⚠️" "no checks registered yet for ${HEAD_SHA:0:8} ($((age))s after commit)" \
                  "give it a minute, then re-run — absent is not green"
        else
            say_note "GitHub reports no check for ${HEAD_SHA:0:8}: no workflow path filter matches what it touched, so none will run"
        fi
        return 0
    fi

    running=$(printf '%s' "$runs" | jq -r '[.check_runs[] | select(.status != "completed") | .name] | unique | join(", ")')
    bad=$(printf '%s' "$runs" | jq -r '[.check_runs[] | select(.conclusion | IN("failure","timed_out","action_required","startup_failure")) | .name] | unique | join(", ")')
    cancelled=$(printf '%s' "$runs" | jq -r '[.check_runs[] | select(.conclusion == "cancelled") | .name] | unique | join(", ")')

    [ -z "$bad" ]       || cap 5 "❌" "CI red on ${HEAD_SHA:0:8}: $bad" "fix and re-push — red is not done"
    [ -z "$running" ]   || cap 9 "⚠️" "CI still running on ${HEAD_SHA:0:8} ($total checks): $running" "wait for terminal status"
    [ -z "$cancelled" ] || cap 9 "⚠️" "CI cancelled on ${HEAD_SHA:0:8}: $cancelled" \
                                "cancelled is unvalidated, not green — re-run or dispatch"
    [ -n "$bad$running$cancelled" ] || say_note "CI green on ${HEAD_SHA:0:8}: $total checks, all successful or skipped"
}

# ------------------------------------------------------------------ friction (informational)
# Borrowed from Tencent's teamai-cli, which scores a session by how much it struggled. It is NOT
# a cap: a failure found and fixed is just work and never deducts. It is printed because a
# session with 30 tool errors claiming a clean 10 is worth a second look.
friction_line() {
    local tx counts
    # One jq pass over a transcript that can reach tens of MB, so it is skipped on the Stop
    # hook's path: friction never caps the score, and a hook that overruns is killed at ~10s.
    [ "$CHEAP" = 0 ] || return 0
    tx=$(transcript_path 2>/dev/null) || return 0
    [ -f "$tx" ] || return 0
    counts=$(jq -rs '
        def blocks: .message.content? | if type == "array" then .[] else empty end;
        def user_text: select(.type == "user") | blocks
                       | select(.type == "text") | .text;
        def results:   select(.type == "user") | blocks
                       | select(.type == "tool_result");
        [ ([ .[] | results | select(.is_error == true) ] | length),
          ([ .[] | user_text
             | select(startswith("[Request interrupted by user")) ] | length),
          ([ .[] | results | .content
             | (if type == "array" then (.[]? | .text? // "") else (. // "") end)
             | select(type == "string")
             | select(startswith("The user doesn'"'"'t want to proceed")) ] | length)
        ] | @tsv' "$tx" 2>/dev/null)
    [ -n "$counts" ] || return 0
    printf '%s\n' "$counts"
}

# ------------------------------------------------------------------ run
run_checks() {
    ensure_baseline
    check_worktree
    check_unpushed
    check_stash
    check_branch_cleanup
    check_validation_marker
    check_carnet_held
    check_carnet_filed
    check_limitation_markers
    check_dev_stack
    check_running_tasks
    check_open_todos
    check_measurable
    # A measurement taken with checks switched off must not be allowed to say "done". --cheap
    # skips CI entirely, and a session quoted its cheap 10/10 as completion while a lane was
    # still red and four agents were running. The reduced measurement now cannot reach 10 by
    # construction, and says why.
    if [ "$CHEAP" = 1 ]; then
        cap 9 "⚠️" "local facts only — CI was not consulted, so this is not a completion verdict" \
              "run bilan.sh without --cheap before reporting a number"
    else
        check_ci
    fi
}

score() {
    local min=10 c
    while IFS=$'\t' read -r c _ _ _; do
        [ -n "$c" ] || continue
        [ "$c" -lt "$min" ] && min=$c
    done < "$CAPS"
    printf '%s' "$min"
}

score_file() { [ -n "$SESSION_ID" ] && printf '%s' "$CFG/bilan/$(printf '%s' "$SESSION_ID" | tr -c 'a-zA-Z0-9._-' '_').score"; }

publish_score() { # <score>
    local f top
    f=$(score_file) || return 0
    mkdir -p "$(dirname "$f")" 2>/dev/null || return 0
    top=$(sort -n "$CAPS" 2>/dev/null | head -1 | cut -f3 | cut -c1-60)
    printf '%s\t%s\t%s\n' "$1" "$(date +%s)" "${top:-nothing outstanding}" > "$f" 2>/dev/null || true
}

report() {
    local s f_errors f_interrupts f_denials fr icon ev rem c line
    s=$(score)
    publish_score "$s"
    fr=$(friction_line)
    [ -n "$fr" ] || fr=$(printf '0\t0\t0')
    IFS=$'\t' read -r f_errors f_interrupts f_denials <<< "$fr"

    if [ "$JSON" = 1 ]; then
        jq -n --argjson score "$s" \
              --rawfile notes "$NOTES" \
              --arg session "${SESSION_ID:-manual}" --arg branch "$BRANCH" --arg sha "$HEAD_SHA" \
              --argjson errors "${f_errors:-0}" --argjson interrupts "${f_interrupts:-0}" \
              --argjson denials "${f_denials:-0}" \
              --rawfile caps "$CAPS" \
          '{score:$score, session:$session, branch:$branch, sha:$sha,
            friction:{tool_errors:$errors, interrupts:$interrupts, denials:$denials},
            notes: ($notes | split("\n") | map(select(length>0))),
            caps: ($caps | split("\n") | map(select(length>0) | split("\t")
                   | {cap:(.[0]|tonumber), icon:.[1], evidence:.[2], remedy:.[3]}))}'
        [ "$s" = 10 ] && return 0 || return 1
    fi

    if [ "$s" = 10 ] && [ "$QUIET" = 1 ]; then return 0; fi

    say "BILAN · ${SESSION_ID:0:8} · $(basename "$REPO_ROOT") @ $BRANCH ${HEAD_SHA:0:8}"
    local ask; ask=$(opening_ask)
    [ -z "$ask" ] || say "asked: $ask"
    say ""
    if [ ! -s "$CAPS" ]; then
        say "  ✅ nothing outstanding — tree clean, nothing unpushed, no issue held"
    else
        sort -n "$CAPS" | while IFS=$'\t' read -r c icon ev rem; do
            say "  $icon $ev"
            say "     → $rem  (caps at $c)"
        done
    fi
    if [ -s "$NOTES" ]; then
        say ""
        while IFS= read -r line; do [ -n "$line" ] && say "  ℹ️  $line"; done < "$NOTES"
    fi
    say ""
    [ "${f_errors:-0}" = 0 ] && [ "${f_interrupts:-0}" = 0 ] && [ "${f_denials:-0}" = 0 ] || \
        say "  friction: ${f_errors} tool error(s) · ${f_interrupts} interrupt(s) · ${f_denials} denial(s)   (informational — a fixed failure never deducts)"
    say ""
    say "COMPLETION: $s/10"
    [ "$s" = 10 ] && return 0 || return 1
}

# ------------------------------------------------------------------ sweep
# The only thing that catches a session that died: a kill -9 fires no exit hook, so the dead
# session can never report on itself. Reads every ledger on this machine and every worktree of
# this repo, and names what a session that is no longer running left behind.
is_session_alive() { # <session-id> <pid>
    local f
    for f in "$HOME"/.claude*/sessions/"$2".json; do
        [ -f "$f" ] || continue
        [ "$(jq -r '.sessionId // empty' "$f" 2>/dev/null)" = "$1" ] || continue
        kill -0 "$2" 2>/dev/null && return 0
    done
    return 1
}

cmd_sweep() {
    local dir f id pid name at issues found=0 path="" branch="" dirty ahead line
    say "BILAN SWEEP · $(basename "$REPO_ROOT")"
    say ""
    for dir in "$HOME"/.claude*/carnet-claims; do
        [ -d "$dir" ] || continue
        for f in "$dir"/*.jsonl; do
            [ -s "$f" ] || continue
            id=$(basename "$f" .jsonl)
            [ "$id" = "${SESSION_ID:-}" ] && continue
            pid=$(head -1 "$f" | jq -r '.pid // 0')
            is_session_alive "$id" "$pid" && continue
            name=$(head -1 "$f" | jq -r '.name // "?"')
            at=$(head -1 "$f"   | jq -r '.at // "?"')
            issues=$(jq -r 'select(.kind == "claim") | "carnet#\(.issue)"' "$f" 2>/dev/null | tr '\n' ' ')
            [ -n "${issues// /}" ] || continue
            found=1
            say "  ☠️  session $name (${id:0:8}, last claim $at) ended holding: ${issues% }"
            say "     → carnet.sh status <n> to see it; a plain claim takes over a stale one"
        done
    done

    while IFS= read -r line; do
        case "$line" in
            worktree\ *) path=${line#worktree } ;;
            branch\ *)
                branch=${line#branch refs/heads/}
                dirty=$(git -C "$path" status --porcelain 2>/dev/null | grep -cv '^??' || true)
                ahead=$(git -C "$path" rev-list --count '@{u}..HEAD' 2>/dev/null || echo 0)
                if [ "${dirty:-0}" -gt 0 ] || [ "${ahead:-0}" -gt 0 ]; then
                    found=1
                    say "  📂 $branch ($path): ${dirty:-0} uncommitted, ${ahead:-0} unpushed"
                fi ;;
        esac
    done <<< "$(git worktree list --porcelain 2>/dev/null)"

    [ "$found" = 1 ] || say "  ✅ nothing left behind by a dead session"
    return 0
}

# ------------------------------------------------------------------ argv
CMD=report
WHY=""
while [ $# -gt 0 ]; do
    case "$1" in
        sweep)      CMD=sweep ;;
        ack)        CMD=ack ;;
        baseline)   CMD=baseline ;;
        --why)      shift; WHY=${1:-} ;;
        --cheap)    CHEAP=1 ;;
        --json)     JSON=1 ;;
        --quiet)    QUIET=1 ;;
        --session)  shift; SESSION_ID=${1:-} ;;
        -h|--help)  usage; exit 0 ;;
        *)          die "unknown argument: $1" ;;
    esac
    shift
done

case "$CMD" in
    sweep)  cmd_sweep ;;
    ack)    cmd_ack "$WHY" ;;
    baseline) cmd_baseline ;;
    report) run_checks; report ;;
esac
