#!/usr/bin/env bash
# ABOUTME: Drives the real TypeScript SDK against a deployed MCP endpoint after deploy
# ABOUTME: Proves routing, auth and the shipped binary work together, which unit CI cannot
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists
# ---------------
# The SDK's own suites run against localhost, and the MCP compliance validator
# boots a local server — so "the SDK works" and "the SDK works against what we
# ship" were never the same claim. On 2026-08-04 all of the following were true
# at once, with every CI workflow green:
#
#   * POST /mcp returned nginx's 405 because /mcp was missing from the frontend
#     proxy alternation, so MCP over HTTP was unreachable for every user;
#   * POST /oauth2/register returned 405 for the same reason, breaking RFC 7591
#     client registration;
#   * every API key failed authentication in deployed environments, because the
#     PostgreSQL get_by_prefix query matched `id` instead of `key_prefix`.
#
# Each was found by hand. This runs the actual published SDK bundle against the
# actual deployed URL with an actual credential, which is the only configuration
# in which those three failures are visible.
#
# Usage: PIERRE_SERVER_URL=... PIERRE_JWT_TOKEN=... scripts/ci/mcp-sdk-smoke.sh

set -uo pipefail

: "${PIERRE_SERVER_URL:?PIERRE_SERVER_URL must be set (deployed base URL)}"
: "${PIERRE_JWT_TOKEN:?PIERRE_JWT_TOKEN must be set (API key or JWT)}"

# An unauthenticated caller legitimately sees only connect_to_dravr, so a
# result of 1 tool means the credential was rejected — the exact shape the
# PostgreSQL API-key bug produced. Anything well below the real catalogue
# (~94 tools) means auth silently degraded rather than failed loudly.
MIN_TOOLS="${MCP_SMOKE_MIN_TOOLS:-50}"

SDK_CLI="${SDK_CLI:-sdk/dist/cli.js}"
OUT="$(mktemp)"
ERR="$(mktemp)"
trap 'rm -f "$OUT" "$ERR"' EXIT

echo "==== MCP SDK smoke test ===="
echo "  target: $PIERRE_SERVER_URL"
echo "  sdk:    $SDK_CLI"

if [ ! -f "$SDK_CLI" ]; then
    echo "❌ SDK bundle not found at $SDK_CLI — build it first (cd sdk && bun run build)."
    exit 1
fi

printf '%s\n%s\n%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"ci-smoke","version":"1.0.0"}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_connection_status","arguments":{}}}' \
  | timeout 180 node "$SDK_CLI" > "$OUT" 2> "$ERR"

python3 - "$OUT" "$ERR" "$MIN_TOOLS" <<'PY'
import json
import sys

out_path, err_path, min_tools = sys.argv[1], sys.argv[2], int(sys.argv[3])
responses = {}
for line in open(out_path):
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except json.JSONDecodeError:
        # Non-JSON on stdout means the transport returned something that is not
        # JSON-RPC at all — an nginx error page, or the SPA shell.
        print(f"  raw non-JSON line: {line[:120]}")
        continue
    if "id" in msg:
        responses[msg["id"]] = msg

failures = []

init = responses.get(1)
if not init or "result" not in init:
    failures.append(f"initialize did not return a result: {init}")
else:
    server = init["result"].get("serverInfo", {}).get("name")
    print(f"  initialize : {server} / {init['result'].get('protocolVersion')}")
    if not server:
        failures.append("initialize returned no serverInfo.name")

listed = responses.get(2)
if not listed or "result" not in listed:
    failures.append(f"tools/list did not return a result: {listed}")
else:
    tools = listed["result"].get("tools", [])
    print(f"  tools/list : {len(tools)} tools")
    if len(tools) < min_tools:
        names = ", ".join(t.get("name", "?") for t in tools[:5])
        failures.append(
            f"tools/list returned {len(tools)} tools (expected >= {min_tools}): [{names}]. "
            "One tool means the credential was rejected and only the public "
            "discovery tool is visible — check the API key, not the tool registry."
        )

called = responses.get(3)
if not called or "result" not in called:
    failures.append(f"tools/call did not return a result: {called}")
else:
    content = json.dumps(called["result"])[:160]
    print(f"  tools/call : {content}")
    if called["result"].get("isError"):
        failures.append(f"tools/call reported isError: {content}")

if failures:
    print("")
    for failure in failures:
        print(f"❌ {failure}")
    print("")
    print("The deployed artifact does not serve MCP end to end. Likely causes, in")
    print("the order they have actually occurred:")
    print("  1. a backend path missing from the frontend nginx proxy alternation")
    print("     (returns the SPA shell with a 200 — run scripts/ci/nginx-routing-check.sh)")
    print("  2. the smoke credential expired or was revoked (rotate DEV_MCP_SMOKE_KEY)")
    print("  3. an auth regression that rejects valid credentials")
    print("")
    print("SDK stderr tail:")
    print("".join(open(err_path).readlines()[-15:]))
    raise SystemExit(1)

print("")
print("✅ MCP SDK smoke test passed against the deployed endpoint")
PY
