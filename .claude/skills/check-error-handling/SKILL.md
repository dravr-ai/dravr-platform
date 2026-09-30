---
name: check-error-handling
description: Validates error handling follows AppResult/AppError pattern, detects anyhow regression, ensures ErrorCode usage
user-invocable: true
---

# Check Error Handling Skill

## Purpose
Quick validation that error handling follows the unified AppResult/AppError pattern (commit b592b5e). Detects anyhow regression and ensures proper use of ErrorCode enum.

## Coding-Standards Compliance
- ✅ Enforces structured error types (no anyhow)
- ✅ Validates ErrorCode usage
- ✅ Prevents error handling regression

## Usage
Run this skill:
- Before committing error handling changes
- Daily pre-commit validation
- After merging branches
- When reviewing error-related PRs

## Prerequisites
- ripgrep (`rg`)

## Commands

### Quick Check (Fast)
```bash
# Check for anyhow regression
echo "🔍 Checking for anyhow usage..."

# 1. Check imports (FORBIDDEN)
if rg "use anyhow::|use anyhow;" src/ --type rust --quiet; then
    echo "❌ FAIL: anyhow imports detected!"
    rg "use anyhow" src/ --type rust -n | head -10
    exit 1
else
    echo "✓ PASS: No anyhow imports"
fi

# 2. Check macro usage (FORBIDDEN)
if rg "anyhow!\(" src/ --type rust --quiet; then
    echo "❌ FAIL: anyhow! macro detected!"
    rg "anyhow!\(" src/ --type rust -n | head -10
    exit 1
else
    echo "✓ PASS: No anyhow! macro"
fi

# 3. Check AppResult usage
APPRESULT_COUNT=$(rg "AppResult<" src/ --type rust | wc -l)
echo "✓ AppResult usage: $APPRESULT_COUNT occurrences"

# 4. Check ErrorCode usage
ERRORCODE_COUNT=$(rg "ErrorCode::" src/ --type rust | wc -l)
echo "✓ ErrorCode usage: $ERRORCODE_COUNT occurrences"

echo ""
echo "✅ Error handling check PASSED"
```

### Comprehensive Check
```bash
#!/bin/bash
set -e

echo "🔍 Comprehensive Error Handling Check"
echo "======================================"

# 1. Anyhow Imports
echo ""
echo "1. Checking for anyhow imports..."
ANYHOW_IMPORTS=$(rg "use anyhow" src/ --type rust | wc -l)
if [ "$ANYHOW_IMPORTS" -gt 0 ]; then
    echo "❌ Found $ANYHOW_IMPORTS anyhow imports:"
    rg "use anyhow" src/ --type rust -n | head -10
    exit 1
else
    echo "✓ No anyhow imports"
fi

# 2. Anyhow Macro
echo ""
echo "2. Checking for anyhow! macro..."
if rg "anyhow!\(" src/ --type rust --quiet; then
    echo "❌ anyhow! macro usage detected:"
    rg "anyhow!\(" src/ --type rust -n
    exit 1
else
    echo "✓ No anyhow! macro"
fi

# 3. Context Method
echo ""
echo "3. Checking for .context() method..."
CONTEXT_COUNT=$(rg "\.context\(\"" src/ --type rust | wc -l)
if [ "$CONTEXT_COUNT" -gt 0 ]; then
    echo "⚠️  Warning: Found $CONTEXT_COUNT .context() usages (prefer .map_err())"
    rg "\.context\(" src/ --type rust -n | head -5
else
    echo "✓ No .context() usage"
fi

# 4. AppResult Usage
echo ""
echo "4. Validating AppResult usage..."
APPRESULT_COUNT=$(rg "AppResult<" src/ --type rust | wc -l)
if [ "$APPRESULT_COUNT" -lt 100 ]; then
    echo "⚠️  Warning: Only $APPRESULT_COUNT AppResult usages (expected >100)"
else
    echo "✓ AppResult usage: $APPRESULT_COUNT"
fi

# 5. ErrorCode Usage
echo ""
echo "5. Validating ErrorCode usage..."
ERRORCODE_COUNT=$(rg "ErrorCode::" src/ --type rust | wc -l)
echo "✓ ErrorCode usage: $ERRORCODE_COUNT"

# Most common error codes
echo ""
echo "Top ErrorCodes:"
rg "ErrorCode::" src/ --type rust -o | sort | uniq -c | sort -rn | head -5

# 6. Error Type Exports
echo ""
echo "6. Checking error module exports..."
if rg "pub use.*AppError|pub use.*AppResult|pub use.*ErrorCode" src/lib.rs --type rust --quiet; then
    echo "✓ Error types exported from lib.rs"
else
    echo "⚠️  Error types may not be exported"
fi

# 7. Cargo.toml Check
echo ""
echo "7. Checking Cargo.toml..."
if rg "^anyhow\s*=" Cargo.toml --quiet; then
    echo "⚠️  anyhow in main dependencies (should be dev-dependencies only)"
else
    echo "✓ anyhow not in main dependencies"
fi

echo ""
echo "✅ Comprehensive error handling check PASSED"
```

## Success Criteria
- ✅ Zero `use anyhow` imports in src/
- ✅ Zero `anyhow!()` macro usage
- ✅ AppResult usage > 100 occurrences
- ✅ ErrorCode usage present
- ✅ Error types exported from lib.rs

## Expected Output (Success)
```
🔍 Checking for anyhow usage...
✓ PASS: No anyhow imports
✓ PASS: No anyhow! macro
✓ AppResult usage: 287 occurrences
✓ ErrorCode usage: 412 occurrences

✅ Error handling check PASSED
```

## Failure Example
```
🔍 Checking for anyhow usage...
❌ FAIL: anyhow imports detected!
src/new_feature.rs:5:use anyhow::{anyhow, Result};
src/new_feature.rs:23:    return Err(anyhow!("Something failed"));
```

## Fixing Violations

### Replace anyhow import
```rust
// ❌ Before
use anyhow::{anyhow, Context, Result};

// ✅ After
use crate::errors::{AppError, AppResult, ErrorCode};
```

### Replace anyhow! macro
```rust
// ❌ Before
return Err(anyhow!("Database connection failed"));

// ✅ After
return Err(AppError::new(
    ErrorCode::DatabaseError,
    "Database connection failed".to_string()
));
```

### Replace .context() method
```rust
// ❌ Before
let user = db.get_user(id).context("Failed to fetch user")?;

// ✅ After
let user = db.get_user(id)
    .map_err(|e| AppError::new(
        ErrorCode::DatabaseError,
        format!("Failed to fetch user {}: {}", id, e)
    ))?;
```

## Related Files
- `src/errors.rs` - ErrorCode and AppError definitions
- `src/lib.rs` - Error type exports
- Commit b592b5e - Error handling migration

## Related Agents
- `error-handling-guardian` - Comprehensive error validation

## Quick Reference

```bash
# One-line check
rg "use anyhow|anyhow!\(" src/ --type rust && echo "FAIL" || echo "PASS"

# Check specific file
rg "use anyhow" src/new_feature.rs --type rust

# Count error patterns
echo "AppResult: $(rg 'AppResult<' src/ --type rust | wc -l)"
echo "ErrorCode: $(rg 'ErrorCode::' src/ --type rust | wc -l)"
```
