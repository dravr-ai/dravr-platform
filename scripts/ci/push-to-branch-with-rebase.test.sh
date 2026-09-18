#!/usr/bin/env bash
# ABOUTME: Pins push-to-branch-with-rebase against a real racing remote, not a mock
# ABOUTME: The invariants are: a race is survived, a conflict fails, and nothing is ever force-pushed

set -uo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/push-to-branch-with-rebase.sh"
[ -x "$SCRIPT" ] || { echo "FAIL: $SCRIPT not executable"; exit 1; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/push-rebase-test.XXXXXX")" || exit 1
trap 'rm -rf "$WORK"' EXIT
export PUSH_RETRY_BACKOFF=0
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@e GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@e

pass=0; fail=0
ok()   { echo "  ✅ $1"; pass=$((pass+1)); }
bad()  { echo "  ❌ $1"; fail=$((fail+1)); }
check(){ if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 — expected '$3', got '$2'"; fi; }

# A bare remote, plus two clones that will race each other.
fresh() {
    rm -rf "$WORK/remote" "$WORK/a" "$WORK/b"
    git init -q --bare "$WORK/remote"
    # Without this the bare repo's HEAD points at refs/heads/master, the clones
    # below check out nothing, and every commit lands on an unborn branch.
    git --git-dir="$WORK/remote" symbolic-ref HEAD refs/heads/main
    git init -q "$WORK/seed"
    ( cd "$WORK/seed" || exit
      git checkout -q -b main
      echo base > pin.txt
      git add pin.txt && git commit -qm base
      git push -q "$WORK/remote" main ) >/dev/null 2>&1
    rm -rf "$WORK/seed"
    git clone -q "$WORK/remote" "$WORK/a"
    git clone -q "$WORK/remote" "$WORK/b"
}

echo "== a push with nothing racing it =="
fresh
( cd "$WORK/a" || exit; echo ours > pin.txt; git add pin.txt; git commit -qm ours
  "$SCRIPT" origin main >/dev/null 2>&1 )
check "lands on the remote" "$(git --git-dir="$WORK/remote" log --oneline main | wc -l | tr -d ' ')" "2"

echo "== a push that races another commit =="
fresh
( cd "$WORK/b" || exit; echo theirs > other.txt; git add other.txt; git commit -qm theirs
  git push -q origin main ) >/dev/null 2>&1
( cd "$WORK/a" || exit; echo ours > ours.txt; git add ours.txt; git commit -qm ours
  "$SCRIPT" origin main >/dev/null 2>&1 )
check "the racing push still lands" "$(git --git-dir="$WORK/remote" log --oneline main | wc -l | tr -d ' ')" "3"
# The whole point: the other commit must survive. A force-push would erase it.
check "the commit it raced is still on the branch" \
      "$(git --git-dir="$WORK/remote" log --oneline main | grep -c theirs)" "1"
check "and our own commit is there too" \
      "$(git --git-dir="$WORK/remote" log --oneline main | grep -c ours)" "1"

echo "== a genuine conflict fails loudly and never forces =="
fresh
( cd "$WORK/b" || exit; echo theirs > pin.txt; git add pin.txt; git commit -qm theirs
  git push -q origin main ) >/dev/null 2>&1
( cd "$WORK/a" || exit; echo ours > pin.txt; git add pin.txt; git commit -qm ours
  "$SCRIPT" origin main >/dev/null 2>&1 )
check "exits non-zero on a conflict" "$?" "1"
check "the other commit is untouched on the remote" \
      "$(git --git-dir="$WORK/remote" log --oneline main | grep -c theirs)" "1"
check "ours did NOT overwrite it" \
      "$(git --git-dir="$WORK/remote" log --oneline main | grep -c ours)" "0"
check "no rebase is left in progress" \
      "$( [ -d "$WORK/a/.git/rebase-merge" ] || [ -d "$WORK/a/.git/rebase-apply" ] && echo dirty || echo clean )" "clean"

echo "== a commit made redundant by the same bump landing first =="
fresh
( cd "$WORK/b" || exit; echo bumped > pin.txt; git add pin.txt; git commit -qm "bump to X"
  git push -q origin main ) >/dev/null 2>&1
( cd "$WORK/a" || exit; echo bumped > pin.txt; git add pin.txt; git commit -qm "bump to X"
  "$SCRIPT" origin main >/dev/null 2>&1 )
check "the duplicate bump is dropped, not forced" \
      "$(git --git-dir="$WORK/remote" log --oneline main | wc -l | tr -d ' ')" "2"

echo "== argument validation =="
fresh
( cd "$WORK/a" || exit; "$SCRIPT" origin main notanumber >/dev/null 2>&1 )
check "a non-numeric attempt count is rejected" "$?" "2"
( cd "$WORK/a" || exit; "$SCRIPT" origin main 0 >/dev/null 2>&1 )
check "a zero attempt count is rejected" "$?" "2"

echo
if [ "$fail" -eq 0 ]; then echo "$pass passed · 0 failed"; exit 0; fi
echo "$pass passed · $fail failed"; exit 1
