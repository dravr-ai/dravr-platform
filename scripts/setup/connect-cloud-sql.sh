#!/bin/bash
# ABOUTME: Connects to Cloud SQL via Auth Proxy for local debugging
# ABOUTME: Starts proxy on localhost:5432 and optionally opens psql session
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
set -euo pipefail

# Configuration — matches infra/environments/dev/terraform.tfvars
GCP_PROJECT="${GCP_PROJECT:-dravr-dev}"
GCP_REGION="${GCP_REGION:-northamerica-northeast1}"
INSTANCE_NAME="${INSTANCE_NAME:-dravr-mcp-server-postgres}"
DB_NAME="${DB_NAME:-dravr}"
DB_USER="${DB_USER:-dravr}"
LOCAL_PORT="${LOCAL_PORT:-5432}"

CONNECTION_NAME="${GCP_PROJECT}:${GCP_REGION}:${INSTANCE_NAME}"

usage() {
    cat <<EOF
Usage: $0 [proxy|psql|url]

Connects to Cloud SQL dev instance via Auth Proxy (public IP + SSL enforced).

Commands:
  proxy   Start Cloud SQL Auth Proxy on localhost:${LOCAL_PORT} (default)
  psql    Start proxy in background and open psql session
  url     Print the DATABASE_URL for use with other tools

Environment variables:
  GCP_PROJECT    GCP project ID (default: dravr-dev)
  GCP_REGION     GCP region (default: northamerica-northeast1)
  INSTANCE_NAME  Cloud SQL instance (default: dravr-mcp-server-postgres)
  DB_NAME        Database name (default: dravr)
  DB_USER        Database user (default: dravr)
  LOCAL_PORT     Local port for proxy (default: 5432)
  DB_PASSWORD    Database password (fetched from Secret Manager if not set)

Prerequisites:
  brew install cloud-sql-proxy
  gcloud auth login
  gcloud auth application-default login

Terraform must have database_enable_public_ip = true in dev tfvars.
EOF
    exit 1
}

check_prerequisites() {
    if ! command -v cloud-sql-proxy &>/dev/null; then
        echo "ERROR: cloud-sql-proxy not found"
        echo "Install: brew install cloud-sql-proxy"
        exit 1
    fi

    if ! gcloud auth application-default print-access-token &>/dev/null 2>&1; then
        echo "ERROR: No application default credentials"
        echo "Run: gcloud auth application-default login"
        exit 1
    fi
}

fetch_db_password() {
    if [ -n "${DB_PASSWORD:-}" ]; then
        return
    fi
    echo "Fetching database password from Secret Manager..."
    DB_PASSWORD=$(gcloud secrets versions access latest \
        --secret="dravr-mcp-server-db-password" \
        --project="${GCP_PROJECT}" 2>/dev/null) || {
        echo "ERROR: Could not fetch DB password from Secret Manager"
        echo "Either set DB_PASSWORD env var or ensure you have secretAccessor IAM role"
        exit 1
    }
}

start_proxy() {
    echo "Starting Cloud SQL Auth Proxy..."
    echo "  Instance: ${CONNECTION_NAME}"
    echo "  Local:    localhost:${LOCAL_PORT}"
    echo ""
    echo "Press Ctrl+C to stop"
    cloud-sql-proxy "${CONNECTION_NAME}" \
        --port "${LOCAL_PORT}"
}

start_psql() {
    check_prerequisites
    fetch_db_password

    # Start proxy in background, clean up on exit
    cloud-sql-proxy "${CONNECTION_NAME}" \
        --port "${LOCAL_PORT}" &
    PROXY_PID=$!
    trap 'kill ${PROXY_PID} 2>/dev/null || true' EXIT

    echo "Waiting for proxy to start..."
    for _ in $(seq 1 10); do
        if pg_isready -h 127.0.0.1 -p "${LOCAL_PORT}" &>/dev/null 2>&1; then
            break
        fi
        sleep 1
    done

    echo "Connecting to ${DB_NAME} as ${DB_USER}..."
    PGPASSWORD="${DB_PASSWORD}" psql -h 127.0.0.1 -p "${LOCAL_PORT}" -U "${DB_USER}" -d "${DB_NAME}"
}

print_url() {
    check_prerequisites
    fetch_db_password

    echo "postgresql://${DB_USER}:${DB_PASSWORD}@127.0.0.1:${LOCAL_PORT}/${DB_NAME}"
    echo ""
    echo "Start the proxy first: $0 proxy"
}

COMMAND="${1:-proxy}"

case "${COMMAND}" in
    proxy)
        check_prerequisites
        start_proxy
        ;;
    psql)
        start_psql
        ;;
    url)
        print_url
        ;;
    -h|--help|help)
        usage
        ;;
    *)
        echo "Unknown command: ${COMMAND}"
        usage
        ;;
esac
