#!/usr/bin/env bash
# ABOUTME: Resolves the base commit a diff-scoped gate compares HEAD against, so a
# ABOUTME: checkout whose base ref already equals HEAD still inspects the tip commit.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   A gate that reads `git diff BASE...HEAD` is disarmed whenever BASE resolves to
#   HEAD itself: the diff is empty, the gate prints its green "nothing changed"
#   line, and it has read no file. actions/checkout produces exactly that shape on
#   a push to main — it force-creates refs/heads/main from refs/remotes/origin/main,
#   so origin/main == HEAD and `origin/main...HEAD` compares HEAD's merge-base with
#   itself. The push event carries the pre-push sha in `github.event.before`, which
#   is the base that actually describes the change; a schedule or workflow_dispatch
#   run carries no such sha.
#
#   Sourced rather than executed so each gate keeps one resolution rule. Callers:
#   check-file-sizes.sh, check-migration-idempotency.sh.
#
# Usage:
#   . "$SCRIPT_DIR/gate-base-ref.sh"
#   if ! BASE_REF="$(resolve_gate_base_ref "${1:-}")"; then ... skip ... ; fi

# Print the ref a diff-scoped gate should treat as its base, in priority order:
#
#   1. $1 — an explicit base, which CI passes as the push/PR base sha.
#   2. $GATE_BASE_REF — set once on a workflow step and inherited by every gate
#      the step nests, so a wrapper that calls its children with no argument
#      still gets the push's real base.
#   3. origin/main — the local pre-push case, where it genuinely trails HEAD.
#
# Whichever wins, a base that does not resolve to a commit — or that resolves to
# HEAD, which is the disarmed shape above — falls back to HEAD~1 so the tip
# commit is still inspected. Returns non-zero, printing nothing, only when HEAD
# is a root commit and there is genuinely nothing to diff against.
resolve_gate_base_ref() {
  local candidate="${1:-}"
  [ -n "$candidate" ] || candidate="${GATE_BASE_REF:-}"
  [ -n "$candidate" ] || candidate="origin/main"

  if git rev-parse --verify --quiet "${candidate}^{commit}" >/dev/null 2>&1; then
    local base_sha head_sha
    base_sha="$(git rev-parse "${candidate}^{commit}")"
    head_sha="$(git rev-parse 'HEAD^{commit}')"
    if [ "$base_sha" != "$head_sha" ]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  fi

  if git rev-parse --verify --quiet 'HEAD~1^{commit}' >/dev/null 2>&1; then
    printf '%s\n' 'HEAD~1'
    return 0
  fi

  return 1
}
