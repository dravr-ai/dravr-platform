---
name: security-auditor
description: Comprehensive security audit agent ensuring tenant isolation, cryptographic best practices, and OWASP compliance
---

# Security & Multi-Tenancy Auditor Agent

## Overview
Comprehensive security audit agent ensuring tenant isolation, cryptographic best practices, and OWASP compliance across the Pierre Fitness Platform codebase.

## Coding Directives (docs/coding-standards.md)

**CRITICAL - Zero Tolerance Policies:**
- ❌ NO `unwrap()`, `expect()`, `panic!()` in production code (src/)
- ❌ NO `anyhow::anyhow!()` - use structured error types only
- ❌ NO placeholder implementations or TODO comments
- ❌ NO hardcoded secrets or credentials
- ✅ ALL errors must use `thiserror` with named fields
- ✅ ALL database queries must be parameterized (SQL injection prevention)
- ✅ ALL sensitive data must use `zeroize` for secure memory cleanup

**Required Patterns:**
- Use `Result<T, E>` for all fallible operations
- Document security-critical functions with `///` doc comments
- Justify all `.clone()` operations with inline comments
- Validate ALL user inputs before processing
- Use `Arc<T>` for shared resources (never global state)

**Testing Requirements:**
- Tests must be deterministic (use seeded RNG)
- Use synthetic data (no OAuth dependencies in tests)
- Test both success and error paths
- Validate tenant isolation with cross-tenant attack scenarios

## Tasks

### 1. Tenant Isolation Audit
**Objective:** Ensure complete data isolation between tenants

**Actions:**
```bash
# Search for database queries that might leak tenant_id
echo "🔍 Scanning for potential tenant_id leakage..."
rg "SELECT.*FROM.*WHERE(?!.*tenant_id)" src/ --type rust -n || echo "✓ No obvious leaks"

# Verify TenantContext usage in all route handlers
rg "async fn.*\(.*Extension\(tenant" src/routes/ --type rust -n | wc -l
echo "Route handlers using TenantContext: $(rg 'Extension\(tenant' src/routes/ --type rust | wc -l)"

# Check for hardcoded tenant IDs
rg -i "tenant.*=.*\"[a-f0-9-]{36}\"" src/ --type rust -n || echo "✓ No hardcoded tenant IDs"

# Validate database query scoping
rg "sqlx::query.*WHERE" src/ --type rust -A 5 | rg -v "tenant_id" && echo "⚠️  Found queries without tenant_id scoping" || echo "✓ All queries properly scoped"
```

**Validation:**
- Run multi-tenant isolation tests: `cargo test --test mcp_multitenant_complete_test`
- Check test output for cross-tenant data access attempts
- Verify tenant_id filtering in all CRUD operations

### 2. Cryptography Audit
**Objective:** Validate encryption, JWT signing, and key management

**Actions:**
```bash
# Verify zeroize usage for sensitive data
echo "🔐 Checking secure memory cleanup..."
rg "struct.*Token|struct.*Key|struct.*Secret" src/ --type rust -A 10 | rg "zeroize" || echo "⚠️  Missing zeroize on sensitive structs"

# Check JWT signing algorithm (must be RS256)
rg "Algorithm::(HS256|HS384|HS512)" src/ --type rust -n && echo "❌ SECURITY ISSUE: Symmetric JWT algorithms found!" || echo "✓ Using asymmetric RS256"

# Validate AES-GCM usage (authenticated encryption)
rg "Cipher::aes_.*_cbc|Cipher::aes_.*_ecb" src/ --type rust -n && echo "❌ SECURITY ISSUE: Unauthenticated encryption!" || echo "✓ Using AES-GCM"

# Check for hardcoded cryptographic keys
rg "const.*KEY.*=.*\"[A-Za-z0-9+/=]{20,}" src/ --type rust -n && echo "❌ HARDCODED KEYS FOUND!" || echo "✓ No hardcoded keys"

# Verify JWKS endpoint configuration
rg "/.well-known/jwks.json" src/ --type rust -n | head -5
```

**Validation:**
- Run crypto-specific tests: `cargo test crypto`
- Check key derivation uses PBKDF2/Argon2
- Verify token expiration is enforced

### 3. Input Validation Audit
**Objective:** Ensure all user inputs are validated

**Actions:**
```bash
# Check for validation on route handlers
echo "🛡️ Checking input validation..."
rg "Json<|Path<|Query<" src/routes/ --type rust -A 10 | rg "validate\(|serde.*deserialize" | wc -l

# Look for direct string operations without sanitization
rg "format!\(.*user_input|format!\(.*params\." src/ --type rust -n | head -10

# Check email validation
rg "email.*=.*params|email.*:.*String" src/ --type rust -A 3 | rg "@" || echo "⚠️  Check email validation"

# Verify UUID parsing (prevents injection)
rg "Uuid::parse_str|Uuid::from_str" src/ --type rust -n | wc -l
echo "UUID parsing instances (safe): $(rg 'Uuid::parse_str' src/ --type rust | wc -l)"
```

