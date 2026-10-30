#!/usr/bin/env bash
# ABOUTME: Fixture test for fleet-duplication.sh — local git remotes stand in for the satellites
# ABOUTME: Pins that every stanza's repo and tronc are scanned once, and that a failed clone fails the run
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Needs a built pierre-sechoir (SECHOIR, or target/release/pierre-sechoir).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
UNDER_TEST="$SCRIPT_DIR/fleet-duplication.sh"
export SECHOIR="${SECHOIR:-$PROJECT_ROOT/target/release/pierre-sechoir}"
[ -x "$SECHOIR" ] || { echo "❌ build pierre-sechoir first: cargo build --release -p pierre-sechoir"; exit 2; }

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# A function long enough to clear pierre-sechoir's default size floor.
loader() {
  cat <<RS
pub fn $1(path: &str) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = trimmed.split(',').collect();
        if parts.len() < 3 {
            return Err(format!("bad line {}", line));
        }
        out.push(parts[2].to_uppercase());
        out.push(parts[1].trim().to_string());
    }
    Ok(out)
}
RS
}

# A git remote named $1 holding src/lib.rs = $2.
remote() {
  local dir="$TMP/remotes/$1"
  mkdir -p "$dir/src"
  printf '%s\n' "$2" >"$dir/src/lib.rs"
  git -C "$dir" init -q
  git -C "$dir" -c user.email=t@example.com -c user.name=t add -A
  git -C "$dir" -c user.email=t@example.com -c user.name=t commit -qm init
}

# The platform side: a satellites.toml whose two stanzas share one repository.
PLATFORM="$TMP/platform"
mkdir -p "$PLATFORM/crates/demo/src"
loader platform_loader >"$PLATFORM/crates/demo/src/lib.rs"
cat >"$PLATFORM/satellites.toml" <<'TOML'
[gates]
backend = "ci-backend.yml"

[dravr-alpha]
repo   = "dravr-alpha"
source = "git-tag"

[alpha-twin]
repo     = "dravr-alpha"
source   = "git-tag"
upstream = "dravr-alpha"
TOML
remote dravr-alpha "$(loader alpha_loader)"
remote dravr-tronc "$(loader tronc_loader)"

export SATELLITE_PIN_ROOT="$PLATFORM" FLEET_GIT_BASE="file://$TMP/remotes"

echo "fleet-duplication.sh"
if out="$("$UNDER_TEST" "$TMP/work" 2>"$TMP/err")"; then
  pass "a complete fleet scans"
  case "$out" in *"3 files, 3 functions compared"*) pass "platform, alpha and tronc are each scanned once" ;;
    *) fail "expected 3 files and 3 functions: $(head -1 <<<"$out")" ;; esac
  case "$out" in *"alpha ↔ platform"*) pass "the cross-repo copy is reported under short labels" ;;
    *) fail "alpha ↔ platform missing from: $out" ;; esac
else
  fail "a complete fleet failed: $(cat "$TMP/err")"
fi

rm -rf "$TMP/remotes/dravr-tronc" "$TMP/work"
if "$UNDER_TEST" "$TMP/work" >/dev/null 2>"$TMP/err"; then
  fail "a missing repository must fail the run"
else
  if grep -q "cannot clone dravr-tronc" "$TMP/err"; then
    pass "a failed clone fails the run and names the repo"
  else
    fail "the failure does not name the repo: $(cat "$TMP/err")"
  fi
fi

[ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }
echo "all passed"
