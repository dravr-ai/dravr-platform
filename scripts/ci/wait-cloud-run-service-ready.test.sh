#!/usr/bin/env bash
# ABOUTME: Runs wait-cloud-run-service-ready.sh against a stubbed gcloud, one case per state and per fail-closed path
# ABOUTME: Then checks every dev Cloud Run roll waits for quiescence before and settling after, and seed jobs follow the rolls
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# carnet#696, and carnet#1129 for the database gate. The first version of this harness ran one case and exited 0:
# `(( TESTS_PASSED++ ))` evaluates to 0 on the first pass, which `set -e` treats
# as a failure and aborts on, silently. So the counters here are plain
# assignments, every failure case asserts the script's own error message (an
# exit code alone cannot tell "failed for the right reason" from "crashed"), and
# the run fails unless exactly EXPECTED_CASES cases ran and all passed.
#
# Usage: scripts/ci/wait-cloud-run-service-ready.test.sh — bash, jq, python3 (PyYAML); no network.
# Cloud Monitoring is a stubbed curl as well.

set -euo pipefail
cd "$(dirname "$0")/../.."
SCRIPT="${PWD}/scripts/ci/wait-cloud-run-service-ready.sh"
WORK=$(mktemp -d); trap 'rm -rf "${WORK}"' EXIT

EXPECTED_CASES=45
CASES_RUN=0
CASES_PASSED=0
CASES_FAILED=0

REGION="northamerica-northeast1"
PREFIX="dravr-mcp-server"
ALL=(api frontend photograveur sciotte)
PROJECT="dravr-dev"

# ---------------------------------------------------------------------------
# Stub gcloud. `run services describe <name>` answers from ${STUB_DIR}/<name>.N.json
# on its Nth call (the last file repeats), or <name>.json every call. A
# <name>.missing file makes it fail as gcloud does for an unknown service, and
# <name>.empty makes it print nothing. `config get-value project` prints the
# content of ${STUB_DIR}/project (dravr-dev without one) and `auth
# print-access-token` a fixed token. Every call is logged. Builtins only, so
# it also runs under the trimmed PATH of the missing-tool cases.
# ---------------------------------------------------------------------------
BIN="${WORK}/bin"; mkdir -p "${BIN}"
cat > "${BIN}/gcloud" <<SH
#!${BASH}
SH
cat >> "${BIN}/gcloud" <<'SH'
echo "$*" >> "${STUB_DIR}/calls.log"
if [ "$1 $2 $3" = "config get-value project" ]; then
  if [ -e "${STUB_DIR}/project" ]; then printf '%s\n' "$(<"${STUB_DIR}/project")"; else echo "dravr-dev"; fi
  exit 0
fi
if [ "$1 $2" = "auth print-access-token" ]; then echo "stub-token"; exit 0; fi
if [ "$1 $2 $3" != "run services describe" ]; then
  echo "stub gcloud: unexpected call: $*" >&2; exit 2
fi
name="$4"
if [ -e "${STUB_DIR}/${name}.missing" ]; then
  echo "ERROR: (gcloud.run.services.describe) Cannot find service [${name}]" >&2; exit 1
fi
[ -e "${STUB_DIR}/${name}.empty" ] && exit 0
n=0; [ -e "${STUB_DIR}/${name}.count" ] && n=$(<"${STUB_DIR}/${name}.count")
n=$(( n + 1 )); echo "${n}" > "${STUB_DIR}/${name}.count"
pick=""
for (( i = n; i >= 1; i-- )); do
  if [ -e "${STUB_DIR}/${name}.${i}.json" ]; then pick="${STUB_DIR}/${name}.${i}.json"; break; fi
done
[ -n "${pick}" ] || pick="${STUB_DIR}/${name}.json"
if [ ! -e "${pick}" ]; then echo "stub gcloud: no state for ${name}" >&2; exit 2; fi
printf '%s\n' "$(<"${pick}")"
SH
chmod +x "${BIN}/gcloud"

# Stub curl for the Cloud Monitoring query. Its Nth call answers the body of
# ${STUB_DIR}/monitoring.N.json (the last file repeats) with the HTTP status in
# monitoring.N.code (200 without one), written where -o points and the status
# printed for -w. A monitoring.fail file makes it fail as curl does on a dead
# network. Every call is logged as "curl <args>".
cat > "${BIN}/curl" <<SH
#!${BASH}
SH
cat >> "${BIN}/curl" <<'SH'
echo "curl $*" >> "${STUB_DIR}/calls.log"
if [ -e "${STUB_DIR}/monitoring.fail" ]; then
  echo "curl: (6) Could not resolve host: monitoring.googleapis.com" >&2; exit 6
