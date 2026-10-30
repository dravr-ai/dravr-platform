#!/usr/bin/env bash
# ABOUTME: Runs cloud-run-canary.sh against a stubbed gcloud: promote, send back on timeout, refuse a bad starting state
# ABOUTME: Asserts the exact traffic moves, because a canary that sends back nothing is a pin no one knows about
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# carnet#1269. Each case asserts the script's own message and the traffic
# commands it ran, in order: an exit code alone cannot tell "sent the canary
# back" from "crashed before moving anything". The run fails unless exactly
# EXPECTED_CASES cases ran and all passed. Counters are plain assignments,
# never `(( n++ ))`, which `set -e` aborts on when n is 0.
#
# Usage: scripts/ci/cloud-run-canary.test.sh — bash and jq; no network.

set -euo pipefail
cd "$(dirname "$0")/../.."
SCRIPT="${PWD}/scripts/ci/cloud-run-canary.sh"
WORK=$(mktemp -d); trap 'rm -rf "${WORK}"' EXIT

EXPECTED_CASES=16
CASES_RUN=0
CASES_PASSED=0
CASES_FAILED=0

SERVICE="dravr-mcp-server-api"
REGION="northamerica-northeast1"
OLD="${SERVICE}-00001-old"
NEW="${SERVICE}-00002-new"

# ---------------------------------------------------------------------------
# Stub gcloud. `run services describe` prints ${STUB_DIR}/service.json.
# `run revisions describe` prints a revision whose Ready status is the Nth line
# of ${STUB_DIR}/ready on its Nth call (the last line repeats; True without the
# file), with the message in ${STUB_DIR}/ready.message; a revision.fail file
# makes it fail.
# `logging read` prints the Nth line of ${STUB_DIR}/logs on its Nth call
# (nothing past the end, or without the file); a logging.fail file makes it
# fail. `run services update-traffic` succeeds unless update-traffic.fail
# exists. Every call is logged to calls.log.
# ---------------------------------------------------------------------------
BIN="${WORK}/bin"; mkdir -p "${BIN}"
cat > "${BIN}/gcloud" <<SH
#!${BASH}
SH
cat >> "${BIN}/gcloud" <<'SH'
echo "$*" >> "${STUB_DIR}/calls.log"
case "$1 $2 $3" in
  "config get-value project") echo "dravr-dev"; exit 0 ;;
  "run services describe") cat "${STUB_DIR}/service.json"; exit 0 ;;
  "run revisions describe")
    [ -e "${STUB_DIR}/revision.fail" ] && { echo "ERROR: (gcloud.run.revisions.describe) Cannot find revision [$4]" >&2; exit 1; }
    status="True"
    if [ -e "${STUB_DIR}/ready" ]; then
      n=0; [ -e "${STUB_DIR}/ready.count" ] && n=$(<"${STUB_DIR}/ready.count")
      n=$(( n + 1 )); echo "${n}" > "${STUB_DIR}/ready.count"
      lines=$(wc -l < "${STUB_DIR}/ready")
      status=$(sed -n "$(( n < lines ? n : lines ))p" "${STUB_DIR}/ready")
    fi
    message=""; [ -e "${STUB_DIR}/ready.message" ] && message=$(<"${STUB_DIR}/ready.message")
    jq -n --arg s "${status}" --arg m "${message}" '{status: {conditions: [{type: "Ready", status: $s, message: $m}]}}'
    exit 0 ;;
  "run services update-traffic")
    [ -e "${STUB_DIR}/update-traffic.fail" ] && { echo "ERROR: stub refused" >&2; exit 1; }
    exit 0 ;;
esac
if [ "$1 $2" = "logging read" ]; then
  [ -e "${STUB_DIR}/logging.fail" ] && { echo "ERROR: PERMISSION_DENIED logging.logEntries.list" >&2; exit 1; }
  n=0; [ -e "${STUB_DIR}/logging.count" ] && n=$(<"${STUB_DIR}/logging.count")
  n=$(( n + 1 )); echo "${n}" > "${STUB_DIR}/logging.count"
  [ -e "${STUB_DIR}/logs" ] && sed -n "${n}p" "${STUB_DIR}/logs"
  exit 0
