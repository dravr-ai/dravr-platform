#!/usr/bin/env bash
# ABOUTME: Serialises dev Cloud Run rollouts: waits until no dev service is mid-rollout, or until one service settles
# ABOUTME: Holds the one list of dev Cloud Run services every deploy workflow waits on; fails closed on every unknown
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why (carnet#696): dev's services share one small Cloud SQL instance. When two
# workflows roll revisions at once, every new revision cold-starts its pools at
# the same moment and the database runs out of connection slots. Each per-workflow
# GitHub concurrency group serialises its own runs only — a shared group would
# cancel the pending runs of the others — so serialisation happens here, against
# Cloud Run itself: every job that rolls a dev revision first waits until no dev
# service is mid-rollout, deploys, then waits until its own service has settled.
#
# Usage:
#   wait-cloud-run-service-ready.sh quiesce --region <region> --prefix <prefix> [--timeout-seconds <n>]
#   wait-cloud-run-service-ready.sh service --region <region> --prefix <prefix> --service <suffix> [--timeout-seconds <n>]
#
#   --region   the dev Cloud Run region (vars.GCP_DEV_CLOUD_RUN_REGION). Required,
#              never defaulted: a defaulted region describes services that do not
#              exist there, or worse, other ones that do.
#   --prefix   the dev service-name prefix (vars.GCP_DEV_SERVICE_NAME).
#   --service  one suffix from DEV_SERVICE_SUFFIXES below (service mode only).
#
# A service is SETTLED when Cloud Run has observed its latest spec
# (observedGeneration == generation), its Ready condition is True, the latest
# created revision is the latest ready one, and that revision takes 100% of the
# traffic. It is ROLLING while any of the first three does not hold, and SPLIT
# when traffic is anywhere else. Two states have nothing starting, so quiesce
# lets the next deploy through with a warning while service mode does not:
#   - FAILED (Ready=False): the rollout stopped. Blocking on it would let one bad
#     revision block the very deploy that fixes it. Service mode fails at once,
#     because that rollout was ours.
#   - SPLIT: traffic pinned to an older revision (a diagnostic
#     `update-traffic --to-revisions`). deploy-cloudrun never touches traffic, so
#     no deploy clears it, and terraform apply-dev, which does, waits here too:
#     blocking would wedge every deploy for good. Service mode keeps waiting and
#     times out naming the remedy, because a deploy that lands without traffic
#     shipped nothing.
#
# Waiters held behind one rollout would otherwise all see it settle on the same
# poll and roll together, the very cold-start pile-up this exists to stop. So a
# quiesce waiter that had to wait sleeps a random 0..JITTER seconds
# (WAIT_CLOUD_RUN_JITTER_SECONDS, default 60) and re-checks before passing; a
# waiter that finds dev quiet on its first poll passes at once. Jitter spreads
# the herd, it does not lock it: two waiters drawing within a few seconds of
# each other can still both pass before either revision is visible. Only a
# cross-workflow lock (a GCS object taken with ifGenerationMatch around deploy
# and settle) would close that window, and none exists yet.
#
# Every gcloud failure (including a service that does not exist), an empty or
# non-JSON answer, a missing tool and an invalid argument exits 1 immediately.
# Only a timeout is reached by waiting, and it exits 1 too.

set -euo pipefail

# The one list of dev Cloud Run services. Every deploy workflow reaches it through
# this script, and the test harness pins which jobs must call it.
DEV_SERVICE_SUFFIXES=(api frontend photograveur sciotte)

POLL_SECONDS="${WAIT_CLOUD_RUN_POLL_SECONDS:-10}"
JITTER_SECONDS="${WAIT_CLOUD_RUN_JITTER_SECONDS:-60}"

# Annotation and message go to stderr, so a failure is never swallowed by a
# caller capturing stdout.
die() {
  echo "::error::$1" >&2
  exit 1
}

usage() {
  die "usage: $0 quiesce|service --region <region> --prefix <prefix> [--service <suffix>] [--timeout-seconds <n>] — $1"
}

# ---------------------------------------------------------------------------
# Arguments
# ---------------------------------------------------------------------------
[ $# -ge 1 ] || usage "no mode given"
MODE="$1"; shift
case "${MODE}" in
  quiesce|service) ;;
  *) usage "unknown mode '${MODE}' (must be quiesce or service)" ;;
esac

