#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: UserPromptSubmit hook — prints the live claim status of every carnet issue the prompt names
# ABOUTME: Best-effort and deterministic: caches each lookup for a minute and exits 0 on every failure
#
# Wire it in .claude/settings.json (cwd is the project root when a hook runs):
#   "UserPromptSubmit": [{ "matcher": "", "hooks": [{ "type": "command", "timeout": 20,
#     "command": "[ -f .agents/skills/carnet/hooks/prompt-status.sh ] && bash .agents/skills/carnet/hooks/prompt-status.sh || true" }]}]
#
# Whatever this prints lands in the model's context before it answers, so a session that is
# told "carnet#197 · held by @jfarcand · session i18Guards [running]" cannot start the same
# work without knowing. One gh call per issue mentioned, at most five per prompt.
#
# It also writes the numbers it found to carnet-claims/pending/<session>.txt, which is what
# the PreToolUse hook claims from on the session's first edit. Knowing is not holding, and a
# claim that waits for the model to remember is the failure the claim exists to prevent.
set -uo pipefail

here=$(cd "$(dirname "$0")" && pwd)
carnet="$here/../carnet.sh"
[ -x "$carnet" ] || [ -f "$carnet" ] || exit 0
command -v jq >/dev/null 2>&1 || exit 0
command -v gh >/dev/null 2>&1 || exit 0

payload=$(cat 2>/dev/null || true)
prompt=$(printf '%s' "$payload" | jq -r '.prompt // empty' 2>/dev/null || true)
[ -n "$prompt" ] || exit 0

# carnet#12 · carnet 12 · carnet-12 · registre#12 · …/dravr-carnet/issues/12
nums=$(printf '%s' "$prompt" \
    | grep -oiE '(carnet|registre)[ #-]?[0-9]+|carnet/issues/[0-9]+' \
    | grep -oE '[0-9]+$' | sort -un | head -5 || true)

# Hand the numbers to the PreToolUse hook. A prompt that names none leaves an earlier
# list alone: work often spans several turns, and only the first turn carries the number.
# auto-claim.sh clears the file once it has acted, and ignores one older than an hour.
if [ -n "$nums" ]; then
    sid=$(printf '%s' "$payload" | jq -r '.session_id // empty' 2>/dev/null || true)
    [ -n "$sid" ] || sid=${CLAUDE_CODE_SESSION_ID:-}
    if [ -n "$sid" ]; then
        pending_dir="${CLAUDE_CONFIG_DIR:-$HOME/.claude}/carnet-claims/pending"
        mkdir -p "$pending_dir" 2>/dev/null && printf '%s\n' $nums > "$pending_dir/$sid.txt"
    fi
fi

[ -n "$nums" ] || exit 0

cache_dir="${CLAUDE_CONFIG_DIR:-$HOME/.claude}/carnet-claims/cache"
mkdir -p "$cache_dir" 2>/dev/null || exit 0

for n in $nums; do
    cache="$cache_dir/$n"
    if [ -f "$cache" ] && [ -n "$(find "$cache" -mmin -1 2>/dev/null)" ]; then
        cat "$cache"
        continue
    fi
    if line=$(bash "$carnet" status "$n" --short 2>/dev/null) && [ -n "$line" ]; then
        printf '%s\n' "$line" > "$cache"
        printf '%s\n' "$line"
    fi
done
exit 0
