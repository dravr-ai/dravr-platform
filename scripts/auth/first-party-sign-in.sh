#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Signs a user in headlessly as Dravr's own app: hosted login page, authorization code, PKCE
# ABOUTME: Prints the /oauth/token JSON (access_token, user, csrf_token, ...) on stdout; never echoes secrets

# The password grant is gone from /oauth/token (carnet#787, RFC 9700 §2.4), so
# a script signs in the way the apps do:
#
#   1. POST /oauth2/login        the hosted form: credentials + the authorization
#                                request → 302 to /oauth2/authorize + session cookie
#   2. GET  /oauth2/authorize    with that cookie → 302 to redirect_uri?code=…&state=…
#                                (first-party clients skip the consent screen)
#   3. POST /oauth/token         grant_type=authorization_code + the PKCE verifier
#
# Usage:
#   first-party-sign-in.sh <base_url> <email> <password|-> [client_id] [scope]
#
#   password   "-" reads it from stdin (keeps it out of the process list)
#   client_id  dravr-mobile (default; redirect_uri dravr://auth/callback, which
#              every deployment accepts) or dravr-web (redirect_uri
#              $FRONTEND_URL/auth/callback, else <base_url origin>/auth/callback —
#              it must match the server's FRONTEND_URL or issuer origin)
#   scope      sent on the token request; "offline_access" also returns a
#              refresh_token. Empty by default: a script rarely needs one, and
#              an unused refresh token is a live credential in a table.
#
# Exit codes: 0 signed in; 1 refused (wrong password, unknown account, no code);
# 2 usage; 3 rate limited (too many refused passwords).

set -euo pipefail

die() {
    local code="$1"
    shift
    echo "first-party-sign-in: $*" >&2
    exit "$code"
}

if [ $# -lt 3 ] || [ $# -gt 5 ]; then
    die 2 "usage: $(basename "$0") <base_url> <email> <password|-> [client_id] [scope]"
fi

for tool in curl openssl jq; do
    command -v "$tool" >/dev/null 2>&1 || die 2 "$tool is required"
done

BASE_URL="${1%/}"
EMAIL="$2"
PASSWORD="$3"
CLIENT_ID="${4:-dravr-mobile}"
SCOPE="${5:-}"

if [ "$PASSWORD" = "-" ]; then
    IFS= read -r PASSWORD || true
fi
[ -n "$PASSWORD" ] || die 2 "empty password"

ORIGIN="$(printf '%s' "$BASE_URL" | sed -E 's#^([a-zA-Z][a-zA-Z0-9+.-]*://[^/]+).*#\1#')"
case "$CLIENT_ID" in
    dravr-mobile) REDIRECT_URI="dravr://auth/callback" ;;
    dravr-web) REDIRECT_URI="${FRONTEND_URL:-$ORIGIN}"; REDIRECT_URI="${REDIRECT_URI%/}/auth/callback" ;;
    *) die 2 "client_id must be dravr-mobile or dravr-web, got '$CLIENT_ID'" ;;
esac

WORK="$(mktemp -d "${TMPDIR:-/tmp}/first-party-sign-in.XXXXXX")"
chmod 700 "$WORK"
trap 'rm -rf "$WORK"' EXIT
JAR="$WORK/cookies"
HEADERS="$WORK/headers"
BODY="$WORK/body"

# RFC 7636: a 43+ character verifier from the unreserved set, its unpadded
# base64url SHA-256 challenge, and an unguessable state.
b64url() { openssl base64 -A | tr '+/' '-_' | tr -d '='; }
VERIFIER="$(openssl rand 32 | b64url)"
CHALLENGE="$(printf '%s' "$VERIFIER" | openssl dgst -sha256 -binary | b64url)"
STATE="$(openssl rand 16 | b64url)"

# The Location header of the last response in $HEADERS, CR stripped.
location() {
    { grep -i '^location:' "$HEADERS" || true; } | tail -n1 | sed -E 's/^[Ll]ocation:[[:space:]]*//' | tr -d '\r'
}

