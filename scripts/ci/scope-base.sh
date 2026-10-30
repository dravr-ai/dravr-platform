#!/usr/bin/env bash
# ABOUTME: Names the commit a scoped CI lane diffs against: main's fork point on a branch, the newest green ancestor on main
# ABOUTME: Prints the sha, or nothing when no base is known; affected-crates.py then judges the diff from it to HEAD
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Usage:
#   scope-base.sh <workflow-file> <job name>...
#   env: EVENT_NAME (github.event_name), PUSHED_REF (github.ref),
#        GITHUB_REPOSITORY, GH_TOKEN; MAIN_REF (default origin/main);
#        SCOPE_DISPATCH ('true' when a dispatch asked to be scoped)
#
# A lane that tests only what a push can change is sound when what it diffs
# against already passed (carnet#1282):
#   - a push to a branch diffs against its fork point from main,
#     merge-base(origin/main, HEAD), never against the branch's previous push
#     (github.event.before), which a cancelled or failed run may never have
#     tested. Main's tip there may itself be red; main's own run is the one
#     that answers for main.
#   - a push to main diffs against the newest ancestor of HEAD on main whose
#     run of <workflow-file> passed every named job (success, not skipped:
#     a job skipped because nothing it reads changed proves only that its
#     inputs equal a commit before it, which may have been red). That
#     ancestor's own run was full or scoped from a green ancestor in turn, so
#     the chain is anchored at a full run: the daily schedule, a dispatch, or
#     any push whose diff was judged full. The branch's own head is never
#     the base: a squash lands it after the branch is deleted, and main has
#     usually moved past its fork point by then.
# A schedule run has no base: it tests everything, and so does a dispatch,
# unless it set the workflow's `scope` input (SCOPE_DISPATCH=true). The bump
# lanes do: their GITHUB_TOKEN push starts no run, so their dispatch is the
# commit's only verdict, and it is judged like the push it stands in for.
#
# Output: the base sha on stdout, or nothing (no base: test everything).
# Exit 2 on bad usage; exit 3 when git or the API fails, which a caller
# treats as no base: a lookup that fails must cost minutes, never coverage.
set -euo pipefail

WORKFLOW="${1:-}"
[ $# -ge 2 ] || { echo "usage: $0 <workflow-file> <job name>..." >&2; exit 2; }
shift
JOBS=("$@")
MAIN_REF="${MAIN_REF:-origin/main}"
# How many green-looking main runs to inspect before giving up on a base.
MAX_CANDIDATES="${SCOPE_BASE_MAX_CANDIDATES:-15}"

case "${EVENT_NAME:-}" in
  push) ;;
  workflow_dispatch) [ "${SCOPE_DISPATCH:-}" = true ] || exit 0 ;;
  *) exit 0 ;;
esac

if [ "${PUSHED_REF:-}" != refs/heads/main ]; then
  if ! git rev-parse --verify --quiet "$MAIN_REF^{commit}" >/dev/null; then
    git fetch --quiet --no-tags origin main >/dev/null 2>&1 || true
    MAIN_REF=FETCH_HEAD
  fi
  if ! base="$(git merge-base "$MAIN_REF" HEAD 2>/dev/null)"; then
    echo "scope-base: no merge-base between $MAIN_REF and HEAD (a shallow checkout?)" >&2
    exit 3
  fi
  echo "scope-base: a branch push diffs against its fork point from main, ${base:0:12}" >&2
  echo "$base"
  exit 0
fi

[ -n "${GITHUB_REPOSITORY:-}" ] || { echo "scope-base: GITHUB_REPOSITORY is not set" >&2; exit 2; }
HEAD_SHA="$(git rev-parse HEAD)"
ERR="$(mktemp)"
trap 'rm -f "$ERR"' EXIT

# The event is read from each run, not from a query parameter, so a run of
# another event cannot slip in if the API ignores a filter.
# shellcheck disable=SC2016 # jq program
if ! CANDIDATES="$(gh api "repos/${GITHUB_REPOSITORY}/actions/workflows/${WORKFLOW}/runs?branch=main&status=completed&per_page=50" \
                     --jq '.workflow_runs
                           | map(select(.head_branch == "main" and (.event == "push" or .event == "schedule" or .event == "workflow_dispatch")))
                           | sort_by(.created_at) | reverse | .[] | "\(.id) \(.head_sha)"' 2>"$ERR")"; then
  echo "scope-base: could not list the ${WORKFLOW} runs on main: $(head -c 300 "$ERR")" >&2
  exit 3
fi

checked=0
while read -r id sha; do
  [ -n "$id" ] || continue
  [ "$sha" != "$HEAD_SHA" ] || continue
  # Only an ancestor of HEAD: its tree is what HEAD's changes were made on top of.
  git cat-file -e "${sha}^{commit}" 2>/dev/null || continue
  git merge-base --is-ancestor "$sha" HEAD 2>/dev/null || continue
  checked=$((checked + 1))
  [ "$checked" -le "$MAX_CANDIDATES" ] || break
  # shellcheck disable=SC2016 # jq program
  if ! passed="$(gh api "repos/${GITHUB_REPOSITORY}/actions/runs/${id}/jobs?filter=latest&per_page=100" \
                   --jq '.jobs[] | select(.conclusion == "success") | .name' 2>"$ERR")"; then
    echo "scope-base: could not list the jobs of run ${id}: $(head -c 300 "$ERR")" >&2
    exit 3
  fi
  green=true
  for job in "${JOBS[@]}"; do
    grep -qxF "$job" <<<"$passed" || { green=false; break; }
  done
  if [ "$green" = true ]; then
    echo "scope-base: main diffs against ${sha:0:12}, whose run ${id} passed: ${JOBS[*]}" >&2
    echo "$sha"
    exit 0
  fi
done <<<"$CANDIDATES"

echo "scope-base: no ancestor of HEAD on main passed ${JOBS[*]} in its last runs: testing everything" >&2
exit 0