fi
out=""
while [ $# -gt 0 ]; do
  [ "$1" = "-o" ] && out="$2"
  shift
done
[ -n "${out}" ] || { echo "stub curl: no -o" >&2; exit 2; }
n=0; [ -e "${STUB_DIR}/monitoring.count" ] && n=$(<"${STUB_DIR}/monitoring.count")
n=$(( n + 1 )); echo "${n}" > "${STUB_DIR}/monitoring.count"
pick=""
for (( i = n; i >= 1; i-- )); do
  if [ -e "${STUB_DIR}/monitoring.${i}.json" ]; then pick="${i}"; break; fi
done
[ -n "${pick}" ] || { echo "stub curl: no monitoring state" >&2; exit 2; }
printf '%s\n' "$(<"${STUB_DIR}/monitoring.${pick}.json")" > "${out}"
code=200; [ -e "${STUB_DIR}/monitoring.${pick}.code" ] && code=$(<"${STUB_DIR}/monitoring.${pick}.code")
printf '%s' "${code}"
SH
chmod +x "${BIN}/curl"

# A PATH with every tool the script needs except the named one.
trimmed_path() {
  local without="$1" dir="${WORK}/path-without-${1}" tool src
  mkdir -p "${dir}"
  for tool in gcloud curl jq date sleep mktemp tr head rm; do
    [ "${tool}" = "${without}" ] && continue
    case "${tool}" in
      gcloud|curl) src="${BIN}/${tool}" ;;
      *) src=$(command -v "${tool}") ;;
    esac
    ln -sf "${src}" "${dir}/${tool}"
  done
  echo "${dir}"
}

# svc_json <file> <generation> <observed> <created> <ready> <Ready status> <traffic rev> <percent> [message]
svc_json() {
  jq -n --argjson g "$2" --argjson o "$3" --arg c "$4" --arg r "$5" --arg s "$6" \
        --arg t "$7" --argjson p "$8" --arg m "${9:-}" '
    { metadata: { generation: $g },
      status: { observedGeneration: $o, latestCreatedRevisionName: $c,
                latestReadyRevisionName: $r,
                conditions: [ { type: "Ready", status: $s, message: $m } ],
                traffic: [ { revisionName: $t, percent: $p, latestRevision: true } ] } }' > "$1"
}

settled() { svc_json "${STUB_DIR}/${PREFIX}-$1${2:-}.json" 3 3 "$1-00003" "$1-00003" True "$1-00003" 100; }
rolling() { svc_json "${STUB_DIR}/${PREFIX}-$1${2:-}.json" 4 4 "$1-00004" "$1-00003" True "$1-00003" 100; }

# backends <N> <age seconds> <database>=<count>... — the Nth Monitoring answer:
# one num_backends series per database, its latest point <age> seconds old.
backends() {
  local n="$1" age="$2"; shift 2
  jq -n --argjson age "${age}" --arg project "${PROJECT}" --arg instance "${PREFIX}-postgres" '
    (now - $age | todate) as $t
    | (now - $age - 60 | todate) as $before
    | { timeSeries: [ $ARGS.positional[] | split("=") as [$db, $count]
        | { metric: { type: "cloudsql.googleapis.com/database/postgresql/num_backends",
                      labels: { database: $db } },
            resource: { labels: { database_id: "\($project):\($instance)" } },
            points: [ { interval: { startTime: $t, endTime: $t }, value: { int64Value: $count } },
                      { interval: { startTime: $before, endTime: $before }, value: { int64Value: "0" } } ] } ] }' \
    --args "$@" > "${STUB_DIR}/monitoring.${n}.json"
}

monitoring_calls() { grep -c "^curl " "${STUB_DIR}/calls.log" || true; }
expect_monitoring_calls() {
  local got; got=$(monitoring_calls)
  test "${got}" "$1" "$2" && return 0
  REASON="Monitoring queried ${got} times, expected $1 $2"; return 1
}

new_case() {
  STUB_DIR="${WORK}/state-${CASES_RUN}"
  mkdir -p "${STUB_DIR}"; : > "${STUB_DIR}/calls.log"
  export STUB_DIR
}

# run_script [PATH] -- <args>; sets OUT and CODE
run_with_path() {
  local path="$1"; shift
  set +e
  OUT=$(PATH="${path}" WAIT_CLOUD_RUN_POLL_SECONDS=1 WAIT_CLOUD_RUN_JITTER_SECONDS="${JITTER:-1}" \
        "${BASH}" "${SCRIPT}" "$@" 2>&1)
  CODE=$?
  set -e
}
run_script() { run_with_path "${BIN}:${PATH}" "$@"; }

REASON=""
expect_code() {
  [ "${CODE}" = "$1" ] && return 0
  REASON="exit ${CODE}, expected $1"; return 1
}
expect_out() {
  grep -qF -- "$1" <<<"${OUT}" && return 0
  REASON="output lacks '$1'"; return 1
}
expect_no_out() {
  grep -qF -- "$1" <<<"${OUT}" || return 0
  REASON="output unexpectedly contains '$1'"; return 1
}
expect_calls() {
  local got; got=$(grep -c "services describe $1 " "${STUB_DIR}/calls.log" || true)
  test "${got}" "$2" "$3" && return 0
  REASON="describe $1 called ${got} times, expected $2 $3"; return 1
}

quiesce() { run_script quiesce --region "${REGION}" --prefix "${PREFIX}" "$@"; }
gated() { quiesce --db-headroom 6 "$@"; }
service() { run_script service --region "${REGION}" --prefix "${PREFIX}" "$@"; }

# ---------------------------------------------------------------------------
# Cases
# ---------------------------------------------------------------------------
case_quiesce_all_settled() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  quiesce --timeout-seconds 5
  expect_code 0 && expect_out "Quiescent: no dev Cloud Run service is mid-rollout" && expect_no_out "::warning::"
}

