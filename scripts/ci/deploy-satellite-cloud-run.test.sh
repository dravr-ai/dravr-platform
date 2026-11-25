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
# A caller's also_label must be the label the quiesce script discovers, or every
# service it adds fails its own settle wait.
if CALLERS=$(python3 - "${WORKFLOW}" "${WORK}" <<'PY'
import glob, os, re, sys, yaml
wf_path, work = sys.argv[1], sys.argv[2]
wf = yaml.safe_load(open(wf_path))
inputs = (wf.get(True) or wf["on"])["workflow_call"]["inputs"]  # PyYAML 1.1 reads a bare `on:` as True
required = {k for k, v in inputs.items() if v.get("required")}
problems, callers = [], []
wait = open("scripts/ci/wait-cloud-run-service-ready.sh").read()
key = re.search(r'^DISCOVERY_LABEL_KEY="([^"]+)"', wait, re.M)
value = re.search(r'^DISCOVERY_LABEL_VALUE="([^"]+)"', wait, re.M)
discovered = f"{key.group(1)}={value.group(1)}" if key and value else None
if not discovered:
    problems.append("wait-cloud-run-service-ready.sh declares no DISCOVERY_LABEL_KEY/VALUE")
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
        if "needs.bump.outputs.merged == 'true'" not in str((job.get("with") or {}).get("ship_platform")):
            problems.append(f"{path} job {job_id} must ship the platform only when the spine landed a bump on main")
        label = (job.get("with") or {}).get("also_label")
        if label and label != discovered:
            problems.append(f"{path} job {job_id} deploys also_label '{label}', but the quiesce script discovers '{discovered}'")
# Each lifted step must be expression-free, so the test runs what the runner runs.
steps = {s.get("name"): s for j in wf["jobs"].values() for s in j["steps"] if s.get("name")}
CAPTURE = "Capture the digest each service is serving"
DEPLOY = "Deploy each service, then assert its bytes and probe its health"
ROLLBACK = "Roll back to the previously serving digest"
lift = {"capture.sh": CAPTURE, "deploy.sh": DEPLOY, "rollback.sh": ROLLBACK,
        "dispatch.sh": "Dispatch the platform image build and deploy"}
for out, name in lift.items():
    run = (steps.get(name) or {}).get("run")
    if not run:
        problems.append(f"no step named '{name}' with a run block in {wf_path}")
        continue
    if "${{" in run:
        problems.append(f"step '{name}' interpolates an expression into its script; move it to env")
    open(os.path.join(work, out), "w").write(run)
# The inputs reach the lifted scripts only through env, which the lift cannot
# see: pin the wiring that decides which services move and from which image.
wiring = {
    (CAPTURE, "SERVICE"): "${{ inputs.service }}",
    (CAPTURE, "ALSO_LABEL"): "${{ inputs.also_label }}",
    (DEPLOY, "IMAGE_NAME"): "${{ inputs.image_name || inputs.service }}",
    (DEPLOY, "EXPECTED"): "${{ inputs.digest }}",
    (DEPLOY, "PROBE_AUDIENCE"): "${{ inputs.probe_audience }}",
}
for (name, var), want in wiring.items():
    got = ((steps.get(name) or {}).get("env") or {}).get(var)
    if got != want:
        problems.append(f"step '{name}' sets {var}={got!r}, expected {want!r}")
rollback = steps.get(ROLLBACK) or {}
if rollback.get("if") != "failure() && steps.prev.outputs.captured == 'true'":
    problems.append(f"the rollback runs on {rollback.get('if')!r}, not on a failure after the targets were captured")
if (steps.get(CAPTURE) or {}).get("id") != "prev":
    problems.append(f"the step '{CAPTURE}' must keep id prev, which the rollback's condition reads")
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
for s in capture deploy rollback dispatch; do
  [ -s "${WORK}/${s}.sh" ] || { echo "could not lift the ${s} step out of ${WORKFLOW}"; exit 1; }
done

# ---------------------------------------------------------------------------
# Stand-ins, logging to one file in call order so a case can read the
# sequence. gcloud keeps a serving digest per service in ${STUB}/serving/<svc>
# (…@OLD_<svc> until an update): an update to :latest makes it serve
# ${NEW_SERVING} (or ${STUB}/new/<svc>), an update to a digest makes it serve
# that digest. ${STUB}/fail/<what> makes one call fail. The wait script is a
# stub at the path the steps call it by, in a scratch checkout the steps run
# from; its list mode answers the suffixes of the names in ${STUB}/list under
# its --prefix, once each, as the real script does. curl serves the registry index
# and the /health body.
# ---------------------------------------------------------------------------
BIN="${WORK}/bin"; mkdir -p "${BIN}"
cat > "${BIN}/gcloud" <<'SH'
#!/usr/bin/env bash
echo "gcloud $*" >> "${STUB}/log"
case "$*" in
  "run services list "*)
    [ ! -e "${STUB}/fail/list" ] || { echo "ERROR: (gcloud.run.services.list) PERMISSION_DENIED" >&2; exit 1; }
    if [ -s "${STUB}/list" ]; then cat "${STUB}/list"; else echo "Listed 0 items." >&2; fi ;;
  "run services describe "*"status.latestReadyRevisionName"*) echo "${4}-00042-abc" ;;
  "run services describe "*"status.url"*) echo "https://${4}.example.invalid" ;;
  "run revisions describe "*"status.imageDigest"*)
    svc="${4%-00042-abc}"
    if [ -e "${STUB}/serving/${svc}" ]; then cat "${STUB}/serving/${svc}"
    elif [ -e "${STUB}/no-digest/${svc}" ]; then echo ""
    else echo "europe-docker.pkg.dev/dravr-artifacts/images/x@OLD_${svc}"; fi ;;
  "run services update "*)
    [ ! -e "${STUB}/fail/update-${4}" ] || { echo "ERROR: (gcloud.run.services.update) update of ${4} failed" >&2; exit 1; }
    image=""; prev=""
    for a in "$@"; do [ "${prev}" = "--image" ] && image="${a}"; prev="${a}"; done
    mkdir -p "${STUB}/serving"
    if [ "${image%:latest}" != "${image}" ]; then
      if [ -e "${STUB}/new/${4}" ]; then cat "${STUB}/new/${4}" > "${STUB}/serving/${4}"
      else echo "${NEW_SERVING}" > "${STUB}/serving/${4}"; fi
    else
      echo "${image}" > "${STUB}/serving/${4}"
    fi ;;
  "auth print-access-token") echo "ya29.stub" ;;
  "auth print-identity-token "*) echo "id.stub" ;;
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
echo "curl ${url}" >> "${STUB}/log"
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
CHECKOUT="${WORK}/checkout"; mkdir -p "${CHECKOUT}/scripts/ci"
cat > "${CHECKOUT}/scripts/ci/wait-cloud-run-service-ready.sh" <<'SH'
#!/usr/bin/env bash
echo "wait $*" >> "${STUB}/log"
mode="$1"; suffix=""; prefix=""; prev=""
for a in "$@"; do
  [ "${prev}" = "--service" ] && suffix="${a}"
  [ "${prev}" = "--prefix" ] && prefix="${a}"
  prev="${a}"