# One query parameter of a URL, percent-decoded.
query_param() {
    local name="$1" url="$2" raw
    raw="$(printf '%s' "${url#*\?}" | tr '&' '\n' | sed -n "s/^${name}=//p" | head -n1)"
    raw="${raw//+/ }"
    printf '%b' "${raw//%/\\x}"
}

# Step 1: the hosted login form. Credentials are written to files and
# url-encoded by curl from there, so they never reach argv or stderr.
printf '%s' "$EMAIL" >"$WORK/email"
printf '%s' "$PASSWORD" >"$WORK/password"
STATUS="$(curl -sS -o "$BODY" -D "$HEADERS" -w '%{http_code}' -c "$JAR" -b "$JAR" \
    -X POST "$BASE_URL/oauth2/login" \
    --data-urlencode "client_id=$CLIENT_ID" \
    --data-urlencode "redirect_uri=$REDIRECT_URI" \
    --data-urlencode "response_type=code" \
    --data-urlencode "state=$STATE" \
    --data-urlencode "scope=" \
    --data-urlencode "code_challenge=$CHALLENGE" \
    --data-urlencode "code_challenge_method=S256" \
    --data-urlencode "resource=" \
    --data-urlencode "email@$WORK/email" \
    --data-urlencode "password@$WORK/password")" || die 1 "could not reach $BASE_URL/oauth2/login"
rm -f "$WORK/password"

case "$STATUS" in
    302 | 303) ;;
    429) die 3 "sign-in for $EMAIL refused: too many refused passwords (HTTP 429), retry later" ;;
    *) die 1 "sign-in for $EMAIL refused by the hosted login page (HTTP $STATUS): wrong password, unknown or suspended account" ;;
esac
AUTHORIZE="$(location)"
case "$AUTHORIZE" in
    /oauth2/authorize*) AUTHORIZE="$ORIGIN$AUTHORIZE" ;;
    http*://*/oauth2/authorize*) ;;
    *) die 1 "hosted login redirected somewhere unexpected: ${AUTHORIZE%%\?*}" ;;
esac

# Step 2: the authorization request, signed in by the session cookie.
STATUS="$(curl -sS -o "$BODY" -D "$HEADERS" -w '%{http_code}' -c "$JAR" -b "$JAR" \
    "$AUTHORIZE")" || die 1 "could not reach /oauth2/authorize"
CALLBACK="$(location)"
case "$CALLBACK" in
    "$REDIRECT_URI"\?*) ;;
    *) die 1 "/oauth2/authorize answered HTTP $STATUS without redirecting to $REDIRECT_URI (redirect_uri not allowed for $CLIENT_ID on this server? check FRONTEND_URL)" ;;
esac
ERROR="$(query_param error "$CALLBACK")"
[ -z "$ERROR" ] || die 1 "authorization refused: $ERROR $(query_param error_description "$CALLBACK")"
[ "$(query_param state "$CALLBACK")" = "$STATE" ] || die 1 "authorization returned a mismatched state"
CODE="$(query_param code "$CALLBACK")"
[ -n "$CODE" ] || die 1 "authorization redirect carried no code"

# Step 3: redeem the code with the PKCE verifier.
TOKEN_ARGS=(
    --data-urlencode "grant_type=authorization_code"
    --data-urlencode "client_id=$CLIENT_ID"
    --data-urlencode "code=$CODE"
    --data-urlencode "redirect_uri=$REDIRECT_URI"
    --data-urlencode "code_verifier=$VERIFIER"
)
[ -z "$SCOPE" ] || TOKEN_ARGS+=(--data-urlencode "scope=$SCOPE")
STATUS="$(curl -sS -o "$BODY" -w '%{http_code}' -c "$JAR" -b "$JAR" \
    -X POST "$BASE_URL/oauth/token" "${TOKEN_ARGS[@]}")" || die 1 "could not reach /oauth/token"
if [ "$STATUS" != "200" ] || ! jq -e '.access_token | strings | length > 0' "$BODY" >/dev/null 2>&1; then
    die 1 "token exchange failed (HTTP $STATUS): $(jq -r '[.error, .error_description] | map(select(.)) | join(": ")' "$BODY" 2>/dev/null || echo 'non-JSON body')"
fi
cat "$BODY"
