#!/usr/bin/env bash
# ABOUTME: Moves 1% of a Cloud Run service's traffic to its newest revision, waits for that one pod to reach the database, then ramps the rest in steps
# ABOUTME: On any failure or cancel it sends the new revision's share back, so the revision already serving keeps every request
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why (carnet#1269): dev's API pods share 22 Cloud SQL slots. Rolled with
# traffic, Cloud Run started as many new pods as the old revision ran, all at
# once: on 2026-10-29 three new pods booted beside three old ones holding 18
# slots, and two of them waited out their boot window and restarted. A deploy
# with `no_traffic` boots no pod at all (2026-10-29 23:43, revision 01543 was
# Ready with nothing running), so Ready proves nothing about the database.
#
# This is the queue: the old revision keeps 99% of the traffic while 1% starts
# ONE new pod (measured 2026-10-30 13:50: one pod, 4 s after the shift). The
# pod logs KEY_INIT_LINE only after its boot read of the encryption key
# succeeded, so that line is the proof one pod reached the database.
#
# One jump from 1% to 100% still started three or four new pods at once while
# the old revision held its connections: Postgres refused 166 connections at
# 18:59 and 34 at 19:13 on 2026-10-30, each burst within seconds of the move.
# With --ramp-db-prefix the rest of the traffic moves in RAMP_STEPS, and before
# each next step the database must show RAMP_HEADROOM free slots on a Cloud
# Monitoring point whose whole 60 s window opens RAMP_SETTLE_SECONDS or more
# after the step: long enough for the old pods, which lost that share, to close
# their idle connections (SQLX_IDLE_TIMEOUT_SECS, 60 s on dev), and late enough
# that the step's new pods have connected. That point is published a minute or
# more after its window closes, so a step waits about 2 to 5 minutes. The wait
# is wait-cloud-run-service-ready.sh's headroom mode.
#
# Usage:
#   cloud-run-canary.sh --service <name> --region <region> [--ramp-db-prefix <prefix>] [--timeout-seconds <n>]
#
# The newest revision must carry no traffic (deploy-cloudrun with no_traffic);
# the script waits up to --timeout-seconds for its own Ready condition, for the
# key read, and for each ramp step's headroom. Needs roles/run.admin,
# roles/logging.viewer and, with a ramp, roles/monitoring.viewer. A workflow
# step runs it with `exec`: a cancel signals the step's own process only, so
# without exec the signal never reaches this script and nothing sends back.
#
# Exit 0: the new revision serves 100% (traffic type LATEST, as terraform
# declares it). Exit 1: the new revision serves nothing and the revision that
# served before keeps 100%, pinned until the next deploy or terraform apply
# moves it; the job goes red and says so.

set -euo pipefail

# Logged by pierre-mcp-server once the DEK has been read from the database and
# installed (crates/pierre-server/src/bin/pierre-mcp-server.rs).
KEY_INIT_LINE="Two-tier key management system fully initialized"
CANARY_PERCENT=1
# The shares the new revision takes between the canary and 100%.
RAMP_STEPS=(33 66)
# Free Cloud SQL slots before the next step: one pod's pool (POSTGRES_MAX_CONNECTIONS).
RAMP_HEADROOM=6
RAMP_SETTLE_SECONDS="${CLOUD_RUN_CANARY_RAMP_SETTLE_SECONDS:-60}"
WAIT_SCRIPT="${CLOUD_RUN_CANARY_WAIT_SCRIPT:-$(dirname "$0")/wait-cloud-run-service-ready.sh}"
POLL_SECONDS="${CLOUD_RUN_CANARY_POLL_SECONDS:-10}"

die() {
  echo "::error::$1" >&2
  exit 1
}

usage() {
  die "usage: $0 --service <name> --region <region> [--ramp-db-prefix <prefix>] [--timeout-seconds <n>] — $1"
}

SERVICE=""
REGION=""
RAMP_DB_PREFIX=""
# The pod's boot read queues up to POSTGRES_BOOT_WAIT_SECS (150 s on dev)
# inside a 240 s startup probe; past that the pod is not coming. The same bound
# applies to the revision's Ready wait, and twice over after RAMP_SETTLE_SECONDS
# to each ramp step (a strictly fresh point arrives up to about five minutes
# after the settle), so the waits of a run add up to at most six times it plus
# two settles, 1920 s (wait-cloud-run-service-ready.sh's
# CANARY_MAX_AGE_SECONDS counts on that).
TIMEOUT_SECONDS="300"
while [ $# -gt 0 ]; do
  [ $# -ge 2 ] || usage "option '$1' has no value"
  case "$1" in
    --service) SERVICE="$2" ;;
    --region) REGION="$2" ;;
    --timeout-seconds) TIMEOUT_SECONDS="$2" ;;
    --ramp-db-prefix) RAMP_DB_PREFIX="$2" ;;
    *) usage "unknown option '$1'" ;;
  esac
  shift 2
done

