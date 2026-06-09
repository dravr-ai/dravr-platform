---
name: strict-clippy-check
description: Enforces zero-tolerance code quality policy using Clippy with strict lints, all warnings treated as errors
user-invocable: true
---

# Strict Clippy Check Skill

## Purpose
Enforces Pierre's zero-tolerance code quality policy using Clippy with strict lints. All warnings are treated as errors per CLAUDE.md standards.

## CLAUDE.md Compliance
- ✅ Enforces zero tolerance for `unwrap()`, `expect()`, `panic!()`
- ✅ Validates no `anyhow::anyhow!()` usage
- ✅ Checks for proper error handling patterns
- ✅ Enforces code quality standards

## Usage
Run the **scoped** check during development on the crate you touched:
- After editing a crate, before committing it
- After refactoring
- During code reviews

Do NOT run the full-workspace `--all-targets` clippy locally as a pre-push gate.
Per CLAUDE.md, that ~25-minute run is CI's job (`preflight-clippy` + `clippy`
jobs fire on every push). The local gate is `./scripts/ci/pre-push-validate.sh`.

## Prerequisites
- Rust toolchain installed, including the **pinned 1.94.0** toolchain CI uses
  (`rustup toolchain install 1.94.0`)
- Clippy component (`rustup component add clippy`)

## Commands

### Scoped Per-Crate Check (the dev loop)
```bash
# Validate ONLY the crate you changed, on the pinned toolchain CI uses.
# Fast (seconds–1 min) and closes the feedback loop without the full-workspace ban.
rustup run 1.94.0 cargo clippy -p <crate> --all-targets --all-features -- -D warnings
```

CRITICAL: validate with `rustup run 1.94.0`, NOT ambient `stable` (currently 1.96).
Stable flags lints absent in CI's 1.94 (e.g. `manual_duration`, `map_unwrap_or`
under `-D warnings`), firing in untouched crates as false alarms. The reverse is
safe: 1.96-clean implies 1.94-clean.

NOTE on `nursery`: this workspace runs `clippy::nursery` at `deny` (hotter than
the usual `warn`). Nursery lints are version-specific and may have false
positives by design — the pinned 1.94 run above is the only reliable predictor
of what CI will flag.

### Pre-Push Gate (what actually gates the push)
```bash
# The ONLY local validation command you need before pushing.
# Tier-scoped, non-compiling; writes the .git/validation-passed marker.
./scripts/ci/pre-push-validate.sh
```

### Specific Lint Categories
```bash
# Check only error handling patterns
cargo clippy --all-targets -- \
  -D clippy::unwrap_used \
  -D clippy::expect_used \
  -D clippy::panic

# Check performance lints
cargo clippy --all-targets -- \
  -W clippy::clone_on_copy \
  -W clippy::redundant_clone
```

### Fix Auto-Fixable Issues
```bash
# Apply automatic fixes (use with caution)
cargo clippy --fix --all-targets --allow-dirty -- -D warnings

# Preview fixes without applying
cargo clippy --fix --all-targets --allow-dirty --dry-run -- -D warnings
```

## Linting Configuration

Pierre uses `Cargo.toml` lints configuration (eliminates need for bash script flags):

```toml
[lints.clippy]
all = { level = "deny", priority = -1 }
pedantic = { level = "deny", priority = -1 }
nursery = { level = "deny", priority = -1 }

# Critical error handling
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
```

## Common Issues & Fixes

### Issue: `unwrap()` detected
```rust
// ❌ Bad
let value = some_option.unwrap();

// ✅ Good
let value = some_option.ok_or(AppError::NotFound)?;
```

### Issue: `expect()` detected
```rust
// ❌ Bad
let config = load_config().expect("Failed to load config");

// ✅ Good
let config = load_config()
    .map_err(|e| AppError::ConfigError(e.to_string()))?;
```

### Issue: `panic!()` detected
```rust
// ❌ Bad
if user_id.is_none() {
    panic!("User ID required");
}

// ✅ Good
let user_id = user_id.ok_or(AppError::MissingUserId)?;
```

### Issue: `anyhow::anyhow!()` detected
```rust
// ❌ Bad (CLAUDE.md violation)
return Err(anyhow::anyhow!("Database connection failed"));

// ✅ Good (use structured errors)
return Err(AppError::DatabaseError(
    DatabaseError::ConnectionFailed
));
```

### Issue: Cast warnings
```rust
// ❌ Might truncate
let small = large_value as u8;

// ✅ Safe conversion
let small = u8::try_from(large_value)
    .map_err(|_| AppError::ConversionError)?;
```

### Issue: Missing documentation
```rust
// ❌ No docs
pub fn calculate_vdot(distance: f64, time: f64) -> f64 { }

// ✅ Documented
/// Calculates VDOT (running performance metric) using Daniels' formula
///
/// # Arguments
/// * `distance` - Distance in meters
/// * `time` - Time in seconds
///
/// # Returns
/// VDOT score (typically 20-85)
///
/// # Errors
/// Returns `AlgorithmError` if inputs are invalid
pub fn calculate_vdot(distance: f64, time: f64) -> Result<f64, AlgorithmError> { }
```

## Allowed Exceptions

Per CLAUDE.md, certain patterns are allowed in specific contexts:

### Test Files
```rust
// Allowed in tests/bin with "// Safe:" comment
// Safe: Test setup with known valid values
let config = load_test_config().unwrap();
```

### Binary Files
```rust
// Allowed in src/bin/ with justification
// Safe: CLI application, error printed to stderr
let args = Args::parse().expect("Failed to parse args");
```

## Integration with CI/CD

Clippy runs in GitHub Actions (`.github/workflows/ci-backend.yml`):
```yaml
# preflight-clippy: per-crate clippy on changed leaf crates (~3–5 min)
# clippy:          full-workspace clippy (~10–12 min, gates release-binary)
- name: Clippy
  run: cargo clippy --all-targets --all-features -- -D warnings
```

## Success Criteria
- ✅ Zero Clippy warnings
- ✅ All error handling uses `Result<T, E>`
- ✅ No `unwrap()` in production code (src/)
- ✅ No `panic!()` in production code
- ✅ No `anyhow::anyhow!()` anywhere
- ✅ Public APIs documented

## Pre-Commit Integration

Set the canonical hooks path (from the `.build` submodule — ALWAYS `.build/hooks`,
NEVER a local `.githooks/`):
```bash
git submodule update --init --recursive
git config core.hooksPath .build/hooks
```

The pre-push hook verifies the `.git/validation-passed` marker written by
`./scripts/ci/pre-push-validate.sh`.

## Troubleshooting

**Issue:** Too many warnings, can't fix all at once
```bash
# Focus on critical issues first
cargo clippy --all-targets -- \
  -D clippy::unwrap_used \
  -D clippy::expect_used \
  -D clippy::panic
```

**Issue:** False positive in generated code
```bash
# Suppress in specific context only (must justify)
#[allow(clippy::specific_lint)]  // Justification: generated code
```

**Issue:** Lint not recognized
```bash
# Ensure the pinned toolchain + clippy are installed. Do NOT `rustup update`
# to a newer stable — that reintroduces 1.96-only false alarms (see Commands).
rustup toolchain install 1.94.0
rustup component add clippy --toolchain 1.94.0
```

## Related Files
- `Cargo.toml` - Workspace `[workspace.lints.clippy]` configuration
- `scripts/ci/pre-push-validate.sh` - The local pre-push gate
- `scripts/ci/architectural-validation.sh` - Pattern validation

## Related Skills
- `validate-architecture` - Architectural pattern validation
- `check-no-secrets` - Secret detection
