#!/usr/bin/env bash
# ABOUTME: Fixture test for pre-push-validate.sh tier selection — one changed file per case,
# ABOUTME: asserting which client tiers the gate selects and, for a Rust-only diff, that it selects none.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Tier selection is the part of the gate nothing else covers: a tier that fails
# to run reports nothing, so the push is green over a diff CI reds. Each case
# builds a throwaway repo with a base commit, applies ONE changed file as HEAD,
# runs the real validator there and asserts on its own output.
#
# The throwaway is what makes this seconds rather than minutes: PROJECT_ROOT
# resolves to it, so the tiers invoke the no-op stubs written below instead of
# the real frontend/mobile/design-system suites. Each stub stands in for a
# script that carries its own coverage; what is under test here is which tiers
# the classification selects, not what those scripts do.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/pre-push-validate.sh}"

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT
OUT="$ROOT/validator.out"
LAST_EXIT=0

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

# Every fixture repo is addressed by path, and `git -C ""` is a documented
# no-op: an unset path silently runs `git add -A` and `git commit` in the
# developer's own repository instead. Nothing here touches a directory that is
# not a throwaway under $ROOT.
require_fixture() {
  case "${1:-}" in
    "$ROOT"/*) [ -d "$1/.git" ] && return 0 ;;
  esac
  echo "❌ not a fixture repo under $ROOT: '${1:-}'" >&2
  exit 1
}

# Every helper the validator invokes, stubbed as a no-op so the tier body it
# guards runs and returns. Tier 1e-move is the one invoked with no existence
# guard, so without its stub a Rust diff aborts there and every later assertion
# passes vacuously. Tier 4 is the exception to the pattern: it calls no script,
# so it is stubbed by its inputs instead — a package test file for its count
# guard and a `bun` on PATH for the two scripts it runs.
STUBS="check-inline-paths.sh architectural-validation.sh check-contremaitre-sync.sh \
check-phantom-surfaces.sh check-turn-envelope.sh check-moved-symbols.sh \
pre-push-frontend-tests.sh design-system-validation.sh pre-push-mobile-tests.sh"

make_repo() {
  local dir
  dir="$(mktemp -d "$ROOT/case-XXXXXX")"
  mkdir -p "$dir/src" "$dir/scripts/ci"
  git -C "$dir" init -q
  git -C "$dir" config user.email test@example.com
  git -C "$dir" config user.name test
  # Tier 0 runs `cargo fmt --all -- --check` on any Rust diff, so the fixture is
  # a well-formed one-crate package whose source is already rustfmt-clean.
  printf '[package]\nname = "probe"\nversion = "0.0.0"\nedition = "2021"\n' >"$dir/Cargo.toml"
  printf 'pub fn probe() {}\n' >"$dir/src/lib.rs"
  local stub
  for stub in $STUBS; do
    printf '#!/bin/sh\nexit 0\n' >"$dir/scripts/ci/$stub"
    chmod +x "$dir/scripts/ci/$stub"
  done
  # Tier 4 counts the shared-package suites first and fails when it finds none,
  # so a packages/ diff needs one to reach the tier body at all. The two scripts
  # it then runs are real commands rather than scripts/ci helpers, so `bun` is
  # stubbed on PATH the way the helpers above are stubbed on disk.
  mkdir -p "$dir/packages/probe/__tests__" "$dir/.stubbin"
  printf 'export {};\n' >"$dir/packages/probe/__tests__/probe.test.ts"
  printf '{"scripts":{"typecheck:packages":"true","test:packages":"true"}}\n' >"$dir/package.json"
  printf '#!/bin/sh\nexit 0\n' >"$dir/.stubbin/bun"
  chmod +x "$dir/.stubbin/bun"
  git -C "$dir" add -A
  git -C "$dir" commit -qm base
  printf '%s\n' "$dir"
}

# Applies one changed file as HEAD and runs the validator in that repo.
# $1 = repo dir, $2 = path inside it, $3 = file contents.
run_change() {
  local dir="$1" path="$2" body="$3" code=0
  require_fixture "$dir"
  mkdir -p "$dir/$(dirname "$path")"
  printf '%s\n' "$body" >"$dir/$path"
  git -C "$dir" add -A
  git -C "$dir" commit -qm "change $path"
  ( cd "$dir" && PATH="$dir/.stubbin:$PATH" "$UNDER_TEST" ) >"$OUT" 2>&1 || code=$?
  LAST_EXIT="$code"
}

dump() { sed 's/^/      /' "$OUT"; }

expect_contains() { # $1 = label, $2 = literal the output must carry
  if grep -qF -- "$2" "$OUT"; then pass "$1"; else
    fail "$1 (output is missing: $2)"
    dump
  fi
}

expect_absent() { # $1 = label, $2 = literal the output must NOT carry
  if grep -qF -- "$2" "$OUT"; then
    fail "$1 (output carries: $2)"
    dump
  else pass "$1"; fi
}

expect_exit() { # $1 = label, $2 = expected exit code
  if [ "$LAST_EXIT" = "$2" ]; then pass "$1"; else
    fail "$1 (exit $LAST_EXIT, expected $2)"
    dump
  fi
}

echo "pre-push-validate.sh tier-selection fixture tests"

# 1. A locale-only diff. Both clients read their whole string catalogue out of
#    packages/i18n, so both client tiers must run even though neither client
#    directory was touched. The SDK does not import it, so Tier 6 must not.
echo "  case 1: packages/i18n/src/locales only"
dir="$(make_repo)"
run_change "$dir" packages/i18n/src/locales/en/translation.json '{"common":{"probe":"probe"}}'
expect_contains "locale diff runs the frontend tier" "Tier 5: Frontend Validation"
expect_contains "locale diff runs the design system tier" "Tier 5b: Design System Validation"
expect_contains "locale diff runs the mobile tier" "Tier 7: Mobile Validation"
expect_absent "locale diff leaves the SDK tier alone" "Tier 6: SDK Validation"
expect_contains "the echo block names the package" "Changed packages: i18n"
expect_contains "the echo block shows why the web tier ran" "Frontend: false (tier runs: true)"
expect_contains "the echo block shows why the mobile tier ran" "Mobile: false (tier runs: true)"
expect_contains "the catalogue flag still reaches Tier 1b" "Tier 1b: Contremaitre Coupling Sync"
expect_contains "the shared-package flag still reaches Tier 1d" "Tier 1d: Turn Envelope Convergence"
expect_exit "locale diff passes" 0

# 2. packages/mcp-types is the SDK's alone — no client imports it, so widening
#    every shared package to every client tier would fail here.
echo "  case 2: packages/mcp-types only"
dir="$(make_repo)"
run_change "$dir" packages/mcp-types/src/tools.ts "export const PROBE_TOOL = 'probe';"
expect_contains "mcp-types diff runs the SDK tier" "Tier 6: SDK Validation"
expect_contains "the echo block shows why the SDK tier ran" "SDK: false (tier runs: true)"
expect_absent "mcp-types diff leaves the frontend tier alone" "Tier 5: Frontend Validation"
expect_absent "mcp-types diff leaves the mobile tier alone" "Tier 7: Mobile Validation"
expect_contains "the mcp-types flag still reaches Tier 1b" "Tier 1b: Contremaitre Coupling Sync"
expect_exit "mcp-types diff passes" 0

# 3. design-system-validation.sh reads packages/shared-constants/src/design-system.ts,
#    so that file changing must run the gate that consumes it.
echo "  case 3: packages/shared-constants only"
dir="$(make_repo)"
run_change "$dir" packages/shared-constants/src/design-system.ts "export const PROBE_TOKEN = '#0f172a';"
expect_contains "shared-constants diff runs the design system tier" "Tier 5b: Design System Validation"
expect_contains "shared-constants diff runs the frontend tier" "Tier 5: Frontend Validation"
expect_contains "the echo block names the package" "Changed packages: shared-constants"
expect_exit "shared-constants diff passes" 0

# 4. api-client feeds both clients, and its own flag drives Tier 1c.
echo "  case 4: packages/api-client only"
dir="$(make_repo)"
run_change "$dir" packages/api-client/src/domains/probe.ts "export const probeEndpoint = '/api/probe';"
expect_contains "api-client diff runs the frontend tier" "Tier 5: Frontend Validation"
expect_contains "api-client diff runs the mobile tier" "Tier 7: Mobile Validation"
expect_contains "the api-client flag still reaches Tier 1c" "Tier 1c: Phantom Surface Detection"
expect_contains "the echo block names the package" "Changed packages: api-client"
expect_exit "api-client diff passes" 0

# 5. A web-only diff stays web-only: the fix follows the dependency edge, it
#    does not make every client tier run for every client diff.
echo "  case 5: frontend/ only"
dir="$(make_repo)"
run_change "$dir" frontend/src/App.tsx "export const App = () => null;"
expect_contains "frontend diff runs the frontend tier" "Tier 5: Frontend Validation"
expect_contains "the echo block reads true for its own directory" "Frontend: true (tier runs: true)"
expect_absent "frontend diff leaves the mobile tier alone" "Tier 7: Mobile Validation"
expect_absent "frontend diff names no changed package" "Changed packages:"
expect_exit "frontend diff passes" 0

# 6. The cost guard. A Rust-only push pays no client tier — this fails the
#    moment anyone makes one of them unconditional.
echo "  case 6: Rust source only"
dir="$(make_repo)"
run_change "$dir" src/lib.rs "$(printf 'pub fn probe() {}\n\npub fn probe_two() {}')"
expect_contains "Rust diff still runs the formatting tier" "Tier 0: Code Formatting"
expect_contains "Rust diff still runs the moved-symbol tier" "Tier 1e-move: moved-symbol importer check"
expect_absent "Rust diff pays no frontend tier" "Tier 5: Frontend Validation"
expect_absent "Rust diff pays no SDK tier" "Tier 6: SDK Validation"
expect_absent "Rust diff pays no mobile tier" "Tier 7: Mobile Validation"
expect_absent "Rust diff names no changed package" "Changed packages:"
expect_exit "Rust diff passes" 0

echo ""
if [ "$failures" -ne 0 ]; then
  echo "❌ $failures tier-selection fixture case(s) failed"
  exit 1
fi
echo "✅ all tier-selection fixture cases passed"
