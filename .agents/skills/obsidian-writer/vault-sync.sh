#!/usr/bin/env bash
# ABOUTME: Syncs ../dravr-vault remote-first — origin/main wins, local uncommitted work is stashed around it
# ABOUTME: `pull` before reading or writing the vault, `push <path>… -m <msg>` to commit and publish your notes
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# ChefFamille's rule (2026-09-27): the vault's REMOTE is the source of truth. Local
# main used to drift from origin for weeks (obsidian-git merges, sessions leaning on
# auto-save), and pushes were threaded past the divergence by hand. Now every sync:
#
#   1. fetches origin/main,
#   2. stashes any uncommitted work (tracked + untracked) — nothing is discarded,
#   3. rebases local commits onto origin/main with `-X ours`, which in a rebase means
#      the UPSTREAM side wins every conflicting hunk; if the rebase still stops
#      (delete/modify, binary), local main is saved to `vault-sync/backup-<ts>` and
#      reset to origin/main,
#   4. re-applies the stash; a conflicting stash is left in `git stash list` and the
#      tree is returned to the remote state, never half-merged.
#
# Usage:
#   vault-sync.sh pull
#   vault-sync.sh push -m "worklog: <what>" <path> [<path>…]   # paths relative to the vault
#   VAULT_DIR=/elsewhere vault-sync.sh pull

set -euo pipefail

VAULT_DIR="${VAULT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)/../dravr-vault}"
BRANCH=main

die() { echo "vault-sync: $*" >&2; exit 1; }
note() { echo "vault-sync: $*"; }
g() { git -C "$VAULT_DIR" "$@"; }

[ -d "$VAULT_DIR/.git" ] || die "no vault checkout at $VAULT_DIR"
VAULT_DIR="$(cd "$VAULT_DIR" && pwd)"

sync_remote_first() {
  local current stashed=0 ts
  current="$(g symbolic-ref --short HEAD 2>/dev/null || true)"
  [ "$current" = "$BRANCH" ] || die "vault is on '${current:-detached HEAD}', expected $BRANCH — refusing to move it"
  for marker in rebase-merge rebase-apply MERGE_HEAD; do
    [ ! -e "$(g rev-parse --git-path "$marker")" ] || die "a $marker is in progress in the vault — resolve it first"
  done

  g fetch origin "$BRANCH" --quiet || die "fetch failed"
  ts="$(date +%Y%m%d-%H%M%S)"

  if [ -n "$(g status --porcelain)" ]; then
    g stash push --include-untracked --quiet -m "vault-sync $ts local work" || die "stash failed"
    stashed=1
    note "stashed local uncommitted work (vault-sync $ts)"
  fi

  if ! g rebase --quiet -X ours "origin/$BRANCH" >/dev/null 2>&1; then
    g rebase --abort >/dev/null 2>&1 || true
    g branch "vault-sync/backup-$ts" HEAD
    g reset --quiet --hard "origin/$BRANCH"
    note "rebase could not auto-resolve; local main saved as vault-sync/backup-$ts and reset to origin/$BRANCH"
  fi

  if [ "$stashed" = 1 ]; then
    if g stash pop --quiet >/dev/null 2>&1; then
      note "re-applied local work on top of origin/$BRANCH"
    else
      g reset --quiet --merge
      note "local work conflicts with origin/$BRANCH — tracked files kept at remote, the full work kept in 'git -C $VAULT_DIR stash list' (vault-sync $ts)"
    fi
  fi

  note "at $(g rev-parse --short HEAD) ($(g rev-list --count "origin/$BRANCH..HEAD") ahead of origin/$BRANCH)"
}

cmd="${1:-}"; shift || true
case "$cmd" in
  pull)
    sync_remote_first
    ;;
  push)
    msg=""
    paths=()
    while [ $# -gt 0 ]; do
      case "$1" in
        -m) [ $# -ge 2 ] || die "-m needs a message"; msg="$2"; shift 2 ;;
        *) paths+=("$1"); shift ;;
      esac
    done
    [ -n "$msg" ] || die "push needs -m \"<message>\""
    [ "${#paths[@]}" -gt 0 ] || die "push needs at least one path — never the whole tree"

    g add -- "${paths[@]}"
    if g diff --cached --quiet -- "${paths[@]}"; then
      note "nothing to commit in the given paths"
    else
      g commit --quiet -m "$msg" -- "${paths[@]}"
    fi
    for attempt in 1 2 3; do
      sync_remote_first
      if g push --quiet origin "HEAD:$BRANCH"; then
        note "pushed $(g rev-parse --short HEAD) to origin/$BRANCH"
        exit 0
      fi
      note "push rejected (attempt $attempt) — re-syncing against the new remote tip"
    done
    die "push still rejected after 3 attempts"
    ;;
  *)
    die "usage: vault-sync.sh pull | push -m \"<message>\" <path>…"
    ;;
esac
