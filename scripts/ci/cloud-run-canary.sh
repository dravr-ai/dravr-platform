#!/usr/bin/env bash
# ABOUTME: Moves 1% of a Cloud Run service's traffic to its newest revision, waits for that one pod to reach the database, then moves the rest
# ABOUTME: On any failure or cancel it sends the 1% back, so the revision already serving keeps every request
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
# succeeded, so that line is the proof one pod reached the database. Then the
# new revision takes 100%. The old pods stop receiving requests, their idle
# connections close (SQLX_IDLE_TIMEOUT_SECS), and any further new pod starts
# into the slots they free, queueing its boot read until one does.
#
# Usage:
#   cloud-run-canary.sh --service <name> --region <region> [--timeout-seconds <n>]
#
# The newest revision must carry no traffic (deploy-cloudrun with no_traffic);
# the script waits up to --timeout-seconds for its own Ready condition. Needs roles/run.admin and roles/logging.viewer. A workflow
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
POLL_SECONDS="${CLOUD_RUN_CANARY_POLL_SECONDS:-10}"

die() {
  echo "::error::$1" >&2
  exit 1
}

usage() {
  die "usage: $0 --service <name> --region <region> [--timeout-seconds <n>] — $1"
}

SERVICE=""
REGION=""
# The pod's boot read queues up to POSTGRES_BOOT_WAIT_SECS (150 s on dev)
# inside a 240 s startup probe; past that the pod is not coming. The same bound
# applies first to the revision's Ready wait, so a run lasts at most twice it
# (wait-cloud-run-service-ready.sh's CANARY_MAX_AGE_SECONDS counts on that).
TIMEOUT_SECONDS="300"
while [ $# -gt 0 ]; do
  [ $# -ge 2 ] || usage "option '$1' has no value"
  case "$1" in
    --service) SERVICE="$2" ;;
    --region) REGION="$2" ;;
    --timeout-seconds) TIMEOUT_SECONDS="$2" ;;
    *) usage "unknown option '$1'" ;;
  esac
  shift 2
done

[[ "${SERVICE}" =~ ^[a-z][a-z0-9-]*$ ]] || usage "--service '${SERVICE}' is not a Cloud Run service name"
[[ "${REGION}" =~ ^[a-z]+-[a-z]+[0-9]+$ ]] || usage "--region '${REGION}' is not a Cloud Run region name"
[[ "${TIMEOUT_SECONDS}" =~ ^[1-9][0-9]*$ ]] || usage "--timeout-seconds '${TIMEOUT_SECONDS}' is not a positive integer"
[[ "${POLL_SECONDS}" =~ ^[1-9][0-9]*$ ]] || usage "CLOUD_RUN_CANARY_POLL_SECONDS '${POLL_SECONDS}' is not a positive integer"
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
send_back() {
  local status=$?
  # A cancel gets SIGINT, then SIGTERM 7.5 s later, then SIGKILL 2.5 s after
  # that (actions/runner ProcessInvoker). The send-back ignores the second
  # signal so it is not cut short, and on a signal it does not wait for the
  # traffic operation to finish, which can take longer than the 10 s left.
  trap '' INT TERM
  local wait_flag=()
  case "${status}" in 130|143) wait_flag=(--async) ;; esac
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

gcloud run services update-traffic "${SERVICE}" --region "${REGION}" --to-latest --quiet \
  || die "could not move ${SERVICE}'s traffic to ${NEW}"
promoted="yes"
echo "${NEW} serves 100% of ${SERVICE}"