fi
echo "stub gcloud: unexpected call: $*" >&2; exit 2
SH
chmod +x "${BIN}/gcloud"

# Stub the ramp's headroom wait (wait-cloud-run-service-ready.sh headroom): it
# logs "wait <args>" and fails on the Nth call when ${STUB_DIR}/wait.fail.N
# exists. With ${STUB_DIR}/wait.block it records its pid in wait.pid and
# blocks, as a wait for a reading that has not arrived does.
cat > "${BIN}/wait-headroom" <<SH
#!${BASH}
SH
cat >> "${BIN}/wait-headroom" <<'SH'
echo "wait $*" >> "${STUB_DIR}/calls.log"
n=0; [ -e "${STUB_DIR}/wait.count" ] && n=$(<"${STUB_DIR}/wait.count")
n=$(( n + 1 )); echo "${n}" > "${STUB_DIR}/wait.count"
if [ -e "${STUB_DIR}/wait.fail.${n}" ]; then echo "::error::stub: no headroom" >&2; exit 1; fi
if [ -e "${STUB_DIR}/wait.block" ]; then echo "$$" > "${STUB_DIR}/wait.pid"; exec sleep 60; fi
echo "Database headroom: stub"
SH
chmod +x "${BIN}/wait-headroom"

# A service whose newest revision is ${created}, ${ready} the latest ready one,
# and ${traffic} its serving split as "rev=percent,…". After a no_traffic deploy
# the service still names the revision serving traffic as its latest ready one
# (2026-10-30 15:33), so the cases below pass ${OLD} there.
service_json() {
  local created="$1" ready="$2" traffic="$3"
  jq -n --arg c "${created}" --arg r "${ready}" --arg t "${traffic}" '{
    status: {
      latestCreatedRevisionName: $c,
      latestReadyRevisionName: $r,
      traffic: ($t | split(",") | map(select(. != "") | split("=") | {revisionName: .[0], percent: (.[1] | tonumber)}))
    }
  }'
}

# The update-traffic and headroom-wait calls a case made, in order, one per
# line, without the region; a wait's --fresh-after epoch reads as <epoch>.
traffic_moves() {
  grep -E '^(run services update-traffic|wait )' "${STUB_DIR}/calls.log" \
    | sed -E -e "s/ --region ${REGION}//" -e 's/ --quiet//' -e 's/^run services update-traffic //' \
        -e 's/ --fresh-after [1-9][0-9]* / --fresh-after <epoch> /' || true
}

# run_case <name> <expected exit> <expected output fragment> <expected moves, ";"-separated>
run_case() {
  local name="$1" want_exit="$2" want_out="$3" want_moves="$4" got_exit=0 got_moves
  CASES_RUN=$(( CASES_RUN + 1 ))
  PATH="${BIN}:${PATH}" STUB_DIR="${STUB_DIR}" CLOUD_RUN_CANARY_POLL_SECONDS=1 \
    CLOUD_RUN_CANARY_RAMP_SETTLE_SECONDS=0 CLOUD_RUN_CANARY_WAIT_SCRIPT="${BIN}/wait-headroom" \
    "${SCRIPT}" --service "${SERVICE}" --region "${REGION}" --timeout-seconds "${TIMEOUT:-5}" ${RAMP:+--ramp-db-prefix "${RAMP}"} \
    >"${STUB_DIR}/out" 2>&1 || got_exit=$?
  got_moves=$(traffic_moves | paste -sd ';' -)
  if [ "${got_exit}" = "${want_exit}" ] && grep -qF -- "${want_out}" "${STUB_DIR}/out" && [ "${got_moves}" = "${want_moves}" ]; then
    CASES_PASSED=$(( CASES_PASSED + 1 )); echo "  ok   ${name}"
  else
    CASES_FAILED=$(( CASES_FAILED + 1 ))
    echo "  FAIL ${name}: exit ${got_exit} (want ${want_exit}); moves '${got_moves}' (want '${want_moves}')"
    sed 's/^/       /' "${STUB_DIR}/out"
  fi
}

