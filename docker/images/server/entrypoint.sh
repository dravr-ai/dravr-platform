#!/bin/bash
# ABOUTME: Docker entrypoint for Pierre MCP Server
# ABOUTME: Optionally sources .envrc for local dev overrides, then executes the server binary
set -e

# If .envrc exists, source it for environment variables
if [ -f "/app/.envrc" ]; then
    echo "Loading environment variables from .envrc..."
    # Remove 'export ' prefix and source the variables
    while IFS= read -r line; do
        # Skip empty lines and comments
        [[ -z "$line" || "$line" =~ ^[[:space:]]*# ]] && continue
        
        if [[ $line =~ ^export[[:space:]]+([^=]+)=(.+)$ ]]; then
            var_name="${BASH_REMATCH[1]}"
            var_value="${BASH_REMATCH[2]}"
            # Remove quotes if present
            var_value=$(echo "$var_value" | sed 's/^"//;s/"$//')
            export "$var_name"="$var_value"
        fi
    done < /app/.envrc
    echo "Environment variables loaded successfully"
fi

# URL-encode a string (handles special chars like %$@! in passwords)
urlencode() {
    local string="$1" i c
    for (( i = 0; i < ${#string}; i++ )); do
        c="${string:$i:1}"
        case "$c" in
            [a-zA-Z0-9.~_-]) printf '%s' "$c" ;;
            *) printf '%%%02X' "'$c" ;;
        esac
    done
}

# Construct DATABASE_URL from Cloud SQL components when deployed on Cloud Run
# Cloud Run injects DATABASE_HOST, DATABASE_NAME, DATABASE_USER as plain env vars
# and DB_PASSWORD from Secret Manager. Assemble into a single connection string.
if [ -n "$DATABASE_HOST" ] && [ -n "$DATABASE_NAME" ] && [ -n "$DATABASE_USER" ] && [ -n "$DB_PASSWORD" ]; then
    ENCODED_PASSWORD=$(urlencode "$DB_PASSWORD")
    export DATABASE_URL="postgresql://${DATABASE_USER}:${ENCODED_PASSWORD}@localhost/${DATABASE_NAME}?host=${DATABASE_HOST}"
    echo "Constructed DATABASE_URL for Cloud SQL (PostgreSQL via unix socket)"
fi

# Pin the GitHub Copilot CLI model for the headless ACP chat provider.
#
# `copilot --acp` selects its model from ~/.copilot/settings.json "model" — it
# IGNORES both the ACP session/new "model" field and the CLI --model flag (those
# only set a cosmetic label). With no settings.json the CLI falls through to the
# account's experiment default (GPT/auto), so chat silently runs GPT/Gemini
# instead of the configured model. Write the file from PIERRE_LLM_MODEL (the
# unified chat model; COPILOT_HEADLESS_MODEL is the vision-only fallback) so the
# ACP session routes to the intended model. Non-fatal: a degraded model must
# never block server startup.
#
# This is a deployment stopgap: the durable fix lives in dravr-embacle, which
# will manage the copilot model config and log the real routing model. Remove
# this block once that embacle release is deployed.
if [ "${PIERRE_LLM_PROVIDER:-}" = "copilot_headless" ] || [ "${PIERRE_LLM_PROVIDER:-}" = "copilot" ]; then
    COPILOT_PIN_MODEL="${PIERRE_LLM_MODEL:-${COPILOT_HEADLESS_MODEL:-claude-sonnet-4.6}}"
    COPILOT_SETTINGS="${HOME}/.copilot/settings.json"
    {
        mkdir -p "${HOME}/.copilot" \
            && printf '{\n  "model": "%s"\n}\n' "$COPILOT_PIN_MODEL" > "$COPILOT_SETTINGS" \
            && echo "Pinned Copilot ACP model to '${COPILOT_PIN_MODEL}' (${COPILOT_SETTINGS})"
    } || echo "WARNING: failed to pin Copilot ACP model; chat may run the account default model"
fi

# Create data directory if it doesn't exist
mkdir -p /app/data

echo "Starting Pierre MCP Server..."
echo "🚀 Multi-tenant MCP server starting on ports $MCP_PORT (MCP) and $HTTP_PORT (HTTP)"

# Debug information
echo "Binary path: $1"
echo "Binary exists: $(test -f "$1" && echo "YES" || echo "NO")"
echo "Binary executable: $(test -x "$1" && echo "YES" || echo "NO")"
echo "Current user: $(whoami)"
echo "Current directory: $(pwd)"
echo "Starting Pierre MCP Server binary..."

# Execute the server directly
exec "$@"