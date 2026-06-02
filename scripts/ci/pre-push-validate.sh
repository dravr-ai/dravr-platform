#!/usr/bin/env bash
# ABOUTME: Pre-push validation — fmt + architectural + secret + vendor-readonly checks only.
# ABOUTME: Heavy compilation (clippy, schema test, targeted tests) runs in CI; the Agent MUST monitor CI after every push.
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
# Detect changed files and classify them
# ============================================================================
# Use the merge-base with origin/main so that rebased branches don't report
# main commits they picked up as branch-owned changes. Falls back to
# origin/main or HEAD~1 if merge-base lookup fails (e.g., fresh clone).
if git rev-parse --verify "origin/main" &>/dev/null; then
    BASE_REF=$(git merge-base "origin/main" HEAD 2>/dev/null || echo "origin/main")
else
    BASE_REF="HEAD~1"
fi

CHANGED_FILES=$(git diff --name-only "$BASE_REF" HEAD 2>/dev/null || git diff --name-only HEAD~1 HEAD 2>/dev/null || echo "")

HAS_RUST_SRC_CHANGES=false
HAS_CARGO_CHANGES=false
HAS_FRONTEND_CHANGES=false
HAS_SDK_CHANGES=false
HAS_MOBILE_CHANGES=false

# Track which crates have changes (folder name under crates/)
declare -A CHANGED_CRATES

while IFS= read -r file; do
    [[ -z "$file" ]] && continue
    case "$file" in
        *.rs) HAS_RUST_SRC_CHANGES=true ;;
    esac
    case "$file" in
        Cargo.toml|Cargo.lock) HAS_CARGO_CHANGES=true ;;
        */Cargo.toml) HAS_CARGO_CHANGES=true ;;
    esac
    case "$file" in
        crates/*)
            crate_dir="${file#crates/}"
            crate_dir="${crate_dir%%/*}"
            if [[ -n "$crate_dir" ]] && [[ -d "$PROJECT_ROOT/crates/$crate_dir" ]]; then
                CHANGED_CRATES["$crate_dir"]=1
            fi
            ;;
        frontend/*) HAS_FRONTEND_CHANGES=true ;;
        sdk/*) HAS_SDK_CHANGES=true ;;
        frontend-mobile/*) HAS_MOBILE_CHANGES=true ;;
    esac
done <<< "$CHANGED_FILES"

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
if [[ ${#CHANGED_CRATES[@]} -gt 0 ]]; then
    echo "   Changed crates: ${!CHANGED_CRATES[*]}"
fi
echo ""

# Map crate folder name -> cargo package name.
# Folder == package name except pierre-server which publishes as pierre_mcp_server.
crate_dir_to_package() {
    case "$1" in
        pierre-server) echo "pierre_mcp_server" ;;
        *) echo "$1" ;;
    esac
}

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
# REMOVED: Heavy compilation tiers (per-crate clippy, schema test, targeted
# tests) now run in CI's ci-backend.yml as parallel jobs from the start of
# every push:
#
#   - preflight-clippy   — per-crate clippy on changed leaf crates (3–5 min)
#   - clippy             — full-workspace clippy (10–12 min)
#   - deadlock-analysis  — lockbud static analysis (~10 min)
#   - backend-tests      — SQLite shards (cron / workflow_dispatch only)
#
# CI now also runs PostgreSQL integration tests (ci-postgres.yml) and HTTP/MCP
# integration tests (integration-tests.yml) on every push, so the previous
# "targeted tests" local gate is redundant.
#
# The Agent MUST monitor CI on every push and not consider work complete until
# the relevant workflows are green. See AGENTS.md → "After Pushing".
# ============================================================================

# ============================================================================
# TIER 5: Frontend Validation (if changed)
# ============================================================================
if [[ "$HAS_FRONTEND_CHANGES" == "true" ]]; then
    echo "Tier 5: Frontend Validation"
    echo "---------------------------"
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
# TIER 6: SDK Validation (if changed)
# ============================================================================
if [[ "$HAS_SDK_CHANGES" == "true" ]]; then
    echo "Tier 6: SDK Validation"
    echo "----------------------"
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
# TIER 7: Mobile Validation (if changed)
# ============================================================================
if [[ "$HAS_MOBILE_CHANGES" == "true" ]]; then
    echo "Tier 7: Mobile Validation"
    echo "-------------------------"
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

CURRENT_COMMIT=$(git rev-parse HEAD)
echo "$END_TIME $CURRENT_COMMIT" > "$MARKER_FILE"

echo "==========================================="
echo "Local pre-push validation passed"
echo "==========================================="
echo ""
echo "Duration: ${DURATION}s (~$((DURATION / 60))m $((DURATION % 60))s)"
echo "Marker:   .git/validation-passed (valid for ${VALIDATION_TTL_MINUTES} minutes)"
echo ""
echo "Local validation covers fmt + architecture + secrets + vendor-readonly only."
echo "The heavy gates (clippy, deadlock, integration tests) run in CI on every push."
echo ""
echo "You can now push:"
echo "  git push"
echo ""
echo "AFTER PUSHING — REQUIRED:"
echo "  Monitor CI until green. The Agent does NOT consider work complete until the"
echo "  relevant CI workflows pass. Watch:"
echo "    https://github.com/dravr-ai/dravr-platform/actions?query=branch%3A$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo '<branch>')"
echo ""
