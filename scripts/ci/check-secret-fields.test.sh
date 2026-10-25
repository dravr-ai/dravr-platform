#!/usr/bin/env bash
# ABOUTME: Fixture test for check-secret-fields.py — builds throwaway crates and asserts each verdict
# ABOUTME: Pins that a String secret in a Debug type fails and a SecretString, descriptive name or hand Debug passes
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-secret-fields.py}"
OUT="$(mktemp)"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

# Every fixture carries one SecretString field, so the scan has something it
# verified and a clean tree exits 0 rather than 2.
BASELINE='#[derive(Debug)]
pub struct Baseline {
    pub key: SecretString,
}'

tree_with() {
  local dir
  dir="$(mktemp -d)"
  mkdir -p "$dir/crates/demo/src"
  printf '%s\n\n%s\n' "$BASELINE" "$1" >"$dir/crates/demo/src/lib.rs"
  printf '%s\n' "$dir"
}

run_gate() {
  local code=0
  python3 "$UNDER_TEST" "$1" >"$OUT" 2>&1 || code=$?
  echo "$code"
}

expect() { # $1 = label, $2 = actual exit, $3 = expected exit
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, expected $3)"
    sed 's/^/      /' "$OUT"
  fi
}

must_fail() { # $1 = label, $2 = source, $3 = text the report must name
  local dir
  dir="$(tree_with "$2")"
  expect "$1" "$(run_gate "$dir")" 1
  if grep -qF "$3" "$OUT"; then pass "$1 names the field"; else
    fail "$1 (report lacks: $3)"
    sed 's/^/      /' "$OUT"
  fi
  rm -rf "$dir"
}

must_pass() { # $1 = label, $2 = source
  local dir
  dir="$(tree_with "$2")"
  expect "$1" "$(run_gate "$dir")" 0
  rm -rf "$dir"
}

echo "check-secret-fields.py"

must_fail "a String password in a Debug struct" '#[derive(Debug, Clone, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}' "lib.rs:9: LoginRequest.password: String"

must_fail "an optional token behind docs and a multi-line derive" '#[derive(
    Debug,
    Serialize,
)]
/// A token response
pub struct TokenResponse {
    /// The access token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}' "TokenResponse.refresh_token: Option<String>"

must_fail "an enum variant holding a client secret" '#[derive(Debug, Clone)]
pub enum Client {
    Server {
        client_id: String,
        client_secret: String,
    },
}' "Client.client_secret: String"

must_fail "an api key named by its parts" '#[derive(Debug)]
struct Upstream {
    openweather_api_key: String,
}' "Upstream.openweather_api_key: String"

must_fail "a connection string that embeds its password" '#[derive(Debug, Default)]
pub struct CacheConfig {
    pub redis_url: Option<String>,
}' "CacheConfig.redis_url: Option<String>"

must_fail "an OAuth code and PKCE verifier" '#[derive(Debug)]
pub struct TokenRequest {
    pub code: Option<String>,
    pub code_verifier: String,
}' "TokenRequest.code: Option<String>"

must_fail "borrowed and shared text holding a credential" '#[derive(Debug)]
struct Refresh<'"'"'a> {
    client_secret: &'"'"'a str,
    user_pat: Arc<str>,
}' "Refresh.client_secret: &'a str"

must_fail "a device-flow user code as bytes" '#[derive(Debug)]
pub struct Device {
    pub user_code: Vec<u8>,
}' "Device.user_code: Vec<u8>"

must_pass "an error code named for what it is" '#[derive(Debug)]
pub struct ApiError {
    pub error_code: String,
    pub status_code: String,
}'

must_pass "a SecretString secret" '#[derive(Debug)]
pub struct LoginRequest {
    pub password: SecretString,
    pub refresh_token: Option<SecretString>,
}'

must_pass "names that describe a secret without holding one" '#[derive(Debug)]
pub struct Endpoints {
    pub token_url: String,
    pub token_type: String,
    pub api_key_id: String,
    pub password_hash: String,
    pub next_page_token: Option<String>,
    pub secret_length: String,
}'

must_pass "a type with a hand-written Debug" 'pub struct Masked {
    pub password: String,
}
impl fmt::Debug for Masked {
    fn fmt(&self, f: &mut fmt::Formatter<'"'"'_>) -> fmt::Result { f.write_str("Masked") }
}'

must_pass "a non-Debug type and a tuple struct" 'pub struct Plain {
    pub password: String,
}
#[derive(Debug)]
pub struct Wrapper(String);'

# A tree with no SecretString field and no Debug type verified nothing.
dir="$(mktemp -d)"
mkdir -p "$dir/crates/demo/src"
printf 'pub fn f() {}\n' >"$dir/crates/demo/src/lib.rs"
expect "an empty scan fails closed" "$(run_gate "$dir")" 2
rm -rf "$dir"

rm -f "$OUT"
if [ "$failures" -gt 0 ]; then
  echo "❌ $failures check-secret-fields.py case(s) failed"
  exit 1
fi
echo "✅ check-secret-fields.py: every case holds"
