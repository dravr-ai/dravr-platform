#!/bin/bash
# ABOUTME: Pre-push validation for web frontend (frontend/) - TypeScript, lint, tests
# ABOUTME: Runs essential checks to catch issues before pushing (~5-10 seconds)
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
FRONTEND_DIR="$PROJECT_ROOT/frontend"

echo "🌐 Pierre Frontend - Pre-Push Validation"
echo "========================================="
echo ""

# Check if frontend directory exists
if [ ! -d "$FRONTEND_DIR" ]; then
    echo "❌ Error: frontend/ directory not found"
    exit 1
fi

# Check if node_modules exists
if [ ! -d "$FRONTEND_DIR/node_modules" ]; then
    echo "⚠️  Warning: frontend/node_modules not found."
    echo "   Run 'cd frontend && bun install' to enable validation."
    exit 0
fi

cd "$FRONTEND_DIR"

START_TIME=$(date +%s)
PASSED=0
FAILED=0

# ============================================================================
# TIER 0: TypeScript Type Checking (fastest feedback)
# ============================================================================
echo "📘 Tier 0: TypeScript Type Checking"
echo "------------------------------------"
echo -n "Running type-check... "

if bun run type-check > /dev/null 2>&1; then
    echo "✅"
    PASSED=$((PASSED + 1))
else
    echo "❌"
    FAILED=$((FAILED + 1))
    echo ""
    echo "TypeScript errors found:"
    bun run type-check 2>&1 | head -30
    echo ""
    echo "Run 'cd frontend && bun run type-check' to see all errors."
    exit 1
fi

echo ""

# ============================================================================
# TIER 1: ESLint (code quality)
# ============================================================================
echo "🔍 Tier 1: ESLint"
echo "-----------------"
echo -n "Running lint... "

if bun run lint -- --quiet > /dev/null 2>&1; then
    echo "✅"
    PASSED=$((PASSED + 1))
else
    echo "❌"
    FAILED=$((FAILED + 1))
    echo ""
    echo "Lint errors found:"
    bun run lint 2>&1 | head -30
    echo ""
    echo "Run 'cd frontend && bun run lint' to see all errors."
    exit 1
fi

echo ""

# ============================================================================
# TIER 2: Unit Tests (functionality)
# ============================================================================
echo "🧪 Tier 2: Unit Tests"
echo "---------------------"
echo -n "Running tests... "

if bun run test -- --run --reporter=dot > /dev/null 2>&1; then
    echo "✅"
    PASSED=$((PASSED + 1))
else
    echo "❌"
    FAILED=$((FAILED + 1))
    echo ""
    echo "Test failures:"
    bun run test -- --run 2>&1 | tail -30
    echo ""
    echo "Run 'cd frontend && bun run test' to see details."
    exit 1
fi

echo ""

# ============================================================================
# TIER 3: Accessibility E2E (Playwright) — fast (~10s) and catches the a11y
# regressions unit tests can't see (icon labeling, <44px touch targets) that
# otherwise only surface in CI's full E2E suite.
#
# Skipped (not failed) when it can't run a clean check:
#   - port 5173 already in use → could test another worktree's stale server
#   - Playwright browsers not installed → `bunx playwright install chromium`
# CI always runs the full E2E suite as the backstop.
# ============================================================================
echo "♿ Tier 3: Accessibility E2E"
echo "---------------------------"
if lsof -ti:5173 >/dev/null 2>&1; then
    echo "⏭️  Skipped — port 5173 in use (can't guarantee a fresh server). CI runs the full E2E suite."
elif [ ! -d "$HOME/Library/Caches/ms-playwright" ] && [ ! -d "$HOME/.cache/ms-playwright" ]; then
    echo "⏭️  Skipped — Playwright browsers not installed (run 'bunx playwright install chromium'). CI runs the full E2E suite."
else
    echo -n "Running a11y specs... "
    if bunx playwright test e2e/accessibility/ --reporter=dot > /tmp/pierre-a11y-prepush.log 2>&1; then
        echo "✅"
        PASSED=$((PASSED + 1))
    else
        echo "❌"
        FAILED=$((FAILED + 1))
        echo ""
        echo "Accessibility failures:"
        tail -30 /tmp/pierre-a11y-prepush.log
        echo ""
        echo "Run 'cd frontend && bunx playwright test e2e/accessibility/' to debug."
        exit 1
    fi
fi

echo ""

# ============================================================================
# Summary
# ============================================================================
END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))

echo "========================================="
echo "Frontend Pre-Push Validation Complete"
echo "========================================="
echo "Checks passed: $PASSED"
echo "Duration:      ${DURATION}s"
echo ""

if [ $FAILED -gt 0 ]; then
    echo "❌ Some checks failed. Please fix before pushing."
    exit 1
else
    echo "✅ All frontend checks passed!"
    echo ""
    echo "ℹ️  a11y E2E ran here when possible; the FULL E2E suite runs in CI."
    echo "   Run all E2E locally: cd frontend && bun run test:e2e"
fi
