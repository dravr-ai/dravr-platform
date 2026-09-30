---
name: check-input-validation
description: Validates input domain safety including division-by-zero prevention, pagination bounds, cache key completeness, and numeric range enforcement
user-invocable: true
---

# Input Validation Check Skill

## Purpose
Validates that user-supplied inputs are properly bounded and validated before use. Catches division-by-zero, unbounded pagination, missing cache key components, and numeric range violations.

## Coding-Standards Compliance
- ✅ Enforces Security Engineering Rules: Input Domain Validation
- ✅ Prevents divide-by-zero panics from user input
- ✅ Validates pagination bounds per docs/coding-standards.md
- ✅ Checks cache key completeness for tenant isolation

## Usage
Run this skill:
- After adding new math operations on user input
- After adding new API endpoints with pagination
- After modifying cache key construction
- After adding recipe/nutrition calculations
- Before committing code that processes numeric user input

## Commands

### Division Safety Check
```bash
echo "========================================="
echo "  INPUT VALIDATION CHECK"
echo "========================================="

echo ""
echo "--- 1. Division Safety ---"

# Find all division operations in production code
echo "🔍 Scanning for division operations..."
rg " / " src/ --type rust -n | \
  rg -v "test|//|mod |use |impl " | \
  rg -v "\.len\(\) / |as f64 / |as f32 / " > /tmp/divisions.txt

# Check each for guards
echo "Division operations found:"
cat /tmp/divisions.txt | head -20

# Check for .max(1) or similar guards near divisions
echo ""
echo "🔍 Checking for zero-guards near divisions..."
rg "\.max\(1\)|\.max\(1\.0\)|checked_div|if.*==.*0|if.*>.*0" src/ --type rust -n | wc -l
echo "Zero-guard patterns found"

# Specific high-risk patterns: recipe servings, pagination
echo ""
echo "🔍 Checking recipe/nutrition division safety..."
rg "servings|portion|per_serving" src/ --type rust -A 3 | \
  rg " / " | \
  rg -v "\.max\(1\)|checked_div" && \
  echo "⚠️  Division by servings without zero-guard!" || \
  echo "✓ Servings division properly guarded"
```

### Pagination Bounds Check
```bash
echo ""
echo "--- 2. Pagination Bounds ---"

# Check for limit/offset parameters
echo "🔍 Checking pagination parameter bounds..."
rg "struct.*Params|struct.*Request|struct.*Query" src/ --type rust -A 15 | \
  rg "limit|offset|page|per_page" | head -10

# Verify clamp/min/max on pagination values
echo ""
echo "🔍 Checking bound enforcement..."
rg "limit.*clamp|limit.*min|limit.*max|\.min\(.*100\)|\.max\(.*1\)" src/ --type rust -n | head -10
echo "Pagination bound patterns found"

# Check for unbounded LIMIT in SQL
echo ""
echo "🔍 Checking SQL LIMIT bounds..."
rg "LIMIT \\\$|LIMIT \{" src/ --type rust -B 3 | \
  rg -v "clamp|min|max" | head -10
echo "(Above should be empty — all LIMIT values need bounds)"
```

### Cache Key Completeness
```bash
echo ""
echo "--- 3. Cache Key Completeness ---"

# Find cache key construction
echo "🔍 Checking cache keys include tenant_id..."
rg "cache_key|format!.*cache|format!.*key" src/ --type rust -n | \
  rg -v "tenant" | \
  rg -v "test|//|use " | head -10
echo "(Above should be empty — all cache keys need tenant_id)"

# Verify cache operations use tenant-scoped keys
rg "cache\.get|cache\.set|cache\.insert|cache\.remove" src/ --type rust -B 5 | \
  rg "tenant" | wc -l
echo "Cache operations with tenant context"
```

### Numeric Range Validation
```bash
echo ""
echo "--- 4. Numeric Range Enforcement ---"

# Check for numeric parameters used without validation
echo "🔍 Checking numeric input validation..."
rg "params\.\w+.*as (f64|f32|i64|i32|u64|u32)" src/ --type rust -n | head -10
echo "(Review above — numeric casts from params need range checks)"

# Check for weight/height/age without bounds
rg "weight|height|age|heart_rate|pace" src/routes/ --type rust -A 5 | \
  rg "params\." | rg -v "validate|clamp|min|max|range" | head -5
echo "(Above should be empty — fitness metrics need bounds)"

echo ""
echo "========================================="
echo "  INPUT VALIDATION CHECK COMPLETE"
echo "========================================="
```

## High-Risk Code Paths

These modules handle user numeric input and need special attention:
- `src/intelligence/` — Training load, VDOT, nutrition calculations
- `src/mcp/tool_handlers/` — MCP tool parameters from external clients
- `src/routes/` — API endpoint query/body parameters
- `src/providers/` — Provider-specific data transformations

## Success Criteria
- ✅ All division operations guarded against zero divisors
- ✅ Pagination limit clamped to sane range (e.g., 1..=100)
- ✅ Pagination offset validated as non-negative
- ✅ Cache keys include tenant_id component
- ✅ Numeric user inputs validated against domain ranges
- ✅ Recipe servings/portions guarded before division
- ✅ No unbounded LIMIT in SQL queries

## Standalone Script
The CI-runnable version of this skill lives at `scripts/ci/check-input-validation.sh`.
It runs automatically in CI via `./scripts/ci/architectural-validation.sh --apply-skills`.

## Related Skills
- `security-review` — Comprehensive security checklist
- `validate-architecture` — Architectural patterns
- `test-intelligence-algorithms` — Algorithm correctness
