#!/usr/bin/env bash
# ABOUTME: Prints the commit a CI step should diff HEAD against for the event that started the run,
# ABOUTME: in one of two modes: the commits this push added (before) or everything since main (fork).
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   origin/main cannot be a CI base. actions/checkout fetches only the pushed
#   ref, so on main origin/main IS HEAD (an empty diff, which once kept
#   preflight-clippy from linting any main push) and on a branch it does not
#   exist. The base has to come from the event, and every ci-backend.yml step
#   that diffs against one takes it from this script.
#
# Modes (argument 1):
#   before  The commits this push added: a pull request's base, else the push's
#           before-sha. A branch's first push (all-zero before-sha) and a
#           schedule or dispatch run (none) use the fork point from main.
#   fork    Everything since the branch forked from main: a pull request's
#           base, else the fork point for a branch push, else as `before`.
#           On main the fork point is HEAD itself, so a main push still diffs
#           against its before-sha.
#
# Inputs, from the environment as the workflow step passes them:
#   PR_BASE     github.event.pull_request.base.sha (empty outside a PR)
#   BEFORE      github.event.before (empty outside a push)
#   EVENT_NAME  github.event_name
#   PUSHED_REF  github.ref
#
# Output: the base sha on stdout, exit 0. A base the 50-deep checkout lacks is
# fetched on its own (a diff needs both trees, not the history between them);
# when that fetch fails the sha is still printed, so the caller sees a base it
# cannot resolve and reacts as it does to a missing one.
# Exit 1, printing nothing, when the event names no base and HEAD and main
# share no commit within 200 of either: an honest "no base". What to do then
# is the caller's call: fast-gate fails, Merge-Risk skips, preflight-clippy and
# changed-server-tests use the tip commit.
# Exit 3 when the fork point could not be looked for at all (a fetch failed,
# the repository is broken). That is a crash, not an answer, so a caller that
# skips on exit 1 must not skip on it. Exit 2 is a usage error.
#
# Usage: base="$(./scripts/ci/resolve-push-base.sh before|fork)"

set -euo pipefail

ZERO=0000000000000000000000000000000000000000

mode="${1:-}"
case "$mode" in
  before | fork) ;;
  *)
    echo "usage: $0 before|fork" >&2
    exit 2
    ;;
esac

pr_base="${PR_BASE:-}"
before="${BEFORE:-}"
event="${EVENT_NAME:-}"
ref="${PUSHED_REF:-}"
[ "$before" = "$ZERO" ] && before=""

# HEAD and main, each 200 deep, so a fork point beyond the 50-commit checkout
# is still found. Returns 1 when they share no commit in reach (merge-base's
# own exit 1), 3 on any other failure.
fork_point() {
  local head code=0
  head="$(git rev-parse HEAD)" || return 3
  git fetch -q --no-tags --depth=200 origin "$head" >&2 || return 3
  git fetch -q --no-tags --depth=200 origin main >&2 || return 3
  git merge-base FETCH_HEAD HEAD || code=$?
  case "$code" in
    0 | 1) return "$code" ;;
    *) return 3 ;;
  esac
}

# Sets base to the fork point, or to empty when there is none; exits 3 when the
# lookup itself failed.
use_fork_point() {
  local code=0
  base="$(fork_point)" || code=$?
  case "$code" in
    0) ;;
    1) base="" ;;
    *)
      echo "resolve-push-base: could not look for the fork point from main (fetch or merge-base failed)" >&2
      exit 3
      ;;
  esac
}

base=""
if [ -n "$pr_base" ]; then
  base="$pr_base"
elif [ "$mode" = fork ] && [ "$event" = push ] && [ "$ref" != refs/heads/main ]; then
  use_fork_point
elif [ -n "$before" ]; then
  base="$before"
else
  use_fork_point
fi

if [ -z "$base" ]; then
  echo "resolve-push-base: no base for this ${event:-unknown} run and no fork point from main within 200 commits" >&2
  exit 1
fi

if ! git cat-file -e "${base}^{commit}" 2>/dev/null; then
  git fetch -q --no-tags --depth=1 origin "$base" >&2 \
    || echo "resolve-push-base: could not fetch base $base" >&2
fi
printf '%s\n' "$base"
