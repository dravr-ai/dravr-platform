#!/usr/bin/env bash
# ABOUTME: Pins check-workflow-test-targets.sh — the carnet#474 catch, package scoping, autotests, and no-false-positive cases
# ABOUTME: Builds a throwaway two-package workspace and workflow directory, so it runs in under a second with no deps
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-workflow-test-targets.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

# Two packages. pkg_a has a plain tests/<name>.rs and a tests/<dir>/main.rs
# target; pkg_b declares one [[test]] whose name differs from its file, and
# owns a target pkg_a does not.
scaffold() {
    rm -rf "$WORK/probe"
    mkdir -p "$WORK/probe/.github/workflows" \
        "$WORK/probe/crates/a/tests/suite" "$WORK/probe/crates/b/tests"
    printf '[workspace]\nresolver = "2"\nmembers = ["crates/*"]\n' > "$WORK/probe/Cargo.toml"
    printf '[package]\nname = "pkg_a"\nversion = "0.0.0"\nedition = "2021"\n' \
        > "$WORK/probe/crates/a/Cargo.toml"
    printf '[package]\nname = "pkg_b"\nversion = "0.0.0"\nedition = "2021"\n\n[[test]]\nname = "declared_test"\npath = "tests/custom.rs"\n' \
        > "$WORK/probe/crates/b/Cargo.toml"
    : > "$WORK/probe/crates/a/tests/a_test.rs"
    : > "$WORK/probe/crates/a/tests/suite/main.rs"
    : > "$WORK/probe/crates/b/tests/custom.rs"
    : > "$WORK/probe/crates/b/tests/b_only_test.rs"
}

# $1 = workflow file name, $2 = the step's run body (indented by the caller).
workflow() {
    printf 'name: probe\non: push\njobs:\n  probe:\n    runs-on: ubuntu-latest\n    steps:\n      - run: |\n%s\n' "$2" \
        > "$WORK/probe/.github/workflows/$1"
}

run_check() {
    "$CHECK" "$WORK/probe" 2>&1
}

echo "check-workflow-test-targets.sh"

# 1. Catch: the carnet#474 shape — a dead name on a continuation line of a
#    multi-target command, next to live ones.
scaffold
workflow live.yml "$(printf '          cargo test --test a_test \\\n            --test moved_out_test \\\n            --test suite \\\n            -- --test-threads=1 --nocapture')"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "UNRESOLVED .github/workflows/live.yml:8 --test moved_out_test (no package has it)"; then
    pass "fails on a dead target on a continuation line, and names file, line and target"
else
    fail "did not catch the dead target (status $STATUS): $OUT"
fi

# 1b. A comment line ends where bash ends it, backslash or not: it must not
#     swallow the command on the next line into itself.
scaffold
workflow swallow.yml "$(printf '          # the next target moved to another repo \\\n          cargo test --test moved_out_test')"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "UNRESOLVED .github/workflows/swallow.yml:9 --test moved_out_test"; then
    pass "a comment ending in a backslash does not hide the command after it"
else
    fail "a trailing-backslash comment swallowed the next command (status $STATUS): $OUT"
fi

# 2. Clean: a plain file target, a tests/<dir>/main.rs target, and a [[test]]
#    name whose file is named something else all resolve.
scaffold
workflow clean.yml "$(printf '          cargo test --test a_test --test suite\n          cargo test -p pkg_b --test=declared_test -- --nocapture')"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -eq 0 ] && echo "$OUT" | grep -q "static --test references: 3"; then
    pass "passes when every target resolves, counting all three"
else
    fail "false positive on resolving targets (status $STATUS): $OUT"
fi

# 3. Package scope: cargo searches only the -p selection, so a target that
#    exists in another package is still a dead flag on this command.
scaffold
workflow scoped.yml '          cargo test -p pkg_a --test b_only_test'
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "b_only_test (it is in pkg_b, outside the -p selection)"; then
    pass "fails on a target outside the command's -p selection"
else
    fail "did not honour -p scoping (status $STATUS): $OUT"
fi

# 3b. A -p naming a package the workspace dropped fails the same way cargo does.
scaffold
workflow gone_pkg.yml '          cargo test -p pkg_gone --test a_test'
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "a_test (package pkg_gone is not in the workspace)"; then
    pass "fails on a -p package the workspace does not have"
else
    fail "did not catch a dropped -p package (status $STATUS): $OUT"
fi

# 4. autotests = false: a tests/*.rs file is not a target unless declared.
scaffold
printf '[package]\nname = "pkg_a"\nversion = "0.0.0"\nedition = "2021"\nautotests = false\n' \
    > "$WORK/probe/crates/a/Cargo.toml"
workflow autotests.yml '          cargo test -p pkg_a --test a_test'
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "UNRESOLVED .* --test a_test"; then
    pass "does not count an undeclared file under autotests = false"
else
    fail "treated an autotests = false file as a target (status $STATUS): $OUT"
fi

# 5. Renamed [[test]]: the file's own stem is no longer a target name.
scaffold
workflow stem.yml '          cargo test -p pkg_b --test custom'
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "UNRESOLVED .* --test custom"; then
    pass "resolves a [[test]] by its declared name, not its file stem"
else
    fail "accepted a file stem a [[test]] entry renamed (status $STATUS): $OUT"
fi

# 6. No false positives: a comment naming a dead target, a runtime-built name,
#    an echo that quotes a cargo command, and test-binary args after `--`.
scaffold
workflow quiet.yml "$(printf '          # cargo test --test retired_test used to run here\n          echo "cargo test --test retired_test"\n          cargo test --test "$%s" -- --test-threads=1\n          cargo test --workspace --test b_only_test -- --test gone' 'target')"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -eq 0 ] && echo "$OUT" | grep -q "runtime-built --test references (not checkable): 1"; then
    pass "ignores comments, quoted prose and binary args; counts a runtime-built name without failing"
else
    fail "false positive on non-cargo --test text (status $STATUS): $OUT"
fi

# 7. Fail closed: workflows that name no test target have verified nothing.
scaffold
workflow empty.yml '          cargo build --workspace'
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "SCAN INCOMPLETE"; then
    pass "fails closed when the scan found no --test reference"
else
    fail "reported a pass on a scan that verified nothing (status $STATUS)"
fi

# 8. Fail closed: a workspace with no test targets cannot resolve anything.
scaffold
rm -rf "$WORK/probe/crates/a/tests" "$WORK/probe/crates/b/tests"
printf '[package]\nname = "pkg_b"\nversion = "0.0.0"\nedition = "2021"\n' > "$WORK/probe/crates/b/Cargo.toml"
workflow live.yml '          cargo test --test a_test'
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "SCAN INCOMPLETE"; then
    pass "fails closed when the workspace declares no test targets"
else
    fail "reported a pass with no test targets to resolve against (status $STATUS)"
fi

echo
if [ "$FAILURES" -gt 0 ]; then
    echo "❌ $FAILURES case(s) failed"
    exit 1
fi
echo "✅ all cases passed"
exit 0
