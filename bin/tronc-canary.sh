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
#                           (default: https://dravr-mcp-server-api-865150413606.northamerica-northeast1.run.app)
#   DRAVR_CANARY_TOKEN      Admin JWT with ViewConfiguration permission (required)

MODE="${1:---prod}"

case "${MODE}" in
    --prod)
        BASE_URL="${DRAVR_CANARY_BASE_URL:-https://dravr-mcp-server-api-865150413606.northamerica-northeast1.run.app}"
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
    echo "       mint one with: cargo run --bin pierre-cli -- token generate --service tronc-canary --super-admin" >&2
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