[[ "${SERVICE}" =~ ^[a-z][a-z0-9-]*$ ]] || usage "--service '${SERVICE}' is not a Cloud Run service name"
[[ "${REGION}" =~ ^[a-z]+-[a-z]+[0-9]+$ ]] || usage "--region '${REGION}' is not a Cloud Run region name"
[[ "${TIMEOUT_SECONDS}" =~ ^[1-9][0-9]*$ ]] || usage "--timeout-seconds '${TIMEOUT_SECONDS}' is not a positive integer"
[[ "${POLL_SECONDS}" =~ ^[1-9][0-9]*$ ]] || usage "CLOUD_RUN_CANARY_POLL_SECONDS '${POLL_SECONDS}' is not a positive integer"
[[ "${RAMP_SETTLE_SECONDS}" =~ ^[0-9]+$ ]] || usage "CLOUD_RUN_CANARY_RAMP_SETTLE_SECONDS '${RAMP_SETTLE_SECONDS}' is not a non-negative integer"
if [ -n "${RAMP_DB_PREFIX}" ]; then
  [[ "${RAMP_DB_PREFIX}" =~ ^[a-z][a-z0-9-]*$ ]] || usage "--ramp-db-prefix '${RAMP_DB_PREFIX}' is not a service-name prefix"
  [ -x "${WAIT_SCRIPT}" ] || die "the ramp's headroom wait ${WAIT_SCRIPT} is not an executable file"
fi
for tool in gcloud jq date sleep mktemp tr cut head; do
  command -v "${tool}" >/dev/null 2>&1 || die "${tool} is not on PATH"
done

WORK=$(mktemp -d)
trap 'rm -rf "${WORK}"' EXIT
PROJECT=$(gcloud config get-value project 2>"${WORK}/project.err") \
  || die "gcloud config get-value project failed: $(tr '\n' ' ' <"${WORK}/project.err")"
[ -n "${PROJECT}" ] && [ "${PROJECT}" != "(unset)" ] || die "no gcloud project is set"

gcloud run services describe "${SERVICE}" --region "${REGION}" --format=json >"${WORK}/svc.json" 2>"${WORK}/svc.err" \
  || die "gcloud run services describe ${SERVICE} failed: $(tr '\n' ' ' <"${WORK}/svc.err")"
NEW=$(jq -r '.status.latestCreatedRevisionName // ""' "${WORK}/svc.json")
[ -n "${NEW}" ] || die "${SERVICE} has no latest created revision"
SERVING=$(jq -r '[(.status.traffic // [])[] | select((.percent // 0) > 0) | "\(.revisionName)=\(.percent)"] | join(",")' "${WORK}/svc.json")
case ",${SERVING}," in
  *",${NEW}="*) die "${NEW} already carries traffic (${SERVING}); this expects a no_traffic deploy" ;;
esac
echo "${SERVICE}: ${SERVING:-nothing} serving; canarying ${NEW}"

# The revision's own Ready condition, never the service's
# latestReadyRevisionName: a no_traffic revision is Ready ("Retired", no pod)
# while the service still names the revision serving traffic as its latest
# ready one, and stays so until traffic moves (2026-10-30 15:33: 01553 Ready,
# the service's latest ready 01552 25 minutes later). The deploy can return
# before the revision has settled, so wait for it.
READY_DEADLINE=$(( $(date +%s) + TIMEOUT_SECONDS ))
while :; do
  gcloud run revisions describe "${NEW}" --region "${REGION}" --format=json \
    >"${WORK}/rev.json" 2>"${WORK}/rev.err" \
    || die "gcloud run revisions describe ${NEW} failed: $(tr '\n' ' ' <"${WORK}/rev.err")"
  [ -s "${WORK}/rev.json" ] || die "gcloud run revisions describe ${NEW} returned nothing"
  ready_status=$(jq -r '[(.status.conditions // [])[] | select(.type == "Ready")][0].status // "none"' "${WORK}/rev.json" 2>/dev/null) \
    || die "gcloud run revisions describe ${NEW} did not return a revision: $(head -c 300 "${WORK}/rev.json" | tr '\n' ' ')"
  case "${ready_status}" in
    True) break ;;
    False) die "${NEW} is not Ready: $(jq -r '[(.status.conditions // [])[] | select(.type == "Ready")][0].message // "no message"' "${WORK}/rev.json"); nothing to send traffic to" ;;
  esac
  now=$(date +%s)
  [ "${now}" -lt "${READY_DEADLINE}" ] \
    || die "${NEW} did not become Ready within ${TIMEOUT_SECONDS}s (Ready=${ready_status:-none}); nothing to send traffic to"
  echo "[$(date -u +%H:%M:%S)] ${NEW}: Ready=${ready_status:-none}, waiting"
  remaining=$(( READY_DEADLINE - now ))
  # In the background, so a cancel lands at once, as in the key-read wait.
  sleep $(( remaining < POLL_SECONDS ? remaining : POLL_SECONDS )) &
  wait "$!"
done

