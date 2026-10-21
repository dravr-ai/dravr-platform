#!/usr/bin/env bash
# ABOUTME: Fixture test for cache-budget.py — a saved cache listing, and the report and warnings it must produce
# ABOUTME: Pins the family grouping, the missing-family, over-quota and retired-toolchain warnings, and the empty-read refusal
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/cache-budget.py}"
WORK="$(mktemp -d)"
OUT="$WORK/out"
trap 'rm -rf "$WORK"' EXIT

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

mkdir -p "$WORK/root"
printf '[toolchain]\nchannel = "1.99.1"\n' > "$WORK/root/rust-toolchain.toml"

entry() { # $1 = key, $2 = bytes
  printf '{"key": "%s", "size_in_bytes": %s, "last_accessed_at": "2026-10-21T12:00:00Z"}\n' "$1" "$2"
}

run_budget() { # $1 = listing file; prints the exit status, output lands in $OUT
  local code=0
  env -u GITHUB_STEP_SUMMARY python3 "$UNDER_TEST" --from-json "$1" --repo-root "$WORK/root" >"$OUT" 2>&1 || code=$?
  echo "$code"
}

expect() { # $1 = label, $2 = actual, $3 = expected
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (got '$2', expected '$3')"
    sed 's/^/      /' "$OUT"
  fi
}

has() { # $1 = label, $2 = fixed string the output must contain
  if grep -qF -- "$2" "$OUT"; then pass "$1"; else
    fail "$1"
    sed 's/^/      /' "$OUT"
  fi
}

lacks() { # $1 = label, $2 = fixed string the output must not contain
  if grep -qF -- "$2" "$OUT"; then
    fail "$1"
    sed 's/^/      /' "$OUT"
  else pass "$1"; fi
}

echo "a healthy listing"
{
  entry "v0-rust-postgres-tests-Linux-x64-9c8f1cf9-07f63762" 1000000000
  entry "v0-rust-test-dev-Linux-x64-9c8f1cf9-07f63762" 930000000
  entry "v0-rust-clippy-Linux-x64-9c8f1cf9-07f63762" 530000000
  entry "v0-rust-leaf-crate-tests-Linux-x64-bd2e8885-07f63762" 660000000
  entry "v0-rust-leaf-crate-tests-Linux-x64-9c8f1cf9-07f63762" 660000000
  entry "v0-rust-release-Linux-x64-9c8f1cf9-07f63762" 1200000000
  entry "Linux-cargo-mobile-server-1.99.1-c90e54d62b6be34f7b830383bbb976fb9e4990b8bb0a38a0040b0f8077a28b56" 2190000000
  entry "macOS-cargo-mobile-server-1.99.1-c90e54d62b6be34f7b830383bbb976fb9e4990b8bb0a38a0040b0f8077a28b56" 2410000000
  entry "bun-hZqfnqaunDYMPOAzj2drjzVpoAE=" 30000000
  entry "bun-ZjPV4hcBdmnYV0w/gFvDzb9RLeo=" 10000000
} > "$WORK/healthy.jsonl"
expect "it reports and exits 0" "$(run_budget "$WORK/healthy.jsonl")" 0
has "the total is summed over every entry" "9.62 of 10.00 GB in 10 entries."
has "two leaf-crate entries are one family" '| `rust:leaf-crate-tests` | 4 | 2 | 1.32 |'
has "a mobile-server entry is its OS's family" '| `mobile-server:macOS` | 7 | 1 | 2.41 |'
has "bun's base64-keyed entries are one family" '| `bun` |  | 2 | 0.04 |'
lacks "no warning when every priority family is present and under quota" "::warning"

echo "a listing missing families, over quota, with a retired toolchain"
{
  entry "v0-rust-clippy-Linux-x64-9c8f1cf9-07f63762" 530000000
  entry "Linux-cargo-mobile-server-1.99.1-aaaaaaaa" 5000000000
  entry "macOS-cargo-mobile-server-1.99.1-aaaaaaaa" 4000000000
  entry "Linux-cargo-mobile-server-1.98.1-4e2d949430237b961d5d56b7d08baf407fd3c2a7a01b25bb390c95274dfb2073" 1120000000
} > "$WORK/unhealthy.jsonl"
expect "findings are reported, never failed on" "$(run_budget "$WORK/unhealthy.jsonl")" 0
has "a missing priority family warns by name" "cache family rust:postgres-tests has no entry"
has "the release family's absence warns" "cache family rust:release has no entry"
lacks "a present family does not warn" "cache family rust:clippy has no entry"
has "a total over the quota warns" "10.65 GB, over the 10.00 GB quota"
has "an entry for a retired toolchain warns" "is for Rust 1.98.1, not 1.99.1"
has "the missing families are listed in the summary" 'Missing: `rust:postgres-tests`'

echo "keys from another runner OS and from another tool"
{
  entry "v0-rust-clippy-Darwin-arm64-9c8f1cf9-07f63762" 100000000
  entry "setup-python-Linux-x64-24.04-Ubuntu-python-3.11.9-pip-0123456789abcdef" 50000000
} > "$WORK/others.jsonl"
expect "it reports and exits 0" "$(run_budget "$WORK/others.jsonl")" 0
has "a macOS rust-cache entry (os.type() Darwin) is its shared key's family" '| `rust:clippy` | 3 | 1 | 0.10 |'
lacks "a version in a non-cargo key is not a retired Rust toolchain" "is for Rust 3.11.9"

echo "a read that verified nothing"
: > "$WORK/empty.jsonl"
expect "an empty listing is refused" "$(run_budget "$WORK/empty.jsonl")" 2
has "it says why" "returned no cache entry"
expect "an unreadable listing is an error" "$(run_budget "$WORK/missing.jsonl")" 1

echo
if [ "$failures" -eq 0 ]; then
  echo "all cache-budget cases pass"
  exit 0
fi
echo "$failures case(s) failed"
exit 1
