#!/bin/bash
# ABOUTME: Reports one user's end-to-end onboarding progress against a deployed environment
# ABOUTME: Account and allow-list state come from pierre-cli over HTTP; step/pillar internals from Cloud SQL

# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -euo pipefail

EMAIL="${1:-}"
if [ -z "$EMAIL" ]; then
    cat >&2 <<EOF
Usage: $0 <email> [--deep]

Reports a user's onboarding progress. Without --deep this is HTTP-only via
pierre-cli and returns in seconds. --deep adds the onboarding step rows, pillar
coverage, provider connections and conversational-walk state, which live only in
the database and cost one Cloud Run Job round trip (~60-90s).

Requires a cached admin login: pierre-cli auth login --server <frontend-url>
EOF
    exit 1
fi
DEEP="${2:-}"

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "${DIR}/../.." && pwd)"

# Prefer a release build, fall back to debug, then to cargo. Keeps the script
# usable straight after a plain `cargo build --bin pierre-cli`.
if [ -x "${REPO}/target/release/pierre-cli" ]; then
    CLI=("${REPO}/target/release/pierre-cli")
elif [ -x "${REPO}/target/debug/pierre-cli" ]; then
    CLI=("${REPO}/target/debug/pierre-cli")
else
    CLI=(cargo run --quiet --manifest-path "${REPO}/Cargo.toml" --bin pierre-cli --)
fi

# pierre-cli logs to stderr; keep stdout clean for the JSON payloads.
echo "== ALLOW-LIST =="
"${CLI[@]}" user list-allowed 2>/dev/null | grep -iF -e "EMAIL " -e "${EMAIL}" || echo "  ${EMAIL} is not pre-approved"

echo
echo "== ACCOUNT =="
if ACCOUNT=$("${CLI[@]}" user get "${EMAIL}" --format json 2>/dev/null) && [ -n "${ACCOUNT}" ]; then
    printf '%s\n' "${ACCOUNT}" | python3 -c '
import json, sys
payload = json.load(sys.stdin)
user = payload.get("data") or {}
if not user:
    print("  no account yet — has not registered")
else:
    for field in ("email", "status", "tier", "display_name", "is_admin", "created_at", "last_active"):
        print(f"  {field:13} {user.get(field)}")
' 2>/dev/null || echo "  no account yet — has not registered"
else
    echo "  no account yet — has not registered"
fi

if [ "${DEEP}" != "--deep" ]; then
    echo
    echo "Run with --deep for onboarding steps, pillar coverage and provider connections."
    exit 0
fi

# The onboarding internals have no admin API — GET /api/me/onboarding-status is
# self-only — so they are read straight from the database. One statement, no
# backslash meta-commands and no newlines: connect-cloud-sql.sh ships this through
# a gcloud env var and runs it under `psql -c`, both of which flatten the query.
# Every join casts BOTH sides to text: user_onboarding / user_facts /
# provider_connections store user_id as TEXT while chat_conversations uses UUID.
SAFE_EMAIL=${EMAIL//\'/\'\'}
SQL="SELECT section, detail FROM ("
SQL+="SELECT '1-STEP' AS section, o.step_id || ' | ' || o.status || ' | channel=' || coalesce(o.chosen_channel,'-') || ' | ' || o.updated_at::text AS detail FROM user_onboarding o JOIN users u ON u.id::text = o.user_id::text WHERE lower(u.email) = lower('${SAFE_EMAIL}')"
SQL+=" UNION ALL SELECT '2-PILLAR', coalesce(f.pillar,'(untagged)') || ' | kind=' || f.kind || ' | source=' || f.source || ' | facts=' || count(*)::text FROM user_facts f JOIN users u ON u.id::text = f.user_id::text WHERE lower(u.email) = lower('${SAFE_EMAIL}') GROUP BY f.pillar, f.kind, f.source"
SQL+=" UNION ALL SELECT '3-COVERAGE', 'topics_covered=' || count(DISTINCT coalesce(f.pillar, CASE WHEN f.kind = 'north_star' THEN 'north_star' END))::text || ' of 7' FROM user_facts f JOIN users u ON u.id::text = f.user_id::text WHERE lower(u.email) = lower('${SAFE_EMAIL}')"
SQL+=" UNION ALL SELECT '4-PROVIDER', p.provider || ' | ' || p.connection_type || ' | ' || p.connected_at::text FROM provider_connections p JOIN users u ON u.id::text = p.user_id::text WHERE lower(u.email) = lower('${SAFE_EMAIL}')"
SQL+=" UNION ALL SELECT '5-WALK', 'conversation ' || c.id::text || ' | walk_active=' || (c.onboarding_state IS NOT NULL)::text || ' | updated=' || c.updated_at::text FROM chat_conversations c JOIN users u ON u.id::text = c.user_id::text WHERE lower(u.email) = lower('${SAFE_EMAIL}')"
SQL+=") t ORDER BY section, detail;"

echo
echo "== ONBOARDING INTERNALS (database) =="
"${DIR}/connect-cloud-sql.sh" "$SQL" 2>&1 \
    | grep -vE "Updating|^Done\.|To execute|gcloud run|Creating exec|Provisioning|Starting exec|Running exec|View details|Or visit|successfully|^Executing:|^---$"
