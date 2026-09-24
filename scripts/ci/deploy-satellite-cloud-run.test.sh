#!/usr/bin/env bash
# ABOUTME: Pins deploy-satellite-cloud-run.yml, the ship tail every satellite lane with a Cloud Run service shares
# ABOUTME: Checks each caller's inputs against the declaration, then runs the lifted steps against stubbed gcloud/curl/gh
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The tail is one file called from several lanes, so two things can break it
# silently. A caller passing an input the workflow does not declare, or leaving
# out a required one, fails only when the lane runs — and a lane runs on a
# satellite release, days after the edit. And a service-name input wired to the
# wrong place deploys fine and then fails the digest assertion, which rolls dev
# back over a correct rollout. The text run here is the text the runner executes;
# a renamed step or an expression moved into a script fails the lift.
#
# Usage: scripts/ci/deploy-satellite-cloud-run.test.sh — bash, python3 (PyYAML) and jq, no network.

set -euo pipefail
cd "$(dirname "$0")/../.."
WORKFLOW=".github/workflows/deploy-satellite-cloud-run.yml"
WORK=$(mktemp -d); trap 'rm -rf "${WORK}"' EXIT
FAILURES=0
pass() { printf '  ok   %s\n' "$1"; }
fail() { printf '  FAIL %s\n' "$1"; FAILURES=$(( FAILURES + 1 )); }

echo "deploy-satellite-cloud-run: callers"
# Every caller passes only declared inputs and every required one; a lane with a
# service is a caller, so finding none means the scan broke, not that all is well.
if CALLERS=$(python3 - "${WORKFLOW}" "${WORK}" <<'PY'
import glob, os, re, sys, yaml
wf_path, work = sys.argv[1], sys.argv[2]
wf = yaml.safe_load(open(wf_path))
inputs = (wf.get(True) or wf["on"])["workflow_call"]["inputs"]  # PyYAML 1.1 reads a bare `on:` as True
required = {k for k, v in inputs.items() if v.get("required")}
problems, callers = [], []
for path in sorted(glob.glob(".github/workflows/*.yml")):
    for job_id, job in (yaml.safe_load(open(path)).get("jobs") or {}).items():
        if job.get("uses") != f"./{wf_path}":
            continue
        callers.append(f"{os.path.basename(path)}:{job_id}")
        given = set((job.get("with") or {}).keys())
        for k in sorted(given - set(inputs)):
            problems.append(f"{path} job {job_id} passes undeclared input '{k}'")
        for k in sorted(required - given):
            problems.append(f"{path} job {job_id} omits required input '{k}'")
        if job.get("secrets") != "inherit":
            problems.append(f"{path} job {job_id} must pass `secrets: inherit` (the deploy and tag jobs read GCP secrets)")
        if "needs.bump.outputs.state == 'pushed'" not in str((job.get("with") or {}).get("ship_platform")):
            problems.append(f"{path} job {job_id} must ship the platform only when the spine pushed a bump")
# Each lifted step must be expression-free, so the test runs what the runner runs.
steps = {s.get("name"): s for j in wf["jobs"].values() for s in j["steps"] if s.get("name")}
lift = {"assert.sh": "Assert the serving bytes are the bytes we built and scanned",
        "probe.sh": "Probe the service over HTTP",
        "dispatch.sh": "Dispatch the platform image build and deploy"}
for out, name in lift.items():
    run = (steps.get(name) or {}).get("run")
    if not run:
        problems.append(f"no step named '{name}' with a run block in {wf_path}")
        continue
    if "${{" in run:
        problems.append(f"step '{name}' interpolates an expression into its script; move it to env")
    open(os.path.join(work, out), "w").write(run)
for p in problems:
    print(p, file=sys.stderr)
print(" ".join(callers))
sys.exit(1 if problems else 0)
PY
); then
  if [ -n "${CALLERS}" ]; then pass "callers match the declared inputs: ${CALLERS}"
  else fail "no workflow calls ${WORKFLOW} — the scan found nothing to check"; fi
else
  fail "a caller or the workflow itself is malformed (see above)"
fi
[ -s "${WORK}/assert.sh" ] && [ -s "${WORK}/probe.sh" ] && [ -s "${WORK}/dispatch.sh" ] \
  || { echo "could not lift the steps out of ${WORKFLOW}"; exit 1; }

# ---------------------------------------------------------------------------
# Stand-ins. gcloud answers the describe calls from env; curl serves the
# registry index and the /health body and logs every URL it was asked for; gh
# logs its arguments.
# ---------------------------------------------------------------------------
BIN="${WORK}/bin"; mkdir -p "${BIN}"
cat > "${BIN}/gcloud" <<'SH'
#!/usr/bin/env bash
case "$*" in
  "run services describe "*"status.latestReadyRevisionName"*) echo "${4}-00042-abc" ;;
  "run services describe "*"status.url"*) echo "https://${4}.example.invalid" ;;
  "run revisions describe "*"status.imageDigest"*) echo "${SERVING}" ;;
  "auth print-access-token") echo "ya29.stub" ;;
  *) echo "unexpected gcloud call: $*" >&2; exit 2 ;;
esac
SH
cat > "${BIN}/curl" <<'SH'
#!/usr/bin/env bash
out=""; url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift ;;
    -H|-w|-m|--retry|--retry-delay) shift ;;
    https://*) url="$1" ;;
  esac
  shift
done
echo "${url}" >> "${CURL_LOG}"
case "${url}" in
  */manifests/*) printf '%s' "${INDEX_JSON}" ;;
  */health) printf '%s' "${HEALTH_BODY}" > "${out}"; printf '%s' "${HEALTH_CODE}" ;;
  *) echo "unexpected curl url: ${url}" >&2; exit 2 ;;
