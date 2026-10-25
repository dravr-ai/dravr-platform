#!/usr/bin/env bash
# ABOUTME: Proves Dravr's web and mobile apps can start sign-in on every origin the server accepts and every run.app origin serving the app
# ABOUTME: Sends the apps' /oauth2/authorize request on each accepted origin, and fails on a serving origin the server would refuse
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists
# ---------------
# The web app (client_id dravr-web) and the mobile app (dravr-mobile) sign in
# through the hosted login page (carnet#787). The server lets them receive a
# code only on the deployment's own origins: FRONTEND_URL, the OAuth issuer
# (OAUTH2_ISSUER_URL, else BASE_URL) and ALLOWED_MOBILE_REDIRECT_ORIGINS
# (terraform local.frontend_extra_origins: frontend_previous_origins, every
# public_domains hostname and the frontend's run.app URL), see
# FirstPartyRedirects in crates/pierre-auth/src/oauth2_server/first_party.rs.
# The web app builds its callback from the page's own origin
# (frontend/src/utils/signIn.ts), so an origin that serves the app but is
# missing from that list answers 400 "Invalid redirect_uri" and the athlete
# cannot sign in.
#
# That is what happened from 2026-10-07 to 2026-10-23: the previous run.app
# frontend origin was refused for 16 days behind green CI, until an athlete
# reported it (carnet#1145). This probe runs right after each deploy
# (publish-images.yml sign-in-smoke-dev, carnet#1152; deploy-gcp.yml
# sign-in-smoke for a manual redeploy) and each dev terraform apply
# (terraform.yml apply-dev), the changes that can refuse an origin.
# It replaced continuous uptime checks, which hit the site every few seconds
# and kept the api and frontend from ever scaling to zero.
#
# Two checks
# ----------
# 1. Coverage: every origin the frontend Cloud Run service is served on (its
#    run.googleapis.com/urls, which includes status.url) must be one the api
#    accepts. An origin that is not fails by name WITHOUT a request: the
#    refusal follows from the config, and a refused request would page Slack
#    through first_party_sign_in_monitoring.tf. Load-balancer hostnames
#    (terraform public_domains) are not visible from Cloud Run; terraform
#    accepts each one by construction, so check 2 probes them.
# 2. Behaviour: on every accepted origin, the request each app sends must be
#    answered 303 to `/oauth2/login?client_id=<client>&…` on the origin it was
#    sent to (the server writes a relative Location). A 400 is the refusal this
#    exists for and is never retried; a transport error or a 5xx is retried a
#    few times, since a deploy can still be settling, then fails too.
#
# The probe writes nothing: an unauthenticated GET /oauth2/authorize reads the
# client row, counts one hit in the caller's own per-address rate-limit window
# (60/min) and redirects. No code, session or grant is created.
#
# Environment
#   SIGN_IN_SMOKE_ORIGINS           accepted origins to probe instead of the live
#                                   api's, space or comma separated; the first one
#                                   also gets the mobile probe (it stands for
#                                   FRONTEND_URL). Setting it skips every gcloud
#                                   call, so no coverage check runs unless
#                                   SIGN_IN_SMOKE_SERVING_ORIGINS is set too
#   SIGN_IN_SMOKE_SERVING_ORIGINS   origins the app is served on, instead of the
#                                   live frontend service's
#   SIGN_IN_SMOKE_ENDPOINT          send every request to this base URL instead of
#                                   to each origin itself, keeping each origin in
#                                   the redirect_uri: a negative test can then name
#                                   an origin the deployment does not serve. Point
#                                   it at a deployed server only knowing that every
#                                   400 it draws pages Slack: the alert in
#                                   first_party_sign_in_monitoring.tf cannot tell a
#                                   probe from an athlete. The negative cases live
#                                   in first-party-sign-in-smoke.test.sh, against a
#                                   stub.
#   SIGN_IN_SMOKE_SERVICE           api service to read (default dravr-mcp-server-api)
#   SIGN_IN_SMOKE_FRONTEND_SERVICE  frontend service to read (default: the api
#                                   service's name with -api replaced by -frontend)
#   SIGN_IN_SMOKE_REGION            their region (default northamerica-northeast1)
#   SIGN_IN_SMOKE_PROJECT           their project (default: gcloud's configured project)
#   SIGN_IN_SMOKE_ATTEMPTS          tries per probe on a transport error or 5xx (default 3)
#   SIGN_IN_SMOKE_RETRY_DELAY       seconds between those tries (default 5)
#
# Usage: scripts/ci/first-party-sign-in-smoke.sh
#        SIGN_IN_SMOKE_ORIGINS="https://app.dravr.ai" scripts/ci/first-party-sign-in-smoke.sh

