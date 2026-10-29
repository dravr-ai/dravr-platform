#!/usr/bin/env bash
# ABOUTME: Lists the jobs of a workflow that already passed on a full-lane branch push of HEAD's exact tree
# ABOUTME: A main push skips those jobs: the same bytes ran the same job green before they reached main
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Usage:
#   proven-on-branch.sh <workflow-file>      e.g. ci-postgres.yml
#
# Why this exists:
#   A branch squashed onto an unchanged main lands the tree its branch run
#   already tested, and main ran every lane again: 14 of 93 main commits on
#   Oct 22-29, about 2,100 runner-minutes a week across Backend (Rust),
#   Backend (PostgreSQL) and Server E2E (carnet#1258). The tree id is the
#   proof: two commits with one tree id hold the same bytes.
#
# What counts as proof, job by job:
#   - a push run of the same workflow file that concluded success, on a
#     branch matching PROVEN_BRANCHES (default feature/, features/, claude/:
#     the refs that run the full PostgreSQL suite and full-workspace clippy;
#     fix/, debug/ and copilot/ run a smoke lane and never count);
#   - whose head commit's tree id is HEAD's;
#   - and only the jobs of that run that concluded success. A job the branch
#     run skipped (its path filter diffed against the branch's previous push,
#     which a squash does not share) proves nothing, so the caller still runs
#     it.
#   The newest three such runs are read; their passing jobs are unioned.
#   The event and conclusion are checked here as well as in the query, so a
#   query parameter the API ignores cannot let a red or pull_request run in.
#
# Output: one job name per line, as the API names it (matrix jobs carry their
# matrix value: "Database Tests (PostgreSQL 1/4)"), and one "<job> -> <step>"
# line per step that concluded success inside such a job. A job whose work is
# gated at step level (clippy reads its filter inside the job, so it concludes
# success having linted nothing on a push no Rust input touches) is proven by
# its work step's line, never by its job line. Nothing printed means nothing
# proven, exit 0.
# Exit 2 on bad usage or a missing GITHUB_REPOSITORY; exit 3 when git or the
# API fails. A caller treats 3 as "nothing proven" and runs everything, under
# a warning: a lookup that silently fails must cost minutes, never coverage.
set -euo pipefail

WORKFLOW="${1:-}"
if [ -z "$WORKFLOW" ] || [ $# -ne 1 ]; then
  echo "usage: $0 <workflow-file>" >&2
  exit 2
fi
if [ -z "${GITHUB_REPOSITORY:-}" ]; then
  echo "proven-on-branch: GITHUB_REPOSITORY is not set" >&2
  exit 2
fi
BRANCHES="${PROVEN_BRANCHES:-^(feature|features|claude)/}"

if ! TREE="$(git rev-parse --verify --quiet 'HEAD^{tree}')"; then
  echo "proven-on-branch: git cannot read HEAD's tree in $(pwd)" >&2
  exit 3
fi

ERR="$(mktemp)"
trap 'rm -f "$ERR"' EXIT
if ! RUNS_JSON="$(gh api "repos/${GITHUB_REPOSITORY}/actions/workflows/${WORKFLOW}/runs?event=push&status=success&per_page=100" 2>"$ERR")"; then
  echo "proven-on-branch: could not list ${WORKFLOW} runs: $(head -c 300 "$ERR")" >&2
  exit 3
fi
# shellcheck disable=SC2016 # $tree and $branches are jq variables
if ! RUN_IDS="$(printf '%s' "$RUNS_JSON" | jq -r --arg tree "$TREE" --arg branches "$BRANCHES" '
                  .workflow_runs[]
                  | select(.event == "push" and .conclusion == "success")
                  | select(.head_branch != "main")
                  | select(.head_commit.tree_id == $tree)
                  | select(.head_branch | test($branches))
                  | .id' 2>"$ERR")"; then
  echo "proven-on-branch: could not read the ${WORKFLOW} run list: $(head -c 300 "$ERR")" >&2
  exit 3
fi

NAMES=""
for id in $(printf '%s\n' "$RUN_IDS" | head -3); do
  # shellcheck disable=SC2016 # $job is a jq variable
  if ! jobs="$(gh api "repos/${GITHUB_REPOSITORY}/actions/runs/${id}/jobs?filter=latest&per_page=100" \
                 --jq '.jobs[] | select(.conclusion == "success") | .name as $job
                       | $job, (.steps[] | select(.conclusion == "success") | "\($job) -> \(.name)")' 2>"$ERR")"; then
    echo "proven-on-branch: could not list the jobs of run ${id}: $(head -c 300 "$ERR")" >&2
    exit 3
  fi
  NAMES="${NAMES}${jobs}"$'\n'
done

printf '%s' "$NAMES" | sed '/^$/d' | sort -u