case_quiesce_reads_every_dev_service_in_the_given_region() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  quiesce --timeout-seconds 5
  expect_code 0 || return 1
  for s in "${ALL[@]}"; do expect_calls "${PREFIX}-${s}" -eq 1 || return 1; done
  local lines other
  lines=$(wc -l <"${STUB_DIR}/calls.log" | tr -d ' ')
  [ "${lines}" = "4" ] || { REASON="${lines} gcloud calls, expected 4"; return 1; }
  other=$(grep -vc -- "--region=${REGION} --format=json" "${STUB_DIR}/calls.log" || true)
  [ "${other}" = "0" ] || { REASON="${other} calls without --region=${REGION}"; return 1; }
  # Quiet on the first poll: no jitter, no re-check.
  expect_no_out "re-checking"
}

case_quiesce_waits_for_a_rollout_then_passes() {
  for s in api frontend photograveur; do settled "${s}"; done
  rolling sciotte .1; settled sciotte .2
  quiesce --timeout-seconds 20
  expect_code 0 && expect_out "rolling sciotte-00004 created, sciotte-00003 ready" \
    && expect_out "Quiescent after waiting; re-checking in" && expect_out "Quiescent: no dev Cloud Run" \
    && expect_calls "${PREFIX}-sciotte" -eq 3
}

# A waiter released by one rollout settling re-checks after its jitter, and a
# rollout another waiter started in between sends it back to waiting.
case_quiesce_rechecks_after_the_jitter() {
  for s in frontend photograveur; do settled "${s}"; done
  rolling sciotte .1; settled sciotte .2
  settled api .1; rolling api .3; settled api .4
  quiesce --timeout-seconds 30
  expect_code 0 && expect_out "${PREFIX}-api: rolling api-00004 created" \
    && expect_out "Quiescent: no dev Cloud Run" && expect_calls "${PREFIX}-api" -eq 5
}

case_quiesce_waits_for_an_unobserved_generation() {
  for s in api frontend sciotte; do settled "${s}"; done
  svc_json "${STUB_DIR}/${PREFIX}-photograveur.1.json" 5 4 p-00003 p-00003 True p-00003 100
  settled photograveur .2
  quiesce --timeout-seconds 20
  expect_code 0 && expect_out "rolling generation 5 not yet observed (observed 4)" && expect_out "Quiescent"
}

case_quiesce_waits_while_ready_is_unknown() {
  for s in api frontend sciotte; do settled "${s}"; done
  svc_json "${STUB_DIR}/${PREFIX}-photograveur.1.json" 3 3 p-00003 p-00003 Unknown p-00003 100
  settled photograveur .2
  quiesce --timeout-seconds 20
  expect_code 0 && expect_out "rolling Ready=Unknown" && expect_out "Quiescent"
}

case_quiesce_times_out_on_a_stuck_rollout() {
  for s in api frontend photograveur; do settled "${s}"; done
  rolling sciotte
  quiesce --timeout-seconds 2
  expect_code 1 && expect_out "::error::dev Cloud Run did not reach quiescence within 2s" \
    && expect_out "${PREFIX}-sciotte: rolling sciotte-00004 created" && expect_no_out "Quiescent:"
}

# A pin nothing in flight will clear (deploy-cloudrun never touches traffic):
# blocking on it would fail every deploy, terraform's repair included.
case_quiesce_lets_split_traffic_through_with_a_warning() {
  for s in frontend photograveur sciotte; do settled "${s}"; done
  svc_json "${STUB_DIR}/${PREFIX}-api.json" 3 3 api-00003 api-00003 True api-00002 100
  quiesce --timeout-seconds 5
  expect_code 0 && expect_out "::warning::${PREFIX}-api has traffic pinned off its latest revision and nothing in progress" \
    && expect_out "split traffic api-00002=100, expected api-00003=100" \
    && expect_out "gcloud run services update-traffic ${PREFIX}-api --region ${REGION} --to-latest" \
    && expect_out "Quiescent" && expect_calls "${PREFIX}-api" -eq 1
}

case_quiesce_lets_a_failed_rollout_through_with_a_warning() {
  for s in api frontend sciotte; do settled "${s}"; done
  svc_json "${STUB_DIR}/${PREFIX}-photograveur.json" 3 3 p-00004 p-00003 False p-00003 100 "Revision p-00004 is not ready"
  quiesce --timeout-seconds 5
  expect_code 0 && expect_out "::warning::${PREFIX}-photograveur has a failed rollout and nothing in progress" \
    && expect_out "Revision p-00004 is not ready" && expect_out "Quiescent"
}

case_quiesce_fails_on_a_missing_service() {
  for s in api frontend sciotte; do settled "${s}"; done
  touch "${STUB_DIR}/${PREFIX}-photograveur.missing"
  quiesce --timeout-seconds 30
  expect_code 1 && expect_out "::error::gcloud run services describe ${PREFIX}-photograveur --region=${REGION} failed" \
    && expect_out "Cannot find service [${PREFIX}-photograveur]" && expect_calls "${PREFIX}-photograveur" -eq 1
}

case_quiesce_fails_on_an_empty_answer() {
  for s in frontend photograveur sciotte; do settled "${s}"; done
  touch "${STUB_DIR}/${PREFIX}-api.empty"
  quiesce --timeout-seconds 30
  expect_code 1 && expect_out "::error::gcloud run services describe ${PREFIX}-api --region=${REGION} returned nothing"
}

