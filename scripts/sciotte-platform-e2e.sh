#!/bin/bash
# ABOUTME: Drives the platform's sciotte login flow end-to-end for the ADR-021 remote path.
# ABOUTME: Reads GARMIN_USER/GARMIN_PASSWORD from .envrc; hits Pierre (:8081), which routes to :8091.

# With DRAVR_SCIOTTE_REMOTE_URL set, Pierre forwards sciotte login/otp/2fa to the
# dedicated dravr-sciotte-server. This script exercises that from the platform edge:
# it mints a JWT for a local test user, then drives login -> otp/2fa -> status using
# the Garmin creds in .envrc. Credentials never appear on the command line — they are
# read from the (gitignored) .envrc and placed only into JSON request bodies.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCIOTTE_REPO="${SCIOTTE_REPO:-$(cd "$HERE/../dravr-sciotte" 2>/dev/null && pwd || echo "$HERE/../dravr-sciotte")}"
set -a
# Platform .envrc: PIERRE_MASTER_ENCRYPTION_KEY, DRAVR_SCIOTTE_REMOTE_URL, DATABASE_URL, …
source "$HERE/.envrc" >/dev/null 2>&1 || true
# dravr-sciotte .envrc: GARMIN_USER / GARMIN_PASSWORD (the scrape creds live with the scraper).
[ -f "$SCIOTTE_REPO/.envrc" ] && source "$SCIOTTE_REPO/.envrc" >/dev/null 2>&1 || true
set +a

RED='\033[0;31m'; GREEN='\033[0;32m'; BLUE='\033[0;34m'; YELLOW='\033[0;33m'; NC='\033[0m'
info() { echo -e "${BLUE}==>${NC} $*"; }
ok()   { echo -e "${GREEN}✓${NC} $*"; }
warn() { echo -e "${YELLOW}!${NC} $*"; }
err()  { echo -e "${RED}✗${NC} $*" >&2; }

PLATFORM="${PLATFORM_URL:-http://localhost:8081}"
E2E_USER="${PLATFORM_E2E_USER:-e2e@dravr.test}"
E2E_PASS="${PLATFORM_E2E_PASSWORD:-E2eTest123!}"
TARGET="${SCIOTTE_TARGET:-garmin}"        # garmin | strava
# Scrape-account credentials picked by target (both live in dravr-sciotte/.envrc).
if [ "$TARGET" = "strava" ]; then
  SCRAPE_USER="${STRAVA_USER:-}"
  SCRAPE_PASSWORD="${STRAVA_PASSWORD:-}"
else
  SCRAPE_USER="${GARMIN_USER:-}"
  SCRAPE_PASSWORD="${GARMIN_PASSWORD:-}"
fi
JWT_CACHE="${TMPDIR:-/tmp}/pierre-e2e-jwt.txt"

pretty() { python3 -m json.tool 2>/dev/null || cat; }

# Mint (and cache) a JWT for the local test user via the OAuth2 password grant.
mint_jwt() {
  local resp
  resp=$(curl -s -X POST "$PLATFORM/oauth/token" \
    -H "Content-Type: application/x-www-form-urlencoded" \
    --data-urlencode "grant_type=password" \
    --data-urlencode "username=$E2E_USER" \
    --data-urlencode "password=$E2E_PASS")
  local tok
  tok=$(printf '%s' "$resp" | python3 -c "import json,sys;print(json.load(sys.stdin).get('access_token',''))" 2>/dev/null || true)
  if [ -z "$tok" ]; then
    err "could not mint JWT for $E2E_USER (create it: scripts/sciotte-platform-e2e.sh setup-user)"
    printf '%s\n' "$resp" | head -c 300; echo; exit 1
  fi
  printf '%s' "$tok" > "$JWT_CACHE"
  printf '%s' "$tok"
}

jwt() { [ -s "$JWT_CACHE" ] && cat "$JWT_CACHE" || mint_jwt; }

require_creds() {
  [ -n "$SCRAPE_USER" ]     || { err "${TARGET^^}_USER not set — add it to dravr-sciotte/.envrc"; exit 1; }
  [ -n "$SCRAPE_PASSWORD" ] || { err "${TARGET^^}_PASSWORD not set — add it to dravr-sciotte/.envrc"; exit 1; }
}

