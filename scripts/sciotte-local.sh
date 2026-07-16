#!/bin/bash
# ABOUTME: Local-first harness for the ADR-021 dedicated sciotte scraper service.
# ABOUTME: Runs dravr-sciotte-server on 127.0.0.1:8091 and drives its login/scrape flow via curl.

# Lets you exercise the isolated scraper on localhost before any Cloud Run deploy:
# start the server, drive a real Strava/Garmin credential login (password read from
# your terminal / env, never placed on the command line), submit OTP / pick a 2FA
# method, then fetch activities — all against the local service. Use it to validate
# the isolation end-to-end and to see exactly what each endpoint returns (esp. the
# session-of-record question: does login hand back a session we can persist?).

set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; BLUE='\033[0;34m'; YELLOW='\033[0;33m'; NC='\033[0m'

# --- Config (override via env) ---------------------------------------------
PORT="${SCIOTTE_LOCAL_PORT:-8091}"
HOST="${SCIOTTE_LOCAL_HOST:-127.0.0.1}"
API_KEY="${DRAVR_SCIOTTE_API_KEY:-localtest}"
SCIOTTE_REPO="${SCIOTTE_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../dravr-sciotte" 2>/dev/null && pwd || echo ../dravr-sciotte)}"
BASE="http://${HOST}:${PORT}"
BIN="${SCIOTTE_REPO}/target/debug/dravr-sciotte-server"

# Server-side queue knobs (fail-fast if unset). Mirror the dev values.
export DRAVR_SCIOTTE_API_KEY="${API_KEY}"
export DRAVR_SCIOTTE_MAX_CONCURRENT="${DRAVR_SCIOTTE_MAX_CONCURRENT:-2}"
export DRAVR_SCIOTTE_MAX_QUEUE="${DRAVR_SCIOTTE_MAX_QUEUE:-8}"
export DRAVR_SCIOTTE_QUEUE_TIMEOUT_SECS="${DRAVR_SCIOTTE_QUEUE_TIMEOUT_SECS:-10}"
export DRAVR_SCIOTTE_PARKED_PERMIT_TTL_SECS="${DRAVR_SCIOTTE_PARKED_PERMIT_TTL_SECS:-300}"
export DRAVR_SCIOTTE_WATCHDOG_INTERVAL_SECS="${DRAVR_SCIOTTE_WATCHDOG_INTERVAL_SECS:-15}"
export DRAVR_SCIOTTE_RETRY_AFTER_HINT_SECS="${DRAVR_SCIOTTE_RETRY_AFTER_HINT_SECS:-5}"
export DRAVR_SCIOTTE_CLOSED_RETRY_AFTER_SECS="${DRAVR_SCIOTTE_CLOSED_RETRY_AFTER_SECS:-60}"

# Login step timeouts (read by the scraper). Pull from .envrc if present, else default.
export DRAVR_SCIOTTE_LOGIN_TIMEOUT="${DRAVR_SCIOTTE_LOGIN_TIMEOUT:-300}"
export DRAVR_SCIOTTE_EMAIL_STEP_TIMEOUT="${DRAVR_SCIOTTE_EMAIL_STEP_TIMEOUT:-60}"
export DRAVR_SCIOTTE_PASSWORD_STEP_TIMEOUT="${DRAVR_SCIOTTE_PASSWORD_STEP_TIMEOUT:-180}"

hdr=(-H "Authorization: Bearer ${API_KEY}" -H "Content-Type: application/json")

info() { echo -e "${BLUE}==>${NC} $*"; }
ok()   { echo -e "${GREEN}✓${NC} $*"; }
warn() { echo -e "${YELLOW}!${NC} $*"; }
err()  { echo -e "${RED}✗${NC} $*" >&2; }

