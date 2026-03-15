#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Mirrors GCP Secret Manager secrets to GitHub repository secrets for disaster recovery
# ABOUTME: Secrets flow through pipes only — never written to disk, state files, or logs

set -euo pipefail

PROJECT_ID="${GCP_PROJECT:-dravr-dev}"
REPO="dravr-ai/dravr-platform"

# All GCP secrets to mirror to GitHub
SECRETS=(
  "dravr-mcp-server-db-password"
  "dravr-mcp-server-encryption-key"
  "dravr-mcp-server-strava-client-id"
  "dravr-mcp-server-strava-client-secret"
  "dravr-mcp-server-gemini-api-key"
  "dravr-mcp-server-usda-api-key"
  "dravr-mcp-server-openweather-api-key"
  "dravr-mcp-server-resend-api-key"
  "dravr-mcp-server-admin-password"
  "dravr-mcp-server-copilot-github-token"
  "google-oauth-client-id"
  "google-oauth-client-secret"
)

echo "Syncing ${#SECRETS[@]} secrets from GCP (${PROJECT_ID}) → GitHub (${REPO})"
echo ""

SYNCED=0
FAILED=0

for SECRET_NAME in "${SECRETS[@]}"; do
  # Convert GCP name to GitHub format: dravr-mcp-server-db-password → DRAVR_MCP_SERVER_DB_PASSWORD
  GH_NAME=$(echo "$SECRET_NAME" | tr '[:lower:]' '[:upper:]' | tr '-' '_')

  # Read from GCP and pipe directly to GitHub — secret never touches disk
  if gcloud secrets versions access latest --secret="$SECRET_NAME" --project="$PROJECT_ID" 2>/dev/null \
    | gh secret set "$GH_NAME" --repo "$REPO" 2>/dev/null; then
    echo "  ✅ ${SECRET_NAME} → ${GH_NAME}"
    ((SYNCED++))
  else
    echo "  ❌ ${SECRET_NAME} (not found or access denied)"
    ((FAILED++))
  fi
done

echo ""
echo "Done: ${SYNCED} synced, ${FAILED} failed"

if [ "$FAILED" -gt 0 ]; then
  echo "⚠️  Some secrets failed — check GCP Secret Manager access"
  exit 1
fi