# run_signal_case <name> <signal> <expected exit> [<expected moves>]: without
# RAMP no pod ever reads its key, and the signal lands once the 1% has moved;
# with RAMP one does, the headroom wait blocks, and the signal lands in that
# wait. The script gets <signal> as a cancelled step does, and must send the
# share back without waiting on the operation, stopping a blocked headroom
# wait. Job control (set -m) keeps SIGINT deliverable: a non-interactive shell
# starts a background job with SIGINT ignored, and an ignored signal cannot be
# trapped.
run_signal_case() {
  local name="$1" sig="$2" want_exit="$3" got_exit=0 got_moves pid tries=0 took started waiter_left=""
  local want_moves="${4:-${SERVICE} --to-revisions ${NEW}=1;${SERVICE} --to-revisions ${NEW}=0 --async}"
  local reached="^logging read"
  [ -z "${RAMP:-}" ] || reached="^wait headroom"
  CASES_RUN=$(( CASES_RUN + 1 ))
  set -m
  PATH="${BIN}:${PATH}" STUB_DIR="${STUB_DIR}" CLOUD_RUN_CANARY_POLL_SECONDS=30 \
    CLOUD_RUN_CANARY_RAMP_SETTLE_SECONDS=0 CLOUD_RUN_CANARY_WAIT_SCRIPT="${BIN}/wait-headroom" \
    "${SCRIPT}" --service "${SERVICE}" --region "${REGION}" --timeout-seconds 120 ${RAMP:+--ramp-db-prefix "${RAMP}"} \
    >"${STUB_DIR}/out" 2>&1 &
  pid=$!
  set +m
  until grep -q -- "--to-revisions ${NEW}=1" "${STUB_DIR}/calls.log" && grep -q "${reached}" "${STUB_DIR}/calls.log" \
      && { [ -z "${RAMP:-}" ] || [ -s "${STUB_DIR}/wait.pid" ]; }; do
    tries=$(( tries + 1 ))
    [ "${tries}" -le 100 ] || break
    sleep 0.1
  done
  started=${SECONDS}
  kill -s "${sig}" "${pid}"
  wait "${pid}" || got_exit=$?
  # The poll sleeps 30 s and the blocked headroom wait 60 s; a send-back that
  # waits either out would miss a real cancel's 10 s before SIGKILL.
  took=$(( SECONDS - started ))
  got_moves=$(traffic_moves | paste -sd ';' -)
  # The headroom wait the send-back stopped must be gone, not left running.
  if [ -s "${STUB_DIR}/wait.pid" ]; then
    tries=0
    while kill -0 "$(<"${STUB_DIR}/wait.pid")" 2>/dev/null; do
      tries=$(( tries + 1 ))
      if [ "${tries}" -gt 20 ]; then
        waiter_left="yes"; kill "$(<"${STUB_DIR}/wait.pid")" 2>/dev/null || true; break
      fi
      sleep 0.1
    done
  fi
  if [ "${got_exit}" = "${want_exit}" ] && [ "${took}" -lt 10 ] && [ -z "${waiter_left}" ] && grep -qF "sending ${NEW}'s canary share back" "${STUB_DIR}/out" && [ "${got_moves}" = "${want_moves}" ]; then
    CASES_PASSED=$(( CASES_PASSED + 1 )); echo "  ok   ${name}"
  else
    CASES_FAILED=$(( CASES_FAILED + 1 ))
    echo "  FAIL ${name}: exit ${got_exit} (want ${want_exit}) after ${took}s (want under 10); headroom wait left running: ${waiter_left:-no}; moves '${got_moves}' (want '${want_moves}')"
    sed 's/^/       /' "${STUB_DIR}/out"
  fi
}

new_case() {
  STUB_DIR="${WORK}/$1"; mkdir -p "${STUB_DIR}"; : > "${STUB_DIR}/calls.log"
}