# POST a JSON body to a platform path with the bearer, print status + body.
post_json() {
  local path="$1" body="$2"
  curl -s -m 300 -w "\n[HTTP %{http_code}]\n" -X POST "$PLATFORM$path" \
    -H "Authorization: Bearer $(jwt)" -H "Content-Type: application/json" \
    --data "$body"
}

cmd_setup_user() {
  info "Creating platform test user $E2E_USER (idempotent)"
  ( cd "$HERE" && cargo run --quiet --bin pierre-cli -- user create --email "$E2E_USER" --password "$E2E_PASS" --force ) 2>&1 | tail -3
  rm -f "$JWT_CACHE"
  ok "user ready — run: $0 login"
}

cmd_jwt() { info "Minting fresh JWT"; rm -f "$JWT_CACHE"; local t; t=$(mint_jwt); ok "JWT (${#t} chars) cached at $JWT_CACHE"; }

cmd_login() {
  require_creds
  # Login method: email (default) | google | apple. The Strava scrape account
  # authenticates via Google OAuth, so strava defaults to the google button.
  local method="${SCIOTTE_METHOD:-$([ "$TARGET" = "strava" ] && echo google || echo email)}"
  info "Platform login for $SCRAPE_USER (target=$TARGET, method=$method) → routes to the sciotte service"
  local body
  body=$(jq -n --arg e "$SCRAPE_USER" --arg p "$SCRAPE_PASSWORD" --arg m "$method" --arg t "$TARGET" \
    '{email:$e, password:$p, method:$m, target:$t}')
  post_json "/api/providers/sciotte/login" "$body"
  warn "otp_required → $0 otp <code>   |   two_factor_choice → $0 2fa <id>   |   number_match → approve on phone, then $0 status"
}

cmd_otp() {
  local code="${1:-}"; [ -n "$code" ] || { err "usage: otp <code>"; exit 1; }
  info "Submitting OTP through the platform"
  post_json "/api/providers/sciotte/submit-otp" "$(jq -n --arg c "$code" '{code:$c}')"
}

cmd_2fa() {
  local opt="${1:-}"; [ -n "$opt" ] || { err "usage: 2fa <option_id>"; exit 1; }
  info "Selecting 2FA method through the platform"
  post_json "/api/providers/sciotte/select-2fa" "$(jq -n --arg o "$opt" '{option_id:$o}')"
}

cmd_status() {
  info "Provider status (is Garmin connected via the mirror?)"
  curl -s -H "Authorization: Bearer $(jwt)" "$PLATFORM/api/oauth/providers/status" | pretty 2>/dev/null \
    || curl -s -H "Authorization: Bearer $(jwt)" "$PLATFORM/api/providers/status" | pretty
}

usage() {
  cat <<EOF
$(echo -e "${BLUE}sciotte-platform-e2e${NC}") — drive Pierre's sciotte login (ADR-021 remote path)

  Platform: $PLATFORM   target: $TARGET   user: $E2E_USER

Usage: scripts/sciotte-platform-e2e.sh <command> [args]
  setup-user      Create the local test user (once, needs cargo)
  jwt             Mint + cache a fresh JWT
  login           POST /api/providers/sciotte/login using GARMIN_USER/GARMIN_PASSWORD from .envrc
  otp <code>      POST /api/providers/sciotte/submit-otp
  2fa <id>        POST /api/providers/sciotte/select-2fa
  status          Provider connection status

Add to .envrc:  export GARMIN_USER=...   export GARMIN_PASSWORD=...
EOF
}

case "${1:-help}" in
  setup-user) cmd_setup_user ;;
  jwt)        cmd_jwt ;;
  login)      cmd_login ;;
  otp)        shift; cmd_otp "$@" ;;
  2fa)        shift; cmd_2fa "$@" ;;
  status)     cmd_status ;;
  help|-h|--help) usage ;;
  *) err "unknown command: ${1}"; usage; exit 1 ;;
esac