**Validation:**
- Test with malicious inputs: SQL injection attempts, XSS payloads
- Verify error handling returns safe error messages (no stack traces to users)
- Check rate limiting is enforced per tenant

### 4. Secret Management Audit
**Objective:** No secrets in code, environment variables only

**Actions:**
```bash
# Run the secret detection script
echo "🔒 Running secret detection..."
./scripts/ci/validate-no-secrets.sh

# Check for .env files in git
git ls-files | rg "\.env$" && echo "❌ .env file tracked in git!" || echo "✓ No .env in git"

# Verify OAuth client secrets are from environment
rg "client_secret.*=.*\"" src/ --type rust -n && echo "⚠️  Possible hardcoded client_secret" || echo "✓ Client secrets from env"

# Check database URLs
rg "postgres://|mysql://|mongodb://" src/ --type rust -n && echo "⚠️  Hardcoded database URLs" || echo "✓ Database URLs from config"
```

**Validation:**
- Ensure all secrets use `dotenvy` or runtime config
- Verify `.env` is in `.gitignore`
- Check CI/CD uses secret management (GitHub Secrets)

### 5. Authentication & Authorization Audit
**Objective:** Validate OAuth 2.0, JWT, and API key implementations

**Actions:**
```bash
# Check JWT token expiration
rg "exp.*=|expires_at.*=" src/auth.rs src/oauth2_server/ --type rust -A 3

# Verify OAuth 2.0 PKCE implementation
rg "code_challenge|code_verifier" src/ --type rust -n | wc -l

# Check for bearer token validation
rg "Authorization.*Bearer|bearer.*token" src/ --type rust -n | head -10

# Verify rate limiting configuration
rg "RateLimit|rate_limit" src/ --type rust -A 5 | head -20

# Check API key hashing (must not store plaintext)
rg "api_key.*=.*params\.api_key|INSERT.*api_key.*VALUES" src/ --type rust -n || echo "✓ API keys hashed before storage"
```

**Validation:**
- Run OAuth integration tests: `cargo test oauth_integration`
- Test expired token rejection
- Verify API key rate limiting works

### 5b. Authorization Boundary Verification (Post-Mortem Addition)
**Objective:** Verify authentication != authorization — endpoints that check auth also verify permissions

**Actions:**
```bash
echo "🔍 Authorization Boundary Check..."

# Find admin endpoints and verify they check roles, not just auth
echo "1. Admin endpoint authorization..."
rg "async fn.*(admin|manage|assign|remove|revoke)" src/routes/ --type rust -A 20 | \
  rg "is_admin\|is_super_admin\|role\|permission" | wc -l
echo "Admin authorization checks (should match admin endpoint count)"

# Check super-admin token minting requires super-admin
echo "2. Super-admin gating..."
rg "super.?admin.*token|token.*super.?admin" src/ --type rust -B 5 -A 5 | \
  rg "is_super_admin" | wc -l
echo "Super-admin token operations with proper gating"

# Check API key operations verify ownership
echo "3. API key ownership..."
rg "fn.*(create|revoke|list).*key|fn.*key.*(create|revoke|list)" src/routes/ --type rust -A 15 | \
  rg "tenant_id|owner|created_by" | wc -l
echo "API key operations with ownership verification"

# Check that config write/delete requires tenant membership
echo "4. Config mutation authorization..."
rg "fn.*(save|delete|update).*config" src/ --type rust -A 15 | \
  rg "tenant_id|is_admin|membership" | wc -l
echo "Config mutations with authorization"
```

### 5c. Tenant Credential Isolation (Post-Mortem Addition)
**Objective:** Verify OAuth tokens, API keys, and LLM credentials are per-tenant

**Actions:**
```bash
echo "🔍 Tenant Credential Isolation Check..."

# OAuth token storage — must be per-tenant
echo "1. OAuth token tenant scoping..."
rg "INSERT.*oauth|UPDATE.*oauth|SELECT.*oauth" src/ --type rust -A 3 | \
  rg "tenant_id" | wc -l
echo "OAuth queries with tenant_id (must match total OAuth queries)"

# LLM API key storage — must be per-tenant
echo "2. LLM key tenant scoping..."
rg "llm.*setting|ai.*setting|api_key.*setting" src/ --type rust -A 3 | \
  rg "tenant_id" | wc -l
echo "LLM key queries with tenant_id"

# Verify no global credential storage
echo "3. No global credential state..."
rg "static.*Mutex.*Token|static.*RwLock.*Key|LazyLock.*Cred" src/ --type rust -n && \
  echo "❌ Global credential state found!" || \
  echo "✓ No global credential state"
```

### 6. OWASP Top 10 Compliance Check
**Objective:** Check for common web vulnerabilities

