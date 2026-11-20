#!/usr/bin/env bash
# ABOUTME: The one definition of what a cloud clippy run linted, and the record that lets Tier 1e trust it
# ABOUTME: patch = the diff sent to the cloud; tree = its git tree id; record/check pin a CLEAN verdict to it
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Clippy on this repo runs in a Claude Code on the web session (the remote-work skill in
# dravr-savoir). A shell cannot reach that session, so pre-push-validate.sh Tier 1e cannot ask
# it anything. Instead, the session records the CLEAN verdict it read, pinned to the git TREE
# that was linted, and Tier 1e skips its local compile only when HEAD's tree is that tree.
# A tree id is equal exactly when the content is equal, so an edit after the run, an untracked
# file the run saw and the commit didn't, or a verdict for another branch all fall back to
# the local compile. Nothing here ever weakens the gate: no record, or any mismatch, means
# Tier 1e runs as before.
#
#   remote-clippy-verdict.sh patch <base>              the diff the cloud applies: tracked changes
#                                                      against <base> plus every untracked file
#   remote-clippy-verdict.sh tree                      tree id of the working state (index untouched)
#   remote-clippy-verdict.sh sent <patch-sha> <scope>    prepare notes which scope it sent with which patch
#   remote-clippy-verdict.sh record <patch-sha> <scope>  after VERDICT: CLEAN; refuses a report whose
#                                                      patch sha is not this checkout's patch, or a
#                                                      scope that was never sent with that patch
#   remote-clippy-verdict.sh check                     exit 0 when a workspace CLEAN covers HEAD's tree
#
# The record lives in the per-worktree git dir, as <git-dir>/remote-clippy-verdict, and the sent
# ledger beside it as <git-dir>/remote-clippy-sent. The ledger is what stops a CLEAN from a
# narrower run (a `cargo check`, one package) being recorded as a workspace clippy: the patch
# sha alone is the same for every command run on the same tree.

set -euo pipefail

die() { echo "remote-clippy-verdict: $*" >&2; exit 2; }

ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || die "not inside a git checkout"
cd "$ROOT"
RECORD="$(git rev-parse --absolute-git-dir)/remote-clippy-verdict"
SENT="$(git rev-parse --absolute-git-dir)/remote-clippy-sent"

sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum | cut -c1-64; else shasum -a 256 | cut -c1-64; fi; }

cmd_patch() {
    local base="${1:?usage: patch <base>}"
    # Each step fails loudly: record reads this through a command substitution, where bash
    # clears `set -e`, and a patch that silently lost a file would pin a tree nobody linted.
    git diff --binary "$base" || die "git diff against $base failed"
    # Untracked files reach the cloud as creations. `--no-index` exits 1 when files differ,
    # so 1 is the expected status; anything above it is git failing.
    # The list goes through a file, not `< <(…)`: a process substitution's exit status is lost.
    local f status untracked
    untracked=$(mktemp)
    git ls-files --others --exclude-standard -z >"$untracked" || { rm -f "$untracked"; die "git ls-files failed"; }
    while IFS= read -r -d '' f; do
        status=0
        git diff --no-index --binary /dev/null "$f" || status=$?
        [ "$status" -le 1 ] || { rm -f "$untracked"; die "git diff --no-index failed on $f (exit $status)"; }
    done <"$untracked"
    rm -f "$untracked"
}

# The working state as a tree, written through a throwaway index: the real index, the tree
# and every other session's staged work stay exactly as they were.
cmd_tree() {
    local idx
    idx=$(mktemp)
    rm -f "$idx"
    # `|| die` on each step, not `set -e`: record calls this as `$(cmd_tree)`, where bash
    # clears errexit, and a failed add would otherwise still write HEAD's tree.
    GIT_INDEX_FILE="$idx" git read-tree HEAD || die "git read-tree HEAD failed"
    GIT_INDEX_FILE="$idx" git add -A || die "git add into the throwaway index failed"
    GIT_INDEX_FILE="$idx" git write-tree || die "git write-tree failed"
    rm -f "$idx"
}

cmd_sent() {
    local sha="${1:?usage: sent <patch-sha> <scope>}" scope="${2:?usage: sent <patch-sha> <scope>}"
    printf '%s\t%s\n' "$sha" "$scope" >> "$SENT"
}

cmd_record() {
    local reported="${1:?usage: record <patch-sha> <scope>}" scope="${2:?usage: record <patch-sha> <scope>}"
    local base actual tree
    git fetch -q origin main 2>/dev/null || true
    base=$(git merge-base HEAD origin/main) || die "no merge-base with origin/main"
    actual=$(cmd_patch "$base" | sha256)
    [ "$reported" = "$actual" ] || die "the report's patch sha $reported is not this checkout's ($actual): the tree changed since the run, or the report is another run's. Nothing recorded."
    [ -f "$SENT" ] && grep -qxF -- "$(printf '%s\t%s' "$actual" "$scope")" "$SENT" \
        || die "no run with scope '$scope' was prepared for this patch ($actual): a CLEAN counts only for the command that produced it. Nothing recorded."
    tree=$(cmd_tree)
    printf 'verdict=CLEAN\nscope=%s\ntree=%s\nbase=%s\npatch_sha=%s\nrecorded=%s\n' \
        "$scope" "$tree" "$base" "$actual" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$RECORD"
    echo "recorded: CLEAN ($scope) for tree $tree"
}

field() { sed -n "s/^$1=//p" "$RECORD" | head -1; }

cmd_check() {
    [ -f "$RECORD" ] || { echo "no cloud clippy verdict recorded"; return 1; }
    local verdict scope tree head
    verdict=$(field verdict); scope=$(field scope); tree=$(field tree)
    head=$(git rev-parse 'HEAD^{tree}')
    [ "$verdict" = "CLEAN" ] || { echo "recorded verdict is '$verdict', not CLEAN"; return 1; }
    # Tier 1e lints server test targets; only a workspace run covers every one of them.
    [ "$scope" = "workspace" ] || { echo "recorded scope is '$scope', not workspace"; return 1; }
    [ "$tree" = "$head" ] || { echo "recorded tree $tree is not HEAD's tree $head"; return 1; }
    echo "cloud clippy CLEAN (workspace) covers HEAD's tree $head"
}

case "${1:-}" in
    patch) shift; cmd_patch "$@" ;;
    tree) cmd_tree ;;
    sent) shift; cmd_sent "$@" ;;
    record) shift; cmd_record "$@" ;;
    check) cmd_check ;;
    *) sed -n '17,24p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
