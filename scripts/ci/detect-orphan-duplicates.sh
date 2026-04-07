#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Detects parallel systems and orphan duplications that violate SSOT
# ABOUTME: Scans for env::var bypass, stale fallback constants, and shadow registries

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"

echo -e "${BLUE}==== Pierre — Orphan Duplication Detection ====${NC}"
cd "$PROJECT_ROOT"

FAILED=false

# ---------------------------------------------------------------------------
# 1. env::var("BASE_URL") bypass — should use resources.config.base_url
# ---------------------------------------------------------------------------
echo -e "\n${BLUE}[1/5] Checking for env::var(\"BASE_URL\") bypass in route handlers...${NC}"

# Count occurrences in routes/ and mcp/ (excluding config/ and bin/ which are legitimate)
BYPASS_COUNT=$(grep -rn 'env::var("BASE_URL")' \
    crates/pierre-server/src/routes/ \
    crates/pierre-server/src/mcp/ \
    --include='*.rs' 2>/dev/null | wc -l)

if [ "$BYPASS_COUNT" -gt 0 ]; then
    echo -e "${YELLOW}  WARNING: $BYPASS_COUNT env::var(\"BASE_URL\") call(s) in route/mcp handlers${NC}"
    grep -rn 'env::var("BASE_URL")' \
        crates/pierre-server/src/routes/ \
        crates/pierre-server/src/mcp/ \
        --include='*.rs' 2>/dev/null | sed 's/^/    /'
    echo -e "${YELLOW}  These should use resources.config.base_url instead${NC}"
    # Warning only — tracked for consolidation in SSOT doc
else
    echo -e "${GREEN}  OK — no env::var(\"BASE_URL\") bypass in handlers${NC}"
fi

# ---------------------------------------------------------------------------
# 2. FALLBACK_BASE_URL constants — indicate env::var bypass
# ---------------------------------------------------------------------------
echo -e "\n${BLUE}[2/5] Checking for FALLBACK_BASE_URL constants...${NC}"

FALLBACK_COUNT=$(grep -rn 'FALLBACK_BASE_URL' \
    crates/pierre-server/src/ \
    --include='*.rs' 2>/dev/null | wc -l)

if [ "$FALLBACK_COUNT" -gt 0 ]; then
    echo -e "${YELLOW}  WARNING: $FALLBACK_COUNT FALLBACK_BASE_URL reference(s)${NC}"
    grep -rn 'FALLBACK_BASE_URL' \
        crates/pierre-server/src/ \
        --include='*.rs' 2>/dev/null | sed 's/^/    /'
    echo -e "${YELLOW}  These indicate env::var bypass — should use ServerConfig${NC}"
else
    echo -e "${GREEN}  OK — no FALLBACK_BASE_URL constants${NC}"
fi

# ---------------------------------------------------------------------------
# 3. Shadow tool registries — anything defining tool schemas outside ToolRegistry
# ---------------------------------------------------------------------------
echo -e "\n${BLUE}[3/5] Checking for shadow tool registries...${NC}"

# Look for get_tools functions outside the canonical registry and known delegators.
# Excluded: registry.rs (canonical), traits.rs (trait def), mcp_request_processor.rs
# (delegates to registry), executor.rs (delegates to self.registry), schema/mod.rs
# (test helper that instantiates ToolRegistry — documented as non-production).
SHADOW_TOOLS=$(grep -rn 'fn get_tools\|fn list_tools\|fn tool_schemas' \
    crates/pierre-server/src/ \
    --include='*.rs' 2>/dev/null \
    | grep -v 'registry.rs' \
    | grep -v 'traits.rs' \
    | grep -v 'mcp_request_processor.rs' \
    | grep -v 'executor.rs' \
    | grep -v 'schema/mod.rs' \
    | grep -v '#\[cfg(test)\]' \
    | grep -v '// ' \
    | grep -v 'tests/' || true)

if [ -n "$SHADOW_TOOLS" ]; then
    echo -e "${RED}  FAIL: Shadow tool registration detected:${NC}"
    echo "$SHADOW_TOOLS" | sed 's/^/    /'
    FAILED=true
else
    echo -e "${GREEN}  OK — no shadow tool registries${NC}"
fi

# ---------------------------------------------------------------------------
# 4. Parallel repository dispatch — enum-based dispatch outside factory
# ---------------------------------------------------------------------------
echo -e "\n${BLUE}[4/5] Checking for parallel database dispatch patterns...${NC}"

# Look for match arms on database backend enums in query paths
ENUM_DISPATCH=$(grep -rn 'DatabaseBackend::Sqlite\|DatabaseBackend::Postgres\|Backend::Sqlite\|Backend::Postgres' \
    crates/pierre-database/src/ \
    crates/pierre-server/src/ \
    --include='*.rs' 2>/dev/null \
    | grep -v 'repository_registry.rs' \
    | grep -v 'factory' \
    | grep -v 'social_dispatch.rs' \
    | grep -v 'tests/' \
    | grep -v '// ' || true)

if [ -n "$ENUM_DISPATCH" ]; then
    echo -e "${RED}  FAIL: Enum-based database dispatch outside factory:${NC}"
    echo "$ENUM_DISPATCH" | sed 's/^/    /'
    FAILED=true
else
    echo -e "${GREEN}  OK — no parallel database dispatch${NC}"
fi

# ---------------------------------------------------------------------------
# 5. Parallel type definitions — inline interfaces duplicating shared-types
# ---------------------------------------------------------------------------
echo -e "\n${BLUE}[5/5] Checking for parallel type definitions in frontend/mobile...${NC}"

# Look for interface definitions that should be in shared-types
PARALLEL_TYPES=0
for DIR in frontend/src frontend-mobile/src; do
    if [ -d "$DIR" ]; then
        # Check for Activity/Coach/User interface declarations not from re-exports
        FOUND=$(grep -rn 'export interface Activity \|export interface Coach \|export interface UserProfile ' \
            "$DIR" \
            --include='*.ts' --include='*.tsx' 2>/dev/null \
            | grep -v 'node_modules' \
            | grep -v 'shared-types' \
            | grep -v 're-export' \
            | grep -v '// from' || true)
        if [ -n "$FOUND" ]; then
            PARALLEL_TYPES=$((PARALLEL_TYPES + $(echo "$FOUND" | wc -l)))
            echo "$FOUND" | sed 's/^/    /'
        fi
    fi
done

if [ "$PARALLEL_TYPES" -gt 0 ]; then
    echo -e "${YELLOW}  WARNING: $PARALLEL_TYPES potential parallel type definition(s)${NC}"
    echo -e "${YELLOW}  Core types should come from @pierre/shared-types${NC}"
else
    echo -e "${GREEN}  OK — no parallel type definitions detected${NC}"
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
if [ "$FAILED" = true ]; then
    echo -e "${RED}==== FAIL: Orphan duplication detected — see above ====${NC}"
    exit 1
else
    echo -e "${GREEN}==== PASS: No orphan duplications detected ====${NC}"
    exit 0
fi
