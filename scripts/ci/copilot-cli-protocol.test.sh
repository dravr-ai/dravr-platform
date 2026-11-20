#!/usr/bin/env bash
# ABOUTME: Fixture test for copilot-cli-protocol.sh — replaces the npm and crate probes with fixed answers
# ABOUTME: Pins that `target` proposes the newest accepted CLI, never a rejected one, and that both protocol parsers fail closed
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=copilot-cli-protocol.sh
source "$SCRIPT_DIR/copilot-cli-protocol.sh"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

# The probes the script would run against npm and crates.io, as fixtures.
VERSIONS=""
PROTOCOLS=""
RANGE=""
PROBED=""
npm_versions() { printf '%s\n' $VERSIONS | sort -V; }
sdk_range() { printf '%s\n' "$RANGE"; }
cli_protocol() {
  PROBED="${PROBED:+$PROBED }$1"
  local pair
  for pair in $PROTOCOLS; do
    [ "${pair%%=*}" = "$1" ] && { printf '%s\n' "${pair#*=}"; return 0; }
  done
  echo "no fixture protocol for $1" >&2
  return 1
}

field() { printf '%s\n' "$1" | sed -n "s/^$2=//p"; }

echo "copilot-cli-protocol.sh target"

VERSIONS="1.0.103 1.0.111 1.0.112 1.0.113 1.0.114"
PROTOCOLS="1.0.103=3 1.0.111=3 1.0.112=3 1.0.113=4 1.0.114=4"
RANGE="3 3"
out="$(target 1.0.103 1.0.26)"
[ "$(field "$out" target)" = "1.0.112" ] && pass "latest on a newer protocol: proposes the newest accepted CLI" \
  || fail "expected target=1.0.112, got: $out"
[ "$(field "$out" rejected)" = "1.0.114=4 1.0.113=4" ] && pass "names each rejected release with its protocol" \
  || fail "expected rejected='1.0.114=4 1.0.113=4', got: $out"
[ "$(field "$out" latest)" = "1.0.114" ] && [ "$(field "$out" latest_protocol)" = "4" ] \
  && pass "reports npm latest and its protocol" || fail "latest fields wrong: $out"

RANGE="3 4"
PROBED=""
out="$(target 1.0.103 1.0.40)"
[ "$(field "$out" target)" = "1.0.114" ] && pass "a crate that accepts the newer protocol lets latest through" \
  || fail "expected target=1.0.114, got: $out"
[ -z "$(field "$out" rejected)" ] && pass "nothing rejected when the crate accepts every release" \
  || fail "expected no rejection, got: $out"

RANGE="3 3"
VERSIONS="1.0.103 1.0.113 1.0.114"
PROTOCOLS="1.0.103=3 1.0.113=4 1.0.114=4"
out="$(target 1.0.103 1.0.26)"
[ "$(field "$out" target)" = "1.0.103" ] && pass "every newer release rejected: stays on the pin" \
  || fail "expected target=1.0.103, got: $out"

VERSIONS="1.0.103"
PROTOCOLS="1.0.103=3"
out="$(target 1.0.103 1.0.26)"
[ "$(field "$out" target)" = "1.0.103" ] && [ "$(field "$out" latest_protocol)" = "3" ] \
  && pass "pin already latest: target is the pin" || fail "expected target=1.0.103, got: $out"

VERSIONS="1.0.103 1.0.113"
PROTOCOLS="1.0.103=3 1.0.113=4"
code=0
err="$(target 1.0.113 1.0.26 2>&1 >/dev/null)" || code=$?
[ "$code" -eq 1 ] && pass "a pin on a rejected protocol fails" || fail "expected exit 1 for a rejected pin, got $code"
printf '%s\n' "$err" | grep -q '::error::the pinned Copilot CLI 1.0.113 speaks protocol 4' \
  && pass "the failure names the pin and its protocol" || fail "unexpected message: $err"

VERSIONS="1.0.112 1.0.113"
PROTOCOLS="1.0.112=3 1.0.113=bogus"
code=0
target 1.0.112 1.0.26 >/dev/null 2>&1 || code=$?
[ "$code" -ne 0 ] && pass "a non-numeric protocol answer fails closed" || fail "a bogus protocol passed"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "copilot-cli-protocol.sh protocol_range_in"

# The crate's own layout: SDK_PROTOCOL_VERSION is public in its own module,
# MIN_PROTOCOL_VERSION private in lib.rs.
mkdir -p "$TMP/sdk/src"
printf 'pub const SDK_PROTOCOL_VERSION: u32 = 4;\n' > "$TMP/sdk/src/sdk_protocol_version.rs"
printf 'const MIN_PROTOCOL_VERSION: u32 = 3;\n' > "$TMP/sdk/src/lib.rs"
range="$(protocol_range_in "$TMP/sdk/src")"
[ "$range" = "3 4" ] && pass "reads min and max from the crate source" || fail "expected '3 4', got '$range'"

rm "$TMP/sdk/src/lib.rs"
code=0
protocol_range_in "$TMP/sdk/src" >/dev/null 2>&1 || code=$?
[ "$code" -ne 0 ] && pass "a source without MIN_PROTOCOL_VERSION fails closed" || fail "a half range passed"

echo "copilot-cli-protocol.sh runtime_protocol"

# A stand-in runtime that writes a notification frame before its ping reply,
# the framing the real runtime uses (Content-Length header, blank line, body).
cat > "$TMP/runtime" <<'EOF'
#!/usr/bin/env bash
cat >/dev/null
note='{"jsonrpc":"2.0","method":"log","params":{"message":"starting"}}'
reply='{"jsonrpc":"2.0","id":1,"result":{"message":"pong","protocolVersion":4}}'
printf 'Content-Length: %d\r\n\r\n%s' "${#note}" "$note"
printf 'Content-Length: %d\r\n\r\n%s' "${#reply}" "$reply"
EOF
chmod +x "$TMP/runtime"
p="$(runtime_protocol "$TMP/runtime")"
[ "$p" = "4" ] && pass "reads the ping reply past a preceding notification" || fail "expected 4, got '$p'"

printf '#!/usr/bin/env bash\ncat >/dev/null\n' > "$TMP/runtime"
code=0
runtime_protocol "$TMP/runtime" >/dev/null 2>&1 || code=$?
[ "$code" -ne 0 ] && pass "a runtime that never replies fails closed" || fail "a silent runtime passed"

if [ "$failures" -ne 0 ]; then
  echo "❌ $failures case(s) failed"
  exit 1
fi
echo "✅ all cases passed"