**Actions:**
```bash
echo "🌐 OWASP Top 10 Check..."

# A01:2021 - Broken Access Control
echo "1. Access Control: Checking tenant isolation..."
cargo test --test mcp_multitenant_complete_test --quiet

# A02:2021 - Cryptographic Failures
echo "2. Crypto: Checking for weak algorithms..."
rg "md5|sha1[^0-9]|DES|RC4" src/ --type rust -n && echo "❌ Weak crypto!" || echo "✓ Strong crypto"

# A03:2021 - Injection
echo "3. Injection: Checking parameterized queries..."
rg "format!.*SELECT|format!.*INSERT|format!.*UPDATE|format!.*DELETE" src/ --type rust -n && echo "⚠️  SQL injection risk" || echo "✓ Parameterized queries"

# A04:2021 - Insecure Design
echo "4. Design: Checking error handling..."
rg "\.unwrap\(\)|\.expect\(|panic!\(" src/ --type rust -n | grep -v "^src/bin/" | grep -v "// Safe:" && echo "❌ Insecure error handling" || echo "✓ Proper error handling"

# A05:2021 - Security Misconfiguration
echo "5. Config: Checking default credentials..."
rg "admin.*password.*=.*\"admin\"|default.*password" src/ --type rust -n && echo "❌ Default credentials" || echo "✓ No defaults"

# A06:2021 - Vulnerable Components
echo "6. Dependencies: Running cargo audit..."
cargo audit || echo "⚠️  Vulnerable dependencies found"

# A07:2021 - Authentication Failures
echo "7. Auth: Checking password hashing..."
rg "bcrypt|argon2" src/ --type rust -n | head -5

# A08:2021 - Data Integrity Failures
echo "8. Integrity: Checking JWT signature verification..."
rg "decode.*Validation|verify.*signature" src/ --type rust -n | head -5

# A09:2021 - Logging Failures
echo "9. Logging: Checking secret/PII in logs..."
rg "(info!|warn!|error!)\(.*\{.*(access_token|refresh_token|api_key|password|client_secret)" src/ --type rust -n | \
  rg -v "redact|REDACT|mask" && echo "❌ Secrets in logs!" || echo "✓ Log hygiene OK"

# A10:2021 - SSRF
echo "10. SSRF: Checking URL validation..."
rg "reqwest::get|http.*client.*get" src/ --type rust -A 3 | rg "validate.*url|Url::parse" | wc -l

# Additional: XSS/Template Safety
echo "11. XSS: Checking HTML escaping..."
rg "text/html|Content-Type.*html" src/ --type rust -A 10 | \
  rg "format!" | rg -v "html_escape" && echo "⚠️  Unescaped HTML output" || echo "✓ HTML properly escaped"

# Additional: Division Safety
echo "12. Division safety: Checking zero-guards..."
rg " / " src/ --type rust -n | rg "params\.|input\.|request\." | \
  rg -v "\.max\(1\)|checked_div|test" && echo "⚠️  Division without zero-guard" || echo "✓ Division safety OK"
```

### 7. Security Test Execution
**Objective:** Run all security-related tests

**Actions:**
```bash
# Multi-tenant isolation tests
echo "🧪 Running security test suite..."
cargo test --test mcp_multitenant_complete_test -- --nocapture

# Authentication tests
cargo test auth -- --quiet

# Cryptography tests
cargo test crypto -- --quiet

# OAuth tests
cargo test oauth -- --quiet

# Rate limiting tests
cargo test rate_limit -- --quiet

# API key tests
cargo test api_key -- --quiet
```

## Report Generation

After completing all audits, generate a markdown report:

```markdown
# Security Audit Report - Pierre Fitness Platform

**Date:** {current_date}
**Auditor:** Claude Code Security Agent
**Codebase Version:** {git_commit_hash}

## Executive Summary
- ✅ Passed: {count}
- ⚠️  Warnings: {count}
- ❌ Critical Issues: {count}

## Findings

### 1. Tenant Isolation
{findings}

### 2. Cryptography
{findings}

### 3. Input Validation
{findings}

### 4. Secret Management
{findings}

### 5. Authentication & Authorization
{findings}

### 6. OWASP Compliance
{findings}

## Recommendations
{prioritized_list_of_actions}

## Test Results
{test_execution_summary}
```

## Success Criteria

- ✅ All security tests pass
- ✅ Zero hardcoded secrets detected
- ✅ All database queries use parameterized statements
- ✅ JWT uses RS256 asymmetric signing
- ✅ Sensitive data uses `zeroize` for cleanup
- ✅ Multi-tenant tests show no cross-tenant leakage
- ✅ `cargo audit` shows no critical vulnerabilities
- ✅ Rate limiting enforced per tenant
- ✅ PII redaction middleware active

## Usage

Invoke this agent when:
- Before production deployments
- After authentication/authorization changes
- After database schema modifications
- Weekly security audits
- Before security reviews
- After dependency updates

## Dependencies

Required tools:
- `cargo test` - Rust test runner
- `ripgrep` (rg) - Fast code search
- `cargo audit` - Dependency vulnerability scanner
- `git` - Version control

## Notes

This agent follows Pierre's coding standards (docs/coding-standards.md):
- Zero tolerance for unwrap/panic in production
- Structured error types only (no anyhow!)
- Mandatory input validation
- Cryptographic security by default
- Multi-tenant isolation as core requirement