echo "cloud-run-canary"

new_case promote
service_json "${NEW}" "${OLD}" "${OLD}=100" > "${STUB_DIR}/service.json"
printf '\n2026-10-30T13:50:08Z\tinstance-1\n' > "${STUB_DIR}/logs"
run_case "a blank answer is not yet; one pod reads its key, then the new revision takes 100%" 0 "one pod read its key at 2026-10-30T13:50:08Z" \
  "${SERVICE} --to-revisions ${NEW}=1;${SERVICE} --to-latest"

new_case ramp
service_json "${NEW}" "${OLD}" "${OLD}=100" > "${STUB_DIR}/service.json"
printf '2026-10-30T13:50:08Z\tinstance-1\n' > "${STUB_DIR}/logs"
# Each step waits for database headroom, on the database the prefix names,
# before the next move.
WAIT_CALL="wait headroom --prefix dravr-mcp-server --db-headroom 6 --fresh-after <epoch> --timeout-seconds 10"
RAMP=dravr-mcp-server run_case "with a ramp, the rest moves 33% then 66% then 100%, waiting for headroom after each step" 0 "33% of ${SERVICE} goes to ${NEW}" \
  "${SERVICE} --to-revisions ${NEW}=1;${SERVICE} --to-revisions ${NEW}=33;${WAIT_CALL};${SERVICE} --to-revisions ${NEW}=66;${WAIT_CALL};${SERVICE} --to-latest"

new_case ramp-no-headroom
service_json "${NEW}" "${OLD}" "${OLD}=100" > "${STUB_DIR}/service.json"
printf '2026-10-30T13:50:08Z\tinstance-1\n' > "${STUB_DIR}/logs"
: > "${STUB_DIR}/wait.fail.1"
RAMP=dravr-mcp-server run_case "no headroom after a step: the new revision's share goes back" 1 "never had 6 free slots with ${NEW} at 33%" \
  "${SERVICE} --to-revisions ${NEW}=1;${SERVICE} --to-revisions ${NEW}=33;${WAIT_CALL};${SERVICE} --to-revisions ${NEW}=0"

new_case timeout
service_json "${NEW}" "${OLD}" "${OLD}=100" > "${STUB_DIR}/service.json"
TIMEOUT=2 run_case "no pod reads its key in time: the 1% goes back" 1 "its pod never reached the database" \
  "${SERVICE} --to-revisions ${NEW}=1;${SERVICE} --to-revisions ${NEW}=0"

new_case no-log-access
service_json "${NEW}" "${OLD}" "${OLD}=100" > "${STUB_DIR}/service.json"
: > "${STUB_DIR}/logging.fail"
run_case "the logs cannot be read: the 1% goes back" 1 "roles/logging.viewer" \
  "${SERVICE} --to-revisions ${NEW}=1;${SERVICE} --to-revisions ${NEW}=0"

new_case not-a-timestamp
service_json "${NEW}" "${OLD}" "${OLD}=100" > "${STUB_DIR}/service.json"
printf 'WARNING: something else\n' > "${STUB_DIR}/logs"
run_case "an answer that is not a log timestamp fails: the 1% goes back" 1 "is not a log timestamp" \
  "${SERVICE} --to-revisions ${NEW}=1;${SERVICE} --to-revisions ${NEW}=0"

new_case cancel-int
service_json "${NEW}" "${OLD}" "${OLD}=100" > "${STUB_DIR}/service.json"
run_signal_case "a cancel's SIGINT mid-wait sends the 1% back at once" INT 130

new_case cancel-term
service_json "${NEW}" "${OLD}" "${OLD}=100" > "${STUB_DIR}/service.json"
run_signal_case "a SIGTERM mid-wait sends the 1% back at once" TERM 143

