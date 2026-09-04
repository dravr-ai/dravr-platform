#!/usr/bin/env bash
# ABOUTME: Decides whether deploying a resolved commit would roll dev BACK behind the commit it serves
# ABOUTME: Exit 0 = deploy, 1 = skip (resolved is already contained in the live commit), 2 = misuse
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   `publish-images.yml` builds the TRIGGERING CI run's commit on a workflow_run
#   event, and CI runs on main finish in whatever order the runners allow. So a
#   deploy for an older commit can complete AFTER a deploy for a newer one, and
#   dev — which is the environment the team actually uses — moves backwards
#   with every run green. Observed 2026-09-04 00:56Z: run 33822126457 put
#   2ad52f348 on dev twenty minutes after e49774745 was already serving
#   (carnet#262).
#
#   A deploy must never move dev to a commit the serving commit already
#   contains. That is an ancestry question, so it is answered with git.
#
# Usage:
#   check-deploy-ancestry.sh <resolved-sha> <live-sha> [repo-path]
#
#   resolved-sha  the commit the deploy is about to build (required, hex)
#   live-sha      the `commit-sha` label of the revision dev is serving; may be
#                 empty or garbage, because it is read from an external label
#   repo-path     a git repository holding enough history to relate the two
#                 (default: the current directory)
#
# Verdict, on stdout and as the exit code:
#   exit 0  deploy — the resolved commit is not contained in the live one, or
#           there is no live commit to compare against
#   exit 1  skip   — the live commit already contains the resolved commit
#                    (including the identical commit), so deploying would roll
#                    dev back or rebuild what is already serving
#   exit 2  misuse — no resolved sha, a malformed one, or no repository
#
# Fail-open by design: every case where the live commit cannot be resolved —
# absent label, a value that is not a sha, a sha this repository has never
# seen — is a deploy, never a skip. The guard exists to stop a rollback, and a
# guard that could block a legitimate deploy would be removed the first time it
# did. The only time it fires is when git can PROVE the deploy is a rollback.
set -euo pipefail

resolved="${1:-}"
live="${2:-}"
repo="${3:-.}"

hex='^[0-9a-f]{7,40}$'

if [[ ! "$resolved" =~ $hex ]]; then
  echo "usage: $0 <resolved-sha> <live-sha> [repo-path]" >&2
  echo "resolved sha must be 7-40 lowercase hex characters, got '${resolved}'" >&2
  exit 2
fi
if ! git -C "$repo" rev-parse --git-dir >/dev/null 2>&1; then
  echo "no git repository at '${repo}'" >&2
  exit 2
fi

if [ -z "$live" ]; then
  echo "deploy: dev carries no commit-sha label, nothing to compare ${resolved} against"
  exit 0
fi
if [[ ! "$live" =~ $hex ]]; then
  echo "deploy: live label '${live}' is not a commit sha, nothing to compare ${resolved} against"
  exit 0
fi
if ! git -C "$repo" cat-file -e "${live}^{commit}" 2>/dev/null; then
  echo "deploy: live commit ${live} is not in this repository's history, nothing to compare ${resolved} against"
  exit 0
fi
if ! git -C "$repo" cat-file -e "${resolved}^{commit}" 2>/dev/null; then
  echo "deploy: resolved commit ${resolved} is not in the fetched history, so ancestry cannot be decided here"
  exit 0
fi

# --is-ancestor is true for the identical commit too, which is the redeploy of
# what dev already serves: a new revision for zero code change.
if git -C "$repo" merge-base --is-ancestor "$resolved" "$live"; then
  if [ "$(git -C "$repo" rev-parse "$resolved")" = "$(git -C "$repo" rev-parse "$live")" ]; then
    echo "skip: ${resolved} is the commit dev already serves"
  else
    echo "skip: ${resolved} is an ancestor of ${live}, which dev already serves — deploying it would roll dev back"
  fi
  exit 1
fi

echo "deploy: ${resolved} is not contained in ${live}, which dev serves"
exit 0