# From here until the 100% move, the new revision may hold the canary share;
# any exit that is not that success sends it back.
promoted=""
waiter=""
send_back() {
  local status=$?
  # A cancel gets SIGINT, then SIGTERM 7.5 s later, then SIGKILL 2.5 s after
  # that (actions/runner ProcessInvoker). The send-back ignores the second
  # signal so it is not cut short, and on a signal it does not wait for the
  # traffic operation to finish, which can take longer than the 10 s left.
  trap '' INT TERM
  local wait_flag=()
  case "${status}" in 130|143) wait_flag=(--async) ;; esac
  [ -z "${waiter}" ] || kill "${waiter}" 2>/dev/null || true
  if [ -z "${promoted}" ]; then
    echo "::warning::sending ${NEW}'s canary share back; ${SERVING:-the previous revision} keeps the traffic, pinned until the next deploy"
    gcloud run services update-traffic "${SERVICE}" --region "${REGION}" --to-revisions "${NEW}=0" --quiet ${wait_flag[@]+"${wait_flag[@]}"} \
      || echo "::error::could not send ${NEW}'s canary share back; repoint it: gcloud run services update-traffic ${SERVICE} --region ${REGION} --to-revisions ${NEW}=0" >&2
  fi
  rm -rf "${WORK}"
  exit "${status}"
}
trap send_back EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

SINCE=$(date -u +%Y-%m-%dT%H:%M:%SZ)
gcloud run services update-traffic "${SERVICE}" --region "${REGION}" --to-revisions "${NEW}=${CANARY_PERCENT}" --quiet \
  || die "could not move ${CANARY_PERCENT}% of ${SERVICE}'s traffic to ${NEW}"
echo "${CANARY_PERCENT}% of ${SERVICE} goes to ${NEW}; waiting up to ${TIMEOUT_SECONDS}s for one pod to read its key"

FILTER="resource.type=\"cloud_run_revision\" AND resource.labels.service_name=\"${SERVICE}\" AND resource.labels.revision_name=\"${NEW}\" AND textPayload=\"${KEY_INIT_LINE}\" AND timestamp>=\"${SINCE}\""
DEADLINE=$(( $(date +%s) + TIMEOUT_SECONDS ))
while :; do
  gcloud logging read "${FILTER}" --project "${PROJECT}" --limit 1 --format='value(timestamp,labels.instanceId)' \
    >"${WORK}/init" 2>"${WORK}/init.err" \
    || die "gcloud logging read failed (the deployer needs roles/logging.viewer): $(tr '\n' ' ' <"${WORK}/init.err")"
  # Promote on a log timestamp, never on mere output: a blank or unexpected
  # line is not the proof, and anything but nothing or a timestamp fails.
  read_at=$(cut -f1 "${WORK}/init")
  if [[ "${read_at}" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z$ ]]; then
    echo "[$(date -u +%H:%M:%S)] ${NEW}: one pod read its key at ${read_at}"
    break
  fi
  [ -z "$(tr -d '[:space:]' <"${WORK}/init")" ] \
    || die "gcloud logging read answered something that is not a log timestamp: $(tr '\n' ' ' <"${WORK}/init")"
  now=$(date +%s)
  [ "${now}" -lt "${DEADLINE}" ] \
    || die "${NEW} logged no '${KEY_INIT_LINE}' within ${TIMEOUT_SECONDS}s of taking ${CANARY_PERCENT}% — its pod never reached the database"
  echo "[$(date -u +%H:%M:%S)] ${NEW}: no pod has read its key yet"
  remaining=$(( DEADLINE - now ))
  # In the background, so a cancel's signal interrupts `wait` at once instead
  # of waiting out the sleep.
  sleep $(( remaining < POLL_SECONDS ? remaining : POLL_SECONDS )) &
  wait "$!"
done

for step in ${RAMP_DB_PREFIX:+"${RAMP_STEPS[@]}"}; do
  gcloud run services update-traffic "${SERVICE}" --region "${REGION}" --to-revisions "${NEW}=${step}" --quiet \
    || die "could not move ${step}% of ${SERVICE}'s traffic to ${NEW}"
  fresh_after=$(( $(date +%s) + RAMP_SETTLE_SECONDS ))
  echo "[$(date -u +%H:%M:%S)] ${step}% of ${SERVICE} goes to ${NEW}; waiting for ${RAMP_HEADROOM} free Cloud SQL slots on a reading from ${RAMP_SETTLE_SECONDS}s on"
  # In the background, so a cancel lands at once; send_back stops it.
  "${WAIT_SCRIPT}" headroom --region "${REGION}" --prefix "${RAMP_DB_PREFIX}" --db-headroom "${RAMP_HEADROOM}" \
    --fresh-after "${fresh_after}" --timeout-seconds "$(( RAMP_SETTLE_SECONDS + 2 * TIMEOUT_SECONDS ))" &
  waiter=$!
  # A waiter that exited is reaped here: clear it before die, so send_back
  # never signals a pid the system may have handed to another process.
  wait "${waiter}" || { waiter=""; die "the database never had ${RAMP_HEADROOM} free slots with ${NEW} at ${step}%"; }
  waiter=""
done

gcloud run services update-traffic "${SERVICE}" --region "${REGION}" --to-latest --quiet \
  || die "could not move ${SERVICE}'s traffic to ${NEW}"
promoted="yes"
echo "${NEW} serves 100% of ${SERVICE}"
