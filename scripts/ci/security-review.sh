#!/bin/bash
# ABOUTME: Automated security review for CI — validates authorization, tenant isolation, logging, and query safety
# ABOUTME: Companion script to .claude/skills/security-review/SKILL.md with machine-enforceable checks
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

VALIDATION_FAILED=false

pass() { echo -e "${GREEN}  ✅ $1${NC}"; }
warn() { echo -e "${YELLOW}  ⚠️  $1${NC}"; }
fail() { echo -e "${RED}  ❌ $1${NC}"; VALIDATION_FAILED=true; }

echo -e "${BLUE}=========================================${NC}"
echo -e "${BLUE}  SECURITY REVIEW (CI)${NC}"
echo -e "${BLUE}=========================================${NC}"

# ============================================================================
# 1. Authorization Boundaries (informational — hard to fully automate)
# ============================================================================
echo ""
echo -e "${BLUE}--- 1. Authorization Boundaries ---${NC}"

SUPER_ADMIN_CHECKS=$(rg "super.?admin|SuperAdmin" crates/pierre-server/src/routes/ --type rust -l 2>/dev/null | wc -l | tr -d ' ')
if [ "$SUPER_ADMIN_CHECKS" -gt 0 ]; then
    SUPER_ADMIN_GATING=$(rg "is_super_admin" crates/pierre-server/src/routes/ --type rust --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
    if [ "$SUPER_ADMIN_GATING" -gt 0 ]; then
        pass "Super-admin gating found ($SUPER_ADMIN_GATING checks across routes)"
    else
        warn "Routes reference super-admin but no is_super_admin checks found"
    fi
else
    pass "No super-admin routes to check"
fi

# ============================================================================
# 2. Multi-Tenant Isolation (smoke gate)
# ============================================================================
echo ""
echo -e "${BLUE}--- 2. Multi-Tenant Isolation ---${NC}"

# Count SQL queries and those referencing tenant_id.
# Scan all non-test crate sources: since the repository-pattern decomposition the
# raw SQL no longer lives in crates/pierre-server/src (it moved to
# crates/pierre-database/src), so scanning only the server crate counted a tiny,
# meaningless slice. Excluding tests/ and migrations/ keeps this to production query sites.
TOTAL_SQL=$(rg "sqlx::query" crates/ --type rust -g '!**/tests/**' -g '!**/migrations/**' --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
SQL_WITH_TENANT=$(rg "sqlx::query" crates/ --type rust -g '!**/tests/**' -g '!**/migrations/**' -A 10 2>/dev/null | rg "tenant_id" | wc -l | tr -d ' ')

if [ "$TOTAL_SQL" -eq 0 ]; then
    # The workspace must contain SQL query sites. Zero means this scan is
    # mis-pointed (e.g. the SQL moved crates again) — fail loudly rather than
    # silently pass, which is exactly the defect this gate previously had.
    fail "No sqlx query sites found in crates/ — scan path is stale (SQL relocated?)"
elif [ "$SQL_WITH_TENANT" -eq 0 ]; then
    # Queries exist but none reference tenant_id: a total multi-tenant isolation regression.
    fail "Found $TOTAL_SQL SQL query sites but none reference tenant_id — multi-tenant isolation regression"
else
    pass "SQL queries: $TOTAL_SQL total, $SQL_WITH_TENANT reference tenant_id"
    echo -e "    ${YELLOW}note:${NC} per-query tenant scoping is enforced at the test layer (test-multitenant-isolation); this is a coarse smoke check"
fi

# ============================================================================
# 3. Logging Hygiene (HARD FAIL)
# ============================================================================
echo ""
echo -e "${BLUE}--- 3. Logging Hygiene ---${NC}"

# Check for sensitive data in log statements at INFO+ level
# Strategy: match log macros that interpolate actual secret values as variables
# Pattern 1: Inline interpolation like {access_token} or {password}
# Pattern 2: Positional args like info!("...", access_token) — the secret as a trailing arg
# Scope: all crates (secret-bearing code moved into pierre-providers/pierre-auth/pierre-llm
# after the workspace split, so scanning only pierre-server missed most log sites).
# Excludes: tests, bin/ CLIs, IDs (_id suffix), failure/error descriptions
SECRETS_INLINE=$(rg '(info!|warn!|error!)\(.*\{(access_token|refresh_token|client_secret|api_key|password|secret_key)\}' crates/ --type rust -g '!**/tests/**' -g '!**/bin/**' -n 2>/dev/null | \
  rg -v 'redact|REDACT|mask|\*\*\*' | wc -l | tr -d ' ')
SECRETS_POSITIONAL=$(rg '(info!|warn!|error!)\(.*,\s*(access_token|refresh_token|client_secret|api_key(?!_id)|password|secret_key)\s*[,)]' crates/ --type rust -g '!**/tests/**' -g '!**/bin/**' -n 2>/dev/null | \
  rg -v 'redact|REDACT|mask|\*\*\*' | wc -l | tr -d ' ')
SECRETS_IN_LOGS=$((SECRETS_INLINE + SECRETS_POSITIONAL))

if [ "$SECRETS_IN_LOGS" -eq 0 ]; then
    pass "No secrets detected in INFO+ log statements"
else
    fail "Found $SECRETS_IN_LOGS potential secrets in log statements"
    rg '(info!|warn!|error!)\(.*\{(access_token|refresh_token|client_secret|api_key|password|secret_key)\}' crates/ --type rust -g '!**/tests/**' -g '!**/bin/**' -n 2>/dev/null | \
      rg -v 'redact|REDACT|mask|\*\*\*' | head -3
    rg '(info!|warn!|error!)\(.*,\s*(access_token|refresh_token|client_secret|api_key(?!_id)|password|secret_key)\s*[,)]' crates/ --type rust -g '!**/tests/**' -g '!**/bin/**' -n 2>/dev/null | \
      rg -v 'redact|REDACT|mask|\*\*\*' | head -3
fi

# ============================================================================
# 4. OAuth & Protocol (informational)
# ============================================================================
echo ""
echo -e "${BLUE}--- 4. OAuth & Protocol Compliance ---${NC}"

STATE_VALIDATION=$(rg "state.*param|validate.*state|verify.*state|state_matches" crates/pierre-server/src/ --type rust --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')
PKCE_REFS=$(rg "code_challenge|code_verifier" crates/pierre-server/src/ --type rust --count 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}')

if [ "$STATE_VALIDATION" -gt 0 ]; then
    pass "OAuth state validation: $STATE_VALIDATION references"
else
    warn "No OAuth state validation patterns found"
fi

if [ "$PKCE_REFS" -gt 0 ]; then
    pass "PKCE enforcement: $PKCE_REFS references"
else
    warn "No PKCE references found"
fi

# ============================================================================
# 5. Template & Query Safety (HARD FAIL)
# ============================================================================
echo ""
echo -e "${BLUE}--- 5. Template & Query Safety ---${NC}"

# Check for format! used to build SQL queries (injection risk)
# Scope: all crates — the raw SQL lives in pierre-database since the repository-pattern
# split, so scanning only pierre-server missed every real query-builder site.
# Excludes:
#   - dynamic query builders that use bind parameters (where_clause with ?N or $N)
#   - table_name() — a `const fn -> &'static str` table identifier; SQL forbids binding
#     identifiers as parameters, so interpolating a compile-time-constant table name is
#     the only available idiom and is not an injection vector.
SQL_FORMAT_EXCLUDE='test|//.*format|\$[0-9]|\?[0-9]|where_clause|bind_values|push_bind|param_index|placeholder|table_name\(\)'
FORMAT_SQL=$(rg 'format!\(.*(?:SELECT|INSERT|UPDATE|DELETE|CREATE|DROP|ALTER)' crates/ --type rust -g '!**/tests/**' -n 2>/dev/null | \
  rg -v "$SQL_FORMAT_EXCLUDE" | \
  wc -l | tr -d ' ')

if [ "$FORMAT_SQL" -eq 0 ]; then
    pass "No format!() SQL injection risks"
else
    fail "Found $FORMAT_SQL format!() SQL construction patterns"
    rg 'format!\(.*(?:SELECT|INSERT|UPDATE|DELETE|CREATE|DROP|ALTER)' crates/ --type rust -g '!**/tests/**' -n 2>/dev/null | \
      rg -v "$SQL_FORMAT_EXCLUDE" | head -5
fi

# Check for unescaped HTML interpolation (server-only: HTML responses are rendered
# exclusively in pierre-server; no other crate emits text/html).
HTML_UNESCAPED=$(rg 'text/html|Content-Type.*html' crates/pierre-server/src/ --type rust -B 5 -A 10 2>/dev/null | \
  rg 'format!' | rg -v 'html_escape|encode_text' | wc -l | tr -d ' ')

if [ "$HTML_UNESCAPED" -eq 0 ]; then
    pass "HTML output properly escaped"
else
    fail "Found $HTML_UNESCAPED unescaped HTML interpolation patterns"
    rg 'text/html|Content-Type.*html' crates/pierre-server/src/ --type rust -B 5 -A 10 -n 2>/dev/null | \
      rg 'format!' | rg -v 'html_escape|encode_text' | head -5
fi

# ============================================================================
# 6. Static OAuth/Config State (HARD FAIL)
# ============================================================================
echo ""
echo -e "${BLUE}--- 6. Tenant Isolation in Non-DB Code ---${NC}"

# Check for global mutable OAuth credential storage that should be per-tenant
# Excludes: read-only app config (ServerConfig, RouteTimeoutConfig), comment lines, provider definitions
GLOBAL_OAUTH_STATE=$(rg 'static.*OAuth.*Mutex|static.*OAuth.*RwLock|LazyLock.*OAuth.*token|LazyLock.*OAuth.*credential' crates/pierre-server/src/ --type rust -n 2>/dev/null | \
  rg -v 'test|//|DEFAULT' | wc -l | tr -d ' ')

if [ "$GLOBAL_OAUTH_STATE" -eq 0 ]; then
    pass "No global mutable OAuth credential storage"
else
    fail "Found $GLOBAL_OAUTH_STATE global OAuth credential storage patterns (should be per-tenant)"
    rg 'static.*OAuth.*Mutex|static.*OAuth.*RwLock|LazyLock.*OAuth.*token|LazyLock.*OAuth.*credential' crates/pierre-server/src/ --type rust -n 2>/dev/null | \
      rg -v 'test|//|DEFAULT' | head -5
fi

# ============================================================================
# SUMMARY
# ============================================================================
echo ""
echo -e "${BLUE}=========================================${NC}"
if [ "$VALIDATION_FAILED" = true ]; then
    echo -e "${RED}  SECURITY REVIEW: FAILED${NC}"
    echo -e "${RED}  Fix issues above before merging${NC}"
    echo -e "${BLUE}=========================================${NC}"
    exit 1
else
    echo -e "${GREEN}  SECURITY REVIEW: PASSED${NC}"
    echo -e "${BLUE}=========================================${NC}"
    exit 0
fi
