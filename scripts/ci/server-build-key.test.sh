#!/usr/bin/env bash
# ABOUTME: Fixture test for server-build-key.sh — proves the key moves with every compile input and with the
# ABOUTME: variant, stays put for files the build never reads, and refuses when it cannot read HEAD.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A key that misses an input hands the e2e lanes a binary built from other
# sources and reports green on code nobody tested. Each case commits one change
# in a throwaway repo and asserts whether the key changed.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/server-build-key.sh}"

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT
cd "$ROOT"
git init -q
git config user.email test@example.com
git config user.name test
git config commit.gpgsign false

failures=0
check() { # $1 = label, $2 = "same" or "differ", $3 = before, $4 = after
  if { [ "$2" = same ] && [ "$3" = "$4" ]; } || { [ "$2" = differ ] && [ "$3" != "$4" ]; }; then
    echo "  ✅ $1"
  else
    echo "  ❌ $1 — expected keys to $2: '$3' vs '$4'"
    failures=$((failures + 1))
  fi
}
put() { mkdir -p "$(dirname "$1")"; printf '%s\n' "$2" >"$1"; git add -A; git commit -qm "$1"; }
key() { "$UNDER_TEST" linux-release "$@"; }

echo "=== exit codes"
code=0; "$UNDER_TEST" linux-release >/dev/null 2>&1 || code=$?
check "a repo with no commit exits 3" same "$code" 3
code=0; "$UNDER_TEST" >/dev/null 2>&1 || code=$?
check "a missing variant exits 2" same "$code" 2

put crates/a/src/lib.rs 'fn a() {}'
put Cargo.lock 'v1'
put frontend/src/App.tsx 'app'
put .github/workflows/lane.yml 'lane v1'

echo "=== shape"
k="$(key)"
if [[ "$k" =~ ^[0-9a-f]{24}$ ]]; then echo "  ✅ 24 hex digits"; else echo "  ❌ key '$k' is not 24 hex digits"; failures=$((failures + 1)); fi
check "the same HEAD gives the same key" same "$k" "$(key)"

echo "=== inputs move the key"
for change in crates/a/src/lib.rs:'fn a() { 1 }' Cargo.lock:v2 packages/i18n/fr.json:'{}' migrations/0001.sql:'select 1' \
              sdk/src/index.ts:'x' .cargo/config.toml:'[build]' packages/shared-constants/x.json:'1' commands/c.md:'c' \
              Cargo.toml:'[workspace]' rust-toolchain.toml:'[toolchain]' migrations_pg/0001.sql:'select 1' \
              packages/mcp-types/t.ts:'t' frontend/src/utils/tokenLink.ts:'link'; do
  before="$(key)"; put "${change%%:*}" "${change#*:}"
  check "${change%%:*}" differ "$before" "$(key)"
done

echo "=== files the build never reads leave it"
before="$(key)"; put frontend/src/App.tsx 'app v2'
check "frontend/src/App.tsx" same "$before" "$(key)"
before="$(key)"; put README.md 'readme'
check "README.md" same "$before" "$(key)"

echo "=== deleting an input moves it"
before="$(key)"; git rm -q packages/i18n/fr.json; git commit -qm rm
check "packages/i18n removed" differ "$before" "$(key)"

echo "=== variant and extra paths"
check "another variant" differ "$(key)" "$("$UNDER_TEST" macos-release)"
before="$(key .github/workflows/lane.yml)"; put .github/workflows/lane.yml 'lane v2'
check "a change to the named workflow" differ "$before" "$(key .github/workflows/lane.yml)"

if [ "$failures" -gt 0 ]; then
  echo "server-build-key.test.sh: ${failures} failure(s)"
  exit 1
fi
echo "server-build-key.test.sh: all passed"