case_quiesce_fails_on_an_answer_that_is_not_a_service() {
  for s in api photograveur sciotte; do settled "${s}"; done
  echo "Listed 0 items." > "${STUB_DIR}/${PREFIX}-frontend.json"
  quiesce --timeout-seconds 30
  expect_code 1 && expect_out "did not return a service object: Listed 0 items."
}

case_quiesce_fails_on_an_object_without_status() {
  for s in api frontend photograveur; do settled "${s}"; done
  echo '{}' > "${STUB_DIR}/${PREFIX}-sciotte.json"
  quiesce --timeout-seconds 30
  expect_code 1 && expect_out "${PREFIX}-sciotte --region=${REGION} did not return a service object: {}"
}

case_quiesce_rejects_a_service_argument() {
  quiesce --service api
  expect_code 1 && expect_out "quiesce mode waits on every dev service; --service is not accepted"
}

case_service_settled() {
  settled api
  service --service api --timeout-seconds 5
  expect_code 0 && expect_out "Settled: ${PREFIX}-api serves its latest revision" \
    && expect_calls "${PREFIX}-api" -eq 1 && expect_calls "${PREFIX}-frontend" -eq 0
}

case_service_waits_for_its_rollout() {
  rolling frontend .1; rolling frontend .2; settled frontend .3
  service --service frontend --timeout-seconds 20
  expect_code 0 && expect_out "rolling frontend-00004 created" && expect_out "Settled" \
    && expect_calls "${PREFIX}-frontend" -eq 3
}

case_service_fails_on_its_failed_rollout() {
  svc_json "${STUB_DIR}/${PREFIX}-api.json" 4 4 api-00004 api-00003 False api-00003 100 "Container failed to start"
  service --service api --timeout-seconds 30
  expect_code 1 && expect_out "::error::${PREFIX}-api did not become ready — api-00004: Container failed to start"
}

case_service_times_out() {
  rolling api
  service --service api --timeout-seconds 2
  expect_code 1 && expect_out "::error::${PREFIX}-api did not settle within 2s"
}

# Our own deploy landing without traffic shipped nothing: never settled.
case_service_times_out_on_split_traffic_naming_the_remedy() {
  svc_json "${STUB_DIR}/${PREFIX}-sciotte.json" 3 3 sciotte-00003 sciotte-00003 True sciotte-00002 100
  service --service sciotte --timeout-seconds 2
  expect_code 1 && expect_out "::error::${PREFIX}-sciotte did not settle within 2s" \
    && expect_out "gcloud run services update-traffic ${PREFIX}-sciotte --region ${REGION} --to-latest"
}

case_service_rejects_an_unknown_suffix() {
  settled api
  service --service billing
  expect_code 1 && expect_out "--service 'billing' is not a dev Cloud Run service (api frontend photograveur sciotte)" \
    && expect_calls "${PREFIX}-billing" -eq 0
}

case_service_requires_a_service() {
  service
  expect_code 1 && expect_out "service mode needs --service"
}

case_rejects_an_empty_region() {
  run_script quiesce --region "" --prefix "${PREFIX}"
  expect_code 1 && expect_out "--region is empty (is vars.GCP_DEV_CLOUD_RUN_REGION set?)"
}

case_rejects_a_missing_prefix() {
  run_script quiesce --region "${REGION}"
  expect_code 1 && expect_out "--prefix is empty (is vars.GCP_DEV_SERVICE_NAME set?)"
}

case_rejects_an_unknown_mode() {
  run_script "${PREFIX}-api" --region "${REGION}" --prefix "${PREFIX}"
  expect_code 1 && expect_out "unknown mode '${PREFIX}-api' (must be quiesce or service)"
}

case_rejects_a_bad_timeout() {
  quiesce --timeout-seconds 0
  expect_code 1 && expect_out "--timeout-seconds '0' is not a positive integer"
}

case_rejects_a_bad_jitter() {
  JITTER=-1 quiesce
  expect_code 1 && expect_out "WAIT_CLOUD_RUN_JITTER_SECONDS '-1' is not a non-negative integer"
}

case_fails_without_jq() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  run_with_path "$(trimmed_path jq)" quiesce --region "${REGION}" --prefix "${PREFIX}"
  expect_code 1 && expect_out "::error::jq is not on PATH" && expect_calls "${PREFIX}-api" -eq 0
}

case_fails_without_gcloud() {
  run_with_path "$(trimmed_path gcloud)" quiesce --region "${REGION}" --prefix "${PREFIX}"
  expect_code 1 && expect_out "::error::gcloud is not on PATH"
}

# ---------------------------------------------------------------------------
# Database gate (carnet#1129)
# ---------------------------------------------------------------------------
case_db_passes_with_headroom() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  backends 1 60 dravr=10 postgres=1 template1=0
  gated --timeout-seconds 5
  expect_code 0 && expect_out "Database headroom: 11 of 22 slots in use (dravr=10,postgres=1,template1=0), 11 free, need 6" \
    && expect_out "${PROJECT}:${PREFIX}-postgres has 6 of its 22 connection slots free" \
    && expect_monitoring_calls -eq 1 && expect_no_out "re-checking" || return 1
  grep -qF "resource.labels.database_id=\"${PROJECT}:${PREFIX}-postgres\"" "${STUB_DIR}/calls.log" \
    || { REASON="the query does not filter on ${PROJECT}:${PREFIX}-postgres"; return 1; }
  grep -qF "monitoring.googleapis.com/v3/projects/${PROJECT}/timeSeries" "${STUB_DIR}/calls.log" \
    || { REASON="the query does not read projects/${PROJECT}/timeSeries"; return 1; }
}

