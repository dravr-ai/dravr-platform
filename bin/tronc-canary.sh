#!/usr/bin/env bash
# ABOUTME: Manually trigger the Slack alert pipeline health check endpoint
# ABOUTME: Verifies dravr-tronc is forwarding ERROR events to #dev-dravr-errors

set -euo pipefail

# Emits a synthetic ERROR event through the running pierre-server so that
# dravr-tronc's ErrorNotificationLayer forwards it to Slack and email. The
# operator then verifies the canary message landed in #dev-dravr-errors.
# If it did not land, the alerting pipeline is broken BEFORE the next real
# outage surfaces the gap.
#
# Usage:
#   bin/tronc-canary.sh [--prod|--local]
#
# Environment:
#   DRAVR_CANARY_BASE_URL   Override the pierre-server base URL
#                           (default: the FRONTEND service — see below)
#   DRAVR_CANARY_TOKEN      Admin JWT with ViewConfiguration permission (required)
#
# The default is the frontend service, not the api service. The api service is
# deployed INGRESS_TRAFFIC_INTERNAL_ONLY, so the api URL this script used to
# default to is unreachable from a laptop or a CI runner and produced a
# connection failure rather than a canary. nginx proxies /admin/ through to the
# backend (its location alternation lists `admin`), so the frontend host serves
# these endpoints.

MODE="${1:---prod}"

case "${MODE}" in
    --prod)
        BASE_URL="${DRAVR_CANARY_BASE_URL:-https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app}"
        ;;
    --local)
        BASE_URL="${DRAVR_CANARY_BASE_URL:-http://127.0.0.1:8081}"
        ;;
    *)
        echo "Usage: $0 [--prod|--local]" >&2
        exit 2
        ;;
esac

if [[ -z "${DRAVR_CANARY_TOKEN:-}" ]]; then
    echo "error: DRAVR_CANARY_TOKEN must be set to an admin JWT with ViewConfiguration permission" >&2
    echo "       Mint it FROM the deployment you are pointing at, not locally:" >&2
    echo "         cargo run --bin pierre-cli -- auth login --server ${BASE_URL}" >&2
    echo "       approve in the browser as a super-admin, then read access_token from ~/.pierre/credentials.json." >&2
    echo "       'token generate' will NOT work here: it signs with the local database's RSA keypair and" >&2
    echo "       registers the token in the local admin_tokens table, so a deployment rejects it on both counts." >&2
    exit 1
fi

ENDPOINT="${BASE_URL}/admin/diagnostics/tronc-canary"
echo "POST ${ENDPOINT}"

RESPONSE=$(curl -sS -X POST \
    -H "Authorization: Bearer ${DRAVR_CANARY_TOKEN}" \
    -H "Content-Type: application/json" \
    -w "\nHTTP %{http_code}" \
    "${ENDPOINT}")

echo "${RESPONSE}"

if echo "${RESPONSE}" | grep -q "HTTP 200"; then
    CORRELATION_ID=$(echo "${RESPONSE}" | grep -o '"correlation_id":"[^"]*"' | cut -d'"' -f4 || echo "(not found)")
    echo ""
    echo "✓ Canary emitted. Correlation ID: ${CORRELATION_ID}"
    echo "  Verify the event lands in #dev-dravr-errors within ~10s (tronc batches every 5s)."
    echo "  If it does not land, the alerting pipeline is broken — investigate dravr-tronc wiring."
else
    echo ""
    echo "✗ Canary emission failed — check the HTTP status above."
    exit 1
fi