esac
SH
cat > "${BIN}/gh" <<'SH'
#!/usr/bin/env bash
echo "$*" >> "${GH_LOG}"
SH
chmod +x "${BIN}"/*

INDEX="sha256:1111111111111111111111111111111111111111111111111111111111111111"
CHILD="sha256:2222222222222222222222222222222222222222222222222222222222222222"
ATTEST="sha256:3333333333333333333333333333333333333333333333333333333333333333"
FOREIGN="sha256:9999999999999999999999999999999999999999999999999999999999999999"
REGISTRY_URL="europe-docker.pkg.dev/dravr-artifacts/images"
INDEX_JSON=$(jq -cn --arg a "${CHILD}" --arg b "${ATTEST}" '{manifests: [{digest: $a}, {digest: $b}]}')

run_step() {  # run_step <script> <env assignments...>
  local script="$1"; shift
  CURL_LOG="${WORK}/curl.log"; GH_LOG="${WORK}/gh.log"; : > "${CURL_LOG}"; : > "${GH_LOG}"
  set +e
  OUT=$(env PATH="${BIN}:${PATH}" CURL_LOG="${CURL_LOG}" GH_LOG="${GH_LOG}" INDEX_JSON="${INDEX_JSON}" \
        REGION="northamerica-northeast1" "$@" bash "${WORK}/${script}" 2>&1)
  RC=$?
  set -e
}
check() {  # check <name> <expected rc> <expected output fragment>
  if [ "${RC}" = "$2" ] && grep -qF -- "$3" <<<"${OUT}"; then pass "$1"
  else fail "$1 (rc=${RC}, want $2; output did not contain '$3')"; printf '%s\n' "${OUT}" | tail -3 | sed 's/^/       /'; fi
}

echo "deploy-satellite-cloud-run: digest assertion"
for svc in sciotte photograveur; do
  run_step assert.sh SVC="dravr-dev-${svc}" REGISTRY="${REGISTRY_URL}" IMAGE_NAME="${svc}" \
    EXPECTED="${INDEX}" SERVING="${REGISTRY_URL}/${svc}@${CHILD}"
  check "${svc}: Cloud Run records the amd64 child of the built index — accepted" 0 "(matches ${CHILD})"
  if grep -qxF "https://europe-docker.pkg.dev/v2/dravr-artifacts/images/${svc}/manifests/${INDEX}" "${CURL_LOG}"; then
    pass "  ...and the index was read from ${svc}'s own repository"
  else
    fail "  ...the index was read from $(cat "${CURL_LOG}"), not ${svc}'s repository"
  fi
done
run_step assert.sh SVC="dravr-dev-sciotte" REGISTRY="${REGISTRY_URL}" IMAGE_NAME="sciotte" \
  EXPECTED="${INDEX}" SERVING="${INDEX}"
check "the index digest itself is accepted" 0 "(matches ${INDEX})"
run_step assert.sh SVC="dravr-dev-sciotte" REGISTRY="${REGISTRY_URL}" IMAGE_NAME="sciotte" \
  EXPECTED="${INDEX}" SERVING="${REGISTRY_URL}/sciotte@${FOREIGN}"
check "a digest the build never produced fails, so the rollback runs" 1 "which is neither ${INDEX} nor any manifest it indexes"

echo "deploy-satellite-cloud-run: HTTP probe"
probe() {  # probe <providers> <http code> <body>
  run_step probe.sh SVC="dravr-dev-sciotte" ID_TOKEN="stub-id-token" PROVIDERS="$1" \
    HEALTH_CODE="$2" HEALTH_BODY="$3"
}
probe "garmin strava" 200 '{"status":"ok","providers":["garmin","strava"]}'
check "ok and every named provider advertised passes" 0 'advertises ["garmin","strava"]'
probe "garmin strava" 200 '{"status":"ok","providers":["garmin"]}'
check "a build that lost a named provider fails, naming it" 1 "does not advertise the strava provider"
probe "garmin strava" 200 '{"status":"degraded","providers":["garmin","strava"]}'
check "a status other than ok fails" 1 "/health reports status=degraded"
probe "garmin strava" 401 '{"error":"audience"}'
check "an identity-token refusal is named as auth, never as a sick container" 1 "returned HTTP 401"

echo "deploy-satellite-cloud-run: platform dispatch"
SUMMARY_FILE="${WORK}/summary.md"; : > "${SUMMARY_FILE}"
run_step dispatch.sh GITHUB_STEP_SUMMARY="${SUMMARY_FILE}" RELEASE="enforme v0.5.1" \
  SUMMARY=$'The sciotte service is on **v0.9.0**;\nrebuilding to match.'
check "the dispatch log names the release the binary picks up" 0 "so the binary picks up enforme v0.5.1"
if [ "$(cat "${GH_LOG}")" = "workflow run publish-images.yml --ref main" ]; then
  pass "  ...and dispatches publish-images.yml on main"
else
  fail "  ...gh was called with '$(cat "${GH_LOG}")'"
fi
WANT=$'### Platform deploy dispatched\n\nThe sciotte service is on **v0.9.0**;\nrebuilding to match.'
if [ "$(cat "${SUMMARY_FILE}")" = "${WANT}" ]; then
  pass "  ...and writes the caller's summary under the heading, line breaks intact"
else
  fail "  ...the step summary was: $(cat "${SUMMARY_FILE}")"
fi

echo
[ "${FAILURES}" -eq 0 ] && { echo "all deploy-satellite-cloud-run scenarios pass"; exit 0; }
echo "${FAILURES} scenario(s) failed"; exit 1