new_case cancel-ramp
service_json "${NEW}" "${OLD}" "${OLD}=100" > "${STUB_DIR}/service.json"
printf '2026-10-30T13:50:08Z\tinstance-1\n' > "${STUB_DIR}/logs"
: > "${STUB_DIR}/wait.block"
RAMP=dravr-mcp-server run_signal_case "a cancel's SIGINT during a headroom wait stops it and sends the 33% back at once" INT 130 \
  "${SERVICE} --to-revisions ${NEW}=1;${SERVICE} --to-revisions ${NEW}=33;${WAIT_CALL/--timeout-seconds 10/--timeout-seconds 240};${SERVICE} --to-revisions ${NEW}=0 --async"

new_case canary-refused
service_json "${NEW}" "${OLD}" "${OLD}=100" > "${STUB_DIR}/service.json"
: > "${STUB_DIR}/update-traffic.fail"
run_case "the 1% move is refused: it still tries to send it back" 1 "could not move 1% of ${SERVICE}'s traffic" \
  "${SERVICE} --to-revisions ${NEW}=1;${SERVICE} --to-revisions ${NEW}=0"

new_case becomes-ready
service_json "${NEW}" "${OLD}" "${OLD}=100" > "${STUB_DIR}/service.json"
printf 'Unknown\nUnknown\nTrue\n' > "${STUB_DIR}/ready"
printf '2026-10-30T13:50:08Z\tinstance-1\n' > "${STUB_DIR}/logs"
run_case "the deploy returns before the revision is Ready: wait for it, then canary" 0 "${NEW}: Ready=Unknown, waiting" \
  "${SERVICE} --to-revisions ${NEW}=1;${SERVICE} --to-latest"

new_case not-ready
service_json "${NEW}" "${OLD}" "${OLD}=100" > "${STUB_DIR}/service.json"
echo False > "${STUB_DIR}/ready"
echo "Container failed to start" > "${STUB_DIR}/ready.message"
run_case "the newest revision failed: no traffic moves" 1 "is not Ready: Container failed to start" ""

new_case never-ready
service_json "${NEW}" "${OLD}" "${OLD}=100" > "${STUB_DIR}/service.json"
echo Unknown > "${STUB_DIR}/ready"
TIMEOUT=2 run_case "the newest revision never settles: no traffic moves" 1 "did not become Ready within 2s" ""

new_case revision-unreadable
service_json "${NEW}" "${OLD}" "${OLD}=100" > "${STUB_DIR}/service.json"
: > "${STUB_DIR}/revision.fail"
run_case "the newest revision cannot be read: no traffic moves" 1 "gcloud run revisions describe ${NEW} failed: ERROR: (gcloud.run.revisions.describe) Cannot find revision" ""

new_case already-serving
service_json "${NEW}" "${NEW}" "${NEW}=100" > "${STUB_DIR}/service.json"
run_case "the newest revision already serves: no traffic moves" 1 "already carries traffic" ""

new_case bad-region
service_json "${NEW}" "${OLD}" "${OLD}=100" > "${STUB_DIR}/service.json"
CASES_RUN=$(( CASES_RUN + 1 ))
got_exit=0
PATH="${BIN}:${PATH}" STUB_DIR="${STUB_DIR}" "${SCRIPT}" --service "${SERVICE}" --region "nowhere" >"${STUB_DIR}/out" 2>&1 || got_exit=$?
if [ "${got_exit}" = 1 ] && grep -qF "is not a Cloud Run region name" "${STUB_DIR}/out" && [ ! -s "${STUB_DIR}/calls.log" ]; then
  CASES_PASSED=$(( CASES_PASSED + 1 )); echo "  ok   an invalid region is refused before any gcloud call"
else
  CASES_FAILED=$(( CASES_FAILED + 1 )); echo "  FAIL an invalid region: exit ${got_exit}"; sed 's/^/       /' "${STUB_DIR}/out"
fi

echo
echo "${CASES_PASSED} passed, ${CASES_FAILED} failed (${CASES_RUN} of ${EXPECTED_CASES} cases ran)"
[ "${CASES_FAILED}" -eq 0 ] && [ "${CASES_RUN}" -eq "${EXPECTED_CASES}" ] && [ "${CASES_PASSED}" -eq "${EXPECTED_CASES}" ]
