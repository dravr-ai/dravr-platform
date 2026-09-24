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
SCIOTTE_REPO="${SCIOTTE_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../dravr-sciotte" 2>/dev/null && pwd || echo ../dravr-sciotte)}"
BASE="http://${HOST}:${PORT}"
BIN="${SCIOTTE_REPO}/target/debug/dravr-sciotte-server"

# Process identity for serve-bg. The library is the platform checkout's, so the
# pid file it writes is the one bin/stop-all.sh reads when it stops `sciotte`.
# shellcheck source=../bin/dev-processes.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")/../bin" && pwd)/dev-processes.sh"

# No auth material: a loopback-bound scraper serves unauthenticated (its
# development mode — deployed instances require Google identity tokens).

# Server-side queue knobs (fail-fast if unset). Mirror the dev values.
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

hdr=(-H "Content-Type: application/json")

info() { echo -e "${BLUE}==>${NC} $*"; }
ok()   { echo -e "${GREEN}✓${NC} $*"; }
warn() { echo -e "${YELLOW}!${NC} $*"; }
err()  { echo -e "${RED}✗${NC} $*" >&2; }

# One instance serves every athlete, so the server resumes a login only by the
# flow_id its first step minted, and reads for an athlete only for the session
# named in X-Session-Id. Each command is its own process, so what a login step
# hands back is kept beside this checkout's pid files (gitignored logs/) for the
# next command to name.
state_file() { echo "${DEV_RUN_DIR}/sciotte-local.$1"; }

remember() { # flow|session value
  ( umask 077; mkdir -p "${DEV_RUN_DIR}"; printf '%s\n' "$2" >"$(state_file "$1")" )
}

forget() { rm -f "$(state_file "$1")"; }

# The id a command names: its argument, else the one the last login step
# handed back. Neither known is refused here, before any request, since the
# server would only refuse it too.
remembered() { # flow|session given hint
  local id="$2"
  [ -n "${id}" ] || [ ! -r "$(state_file "$1")" ] || id="$(cat "$(state_file "$1")")"
  if [ -z "${id}" ]; then
    err "no ${1}_id known — $3"
    return 1
  fi
  printf '%s' "${id}"
}

# The value of one top-level field of a JSON response, empty when absent or
# when the response is not a JSON object.
json_field() { jq -r --arg k "$1" '.[$k] // empty' <<<"$2" 2>/dev/null || true; }

print_json() { jq . <<<"$1" 2>/dev/null || printf '%s\n' "$1"; }

# POST one login step (the JSON body on stdin), print the response and keep
# what it hands back: a continuation's flow_id for otp/2fa, or the session a
# successful login established. A concluded flow — authenticated or failed —
# is forgotten so no later step names it.
login_step() { # path
  local resp session flow
  if ! resp="$(curl -s "${hdr[@]}" -X POST "${BASE}$1" --data @-)"; then
    err "no answer from ${BASE}$1 — is the server up? (scripts/sciotte-local.sh serve)"
    return 1
  fi
  print_json "${resp}"
  session="$(json_field session_id "${resp}")"
  flow="$(json_field flow_id "${resp}")"
  if [ -n "${session}" ]; then
    remember session "${session}"
    forget flow
    ok "session ${session} remembered for status / athlete / activities"
  elif [ -n "${flow}" ]; then
    remember flow "${flow}"
    ok "flow ${flow} remembered for otp / 2fa"
  elif [ "$(json_field status "${resp}")" = "failed" ]; then
    forget flow
  fi
}

