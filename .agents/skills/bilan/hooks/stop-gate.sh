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

score=$(printf '%s' "$report" | jq -r '.score // 10' 2>/dev/null)
[ "$score" = 10 ] && exit 0

CFG=${CLAUDE_CONFIG_DIR:-$HOME/.claude}
state_dir="$CFG/bilan"
mkdir -p "$state_dir" 2>/dev/null || exit 0
state="$state_dir/$(printf '%s' "$sid" | tr -c 'a-zA-Z0-9._-' '_').json"

signature=$(printf '%s' "$report" | jq -r '[.caps[] | .evidence] | sort | join("|")' 2>/dev/null \
            | shasum 2>/dev/null | cut -d' ' -f1)
[ -n "$signature" ] || exit 0
[ "$(jq -r '.signature // empty' "$state" 2>/dev/null)" = "$signature" ] && exit 0

jq -n --arg s "$signature" --arg at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --argjson score "$score" \
   '{signature:$s, blockedAt:$at, score:$score}' > "$state" 2>/dev/null || true

reason=$(printf '%s' "$report" | jq -r '
    "bilan says this session is at \(.score)/10, not done. Outstanding:\n"
    + ([.caps[] | "  · \(.evidence)\n    → \(.remedy)"] | join("\n"))
    + "\n\nFinish these, then report the number bilan gives — not your own. If one of them is "
    + "deliberate, say which and why in your next message rather than leaving it unstated."')

jq -n --arg r "$reason" '{decision:"block", reason:$r}'
exit 0
