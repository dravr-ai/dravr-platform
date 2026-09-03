#!/usr/bin/env bash
# ABOUTME: Prepares a feature branch for landing: rebases onto origin/main, runs the pre-push gate, pushes
# ABOUTME: Records the branch and worktree for merge-and-cleanup.sh and prints how to watch CI
#
# Licensed under either of Apache License, Version 2.0 or MIT License at your option.
# Copyright (c) 2026 dravr.ai

set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/../lib/worktree.sh"

BRANCH_NAME="$(git branch --show-current)"
MAIN_BRANCH="main"

if [[ -z "$BRANCH_NAME" ]]; then
    echo "Error: detached HEAD. Check out the feature branch first."
    exit 1
fi

if [[ "$BRANCH_NAME" == "$MAIN_BRANCH" ]]; then
    echo "Error: already on $MAIN_BRANCH. Switch to the feature branch first."
    exit 1
fi

if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
    echo "Error: uncommitted changes on $BRANCH_NAME. Commit them first; a rebase must not carry loose edits."
    git status --short
    exit 1
fi

echo "Finishing branch: $BRANCH_NAME"
echo ""

echo "Fetching origin/$MAIN_BRANCH..."
git fetch origin "$MAIN_BRANCH"

if [[ "$(git rev-parse "origin/$MAIN_BRANCH")" != "$(git merge-base HEAD "origin/$MAIN_BRANCH")" ]]; then
    echo "Rebasing onto origin/$MAIN_BRANCH..."
    if ! git rebase "origin/$MAIN_BRANCH"; then
        echo ""
        echo "Error: the rebase stopped on a conflict. Resolve it, 'git rebase --continue', then rerun."
        exit 1
    fi
    echo "Rebase complete."
else
    echo "Branch is already up to date with $MAIN_BRANCH."
fi

echo ""
# The only local gate: it runs the tiers whose files changed and writes the
# per-commit marker the pre-push hook checks. The heavy gates run in CI.
echo "Running the pre-push gate..."
./scripts/ci/pre-push-validate.sh

echo ""
echo "Pushing $BRANCH_NAME to origin..."
git push --force-with-lease origin "$BRANCH_NAME"

# Save the branch and worktree for merge-and-cleanup.sh, in the main worktree.
echo "$BRANCH_NAME|$(current_worktree_root)" > "$(last_branch_file)"

cat <<STEPS

Branch pushed.

==========================================
NEXT STEPS
==========================================

1. Watch CI for the branch until every lane is terminal (re-check on a
   schedule; never 'gh run watch', no loop under 60s):
     gh run list --branch $BRANCH_NAME
     https://github.com/dravr-ai/dravr-platform/actions?query=branch%3A$BRANCH_NAME

   A feature/* ref runs the full PostgreSQL suite; a fix/* ref runs only the
   8-file smoke, so green there is not a full verdict.

2. Once every lane is green, land it from the main worktree:
     cd $MAIN_WORKTREE
     ./.claude/skills/finish-worktree/merge-and-cleanup.sh -m "<subject>

<body>"

   (Branch and worktree are saved - no other arguments needed.)
STEPS