REGION=""
PREFIX=""
SERVICE_SUFFIX=""
TIMEOUT_SECONDS=""
while [ $# -gt 0 ]; do
  [ $# -ge 2 ] || usage "option '$1' has no value"
  case "$1" in
    --region) REGION="$2" ;;
    --prefix) PREFIX="$2" ;;
    --service) SERVICE_SUFFIX="$2" ;;
    --timeout-seconds) TIMEOUT_SECONDS="$2" ;;
    *) usage "unknown option '$1'" ;;
  esac
  shift 2
done

[ -n "${REGION}" ] || usage "--region is empty (is vars.GCP_DEV_CLOUD_RUN_REGION set?)"
[[ "${REGION}" =~ ^[a-z]+-[a-z]+[0-9]+$ ]] || usage "--region '${REGION}' is not a Cloud Run region name"
[ -n "${PREFIX}" ] || usage "--prefix is empty (is vars.GCP_DEV_SERVICE_NAME set?)"
[[ "${PREFIX}" =~ ^[a-z][a-z0-9-]*$ ]] || usage "--prefix '${PREFIX}' is not a Cloud Run service-name prefix"
TIMEOUT_SECONDS="${TIMEOUT_SECONDS:-600}"
[[ "${TIMEOUT_SECONDS}" =~ ^[1-9][0-9]*$ ]] || usage "--timeout-seconds '${TIMEOUT_SECONDS}' is not a positive integer"
[[ "${POLL_SECONDS}" =~ ^[1-9][0-9]*$ ]] || usage "WAIT_CLOUD_RUN_POLL_SECONDS '${POLL_SECONDS}' is not a positive integer"
[[ "${JITTER_SECONDS}" =~ ^[0-9]+$ ]] || usage "WAIT_CLOUD_RUN_JITTER_SECONDS '${JITTER_SECONDS}' is not a non-negative integer"
[ "${#DEV_SERVICE_SUFFIXES[@]}" -gt 0 ] || die "DEV_SERVICE_SUFFIXES is empty — there is nothing to wait on, which is never right"

if [ "${MODE}" = "service" ]; then
  [ -n "${SERVICE_SUFFIX}" ] || usage "service mode needs --service"
  known=""
  for s in "${DEV_SERVICE_SUFFIXES[@]}"; do
    [ "${s}" = "${SERVICE_SUFFIX}" ] && known="yes"
  done
  [ -n "${known}" ] || die "--service '${SERVICE_SUFFIX}' is not a dev Cloud Run service (${DEV_SERVICE_SUFFIXES[*]}); add it to DEV_SERVICE_SUFFIXES in $0 so every deploy waits on it"
  SERVICES=("${PREFIX}-${SERVICE_SUFFIX}")
else
  [ -z "${SERVICE_SUFFIX}" ] || usage "quiesce mode waits on every dev service; --service is not accepted"
  SERVICES=()
  for s in "${DEV_SERVICE_SUFFIXES[@]}"; do
    SERVICES+=("${PREFIX}-${s}")
  done
fi

for tool in gcloud jq date sleep mktemp tr head; do
  command -v "${tool}" >/dev/null 2>&1 || die "${tool} is not on PATH — cannot read Cloud Run state"
done

WORK=$(mktemp -d)
trap 'rm -rf "${WORK}"' EXIT

