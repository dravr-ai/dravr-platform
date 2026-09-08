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
trap 'rm -f "$CAPS"' EXIT

cap() { # <cap> <icon> <evidence> <remedy>
    printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" >> "$CAPS"
}

# ------------------------------------------------------------------ session facts
ledger_file() { [ -n "$SESSION_ID" ] && printf '%s' "$LEDGER_DIR/$SESSION_ID.jsonl"; }

session_started_epoch() {
    local f
    f=$(ledger_file) || return 1
    [ -s "$f" ] || return 1
    date -j -f '%Y-%m-%dT%H:%M:%SZ' "$(head -1 "$f" | jq -r '.at // empty')" +%s 2>/dev/null \
        || date -d "$(head -1 "$f" | jq -r '.at // empty')" +%s 2>/dev/null
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
    local porcelain tracked untracked t_n u_n
    porcelain=$(git status --porcelain 2>/dev/null)
    [ -n "$porcelain" ] || return 0
    tracked=$(printf '%s\n' "$porcelain" | grep -v '^??' | sed 's/^...//')
    untracked=$(printf '%s\n' "$porcelain" | grep '^??' | sed 's/^...//')
    t_n=$(printf '%s\n' "$tracked" | grep -cv '^$')
    u_n=$(printf '%s\n' "$untracked" | grep -cv '^$')
    if [ "${t_n:-0}" -gt 0 ]; then
        cap 7 "❌" "$t_n tracked file(s) modified and uncommitted: $(name_files 8 "$tracked")" \
              "commit them — or, if they are a peer's work in a shared worktree, say so and leave them alone"
    fi
    if [ "${u_n:-0}" -gt 0 ]; then
        cap 9 "⚠️" "$u_n untracked file(s): $(name_files 8 "$untracked")" \
              "add them, delete them, or move them to the scratchpad"
    fi
}

check_unpushed() {
    local upstream ahead shas
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
    ahead=$(git rev-list --count "$upstream..HEAD" 2>/dev/null || echo 0)
    [ "$ahead" -gt 0 ] || return 0
    shas=$(git log --format=%h "$upstream..HEAD" 2>/dev/null | tr '\n' ' ')
    cap 8 "❌" "$ahead commit(s) not pushed to $upstream ($shas)" "git push"
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
    local marker epoch sha age upstream ahead
    upstream=$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || true)
    [ -n "$upstream" ] || return 0
    ahead=$(git rev-list --count "$upstream..HEAD" 2>/dev/null || echo 0)
    [ "$ahead" -gt 0 ] || return 0   # nothing to push, nothing to validate
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

# `create` writes a "filed" line, so a session that opened issues instead of fixing them cannot
# report 10 without saying, per issue, why it is residue rather than the work it was asked to do.
check_carnet_filed() {
    local f n list=""
    f=$(ledger_file) || return 0
    [ -s "$f" ] || return 0
    for n in $(jq -r 'select(.kind == "filed") | .issue' "$f" 2>/dev/null); do
        list="$list carnet#$n"
    done
    [ -n "${list// /}" ] || return 0
    cap 9 "⚠️" "filed this session:${list}" \
          "name each one as residue you are deliberately not fixing — fix first, file the rest"
}

