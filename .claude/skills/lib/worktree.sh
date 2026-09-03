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