set -euo pipefail

WEB_CLIENT_ID="dravr-web"
MOBILE_CLIENT_ID="dravr-mobile"
CALLBACK_PATH="/auth/callback"
MOBILE_CALLBACK_URI="dravr://auth/callback"
LOGIN_PATH="/oauth2/login"

SERVICE="${SIGN_IN_SMOKE_SERVICE:-dravr-mcp-server-api}"
FRONTEND_SERVICE="${SIGN_IN_SMOKE_FRONTEND_SERVICE:-${SERVICE%-api}-frontend}"
REGION="${SIGN_IN_SMOKE_REGION:-northamerica-northeast1}"
PROJECT="${SIGN_IN_SMOKE_PROJECT:-}"
ATTEMPTS="${SIGN_IN_SMOKE_ATTEMPTS:-3}"
ENDPOINT="${SIGN_IN_SMOKE_ENDPOINT:-}"
RETRY_DELAY_SECONDS="${SIGN_IN_SMOKE_RETRY_DELAY:-5}"

case "$ATTEMPTS" in
    '' | *[!0-9]* | 0) echo "❌ SIGN_IN_SMOKE_ATTEMPTS must be a positive integer, got '${ATTEMPTS}'" >&2; exit 2 ;;
esac

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# One `scheme://host[:port]` per line, lowercased, default ports dropped,
# duplicates removed in order: the same reduction first_party_web_origins
# applies, so two spellings of one origin are probed once. A value that is not
# an http(s) URL with a host is reported and left out, as the server does.
normalize_origins() {
    python3 -I -c '
import sys
from urllib.parse import urlsplit

seen = []
for raw in sys.stdin.read().replace(",", " ").split():
    try:
        parts = urlsplit(raw.strip())
        port = parts.port
    except ValueError:
        parts, port = None, None
    if parts is None or parts.scheme not in ("http", "https") or not parts.hostname:
        print(f"⚠️  ignoring {raw!r}: not an http(s) origin", file=sys.stderr)
        continue
    default = {"http": 80, "https": 443}[parts.scheme]
    origin = f"{parts.scheme}://{parts.hostname.lower()}"
    if port is not None and port != default:
        origin += f":{port}"
    if origin not in seen:
        seen.append(origin)
print("\n".join(seen))
'
}

# describe <service> <output file>: errexit does not reach into the command
# substitutions that call the readers below, so a gcloud failure is reported
# and returned here rather than surfacing later as a parse error.
describe() {
    local project_args=()
    [ -n "$PROJECT" ] && project_args=(--project "$PROJECT")
    if ! gcloud run services describe "$1" "${project_args[@]+"${project_args[@]}"}" \
        --region "$REGION" --format=json > "$2"; then
        echo "❌ gcloud run services describe $1 failed" >&2
        return 1
    fi
}

# FRONTEND_URL first (the mobile probe uses it), then the issuer, then the
# previous origins: the order first_party_web_origins builds its list in.
# Only these four variables are read out of the service description, so no
# other value (secrets included) is ever printed.
accepted_origins() {
    describe "$SERVICE" "$WORK/api.json" || return 1
    python3 -I - "$WORK/api.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    service = json.load(handle)
containers = service["spec"]["template"]["spec"]["containers"]
env = {}
for container in containers:
    for entry in container.get("env", []):
        if "value" in entry:
            env.setdefault(entry["name"], entry["value"])

def configured(name):
    return env.get(name, "").strip()

origins = [configured("FRONTEND_URL")]
# resolve_issuer_url: OAUTH2_ISSUER_URL, else BASE_URL.
origins.append(configured("OAUTH2_ISSUER_URL") or configured("BASE_URL"))
origins.extend(configured("ALLOWED_MOBILE_REDIRECT_ORIGINS").split(","))
print(" ".join(o for o in origins if o.strip()))
PY
}

