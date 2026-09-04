#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: PreToolUse hook — claims the carnet issues a prompt named, on the session's first edit
# ABOUTME: Makes the claim mechanical instead of advisory; every failure path leaves the tool alone
#
# Wire it in .claude/settings.json:
#   "PreToolUse": [{ "matcher": "Edit|Write|MultiEdit|NotebookEdit|Bash", "hooks": [{ "type": "command",
#     "timeout": 20,
#     "command": "[ -f .agents/skills/carnet/hooks/auto-claim.sh ] && bash .agents/skills/carnet/hooks/auto-claim.sh || true" }]}]
#
# The rule was "claim before the first edit", and a rule the model has to remember is one it
# will sometimes forget -- which is the whole failure the claim exists to prevent. So the
# trigger is the edit itself: prompt-status.sh writes the issue numbers a prompt named, and
# the first write-shaped tool call after that claims them.
#
# Reading is not working. A prompt that only asks about an issue never reaches a write tool,
# so it never claims. A Bash call is treated as an edit only when the command looks like one.
#
# On a conflict the tool is blocked ONCE (exit 2, stderr reaches the model) naming the live
# peer that holds the issue. Once told, the session is accountable and later edits pass: a
# permanent block would be a deadlock over an issue that may only have been mentioned.
set -uo pipefail

CFG=${CLAUDE_CONFIG_DIR:-$HOME/.claude}
PENDING_DIR="$CFG/carnet-claims/pending"

# Cheapest possible exit for the overwhelmingly common case: nothing is pending. No jq, no
# payload parse, no subprocess -- this runs before every edit in every session.
[ -d "$PENDING_DIR" ] || exit 0
set -- "$PENDING_DIR"/*.txt
[ -e "$1" ] || exit 0

here=$(cd "$(dirname "$0")" && pwd)
carnet="$here/../carnet.sh"
[ -f "$carnet" ] || exit 0
command -v jq >/dev/null 2>&1 || exit 0
command -v gh >/dev/null 2>&1 || exit 0

payload=$(cat 2>/dev/null || true)
[ -n "$payload" ] || exit 0
tool=$(printf '%s' "$payload" | jq -r '.tool_name // empty' 2>/dev/null || true)

case "$tool" in
    Edit|Write|MultiEdit|NotebookEdit) ;;
    Bash)
        cmd=$(printf '%s' "$payload" | jq -r '.tool_input.command // empty' 2>/dev/null || true)
        # Discard redirects to /dev/null before classifying. `2>/dev/null` is the
        # commonest idiom in a READ, and it contains a `>`, so the redirect test
        # below counted every quiet read as a write: `git status -sb` claimed
        # nothing while `git log ... 2>/dev/null` claimed everything pending.
        # Four issues were taken that way in a day, one of them a peer's, off a
        # message that only NAMED the number.
        probe=$(printf '%s' "$cmd" | sed -E 's/(^|[[:space:]&])[0-9]*>>?[[:space:]]*\/dev\/null//g')
        # `git add --dry-run` / `git apply --check` report and change nothing.
        probe=$(printf '%s' "$probe" | sed -E 's/git[[:space:]]+[a-z-]+([[:space:]]+[^;|&]*)?(--dry-run|--check)/git-reporting-only/g')
        # A write-shaped command: a redirect into a file, an in-place edit, or git recording
        # something. Anything else is a read, and a read claims nothing.
        #
        # The git verbs need a terminator. Without one, `merge` matches inside
        # `git merge-base` — the standard way to ask "is this commit on main?" —
        # so a pure ancestry query counted as a write and claimed every pending
        # issue. That took carnet#323 off a peer mid-investigation, who then
        # stood down. Same failure class as the `2>/dev/null` bug above: a read
        # that pattern-matches as a write, costing an issue that was only NAMED.
        #
        # `--dry-run` and `--check` are excluded for the same reason: `git add
        # --dry-run` and `git apply --check` write nothing and only report.
        printf '%s' "$probe" | grep -qE '>>?[[:space:]]*[^&|[:space:]]|sed -i|(^|[|;&[:space:]])tee[[:space:]]|(^|[|;&[:space:]])(mv|cp|rm|mkdir|touch|install)[[:space:]]|git[[:space:]]+(commit|add|apply|am|merge|rebase|revert|cherry-pick|checkout|restore|reset)([[:space:]]|$)' || exit 0
        ;;
    *) exit 0 ;;
esac

sid=$(printf '%s' "$payload" | jq -r '.session_id // empty' 2>/dev/null || true)
[ -n "$sid" ] || sid=${CLAUDE_CODE_SESSION_ID:-}
[ -n "$sid" ] || exit 0

pending="$PENDING_DIR/$sid.txt"
[ -s "$pending" ] || exit 0

# A list the session never acted on goes stale: an issue named an hour ago is not what this
# edit is about. Bounded rather than permanent, so multi-turn work still claims.
if [ -z "$(find "$pending" -mmin -60 2>/dev/null)" ]; then
    rm -f "$pending"
    exit 0
fi

nums=$(tr -d ' \r' < "$pending" | grep -E '^[0-9]+$' | sort -un || true)
rm -f "$pending"                      # consumed: act once, never on every later edit
[ -n "$nums" ] || exit 0

ledger="$CFG/carnet-claims/$sid.jsonl"
warned="$CFG/carnet-claims/warned/$sid.txt"
mkdir -p "$(dirname "$warned")" 2>/dev/null || true

held=""
[ -s "$ledger" ] && held=$(jq -r 'select(.kind == "claim") | .issue' "$ledger" 2>/dev/null || true)

claimed=""
blocked=""
for n in $nums; do
    printf '%s\n' $held | grep -qx "$n" && continue     # this session already holds it
    out=$(bash "$carnet" claim "$n" 2>&1); rc=$?
    case $rc in
        0) claimed="$claimed $n" ;;
        2)
            # Refused: a live peer holds it. Say so once, then stop repeating it.
            if ! { [ -f "$warned" ] && grep -qx "$n" "$warned"; }; then
                printf '%s\n' "$n" >> "$warned"
                blocked="$blocked
carnet#$n — $(printf '%s' "$out" | grep -v '^$' | tail -2)"
            fi
            ;;
        *) : ;;                                          # unreachable issue, bad tracker: never block on it
    esac
done

if [ -n "$blocked" ]; then
    {
        echo "A live peer session already holds work you are about to start."
        printf '%s\n' "$blocked"
        echo
        echo "Do not do this work twice. Say who holds it and stop, or ask ChefFamille"
        echo "whether to take it over (--steal warns the displaced session)."
    } >&2
    exit 2
fi

[ -n "$claimed" ] && echo "🔒 carnet auto-claimed:$claimed"
exit 0
