#!/usr/bin/env bash
# ABOUTME: Creates a git worktree with all necessary environment files
# ABOUTME: Copies .envrc and runs direnv allow automatically

set -euo pipefail

usage() {
    echo "Usage: $0 <branch-name> [worktree-path]"
    echo ""
    echo "Creates a git worktree with proper environment setup."
    echo ""
    echo "Arguments:"
    echo "  branch-name    Name of the branch to create"
    echo "  worktree-path  Optional path for worktree (default: ../pierre_mcp_server-<branch>)"
    echo ""
    echo "Example:"
    echo "  $0 feature/new-api"
    echo "  $0 fix/bug-123 /tmp/bug-fix"
    exit 1
}

if [[ $# -lt 1 ]]; then
    usage
fi

BRANCH_NAME="$1"
MAIN_WORKTREE="$(git rev-parse --show-toplevel)"
WORKTREE_PATH="${2:-$(dirname "$MAIN_WORKTREE")/pierre_mcp_server-${BRANCH_NAME//\//-}}"

echo "Creating worktree for branch: $BRANCH_NAME"
echo "Worktree path: $WORKTREE_PATH"

# Create the worktree and branch
git worktree add -b "$BRANCH_NAME" "$WORKTREE_PATH"

# Populate submodules. NOT optional: core.hooksPath is the relative path
# .build/hooks, which git resolves against each worktree's own root. A fresh
# worktree gets an empty .build/, so git finds no hooks directory and silently
# runs NO hooks at all — commit-msg, pre-commit, and pre-push alike. That reads
# exactly like hooks passing, so an unvalidated push looks clean. The nested
# vendor/llm-registre submodule carries limitation-gates.sh, which
# architectural-validation.sh fails without.
echo "Initializing submodules (hooks + validation)..."
git -C "$WORKTREE_PATH" submodule update --init --recursive

if [[ ! -x "$WORKTREE_PATH/.build/hooks/commit-msg" ]]; then
    echo "ERROR: .build/hooks/commit-msg missing after submodule init." >&2
    echo "       This worktree would run with NO git hooks. Fix before committing." >&2
    exit 1
fi

# Copy environment files
echo "Copying environment files..."
cp "$MAIN_WORKTREE/.envrc" "$WORKTREE_PATH/.envrc"

# Copy .mcp.json if it exists
if [[ -f "$MAIN_WORKTREE/.mcp.json" ]]; then
    cp "$MAIN_WORKTREE/.mcp.json" "$WORKTREE_PATH/.mcp.json"
fi

# Point target/ at the repository's shared build directory. Without this a
# worktree rebuilds and then keeps its own near-complete copy of the main
# worktree's build tree — tens of gigabytes each, per worktree, that nothing
# ever reclaims. Sharing also means the first build here starts warm.
SHARE_TOOL="$MAIN_WORKTREE/scripts/setup/cargo-target-share.sh"
if [[ -x "$SHARE_TOOL" ]]; then
    echo "Linking shared Cargo target directory..."
    "$SHARE_TOOL" link "$WORKTREE_PATH"
fi

# Run direnv allow if direnv is available
if command -v direnv &> /dev/null; then
    echo "Running direnv allow..."
    cd "$WORKTREE_PATH" && direnv allow
fi

echo ""
echo "Worktree created successfully!"
echo ""
echo "Next steps:"
echo "  cd $WORKTREE_PATH"
echo ""