# ---------------------------------------------------------------------------
# One service's state, set in SERVICE_STATE rather than printed, so a die here
# exits the script itself and not just a command substitution: "settled <rev>",
# "rolling <why>", "split <traffic>" or "failed <message>".
# ---------------------------------------------------------------------------
service_state() {
  local svc="$1"
  local json="${WORK}/${svc}.json" err="${WORK}/${svc}.err"

  if ! gcloud run services describe "${svc}" --region="${REGION}" --format=json >"${json}" 2>"${err}"; then
    die "gcloud run services describe ${svc} --region=${REGION} failed: $(tr '\n' ' ' <"${err}")"
  fi
  [ -s "${json}" ] || die "gcloud run services describe ${svc} --region=${REGION} returned nothing"
  jq -e 'type == "object" and (.status | type == "object")' "${json}" >/dev/null 2>&1 \
    || die "gcloud run services describe ${svc} --region=${REGION} did not return a service object: $(head -c 300 "${json}" | tr '\n' ' ')"

  local fields
  fields=$(jq -r '
    def ready: [(.status.conditions // [])[] | select(.type == "Ready")][0];
    [ (.metadata.generation // "" | tostring),
      (.status.observedGeneration // "" | tostring),
      (.status.latestCreatedRevisionName // ""),
      (.status.latestReadyRevisionName // ""),
      (ready.status // "Unknown"),
      (ready.message // "" | gsub("[\u001f\n]"; " ")),
      ([(.status.traffic // [])[] | select((.percent // 0) > 0)
        | "\(.revisionName // "?")=\(.percent)"] | join(","))
    ] | join("\u001f")' "${json}") || die "could not parse the state of ${svc}"

  local generation observed created ready condition message traffic
  IFS=$'\x1f' read -r generation observed created ready condition message traffic <<<"${fields}"

  if [ -z "${generation}" ] || [ "${generation}" != "${observed}" ]; then
    SERVICE_STATE="rolling generation ${generation:-?} not yet observed (observed ${observed:-none})"
  elif [ "${condition}" = "False" ]; then
    SERVICE_STATE="failed ${created:-?}: ${message:-no message}"
  elif [ "${condition}" != "True" ]; then
    SERVICE_STATE="rolling Ready=${condition}"
  elif [ -z "${created}" ] || [ "${created}" != "${ready}" ]; then
    SERVICE_STATE="rolling ${created:-?} created, ${ready:-none} ready"
  elif [ "${traffic}" != "${ready}=100" ]; then
    SERVICE_STATE="split traffic ${traffic:-none}, expected ${ready}=100 (repoint it: gcloud run services update-traffic ${svc} --region ${REGION} --to-latest)"
  else
    SERVICE_STATE="settled ${ready}"
  fi
}

# ---------------------------------------------------------------------------
# Poll until done or the deadline
# ---------------------------------------------------------------------------
if [ "${MODE}" = "quiesce" ]; then
  echo "Waiting until no dev Cloud Run service is mid-rollout: ${SERVICES[*]} in ${REGION} (up to ${TIMEOUT_SECONDS}s)"
else
  echo "Waiting for ${SERVICES[0]} in ${REGION} to settle (up to ${TIMEOUT_SECONDS}s)"
fi

DEADLINE=$(( $(date +%s) + TIMEOUT_SECONDS ))
# waited: a poll found something in progress. confirmed: a quiescent poll was
# followed by the jitter sleep, so the next quiescent poll may pass.
waited=""
confirmed=""
while :; do
  done_all="yes"
  summary=""
  warnings=()
  for svc in "${SERVICES[@]}"; do
    service_state "${svc}"
    state="${SERVICE_STATE}"
    summary="${summary} ${svc}: ${state};"
    case "${state}" in
      settled*) ;;
      failed*)
        if [ "${MODE}" = "service" ]; then
          die "${svc} did not become ready — ${state#failed }"
        fi
        warnings+=("${svc} has a failed rollout and nothing in progress (${state#failed }); not blocking on it")
        ;;
      split*)
        if [ "${MODE}" = "service" ]; then
          done_all=""
        else
          warnings+=("${svc} has traffic pinned off its latest revision and nothing in progress (${state#split }); not blocking on it")
        fi
        ;;
      *) done_all="" ;;
    esac
  done
  echo "[$(date -u +%H:%M:%S)]${summary}"

  if [ -n "${done_all}" ] && [ "${MODE}" = "quiesce" ] && [ -n "${waited}" ] && [ -z "${confirmed}" ]; then
    confirmed="yes"
    now=$(date +%s)
    remaining=$(( DEADLINE > now ? DEADLINE - now : 0 ))
    jitter=$(( JITTER_SECONDS > 0 ? RANDOM % (JITTER_SECONDS + 1) : 0 ))
    jitter=$(( jitter < remaining ? jitter : remaining ))
    echo "Quiescent after waiting; re-checking in ${jitter}s so waiters released together do not roll together"
    sleep "${jitter}"
    continue
  fi

  if [ -n "${done_all}" ]; then
    for w in ${warnings[@]+"${warnings[@]}"}; do
      echo "::warning::${w}"
    done
    if [ "${MODE}" = "quiesce" ]; then
      echo "Quiescent: no dev Cloud Run service is mid-rollout"
    else
      echo "Settled: ${SERVICES[0]} serves its latest revision with 100% of traffic"
    fi
    exit 0
  fi

  waited="yes"
  confirmed=""
  now=$(date +%s)
  if [ "${now}" -ge "${DEADLINE}" ]; then
    if [ "${MODE}" = "quiesce" ]; then
      die "dev Cloud Run did not reach quiescence within ${TIMEOUT_SECONDS}s —${summary}"
    fi
    die "${SERVICES[0]} did not settle within ${TIMEOUT_SECONDS}s —${summary}"
  fi
  remaining=$(( DEADLINE - now ))
  sleep $(( remaining < POLL_SECONDS ? remaining : POLL_SECONDS ))
done
