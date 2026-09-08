#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: SessionStart hook — reports what a session that died left behind, across every worktree
# ABOUTME: The only cover for a kill -9: a dead session fires no exit hook and cannot report on itself
#
# Wire it in .claude/settings.json alongside the other SessionStart hooks.
#
# A stopped session is caught by the Stop gate and a clean exit by carnet's SessionEnd release.
# Neither fires when the process is killed, the terminal is closed, or the context runs out —
# and that is the case that has been costing whole nights. Nothing inside the dead session can
# find it, so the next session in the repo looks for it instead.
set -uo pipefail

here=$(cd "$(dirname "$0")" && pwd)
bilan="$here/../bilan.sh"
[ -f "$bilan" ] || exit 0

out=$(bash "$bilan" sweep 2>/dev/null) || exit 0
printf '%s\n' "$out" | grep -q '✅ nothing left behind' && exit 0
printf '%s\n' "$out"
exit 0
