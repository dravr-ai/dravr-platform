#!/usr/bin/env bash
# ABOUTME: Names the newest commit a dev deploy should build: the newest green main commit that contains the trigger
# ABOUTME: Reads green Backend CI head shas on stdin; prints one sha; never names a commit outside the trigger's line
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   publish-images.yml deploys when a Backend CI run on main completes green,
#   and its concurrency group keeps one waiting run that a newer ARRIVAL
#   replaces. CI runs finish in whatever order the runners allow, so an older
#   commit whose CI finishes late can replace a newer green commit's waiting
#   run — and building the triggering commit would then strand the newer one
#   until the next push. Building the newest green commit instead makes the
#   arrival order irrelevant: whichever run gets to build, it builds the newest
#   validated work.
#
# Usage:
#   <green shas, one per line> | pick-newest-green.sh <trigger-sha> [repo-path]
#
#   trigger-sha  the commit whose green CI run started this deploy (required, hex)
#   stdin        head shas of Backend CI push runs on main that concluded success;
#                any order, duplicates and blank lines allowed
#   repo-path    a git repository whose HEAD is main's tip (default: .) — the
#                workflow's checkout of the default branch
#
# Output: one sha on stdout — the trigger itself, or a green commit that
# contains it and is contained by no other such candidate. Exit 2 on misuse.
#
# Fail-safe by design: a candidate that is malformed, unknown to this
# repository, not a descendant of the trigger, or no longer on main (HEAD) —
# a commit a rewrite of main dropped — is ignored, so the worst
# answer is the trigger — exactly what the deploy built before this existed.
# Only commits CI validated can be named, and never one older than the trigger
# (check-deploy-ancestry.sh still refuses anything dev already serves).
set -euo pipefail

trigger="${1:-}"
repo="${2:-.}"
hex='^[0-9a-f]{7,40}$'

if [[ ! "$trigger" =~ $hex ]]; then
  echo "usage: <green shas> | $0 <trigger-sha> [repo-path]" >&2
  echo "trigger sha must be 7-40 lowercase hex characters, got '${trigger}'" >&2
  exit 2
fi
if ! git -C "$repo" rev-parse --git-dir >/dev/null 2>&1; then
  echo "no git repository at '${repo}'" >&2
  exit 2
fi
if ! git -C "$repo" cat-file -e "${trigger}^{commit}" 2>/dev/null; then
  echo "trigger ${trigger} is not a commit in '${repo}'" >&2
  exit 2
fi

best="$trigger"
while IFS= read -r candidate; do
  [[ "$candidate" =~ $hex ]] || continue
  git -C "$repo" cat-file -e "${candidate}^{commit}" 2>/dev/null || continue
  git -C "$repo" merge-base --is-ancestor "$candidate" HEAD || continue
  # A newer best must contain the current one. On main's linear history this
  # walks forward to the descendant-most green commit whatever the input order;
  # a candidate on another line of history never qualifies.
  if git -C "$repo" merge-base --is-ancestor "$best" "$candidate"; then
    best="$candidate"
  fi
done

git -C "$repo" rev-parse "${best}^{commit}"