# GET a per-athlete route for one session. The id goes to curl on stdin
# (`-H @-`), keeping it out of the process table as the password is.
get_for_session() { # path session_id
  local resp
  if ! resp="$(printf 'X-Session-Id: %s\n' "$2" | curl -s "${hdr[@]}" -H @- "${BASE}$1")"; then
    err "no answer from ${BASE}$1 — is the server up? (scripts/sciotte-local.sh serve)"
    return 1
  fi
  print_json "${resp}"
}

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
  otp <code> [flow_id]       POST /auth/submit-otp
  2fa <option_id> [flow_id]  POST /auth/select-2fa   ('2fa poll' after number_match)
                       flow_id: the one the last login step returned, unless given
  status [session_id]        GET /auth/status
  athlete [session_id]       GET /api/athlete
  activities [session_id]    GET /api/activities
                       sent as X-Session-Id: the session the last login step
                       returned, unless given — the server has no default session

  login/otp/2fa remember the flow_id and session_id each response hands back,
  in $(state_file flow) and $(state_file session).

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
# invocation in a process group of its own, records its pid where
# bin/stop-all.sh reads it, and waits for the health endpoint to answer from
# that pid. Idempotent (a no-op when this checkout's own service already holds
# the port) and self-building (compiles the binary on demand) so the
# orchestrating startup script needs no separate build/serve steps.
cmd_serve_bg() {
  local -a args=()
  while IFS= read -r line; do args+=("${line}"); done < <(provider_args "${1:-}")
  local owned pid logdir logf
  # A 200 on this port says the port answered, not whose service answered it:
  # every worktree runs its own sciotte on 8091. The skip needs both halves — a
  # live record of ours, and that recorded process behind the port.
  if owned="$(dev_owned sciotte)"; then
    read -r pid _ <<<"${owned}"
    if dev_pid_owns_port "${pid}" "${PORT}"; then
      ok "sciotte service already up on ${BASE} (pid ${pid})"
      return 0
    fi
  fi
  [ -x "${BIN}" ] || cmd_build
  logdir="${SCIOTTE_LOCAL_LOGDIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/logs}"
  mkdir -p "${logdir}"
  logf="${logdir}/sciotte-service.log"
  info "Starting sciotte service (${1:-both providers}) on ${BASE} (log: ${logf})"
  dev_stop sciotte "Sciotte scraper service" >/dev/null
  dev_spawn sciotte "${logf}" \
    "${BIN}" ${args[@]+"${args[@]}"} serve --host "${HOST}" --port "${PORT}"
  pid="${DEV_SPAWNED_PID}"
  if dev_wait_healthy "${PORT}" "${pid}" /health 30; then
    ok "sciotte service healthy (pid ${pid}, log ${logf})"
    return 0
  fi
  err "sciotte service did not become healthy in 30s — see ${logf}"
  return 1
}

cmd_health()     { require_bin; curl -s "${hdr[@]}" "${BASE}/health" | jq . 2>/dev/null || curl -s "${hdr[@]}" "${BASE}/health"; echo; }

# The per-athlete reads, each for the session its argument names, else the one
# the last login step established.
cmd_for_session() { # path [session_id]
  local sid
  sid="$(remembered session "${2:-}" "log in first (scripts/sciotte-local.sh login <email>) or pass the session id as the argument")" || exit 1
  get_for_session "$1" "${sid}"
}

cmd_status()     { cmd_for_session /auth/status "${1:-}"; }
cmd_athlete()    { cmd_for_session /api/athlete "${1:-}"; }
cmd_activities() { cmd_for_session /api/activities "${1:-}"; }

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
  # A new login abandons any flow an earlier one left parked.
  forget flow
  # Password goes only into the JSON body via --data @-, never argv/history.
  jq -n --arg e "${email}" --arg p "${password}" --arg m "${method}" --arg pr "${provider}" \
     '{email:$e, password:$p, method:$m, provider:$pr}' \
    | login_step /auth/login-with-credentials
  warn "If status=otp_required → run: scripts/sciotte-local.sh otp <code>"
  warn "If status=two_factor_choice → run: scripts/sciotte-local.sh 2fa <option_id>"
  warn "If status=number_match → approve the number on your phone, then: scripts/sciotte-local.sh 2fa poll"
}

cmd_otp() {
  local code="${1:-}" flow
  [ -n "${code}" ] || { err "usage: otp <code> [flow_id]"; exit 1; }
  flow="$(remembered flow "${2:-}" "start one with 'scripts/sciotte-local.sh login <email>' or pass it: otp <code> <flow_id>")" || exit 1
  jq -n --arg c "${code}" --arg f "${flow}" '{code:$c, flow_id:$f}' \
    | login_step /auth/submit-otp
}

cmd_2fa() {
  local opt="${1:-}" flow
  [ -n "${opt}" ] || { err "usage: 2fa <option_id> [flow_id]"; exit 1; }
  flow="$(remembered flow "${2:-}" "start one with 'scripts/sciotte-local.sh login <email>' or pass it: 2fa <option_id> <flow_id>")" || exit 1
  jq -n --arg o "${opt}" --arg f "${flow}" '{option_id:$o, flow_id:$f}' \
    | login_step /auth/select-2fa
}

case "${1:-help}" in
  build)      cmd_build ;;
  serve)      shift; cmd_serve "$@" ;;
  serve-bg)   shift; cmd_serve_bg "$@" ;;
  health)     cmd_health ;;
  login)      shift; cmd_login "$@" ;;
  otp)        shift; cmd_otp "$@" ;;
  2fa)        shift; cmd_2fa "$@" ;;
  status)     shift; cmd_status "$@" ;;
  athlete)    shift; cmd_athlete "$@" ;;
  activities) shift; cmd_activities "$@" ;;
  help|-h|--help) usage ;;
  *) err "unknown command: ${1}"; usage; exit 1 ;;
esac
