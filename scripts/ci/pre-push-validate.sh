#!/usr/bin/env bash
# ABOUTME: Pre-push validation script - runs all checks before pushing
# ABOUTME: Creates validation-passed marker in git dir (supports worktrees)
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -e

PROJECT_ROOT="$(git rev-parse --show-toplevel)"
GIT_DIR="$(git rev-parse --git-dir)"
MARKER_FILE="$GIT_DIR/validation-passed"
VALIDATION_TTL_MINUTES=15

echo ""
echo "Pre-Push Validation"
echo "==========================================="
echo ""

START_TIME=$(date +%s)

# Remove any stale marker
rm -f "$MARKER_FILE"

# ============================================================================
# Detect changed file types
# ============================================================================
CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)

if git rev-parse --verify "origin/$CURRENT_BRANCH" &>/dev/null; then
    BASE_REF="origin/$CURRENT_BRANCH"
elif git rev-parse --verify "origin/main" &>/dev/null; then
    BASE_REF="origin/main"
else
    BASE_REF="HEAD~1"
fi

CHANGED_FILES=$(git diff --name-only "$BASE_REF" HEAD 2>/dev/null || git diff --name-only HEAD~1 HEAD 2>/dev/null || echo "")

HAS_RUST_SRC_CHANGES=false
HAS_CARGO_CHANGES=false
HAS_FRONTEND_CHANGES=false
HAS_SDK_CHANGES=false
HAS_MOBILE_CHANGES=false

while IFS= read -r file; do
    case "$file" in
        *.rs) HAS_RUST_SRC_CHANGES=true ;;
        Cargo.toml|*/Cargo.toml|Cargo.lock) HAS_CARGO_CHANGES=true ;;
        frontend/*) HAS_FRONTEND_CHANGES=true ;;
        sdk/*) HAS_SDK_CHANGES=true ;;
        frontend-mobile/*) HAS_MOBILE_CHANGES=true ;;
    esac
done <<< "$CHANGED_FILES"

# Any Rust ecosystem change triggers fmt + clippy
HAS_RUST_CHANGES=false
if [[ "$HAS_RUST_SRC_CHANGES" == "true" ]] || [[ "$HAS_CARGO_CHANGES" == "true" ]]; then
    HAS_RUST_CHANGES=true
fi

echo "Changed file types:"
echo "   Rust src: $HAS_RUST_SRC_CHANGES"
echo "   Cargo config: $HAS_CARGO_CHANGES"
echo "   Frontend: $HAS_FRONTEND_CHANGES"
echo "   SDK: $HAS_SDK_CHANGES"
echo "   Mobile: $HAS_MOBILE_CHANGES"
echo ""

# ============================================================================
# TIER 0: Code Formatting
# ============================================================================
if [[ "$HAS_RUST_CHANGES" == "true" ]]; then
    echo "Tier 0: Code Formatting"
    echo "--------------------------"
    echo -n "Checking cargo fmt... "

    if cargo fmt --all -- --check > /dev/null 2>&1; then
        echo "OK"
    else
        echo "FAIL"
        echo ""
        echo "Code is not properly formatted. Run:"
        echo "  cargo fmt --all"
        exit 1
    fi
    echo ""
fi

# ============================================================================
# TIER 1: Architectural Validation
# ============================================================================
if [[ "$HAS_RUST_CHANGES" == "true" ]] && [[ -f "$PROJECT_ROOT/scripts/ci/architectural-validation.sh" ]]; then
    echo "Tier 1: Architectural Validation"
    echo "------------------------------------"
    if ! "$PROJECT_ROOT/scripts/ci/architectural-validation.sh"; then
        echo ""
        echo "FAIL: Architectural validation failed!"
        exit 1
    fi
    echo ""
fi

# ============================================================================
# TIER 2: Clippy (same flags as CI — zero tolerance)
# Tests are handled by CI's 4-shard parallel pipeline.
# ============================================================================
if [[ "$HAS_RUST_CHANGES" == "true" ]]; then
    echo "Tier 2: Clippy (--all-targets --all-features)"
    echo "----------------------------------------------"
    echo "Running cargo clippy (this may take a few minutes)..."

    if cargo clippy --all-targets --all-features -- -D warnings 2>&1; then
        echo "OK: Clippy passed"
    else
        echo ""
        echo "FAIL: Clippy failed! Fix all warnings before pushing."
        exit 1
    fi
    echo ""
fi

# ============================================================================
# TIER 3: Frontend Lint + Type Check (if changed)
# ============================================================================
if [[ "$HAS_FRONTEND_CHANGES" == "true" ]]; then
    echo "Tier 3: Frontend Validation"
    echo "-------------------------"
    if [[ -f "$PROJECT_ROOT/scripts/ci/pre-push-frontend-tests.sh" ]]; then
        if ! "$PROJECT_ROOT/scripts/ci/pre-push-frontend-tests.sh"; then
            echo "FAIL: Frontend validation failed!"
            exit 1
        fi
    else
        echo "WARN: pre-push-frontend-tests.sh not found, skipping"
    fi
    echo ""
fi

# ============================================================================
# TIER 4: SDK Validation (if changed)
# ============================================================================
if [[ "$HAS_SDK_CHANGES" == "true" ]]; then
    echo "Tier 4: SDK Validation"
    echo "--------------------"
    if [[ -d "$PROJECT_ROOT/sdk/node_modules" ]]; then
        echo "Running SDK unit tests..."
        if ! (cd "$PROJECT_ROOT/sdk" && npm run test:unit --silent 2>&1 | tail -5); then
            echo "FAIL: SDK tests failed!"
            exit 1
        fi
        echo "OK: SDK tests passed"
    else
        echo "WARN: sdk/node_modules not found, skipping"
    fi
    echo ""
fi

# ============================================================================
# TIER 5: Mobile Validation (if changed)
# ============================================================================
if [[ "$HAS_MOBILE_CHANGES" == "true" ]]; then
    echo "Tier 5: Mobile Validation"
    echo "-----------------------"
    if [[ -f "$PROJECT_ROOT/scripts/ci/pre-push-mobile-tests.sh" ]]; then
        if ! "$PROJECT_ROOT/scripts/ci/pre-push-mobile-tests.sh"; then
            echo "FAIL: Mobile validation failed!"
            exit 1
        fi
    else
        echo "WARN: pre-push-mobile-tests.sh not found, skipping"
    fi
    echo ""
fi

# ============================================================================
# SUCCESS - Create marker file
# ============================================================================
END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))

# Create marker with timestamp and commit hash
CURRENT_COMMIT=$(git rev-parse HEAD)
echo "$END_TIME $CURRENT_COMMIT" > "$MARKER_FILE"

echo "==========================================="
echo "✅ All validations passed!"
echo "==========================================="
echo ""
echo "Duration: ${DURATION}s (~$((DURATION / 60))m $((DURATION % 60))s)"
echo "Marker:   .git/validation-passed (valid for ${VALIDATION_TTL_MINUTES} minutes)"
echo ""
echo "You can now push:"
echo "  git push"
echo ""
