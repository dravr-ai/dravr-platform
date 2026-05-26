#!/bin/bash
# ABOUTME: Cloud Run Job entrypoint that constructs DATABASE_URL from Cloud SQL env vars
# ABOUTME: Clones dravr-contremaitre when seeding coaches so the binary gets a fresh content snapshot
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

VERB="$1"
shift

# The drift-check verb is identical to the seed verb in setup terms: it also
# needs a fresh contremaitre clone so the binary can compute file hashes for
# the same set of `prompts/coaches/<category>/<slug>/en.md` files. The
# wrapping `if` below clones for either entry point and rewrites the
# command we exec at the end.
case "$VERB" in
    seed|check-drift)
        DOMAIN="$1"
        shift
        ;;
    *)
        echo "Unknown verb '$VERB' (expected: seed | check-drift)" >&2
        exit 64
        ;;
esac

# Coach definitions live in the dravr-contremaitre repo as the single source
# of truth. Clone a shallow snapshot to a temp dir and point the binary at
# its prompts/coaches subtree. CONTREMAITRE_REPO / CONTREMAITRE_BRANCH /
# CONTREMAITRE_GITHUB_PAT are the same env vars used by the runtime
# hot-reload client, so this seed step shares one auth surface.
if [ "$DOMAIN" = "coaches" ]; then
    : "${CONTREMAITRE_REPO:?CONTREMAITRE_REPO is required to ${VERB} coaches}"
    : "${CONTREMAITRE_GITHUB_PAT:?CONTREMAITRE_GITHUB_PAT is required to ${VERB} coaches}"
    BRANCH="${CONTREMAITRE_BRANCH:-main}"
    CLONE_DIR=$(mktemp -d -t contremaitre.XXXXXX)
    trap 'rm -rf "$CLONE_DIR"' EXIT
    echo "Cloning ${CONTREMAITRE_REPO}@${BRANCH} for coach ${VERB}"
    git clone \
        --depth 1 \
        --branch "$BRANCH" \
        "https://x-access-token:${CONTREMAITRE_GITHUB_PAT}@github.com/${CONTREMAITRE_REPO}.git" \
        "$CLONE_DIR"
    export PIERRE_COACHES_DIR="${CLONE_DIR}/prompts/coaches"
fi

echo "Running: pierre-cli ${VERB} ${DOMAIN}"
exec /app/pierre-cli "$VERB" "$DOMAIN" "$@"