case_db_waits_while_saturated_then_passes() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  backends 1 60 dravr=18 postgres=0; backends 2 60 dravr=12 postgres=0
  gated --timeout-seconds 20
  expect_code 0 && expect_out "db: 18 of 22 slots in use (dravr=18,postgres=0), 4 free, need 6" \
    && expect_out "Quiescent after waiting; re-checking in" \
    && expect_out "Database headroom: 12 of 22 slots in use" && expect_monitoring_calls -eq 3
}

# The jitter re-check reads the database again: slots another roll took while
# this waiter slept send it back to waiting.
case_db_rechecks_after_the_jitter() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  backends 1 60 dravr=18; backends 2 60 dravr=6; backends 3 60 dravr=20; backends 4 60 dravr=6
  gated --timeout-seconds 30
  expect_code 0 && expect_out "db: 20 of 22 slots in use (dravr=20), 2 free, need 6" \
    && expect_out "Database headroom: 6 of 22" && expect_monitoring_calls -eq 5
}

case_db_is_read_only_once_cloud_run_is_quiet() {
  for s in api frontend photograveur; do settled "${s}"; done
  rolling sciotte .1; settled sciotte .2
  backends 1 60 dravr=6
  gated --timeout-seconds 20
  expect_code 0 && expect_calls "${PREFIX}-sciotte" -eq 3 && expect_monitoring_calls -eq 2
}

case_db_excludes_cloudsqladmin() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  backends 1 60 dravr=16 cloudsqladmin=2 postgres=0
  gated --timeout-seconds 5
  expect_code 0 && expect_out "Database headroom: 16 of 22 slots in use (dravr=16,postgres=0), 6 free, need 6" \
    && expect_no_out "cloudsqladmin"
}

case_db_waits_on_a_stale_point_then_times_out() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  backends 1 900 dravr=2
  gated --timeout-seconds 2
  expect_code 1 && expect_out "db: latest num_backends point older than 300s (dravr 9" \
    && expect_out "::error::dev Cloud Run did not reach quiescence with 6 free Cloud SQL slots within 2s" \
    && expect_no_out "Quiescent:"
}

case_db_times_out_naming_the_count_and_the_headroom() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  backends 1 60 dravr=18 postgres=1
  gated --timeout-seconds 2
  expect_code 1 && expect_out "::error::dev Cloud Run did not reach quiescence with 6 free Cloud SQL slots within 2s" \
    && expect_out "db: 19 of 22 slots in use (dravr=18,postgres=1), 3 free, need 6;"
}

case_db_waits_on_no_series() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  echo '{}' > "${STUB_DIR}/monitoring.1.json"
  backends 2 60 dravr=6
  gated --timeout-seconds 20
  expect_code 0 && expect_out "db: no num_backends series for ${PROJECT}:${PREFIX}-postgres" \
    && expect_out "Database headroom: 6 of 22"
}

case_db_fails_at_once_on_403_naming_monitoring_viewer() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  echo '{"error":{"code":403,"message":"Permission monitoring.timeSeries.list denied"}}' > "${STUB_DIR}/monitoring.1.json"
  echo 403 > "${STUB_DIR}/monitoring.1.code"
  gated --timeout-seconds 30
  expect_code 1 && expect_out "::error::Cloud Monitoring refused the num_backends query for ${PROJECT} (HTTP 403): the deployer service account needs roles/monitoring.viewer" \
    && expect_out "Permission monitoring.timeSeries.list denied" && expect_monitoring_calls -eq 1
}

case_db_fails_at_once_on_an_http_error() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  echo '{"error":{"code":500}}' > "${STUB_DIR}/monitoring.1.json"
  echo 500 > "${STUB_DIR}/monitoring.1.code"
  gated --timeout-seconds 30
  expect_code 1 && expect_out "::error::Cloud Monitoring answered HTTP 500" && expect_monitoring_calls -eq 1
}

case_db_fails_at_once_on_non_json() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  echo '<html>upstream connect error</html>' > "${STUB_DIR}/monitoring.1.json"
  gated --timeout-seconds 30
  expect_code 1 && expect_out "::error::Cloud Monitoring did not return a JSON object for the num_backends query: <html>upstream" \
    && expect_monitoring_calls -eq 1
}

case_db_fails_at_once_when_curl_fails() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  touch "${STUB_DIR}/monitoring.fail"
  gated --timeout-seconds 30
  expect_code 1 && expect_out "::error::the Cloud Monitoring num_backends query for ${PROJECT}:${PREFIX}-postgres failed: curl: (6)" \
    && expect_monitoring_calls -eq 1
}

case_db_fails_without_a_project() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  echo "(unset)" > "${STUB_DIR}/project"
  gated --timeout-seconds 30
  expect_code 1 && expect_out "::error::no gcloud project is set" && expect_monitoring_calls -eq 0 \
    && expect_calls "${PREFIX}-api" -eq 0
}

