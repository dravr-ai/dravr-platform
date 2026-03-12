#!/bin/bash
# ABOUTME: Cloud Run Job entrypoint that constructs DATABASE_URL from Cloud SQL env vars
# ABOUTME: Reuses the same urlencode + connection string logic as entrypoint.sh
set -e

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
if [ -n "$DATABASE_HOST" ] && [ -n "$DATABASE_NAME" ] && [ -n "$DATABASE_USER" ] && [ -n "$DB_PASSWORD" ]; then
    ENCODED_PASSWORD=$(urlencode "$DB_PASSWORD")
    export DATABASE_URL="postgresql://${DATABASE_USER}:${ENCODED_PASSWORD}@localhost/${DATABASE_NAME}?host=${DATABASE_HOST}"
    echo "Constructed DATABASE_URL for Cloud SQL (PostgreSQL via unix socket)"
fi

SEEDER_BINARY="$1"
shift

echo "Running seeder: ${SEEDER_BINARY}"
exec "/app/${SEEDER_BINARY}" "$@"
