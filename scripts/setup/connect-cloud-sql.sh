#!/bin/bash
# ABOUTME: Executes SQL against Cloud SQL via a Terraform-managed Cloud Run Job
# ABOUTME: Uses the dravr-mcp-server-sql-client job (postgres:15-alpine with psql)
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
set -euo pipefail

GCP_PROJECT="${GCP_PROJECT:-dravr-dev}"
GCP_REGION="${GCP_REGION:-northamerica-northeast1}"
JOB_NAME="dravr-mcp-server-sql-client"

usage() {
    cat <<EOF
Usage: $0 <sql-query>

Runs a SQL query against Cloud SQL via Cloud Run Job.
The job is Terraform-managed (infra/environments/dev/main.tf).

Examples:
  $0 "SELECT count(*) FROM users"
  $0 "SELECT email, role FROM users LIMIT 10"
  $0 "\\dt+"
  $0 "\\d+ tenants"
EOF
    exit 1
}

[ -z "${1:-}" ] && usage

SQL="$1"

echo "Executing: ${SQL}"
echo "---"

# Pass the SQL query via env var to avoid gcloud arg parsing issues with spaces/commas.
# ^;;^ sets the key-value delimiter to ;; so commas in SQL don't break parsing.
gcloud run jobs update "${JOB_NAME}" \
    --project="${GCP_PROJECT}" \
    --region="${GCP_REGION}" \
    --command='sh,-c,psql -c "$PGQUERY"' \
    --args="" \
    --update-env-vars="^;;^PGQUERY=${SQL}" \
    --quiet \
    2>&1

gcloud run jobs execute "${JOB_NAME}" \
    --project="${GCP_PROJECT}" \
    --region="${GCP_REGION}" \
    --wait \
    2>&1

# Get the latest execution name
EXECUTION=$(gcloud run jobs executions list \
    --job="${JOB_NAME}" \
    --project="${GCP_PROJECT}" \
    --region="${GCP_REGION}" \
    --sort-by="~createTime" \
    --limit=1 \
    --format="value(name)")

# Fetch stdout from Cloud Logging
gcloud logging read \
    "resource.type=cloud_run_job AND resource.labels.job_name=${JOB_NAME} AND labels.\"run.googleapis.com/execution_name\"=$(basename "${EXECUTION}")" \
    --project="${GCP_PROJECT}" \
    --limit=100 \
    --format="value(textPayload)" \
    --freshness=5m \
    2>/dev/null | tac
