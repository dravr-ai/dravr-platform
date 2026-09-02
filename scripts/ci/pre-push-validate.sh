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
HAS_INFRA_CHANGES=false
HAS_INFRA_MODULE_CHANGES=false
HAS_MCP_TYPES_CHANGES=false
HAS_I18N_CATALOGUE_CHANGES=false
HAS_API_CLIENT_CHANGES=false
HAS_SHARED_PACKAGE_CHANGES=false

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
        packages/i18n/src/locales/*) HAS_I18N_CATALOGUE_CHANGES=true; HAS_SHARED_PACKAGE_CHANGES=true ;;
        packages/mcp-types/*) HAS_MCP_TYPES_CHANGES=true; HAS_SHARED_PACKAGE_CHANGES=true ;;
        packages/api-client/*) HAS_API_CLIENT_CHANGES=true; HAS_SHARED_PACKAGE_CHANGES=true ;;
        packages/*) HAS_SHARED_PACKAGE_CHANGES=true ;;
        sdk/*) HAS_SDK_CHANGES=true ;;
        frontend-mobile/*) HAS_MOBILE_CHANGES=true ;;
        infra/modules/*) HAS_INFRA_CHANGES=true; HAS_INFRA_MODULE_CHANGES=true ;;
        infra/*) HAS_INFRA_CHANGES=true ;;
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
echo "   MCP types: $HAS_MCP_TYPES_CHANGES"
echo "   API client: $HAS_API_CLIENT_CHANGES"
echo "   Shared packages: $HAS_SHARED_PACKAGE_CHANGES"
echo "   Mobile: $HAS_MOBILE_CHANGES"
echo "   Infra: $HAS_INFRA_CHANGES"
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
# TIER 0b: Inline Path Lint (compile-free)
# ============================================================================
# clippy::absolute_paths is denied workspace-wide but only reported by the
# full-workspace clippy job ~12 minutes into CI. Grepping the diff finds the
# same thing in under a second. Added after this rule alone caused three
# separate red-then-fix-then-wait cycles on 2026-08-07.
if [[ "$HAS_RUST_CHANGES" == "true" ]] && [[ -x "$PROJECT_ROOT/scripts/ci/check-inline-paths.sh" ]]; then
    echo "Tier 0b: Inline Path Lint"
    echo "--------------------------"
    if ! "$PROJECT_ROOT/scripts/ci/check-inline-paths.sh" "$BASE_REF"; then
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
# TIER 1b: Contremaitre Coupling Sync (compile-free static drift check)
# ============================================================================
# Makes the three platform<->contremaitre drifts (messaging locale completeness,
# notify-event catalogue, MCP tool list) PREVENTIVE at pre-push. The same
# coupling is tested for real by the contremaitre-sync CI job, but that costs a
# Rust compile; this tier is the seconds-long grep that runs before the push.
# Also fires on packages/mcp-types changes, since the tool check reads the
# generated SDK types, and on the string catalogue itself: a JSON-only push
# (a translator fixing one locale) is exactly the push that drops a key from
# four of the five files. See AGENTS.md.
if { [[ "$HAS_RUST_CHANGES" == "true" ]] || [[ "$HAS_MCP_TYPES_CHANGES" == "true" ]] || [[ "$HAS_I18N_CATALOGUE_CHANGES" == "true" ]]; } \
    && [[ -f "$PROJECT_ROOT/scripts/ci/check-contremaitre-sync.sh" ]]; then
    echo "Tier 1b: Contremaitre Coupling Sync"
    echo "------------------------------------"
    if ! "$PROJECT_ROOT/scripts/ci/check-contremaitre-sync.sh"; then
        echo ""
        echo "FAIL: Contremaitre coupling check failed!"
        exit 1
    fi
    echo ""
fi

# ============================================================================
# TIER 1c: Phantom Surface Detection (compile-free)
# ============================================================================
# The dual rule says a capability nothing implements or calls is a phantom
# surface. Nothing breaks when a phantom is wrong — no test covers a branch
# that never executes — so these are invisible to every regression gate and
# surface only in a manual cold read months later (QuotaGate, UsageRecorder,
# authApi.refreshToken). Gates the diff: a NEW unimplemented trait or uncalled
# api-client method fails here, while the author still has the context to wire
# a consumer. The standing stock is reported, not blessed — clearing it is a
# per-surface deletion decision tracked in dravr-ai/carnet (carnet#17).
# Also fires on either client's changes: since the api-client scan is split
# per surface, a client that drops its last caller of a method the other client
# still uses is a parity gap this catches — and that change touches neither
# crates/ nor packages/api-client.
if { [[ "$HAS_RUST_SRC_CHANGES" == "true" ]] || [[ "$HAS_API_CLIENT_CHANGES" == "true" ]] \
    || [[ "$HAS_FRONTEND_CHANGES" == "true" ]] || [[ "$HAS_MOBILE_CHANGES" == "true" ]]; } \
    && [[ -x "$PROJECT_ROOT/scripts/ci/check-phantom-surfaces.sh" ]]; then
    echo "Tier 1c: Phantom Surface Detection"
    echo "------------------------------------"
    if ! "$PROJECT_ROOT/scripts/ci/check-phantom-surfaces.sh" "$BASE_REF"; then
        echo ""
        echo "FAIL: Phantom surface check failed!"
        exit 1
    fi
    echo ""
fi

# ============================================================================
# TIER 1d: Turn Envelope Convergence (compile-free)
# ============================================================================
# The surfaces converged onto one profile, one envelope and one transport. What
# makes that a framework rather than a snapshot is that adding a capability to
# one surface without the others fails at authoring time. Nothing did that
# before — which is why every gap in the original parity survey passed CI green.
# Catches a reintroduced channel-identity flag, a reply block only one client
# renders, a hand-rolled fetch() at /api/chat, and a generated capability
# catalogue that has fallen behind surface_profile.rs.
if { [[ "$HAS_RUST_SRC_CHANGES" == "true" ]] || [[ "$HAS_FRONTEND_CHANGES" == "true" ]] \
    || [[ "$HAS_MOBILE_CHANGES" == "true" ]] || [[ "$HAS_SHARED_PACKAGE_CHANGES" == "true" ]]; } \
    && [[ -x "$PROJECT_ROOT/scripts/ci/check-turn-envelope.sh" ]]; then
    echo "Tier 1d: Turn Envelope Convergence"
    echo "------------------------------------"
    if ! "$PROJECT_ROOT/scripts/ci/check-turn-envelope.sh" "$BASE_REF"; then
        echo ""
        echo "FAIL: Turn envelope convergence check failed!"
        exit 1
    fi
    echo ""
fi

# ============================================================================
# TIER 1e: Changed server-test clippy (compiles ONLY what this push touched)
# ============================================================================
# A new or edited file under crates/pierre-server/tests compiles into no local
# gate: per-crate clippy on the crate you were thinking about never reaches
# the server's test targets, so the lint fails 10+ minutes later in CI's
# full-workspace job (recurrences b2bd18741 on 08-28, the large_futures case
# on 08-30 — carnet#153). This stays inside the heavy-compilation-lives-in-CI
# rule by compiling exactly the changed top-level test targets and nothing
# else: a push touching none pays zero. common.rs / helpers/ changes fan into
# every target and stay CI-covered on purpose — recompiling 300+ binaries
# locally is the cost this script exists to avoid.
if [[ "$HAS_RUST_SRC_CHANGES" == "true" || -n "$(git diff --name-only --diff-filter=AM "$BASE_REF"...HEAD -- 'crates/pierre-server/tests' 2>/dev/null)" ]]; then
    CHANGED_SERVER_TESTS="$(git diff --name-only --diff-filter=AM "$BASE_REF"...HEAD -- 'crates/pierre-server/tests/*.rs' 2>/dev/null \
        | grep -E '^crates/pierre-server/tests/[^/]+\.rs$' \
        | grep -v '/common\.rs$' || true)"
    if [[ -n "$CHANGED_SERVER_TESTS" ]]; then
        echo "Tier 1e: Changed server-test clippy"
        echo "------------------------------------"
        while IFS= read -r f; do
            target="$(basename "$f" .rs)"
            echo "  cargo clippy -p pierre_mcp_server --test $target"
            if ! cargo clippy -p pierre_mcp_server --test "$target" --all-features -- -D warnings; then
                echo ""
                echo "FAIL: clippy on changed test target $target!"
                exit 1
            fi
        done <<< "$CHANGED_SERVER_TESTS"
        echo ""
    fi
fi

# ============================================================================
# TIER 1f: --no-default-features dead_code probe (compiles ONLY changed crates)
# ============================================================================
# CI's feature-profiles job builds `-p pierre_mcp_server --no-default-features`
# under RUSTFLAGS="-D warnings". An item whose SOLE caller sits behind a cargo
# feature is unreachable there, so rustc's dead_code — a warning everywhere
# else — is an error. No local gate sees it: per-crate clippy runs
# --all-features, which turns the caller back on (carnet#153; the fix in
# 6f18100f6 had to widen write_through_served_window to `pub`). `cargo check`
# is enough — dead_code is a rustc lint, the clippy driver adds nothing here —
# but only under RUSTFLAGS="-D warnings", CI's own env: a bare check exits 0 on
# the exact failure being chased, and `cargo check` accepts no trailing lint
# args (that idiom is clippy-only). The deny rides RUSTFLAGS, which changes
# fingerprints, so the probe gets its own CARGO_TARGET_DIR — otherwise every
# run would rebuild the crate's whole dep graph twice (once with the flag,
# once without on the next dev build).
#
# The crates below are pierre-server's NON-OPTIONAL deps whose every feature is
# forwarded from a pierre-server feature that --no-default-features turns off,
# so a per-crate zero-feature build is exactly the unit CI compiles. Deliberate
# exclusions:
#   pierre-core / pierre-database / pierre-providers — pierre-server passes
#     their features (or their defaults) unconditionally, so stripping locally
#     is STRICTER than CI and would red on code CI never builds that way.
#   pierre-chat-pipeline, pierre-commands, pierre-messaging and the other
#     optional deps — absent from this profile entirely; the `production`
#     matrix arm covers them, in CI.
#   pierre-server itself — its unit is the whole graph, which is the cost this
#     script exists to avoid. CI owns it.
NO_DEFAULT_PROBE_CRATES="pierre-tool-runtime pierre-services pierre-routes-admin \
pierre-routes-coaches pierre-runtime-context pierre-auth pierre-formatters"

if [[ "$HAS_RUST_SRC_CHANGES" == "true" ]]; then
    TIER1F_CRATES=""
    for c in $NO_DEFAULT_PROBE_CRATES; do
        [[ -n "$(git diff --name-only "$BASE_REF" HEAD -- "crates/$c/src" 2>/dev/null)" ]] || continue
        TIER1F_CRATES="$TIER1F_CRATES $c"
    done
    if [[ -n "$TIER1F_CRATES" ]]; then
        echo "Tier 1f: --no-default-features probe"
        echo "------------------------------------"
        for c in $TIER1F_CRATES; do
            echo "  RUSTFLAGS=\"-D warnings\" cargo check -p $c --no-default-features"
            if ! CARGO_TARGET_DIR="$PROJECT_ROOT/target/t1f-probe" RUSTFLAGS="-D warnings" \
                cargo check -p "$c" --no-default-features; then
                echo ""
                echo "FAIL: $c has an item unreachable with features off."
                echo "  Its sole caller is behind a #[cfg(feature = ...)]. Either gate"
                echo "  the item with the same cfg, or widen it to \`pub\` if its"
                echo "  siblings are pub for the same reason."
                exit 1
            fi
        done
        echo ""
    fi
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
# TIER 5b: Design System Validation (if web or mobile UI changed)
#
# Compile-free token/primitive conformance. Runs on either platform's changes
# because the ratchets span both — DESIGN.md is one system with two renderers.
# ============================================================================
if [[ "$HAS_FRONTEND_CHANGES" == "true" || "$HAS_MOBILE_CHANGES" == "true" ]]; then
    echo "Tier 5b: Design System Validation"
    echo "---------------------------------"
    if [[ -f "$PROJECT_ROOT/scripts/ci/design-system-validation.sh" ]]; then
        if ! "$PROJECT_ROOT/scripts/ci/design-system-validation.sh"; then
            echo "FAIL: Design system validation failed!"
            exit 1
        fi
    else
        echo "WARN: design-system-validation.sh not found, skipping"
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
        # Build first: several unit tests assert against dist/ (the published
        # bin's shebang, --version, the shipped declarations), so a stale build
        # reports failures the source does not have — and, worse, could pass an
        # artifact that no longer matches src/.
        echo "Building SDK..."
        sdk_build_log="$(mktemp)"
        if ! (cd "$PROJECT_ROOT/sdk" && bun run build) > "$sdk_build_log" 2>&1; then
            tail -20 "$sdk_build_log"
            rm -f "$sdk_build_log"
            echo "FAIL: SDK build failed!"
            exit 1
        fi
        rm -f "$sdk_build_log"

        # The test output is captured rather than piped, because a pipeline
        # reports the LAST command's status — `jest | tail` is always 0, so a
        # piped form reported success over failing tests.
        #
        # jest is invoked through node rather than `bun run test:unit`: `bun run`
        # puts its own `node` shim first on PATH, so everything a script spawns
        # executes on bun's runtime, and jest cannot start there on macOS — all
        # suites die in jest-runtime with "Attempted to assign to readonly
        # property" and report 0 tests. Reproduced on bun 1.3.13 and 1.3.4, so it
        # is the darwin runtime, not version drift. CI keeps using `bun run
        # test:unit` on ubuntu, where it works; this runs the same jest with the
        # same args, only on a runtime that can start it. Keep the args in sync
        # with the `test:unit` script in sdk/package.json.
        # `node` is commonly an nvm lazy-load shell function, which does not
        # exist in a non-interactive script, so resolve a real binary first.
        sdk_node="$(command -v node || true)"
        if [[ -z "$sdk_node" && -s "${NVM_DIR:-$HOME/.nvm}/nvm.sh" ]]; then
            # shellcheck disable=SC1090,SC1091
            . "${NVM_DIR:-$HOME/.nvm}/nvm.sh" >/dev/null 2>&1 || true
            sdk_node="$(command -v node || true)"
        fi

        if [[ -z "$sdk_node" ]]; then
            echo "SKIP: no node binary found to run jest. CI runs the SDK suite."
        else
            echo "Running SDK unit tests..."
            sdk_test_log="$(mktemp)"
            if ! (cd "$PROJECT_ROOT/sdk" && "$sdk_node" node_modules/.bin/jest --testPathPattern=test/unit) > "$sdk_test_log" 2>&1; then
                tail -25 "$sdk_test_log"
                rm -f "$sdk_test_log"
                echo "FAIL: SDK tests failed!"
                exit 1
            fi
            tail -5 "$sdk_test_log"
            rm -f "$sdk_test_log"
            echo "OK: SDK tests passed"
        fi
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
# TIER 8: Infra / Terraform Validation (if changed)
# ============================================================================
# Offline only: fmt + validate + native plan-mode tests (mock providers). No
# GCP credentials and no `terraform plan` against live state — the real diff is
# reviewed at manual apply time (infra/artifacts has no CI apply path).
if [[ "$HAS_INFRA_CHANGES" == "true" ]]; then
    echo "Tier 8: Infra / Terraform Validation"
    echo "------------------------------------"
    if ! command -v terraform > /dev/null 2>&1; then
        echo "WARN: terraform not installed, skipping infra validation"
        echo ""
    else
        INFRA_FAILED=false

        echo -n "Checking terraform fmt... "
        if terraform fmt -check -recursive "$PROJECT_ROOT/infra" > /dev/null 2>&1; then
            echo "OK"
        else
            echo "FAIL (run: terraform fmt -recursive infra)"
            INFRA_FAILED=true
        fi

        # Each root is initialised into a throwaway TF_DATA_DIR rather than its
        # own .terraform/. A developer who has ever run a real `terraform init`
        # has a gcs backend recorded in .terraform/terraform.tfstate, and
        # terraform loads a recorded backend even under `-backend=false` — so
        # without this the offline tier demands live GCP credentials and fails
        # on an expired login. Isolating the data dir also leaves the
        # developer's real terraform state untouched by a push.
        INFRA_TF_DATA_ROOT="$(mktemp -d)"

        # Validate each Terraform root config (a dir with providers.tf) that was
        # touched. A shared-module change also validates the environment roots
        # that consume modules, since the module feeds their plan.
        while IFS= read -r providers_file; do
            [[ -z "$providers_file" ]] && continue
            root="$(dirname "$providers_file")"
            rel="${root#"$PROJECT_ROOT"/}"

            root_changed=false
            while IFS= read -r file; do
                [[ -z "$file" ]] && continue
                case "$file" in
                    "$rel"/*) root_changed=true; break ;;
                esac
            done <<< "$CHANGED_FILES"
            if [[ "$HAS_INFRA_MODULE_CHANGES" == "true" ]] && [[ "$rel" == infra/environments/* ]]; then
                root_changed=true
            fi
            [[ "$root_changed" == "false" ]] && continue

            echo -n "Validating $rel... "
            export TF_DATA_DIR="$INFRA_TF_DATA_ROOT/${rel//\//_}"
            if ! terraform -chdir="$root" init -backend=false -input=false > /dev/null 2>&1; then
                echo "FAIL (terraform init)"
                INFRA_FAILED=true
                continue
            fi
            if ! terraform -chdir="$root" validate > /dev/null 2>&1; then
                echo "FAIL (terraform validate)"
                INFRA_FAILED=true
                continue
            fi
            if [[ -d "$root/tests" ]]; then
                if ! terraform -chdir="$root" test > /dev/null 2>&1; then
                    echo "FAIL (terraform test — run: terraform -chdir=$rel test)"
                    INFRA_FAILED=true
                    continue
                fi
            fi
            echo "OK"
        done < <(find "$PROJECT_ROOT/infra" -name providers.tf -type f 2>/dev/null)
        unset TF_DATA_DIR
        rm -rf "$INFRA_TF_DATA_ROOT"

        if [[ "$INFRA_FAILED" == "true" ]]; then
            echo ""
            echo "FAIL: Infra validation failed!"
            exit 1
        fi
        echo ""
    fi
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
echo "Local validation covers fmt + architecture + secrets + vendor-readonly + infra,"
echo "plus two scoped compile probes: changed server-test clippy (Tier 1e) and the"
echo "--no-default-features check on changed probe crates (Tier 1f)."
echo "The heavy gates (clippy, deadlock, integration tests) run in CI on every push."
echo ""

# ============================================================================
# What this gate CANNOT see — named, because a green run here reads as coverage
# ============================================================================
# Clippy is deliberately not run locally: a full-workspace pass costs more CPU
# than this gate is allowed to spend, and CI runs it on every push. The cost of
# that trade is that a whole lint class — pedantic/nursery lints, doc_markdown
# and too_long_first_doc_paragraph among them — is invisible until CI reports,
# and `cargo check` finds NONE of them. On 2026-08-26 that cost four
# red-then-fix-then-wait cycles on main for four doc comments.
#
# Tier 1f narrows the --no-default-features blind spot but does not close it:
# pierre-server's own cfg sites, the `production` feature matrix arm, and the
# optional-dep crates stay CI-only (feature-profiles job), deliberately. It
# probes the cargo-FEATURE axis only — the target-cfg axis (#[cfg(windows)],
# the bd1afbb85 log_drain class) has no local gate at all: cross-compiling to
# msvc is not feasible here, so the weekly cross-platform cron lane is the
# only check on it.
#
# So this prints the exact scoped commands for what THIS diff touched. Running
# them is a judgement call, not a gate: seconds on a warm target/, minutes on a
# cold one. Nothing below spends CPU — it is git-diff and echo.
if [[ "$HAS_RUST_CHANGES" == "true" ]]; then
    ADVISORY_CRATES=$(echo "$CHANGED_FILES" \
        | grep '^crates/' \
        | sed 's|^crates/||; s|/.*||' \
        | sort -u \
        | grep -v '^pierre-server$' \
        || true)
    ADVISORY_TARGETS=$(echo "$CHANGED_FILES" \
        | grep '^crates/pierre-server/tests/[^/]*\.rs$' \
        | sed 's|.*/||; s|\.rs$||' \
        | sort -u \
        || true)

    if [[ -n "$ADVISORY_CRATES" ]] || [[ -n "$ADVISORY_TARGETS" ]]; then
        echo "NOT COVERED HERE — clippy (deliberate: CPU). CI runs it; these catch it sooner:"
        while IFS= read -r crate; do
            [[ -z "$crate" ]] && continue
            echo "  cargo clippy -p $crate --all-targets --all-features -- -D warnings"
        done <<< "$ADVISORY_CRATES"
        while IFS= read -r target; do
            [[ -z "$target" ]] && continue
            echo "  cargo clippy -p pierre_mcp_server --test $target --all-features -- -D warnings"
        done <<< "$ADVISORY_TARGETS"
        echo ""
        echo "  A red clippy run's error list is PARTIAL: a crate whose dependency"
        echo "  failed to compile emits nothing, which reads the same as clean. Fix"
        echo "  what it reports and expect the next run to find more, not fewer."
        echo ""
    fi
fi
echo "You can now push:"
echo "  git push"
echo ""
echo "AFTER PUSHING — REQUIRED:"
echo "  Monitor CI until green. The Agent does NOT consider work complete until the"
echo "  relevant CI workflows pass. Watch:"
echo "    https://github.com/dravr-ai/dravr-platform/actions?query=branch%3A$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo '<branch>')"
echo ""
