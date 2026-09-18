#!/usr/bin/env bash
# ABOUTME: Pushes the current commit to a branch, rebasing onto whatever landed while the job ran
# ABOUTME: A scheduled lane that races an ordinary push must not red on a non-fast-forward
#
# The bump lanes commit locally and then push to main. `main` moves while they
# run — the lanes are scheduled or dispatched, so racing an ordinary push is the
# normal case, not the exception. A bare push then rejects non-fast-forward and
# reds a lane nobody is watching, over a bump that was itself correct:
# contremaitre-bump did this twice in 45 minutes on 2026-09-07 (carnet#383), and
# the monitor that exists to surface a red scheduled lane surfaced two reds that
# meant nothing.
#
# Rebase, retry, and never force. A commit that became redundant because the
# same bump already landed is dropped by the rebase and the push is a no-op,
# which is the correct outcome. A genuine conflict — someone edited the same
# lines — aborts and fails loudly, because a pin race is never worth resolving
# by overwriting another commit on main.

set -euo pipefail

REMOTE="${1:-origin}"
BRANCH="${2:-main}"
ATTEMPTS="${3:-5}"
# Overridable so the test does not spend real seconds proving the retry works.
BACKOFF="${PUSH_RETRY_BACKOFF:-5}"

case "$ATTEMPTS" in
    ''|*[!0-9]*) echo "attempts must be a non-negative integer, got '$ATTEMPTS'" >&2; exit 2 ;;
esac
[ "$ATTEMPTS" -ge 1 ] || { echo "attempts must be at least 1, got '$ATTEMPTS'" >&2; exit 2; }

attempt=1
while :; do
    if git push "$REMOTE" "HEAD:$BRANCH"; then
        exit 0
    fi
    if [ "$attempt" -ge "$ATTEMPTS" ]; then
        echo "::error::could not push to ${REMOTE}/${BRANCH} after ${ATTEMPTS} attempt(s) — it is moving faster than this lane" >&2
        exit 1
    fi
    echo "push rejected (attempt ${attempt}/${ATTEMPTS}) — ${BRANCH} moved; rebasing onto it"
    if ! git fetch "$REMOTE" "$BRANCH"; then
        echo "::error::could not fetch ${REMOTE}/${BRANCH} to rebase onto" >&2
        exit 1
    fi
    if ! git rebase "${REMOTE}/${BRANCH}"; then
        git rebase --abort 2>/dev/null || true
        echo "::error::the commit conflicts with something that landed on ${BRANCH} — resolve by hand; this lane will not force-push" >&2
        exit 1
    fi
    sleep "$(( attempt * BACKOFF ))"
    attempt=$(( attempt + 1 ))
done