usage() {
  cat <<EOF
$(echo -e "${BLUE}sciotte-local${NC}") — local harness for the dedicated sciotte scraper service

  Server:   ${BASE}   (bin: ${BIN})

Usage: scripts/sciotte-local.sh <command> [args]

  build              Build dravr-sciotte-server (debug)
  serve [provider]   Run the server in the foreground (Ctrl-C to stop)
                       no arg = ONE instance serving garmin+strava (ADR-021);
                       an explicit provider serves only that config
  serve-bg [provider] Build-if-needed + run detached with a log, wait for health
                       (used by the one-shot dev startup; idempotent)
  health             GET /health
  login <email> [provider]  POST /auth/login-with-credentials
                       password from \$SCIOTTE_TEST_PASSWORD, else prompted (hidden)
                       provider: garmin (default) | strava; method from \$SCIOTTE_TEST_METHOD
  otp <code>         POST /auth/submit-otp
  2fa <option_id>    POST /auth/select-2fa
  status             GET /auth/status
  sessions           GET /auth/sessions
  athlete            GET /api/athlete
  activities         GET /api/activities

Local-first flow:
  1) In terminal A:  scripts/sciotte-local.sh serve
  2) In terminal B:  scripts/sciotte-local.sh login jf@dravr.ai
       (then 'otp <code>' or '2fa <id>' if challenged), then 'activities'
EOF
}

require_bin() {
  [ -x "${BIN}" ] || { err "server binary not built at ${BIN}"; warn "run: scripts/sciotte-local.sh build"; exit 1; }
}

cmd_build() {
  info "Building dravr-sciotte-server (debug) in ${SCIOTTE_REPO}"
  # -p (not --bin): the bin lives in a member crate; `--bin` from the workspace
  # root fails with "no bin target … in default-run packages".
  ( cd "${SCIOTTE_REPO}" && cargo build -p dravr-sciotte-server )
  ok "built: ${BIN}"
}

# Build the --provider args for serve: no request = bare serve (the server
# loads BOTH built-in providers — one multi-provider instance, ADR-021); an
# explicit provider serves only that config file.
provider_args() {
  local provider="${1:-${SCIOTTE_PROVIDER:-}}"
  [ -z "${provider}" ] && return 0
  local cfg="${SCIOTTE_REPO}/providers/${provider}.toml"
  [ -f "${cfg}" ] || { err "provider config not found: ${cfg}"; exit 1; }
  printf -- '--provider\n%s\n' "${cfg}"
}

cmd_serve() {
  require_bin
  local -a args=()
  while IFS= read -r line; do args+=("${line}"); done < <(provider_args "${1:-}")
  info "Serving ${1:-both providers (garmin+strava)} on ${BASE} — Ctrl-C to stop"
  exec "${BIN}" ${args[@]+"${args[@]}"} serve --host "${HOST}" --port "${PORT}"
}

# Background variant of `serve` for the one-shot dev startup: launches the same
# invocation detached with a log file and waits for /health, instead of the
# foreground `exec`. Idempotent (a no-op if the service is already healthy) and
# self-building (compiles the binary on demand) so the orchestrating startup
# script needs no separate build/serve steps.
cmd_serve_bg() {
  local -a args=()
  while IFS= read -r line; do args+=("${line}"); done < <(provider_args "${1:-}")
  # /health is unauthenticated; skip a double-launch if one is already up.
  if curl -s -m 2 "${BASE}/health" >/dev/null 2>&1; then
    ok "sciotte service already healthy on ${BASE}"
    return 0
  fi
  [ -x "${BIN}" ] || cmd_build
  local logdir logf pid
  logdir="${SCIOTTE_LOCAL_LOGDIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/logs}"
  mkdir -p "${logdir}"
  logf="${logdir}/sciotte-service.log"
  info "Starting sciotte service (${1:-both providers}) on ${BASE} (log: ${logf})"
  nohup "${BIN}" ${args[@]+"${args[@]}"} serve --host "${HOST}" --port "${PORT}" > "${logf}" 2>&1 &
  pid=$!
  for _ in $(seq 1 30); do
    if curl -s -m 2 "${BASE}/health" >/dev/null 2>&1; then
      ok "sciotte service healthy (pid ${pid}, log ${logf})"
      return 0
    fi
    sleep 1
  done
  err "sciotte service did not become healthy in 30s — see ${logf}"
  return 1
}