done
if [ "${mode}" = "list" ]; then
  [ ! -e "${STUB}/fail/list" ] || { echo "::error::gcloud run services list failed: PERMISSION_DENIED" >&2; exit 1; }
  [ -s "${STUB}/list" ] || exit 0
  while IFS= read -r name; do
    [ "${name#"${prefix}"-}" != "${name}" ] && echo "${name#"${prefix}"-}"
  done <"${STUB}/list" | awk '!seen[$0]++'
  exit 0
fi
if [ -e "${STUB}/fail/wait-${mode}${suffix:+-${suffix}}" ]; then echo "::error::stub ${mode} ${suffix} timed out" >&2; exit 1; fi
SH
chmod +x "${BIN}"/* "${CHECKOUT}/scripts/ci/wait-cloud-run-service-ready.sh"

INDEX="sha256:1111111111111111111111111111111111111111111111111111111111111111"
CHILD="sha256:2222222222222222222222222222222222222222222222222222222222222222"
ATTEST="sha256:3333333333333333333333333333333333333333333333333333333333333333"
FOREIGN="sha256:9999999999999999999999999999999999999999999999999999999999999999"
REGISTRY_URL="europe-docker.pkg.dev/dravr-artifacts/images"
OLD_IMAGE="europe-docker.pkg.dev/dravr-artifacts/images/x@OLD"
INDEX_JSON=$(jq -cn --arg a "${CHILD}" --arg b "${ATTEST}" '{manifests: [{digest: $a}, {digest: $b}]}')
PFX="dravr-dev"
REGION_NAME="northamerica-northeast1"
OK_HEALTH='{"status":"ok","providers":["garmin","strava"]}'

# Every scenario gets a fresh stub state; the steps of one scenario share it,
# as the steps of one job share RUNNER_TEMP.
N=0
scenario() {
  N=$(( N + 1 )); STUB="${WORK}/stub-${N}"
  mkdir -p "${STUB}/fail" "${STUB}/runner"; : > "${STUB}/log"; : > "${STUB}/output"
}
run_step() {  # run_step <script> <env assignments...>
  local script="$1"; shift
  GH_LOG="${WORK}/gh.log"; : > "${GH_LOG}"
  set +e
  OUT=$(cd "${CHECKOUT}" && env PATH="${BIN}:${PATH}" STUB="${STUB}" GH_LOG="${GH_LOG}" INDEX_JSON="${INDEX_JSON}" \
        RUNNER_TEMP="${STUB}/runner" GITHUB_OUTPUT="${STUB}/output" GITHUB_SHA="0123abc" \
        REGION="${REGION_NAME}" PREFIX="${PFX}" REGISTRY="${REGISTRY_URL}" EXPECTED="${INDEX}" \
        NEW_SERVING="${REGISTRY_URL}/sciotte@${CHILD}" PROBE_AUDIENCE="" ID_TOKEN="" PROVIDERS="" \
        HEALTH_CODE=200 HEALTH_BODY="${OK_HEALTH}" "$@" bash "${WORK}/${script}" 2>&1)
  RC=$?
  set -e
}
check() {  # check <name> <expected rc> <expected output fragment>
  if [ "${RC}" = "$2" ] && grep -qF -- "$3" <<<"${OUT}"; then pass "$1"
  else fail "$1 (rc=${RC}, want $2; output did not contain '$3')"; printf '%s\n' "${OUT}" | tail -3 | sed 's/^/       /'; fi
}
expect() {  # expect <name> <actual> <wanted>
  if [ "$2" = "$3" ]; then pass "$1"
  else fail "$1 — got '$(tr '\n' '|' <<<"$2")', want '$(tr '\n' '|' <<<"$3")'"; fi
}
targets() { cat "${STUB}/runner/satellite-deploy/targets" 2>/dev/null || true; }
moved() { tr '\n' ' ' <"${STUB}/runner/satellite-deploy/moved" 2>/dev/null | sed 's/ $//'; }
# The log with the reads left out: the moves and the waits, in order.
rolls() {
  grep -E '^(gcloud run services update|wait (service|quiesce))' "${STUB}/log" \
    | sed -E 's/ --(region|prefix|timeout-seconds|update-labels) [^ ]+//g; s/ --quiet//'
}

echo "deploy-satellite-cloud-run: which services move, in which order"
scenario
run_step capture.sh SERVICE="photograveur" ALSO_LABEL=""
check "no also_label: the one service alone" 0 "${PFX}-photograveur serves ${PFX}-photograveur-00042-abc"
expect "  ...its own digest is its rollback target, and nothing is listed" \
  "$(targets)|$(grep -cE 'services list|wait list' "${STUB}/log" || true)|$(cat "${STUB}/output")" \
  "photograveur ${OLD_IMAGE}_${PFX}-photograveur|0|captured=true"

scenario
printf '%s\n' "${PFX}-sciotte-login" "${PFX}-sciotte-read-10" "${PFX}-sciotte-read-2" "${PFX}-sciotte" \
  "other-sciotte-read-1" "${PFX}-sciotte-read-2" "${PFX}-sciotte-read-1" > "${STUB}/list"
run_step capture.sh SERVICE="sciotte" ALSO_LABEL="component=sciotte"
check "also_label: every labelled service under the prefix is captured" 0 "${PFX}-sciotte-login serves"
expect "  ...service first, reads by number (2 before 10), then the rest; once each; other prefixes left out" \
  "$(targets | cut -d' ' -f1 | tr '\n' ' ')" "sciotte sciotte-read-1 sciotte-read-2 sciotte-read-10 sciotte-login "
if grep -qxF "wait list --region ${REGION_NAME} --prefix ${PFX}" "${STUB}/log"; then
  pass "  ...listed by the wait script, the one reader of the labelled services"
else
  fail "  ...the list call was: $(grep -E 'services list|wait list' "${STUB}/log")"
fi

scenario
run_step capture.sh SERVICE="sciotte" ALSO_LABEL="component=sciotte"
check "an empty label answer fails, since the service itself carries the label" 1 "listing does not name ${PFX}-sciotte"
expect "  ...and marks nothing captured" "$(cat "${STUB}/output")" ""

scenario
printf '%s\n' "${PFX}-sciotte" > "${STUB}/list"
run_step capture.sh SERVICE="sciotte" ALSO_LABEL="component=sciotte"
check "a label answer naming only the service deploys it" 0 "${PFX}-sciotte serves"
expect "  ...and only it" "$(targets)" "sciotte ${OLD_IMAGE}_${PFX}-sciotte"

scenario
printf '%s\n' "${PFX}-sciotte-read-1" "${PFX}-sciotte-login" > "${STUB}/list"
run_step capture.sh SERVICE="sciotte" ALSO_LABEL="component=sciotte"
check "a label answer that leaves the service out fails" 1 "listing does not name ${PFX}-sciotte"

scenario
touch "${STUB}/fail/list"
run_step capture.sh SERVICE="sciotte" ALSO_LABEL="component=sciotte"
check "a failed label list fails the capture" 1 "PERMISSION_DENIED"
expect "  ...and marks nothing captured" "$(cat "${STUB}/output")" ""

scenario
printf '%s\n' "${PFX}-sciotte" "${PFX}-sciotte-read-1" > "${STUB}/list"
mkdir -p "${STUB}/no-digest"; touch "${STUB}/no-digest/${PFX}-sciotte-read-1"
run_step capture.sh SERVICE="sciotte" ALSO_LABEL="component=sciotte"
check "one service with no readable digest refuses the whole deploy" 1 "(${PFX}-sciotte-read-1) — refusing to deploy without a rollback target"
expect "  ...before anything moves (nothing marked captured)" "$(cat "${STUB}/output")" ""

scenario
run_step capture.sh SERVICE="sciotte" ALSO_LABEL="sciotte"
check "an also_label that is not key=value fails" 1 "also_label 'sciotte' is not a key=value Cloud Run label"

echo "deploy-satellite-cloud-run: deploy, digest assertion and HTTP probe"
for svc in sciotte photograveur; do
  scenario
  run_step capture.sh SERVICE="${svc}" ALSO_LABEL=""
  run_step deploy.sh IMAGE_NAME="${svc}" NEW_SERVING="${REGISTRY_URL}/${svc}@${CHILD}"
  check "${svc}: Cloud Run records the amd64 child of the built index — accepted" 0 "(matches ${CHILD})"
  if grep -qxF "curl https://europe-docker.pkg.dev/v2/dravr-artifacts/images/${svc}/manifests/${INDEX}" "${STUB}/log"; then
    pass "  ...and the index was read from ${svc}'s own repository"
  else
    fail "  ...the index was read from $(grep manifests "${STUB}/log"), not ${svc}'s repository"
  fi
  expect "  ...one roll from ${svc}:latest, then its settle; no probe without an audience" \
    "$(rolls)|$(grep -c '/health' "${STUB}/log" || true)" \
    "gcloud run services update ${PFX}-${svc} --image ${REGISTRY_URL}/${svc}:latest
wait service --service ${svc}|0"
done
if grep -qF -- "--update-labels managed-by=github-actions,commit-sha=0123abc --quiet" "${STUB}/log"; then
  pass "  ...stamping the revision with the deployer and the commit"
else
  fail "  ...the update was: $(grep 'services update' "${STUB}/log")"
fi

scenario
printf '%s\n' "${PFX}-sciotte-login" "${PFX}-sciotte-read-1" "${PFX}-sciotte" > "${STUB}/list"
run_step capture.sh SERVICE="sciotte" ALSO_LABEL="component=sciotte"
run_step deploy.sh IMAGE_NAME="sciotte" PROBE_AUDIENCE="dravr-sciotte-development" ID_TOKEN="stub-id-token" PROVIDERS="garmin strava"
check "three services: each asserted on its own serving digest" 0 "${PFX}-sciotte-login: serving ${PFX}-sciotte-login-00042-abc"
expect "  ...in order, each settled before the next, every one after the first behind a quiesce" "$(rolls)" \
  "gcloud run services update ${PFX}-sciotte --image ${REGISTRY_URL}/sciotte:latest
wait service --service sciotte
wait quiesce
gcloud run services update ${PFX}-sciotte-read-1 --image ${REGISTRY_URL}/sciotte:latest
wait service --service sciotte-read-1
wait quiesce
gcloud run services update ${PFX}-sciotte-login --image ${REGISTRY_URL}/sciotte:latest
wait service --service sciotte-login"
expect "  ...each one's /health probed" \
  "$(grep '/health$' "${STUB}/log" | tr '\n' ' ')" \
  "curl https://${PFX}-sciotte.example.invalid/health curl https://${PFX}-sciotte-read-1.example.invalid/health curl https://${PFX}-sciotte-login.example.invalid/health "
expect "  ...and each recorded as moved" "$(moved)" "sciotte sciotte-read-1 sciotte-login"

scenario
printf '%s\n' "${PFX}-sciotte-login" "${PFX}-sciotte-read-1" "${PFX}-sciotte" > "${STUB}/list"
mkdir -p "${STUB}/new"; echo "${REGISTRY_URL}/sciotte@${FOREIGN}" > "${STUB}/new/${PFX}-sciotte-read-1"
run_step capture.sh SERVICE="sciotte" ALSO_LABEL="component=sciotte"
run_step deploy.sh IMAGE_NAME="sciotte"
check "a digest the build never produced fails on the service serving it" 1 "${PFX}-sciotte-read-1: ${PFX}-sciotte-read-1-00042-abc serves ${REGISTRY_URL}/sciotte@${FOREIGN}, which is neither ${INDEX} nor any manifest it indexes"
expect "  ...and nothing after it moves" "$(moved)" "sciotte sciotte-read-1"

scenario
run_step capture.sh SERVICE="sciotte" ALSO_LABEL=""
run_step deploy.sh IMAGE_NAME="sciotte" NEW_SERVING="${INDEX}"
check "the index digest itself is accepted" 0 "(matches ${INDEX})"

scenario
printf '%s\n' "${PFX}-sciotte" "${PFX}-sciotte-read-1" > "${STUB}/list"
touch "${STUB}/fail/update-${PFX}-sciotte-read-1"
run_step capture.sh SERVICE="sciotte" ALSO_LABEL="component=sciotte"
run_step deploy.sh IMAGE_NAME="sciotte"
check "a failed update fails the step" 1 "update of ${PFX}-sciotte-read-1 failed"
expect "  ...with that service already recorded, so the rollback covers a half-applied update" "$(moved)" "sciotte sciotte-read-1"

probe() {  # probe <providers> <http code> <body>
  scenario
  run_step capture.sh SERVICE="sciotte" ALSO_LABEL=""
  run_step deploy.sh IMAGE_NAME="sciotte" PROBE_AUDIENCE="dravr-sciotte-development" ID_TOKEN="stub-id-token" \
    PROVIDERS="$1" HEALTH_CODE="$2" HEALTH_BODY="$3"
}
probe "garmin strava" 200 "${OK_HEALTH}"
check "ok and every named provider advertised passes" 0 'advertises ["garmin","strava"]'
probe "garmin strava" 200 '{"status":"ok","providers":["garmin"]}'
check "a build that lost a named provider fails, naming it" 1 "${PFX}-sciotte: /health does not advertise the strava provider"
probe "garmin strava" 200 '{"status":"degraded","providers":["garmin","strava"]}'
check "a status other than ok fails" 1 "${PFX}-sciotte: /health reports status=degraded"
probe "garmin strava" 401 '{"error":"audience"}'
check "an identity-token refusal is named as auth, never as a sick container" 1 "returned HTTP 401"

echo "deploy-satellite-cloud-run: rollback"
# A deploy that failed on the second of three services.
failed_on_read_1() {
  scenario
  printf '%s\n' "${PFX}-sciotte-login" "${PFX}-sciotte-read-1" "${PFX}-sciotte" > "${STUB}/list"
  mkdir -p "${STUB}/new"; echo "${REGISTRY_URL}/sciotte@${FOREIGN}" > "${STUB}/new/${PFX}-sciotte-read-1"
  run_step capture.sh SERVICE="sciotte" ALSO_LABEL="component=sciotte"
  run_step deploy.sh IMAGE_NAME="sciotte"
  : > "${STUB}/log"
}
failed_on_read_1
run_step rollback.sh
check "every moved service goes back" 0 "rolled ${PFX}-sciotte-read-1 back to ${OLD_IMAGE}_${PFX}-sciotte-read-1"
expect "  ...after one quiesce, each to its own digest and settled, never the unmoved one" "$(rolls)" \
  "wait quiesce
gcloud run services update ${PFX}-sciotte --image ${OLD_IMAGE}_${PFX}-sciotte
wait service --service sciotte
gcloud run services update ${PFX}-sciotte-read-1 --image ${OLD_IMAGE}_${PFX}-sciotte-read-1
wait service --service sciotte-read-1"

failed_on_read_1
touch "${STUB}/fail/update-${PFX}-sciotte"
run_step rollback.sh
check "a service that does not come back is named with its by-hand command" 1 \
  "${PFX}-sciotte did not roll back; once dev is quiet: gcloud run services update ${PFX}-sciotte --region ${REGION_NAME} --image ${OLD_IMAGE}_${PFX}-sciotte"
if grep -qF "gcloud run services update ${PFX}-sciotte-read-1 " "${STUB}/log"; then
  pass "  ...and the others are still rolled back"
else
  fail "  ...the rollback stopped at the first failure: $(rolls | tr '\n' '|')"
fi

failed_on_read_1
touch "${STUB}/fail/wait-quiesce"
run_step rollback.sh
check "dev never quiescing rolls nothing back" 1 \
  "::error::gcloud run services update ${PFX}-sciotte-read-1 --region ${REGION_NAME} --image ${OLD_IMAGE}_${PFX}-sciotte-read-1"
expect "  ...and names the command for every moved service, issuing none" \
  "$(grep -c '^::error::gcloud run services update' <<<"${OUT}")|$(grep -c '^gcloud run services update' "${STUB}/log" || true)" "2|0"

scenario
run_step capture.sh SERVICE="sciotte" ALSO_LABEL=""
: > "${STUB}/log"
run_step rollback.sh
check "a failure before any move rolls nothing back" 0 "no service was moved, so there is nothing to roll back"
expect "  ...and calls nothing" "$(cat "${STUB}/log")" ""

echo "deploy-satellite-cloud-run: platform dispatch"
scenario
SUMMARY_FILE="${WORK}/summary.md"; : > "${SUMMARY_FILE}"
run_step dispatch.sh GITHUB_STEP_SUMMARY="${SUMMARY_FILE}" RELEASE="enforme v0.5.1" \
  SUMMARY=$'The session-read service is on **v0.9.0**;\nrebuilding to match.'
check "the dispatch log names the release the binary picks up" 0 "so the binary picks up enforme v0.5.1"
if [ "$(cat "${GH_LOG}")" = "workflow run publish-images.yml --ref main" ]; then
  pass "  ...and dispatches publish-images.yml on main"
else
  fail "  ...gh was called with '$(cat "${GH_LOG}")'"
fi
WANT=$'### Platform deploy dispatched\n\nThe session-read service is on **v0.9.0**;\nrebuilding to match.'
if [ "$(cat "${SUMMARY_FILE}")" = "${WANT}" ]; then
  pass "  ...and writes the caller's summary under the heading, line breaks intact"
else
  fail "  ...the step summary was: $(cat "${SUMMARY_FILE}")"
fi

echo
[ "${FAILURES}" -eq 0 ] && { echo "all deploy-satellite-cloud-run scenarios pass"; exit 0; }
echo "${FAILURES} scenario(s) failed"; exit 1