# Every URL Cloud Run serves the frontend on: the run.googleapis.com/urls
# annotation (a JSON list inside the JSON) holds both the project-number URL
# and the hash URL that status.url reports.
serving_origins() {
    describe "$FRONTEND_SERVICE" "$WORK/frontend.json" || return 1
    python3 -I - "$WORK/frontend.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    service = json.load(handle)
annotation = service.get("metadata", {}).get("annotations", {}).get("run.googleapis.com/urls", "[]")
urls = list(json.loads(annotation))
status_url = service.get("status", {}).get("url")
if status_url:
    urls.append(status_url)
print(" ".join(urls))
PY
}

CHECK_COVERAGE=0
if [ -n "${SIGN_IN_SMOKE_ORIGINS:-}" ]; then
    SOURCE="SIGN_IN_SMOKE_ORIGINS"
    RAW_ORIGINS="$SIGN_IN_SMOKE_ORIGINS"
else
    SOURCE="live env of Cloud Run service ${SERVICE} (${REGION}${PROJECT:+, project ${PROJECT}})"
    RAW_ORIGINS="$(accepted_origins)" || exit 1
fi

if [ -n "${SIGN_IN_SMOKE_SERVING_ORIGINS:-}" ]; then
    CHECK_COVERAGE=1
    SERVING_SOURCE="SIGN_IN_SMOKE_SERVING_ORIGINS"
    RAW_SERVING="$SIGN_IN_SMOKE_SERVING_ORIGINS"
elif [ -z "${SIGN_IN_SMOKE_ORIGINS:-}" ]; then
    CHECK_COVERAGE=1
    SERVING_SOURCE="URLs of Cloud Run service ${FRONTEND_SERVICE}"
    RAW_SERVING="$(serving_origins)" || exit 1
fi

printf '%s\n' "$RAW_ORIGINS" | normalize_origins > "$WORK/origins"
ORIGIN_COUNT="$(grep -c . "$WORK/origins" || true)"

echo "==== First-party sign-in smoke ===="
echo "  accepted origins from: ${SOURCE}"
[ "$CHECK_COVERAGE" -eq 1 ] && echo "  serving origins from:  ${SERVING_SOURCE}"
[ -n "$ENDPOINT" ] && echo "  requests to:           ${ENDPOINT} (SIGN_IN_SMOKE_ENDPOINT)"

if [ "$ORIGIN_COUNT" -eq 0 ]; then
    echo "❌ No origin to probe: ${SOURCE} named none. A deployment always has FRONTEND_URL or BASE_URL;"
    echo "   an empty list means the lookup failed, not that sign-in works."
    exit 1
fi

FAILURES=()
SERVING_COUNT=0

if [ "$CHECK_COVERAGE" -eq 1 ]; then
    printf '%s\n' "$RAW_SERVING" | normalize_origins > "$WORK/serving"
    SERVING_COUNT="$(grep -c . "$WORK/serving" || true)"
    if [ "$SERVING_COUNT" -eq 0 ]; then
        echo "❌ No serving origin: ${SERVING_SOURCE} named none. A Cloud Run service always has a URL;"
        echo "   an empty list means the lookup failed, not that every origin is covered."
        exit 1
    fi
    while IFS= read -r origin; do
        [ -n "$origin" ] || continue
        if grep -qFx -- "$origin" "$WORK/origins"; then
            echo "✅ ${origin}: served and accepted"
        else
            reason="serves the app but is not an accepted origin, so web sign-in there answers 400 Invalid redirect_uri (not requested: the 400 would page Slack)"
            echo "❌ ${WEB_CLIENT_ID} on ${origin}: ${reason}"
            FAILURES+=("${WEB_CLIENT_ID} on ${origin}: ${reason}")
        fi
    done < "$WORK/serving"
fi

FRONTEND_ORIGIN="$(head -n 1 "$WORK/origins")"

# A PKCE S256 challenge is 43 base64url characters; the server checks only its
# shape before the login redirect, and no code is ever minted for it.
CODE_CHALLENGE="$(python3 -I -c 'import base64, os; print(base64.urlsafe_b64encode(os.urandom(32)).decode().rstrip("="))')"
STATE="sign-in-smoke-$(date +%s)"

urlencode() {
    python3 -I -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$1"
}

