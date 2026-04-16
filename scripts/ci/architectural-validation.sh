#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Custom architectural validation that Cargo/Clippy cannot check
# ABOUTME: Enforces project-specific patterns using validation-patterns.toml

# This script contains ONLY validation logic that has NO native Cargo equivalent:
# 1. TOML-based pattern validation (NULL UUIDs, placeholders, Algorithm DI, etc.)
# 2. Clone/Arc usage analysis and documentation validation
# 3. Binary size enforcement (production quality gate)
# 4. Legacy function detection (UX anti-patterns)
#
# Everything else (formatting, linting, security) is now handled by:
# - cargo fmt --check (formatting)
# - cargo clippy (lints from Cargo.toml [lints] table)
# - cargo deny check (security via deny.toml)

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Parse arguments
APPLY_SKILLS=false
for arg in "$@"; do
    case $arg in
        --apply-skills)
            APPLY_SKILLS=true
            shift
            ;;
    esac
done

# Get the directory where this script is located
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"

echo -e "${BLUE}==== Pierre MCP Server - Architectural Validation ====${NC}"
echo "Project root: $PROJECT_ROOT"
if [ "$APPLY_SKILLS" = true ]; then
    echo -e "${BLUE}Security skills enabled (--apply-skills)${NC}"
fi
cd "$PROJECT_ROOT"

# Track overall success
VALIDATION_FAILED=false

# Function to report validation failure
fail_validation() {
    echo -e "${RED}❌ ARCHITECTURAL VALIDATION FAILED${NC}"
    echo -e "${RED}$1${NC}"
    VALIDATION_FAILED=true
}

# Function to report warning
warn_validation() {
    echo -e "${YELLOW}⚠️  ARCHITECTURAL WARNING${NC}"
    echo -e "${YELLOW}$1${NC}"
}

# Function to report success
pass_validation() {
    echo -e "${GREEN}✅ $1${NC}"
}

# ============================================================================
# FAST-FAIL: Check for backup files BEFORE any other validation
# ============================================================================
# This must run first to avoid corrupting validation metrics output

echo ""
echo -e "${BLUE}==== Checking for backup files (fast-fail) ====${NC}"

BACKUP_FILES=$(find crates/pierre-server/src crates/pierre-server/tests -name "*.backup" -o -name "*.bak" 2>/dev/null)
if [ -n "$BACKUP_FILES" ]; then
    echo -e "${RED}[FAIL] Backup files found (must be removed):${NC}"
    echo "$BACKUP_FILES"
    echo -e "${RED}❌ ARCHITECTURAL VALIDATION FAILED${NC}"
    echo -e "${RED}Remove backup files before running validation${NC}"
    exit 1
fi

pass_validation "No backup files found"

# ============================================================================
# TABLE FORMATTING HELPERS
# ============================================================================

# Truncate text to fit in table column
truncate_text() {
    local text="$1"
    local max_length="$2"
    if [ ${#text} -gt $max_length ]; then
        echo "${text:0:$((max_length-3))}..."
    else
        echo "$text"
    fi
}

# Format status with proper padding for table alignment
format_status() {
    local status="$1"
    # The Status column is 10 characters wide (including padding)
    # We need to account for emoji width differences
    case "$status" in
        "✅ PASS")
            printf "%-9s " "$status"  # Green checkmark is wider, needs less padding
            ;;
        "⚠️ WARN")
            printf "%-8s  " "$status"  # Warning triangle is narrower, needs more padding
            ;;
        "⚠️ INFO")
            printf "%-8s  " "$status"  # Same as WARN
            ;;
        "❌ FAIL")
            printf "%-8s  " "$status"  # X mark is narrower, needs more padding
            ;;
        *)
            printf "%-10s" "$status"   # Default case
            ;;
    esac
}

# Get first location of pattern match
get_first_location() {
    local pattern="$1"
    local result=$(eval "$pattern" 2>/dev/null | head -1 | cut -d: -f1-2)
    if [ -n "$result" ]; then
        truncate_text "$result" 37
    else
        echo "No specific location found"
    fi
}

echo ""
echo -e "${BLUE}============================================================================${NC}"
echo -e "${BLUE}==== UNIFIED ARCHITECTURAL VALIDATION SUITE ====${NC}"
echo -e "${BLUE}============================================================================${NC}"
echo ""

# Load validation patterns from TOML configuration
VALIDATION_PATTERNS_FILE="$SCRIPT_DIR/validation-patterns.toml"
if [ ! -f "$VALIDATION_PATTERNS_FILE" ]; then
    echo -e "${RED}[CRITICAL] Validation patterns file not found: $VALIDATION_PATTERNS_FILE${NC}"
    exit 1
fi

# Parse TOML patterns
eval "$(python3 "$SCRIPT_DIR/parse-validation-patterns.py" "$VALIDATION_PATTERNS_FILE")"

# ============================================================================
# METRIC CALCULATIONS
# ============================================================================

