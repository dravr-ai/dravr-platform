#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Stop hook — refuses the first stop while this session still holds carnet issues or unpushed work
# ABOUTME: Blocks once per distinct dirty state, so it cannot wedge a session that has nothing left to do
#
# Wire it in .claude/settings.json:
#   "Stop": [{ "hooks": [{ "type": "command", "timeout": 15,
#     "command": "[ -f .agents/skills/bilan/hooks/stop-gate.sh ] && bash .agents/skills/bilan/hooks/stop-gate.sh || true" }]}]
#
# The whole point: a session that believes it is done says 10 and stops, leaving open carnet
# issues behind. This runs the same measurement the completion number now comes from, and sends
# the session back to work with the list instead of letting it idle.
#
# Two loop guards, because a hook that blocks forever is worse than no hook:
#   1. stop_hook_active — the host sets it on the stop that this hook already blocked once.
#   2. the state signature — the same set of caps never blocks twice. Fix one thing and the
#      signature changes, so the gate speaks again about what is left; fix nothing and it stays
#      quiet, and the outstanding work is the session's accountability, not the hook's.
set -uo pipefail

here=$(cd "$(dirname "$0")" && pwd)
bilan="$here/../bilan.sh"
[ -f "$bilan" ] || exit 0
command -v jq >/dev/null 2>&1 || exit 0

payload=$(cat 2>/dev/null || true)
[ "$(printf '%s' "$payload" | jq -r '.stop_hook_active // false' 2>/dev/null)" = true ] && exit 0

sid=$(printf '%s' "$payload" | jq -r '.session_id // empty' 2>/dev/null || true)
[ -n "$sid" ] || sid=${CLAUDE_CODE_SESSION_ID:-}
[ -n "$sid" ] || exit 0

# --cheap keeps this off the network: hosts kill a hook at ~10s regardless of the declared
# timeout, and this runs when every turn ends.
report=$(CLAUDE_CODE_SESSION_ID="$sid" bash "$bilan" --cheap --json 2>/dev/null) || true
[ -n "$report" ] || exit 0

# The gate blocks at 8 or below. A cap of 9 — an untracked scratch file, a stash, a stale
# validation marker, CI still running — is worth reporting and is not worth refusing a stop
# over; a session would hit one on nearly every turn and the gate would become wallpaper. What
# blocks is what ChefFamille actually kept finding at the end of a session: a carnet issue still
# held (6), an unregistered LIMITATION marker (6), uncommitted tracked files (7), commits never
# pushed (8), CI red (5). The block reason still lists every cap, so nothing is hidden by the
# threshold — only the decision to interrupt turns on it.
score=$(printf '%s' "$report" | jq -r '.score // 10' 2>/dev/null)
case "$score" in ''|*[!0-9]*) exit 0 ;; esac
[ "$score" -ge 9 ] && exit 0

CFG=${CLAUDE_CONFIG_DIR:-$HOME/.claude}
state_dir="$CFG/bilan"
mkdir -p "$state_dir" 2>/dev/null || exit 0
state="$state_dir/$(printf '%s' "$sid" | tr -c 'a-zA-Z0-9._-' '_').json"

signature=$(printf '%s' "$report" | jq -r '[.caps[] | .evidence] | sort | join("|")' 2>/dev/null \
            | shasum 2>/dev/null | cut -d' ' -f1)
[ -n "$signature" ] || exit 0
# One rule, so the cost is predictable and the behaviour is not: never block twice inside
# thirty minutes, and after that block again while the score is still 8 or below.
#
# The two extremes were both wrong. Blocking on every distinct state meant a peer editing beside
# you in the shared checkout produced a new signature every few minutes and the gate never shut
# up. Blocking once per session, ever, meant a session that held carnet#384 for nineteen hours
# was told at 11:02 and never again — ChefFamille had to run /bilan by hand at 13:30 to find it
# still sitting at 6/10. A cooldown costs at most two model turns an hour and keeps telling a
# session that is genuinely stuck.
#
# The status line carries the same number continuously at zero cost, so this channel only has to
# catch the session that is about to stop, not keep anyone informed.
COOLDOWN=1800
last=$(jq -r '.blockedAt // empty' "$state" 2>/dev/null)
if [ -n "$last" ]; then
    # -u, or BSD date reads the UTC stamp as local time and the elapsed value comes out
    # NEGATIVE — which is always "inside the cooldown", so a session blocked once could never
    # be blocked again. That is the silent version of the ceiling bug this cooldown replaced.
    last_epoch=$(date -j -u -f '%Y-%m-%dT%H:%M:%SZ' "$last" +%s 2>/dev/null \
                 || date -u -d "$last" +%s 2>/dev/null || echo 0)
    [ $(( $(date +%s) - ${last_epoch:-0} )) -lt "$COOLDOWN" ] && exit 0
fi
blocks=$(jq -r '.blocks // 0' "$state" 2>/dev/null); blocks=${blocks:-0}
case "$blocks" in ''|*[!0-9]*) blocks=0 ;; esac

jq -n --arg s "$signature" --arg at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --argjson score "$score" \
   --argjson n "$((blocks + 1))" \
   '{signature:$s, blockedAt:$at, score:$score, blocks:$n}' > "$state" 2>/dev/null || true

reason=$(printf '%s' "$report" | jq -r '
    "bilan says this session is at \(.score)/10, not done. Outstanding:\n"
    + ([.caps[] | "  · \(.evidence)\n    → \(.remedy)"] | join("\n"))
    + "\n\nFinish these, then report the number bilan gives — not your own. If one of them is "
    + "deliberate, say which and why in your next message rather than leaving it unstated."')

jq -n --arg r "$reason" '{decision:"block", reason:$r}'
exit 0
