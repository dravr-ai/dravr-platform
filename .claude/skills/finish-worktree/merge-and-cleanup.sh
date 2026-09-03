#!/usr/bin/env bash
# ABOUTME: Squash-merges a CI-green feature branch onto main, pushes, then cleans up the branch and worktree
# ABOUTME: Refuses to touch anything it cannot land safely; cleanup runs only after the push is confirmed
#
# Licensed under either of Apache License, Version 2.0 or MIT License at your option.
# Copyright (c) 2026 dravr.ai

set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/../lib/worktree.sh"

usage() {
    cat <<'USAGE'
Usage: merge-and-cleanup.sh [-m <message> | -F <file>] [branch-name] [worktree-path]

Squash-merges origin/<branch-name> onto main, runs the pre-push gate, pushes
main, and only then removes the worktree and deletes the branch. Run it from
the main worktree, on main, after CI is green on the branch.

  -m <message>   Commit message (subject line, blank line, body). Pass a
                 multi-line string; the commit-msg hook enforces the shape.
  -F <file>      Read the commit message from <file>.

With neither, an interactive terminal opens the editor with the branch's
commit subjects prefilled; a non-interactive run fails instead of guessing.

Without arguments the branch and worktree come from the file written by
finish-worktree.sh. The worktree path defaults to the create-worktree layout.

What it refuses, and why:
  - not on main, or not in the main worktree: the squash must advance the one
    shared main ref.
  - uncommitted changes that overlap the branch's files: someone's work in
    progress would be swallowed by the squash.
  - local main that cannot fast-forward to origin/main: unpushed local commits
    or a divergence must be reconciled by a person, not merged around.
  - a rejected push: main moved while the gate ran. The squash commit stays on
    local main; the fix is printed, and nothing is cleaned up.
USAGE
    exit 1
}

MESSAGE=""
MESSAGE_FILE=""
while getopts ":m:F:h" opt; do
    case "$opt" in
        m) MESSAGE="$OPTARG" ;;
        F) MESSAGE_FILE="$OPTARG" ;;
        h) usage ;;
        *) usage ;;
    esac
done
shift $((OPTIND - 1))

MAIN_WORKTREE="$(main_worktree_root)"
LAST_BRANCH_FILE="$(last_branch_file)"

if [[ "$(current_worktree_root)" != "$MAIN_WORKTREE" ]]; then
    echo "Error: run this from the main worktree ($MAIN_WORKTREE), not from $(current_worktree_root)"
    exit 1
fi

