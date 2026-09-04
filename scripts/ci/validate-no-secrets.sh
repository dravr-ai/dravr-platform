#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: CI validation script to detect secret patterns that should never appear in logs or code
# ABOUTME: Prevents PII leakage, credential exposure, and GDPR/CCPA violations
#
# Licensed under either of Apache License, Version 2.0 or MIT License at your option.
# Copyright (c) 2026 dravr.ai

# Pierre MCP Server - Secret Pattern Detection
# This script validates that sensitive data patterns are not present in source code or logs

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}==== Pierre MCP Server - Secret Pattern Detection ====${NC}"
echo ""

# Get the directory where this script is located
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"

# Change to project root
cd "$PROJECT_ROOT"

VALIDATION_FAILED=false

# Function to report validation failure
fail_validation() {
    echo -e "${RED}❌ SECRET PATTERN DETECTED${NC}"
    echo -e "${RED}$1${NC}"
    VALIDATION_FAILED=true
}

# Function to report success
pass_validation() {
    echo -e "${GREEN}✅ $1${NC}"
}

# Scan surface: production Rust source of EVERY workspace crate. A secret is not
# less committed for living in pierre-auth, pierre-database or pierre-providers,
# and scoping this to one crate left 89% of the workspace unscanned.
SCAN_ROOT="crates"
SRC_GLOB=(-g 'crates/*/src/**/*.rs')

# Escape hatch, same idiom as `// file-size-ok:` and `-- idempotency-ok:`: a line
# carrying `// secret-scan-ok: <reason>` is dropped from every scan below. The
# reason lives on the line it excuses rather than in a central list of paths that
# nobody prunes — which is how this script came to exclude a file deleted months
# earlier. The one current use is the plaintext-PEM detector, whose doc example
# has to contain PEM armour to demonstrate what it recognises.
SCAN_SKIP_MARKER='secret-scan-ok:'

# Print `path:line:text` for every source line matching $1, minus skipped lines.
scan_src() {
    rg -i --no-heading --line-number -e "$1" "${SRC_GLOB[@]}" "$SCAN_ROOT" 2>/dev/null \
        | grep -v "$SCAN_SKIP_MARKER" || true
}

# Count the lines a scan produced (0 when it produced none).
count_lines() {
    grep -c . || true
}

SRC_FILE_COUNT=$(rg --files "${SRC_GLOB[@]}" "$SCAN_ROOT" 2>/dev/null | count_lines)
if [ "$SRC_FILE_COUNT" -eq 0 ]; then
    echo -e "${RED}❌ Secret scan selected no source files under ${SCAN_ROOT}/${NC}"
    echo -e "${RED}A scan over an empty selection passes forever. Repoint SRC_GLOB.${NC}"
    exit 1
fi

echo -e "${BLUE}Scanning $SRC_FILE_COUNT crate source files for secret patterns...${NC}"
echo ""

# ============================================================================
# CRITICAL PATTERNS: Authorization tokens and credentials
# ============================================================================

echo -e "${BLUE}[1/7] Checking for exposed authorization tokens...${NC}"
# Only match Bearer tokens with actual token values (20+ chars), not documentation
EXPOSED_TOKEN_HITS=$(scan_src "bearer\s+[A-Za-z0-9\.\-_]{20,}")
EXPOSED_TOKENS=$(printf '%s' "$EXPOSED_TOKEN_HITS" | count_lines)
if [ "$EXPOSED_TOKENS" -eq 0 ]; then
    pass_validation "No authorization tokens found in source code"
else
    fail_validation "Found $EXPOSED_TOKENS authorization tokens in source code"
    echo -e "${YELLOW}Locations:${NC}"
    printf '%s\n' "$EXPOSED_TOKEN_HITS" | head -5
    echo ""
fi

# ============================================================================
# CRITICAL PATTERNS: API keys and secrets
# ============================================================================

echo -e "${BLUE}[2/7] Checking for hardcoded API keys...${NC}"
HARDCODED_KEY_HITS=$(scan_src "api[_-]?key\s*[=:]\s*['\"][a-zA-Z0-9]{20,}['\"]|client[_-]?secret\s*[=:]\s*['\"][a-zA-Z0-9]{20,}['\"]")
HARDCODED_KEYS=$(printf '%s' "$HARDCODED_KEY_HITS" | count_lines)
if [ "$HARDCODED_KEYS" -eq 0 ]; then
    pass_validation "No hardcoded API keys found in source code"
else
    fail_validation "Found $HARDCODED_KEYS hardcoded API keys in source code"
    echo -e "${YELLOW}Locations:${NC}"
    printf '%s\n' "$HARDCODED_KEY_HITS" | head -5
    echo ""
fi

# ============================================================================
# CRITICAL PATTERNS: Passwords in code
# ============================================================================

echo -e "${BLUE}[3/7] Checking for hardcoded passwords...${NC}"
HARDCODED_PASSWORD_HITS=$(scan_src "password\s*[=:]\s*['\"][^'\"]{8,}['\"]")
HARDCODED_PASSWORDS=$(printf '%s' "$HARDCODED_PASSWORD_HITS" | count_lines)
if [ "$HARDCODED_PASSWORDS" -eq 0 ]; then
    pass_validation "No hardcoded passwords found in production code"