# Anti-Pattern Detection
NULL_UUIDS=$(rg "00000000-0000-0000-0000-000000000000" crates/pierre-server/src/ --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
RESOURCE_CREATION=$(rg "AuthManager::new|OAuthManager::new|A2AClientManager::new|TenantOAuthManager::new" crates/pierre-server/src/ -g "!crates/pierre-server/src/mcp/multitenant.rs" -g "!crates/pierre-server/src/mcp/resources.rs" -g "!crates/pierre-server/src/bin/*" -g "!crates/pierre-server/tests/*" --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
FAKE_RESOURCES=$(rg "Arc::new\(ServerResources\s*[\{\:]" crates/pierre-server/src/ -g "!crates/pierre-server/src/bin/*" 2>/dev/null | wc -l | awk '{print $1+0}')
OBSOLETE_FUNCTIONS=$(rg "fn.*run_http_server\(" crates/pierre-server/src/ 2>/dev/null | wc -l | awk '{print $1+0}')

# Error Handling Pattern Detection
# Note: crates/pierre-server/src/bin/* is now included - all binaries must use structured errors
TOML_ERROR_CONTEXT=$(rg "$ERROR_CONTEXT_ANTIPATTERNS_PATTERNS" crates/pierre-server/src/ -g "!crates/pierre-server/tests/*" --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
ANYHOW_IMPORTS=$(rg "$ANYHOW_IMPORT_ANTIPATTERNS_PATTERNS" crates/pierre-server/src/ -g "!crates/pierre-server/tests/*" --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
ANYHOW_TYPES=$(rg "$ANYHOW_TYPE_ANTIPATTERNS_PATTERNS" crates/pierre-server/src/ -g "!crates/pierre-server/tests/*" --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
ANYHOW_METHODS=$(rg "$ANYHOW_METHOD_ANTIPATTERNS_PATTERNS" crates/pierre-server/src/ -g "!crates/pierre-server/tests/*" --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')

# Code Quality Analysis
PROBLEMATIC_UNWRAPS=$(rg "\.unwrap\(\)" crates/pierre-server/src/ | rg -v "// Safe|hardcoded.*valid|static.*data|00000000-0000-0000-0000-000000000000" | wc -l 2>/dev/null | tr -d ' ' || echo 0)
PROBLEMATIC_EXPECTS=$(rg "\.expect\(" crates/pierre-server/src/ | rg -v "// Safe|ServerResources.*required" | wc -l 2>/dev/null | tr -d ' ' || echo 0)
PANICS=$(rg "panic!\(" crates/pierre-server/src/ --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
TODOS_SRC=$(rg "TODO|FIXME|XXX" crates/pierre-server/src/ -g "!*.json" -g "!*.md" --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
TODOS_TESTS=$(rg "TODO|FIXME|XXX" crates/pierre-server/tests/ -g "!*.json" -g "!*.md" --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
TODOS_SDK=$(rg "TODO|FIXME|XXX" sdk/ -g "!*.json" -g "!*.md" -g "!*.lock" -g "!node_modules/*" --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
TODOS_FRONTEND=$(rg "TODO|FIXME|XXX" frontend/ -g "!*.json" -g "!*.md" -g "!*.lock" -g "!node_modules/*" --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
TODOS=$((TODOS_SRC + TODOS_TESTS + TODOS_SDK + TODOS_FRONTEND))
PRODUCTION_MOCKS=$(rg "mock_|get_mock|return.*mock|demo purposes|for demo|stub implementation|mock implementation" crates/pierre-server/src/ -g "!crates/pierre-server/src/bin/*" -g "!crates/pierre-server/tests/*" | wc -l 2>/dev/null | tr -d ' ' || echo 0)
# Magic input anti-patterns: tools that generate fake data based on special input values
# Excludes: synthetic_provider.rs (legitimate provider), registry.rs (provider registration), spi.rs (provider interface)
MAGIC_INPUT_ANTIPATTERNS=$(rg "SyntheticProvider::new\(\)|SyntheticProvider::from_seed|generate_.*_data\(|create_synthetic_|if.*provider.*==.*\"synthetic\"|match.*provider.*synthetic" crates/pierre-server/src/tools/ crates/pierre-server/src/protocols/universal/handlers/ -g "!*synthetic_provider.rs" -g "!*registry.rs" -g "!*spi.rs" 2>/dev/null | wc -l | tr -d ' ' || echo 0)
PROBLEMATIC_UNDERSCORE_NAMES=$(rg "fn _|let _[a-zA-Z]|struct _|enum _" crates/pierre-server/src/ | rg -v "let _[[:space:]]*=" | rg -v "let _result|let _response|let _output" | wc -l 2>/dev/null | tr -d ' ' || echo 0)
CFG_TEST_IN_SRC=$(rg "#\[cfg\(test\)\]" crates/pierre-server/src/ --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
CLIPPY_ALLOWS_PROBLEMATIC=$(rg "#!?\[allow\(clippy::" crates/pierre-server/src/ -g '!crates/pierre-server/src/routes/openapi.rs' | rg -v "cast_possible_truncation|cast_sign_loss|cast_precision_loss|cast_possible_wrap|struct_excessive_bools|too_many_lines|let_unit_value|option_if_let_else|cognitive_complexity|bool_to_int_with_if|type_complexity|too_many_arguments|use_self" | wc -l 2>/dev/null | tr -d ' ' || echo 0)
DEAD_CODE=$(rg "#\[allow\(dead_code\)\]" crates/pierre-server/src/ --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
TEMP_SOLUTIONS=$(rg "\bhack\b|\bworkaround\b|\bquick.*fix\b|future.*implementation|temporary.*solution|temp.*fix" crates/pierre-server/src/ --count-matches 2>/dev/null | cut -d: -f2 | python3 -c "import sys; lines = sys.stdin.readlines(); print(sum(int(x.strip()) for x in lines) if lines else 0)" 2>/dev/null || echo 0)
# Ignored tests detection - matches both #[ignore] and #[ignore = "reason"]
# Allowlist is defined in validation-patterns.toml [ignored_tests_allowlist]
IGNORED_TESTS_ALLOWLIST_FILES=$(python3 -c "
import tomllib
with open('$SCRIPT_DIR/validation-patterns.toml', 'rb') as f:
    config = tomllib.load(f)
files = config.get('ignored_tests_allowlist', {}).get('files', [])
print(' '.join(['crates/pierre-server/tests/' + f for f in files]))
" 2>/dev/null || echo "")
IGNORED_TESTS=$(rg '#\[ignore' crates/pierre-server/tests/ --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
if [ -n "$IGNORED_TESTS_ALLOWLIST_FILES" ]; then
    IGNORED_TESTS_ALLOWED=$(rg '#\[ignore' $IGNORED_TESTS_ALLOWLIST_FILES --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
else
    IGNORED_TESTS_ALLOWED=0
fi
IGNORED_TESTS_UNAUTHORIZED=$((IGNORED_TESTS - IGNORED_TESTS_ALLOWED))
BACKUP_FILES=$(find crates/pierre-server/src/ -name "*.bak" -o -name "*.backup" -o -name "*~" 2>/dev/null | wc -l | tr -d ' ')
BACKUP_FILES=${BACKUP_FILES:-0}

# Memory Management Analysis
TOTAL_CLONES=$(rg "\.clone\(\)" crates/pierre-server/src/ | grep -v 'crates/pierre-server/src/bin/' | wc -l 2>/dev/null | tr -d ' ' || echo 0)
# Legitimate clone patterns include:
# - Arc/shared resource cloning (Arc::, resources., database., *_manager.)
# - String conversions (.to_string(), format!, String::from)
# - Auth/OAuth fields (token, client_id, client_secret, redirect_uri, access_token, etc.)
# - Request/response/context handling (request., response., context., config., profile.)
# - Common struct field ownership transfers (email, password, hash, key, id, plan, tier, etc.)
# - JSON/serde operations (args, params, value, data)
# - Connection pools and managers (manager, pool, conn, sender, tx, client)
# - Service/plugin/handler cloning (service, plugin, handler, jwks, store, stdout)
# - Domain objects (scope, session, result, validation, overrides, target, goal, sport, provider, host)
# - Location/address fields (town, village, suburb, state, county, road, country, amenity, natural, tourism, leisure, city, location, region, display)
# - Intelligence outputs (pattern, insight, warning, slug, permissions, address, cache, nutrition, algorithm)
# - Protocol/config fields (placeholder, version, logging, mcp, grant, oauth)
# - Documented safe clones (// Safe, // NOTE)
LEGITIMATE_CLONES=$(rg "\.clone\(\)" crates/pierre-server/src/ | grep -v 'crates/pierre-server/src/bin/' | rg "Arc::|resources\.|database\.|auth_manager\.|sse_manager\.|websocket_manager\.|\.to_string\(\)|format!|String::from|token|url|name|path|message|error|Error|client_id|client_secret|redirect_uri|access_token|refresh_token|user_id|tenant_id|request\.|response\.|context\.|config\.|profile\.|email|password|hash|key|plan|tier|args|params|value|data|manager|pool|conn|sender|tx|client|id\.|header|description|weather|// Safe|// NOTE|\.id\b|_id\b|scope|stdout|store|session|jwks|plugin|service|host|target|goal|sport|provider|result|validation|overrides|town|village|suburb|state|county|road|country|amenity|natural|tourism|leisure|pattern|insight|warning|slug|permissions|address|placeholder|version|logging|mcp|city|location|region|display|cache|nutrition|algorithm|grant|oauth" | wc -l 2>/dev/null | tr -d ' ' || echo 0)
PROBLEMATIC_CLONES=$((TOTAL_CLONES - LEGITIMATE_CLONES))
TOTAL_ARCS=$(rg "Arc::" crates/pierre-server/src/ | wc -l 2>/dev/null | tr -d ' ' || echo 0)
# Magic number detection - must use positive glob first to avoid ripgrep negative-glob-only bug
# Excludes: constants directory, config directory, and legitimate patterns in strings/comments
# Note: globs need **/ prefix to match paths within subdirectories of crates/pierre-server/src/
MAGIC_NUMBERS=$(rg "\b[0-9]{4,}\b" crates/pierre-server/src/ -g '*.rs' -g '!**/constants/**' -g '!**/config/**' | grep -v -E "(Licensed|http://|https://|Duration|timestamp|//.*[0-9]|seconds|minutes|hours|Version|\.[0-9]|[0-9]\.|test|mock|example|error.*code|status.*code|port|timeout|limit|capacity|-32[0-9]{3}|1000\.0|60\.0|24\.0|7\.0|365\.0|METERS_PER|PER_METER|conversion|unit|\.60934|12345|0000-0000|202[0-9]-[0-9]{2}-[0-9]{2}|Some\([0-9]+\)|Trial.*1000|Standard.*10000|RFC [0-9]|ISO [0-9]|scientific_basis|backoff|cache_|RSA|key_size|unwrap_or\([0-9]|\.into\(\)|max_entries|max_tokens|DIVISOR|SECONDS)" | wc -l 2>/dev/null | tr -d ' ' || echo 0)

# Repository pattern — detect nominal (defined but unwired) repository traits
# A trait defined in repositories/mod.rs with zero occurrences elsewhere in crates/pierre-server/src/ is dead weight:
# no blanket impl, no direct impl, no call site. This is the "defined but not wired" anti-pattern.
NOMINAL_REPOS_COUNT=0
NOMINAL_REPOS_LIST=""
REPOS_MOD="crates/pierre-server/src/database/repositories/mod.rs"
if [ -f "$REPOS_MOD" ]; then
    while IFS= read -r trait_name; do
        [ -z "$trait_name" ] && continue
        USAGE_COUNT=$(rg "\b${trait_name}\b" crates/pierre-server/src/ \
            --glob '!crates/pierre-server/src/database/repositories/mod.rs' \
            --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
        if [ "$USAGE_COUNT" -eq 0 ]; then
            NOMINAL_REPOS_COUNT=$((NOMINAL_REPOS_COUNT + 1))
            if [ -z "$NOMINAL_REPOS_LIST" ]; then
                NOMINAL_REPOS_LIST="$trait_name"
            else
                NOMINAL_REPOS_LIST="$NOMINAL_REPOS_LIST, $trait_name"
            fi
        fi
    done < <(rg "^pub trait \w+Repository" "$REPOS_MOD" --no-filename -o 2>/dev/null \
        | sed 's/pub trait //' | sed 's/[: ].*//')
fi

# Unsafe and dangerous patterns
UNSAFE_BLOCKS=$(rg "unsafe \{" crates/pierre-server/src/ -g "!crates/pierre-server/src/health.rs" --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')

# ============================================================================
# CRITICAL PATTERN VALIDATION (Fast-Fail)
# ============================================================================

echo -e "${BLUE}Checking for critical anti-patterns...${NC}"

# NULL UUID detection (absolute blocker)
NULL_UUIDS=$(rg "00000000-0000-0000-0000-000000000000" crates/pierre-server/src/ --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
if [ "$NULL_UUIDS" -gt 0 ]; then
    echo -e "${RED}❌ CRITICAL: Found $NULL_UUIDS null UUIDs (test/placeholder code)${NC}"
    rg "00000000-0000-0000-0000-000000000000" crates/pierre-server/src/ -n
    fail_validation "Null UUIDs indicate incomplete implementation"
    exit 1
fi

# Implementation placeholders — scan ALL crates, not just pierre-server
IMPLEMENTATION_PLACEHOLDERS=$(rg -i "$CRITICAL_PATTERNS" crates/*/src/ --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
if [ "$IMPLEMENTATION_PLACEHOLDERS" -gt 0 ]; then
    echo -e "${RED}❌ Found $IMPLEMENTATION_PLACEHOLDERS placeholder implementations${NC}"
    rg -i "$CRITICAL_PATTERNS" crates/*/src/ -n | head -10
    fail_validation "Placeholder implementations must be completed"
fi

# Structural check: PostgreSQL factory stubs that return hardcoded errors instead of real implementations
# Detects any PostgreSQL(_db) => Err(...) arm — regardless of the error message text.
# A real PG implementation calls db.method(), it never returns a static Err().
echo -e "${BLUE}Checking for PostgreSQL factory stubs...${NC}"
PG_FACTORY_STUBS=$(rg -U 'PostgreSQL\(_\w+\)\s*=>\s*Err\(' crates/*/src/ --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
if [ "$PG_FACTORY_STUBS" -gt 0 ]; then
    echo -e "${RED}❌ Found $PG_FACTORY_STUBS PostgreSQL factory stubs (return Err instead of real implementation)${NC}"
    rg -U 'PostgreSQL\(_\w+\)\s*=>\s*Err\(' crates/*/src/ -n | head -10
    fail_validation "PostgreSQL factory arms must have real implementations, not error stubs"
fi

# Placeholder test bodies in JS/TS test files
# Detects tests that only log messages or have comment-only bodies instead of real assertions
echo -e "${BLUE}Checking for placeholder test implementations in JS/TS...${NC}"
JS_PLACEHOLDER_PATTERNS="In a full E2E test, we would|testing infrastructure ready|testing ready|validation infrastructure ready"
JS_PLACEHOLDER_TESTS=$(rg -i "$JS_PLACEHOLDER_PATTERNS" sdk/test/ frontend/src/ frontend-mobile/tests/ --count -g "*.test.*" -g "*.spec.*" 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
if [ "$JS_PLACEHOLDER_TESTS" -gt 0 ]; then
    echo -e "${RED}❌ Found $JS_PLACEHOLDER_TESTS placeholder test implementations in JS/TS${NC}"
    rg -i "$JS_PLACEHOLDER_PATTERNS" sdk/test/ frontend/src/ frontend-mobile/tests/ -n -g "*.test.*" -g "*.spec.*" 2>/dev/null | head -10
    fail_validation "JS/TS test placeholders must be replaced with real assertions"
fi

# Structural dead code: empty Rust source modules (comment-only, no declarations)
# A .rs file in src/ with zero fn/struct/enum/impl/const/static/type/trait/macro/mod/use is dead weight.
# Files containing pub mod/pub use re-exports (mod.rs, lib.rs) are structural and exempt.
echo -e "${BLUE}Checking for empty source modules...${NC}"
EMPTY_MODULES=0
EMPTY_MODULE_LIST=""
EMPTY_MODULE_TMPFILE=$(mktemp)
find crates/*/src -name "*.rs" -not -path "*/tests/*" -not -path "*/bin/*" 2>/dev/null | while IFS= read -r rs_file; do
    DECL_COUNT=$(rg "^(pub )?(pub\(crate\) )?(pub\(super\) )?(async )?(unsafe )?(fn |struct |enum |impl |const |static |type |trait |macro|mod |use )" "$rs_file" --count 2>/dev/null || echo 0)
    if [ "$DECL_COUNT" -eq 0 ]; then
        echo "$rs_file" >> "$EMPTY_MODULE_TMPFILE"
    fi
done
while IFS= read -r rs_file; do
    EMPTY_MODULES=$((EMPTY_MODULES + 1))
    SHORT_PATH="${rs_file#$PROJECT_ROOT/}"
    if [ -z "$EMPTY_MODULE_LIST" ]; then
        EMPTY_MODULE_LIST="$SHORT_PATH"
    else
        EMPTY_MODULE_LIST="$EMPTY_MODULE_LIST, $SHORT_PATH"
    fi
done < "$EMPTY_MODULE_TMPFILE"
rm -f "$EMPTY_MODULE_TMPFILE"
if [ "$EMPTY_MODULES" -gt 0 ]; then
    echo -e "${RED}❌ Found $EMPTY_MODULES empty source module(s) with no declarations:${NC}"
    echo -e "${RED}   $EMPTY_MODULE_LIST${NC}"
    fail_validation "Empty source modules are dead code — remove or implement them"
fi

# Structural dead code: no-op functions (body is only reserve(0), logging, or comments)
echo -e "${BLUE}Checking for no-op function bodies...${NC}"
NOOP_FUNCTIONS=$(rg "\.reserve\(0\)" crates/*/src/ --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
if [ "$NOOP_FUNCTIONS" -gt 0 ]; then
    echo -e "${RED}❌ Found $NOOP_FUNCTIONS no-op reserve(0) call(s) in production code:${NC}"
    rg "\.reserve\(0\)" crates/*/src/ -n | head -5
    fail_validation "reserve(0) is a no-op — remove the function or implement it"
fi

# FORBIDDEN anyhow! macro usage (CLAUDE.md violation)
if [ "$TOML_ERROR_CONTEXT" -gt 0 ]; then
    echo -e "${RED}❌ FORBIDDEN: Found $TOML_ERROR_CONTEXT uses of anyhow! macro${NC}"
    rg "\\banyhow!\\(|anyhow::anyhow!\\(" crates/pierre-server/src/ -g "!crates/pierre-server/tests/*" -n | head -5
    fail_validation "Use AppError/DatabaseError/ProviderError instead of anyhow!"
fi

# STRICT unsafe code usage validation (CLAUDE.md enforcement)
# Only allowed in crates/pierre-server/src/health.rs for Windows FFI (GlobalMemoryStatusEx, GetDiskFreeSpaceExW)
echo -e "${BLUE}Validating unsafe code usage...${NC}"
UNSAFE_USAGE=$(rg "#\[allow\(unsafe_code\)\]|unsafe \{|unsafe fn" crates/pierre-server/src/ -g "!crates/pierre-server/src/bin/*" --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
if [ "$UNSAFE_USAGE" -gt 0 ]; then
    # Check if unsafe usage is ONLY in approved locations
    APPROVED_UNSAFE=$(rg "#\[allow\(unsafe_code\)\]|unsafe \{|unsafe fn" crates/pierre-server/src/health.rs --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
    UNAPPROVED_UNSAFE=$(rg "#\[allow\(unsafe_code\)\]|unsafe \{|unsafe fn" crates/pierre-server/src/ -g "!crates/pierre-server/src/health.rs" -g "!crates/pierre-server/src/bin/*" --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')

    if [ "$UNAPPROVED_UNSAFE" -gt 0 ]; then
        echo -e "${RED}❌ FORBIDDEN: Found $UNAPPROVED_UNSAFE unauthorized unsafe code usages${NC}"
        echo -e "${RED}Unsafe code is ONLY permitted in crates/pierre-server/src/health.rs for Windows FFI${NC}"
        rg "#\[allow\(unsafe_code\)\]|unsafe \{|unsafe fn" crates/pierre-server/src/ -g "!crates/pierre-server/src/health.rs" -g "!crates/pierre-server/src/bin/*" -n | head -10
        fail_validation "Remove unsafe code or get explicit approval before committing"
    else
        pass_validation "Unsafe code usage limited to approved locations (crates/pierre-server/src/health.rs for Windows FFI)"
    fi
else
    pass_validation "No unsafe code found in production code"
fi

# ============================================================================
# CLIPPY ALLOW ATTRIBUTE VALIDATION (CLAUDE.md enforcement)
# ============================================================================

echo -e "${BLUE}Validating clippy allow attribute usage...${NC}"

# Allowed exceptions (from Cargo.toml [lints.clippy] section and justified cases)
# Cast-related exceptions (CLAUDE.md explicitly allows these when validated):
#   - cast_possible_truncation, cast_sign_loss, cast_precision_loss (explicit CLAUDE.md policy)
#   - cast_possible_wrap (similar cast safety validation with "// Safe:" comments)
# Structural exceptions (allowed in Cargo.toml):
#   - struct_excessive_bools (configuration structs with boolean flags)
#   - too_many_lines (long functions with mandatory documentation)
# Legitimate technical exceptions (with justification comments):
#   - let_unit_value (intentional unit value patterns)
#   - option_if_let_else (borrow checker constraints)
#   - cognitive_complexity (complex algorithms requiring detailed logic)
#   - bool_to_int_with_if (multi-level thresholds, not simple conversions)
#   - type_complexity (complex types in generic code)
#   - too_many_arguments (algorithm functions with many validated parameters)
#   - use_self (trait delegation pattern - calling Database::method() instead of Self:: to avoid infinite recursion)
ALLOWED_CLIPPY_ALLOWS="cast_possible_truncation|cast_sign_loss|cast_precision_loss|cast_possible_wrap|struct_excessive_bools|too_many_lines|let_unit_value|option_if_let_else|cognitive_complexity|bool_to_int_with_if|type_complexity|too_many_arguments|use_self"

# Find all #[allow(clippy::...)] usages
CLIPPY_ALLOWS=$(rg "#\[allow\(clippy::" crates/pierre-server/src/ -g "!crates/pierre-server/src/bin/*" --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')

if [ "$CLIPPY_ALLOWS" -gt 0 ]; then
    # Check if any are NOT in the allowed exceptions list
    FORBIDDEN_ALLOWS=$(rg "#\[allow\(clippy::" crates/pierre-server/src/ -g "!crates/pierre-server/src/bin/*" | grep -v -E "$ALLOWED_CLIPPY_ALLOWS" | wc -l | awk '{print $1+0}')

    if [ "$FORBIDDEN_ALLOWS" -gt 0 ]; then
        echo -e "${RED}❌ FORBIDDEN: Found $FORBIDDEN_ALLOWS unauthorized #[allow(clippy::)] attributes${NC}"
        echo -e "${RED}Only allowed for: cast_possible_truncation, cast_sign_loss, cast_precision_loss${NC}"
        rg "#\[allow\(clippy::" crates/pierre-server/src/ -g "!crates/pierre-server/src/bin/*" -n | grep -v -E "$ALLOWED_CLIPPY_ALLOWS" | head -10
        fail_validation "Fix the underlying issue instead of silencing warnings"
    else
        pass_validation "Clippy allow attributes limited to approved cast exceptions"
    fi
else
    pass_validation "No clippy allow attributes found"
fi

# ============================================================================
# UNDERSCORE-PREFIXED NAME VALIDATION (CLAUDE.md enforcement)
# ============================================================================

echo -e "${BLUE}Validating underscore-prefixed names...${NC}"

# Pattern: fn _, let _foo, struct _, enum _
# Note: This allows single underscore (_) for unused variables, but forbids
# names like _foo, _bar, _test, etc.
UNDERSCORE_NAMES=$(rg "fn _[a-zA-Z]|let _[a-zA-Z]|struct _[a-zA-Z]|enum _[a-zA-Z]" crates/pierre-server/src/ -g "!crates/pierre-server/src/bin/*" --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')

if [ "$UNDERSCORE_NAMES" -gt 0 ]; then
    echo -e "${RED}❌ FORBIDDEN: Found $UNDERSCORE_NAMES underscore-prefixed names${NC}"
    echo -e "${RED}Use meaningful names or proper unused variable handling${NC}"
    rg "fn _[a-zA-Z]|let _[a-zA-Z]|struct _[a-zA-Z]|enum _[a-zA-Z]" crates/pierre-server/src/ -g "!crates/pierre-server/src/bin/*" -n | head -10
    fail_validation "Replace underscore-prefixed names with meaningful identifiers"
else
    pass_validation "No underscore-prefixed names found"
fi

# ============================================================================
# ALGORITHM DI ARCHITECTURE ENFORCEMENT
# ============================================================================

echo -e "${BLUE}Validating Algorithm DI patterns...${NC}"

TOTAL_ALGORITHM_VIOLATIONS=0
ALGORITHMS_WITH_VIOLATIONS=""

if [ -n "$MIGRATED_ALGORITHMS" ]; then
    for algo in $MIGRATED_ALGORITHMS; do
        algo_upper=$(echo "$algo" | tr '[:lower:]' '[:upper:]' | tr '-' '_')
        patterns_var="ALGORITHM_${algo_upper}_PATTERNS"
        excludes_var="ALGORITHM_${algo_upper}_EXCLUDES"
        name_var="ALGORITHM_${algo_upper}_NAME"

        eval "patterns=\$$patterns_var"
        eval "excludes=\$$excludes_var"
        eval "algo_name=\$$name_var"

        if [ -n "$patterns" ] && [ -n "$excludes" ]; then
            EXCLUDE_FLAGS=""
            for exclude in $excludes; do
                EXCLUDE_FLAGS="$EXCLUDE_FLAGS -g !$exclude"
            done

            violations=$(rg "$patterns" crates/pierre-server/src/ $EXCLUDE_FLAGS 2>/dev/null | grep -v "^\s*//" | wc -l | awk '{print $1+0}')

            if [ "$violations" -gt 0 ]; then
                TOTAL_ALGORITHM_VIOLATIONS=$((TOTAL_ALGORITHM_VIOLATIONS + violations))
                if [ -z "$ALGORITHMS_WITH_VIOLATIONS" ]; then
                    ALGORITHMS_WITH_VIOLATIONS="$algo_name($violations)"
                else
                    ALGORITHMS_WITH_VIOLATIONS="$ALGORITHMS_WITH_VIOLATIONS, $algo_name($violations)"
                fi
            fi
        fi
    done
fi

if [ "$TOTAL_ALGORITHM_VIOLATIONS" -gt 0 ]; then
    echo -e "${RED}❌ Algorithm DI violations: $ALGORITHMS_WITH_VIOLATIONS${NC}"
    fail_validation "Use enum-based DI in crates/pierre-server/src/intelligence/algorithms/"
else
    pass_validation "Algorithm DI architecture compliance"
fi

# ============================================================================
# REPOSITORY PATTERN ENFORCEMENT (Nominal trait detection)
# ============================================================================

echo -e "${BLUE}Validating repository trait wiring...${NC}"

if [ "$NOMINAL_REPOS_COUNT" -gt 0 ]; then
    echo -e "${RED}❌ REPOSITORY PATTERN VIOLATION: $NOMINAL_REPOS_COUNT nominal (unwired) repository traits${NC}"
    echo -e "${RED}These traits are defined in repositories/mod.rs but have zero implementations${NC}"
    echo -e "${RED}and zero call sites anywhere in crates/pierre-server/src/ — they are purely dead API surface:${NC}"
    echo ""
    IFS=', ' read -ra REPO_NAMES <<< "$NOMINAL_REPOS_LIST"
    for repo in "${REPO_NAMES[@]}"; do
        echo -e "${RED}  - $repo${NC}"
    done
    echo ""
    echo -e "${RED}Each nominal trait MUST have:${NC}"
    echo -e "${RED}  1. An implementation in crates/pierre-server/src/database/repositories/blanket_impls.rs${NC}"
    echo -e "${RED}     (or a direct impl on Database if the method delegation differs)${NC}"
    echo -e "${RED}  2. At least one call site in crates/pierre-server/src/ outside repositories/mod.rs${NC}"
    echo -e "${RED}     (route handlers or services must import and use the trait)${NC}"
    fail_validation "Wire all repository traits before committing"
else
    pass_validation "All repository traits are implemented and used"
fi

# ============================================================================
# BINARY SIZE VALIDATION (Production Quality Gate)
# ============================================================================

echo ""
echo -e "${BLUE}==== Binary Size Validation ====${NC}"

# Binary size is validated at the end of lint-and-test.sh after release build
# Skip this check during early architectural validation
pass_validation "Binary size check deferred to release build step"

# ============================================================================
# RUST IDIOM ENFORCEMENT (Prevent AI-generated regressions)
# ============================================================================

echo ""
echo -e "${BLUE}==== Rust Idiom Enforcement ====${NC}"

# Check for .map().unwrap_or(false) anti-pattern - should use is_some_and()
OPTION_MAP_UNWRAP=$(rg '\.map\([^)]+\)\.unwrap_or\(false\)' crates/pierre-server/src/ --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')

if [ "$OPTION_MAP_UNWRAP" -gt 0 ]; then
    echo -e "${YELLOW}⚠️  Found $OPTION_MAP_UNWRAP uses of .map().unwrap_or(false) - prefer is_some_and()${NC}"
    rg '\.map\([^)]+\)\.unwrap_or\(false\)' crates/pierre-server/src/ -n | head -3
else
    pass_validation "Modern Option patterns used (is_some_and preferred)"
fi

# ============================================================================
# IGNORED DOCTESTS VALIDATION
# ============================================================================

echo ""
echo -e "${BLUE}==== Ignored Doctests Validation ====${NC}"

# Check for ignored doctests - these are not compiled or tested
# Patterns: ```ignore, ```rust,ignore, ```rust, ignore
IGNORED_DOCTESTS=$(rg '///\s*```(rust,\s*)?ignore' crates/pierre-server/src/ --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')

if [ "$IGNORED_DOCTESTS" -gt 0 ]; then
    echo -e "${RED}❌ Found $IGNORED_DOCTESTS ignored doctests${NC}"
    echo -e "${RED}Doctests marked with 'ignore' are not compiled or tested.${NC}"
    echo -e "${RED}Use 'no_run' if code should compile but not execute,${NC}"
    echo -e "${RED}or remove the code fence if it's not meant to be code.${NC}"
    rg '///\s*```(rust,\s*)?ignore' crates/pierre-server/src/ -n | head -5
    fail_validation "Replace 'ignore' with 'no_run' or remove code fence"
else
    pass_validation "No ignored doctests found"
fi

# ============================================================================
# TEST INTEGRITY VALIDATION (No Skipping, No Ignoring)
# ============================================================================

echo ""
echo -e "${BLUE}==== Test Integrity Validation ====${NC}"

# Parse CI continue-on-error allowlist from TOML
CI_COE_ALLOWLIST=$(python3 -c "
import tomllib
with open('$SCRIPT_DIR/validation-patterns.toml', 'rb') as f:
    config = tomllib.load(f)
allowed = config.get('ci_continue_on_error_allowlist', {}).get('allowed', [])
print('\n'.join(allowed))
" 2>/dev/null || echo "")

# Check for continue-on-error: true in CI workflows (excluding commented lines)
# Note: grep -v pattern must match the file:line format from rg, not just the line content
CI_CONTINUE_ON_ERROR_TOTAL=$(rg "continue-on-error:\s*true" .github/workflows/ -n 2>/dev/null | grep -v "#.*continue-on-error" | wc -l | tr -d ' ' || echo 0)

# Filter out allowlisted entries
CI_CONTINUE_ON_ERROR=$CI_CONTINUE_ON_ERROR_TOTAL
if [ -n "$CI_COE_ALLOWLIST" ] && [ "$CI_CONTINUE_ON_ERROR_TOTAL" -gt 0 ]; then
    # Build grep pattern to exclude allowlisted entries
    EXCLUDE_PATTERN=""
    while IFS= read -r entry; do
        [ -z "$entry" ] && continue
        if [ -z "$EXCLUDE_PATTERN" ]; then
            EXCLUDE_PATTERN="$entry"
        else
            EXCLUDE_PATTERN="$EXCLUDE_PATTERN|$entry"
        fi
    done <<< "$CI_COE_ALLOWLIST"

    if [ -n "$EXCLUDE_PATTERN" ]; then
        CI_CONTINUE_ON_ERROR=$(rg "continue-on-error:\s*true" .github/workflows/ -n 2>/dev/null | grep -v "#.*continue-on-error" | grep -v -E "$EXCLUDE_PATTERN" | wc -l | tr -d ' ' || echo 0)
    fi
fi

if [ "$CI_CONTINUE_ON_ERROR" -gt 0 ]; then
    echo -e "${RED}❌ FORBIDDEN: Found $CI_CONTINUE_ON_ERROR 'continue-on-error: true' in CI workflows${NC}"
    echo -e "${RED}All test jobs must fail the build when tests fail.${NC}"
    echo -e "${YELLOW}To allowlist (requires explicit approval), add to validation-patterns.toml [ci_continue_on_error_allowlist]${NC}"
    if [ -n "$EXCLUDE_PATTERN" ]; then
        rg "continue-on-error:\s*true" .github/workflows/ -n | grep -v "#.*continue-on-error" | grep -v -E "$EXCLUDE_PATTERN" | head -5
    else
        rg "continue-on-error:\s*true" .github/workflows/ -n | grep -v "#.*continue-on-error" | head -5
    fi
    fail_validation "Remove continue-on-error: true from test jobs"
else
    pass_validation "No unauthorized continue-on-error: true in CI workflows"
fi

# Parse JS test skip allowlist from TOML
JS_SKIP_ALLOWLIST=$(python3 -c "
import tomllib
with open('$SCRIPT_DIR/validation-patterns.toml', 'rb') as f:
    config = tomllib.load(f)
allowed = config.get('js_test_skip_allowlist', {}).get('allowed', [])
print('\n'.join(allowed))
" 2>/dev/null || echo "")

# Check for .skip(), xit(), xdescribe(), test.skip() in JS/TS test files
JS_TEST_SKIPS_TOTAL=$(rg '\.skip\(\)|\.only\(\)|xit\(|xdescribe\(|test\.skip|it\.skip|describe\.skip' frontend/ frontend-mobile/ sdk/ -g '*.test.ts' -g '*.test.tsx' -g '*.test.js' -g '*.spec.ts' -g '*.spec.js' --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')

# Filter out allowlisted entries
JS_TEST_SKIPS=$JS_TEST_SKIPS_TOTAL
JS_SKIP_EXCLUDE_PATTERN=""
if [ -n "$JS_SKIP_ALLOWLIST" ] && [ "$JS_TEST_SKIPS_TOTAL" -gt 0 ]; then
    # Build grep pattern to exclude allowlisted entries
    while IFS= read -r entry; do
        [ -z "$entry" ] && continue
        if [ -z "$JS_SKIP_EXCLUDE_PATTERN" ]; then
            JS_SKIP_EXCLUDE_PATTERN="$entry"
        else
            JS_SKIP_EXCLUDE_PATTERN="$JS_SKIP_EXCLUDE_PATTERN|$entry"
        fi
    done <<< "$JS_SKIP_ALLOWLIST"

    if [ -n "$JS_SKIP_EXCLUDE_PATTERN" ]; then
        JS_TEST_SKIPS=$(rg '\.skip\(\)|\.only\(\)|xit\(|xdescribe\(|test\.skip|it\.skip|describe\.skip' frontend/ frontend-mobile/ sdk/ -g '*.test.ts' -g '*.test.tsx' -g '*.test.js' -g '*.spec.ts' -g '*.spec.js' -n 2>/dev/null | grep -v -E "$JS_SKIP_EXCLUDE_PATTERN" | wc -l | tr -d ' ' || echo 0)
    fi
fi

if [ "$JS_TEST_SKIPS" -gt 0 ]; then
    echo -e "${RED}❌ FORBIDDEN: Found $JS_TEST_SKIPS skipped/focused tests in JS/TS${NC}"
    echo -e "${RED}Remove .skip(), .only(), xit(), xdescribe() from test files.${NC}"
    echo -e "${YELLOW}To allowlist (requires explicit approval), add to validation-patterns.toml [js_test_skip_allowlist]${NC}"
    if [ -n "$JS_SKIP_EXCLUDE_PATTERN" ]; then
        rg '\.skip\(\)|\.only\(\)|xit\(|xdescribe\(|test\.skip|it\.skip|describe\.skip' frontend/ frontend-mobile/ sdk/ -g '*.test.ts' -g '*.test.tsx' -g '*.test.js' -g '*.spec.ts' -g '*.spec.js' -n | grep -v -E "$JS_SKIP_EXCLUDE_PATTERN" | head -5
    else
        rg '\.skip\(\)|\.only\(\)|xit\(|xdescribe\(|test\.skip|it\.skip|describe\.skip' frontend/ frontend-mobile/ sdk/ -g '*.test.ts' -g '*.test.tsx' -g '*.test.js' -g '*.spec.ts' -g '*.spec.js' -n | head -5
    fi
    fail_validation "Remove skipped/focused tests"
else
    pass_validation "No skipped tests in JS/TS"
fi

# ============================================================================
# LEGACY FUNCTION DETECTION (UX Anti-Patterns)
# ============================================================================

echo ""
echo -e "${BLUE}==== Legacy Function Detection ====${NC}"

LEGACY_OAUTH=$(rg "Legacy OAuth not supported|legacy.*oauth|connect_strava|connect_fitbit" crates/pierre-server/src/ --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
DEPRECATED_FUNCTIONS=$(rg "deprecated.*use.*instead|Universal.*deprecated|ProviderManager deprecated" crates/pierre-server/src/ --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
# Exclude A2A service - those are legitimate A2A protocol handlers returning JSON Values
PLACEHOLDER_IMPLEMENTATIONS=$(rg "fn handle_.*-> Value" crates/pierre-server/src/ --glob '!crates/pierre-server/src/routes/a2a/service.rs' --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
DISCARDED_EXPENSIVE_OPS=$(rg -B 2 -A 5 'let _ = \(' crates/pierre-server/src/ | grep -v 'crates/pierre-server/src/bin/' | rg '\.clone\(\)' | wc -l 2>/dev/null | tr -d ' ' || echo 0)

LEGACY_ISSUES=0
LEGACY_ISSUES=$((LEGACY_ISSUES + LEGACY_OAUTH + DEPRECATED_FUNCTIONS + PLACEHOLDER_IMPLEMENTATIONS + DISCARDED_EXPENSIVE_OPS))

if [ "$LEGACY_ISSUES" -gt 0 ]; then
    echo -e "${RED}❌ Found $LEGACY_ISSUES legacy/stub functions that confuse users${NC}"
    [ "$LEGACY_OAUTH" -gt 0 ] && echo "  - Legacy OAuth patterns: $LEGACY_OAUTH"
    [ "$DEPRECATED_FUNCTIONS" -gt 0 ] && echo "  - Deprecated functions: $DEPRECATED_FUNCTIONS"
    [ "$PLACEHOLDER_IMPLEMENTATIONS" -gt 0 ] && echo "  - Placeholder handlers: $PLACEHOLDER_IMPLEMENTATIONS"
    [ "$DISCARDED_EXPENSIVE_OPS" -gt 0 ] && echo "  - Discarded expensive ops: $DISCARDED_EXPENSIVE_OPS"
    fail_validation "Remove legacy functions that advertise but don't work"
else
    pass_validation "No legacy UX anti-patterns detected"
fi

# ============================================================================
# PRE-RELEASE LEGACY CODE DETECTION (No backward compatibility before v1.0)
# ============================================================================

echo ""
echo -e "${BLUE}==== Pre-Release Legacy Code Detection ====${NC}"

# Before first production release, there should be NO legacy code or backward compatibility
# Excludes:
# - external/usda_client.rs: "SR Legacy" is an actual USDA database name
# - crates/pierre-server/tests/: test files may reference legacy patterns for testing
# - *.md files: documentation
LEGACY_CODE_REFS=$(rg -i "legacy|backward.?compat" crates/pierre-server/src/ -g "!crates/pierre-server/src/external/usda_client.rs" --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')

if [ "$LEGACY_CODE_REFS" -gt 0 ]; then
    echo -e "${RED}❌ FORBIDDEN: Found $LEGACY_CODE_REFS 'legacy' or 'backward compatibility' references${NC}"
    echo -e "${RED}No legacy code allowed before first production release!${NC}"
    echo ""
    echo "Violations:"
    rg -i "legacy|backward.?compat" crates/pierre-server/src/ -g "!crates/pierre-server/src/external/usda_client.rs" -n | head -20
    fail_validation "Remove all legacy code and backward compatibility - not released yet!"
else
    pass_validation "No pre-release legacy code found"
fi

# ============================================================================
# LOG REDACTION ENFORCEMENT (CLAUDE.md "Logging Hygiene")
# ============================================================================
# Catches the exact regressions that triggered the 2026-04-16 Cloud Run DB-URL
# leak. See "Canonical Redaction Helpers" and "Forbidden Logging Patterns" in
# .claude/CLAUDE.md for the policy this enforces.

echo ""
echo -e "${BLUE}==== Log Redaction Enforcement ====${NC}"

# Check 1: Database URL logged without redact_url on the same line.
# Any info!/warn!/error!/println!/eprintln!/debug! line that references a
# database URL variable or "Database URL" label MUST also call redact_url.
DB_URL_LEAKS=$(rg '^\s*(info|warn|error|debug|trace|println|eprintln)!.*(Database URL|database_url|connection_string)' crates/*/src/ -g '!**/redaction.rs' 2>/dev/null | rg -v 'redact_url' | wc -l | tr -d ' ')
if [ "$DB_URL_LEAKS" -gt 0 ]; then
    echo -e "${RED}❌ Found $DB_URL_LEAKS log line(s) referencing a database URL without redact_url():${NC}"
    rg '^\s*(info|warn|error|debug|trace|println|eprintln)!.*(Database URL|database_url|connection_string)' crates/*/src/ -g '!**/redaction.rs' -n | rg -v 'redact_url' | head -10
    fail_validation "Wrap database URLs with pierre_core::redaction::redact_url before logging"
else
    pass_validation "No unredacted database URL log lines"
fi

# Check 2: Direct interpolation of variables named after secrets.
# Catches info!("...{password}..."), error!("...{client_secret}..."), etc.
# Allowlisted: the secret_fingerprint diagnostic pattern (logs length + hash, not the value).
SECRET_VAR_LEAKS=$(rg '^\s*(info|warn|error|debug|trace|println|eprintln)!.*\{(password|client_secret|jwt_secret|encryption_key|access_token|refresh_token)(:|\})' crates/*/src/ 2>/dev/null | rg -v 'secret_fingerprint|secret_length' | wc -l | tr -d ' ')
if [ "$SECRET_VAR_LEAKS" -gt 0 ]; then
    echo -e "${RED}❌ Found $SECRET_VAR_LEAKS log line(s) interpolating a secret-named variable directly:${NC}"
    rg '^\s*(info|warn|error|debug|trace|println|eprintln)!.*\{(password|client_secret|jwt_secret|encryption_key|access_token|refresh_token)(:|\})' crates/*/src/ -n | rg -v 'secret_fingerprint|secret_length' | head -10
    fail_validation "Log a fingerprint (SHA256 prefix) or length instead of the raw secret"
fi
if [ "$SECRET_VAR_LEAKS" -eq 0 ]; then
    pass_validation "No direct secret-variable interpolation in log macros"
fi

# Check 3: Debug formatting of sensitive config structs via inline capture.
# {config:?}, {config:#?}, {oauth_config:?}, etc. would dump every nested secret.
# Positional-arg forms ("{:?}", config) are not caught here — the type-level
# follow-up (secrecy crate) is the complete fix for that class of leak.
CONFIG_DEBUG_LEAKS=$(rg '^\s*(info|warn|error|debug|trace|println|eprintln)!.*\{(server_config|database_config|oauth_config|firebase_config|weather_config|oauth2_server_config|config):#?\?\}' crates/*/src/ 2>/dev/null | wc -l | tr -d ' ')
if [ "$CONFIG_DEBUG_LEAKS" -gt 0 ]; then
    echo -e "${RED}❌ Found $CONFIG_DEBUG_LEAKS log line(s) debug-formatting a config struct that contains secrets:${NC}"
    rg '^\s*(info|warn|error|debug|trace|println|eprintln)!.*\{(server_config|database_config|oauth_config|firebase_config|weather_config|oauth2_server_config|config):#?\?\}' crates/*/src/ -n | head -10
    fail_validation "ServerConfig/DatabaseConfig/OAuthProviderConfig derive Debug but contain secrets — log only the specific fields you need"
else
    pass_validation "No config-struct debug formatting in log macros"
fi

# ============================================================================
# VALIDATION RESULTS TABLE
# ============================================================================

echo ""
echo -e "${BLUE}==== Validation Results Table ====${NC}"
echo ""

echo "┌─────────────────────────────────────┬───────┬──────────┬─────────────────────────────────────────┐"
echo "│ Validation Category                 │ Count │ Status   │ Details / First Location                │"
echo "├─────────────────────────────────────┼───────┼──────────┼─────────────────────────────────────────┤"

# Anti-Pattern Detection
printf "│ %-35s │ %5d │ " "NULL UUIDs" "$NULL_UUIDS"
if [ "$NULL_UUIDS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "No test/placeholder UUIDs"
else
    FIRST_NULL=$(get_first_location 'rg "00000000-0000-0000-0000-000000000000" crates/pierre-server/src/ -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_NULL"
fi

printf "│ %-35s │ %5d │ " "Placeholder implementations" "$IMPLEMENTATION_PLACEHOLDERS"
if [ "$IMPLEMENTATION_PLACEHOLDERS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "No placeholder implementations"
else
    FIRST_PLACEHOLDER=$(get_first_location 'rg -i "$CRITICAL_PATTERNS" crates/pierre-server/src/ -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_PLACEHOLDER"
fi

printf "│ %-35s │ %5d │ " "PostgreSQL factory stubs" "$PG_FACTORY_STUBS"
if [ "$PG_FACTORY_STUBS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "All PG arms have real implementations"
else
    FIRST_PG_STUB=$(get_first_location 'rg -U "PostgreSQL\(_\w+\)\s*=>\s*Err\(" crates/*/src/ -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_PG_STUB"
fi

printf "│ %-35s │ %5d │ " "JS/TS test placeholders" "$JS_PLACEHOLDER_TESTS"
if [ "$JS_PLACEHOLDER_TESTS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "No JS/TS placeholder tests"
else
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "Replace placeholder tests"
fi

printf "│ %-35s │ %5d │ " "Empty source modules" "$EMPTY_MODULES"
if [ "$EMPTY_MODULES" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "No dead code modules"
else
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$EMPTY_MODULE_LIST"
fi

printf "│ %-35s │ %5d │ " "No-op reserve(0) calls" "$NOOP_FUNCTIONS"
if [ "$NOOP_FUNCTIONS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "No no-op function bodies"
else
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "Dead code in production"
fi

printf "│ %-35s │ %5d │ " "Resource creation patterns" "$RESOURCE_CREATION"
if [ "$RESOURCE_CREATION" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Using dependency injection"
else
    FIRST_RESOURCE=$(get_first_location 'rg "AuthManager::new|OAuthManager::new" crates/pierre-server/src/ -g "!crates/pierre-server/src/mcp/multitenant.rs" -g "!crates/pierre-server/src/bin/*" -n')
    printf "$(format_status "⚠️ WARN")│ %-39s │\n" "$FIRST_RESOURCE"
fi

printf "│ %-35s │ %5d │ " "Fake resource assemblies" "$FAKE_RESOURCES"
if [ "$FAKE_RESOURCES" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "No fake ServerResources"
else
    FIRST_FAKE=$(get_first_location 'rg "Arc::new\(ServerResources" crates/pierre-server/src/ -g "!crates/pierre-server/src/bin/*" -n')
    printf "$(format_status "⚠️ WARN")│ %-39s │\n" "$FIRST_FAKE"
fi

printf "│ %-35s │ %5d │ " "Unsafe code blocks" "$UNSAFE_BLOCKS"
if [ "$UNSAFE_BLOCKS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Limited to approved locations"
else
    FIRST_UNSAFE=$(get_first_location 'rg "unsafe \{" crates/pierre-server/src/ -g "!crates/pierre-server/src/health.rs" -n')
    printf "$(format_status "⚠️ WARN")│ %-39s │\n" "$FIRST_UNSAFE"
fi

printf "│ %-35s │ %5d │ " "Forbidden anyhow! macro usage" "$TOML_ERROR_CONTEXT"
if [ "$TOML_ERROR_CONTEXT" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Using structured error types"
else
    FIRST_ANYHOW=$(get_first_location 'rg "\\banyhow!\\(|anyhow::anyhow!\\(" crates/pierre-server/src/ -g "!crates/pierre-server/tests/*" -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_ANYHOW"
    VALIDATION_FAILED=true
fi

printf "│ %-35s │ %5d │ " "Forbidden anyhow imports" "$ANYHOW_IMPORTS"
if [ "$ANYHOW_IMPORTS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Using AppResult imports"
else
    FIRST_IMPORT=$(get_first_location 'rg "$ANYHOW_IMPORT_ANTIPATTERNS_PATTERNS" crates/pierre-server/src/ -g "!crates/pierre-server/tests/*" -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_IMPORT"
    VALIDATION_FAILED=true
fi

printf "│ %-35s │ %5d │ " "Forbidden anyhow::Result types" "$ANYHOW_TYPES"
if [ "$ANYHOW_TYPES" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Using AppResult types"
else
    FIRST_TYPE=$(get_first_location 'rg "$ANYHOW_TYPE_ANTIPATTERNS_PATTERNS" crates/pierre-server/src/ -g "!crates/pierre-server/tests/*" -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_TYPE"
    VALIDATION_FAILED=true
fi

printf "│ %-35s │ %5d │ " "Anyhow .context() method usage" "$ANYHOW_METHODS"
if [ "$ANYHOW_METHODS" -le "$MAX_ANYHOW_METHOD_ANTIPATTERNS" ]; then
    if [ "$ANYHOW_METHODS" -eq 0 ]; then
        printf "$(format_status "✅ PASS")│ %-39s │\n" "Using .map_err() pattern"
    else
        printf "$(format_status "⚠️ INFO")│ %-39s │\n" "Migration in progress (threshold: $MAX_ANYHOW_METHOD_ANTIPATTERNS)"
    fi
else
    FIRST_CONTEXT=$(get_first_location 'rg "$ANYHOW_METHOD_ANTIPATTERNS_PATTERNS" crates/pierre-server/src/ -g "!crates/pierre-server/tests/*" -n')
    printf "$(format_status "⚠️ WARN")│ %-39s │\n" "$FIRST_CONTEXT"
fi

printf "│ %-35s │ %5d │ " "Algorithm DI violations" "$TOTAL_ALGORITHM_VIOLATIONS"
if [ "$TOTAL_ALGORITHM_VIOLATIONS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Using enum-based DI pattern"
else
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$(truncate_text "$ALGORITHMS_WITH_VIOLATIONS" 37)"
fi

printf "│ %-35s │ %5d │ " "Nominal repository traits" "$NOMINAL_REPOS_COUNT"
if [ "$NOMINAL_REPOS_COUNT" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "All repo traits wired"
else
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$(truncate_text "$NOMINAL_REPOS_LIST" 37)"
fi

echo "├─────────────────────────────────────┼───────┼──────────┼─────────────────────────────────────────┤"

# Code Quality Analysis
printf "│ %-35s │ %5d │ " "Problematic unwraps" "$PROBLEMATIC_UNWRAPS"
if [ "$PROBLEMATIC_UNWRAPS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Proper error handling"
else
    FIRST_UNWRAP=$(get_first_location 'rg "\.unwrap\(\)" crates/pierre-server/src/ | rg -v "// Safe" -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_UNWRAP"
    VALIDATION_FAILED=true
fi

printf "│ %-35s │ %5d │ " "Problematic expects" "$PROBLEMATIC_EXPECTS"
if [ "$PROBLEMATIC_EXPECTS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Proper error handling"
else
    FIRST_EXPECT=$(get_first_location 'rg "\.expect\(" crates/pierre-server/src/ | rg -v "// Safe" -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_EXPECT"
    VALIDATION_FAILED=true
fi

printf "│ %-35s │ %5d │ " "Panic calls" "$PANICS"
if [ "$PANICS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "No panic! found"
else
    FIRST_PANIC=$(get_first_location 'rg "panic!\(" crates/pierre-server/src/ -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_PANIC"
    VALIDATION_FAILED=true
fi

printf "│ %-35s │ %5d │ " "TODOs/FIXMEs (all dirs)" "$TODOS"
if [ "$TODOS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "No incomplete code"
else
    TODO_BREAKDOWN="src:$TODOS_SRC tests:$TODOS_TESTS sdk:$TODOS_SDK fe:$TODOS_FRONTEND"
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$TODO_BREAKDOWN"
    VALIDATION_FAILED=true
fi

printf "│ %-35s │ %5d │ " "Production mock implementations" "$PRODUCTION_MOCKS"
if [ "$PRODUCTION_MOCKS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "No mock code in production"
else
    FIRST_MOCK=$(get_first_location 'rg "mock_|get_mock|stub implementation" crates/pierre-server/src/ -g "!crates/pierre-server/src/bin/*" -g "!crates/pierre-server/tests/*" -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_MOCK"
fi

printf "│ %-35s │ %5d │ " "Magic input anti-patterns" "$MAGIC_INPUT_ANTIPATTERNS"
if [ "$MAGIC_INPUT_ANTIPATTERNS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Tools require explicit input"
else
    FIRST_MAGIC=$(get_first_location 'rg "SyntheticProvider::new|SyntheticProvider::from_seed|generate_.*_data\(|if.*provider.*==.*synthetic" crates/pierre-server/src/tools/ crates/pierre-server/src/protocols/universal/handlers/ -g "!*synthetic_provider.rs" -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_MAGIC"
    VALIDATION_FAILED=true
fi

printf "│ %-35s │ %5d │ " "Underscore-prefixed names" "$PROBLEMATIC_UNDERSCORE_NAMES"
if [ "$PROBLEMATIC_UNDERSCORE_NAMES" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Good naming conventions"
else
    FIRST_UNDERSCORE=$(get_first_location 'rg "fn _|let _[a-zA-Z]|struct _|enum _" crates/pierre-server/src/ | rg -v "let _[[:space:]]*=" -n')
    printf "$(format_status "⚠️ WARN")│ %-39s │\n" "$FIRST_UNDERSCORE"
fi

printf "│ %-35s │ %5d │ " "Test modules in crates/pierre-server/src/" "$CFG_TEST_IN_SRC"
if [ "$CFG_TEST_IN_SRC" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Tests in crates/pierre-server/tests/ directory"
else
    FIRST_CFG=$(get_first_location 'rg "#\[cfg\(test\)\]" crates/pierre-server/src/ -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_CFG"
    VALIDATION_FAILED=true
fi

printf "│ %-35s │ %5d │ " "Problematic clippy allows" "$CLIPPY_ALLOWS_PROBLEMATIC"
if [ "$CLIPPY_ALLOWS_PROBLEMATIC" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Fix issues, don't silence"
else
    FIRST_ALLOW=$(get_first_location 'rg "#\[allow\(clippy::" crates/pierre-server/src/ -g "!crates/pierre-server/src/routes/openapi.rs" | rg -v "cast_possible_truncation|cast_sign_loss|cast_precision_loss|cast_possible_wrap|struct_excessive_bools|too_many_lines|let_unit_value|option_if_let_else|cognitive_complexity|bool_to_int_with_if|type_complexity|too_many_arguments|use_self" -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_ALLOW"
fi

printf "│ %-35s │ %5d │ " "Dead code annotations" "$DEAD_CODE"
if [ "$DEAD_CODE" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Remove, don't hide"
else
    FIRST_DEAD=$(get_first_location 'rg "#\[allow\(dead_code\)\]" crates/pierre-server/src/ -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_DEAD"
    VALIDATION_FAILED=true
fi

printf "│ %-35s │ %5d │ " "Temporary solutions" "$TEMP_SOLUTIONS"
if [ "$TEMP_SOLUTIONS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "No temporary code"
else
    FIRST_TEMP=$(get_first_location 'rg "\bhack\b|\bworkaround\b" crates/pierre-server/src/ -n')
    printf "$(format_status "⚠️ WARN")│ %-39s │\n" "$FIRST_TEMP"
fi

printf "│ %-35s │ %5d │ " "Ignored tests (total)" "$IGNORED_TESTS"
if [ "$IGNORED_TESTS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "All tests run in CI/CD"
elif [ "$IGNORED_TESTS_UNAUTHORIZED" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "All $IGNORED_TESTS in allowlist"
else
    # Find first unauthorized ignored test (not in allowlist files)
    ALLOWLIST_GREP=$(echo "$IGNORED_TESTS_ALLOWLIST_FILES" | tr ' ' '|' | sed 's/tests\///g')
    FIRST_IGNORED=$(rg '#\[ignore' crates/pierre-server/tests/ -l 2>/dev/null | grep -v -E "$ALLOWLIST_GREP" | head -1)
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$IGNORED_TESTS_UNAUTHORIZED in: $FIRST_IGNORED"
fi

printf "│ %-35s │ %5d │ " "Ignored doctests" "$IGNORED_DOCTESTS"
if [ "$IGNORED_DOCTESTS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "All doctests compiled/tested"
else
    FIRST_DOCTEST=$(get_first_location 'rg "///\s*\`\`\`(rust,\s*)?ignore" crates/pierre-server/src/ -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_DOCTEST"
fi

printf "│ %-35s │ %5d │ " "CI continue-on-error: true" "$CI_CONTINUE_ON_ERROR"
if [ "$CI_CONTINUE_ON_ERROR" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Tests fail the build"
else
    FIRST_COE=$(rg "continue-on-error:\s*true" .github/workflows/ -n 2>/dev/null | grep -v "^\s*#" | head -1 | cut -d: -f1-2)
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$(truncate_text "$FIRST_COE" 37)"
fi

printf "│ %-35s │ %5d │ " "JS/TS skipped tests" "$JS_TEST_SKIPS"
if [ "$JS_TEST_SKIPS" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "All JS/TS tests run"
else
    FIRST_SKIP=$(rg '\.skip\(\)|\.only\(\)|xit\(|xdescribe\(' frontend/ frontend-mobile/ sdk/ -g '*.test.*' -n 2>/dev/null | head -1 | cut -d: -f1-2)
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$(truncate_text "$FIRST_SKIP" 37)"
fi

printf "│ %-35s │ %5d │ " "Backup files" "${BACKUP_FILES:-0}"
if [ "${BACKUP_FILES:-0}" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "No backup files"
else
    FIRST_BACKUP=$(find crates/pierre-server/src/ -name "*.bak" -o -name "*.backup" -o -name "*~" 2>/dev/null | head -1)
    printf "$(format_status "⚠️ WARN")│ %-39s │\n" "$(truncate_text "$FIRST_BACKUP" 37)"
fi

printf "│ %-35s │ %5d │ " "Legacy UX anti-patterns" "$LEGACY_ISSUES"
if [ "$LEGACY_ISSUES" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "No legacy functions"
else
    FIRST_LEGACY=$(get_first_location 'rg "Legacy OAuth not supported|connect_strava|deprecated.*use.*instead" crates/pierre-server/src/ -n')
    printf "$(format_status "❌ FAIL")│ %-39s │\n" "$FIRST_LEGACY"
fi

echo "├─────────────────────────────────────┼───────┼──────────┼─────────────────────────────────────────┤"

# Memory Management Analysis
printf "│ %-35s │ %5d │ " "Problematic clones" "$PROBLEMATIC_CLONES"
if [ "$PROBLEMATIC_CLONES" -eq 0 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "All clones documented"
else
    FIRST_CLONE=$(get_first_location 'rg "\.clone\(\)" crates/pierre-server/src/ | rg -v "// Safe|Arc::|String::from" -n')
    printf "$(format_status "⚠️ WARN")│ %-39s │\n" "$FIRST_CLONE"
fi

printf "│ %-35s │ %5d │ " "Arc usage" "$TOTAL_ARCS"
if [ "$TOTAL_ARCS" -lt 100 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Appropriate for architecture"
else
    printf "$(format_status "⚠️ WARN")│ %-39s │\n" "High Arc usage, review"
fi

printf "│ %-35s │ %5d │ " "Magic numbers" "$MAGIC_NUMBERS"
if [ "$MAGIC_NUMBERS" -lt 10 ]; then
    printf "$(format_status "✅ PASS")│ %-39s │\n" "Good configuration practices"
else
    FIRST_MAGIC=$(get_first_location 'rg "\b[0-9]{4,}\b" crates/pierre-server/src/ -g "!crates/pierre-server/src/constants.rs" -g "!crates/pierre-server/src/config/*" | grep -v -E "(http://|https://|Duration)" -n')
    printf "$(format_status "⚠️ WARN")│ %-39s │\n" "$FIRST_MAGIC"
fi

echo "└─────────────────────────────────────┴───────┴──────────┴─────────────────────────────────────────┘"

# ============================================================================
# ROUTE FILE SIZE ENFORCEMENT
# ============================================================================
# Route handlers must be thin — business logic belongs in services/.
# Any route file over 500 lines likely contains business logic that should
# be extracted to a service module.

ROUTE_SIZE_LIMIT=1750
OVERSIZED_ROUTES=0
OVERSIZED_LIST=""

for route_file in $(find crates/pierre-server/src/routes -name "*.rs" -not -name "mod.rs" 2>/dev/null); do
    LINES=$(wc -l < "$route_file" | tr -d ' ')
    if [ "$LINES" -gt "$ROUTE_SIZE_LIMIT" ]; then
        OVERSIZED_ROUTES=$((OVERSIZED_ROUTES + 1))
        BASENAME=$(basename "$route_file")
        OVERSIZED_LIST="${OVERSIZED_LIST}  ${BASENAME} (${LINES} lines)\n"
    fi
done

if [ "$OVERSIZED_ROUTES" -gt 0 ]; then
    echo ""
    echo -e "${RED}❌ FORBIDDEN: Found ${OVERSIZED_ROUTES} route file(s) exceeding ${ROUTE_SIZE_LIMIT} lines${NC}"
    echo -e "${RED}Route handlers must be thin — extract business logic to services/${NC}"
    echo ""
    echo -e "Violations:"
    echo -e "$OVERSIZED_LIST"
    VALIDATION_FAILED=true
fi

# ============================================================================
# BUILD PROFILE GUARDRAILS
# ============================================================================
# Prevent regressions that silently multiply release build times.

echo ""
echo -e "${BLUE}==== Build Profile Guardrails ====${NC}"

# Check codegen-units is not 1 in [profile.release] (forces single-threaded LLVM codegen)
RELEASE_CU=$(grep -A5 '^\[profile\.release\]' "$PROJECT_ROOT/Cargo.toml" | grep 'codegen-units' | head -1 | grep -o '[0-9]*')
if [ -n "$RELEASE_CU" ] && [ "$RELEASE_CU" -eq 1 ]; then
    echo -e "${RED}❌ FORBIDDEN: [profile.release] has codegen-units = 1${NC}"
    echo -e "${RED}This forces single-threaded LLVM codegen and multiplies build time by 4-8x.${NC}"
    echo -e "${RED}Use codegen-units = 16 (default). Reserve codegen-units = 1 for [profile.release-lto] only.${NC}"
    VALIDATION_FAILED=true
else
    echo -e "${GREEN}  ✅ Release profile codegen-units OK (${RELEASE_CU:-default})${NC}"
fi

# Check for heavy duplicate crates that indicate transitive dependency divergence
HEAVY_DUPES=""
for CRATE in chromiumoxide sqlx axum tokio hyper; do
    DISTINCT_VERSIONS=$(cargo tree --duplicates 2>/dev/null | grep "^${CRATE} v" | sed 's/ (.*//' | sort -u | wc -l | tr -d ' ')
    if [ "$DISTINCT_VERSIONS" -gt 1 ]; then
        VERSION_LIST=$(cargo tree --duplicates 2>/dev/null | grep "^${CRATE} v" | sed 's/ (.*//' | sort -u | tr '\n' ', ' | sed 's/,$//')
        HEAVY_DUPES="${HEAVY_DUPES}\n  - ${VERSION_LIST}"
    fi
done

if [ -n "$HEAVY_DUPES" ]; then
    echo -e "${RED}❌ FORBIDDEN: Heavy crates compiled in multiple versions:${HEAVY_DUPES}${NC}"
    echo -e "${RED}Run 'cargo tree --duplicates' to trace the source and deduplicate.${NC}"
    VALIDATION_FAILED=true
else
    echo -e "${GREEN}  ✅ No heavy crate duplication detected${NC}"
fi

# ============================================================================
# SUMMARY
# ============================================================================

# ============================================================================
# SECURITY SKILL SCRIPTS (opt-in via --apply-skills)
# ============================================================================

if [ "$APPLY_SKILLS" = true ]; then
    echo ""
    echo -e "${BLUE}==== Running Security Skill Scripts ====${NC}"

    if [ -x "$SCRIPT_DIR/security-review.sh" ]; then
        "$SCRIPT_DIR/security-review.sh" || VALIDATION_FAILED=true
    else
        echo -e "${YELLOW}⚠️  scripts/ci/security-review.sh not found or not executable${NC}"
    fi

    if [ -x "$SCRIPT_DIR/check-input-validation.sh" ]; then
        "$SCRIPT_DIR/check-input-validation.sh" || VALIDATION_FAILED=true
    else
        echo -e "${YELLOW}⚠️  scripts/ci/check-input-validation.sh not found or not executable${NC}"
    fi
fi

echo ""
echo -e "${BLUE}==== Architectural Validation Summary ====${NC}"

if [ "$VALIDATION_FAILED" = true ]; then
    echo -e "${RED}❌ Architectural validation FAILED${NC}"
    echo -e "${RED}Fix critical issues above before deployment${NC}"
    exit 1
else
    echo -e "${GREEN}✅ All architectural validations passed${NC}"
    exit 0
fi