if [[ $# -ge 1 ]]; then
    BRANCH_NAME="$1"
    WORKTREE_PATH="${2:-$(feature_worktree_path "$BRANCH_NAME")}"
elif [[ -f "$LAST_BRANCH_FILE" ]]; then
    SAVED_INFO="$(cat "$LAST_BRANCH_FILE")"
    BRANCH_NAME="${SAVED_INFO%%|*}"
    WORKTREE_PATH="${SAVED_INFO##*|}"
    echo "Using saved branch: $BRANCH_NAME"
    echo "Worktree: $WORKTREE_PATH"
    echo ""
else
    echo "Error: no branch specified and no saved branch found."
    echo "Run finish-worktree.sh first, or name the branch."
    echo ""
    usage
fi

if [[ "$(git branch --show-current)" != "main" ]]; then
    echo "Error: must be on main. Currently on: $(git branch --show-current)"
    exit 1
fi

echo "Fetching origin/main and origin/$BRANCH_NAME..."
git fetch origin main "$BRANCH_NAME"
REMOTE_BRANCH="origin/$BRANCH_NAME"

# CI validated what is on origin; merge exactly that. A local branch that is
# ahead of it carries commits no lane has seen.
if git rev-parse --verify --quiet "$BRANCH_NAME" >/dev/null; then
    if [[ "$(git rev-parse "$BRANCH_NAME")" != "$(git rev-parse "$REMOTE_BRANCH")" ]]; then
        echo "Error: local $BRANCH_NAME ($(git rev-parse --short "$BRANCH_NAME")) differs from"
        echo "       $REMOTE_BRANCH ($(git rev-parse --short "$REMOTE_BRANCH")). Push the branch and let CI run first."
        exit 1
    fi
fi

# The main worktree is shared. Work in progress that does not touch the
# branch's files survives a squash untouched (only staged paths get committed);
# work that does would be swallowed, so refuse rather than guess.
BRANCH_FILES="$(git diff --name-only "origin/main...$REMOTE_BRANCH")"
DIRTY_FILES="$(git status --porcelain --untracked-files=no | cut -c4- | sed 's/.* -> //')"
OVERLAP="$(comm -12 <(printf '%s\n' "$BRANCH_FILES" | sort -u) <(printf '%s\n' "$DIRTY_FILES" | sort -u) | sed '/^$/d')"
if [[ -n "$OVERLAP" ]]; then
    echo "Error: uncommitted changes in the main worktree overlap the branch:"
    printf '   %s\n' "$OVERLAP"
    echo "Commit or stash that work (it is not yours to discard), then rerun."
    exit 1
fi

echo "Fast-forwarding local main to origin/main..."
if ! git pull --ff-only origin main; then
    echo ""
    echo "Error: local main cannot fast-forward to origin/main."
    echo "It has unpushed commits or has diverged; reconcile it first"
    echo "(git log --oneline origin/main..main shows what only exists locally)."
    exit 1
fi

echo "Squash-merging $REMOTE_BRANCH..."
if ! git merge --squash "$REMOTE_BRANCH"; then
    echo ""
    echo "Error: the squash did not apply cleanly. Restoring the index and tree."
    git reset --merge
    echo "Rebase the branch onto origin/main in its worktree (finish-worktree.sh), let CI run, then rerun."
    exit 1
fi

echo ""
echo "Staged for the squash commit:"
git diff --cached --name-only | sed 's/^/   /'
echo ""

TMP_MESSAGE="$(mktemp)"
trap 'rm -f "$TMP_MESSAGE"' EXIT
if [[ -n "$MESSAGE" ]]; then
    printf '%s\n' "$MESSAGE" > "$TMP_MESSAGE"
elif [[ -n "$MESSAGE_FILE" ]]; then
    cp "$MESSAGE_FILE" "$TMP_MESSAGE"
elif [[ -t 0 ]]; then
    {
        echo ""
        echo ""
        echo "# Squash of $BRANCH_NAME. Line 1: subject (<=72 chars). Line 2: blank."
        echo "# Line 3+: what changed and why. Lines starting with # are dropped."
        echo "# The branch's commits, oldest first:"
        git log --reverse --format='#   %s' "origin/main..$REMOTE_BRANCH"
    } > "$TMP_MESSAGE"
    "${EDITOR:-vi}" "$TMP_MESSAGE"
    sed -i.bak '/^#/d' "$TMP_MESSAGE" && rm -f "$TMP_MESSAGE.bak"
else
    echo "Error: no commit message. Pass -m or -F when running non-interactively."
    git reset --merge
    exit 1
fi

if [[ -z "$(sed '/^[[:space:]]*$/d' "$TMP_MESSAGE")" ]]; then
    echo "Error: empty commit message. The squash is undone; rerun with a message."
    git reset --merge
    exit 1
fi

# Only what the squash staged is committed; a peer's unrelated edits stay
# where they are.
git commit -F "$TMP_MESSAGE"
echo ""

# The pre-push hook checks a per-commit marker; the feature worktree's marker
# is for another commit, so the gate runs again here, on the squash.
echo "Running the pre-push gate on the squash commit..."
./scripts/ci/pre-push-validate.sh

# Main can move while the gate runs (auto-bumps land on their own). Look
# before pushing so the failure names itself instead of surfacing as a
# rejected push, and never pipe the push: a pipe hides its exit status, and a
# masked rejection followed by cleanup is how a landed-looking squash got its
# worktree deleted out from under it.
git fetch origin main
if [[ "$(git rev-parse HEAD~1)" != "$(git rev-parse origin/main)" ]]; then
    echo ""
    echo "Error: origin/main moved to $(git rev-parse --short origin/main) while the gate ran."
    echo "The squash commit $(git rev-parse --short HEAD) is on local main. To land it:"
    echo "   git pull --rebase origin main"
    echo "   ./scripts/ci/pre-push-validate.sh"
    echo "   git push origin main"
    echo "then rerun this script for the cleanup (it will find nothing to merge and only clean up)."
    exit 2
fi

echo "Pushing main..."
if ! git push origin main; then
    echo ""
    echo "Error: the push was rejected. The squash commit $(git rev-parse --short HEAD) is on local main."
    echo "Nothing was cleaned up. Reconcile with 'git pull --rebase origin main', rerun the gate, push, then rerun this script."
    exit 2
fi
LANDED="$(git rev-parse --short HEAD)"

# Cleanup, and only now: the squash is on origin/main.
if [[ -d "$WORKTREE_PATH" ]]; then
    echo ""
    echo "Removing worktree at $WORKTREE_PATH..."
    # A worktree created by create-worktree.sh carries the .build submodule; a
    # plain remove refuses it, so deinit first and force the removal.
    git -C "$WORKTREE_PATH" submodule deinit -f --all >/dev/null 2>&1 || true
    git worktree remove --force "$WORKTREE_PATH"
else
    echo ""
    echo "Worktree not found at $WORKTREE_PATH (already removed)."
fi
git worktree prune

if git rev-parse --verify --quiet "$BRANCH_NAME" >/dev/null; then
    # A squash is not seen as a merge, so only -D deletes the branch.
    git branch -D "$BRANCH_NAME"
fi

# Last, because it is a push too: the hook accepts it on the marker the gate
# just wrote for the squash commit.
echo "Deleting $REMOTE_BRANCH..."
git push origin --delete "$BRANCH_NAME"

rm -f "$LAST_BRANCH_FILE"

echo ""
echo "Landed $LANDED on main; branch and worktree cleaned up."
echo ""
echo "AFTER PUSHING - REQUIRED: watch main's CI for $LANDED until every lane is terminal."
echo "   gh run list --branch main --limit 15"
echo "   https://github.com/dravr-ai/dravr-platform/actions?query=branch%3Amain"
echo "Not 'gh run watch' and no loop under 60s; re-check on a schedule."
echo ""