# probe <label> <client_id> <redirect_uri> <request base URL>
probe() {
    local label="$1" client="$2" redirect_uri="$3" base="${4%/}"
    local encoded_redirect url
    encoded_redirect="$(urlencode "$redirect_uri")"
    # The query packages/api-client beginSignIn builds: both apps always send
    # prompt=login and a ui_locales.
    url="${base}/oauth2/authorize?response_type=code&client_id=${client}&redirect_uri=${encoded_redirect}&code_challenge=${CODE_CHALLENGE}&code_challenge_method=S256&state=${STATE}&prompt=login&ui_locales=en"
    local expected_prefix="${base}${LOGIN_PATH}?client_id=${client}&"
    local attempt=1 curl_status out code location reason=""

    while :; do
        curl_status=0
        # No -L: the 303 itself is the verdict. %{redirect_url} resolves the
        # relative Location against the request URL.
        out="$(curl -sS --max-time 20 -o "$WORK/body" -w '%{http_code} %{redirect_url}' "$url" 2>"$WORK/curl.err")" || curl_status=$?
        code="${out%% *}"
        location="${out#* }"
        [ "$location" = "$out" ] && location=""

        if [ "$curl_status" -ne 0 ] || [ "$code" = "000" ]; then
            reason="transport error (curl exit ${curl_status}: $(tr '\n' ' ' < "$WORK/curl.err"))"
        elif [ "${code:0:1}" = "5" ]; then
            reason="HTTP ${code} from the server"
        else
            reason=""
            break
        fi
        if [ "$attempt" -ge "$ATTEMPTS" ]; then
            break
        fi
        echo "  … ${label}: ${reason}; retry ${attempt}/$((ATTEMPTS - 1)) in ${RETRY_DELAY_SECONDS}s"
        attempt=$((attempt + 1))
        sleep "$RETRY_DELAY_SECONDS"
    done

    if [ -z "$reason" ]; then
        if [ "$code" = "303" ] && [ "${location#"$expected_prefix"}" != "$location" ]; then
            echo "✅ ${label}: 303 → ${LOGIN_PATH}"
            return 0
        fi
        if [ "$code" = "400" ] && grep -q "Invalid redirect_uri" "$WORK/body"; then
            reason="400 Invalid redirect_uri: the server refuses this origin"
        elif [ "$code" = "303" ]; then
            reason="303 to '${location}', not ${expected_prefix}…"
        else
            reason="HTTP ${code}, expected 303 to ${LOGIN_PATH}"
        fi
    fi
    echo "❌ ${label}: ${reason}"
    FAILURES+=("${label}: ${reason}")
    return 0
}

while IFS= read -r origin; do
    [ -n "$origin" ] || continue
    probe "${WEB_CLIENT_ID} on ${origin}" "$WEB_CLIENT_ID" "${origin}${CALLBACK_PATH}" "${ENDPOINT:-$origin}"
done < "$WORK/origins"

probe "${MOBILE_CLIENT_ID} via ${FRONTEND_ORIGIN}" "$MOBILE_CLIENT_ID" "$MOBILE_CALLBACK_URI" "${ENDPOINT:-$FRONTEND_ORIGIN}"

CHECKS=$((ORIGIN_COUNT + 1 + SERVING_COUNT))
SUMMARY="${ORIGIN_COUNT} web origin(s) + mobile, ${SERVING_COUNT} serving origin(s) checked for coverage"
if [ "${#FAILURES[@]}" -eq 0 ]; then
    echo "==== ${CHECKS}/${CHECKS} sign-in checks passed (${SUMMARY}) ===="
    exit 0
fi

echo "==== ${#FAILURES[@]}/${CHECKS} sign-in checks FAILED (${SUMMARY}) ===="
for failure in "${FAILURES[@]}"; do
    echo "  - ${failure}"
    if [ -n "${GITHUB_ACTIONS:-}" ]; then
        echo "::error title=First-party sign-in refused::${failure}"
    fi
done
cat <<'EOF'
Athletes opening the app on a failing origin cannot sign in. Every origin the
app is served from must be one the server accepts: FRONTEND_URL, the issuer
(OAUTH2_ISSUER_URL, else BASE_URL) or ALLOWED_MOBILE_REDIRECT_ORIGINS, which
terraform sets from frontend_base_url, frontend_previous_origins, every
public_domains hostname and the frontend's own run.app URLs
(infra/environments/dev/main.tf). A 400 on an
origin listed there means the server's FirstPartyRedirects regressed; a
transport error or 5xx means the origin does not route to the api at all. See
carnet#1145 / carnet#1152.
EOF
exit 1
