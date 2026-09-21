#!/usr/bin/env bash
# ABOUTME: The worktree facts the three worktree skills share — which root, which path, which hand-off file
# ABOUTME: Sourced, never executed; every function echoes one value and touches nothing
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# `git rev-parse --show-toplevel` means opposite things in the two scripts that
# called it: in create/finish-worktree it is the FEATURE worktree the caller is
# standing in, and in merge-and-cleanup it is expected to be MAIN. Reading the
# same expression as two different facts is how a cleanup run from the wrong
# directory removes the wrong tree, so the two are named apart here and neither
# script spells the expression again.

# The worktree the caller is standing in.
current_worktree_root() {
    git rev-parse --show-toplevel
}

# The repository's main worktree — always the first entry `git worktree list`
# reports, whichever tree the caller is standing in.
main_worktree_root() {
    git worktree list --porcelain | sed -n 's/^worktree //p' | head -1
}

# Where create-worktree.sh puts a feature worktree, and therefore where
# merge-and-cleanup.sh looks for it when the caller names only a branch.
#
# The `pierre_mcp_server-` prefix predates the repository's rename to
# dravr-platform; existing worktrees on every machine carry it, so it stays.
feature_worktree_path() {
    local branch="$1"
    echo "$(dirname "$(main_worktree_root)")/pierre_mcp_server-${branch//\//-}"
}

# The hand-off file finish-worktree.sh writes and merge-and-cleanup.sh reads.
# It lives in the main worktree because that is where the cleanup runs.
last_branch_file() {
    echo "$(main_worktree_root)/.claude/skills/.last-feature-branch"
}

# ---------------------------------------------------------------- ownership stamp
# Which Claude Code session a worktree belongs to. Nothing in git records it,
# and the status line can only describe the directory a session sits in — so a
# session driving lanes in three worktrees reads as "on main", and the operator
# cannot tell whose tree is whose. The stamp lives in the worktree's own git-dir
# (`.git/worktrees/<name>/claude-session`): outside the working tree, so it never
# dirties it, and removed with the worktree by `git worktree remove`. Written by
# create-worktree.sh and by any lane that adopts a slot; read by the status
# line and by bin/worktrees.sh.

worktree_stamp_file() { # $1 = worktree path (default: the caller's)
    local dir="${1:-.}"
    local gitdir
    gitdir="$(git -C "$dir" rev-parse --git-dir 2>/dev/null)" || return 1
    case "$gitdir" in /*) ;; *) gitdir="$(cd "$dir" && cd "$gitdir" && pwd)" ;; esac
    echo "$gitdir/claude-session"
}

# The session's display name: what /rename set, else the id prefix, else the
# shell user when called outside Claude Code. Same resolution carnet.sh uses.
worktree_session_name() {
    local cfg="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
    local sid="${CLAUDE_CODE_SESSION_ID:-}" pid="${CLAUDE_PID:-}" n=""
    if [ -n "$sid" ] && [ -n "$pid" ] && [ -f "$cfg/sessions/$pid.json" ]; then
        n=$(jq -r --arg sid "$sid" 'select(.sessionId == $sid) | .name // empty' \
            "$cfg/sessions/$pid.json" 2>/dev/null || true)
    fi
    [ -n "$n" ] || n="${sid:0:8}"
    [ -n "$n" ] || n="${USER:-shell}"
    printf '%s' "$n"
}

# Stamp a worktree as this session's. Idempotent; re-stamping moves ownership.
claim_worktree() { # $1 = worktree path (default: the caller's)
    local f
    f="$(worktree_stamp_file "${1:-.}")" || return 1
    {
        printf 'session_id=%s\n' "${CLAUDE_CODE_SESSION_ID:-shell}"
        printf 'name=%s\n' "$(worktree_session_name)"
        printf 'pid=%s\n' "${CLAUDE_PID:-$$}"
        printf 'host=%s\n' "$(hostname -s 2>/dev/null || hostname)"
        printf 'claimed_at=%s\n' "$(date +%s)"
    } > "$f"
}

# One field of a worktree's stamp: session_id, name, pid, host or claimed_at.
# Empty when the worktree carries no stamp.
worktree_owner() { # $1 = worktree path, $2 = field (default: name)
    local f field="${2:-name}"
    f="$(worktree_stamp_file "$1")" || return 0
    [ -f "$f" ] || return 0
    sed -n "s/^${field}=//p" "$f" | head -1
}