# A LIMITATION marker is the sanctioned way to ship a gap, but only when it names a live issue.
check_limitation_markers() {
    local upstream range added marker n bad=""
    upstream=$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || echo origin/main)
    range="$upstream..HEAD"
    # Both halves of the session's work: what is committed but unpushed, and what is still in
    # the tree. The marker is matched with its whole payload, so one that names nothing at all
    # is a row here rather than an empty extraction that iterates zero times.
    added=$( { git diff -U0 "$range" 2>/dev/null; git diff -U0 HEAD 2>/dev/null; } \
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
    local upstream ahead rows mine running bad cancelled
    command -v gh >/dev/null 2>&1 || return 0
    upstream=$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || true)
    [ -n "$upstream" ] || return 0
    ahead=$(git rev-list --count "$upstream..HEAD" 2>/dev/null || echo 0)
    [ "$ahead" -gt 0 ] && return 0   # unpushed already caps at 8; CI cannot have run

    rows=$(gh run list --branch "$BRANCH" --limit 100 \
             --json headSha,status,conclusion,workflowName 2>/dev/null) || return 0
    mine=$(printf '%s' "$rows" | jq --arg s "$HEAD_SHA" '[.[] | select(.headSha == $s)]' 2>/dev/null)
    [ -n "$mine" ] || return 0

    if [ "$(printf '%s' "$mine" | jq 'length')" = 0 ]; then
        cap 9 "⚠️" "no CI row for ${HEAD_SHA:0:8} on $BRANCH — absent, not necessarily done" \
              "check the Actions page; a sha that was never a push tip gets no run of its own"
        return 0
    fi
    running=$(printf '%s' "$mine" | jq -r '[.[] | select(.status != "completed") | .workflowName] | join(", ")')
    # An in-progress run reports conclusion "" (not null), so a naive filter reads running as red.
    bad=$(printf '%s' "$mine" | jq -r '[.[] | select(.conclusion != "" and .conclusion != null
            and (.conclusion | IN("success","skipped","cancelled") | not)) | .workflowName] | join(", ")')
    cancelled=$(printf '%s' "$mine" | jq -r '[.[] | select(.conclusion == "cancelled") | .workflowName] | join(", ")')

    [ -z "$bad" ]       || cap 5 "❌" "CI red on ${HEAD_SHA:0:8}: $bad" "fix and re-push — red is not done"
    [ -z "$running" ]   || cap 9 "⚠️" "CI still running on ${HEAD_SHA:0:8}: $running" "wait for terminal status"
    [ -z "$cancelled" ] || cap 9 "⚠️" "CI cancelled on ${HEAD_SHA:0:8}: $cancelled" \
                                "cancelled is unvalidated, not green — re-run or dispatch"
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
    check_worktree
    check_unpushed
    check_stash
    check_branch_cleanup
    check_validation_marker
    check_carnet_held
    check_carnet_filed
    check_limitation_markers
    check_dev_stack
    [ "$CHEAP" = 1 ] || check_ci
}

score() {
    local min=10 c
    while IFS=$'\t' read -r c _ _ _; do
        [ -n "$c" ] || continue
        [ "$c" -lt "$min" ] && min=$c
    done < "$CAPS"
    printf '%s' "$min"
}

report() {
    local s f_errors f_interrupts f_denials fr icon ev rem c line
    s=$(score)
    fr=$(friction_line)
    [ -n "$fr" ] || fr=$(printf '0\t0\t0')
    IFS=$'\t' read -r f_errors f_interrupts f_denials <<< "$fr"

    if [ "$JSON" = 1 ]; then
        jq -n --argjson score "$s" \
              --arg session "${SESSION_ID:-manual}" --arg branch "$BRANCH" --arg sha "$HEAD_SHA" \
              --argjson errors "${f_errors:-0}" --argjson interrupts "${f_interrupts:-0}" \
              --argjson denials "${f_denials:-0}" \
              --rawfile caps "$CAPS" \
          '{score:$score, session:$session, branch:$branch, sha:$sha,
            friction:{tool_errors:$errors, interrupts:$interrupts, denials:$denials},
            caps: ($caps | split("\n") | map(select(length>0) | split("\t")
                   | {cap:(.[0]|tonumber), icon:.[1], evidence:.[2], remedy:.[3]}))}'
        [ "$s" = 10 ] && return 0 || return 1
    fi

    if [ "$s" = 10 ] && [ "$QUIET" = 1 ]; then return 0; fi

    say "BILAN · ${SESSION_ID:0:8} · $(basename "$REPO_ROOT") @ $BRANCH ${HEAD_SHA:0:8}"
    say ""
    if [ ! -s "$CAPS" ]; then
        say "  ✅ nothing outstanding — tree clean, nothing unpushed, no issue held"
    else
        sort -n "$CAPS" | while IFS=$'\t' read -r c icon ev rem; do
            say "  $icon $ev"
            say "     → $rem  (caps at $c)"
        done
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
while [ $# -gt 0 ]; do
    case "$1" in
        sweep)      CMD=sweep ;;
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
    report) run_checks; report ;;
esac
