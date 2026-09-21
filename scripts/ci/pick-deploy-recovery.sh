#!/usr/bin/env bash
# ABOUTME: After a deploy is refused because its commit's Backend CI is not green, names the newest
# ABOUTME: green commit behind it that dev does not serve yet — or nothing, when there is none to deploy
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   `publish-images.yml` builds a commit while its Backend CI is still running
#   and deploys only once that run is green (carnet#498). Its concurrency group
#   keeps one waiting run, and a newer arrival replaces it — harmless while the
#   newer commit is green, because it contains the older one. But a commit that
#   turns out RED has by then displaced a green commit's waiting run, and when
#   its own deploy is refused the green work behind it reaches dev only when
#   somebody repairs main. Under the old completed-CI trigger a red commit
#   never entered the group at all, so that green commit would have deployed.
#
#   This script answers the one question the recovery needs: which commit, if
#   any, should be deployed instead.
#
# Usage:
#   pick-deploy-recovery.sh <refused-sha> <live-sha> [repo-path]
#
#   refused-sha  the commit whose deploy was refused (required, hex)
#   live-sha     the `commit-sha` label of the revision dev is serving
#   repo-path    a checkout holding main's commit history (default: .)
#
#   CI_VERDICT_CMD  command that prints a commit's Backend CI conclusion on
#                   stdout given the sha as $1 (default: the GitHub API, which
#                   needs GH_TOKEN and GITHUB_REPOSITORY). The fixture test
#                   points it at a table.
#
# Output: the sha to deploy on stdout, or nothing. The reason goes to stderr.
# Exit 0 = a decision was made (an empty stdout is a decision), 2 = misuse.
#
# Conservative where the rollback guard is fail-open, on purpose: the guard
# protects a deploy somebody asked for, this one starts a deploy nobody asked
# for. When what dev serves is unknown, or git cannot relate the two commits,
# the answer is "nothing" — waiting for the next green push costs time, moving
# dev backwards costs the team its environment.
set -euo pipefail

REFUSED="${1:-}"
LIVE="${2:-}"
REPO="${3:-.}"
# How far back a green commit is looked for. A burst is a handful of commits;
# past this, main has been red long enough that a person is already on it.
LOOKBACK=30

is_sha() { [[ "$1" =~ ^[0-9a-f]{7,40}$ ]]; }

if ! is_sha "$REFUSED"; then
  echo "usage: pick-deploy-recovery.sh <refused-sha> <live-sha> [repo-path]" >&2
  exit 2
fi
if ! git -C "$REPO" cat-file -e "${REFUSED}^{commit}" 2>/dev/null; then
  echo "refused commit ${REFUSED} is not in ${REPO}" >&2
  exit 2
fi

if ! is_sha "$LIVE" || ! git -C "$REPO" cat-file -e "${LIVE}^{commit}" 2>/dev/null; then
  echo "nothing: the commit dev serves is unknown ('${LIVE}'), so no deploy is started on a guess" >&2
  exit 0
fi

ci_verdict() { # $1 = sha; prints the conclusion of its push-triggered Backend CI run, or nothing
  if [ -n "${CI_VERDICT_CMD:-}" ]; then
    "$CI_VERDICT_CMD" "$1"
  else
    gh api "repos/${GITHUB_REPOSITORY}/actions/workflows/ci-backend.yml/runs?head_sha=$1&event=push&per_page=1" \
      --jq '.workflow_runs[0].conclusion // ""'
  fi
}

candidate=""
while read -r sha; do
  [ -n "$sha" ] || continue
  # Exit status first: an unreadable verdict is not a red one, and not a green one.
  if ! verdict="$(ci_verdict "$sha")"; then
    echo "nothing: could not read the Backend CI verdict of ${sha}" >&2
    exit 0
  fi
  if [ "$verdict" = "success" ]; then
    candidate="$sha"
    break
  fi
done < <(git -C "$REPO" rev-list --first-parent -n "$LOOKBACK" "${REFUSED}^" 2>/dev/null || true)

if [ -z "$candidate" ]; then
  echo "nothing: no green commit within ${LOOKBACK} commits behind ${REFUSED}" >&2
  exit 0
fi

# dev already serves the candidate, or something that contains it.
if git -C "$REPO" merge-base --is-ancestor "$candidate" "$LIVE" 2>/dev/null; then
  echo "nothing: dev serves ${LIVE}, which already contains the newest green commit ${candidate}" >&2
  exit 0
fi
# The candidate must be strictly AHEAD of what dev serves; a diverged pair is a guess.
if ! git -C "$REPO" merge-base --is-ancestor "$LIVE" "$candidate" 2>/dev/null; then
  echo "nothing: ${candidate} does not contain the commit dev serves (${LIVE})" >&2
  exit 0
fi

echo "deploy ${candidate}: newest green commit behind the refused ${REFUSED}, ahead of dev's ${LIVE}" >&2
printf '%s\n' "$candidate"