case_db_fails_without_curl() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  run_with_path "$(trimmed_path curl)" quiesce --region "${REGION}" --prefix "${PREFIX}" --db-headroom 6
  expect_code 1 && expect_out "::error::curl is not on PATH" && expect_calls "${PREFIX}-api" -eq 0
}

# Without the flag nothing about the database is touched: no project, no token,
# no Monitoring call, and curl is not even required.
case_db_ungated_quiesce_never_reads_the_database() {
  for s in "${ALL[@]}"; do settled "${s}"; done
  backends 1 60 dravr=22
  run_with_path "$(trimmed_path curl)" quiesce --region "${REGION}" --prefix "${PREFIX}" --timeout-seconds 5
  expect_code 0 && expect_out "Quiescent: no dev Cloud Run" && expect_no_out "Database headroom" \
    && expect_monitoring_calls -eq 0 || return 1
  if grep -qE "^(config|auth) " "${STUB_DIR}/calls.log"; then
    REASON="gcloud config/auth called without --db-headroom"; return 1
  fi
}

case_db_rejects_headroom_in_service_mode() {
  service --service api --db-headroom 6
  expect_code 1 && expect_out "--db-headroom gates the next roll, so it belongs to quiesce mode"
}

case_db_rejects_headroom_beyond_the_slot_limit() {
  quiesce --db-headroom 23
  expect_code 1 && expect_out "--db-headroom 23 exceeds the 22 slots dev Cloud SQL has" || return 1
  quiesce --db-headroom 0
  expect_code 1 && expect_out "--db-headroom '0' is not a positive integer"
}

CASES=(
  case_quiesce_all_settled
  case_quiesce_reads_every_dev_service_in_the_given_region
  case_quiesce_waits_for_a_rollout_then_passes
  case_quiesce_rechecks_after_the_jitter
  case_quiesce_waits_for_an_unobserved_generation
  case_quiesce_waits_while_ready_is_unknown
  case_quiesce_times_out_on_a_stuck_rollout
  case_quiesce_lets_split_traffic_through_with_a_warning
  case_quiesce_lets_a_failed_rollout_through_with_a_warning
  case_quiesce_fails_on_a_missing_service
  case_quiesce_fails_on_an_empty_answer
  case_quiesce_fails_on_an_answer_that_is_not_a_service
  case_quiesce_fails_on_an_object_without_status
  case_quiesce_rejects_a_service_argument
  case_service_settled
  case_service_waits_for_its_rollout
  case_service_fails_on_its_failed_rollout
  case_service_times_out
  case_service_times_out_on_split_traffic_naming_the_remedy
  case_service_rejects_an_unknown_suffix
  case_service_requires_a_service
  case_rejects_an_empty_region
  case_rejects_a_missing_prefix
  case_rejects_an_unknown_mode
  case_rejects_a_bad_timeout
  case_rejects_a_bad_jitter
  case_fails_without_jq
  case_fails_without_gcloud
  case_db_passes_with_headroom
  case_db_waits_while_saturated_then_passes
  case_db_rechecks_after_the_jitter
  case_db_is_read_only_once_cloud_run_is_quiet
  case_db_excludes_cloudsqladmin
  case_db_waits_on_a_stale_point_then_times_out
  case_db_times_out_naming_the_count_and_the_headroom
  case_db_waits_on_no_series
  case_db_fails_at_once_on_403_naming_monitoring_viewer
  case_db_fails_at_once_on_an_http_error
  case_db_fails_at_once_on_non_json
  case_db_fails_at_once_when_curl_fails
  case_db_fails_without_a_project
  case_db_fails_without_curl
  case_db_ungated_quiesce_never_reads_the_database
  case_db_rejects_headroom_in_service_mode
  case_db_rejects_headroom_beyond_the_slot_limit
)

echo "wait-cloud-run-service-ready: script"
for c in "${CASES[@]}"; do
  new_case
  CASES_RUN=$(( CASES_RUN + 1 ))
  REASON=""
  if "${c}"; then
    CASES_PASSED=$(( CASES_PASSED + 1 ))
    printf '  ok   %s\n' "${c#case_}"
  else
    CASES_FAILED=$(( CASES_FAILED + 1 ))
    printf '  FAIL %s — %s\n' "${c#case_}" "${REASON:-assertion failed}"
    printf '%s\n' "${OUT:-}" | sed 's/^/       | /'
  fi
done