cmd_health()     { require_bin; curl -s "${hdr[@]}" "${BASE}/health" | jq . 2>/dev/null || curl -s "${hdr[@]}" "${BASE}/health"; echo; }
cmd_status()     { curl -s "${hdr[@]}" "${BASE}/auth/status" | jq . 2>/dev/null || curl -s "${hdr[@]}" "${BASE}/auth/status"; echo; }
cmd_sessions()   { curl -s "${hdr[@]}" "${BASE}/auth/sessions" | jq . 2>/dev/null || curl -s "${hdr[@]}" "${BASE}/auth/sessions"; echo; }
cmd_athlete()    { curl -s "${hdr[@]}" "${BASE}/api/athlete" | jq . 2>/dev/null || curl -s "${hdr[@]}" "${BASE}/api/athlete"; echo; }
cmd_activities() { curl -s "${hdr[@]}" "${BASE}/api/activities" | jq . 2>/dev/null || curl -s "${hdr[@]}" "${BASE}/api/activities"; echo; }

cmd_login() {
  local email="${1:-}"
  [ -n "${email}" ] || { err "usage: login <email> [provider]"; exit 1; }
  # Multi-provider server: name the provider on every new flow (defaults to
  # garmin — our sciotte-scraped account). The continuation steps (otp/2fa)
  # need no provider: the server resumes the flow by its id.
  local provider="${2:-${SCIOTTE_PROVIDER:-garmin}}"
  local method="${SCIOTTE_TEST_METHOD:-email}"
  local password="${SCIOTTE_TEST_PASSWORD:-}"
  if [ -z "${password}" ]; then
    read -rs -p "Password for ${email}: " password; echo
  fi
  info "POST /auth/login-with-credentials (email=${email}, provider=${provider}, method=${method}) — driving a REAL scrape login"
  # Password goes only into the JSON body via --data @-, never argv/history.
  jq -n --arg e "${email}" --arg p "${password}" --arg m "${method}" --arg pr "${provider}" \
     '{email:$e, password:$p, method:$m, provider:$pr}' \
    | curl -s "${hdr[@]}" -X POST "${BASE}/auth/login-with-credentials" --data @- \
    | jq . 2>/dev/null || warn "(non-JSON response above)"
  echo
  warn "If status=otp_required → run: scripts/sciotte-local.sh otp <code>"
  warn "If status=two_factor_choice → run: scripts/sciotte-local.sh 2fa <option_id>"
  warn "If status=number_match → approve the number on your phone, then: status"
}

cmd_otp() {
  local code="${1:-}"; [ -n "${code}" ] || { err "usage: otp <code>"; exit 1; }
  jq -n --arg c "${code}" '{code:$c}' \
    | curl -s "${hdr[@]}" -X POST "${BASE}/auth/submit-otp" --data @- \
    | jq . 2>/dev/null || true; echo
}

cmd_2fa() {
  local opt="${1:-}"; [ -n "${opt}" ] || { err "usage: 2fa <option_id>"; exit 1; }
  jq -n --arg o "${opt}" '{option_id:$o}' \
    | curl -s "${hdr[@]}" -X POST "${BASE}/auth/select-2fa" --data @- \
    | jq . 2>/dev/null || true; echo
}

case "${1:-help}" in
  build)      cmd_build ;;
  serve)      cmd_serve ;;
  serve-bg)   shift; cmd_serve_bg "$@" ;;
  health)     cmd_health ;;
  login)      shift; cmd_login "$@" ;;
  otp)        shift; cmd_otp "$@" ;;
  2fa)        shift; cmd_2fa "$@" ;;
  status)     cmd_status ;;
  sessions)   cmd_sessions ;;
  athlete)    cmd_athlete ;;
  activities) cmd_activities ;;
  help|-h|--help) usage ;;
  *) err "unknown command: ${1}"; usage; exit 1 ;;
esac
