#!/usr/bin/env bash
# ABOUTME: Fails when a push moves the .build submodule pointer backwards in history
# ABOUTME: The CI half of the guard — the pre-commit hook cannot catch a stale checkout

# The pre-commit hook in .build/hooks refuses a backwards pointer, but git runs
# that hook from the WORKING TREE copy of .build. A checkout left stale at a
# revision from before the hook existed therefore runs the old, guardless hook
# and the rewind sails through — which is exactly how 6105b810b put .build back
# to the revision before the carnet skill existed, taking the skill away from
# eight of nine live sessions until 5468acc4b restored it.
#
# This check runs on GitHub, where no local checkout can be stale, so it closes
# that hole. It compares the pointer at two superproject commits and fails only
# when the newer one is an ANCESTOR of the older: a rewind, never a bump.

set -uo pipefail

OLD_REF=${1:-}
NEW_REF=${2:-}
REPO=${GITHUB_REPOSITORY:-dravr-ai/dravr-platform}
SUB_PATH=.build
SUB_REPO=dravr-ai/dravr-build-config

if [ -z "$OLD_REF" ] || [ -z "$NEW_REF" ]; then
    echo "usage: $(basename "$0") <old-superproject-ref> <new-superproject-ref>" >&2
    exit 64
fi

# A branch's first push reports an all-zero "before"; there is no direction yet.
case "$OLD_REF" in
    0000000000000000000000000000000000000000|"") echo "ℹ️  No previous commit to compare against — skipping."; exit 0 ;;
esac

gitlink_at() { # <ref> -> the .build commit recorded at that superproject ref
    gh api "repos/$REPO/contents/$SUB_PATH?ref=$1" --jq 'select(.type=="submodule") | .sha' 2>/dev/null
}

OLD_SHA=$(gitlink_at "$OLD_REF")
NEW_SHA=$(gitlink_at "$NEW_REF")

if [ -z "$OLD_SHA" ] || [ -z "$NEW_SHA" ]; then
    echo "⚠️  Could not read the $SUB_PATH pointer at both refs — nothing to compare."
    exit 0
fi
if [ "$OLD_SHA" = "$NEW_SHA" ]; then
    echo "✅ $SUB_PATH unchanged (${NEW_SHA:0:8})."
    exit 0
fi

# Ancestry needs the submodule's object graph. Reuse a local checkout when it
# already holds both commits; otherwise clone (the repo is public and small).
WORK=""
if [ -e "$SUB_PATH/.git" ] &&
   git -C "$SUB_PATH" cat-file -e "$OLD_SHA^{commit}" 2>/dev/null &&
   git -C "$SUB_PATH" cat-file -e "$NEW_SHA^{commit}" 2>/dev/null; then
    SUB_DIR=$SUB_PATH
else
    WORK=$(mktemp -d)
    trap 'rm -rf "$WORK"' EXIT
    if ! git clone -q --filter=blob:none "https://github.com/$SUB_REPO" "$WORK/sub" 2>/dev/null; then
        echo "⚠️  Could not clone $SUB_REPO — direction unverified."
        exit 0
    fi
    SUB_DIR=$WORK/sub
fi

for sha in "$OLD_SHA" "$NEW_SHA"; do
    if ! git -C "$SUB_DIR" cat-file -e "$sha^{commit}" 2>/dev/null; then
        echo "⚠️  ${sha:0:8} is not in $SUB_REPO — direction unverified."
        exit 0
    fi
done

if git -C "$SUB_DIR" merge-base --is-ancestor "$NEW_SHA" "$OLD_SHA"; then
    LOST=$(git -C "$SUB_DIR" rev-list --count "$NEW_SHA..$OLD_SHA")
    echo "❌ $SUB_PATH moves BACKWARDS $LOST commit(s):"
    echo "     ${OLD_SHA:0:8}  →  ${NEW_SHA:0:8}"
    echo
    echo "   Dropped from $SUB_REPO:"
    git -C "$SUB_DIR" log --oneline --no-decorate "$NEW_SHA..$OLD_SHA" | sed 's/^/     /'
    echo
    echo "   Everything resolving through $SUB_PATH degrades in silence: core.hooksPath"
    echo "   finds fewer hooks, validate.sh loses its register gates, and .claude/skills"
    echo "   symlinks dangle — a dangling skill symlink is not an error, the skill just"
    echo "   stops existing."
    echo
    echo "   Restore the newer pointer:"
    echo "     git submodule update --init --recursive $SUB_PATH"
    echo "     git -C $SUB_PATH checkout ${OLD_SHA:0:8} && git add $SUB_PATH"
    exit 1
fi

echo "✅ $SUB_PATH moves forward: ${OLD_SHA:0:8} → ${NEW_SHA:0:8}."