# ---------------------------------------------------------------------------
# Workflows: every roll of a dev Cloud Run revision runs the quiesce wait after
# the previous roll of its job (or inline before it, for a multi-command step
# like the satellite rollback) and a wait after it; every job that updates or
# executes a dev Cloud Run job needs every roll job of its workflow; every wait
# takes region and prefix from the dev vars; the quiesce before an API
# deploy-cloudrun passes --db-headroom and the one before any other service's
# does not (carnet#1129); and checkout runs before auth
# (actions/checkout cleans the workspace, deleting the credentials file
# google-github-actions/auth wrote). The scan then runs against broken copies
# of the workflows and must fail on each, so a scan that checks nothing cannot
# pass.
# ---------------------------------------------------------------------------
echo "wait-cloud-run-service-ready: workflows"
WORKFLOW_CHECKS=0
if WF_OUT=$(python3 - <<'PY' 2>&1
import copy, glob, os, re, sys, yaml

WAIT = "scripts/ci/wait-cloud-run-service-ready.sh"
ROLL_RUN = re.compile(r"gcloud\s+run\s+(deploy|services\s+(update|replace|update-traffic))\b|terraform\s+(-chdir=\S+\s+)?apply\b")
JOB_RUN = re.compile(r"gcloud\s+run\s+jobs\s+(execute|update)\b")
# Every job that rolls a dev revision today. The scan must find each one, so a
# broken scan fails instead of passing on nothing.
KNOWN = {
    "publish-images.yml:deploy-dev",
    "deploy-gcp.yml:deploy-api",
    "deploy-gcp.yml:deploy-frontend",
    "fasttrack-deploy.yml:publish-and-deploy",
    "deploy-satellite-cloud-run.yml:deploy",
    "terraform.yml:apply-dev",
}

def text(step):
    return yaml.safe_dump({k: step.get(k) for k in ("uses", "with", "run", "env")})

def is_roll(step):
    uses = step.get("uses") or ""
    return uses.startswith("google-github-actions/deploy-cloudrun") or bool(ROLL_RUN.search(step.get("run") or ""))

def wait_re(mode):
    # An invocation, not a mention: the path followed by a mode.
    return re.compile(rf"{re.escape(WAIT)}\"?\s+{mode}\b")

def waits(step, mode):
    return wait_re(mode).search(step.get("run") or "") is not None

def needs_of(job):
    n = job.get("needs") or []
    return [n] if isinstance(n, str) else list(n)

def ancestors(jobs, job_id):
    seen, todo = set(), needs_of(jobs[job_id])
    while todo:
        j = todo.pop()
        if j in seen or j not in jobs:
            continue
        seen.add(j)
        todo.extend(needs_of(jobs[j]))
    return seen

def scan(workflows):
    problems, found = [], set()
    for name, wf in sorted(workflows.items()):
        jobs = wf.get("jobs") or {}
        roll_jobs = set()
        for job_id, job in jobs.items():
            steps = job.get("steps") or []
            dev = job.get("environment") == "development" or any("GCP_DEV" in text(s) for s in steps if is_roll(s))
            if job.get("environment") == "production":
                dev = False
            rolls = [i for i, s in enumerate(steps) if is_roll(s)]
            uses_wait = [i for i, s in enumerate(steps) if waits(s, "(quiesce|service)")]
            where = f"{name}:{job_id}"
            if rolls and dev:
                found.add(where)
                roll_jobs.add(job_id)
                prev = -1
                for r in rolls:
                    run = steps[r].get("run") or ""
                    m = ROLL_RUN.search(run)
                    roll_at = m.start() if m else None
                    # Before this roll and after the previous one: a quiesce
                    # step in between, or one inline ahead of the roll command.
                    inline_before = roll_at is not None and any(w.start() < roll_at for w in wait_re("quiesce").finditer(run))
                    guards = [i for i in uses_wait if waits(steps[i], "quiesce") and prev < i < r]
                    if not inline_before and not guards:
                        problems.append(f"{where}: no quiesce wait between the previous roll and '{steps[r].get('name')}'")
                    # Only the API holds Cloud SQL connections, so only its roll
                    # waits for free slots; a frontend redeploy never does.
                    if guards and (steps[r].get("uses") or "").startswith("google-github-actions/deploy-cloudrun"):
                        api = str((steps[r].get("with") or {}).get("service") or "").endswith("-api")
                        gated = "--db-headroom" in (steps[guards[-1]].get("run") or "")
                        if api and not gated:
                            problems.append(f"{where}: the quiesce before '{steps[r].get('name')}' lacks --db-headroom; an API roll waits for Cloud SQL slots")
                        if gated and not api:
                            problems.append(f"{where}: the quiesce before '{steps[r].get('name')}' passes --db-headroom; only an API roll waits on Cloud SQL")
                    inline_after = roll_at is not None and run.rfind(WAIT) > roll_at
                    if not inline_after and not [i for i in uses_wait if i > r]:
                        problems.append(f"{where}: nothing waits for '{steps[r].get('name')}' to settle")
                    prev = r
            for i in uses_wait:
                t = text(steps[i])
                for var in ("vars.GCP_DEV_CLOUD_RUN_REGION", "vars.GCP_DEV_SERVICE_NAME"):
                    if var not in t:
                        problems.append(f"{where}: '{steps[i].get('name')}' does not take {var}")
                if "${{" in (steps[i].get("run") or ""):
                    problems.append(f"{where}: '{steps[i].get('name')}' interpolates an expression into its script; pass it through env")
            if uses_wait:
                checkout = [i for i, s in enumerate(steps) if (s.get("uses") or "").startswith("actions/checkout")]
                auth = [i for i, s in enumerate(steps) if (s.get("uses") or "").startswith("google-github-actions/auth")]
                if not checkout or (auth and checkout[0] > auth[0]):
                    problems.append(f"{where}: actions/checkout must run before google-github-actions/auth (it deletes the credentials file)")
        # A Cloud Run job execution opens Cloud SQL connections as a cold start
        # does, and its image must not be swapped mid-roll: a job that updates
        # or executes one runs after every roll of its workflow, never beside.
        for job_id, job in jobs.items():
            if job.get("environment") == "production" or job_id in roll_jobs:
                continue
            if any(JOB_RUN.search(s.get("run") or "") for s in job.get("steps") or []):
                missing = sorted(roll_jobs - ancestors(jobs, job_id))
                if missing:
                    problems.append(f"{name}:{job_id}: runs Cloud Run jobs beside the roll of {' '.join(missing)}; it must need them")
    return problems, found

workflows = {os.path.basename(p): yaml.safe_load(open(p)) for p in sorted(glob.glob(".github/workflows/*.yml"))}
problems, found = scan(workflows)
for k in sorted(KNOWN - found):
    problems.append(f"{k}: known dev roll not found by the scan — the scan or the job moved")

# Broken copies: each must make the scan fail, naming what it broke.
def drop_step(wf, job, name):
    steps = wf["jobs"][job]["steps"]
    kept = [s for s in steps if s.get("name") != name]
    assert len(kept) == len(steps) - 1, f"no step '{name}' in {job}"
    wf["jobs"][job]["steps"] = kept

def strip_inline_quiesce(wf, job, name):
    step = next(s for s in wf["jobs"][job]["steps"] if s.get("name") == name)
    run = step["run"]
    step["run"] = "\n".join(l for l in run.splitlines() if not wait_re("quiesce").search(l))
    assert step["run"] != run, f"no inline quiesce in '{name}'"

def edit_run(wf, job, name, old, new):
    step = next(s for s in wf["jobs"][job]["steps"] if s.get("name") == name)
    assert old in step["run"], f"no '{old}' in '{name}'"
    step["run"] = step["run"].replace(old, new)

def set_needs(wf, job, needs):
    assert job in wf["jobs"], f"no job {job}"
    wf["jobs"][job]["needs"] = needs

BEFORE_FRONTEND = "Wait until no dev Cloud Run service is mid-rollout (before the frontend)"
BEFORE_API = "Wait until no dev Cloud Run service is mid-rollout (before the API)"
BROKEN = [
    ("publish-images.yml", "publish-images.yml:deploy-dev: no quiesce wait between the previous roll and 'Deploy frontend",
     lambda wf: drop_step(wf, "deploy-dev", BEFORE_FRONTEND)),
    ("fasttrack-deploy.yml", "fasttrack-deploy.yml:publish-and-deploy: no quiesce wait between the previous roll and 'Deploy frontend",
     lambda wf: drop_step(wf, "publish-and-deploy", BEFORE_FRONTEND)),
    ("deploy-satellite-cloud-run.yml", "deploy-satellite-cloud-run.yml:deploy: no quiesce wait between the previous roll and 'Roll back",
     lambda wf: strip_inline_quiesce(wf, "deploy", "Roll back to the previously serving digest")),
    ("publish-images.yml", "publish-images.yml:deploy-dev: the quiesce before 'Deploy backend to Cloud Run (dev)' lacks --db-headroom",
     lambda wf: edit_run(wf, "deploy-dev", BEFORE_API, " --db-headroom 6", "")),
    ("fasttrack-deploy.yml", "fasttrack-deploy.yml:publish-and-deploy: the quiesce before 'Deploy frontend to Cloud Run (dev)' passes --db-headroom",
     lambda wf: edit_run(wf, "publish-and-deploy", BEFORE_FRONTEND, " --timeout-seconds", " --db-headroom 6 --timeout-seconds")),
    ("deploy-gcp.yml", "deploy-gcp.yml:update-seed-jobs: runs Cloud Run jobs beside the roll of deploy-api deploy-frontend",
     lambda wf: set_needs(wf, "update-seed-jobs", "resolve-images")),
]
for wf_name, expected, breaker in BROKEN:
    broken = copy.deepcopy(workflows)
    try:
        breaker(broken[wf_name])
    except (AssertionError, KeyError, StopIteration) as e:
        problems.append(f"self-check could not break {wf_name}: {e!r}")
        continue
    got, _ = scan(broken)
    if not any(p.startswith(expected) for p in got):
        problems.append(f"self-check: a broken {wf_name} passed the scan (expected '{expected}', got {got})")

for p in problems:
    print(p)
print(f"dev roll jobs: {' '.join(sorted(found))}; {len(BROKEN)} broken copies rejected")
sys.exit(1 if problems else 0)
PY
); then
  WORKFLOW_CHECKS=1
  printf '  ok   %s\n' "$(tail -1 <<<"${WF_OUT}")"
else
  printf '  FAIL workflow wiring\n'
  printf '%s\n' "${WF_OUT}" | sed 's/^/       | /'
fi

echo
echo "${CASES_PASSED} passed, ${CASES_FAILED} failed (${CASES_RUN} of ${EXPECTED_CASES} cases ran); workflow wiring: $([ "${WORKFLOW_CHECKS}" = 1 ] && echo ok || echo FAIL)"
if [ "${CASES_RUN}" -ne "${EXPECTED_CASES}" ]; then
  echo "expected ${EXPECTED_CASES} cases to run, ${CASES_RUN} did — update EXPECTED_CASES with the case list, never past it"
  exit 1
fi
[ "${CASES_FAILED}" -eq 0 ] && [ "${CASES_PASSED}" -eq "${EXPECTED_CASES}" ] && [ "${WORKFLOW_CHECKS}" = 1 ]
