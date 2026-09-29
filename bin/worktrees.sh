#!/usr/bin/env bash
# ABOUTME: Lists this repository's worktrees with the Claude Code session that owns each one
# ABOUTME: Reads the git-dir stamp claim_worktree writes; an unstamped tree prints as unowned
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# `git worktree list` says where and which branch; it cannot say whose. With
# several sessions each driving lanes in their own trees, "whose is this?" is
# the question the operator actually has. The answer is the stamp the creating
# session wrote (see claim_worktree in .build/skills/lib/worktree.sh); a tree
# made by hand, without the helper, prints "-" and `claim` fixes that:
#
#   bin/worktrees.sh            list every worktree with its owner
#   bin/worktrees.sh claim [p]  stamp the worktree at p (default: the current one) as this session's
set -euo pipefail

# The library ships in .build (dravr-build-config). Prefer this tree's copy; a
# worktree made by hand has an empty .build, so fall back to the main
# worktree's, which bootstrap keeps initialised.
worktree_lib=""
here="$(dirname "${BASH_SOURCE[0]}")"
for root in "$(git -C "$here" rev-parse --show-toplevel)" \
            "$(git -C "$here" worktree list --porcelain | sed -n 's/^worktree //p' | head -1)"; do
    if [[ -f "$root/.build/skills/lib/worktree.sh" ]]; then
        worktree_lib="$root/.build/skills/lib/worktree.sh"
        break
    fi
done
if [[ -z "$worktree_lib" ]]; then
    echo "bin/worktrees.sh: .build/skills/lib/worktree.sh is missing; run bash .build/ci/bootstrap-repo.sh" >&2
    exit 1
fi
# shellcheck source=/dev/null
source "$worktree_lib"

case "${1:-list}" in
    claim)
        target="${2:-.}"
        claim_worktree "$target"
        echo "claimed $(cd "$target" && pwd -P) for $(worktree_session_name)"
        ;;
    list)
        me="${CLAUDE_CODE_SESSION_ID:-}"
        printf '%-72s %-32s %-22s %s\n' WORKTREE BRANCH OWNER SINCE
        git worktree list --porcelain | awk '/^worktree /{print substr($0,10)}' | while read -r wt; do
            branch="$(git -C "$wt" branch --show-current 2>/dev/null)"
            [ -n "$branch" ] || branch="detached@$(git -C "$wt" rev-parse --short HEAD 2>/dev/null)"
            owner="$(worktree_owner "$wt" name)"
            sid="$(worktree_owner "$wt" session_id)"
            at="$(worktree_owner "$wt" claimed_at)"
            since=""
            [ -n "$at" ] && since="$(date -r "$at" '+%Y-%m-%d %H:%M' 2>/dev/null || echo "$at")"
            tag=""
            [ -n "$me" ] && [ "$sid" = "$me" ] && tag=" (this session)"
            printf '%-72s %-32s %-22s %s\n' "$wt" "$branch" "${owner:--}$tag" "$since"
        done
        ;;
    *)
        echo "usage: $0 [list|claim [path]]" >&2
        exit 2
        ;;
esac