else
    fail_validation "Found $HARDCODED_PASSWORDS hardcoded passwords in production code"
    echo -e "${YELLOW}Locations:${NC}"
    printf '%s\n' "$HARDCODED_PASSWORD_HITS" | head -5
    echo ""
fi

# ============================================================================
# CRITICAL PATTERNS: JWT tokens
# ============================================================================

echo -e "${BLUE}[4/7] Checking for exposed JWT tokens...${NC}"
EXPOSED_JWT_HITS=$(scan_src "eyJ[a-zA-Z0-9_-]+\.[a-zA-Z0-9_-]+\.[a-zA-Z0-9_-]+")
EXPOSED_JWTS=$(printf '%s' "$EXPOSED_JWT_HITS" | count_lines)
if [ "$EXPOSED_JWTS" -eq 0 ]; then
    pass_validation "No JWT tokens found in production code"
else
    fail_validation "Found $EXPOSED_JWTS JWT tokens in production code"
    echo -e "${YELLOW}Locations:${NC}"
    printf '%s\n' "$EXPOSED_JWT_HITS" | head -5
    echo ""
fi

# ============================================================================
# WARNING PATTERNS: Private keys (RSA, SSH, etc.)
# ============================================================================

echo -e "${BLUE}[5/7] Checking for private keys...${NC}"
PRIVATE_KEY_HITS=$(scan_src "-----BEGIN (RSA |EC |DSA )?PRIVATE KEY-----")
PRIVATE_KEYS=$(printf '%s' "$PRIVATE_KEY_HITS" | count_lines)
if [ "$PRIVATE_KEYS" -eq 0 ]; then
    pass_validation "No private keys found in source code"
else
    fail_validation "Found $PRIVATE_KEYS private keys in source code"
    echo -e "${YELLOW}Locations:${NC}"
    printf '%s\n' "$PRIVATE_KEY_HITS" | head -5
    echo ""
fi

# ============================================================================
# WARNING PATTERNS: Unredacted PII in logs
# ============================================================================

echo -e "${BLUE}[6/7] Checking for potential PII leakage patterns...${NC}"
# Check for logging statements that might leak PII without redaction
PII_LOGGING_HITS=$(scan_src "log::|tracing::|info!|debug!|warn!|error!" \
    | rg -i "email|password|token|secret|authorization|cookie|session" \
    | rg -v "// Safe|redact|mask" || true)
PII_LOGGING=$(printf '%s' "$PII_LOGGING_HITS" | count_lines)
if [ "$PII_LOGGING" -eq 0 ]; then
    pass_validation "No obvious PII leakage patterns in logging statements"
else
    echo -e "${YELLOW}⚠️  Found $PII_LOGGING logging statements that may leak PII${NC}"
    echo -e "${YELLOW}Review these locations to ensure PII is properly redacted:${NC}"
    printf '%s\n' "$PII_LOGGING_HITS" | head -10
    echo -e "${YELLOW}Note: This is a warning - verify that redaction is applied${NC}"
    echo ""
fi

# ============================================================================
# WARNING PATTERNS: Database connection strings with credentials
# ============================================================================

echo -e "${BLUE}[7/7] Checking for database connection strings with embedded credentials...${NC}"
DB_CREDENTIAL_HITS=$(scan_src "postgres://[^:]+:[^@]+@|mysql://[^:]+:[^@]+@|mongodb://[^:]+:[^@]+@")
DB_CREDENTIALS=$(printf '%s' "$DB_CREDENTIAL_HITS" | count_lines)
if [ "$DB_CREDENTIALS" -eq 0 ]; then
    pass_validation "No database connection strings with embedded credentials"
else
    fail_validation "Found $DB_CREDENTIALS database connection strings with embedded credentials"
    echo -e "${YELLOW}Locations:${NC}"
    printf '%s\n' "$DB_CREDENTIAL_HITS" | head -5
    echo ""
fi

# ============================================================================
# SUMMARY
# ============================================================================

echo ""
echo -e "${BLUE}==== Secret Pattern Detection Summary ====${NC}"

if [ "$VALIDATION_FAILED" = true ]; then
    echo -e "${RED}❌ VALIDATION FAILED${NC}"
    echo -e "${RED}Found sensitive data patterns that must be removed before deployment${NC}"
    echo ""
    echo -e "${YELLOW}Remediation steps:${NC}"
    echo -e "${YELLOW}1. Remove hardcoded secrets from source code${NC}"
    echo -e "${YELLOW}2. Use environment variables for sensitive configuration${NC}"
    echo -e "${YELLOW}3. Ensure PII redaction middleware is applied to all logging${NC}"
    echo -e "${YELLOW}4. Use the redaction utilities in crates/pierre-middleware/src/redaction.rs${NC}"
    echo ""
    exit 1
else
    echo -e "${GREEN}✅ All secret pattern validations passed${NC}"
    echo -e "${GREEN}No sensitive data patterns detected in source code${NC}"
    exit 0
fi
