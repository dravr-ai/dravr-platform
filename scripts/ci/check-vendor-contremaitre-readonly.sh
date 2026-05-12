#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Verifies vendor/contremaitre submodule SHA is on dravr-contremaitre's main
# ABOUTME: Catches local-only commits + dirty working tree before they reach origin

set -e

PROJECT_ROOT="$(git rev-parse --show-toplevel)"
SUBMODULE="vendor/contremaitre"

if [ ! -d "$PROJECT_ROOT/$SUBMODULE/.git" ] && [ ! -f "$PROJECT_ROOT/$SUBMODULE/.git" ]; then
    # Submodule not initialized — skip; CI will catch it via `cargo build`.
    exit 0
fi

# Pinned SHA in the parent repo's tree (what platform's commit records)
PINNED_SHA=$(git ls-tree HEAD "$SUBMODULE" | awk '{print $3}')

if [ -z "$PINNED_SHA" ]; then
    echo "FAIL: cannot read pinned $SUBMODULE SHA from HEAD"
    exit 1
fi

# Fetch contremaitre's main quietly so we have a reference to check against
git -C "$PROJECT_ROOT/$SUBMODULE" fetch origin main --quiet 2>/dev/null || {
    echo "WARN: could not fetch dravr-contremaitre/main (offline?) — skipping ancestry check"
    exit 0
}

# Mode B (catastrophic): pinned SHA isn't on contremaitre's published main.
# Platform commit would reference a private/local-only contremaitre state.
if ! git -C "$PROJECT_ROOT/$SUBMODULE" merge-base --is-ancestor "$PINNED_SHA" origin/main 2>/dev/null; then
    echo ""
    echo "BLOCKED: $SUBMODULE pinned at $PINNED_SHA which is NOT on dravr-contremaitre/main."
    echo ""
    echo "  This usually means a local-only commit was made inside the submodule"
    echo "  and the parent SHA was bumped to point at it. Other contributors will"
    echo "  fail to clone — the SHA doesn't exist on origin."
    echo ""
    echo "  Fix:"
    echo "    1. Push the contremaitre commit to its origin/main, OR"
    echo "    2. Reset the submodule pointer to a public SHA:"
    echo "       cd $SUBMODULE && git checkout origin/main && cd $PROJECT_ROOT"
    echo "       git add $SUBMODULE && git commit --amend --no-edit"
    echo ""
    exit 1
fi

# Mode C (silent loss warning): dirty working tree inside the submodule.
# These edits will be discarded silently by the next 'git submodule update'.
DIRTY=$(git -C "$PROJECT_ROOT/$SUBMODULE" status --porcelain 2>/dev/null || true)
if [ -n "$DIRTY" ]; then
    echo ""
    echo "BLOCKED: $SUBMODULE has uncommitted edits in its working tree."
    echo ""
    echo "  These edits will be silently discarded by the bump workflow OR by"
    echo "  the next 'git submodule update'. They are NOT going anywhere useful."
    echo ""
    echo "  Modified inside $SUBMODULE:"
    echo "$DIRTY" | sed 's/^/    /'
    echo ""
    echo "  To change a prompt for real, edit it in the contremaitre repo:"
    echo "    https://github.com/dravr-ai/dravr-contremaitre/edit/main/<path>"
    echo ""
    echo "  Or via gh CLI:"
    echo "    SHA=\$(gh api repos/dravr-ai/dravr-contremaitre/contents/<path> --jq '.sha')"
    echo "    CONTENT=\$(base64 -i <your-edited-file>)"
    echo "    gh api -X PUT repos/dravr-ai/dravr-contremaitre/contents/<path> \\"
    echo "      -f message='<msg>' -f content=\"\$CONTENT\" -f sha=\"\$SHA\" -f branch=main"
    echo ""
    echo "  Pierre hot-reloads from contremaitre in ~60s; platform submodule SHA"
    echo "  is bumped automatically by the Contremaitre: Bump Submodule workflow."
    echo ""
    echo "  To discard the dirty local edits and move on:"
    echo "    git -C $SUBMODULE restore ."
    echo ""
    exit 1
fi

exit 0
