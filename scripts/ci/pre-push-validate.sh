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
# TIER 2: Per-Crate Clippy (small crates only — pierre-server is CI's job)
# pierre_mcp_server is large enough that per-crate clippy is as slow as the
# full workspace run. For pierre-server changes, Tier 4 targeted tests are
# the local feedback loop; CI's ci-backend.yml handles workspace clippy.
# ============================================================================
if [[ "$HAS_RUST_CHANGES" == "true" ]] && [[ ${#CHANGED_CRATES[@]} -gt 0 ]]; then
    SMALL_CRATES_CHANGED=()
    for crate_dir in "${!CHANGED_CRATES[@]}"; do
        if [[ "$crate_dir" == "pierre-server" ]]; then
            continue
        fi
        SMALL_CRATES_CHANGED+=("$crate_dir")
    done

    if [[ ${#SMALL_CRATES_CHANGED[@]} -eq 0 ]]; then
        if [[ -n "${CHANGED_CRATES[pierre-server]:-}" ]]; then
            echo "Tier 2: Per-Crate Clippy"
            echo "------------------------"
            echo "Only pierre-server changed — skipping local clippy (CI handles pierre_mcp_server)"
            echo ""
        fi
    else
        echo "Tier 2: Per-Crate Clippy"
        echo "------------------------"
        for crate_dir in "${SMALL_CRATES_CHANGED[@]}"; do
            pkg=$(crate_dir_to_package "$crate_dir")
            echo "  clippy -p $pkg --all-targets --all-features"
            if ! cargo clippy -p "$pkg" --all-targets --all-features -- -D warnings; then
                echo ""
                echo "FAIL: Clippy failed for $pkg"
                echo ""
                echo "Fix warnings then re-run:"
                echo "  cargo clippy -p $pkg --all-targets --all-features -- -D warnings"
                exit 1
            fi
        done
        if [[ -n "${CHANGED_CRATES[pierre-server]:-}" ]]; then
            echo "Note: skipped pierre-server clippy (CI handles workspace clippy for pierre_mcp_server)"
        fi
        echo "OK: Per-crate clippy passed"
        echo ""
    fi
fi

# ============================================================================
# TIER 3: Schema Validation
# ============================================================================
if [[ "$HAS_RUST_SRC_CHANGES" == "true" ]]; then
    echo "Tier 3: Schema Validation"
    echo "-------------------------"
    echo -n "Running schema consistency check... "

    if cargo test --test schema_completeness_test --quiet -- --test-threads=4 > /dev/null 2>&1; then
        echo "OK"
    else
        echo "FAIL"
        echo ""
        echo "Schema validation failed. Run: cargo test --test schema_completeness_test"
        exit 1
    fi
    echo ""
fi

# ============================================================================
# TIER 4: Targeted Tests (smart selection by changed file path)
# ============================================================================
if [[ "$HAS_RUST_SRC_CHANGES" == "true" ]]; then
    echo "Tier 4: Targeted Tests"
    echo "----------------------"

    RUST_CHANGED_FILES=$(echo "$CHANGED_FILES" | grep -E '\.rs$' || echo "")

    if [[ -z "$RUST_CHANGED_FILES" ]]; then
        echo "No Rust source files changed - skipping targeted tests"
    else
        declare -A TESTS_TO_RUN

        add_tests() {
            for test in "$@"; do
                TESTS_TO_RUN["$test"]=1
            done
        }

        while IFS= read -r file; do
            case "$file" in
                crates/pierre-server/src/database/*) add_tests database_test database_plugins_test tenant_data_isolation ;;
                crates/pierre-server/src/auth/*|crates/pierre-server/src/routes/auth.rs) add_tests auth_test api_keys_test jwt_secret_persistence_test oauth2_security_test ;;
                crates/pierre-server/src/routes/*) add_tests routes_health_http_test security_headers_test rate_limiting_middleware_test ;;
                crates/pierre-server/src/protocols/*|crates/pierre-server/src/mcp/*) add_tests mcp_compliance_test jsonrpc_test mcp_tools_unit ;;
                crates/pierre-server/src/tools/*) add_tests mcp_tools_unit ;;
                crates/pierre-server/src/intelligence/*) add_tests intelligence_algorithms_test ;;
                crates/pierre-server/src/a2a/*) add_tests a2a_system_user_test ;;
                crates/pierre-server/src/models/*) add_tests models_test ;;
                crates/pierre-server/src/errors/*) add_tests errors_test ;;
                crates/pierre-server/src/crypto/*) add_tests crypto_keys_test ;;
                crates/pierre-server/src/context/*|crates/pierre-server/src/tenant/*) add_tests tenant_context_resolution_test tenant_data_isolation ;;
                crates/pierre-server/src/config/*) add_tests simple_integration_test ;;
                crates/pierre-database/migrations/*|migrations/*) add_tests database_test ;;
                crates/pierre-server/tests/*.rs)
                    if [[ "$file" =~ ^crates/pierre-server/tests/[^/]+\.rs$ ]]; then
                        test_name=$(basename "$file" .rs)
                        # Skipped: common/helpers/fixtures are shared modules, not test binaries.
                        # messaging_commands_test hangs during pre-push (CI runs it separately).
                        if [[ "$test_name" != "common" && "$test_name" != "helpers" && "$test_name" != "fixtures" && "$test_name" != "messaging_commands_test" ]]; then
                            add_tests "$test_name"
                        fi
                    fi
                    ;;
                crates/pierre-server/src/lib.rs) add_tests simple_integration_test routes_health_http_test ;;
                crates/pierre-server/src/*) add_tests simple_integration_test ;;
            esac
        done <<< "$RUST_CHANGED_FILES"

        TEST_COUNT=${#TESTS_TO_RUN[@]}

        if [[ "$TEST_COUNT" -eq 0 ]]; then
            echo "No tests mapped for changed files"
        else
            echo "Running $TEST_COUNT targeted test file(s):"

            TEST_ARGS=""
            for test in "${!TESTS_TO_RUN[@]}"; do
                echo "  - $test"
                TEST_ARGS="$TEST_ARGS --test $test"
            done
            echo ""

            if ! cargo test $TEST_ARGS --quiet -- --test-threads=4; then
                echo ""
                echo "FAIL: Targeted tests failed!"
                echo ""
                echo "Debug individual tests with:"
                echo "  cargo test --test <test_name> -- --nocapture"
                exit 1
            fi
            echo "OK: Targeted tests passed"
        fi
    fi
    echo ""
fi

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
echo "All validations passed"
echo "==========================================="
echo ""
echo "Duration: ${DURATION}s (~$((DURATION / 60))m $((DURATION % 60))s)"
echo "Marker:   .git/validation-passed (valid for ${VALIDATION_TTL_MINUTES} minutes)"
echo ""
echo "You can now push:"
echo "  git push"
echo ""
