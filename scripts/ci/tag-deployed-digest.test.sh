#!/usr/bin/env bash
# ABOUTME: Pins tag-deployed-digest.sh, the deployed-<env> / deployed-<env>-previous move both dev deploy paths run
# ABOUTME: Runs the script and the workflow steps that call it against a stateful stub registry, then checks the wiring
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The tag move runs only after a dev deploy on main, so a broken one surfaces
# when a rollback reaches for an image the cleanup policy has already reaped
# (carnet#166, carnet#137). Three things are pinned here:
#   - the move itself, against a stub `gcloud` that keeps the registry's tags in
#     a file and prints `tags list` the way the SDK's value() projection does: a
#     first deploy with no outgoing digest, the N-1 handover across successive
#     deploys, a re-run that must not overwrite N-1 with N, and every failure,
#     each of which must exit 1 and leave deployed-<env> where a re-run finishes
#     the move;
#   - the two workflow steps that call it, lifted and run as the runner runs
#     them, against the same stub and the real script;
#   - the wiring: both deploy paths reach the deployed- tags only through the
#     script, with scripts/ci checked out before auth, and the anchor names stay
#     under a prefix keep-referenced-tags holds (infra/artifacts). The wiring scan
#     then runs against broken copies of the workflows and must fail on each, so
#     a scan that checks nothing cannot pass.
#
# Usage: scripts/ci/tag-deployed-digest.test.sh — bash, python3 (PyYAML) and awk, no network.

set -euo pipefail
cd "$(dirname "$0")/../.."
SCRIPT="./scripts/ci/tag-deployed-digest.sh"
WORK=$(mktemp -d); trap 'rm -rf "${WORK}"' EXIT
FAILURES=0
pass() { printf '  ok   %s\n' "$1"; }
fail() { printf '  FAIL %s\n' "$1"; FAILURES=$(( FAILURES + 1 )); }

# ---------------------------------------------------------------------------
# Stand-in registry. REGISTRY_STATE holds one "<package>\t<tag>\t<digest>" line
# per tag. `tags list` prints "<tag>\t<digest>" per tag of the package, which is
# what `--format='value(tag.basename(),version.basename())'` renders from the
# SDK's {tag, version} resource names; `tags add` moves a tag the way the SDK
# does (delete if present, then create) and appends the tag's name to
# WRITTEN_TAGS, which is never reset. FAIL_LIST and FAIL_ADD_TAG inject the
# refusals the script must not swallow; LIST_OVERRIDE replaces the listing.
# ---------------------------------------------------------------------------
BIN="${WORK}/bin"; mkdir -p "${BIN}"
cat > "${BIN}/gcloud" <<'SH'
#!/usr/bin/env bash
echo "$*" >> "${GCLOUD_LOG}"
case "$1 $2 $3 $4" in
  "artifacts docker tags list")
    [ "$#" -eq 6 ] && [ "$6" = "--format=value(tag.basename(),version.basename())" ] \
      || { echo "unexpected tags list call: $*" >&2; exit 2; }
    [ "${FAIL_LIST:-}" != "1" ] \
      || { echo "ERROR: (gcloud.artifacts.docker.tags.list) PERMISSION_DENIED" >&2; exit 1; }
    if [ -n "${LIST_OVERRIDE:-}" ]; then printf '%b\n' "${LIST_OVERRIDE}"; exit 0; fi
    awk -F '\t' -v OFS='\t' -v pkg="$5" '$1 == pkg { print $2, $3 }' "${REGISTRY_STATE}"
    ;;
  "artifacts docker tags add")
    [ "$#" -eq 7 ] && [ "$7" = "--quiet" ] || { echo "unexpected tags add call: $*" >&2; exit 2; }
    src_pkg="${5%@*}"; digest="${5##*@}"; dest_pkg="${6%:*}"; tag="${6##*:}"
    [ "${src_pkg}" != "$5" ] || { echo "ERROR: $5 is not digest-pinned" >&2; exit 1; }
    [ "${src_pkg}" = "${dest_pkg}" ] || { echo "ERROR: Image ${src_pkg} does not match image ${dest_pkg}" >&2; exit 1; }
    [ "${tag}" != "${FAIL_ADD_TAG:-}" ] \
      || { echo "ERROR: (gcloud.artifacts.docker.tags.add) PERMISSION_DENIED on ${tag}" >&2; exit 1; }
    awk -F '\t' -v pkg="${dest_pkg}" -v tag="${tag}" '!($1 == pkg && $2 == tag)' "${REGISTRY_STATE}" > "${REGISTRY_STATE}.next"
    printf '%s\t%s\t%s\n' "${dest_pkg}" "${tag}" "${digest}" >> "${REGISTRY_STATE}.next"
    mv "${REGISTRY_STATE}.next" "${REGISTRY_STATE}"
    printf '%s\n' "${tag}" >> "${WRITTEN_TAGS}"
    ;;
  *) echo "unexpected gcloud call: $*" >&2; exit 2 ;;
esac
SH
chmod +x "${BIN}/gcloud"

STATE="${WORK}/registry.tsv"
GCLOUD_LOG="${WORK}/gcloud.log"
WRITTEN="${WORK}/written-tags"
REG="northamerica-northeast1-docker.pkg.dev/dravr-artifacts/dravr-images"
SERVER="${REG}/server"
FRONTEND="${REG}/frontend"
D1="sha256:1111111111111111111111111111111111111111111111111111111111111111"
D2="sha256:2222222222222222222222222222222222222222222222222222222222222222"
D3="sha256:3333333333333333333333333333333333333333333333333333333333333333"
F1="sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
F2="sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"

registry() {  # registry <package> <tag> <digest> ... — replaces the whole registry
  : > "${STATE}"
  while [ $# -ge 3 ]; do printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "${STATE}"; shift 3; done
}
tag_of() { awk -F '\t' -v pkg="$1" -v tag="$2" '$1 == pkg && $2 == tag { print $3 }' "${STATE}"; }
adds() { grep -c '^artifacts docker tags add' "${GCLOUD_LOG}" || true; }

run_cmd() {  # run_cmd [VAR=value ...] -- <command...>
  local vars=()
  while [ "$1" != "--" ]; do vars+=("$1"); shift; done
  shift
  : > "${GCLOUD_LOG}"
  set +e
  OUT=$(env PATH="${BIN}:${PATH}" GCLOUD_LOG="${GCLOUD_LOG}" REGISTRY_STATE="${STATE}" WRITTEN_TAGS="${WRITTEN}" ${vars[@]+"${vars[@]}"} "$@" 2>&1)
  RC=$?
  set -e
}
check() {  # check <name> <expected rc> <expected output fragment>
  if [ "${RC}" = "$2" ] && grep -qF -- "$3" <<<"${OUT}"; then pass "$1"
  else fail "$1 (rc=${RC}, want $2; output did not contain '$3')"; printf '%s\n' "${OUT}" | tail -3 | sed 's/^/       /'; fi
}
check_tag() {  # check_tag <name> <package> <tag> <digest, or empty for no such tag>
  local got; got=$(tag_of "$2" "$3")
  if [ "${got}" = "$4" ]; then pass "$1"
  else fail "$1 (${3} on ${2##*/} names '${got:-nothing}', want '${4:-nothing}')"; fi
}
check_no_gcloud() {  # check_no_gcloud <name> — the last run_cmd made no gcloud call
  if [ ! -s "${GCLOUD_LOG}" ]; then pass "$1"; else fail "$1 (gcloud was called: $(tr '\n' ';' < "${GCLOUD_LOG}"))"; fi
}
check_adds() {  # check_adds <name> <expected number of tags add calls>
  if [ "$(adds)" = "$2" ]; then pass "$1"; else fail "$1 ($(adds) tags add calls, want $2)"; fi
}

echo "tag-deployed-digest: the move"
# A package's first deploy: nothing names an outgoing digest. Another package's
# deployed-dev and this package's own SHA tag are in the listing and must be
# ignored.
registry "${FRONTEND}" deployed-dev "${F1}" "${SERVER}" 1a2b3c4 "${D1}"
run_cmd -- "${SCRIPT}" --image "${SERVER}@${D1}" --env dev
check "first deploy: no deployed-dev yet is not an error" 0 "its first deploy, so there is no outgoing digest"
check_tag "  ...deployed-dev lands on the shipped digest" "${SERVER}" deployed-dev "${D1}"
check_tag "  ...and no deployed-dev-previous is invented" "${SERVER}" deployed-dev-previous ""
check_tag "  ...and the frontend's deployed-dev is untouched" "${FRONTEND}" deployed-dev "${F1}"

run_cmd -- "${SCRIPT}" --image "${SERVER}@${D2}" --env dev
check "second deploy: the outgoing digest is named" 0 "Tagging the outgoing ${SERVER}@${D1} as ${SERVER}:deployed-dev-previous"
check_tag "  ...deployed-dev-previous holds N-1" "${SERVER}" deployed-dev-previous "${D1}"
check_tag "  ...deployed-dev holds N" "${SERVER}" deployed-dev "${D2}"
# `|| true`: a missing write is this check's failure to report, not the harness's to die on.
PREV_LINE=$(grep -n 'deployed-dev-previous' "${GCLOUD_LOG}" | head -1 | cut -d: -f1 || true)
CURR_LINE=$(grep -n ':deployed-dev --quiet$' "${GCLOUD_LOG}" | head -1 | cut -d: -f1 || true)
if [ -n "${PREV_LINE}" ] && [ -n "${CURR_LINE}" ] && [ "${PREV_LINE}" -lt "${CURR_LINE}" ]; then
  pass "  ...N-1 is anchored before deployed-dev leaves it, so it is never untagged"
else
  fail "  ...the tags were written out of order: $(tr '\n' ';' < "${GCLOUD_LOG}")"
fi

run_cmd -- "${SCRIPT}" --image "${SERVER}@${D3}" --env dev
check "third deploy: N-1 hands over" 0 "Tagging the outgoing ${SERVER}@${D2}"
check_tag "  ...deployed-dev-previous moves to the digest deployed-dev left" "${SERVER}" deployed-dev-previous "${D2}"
check_tag "  ...deployed-dev holds the new digest" "${SERVER}" deployed-dev "${D3}"
if ! awk -F '\t' -v d="${D1}" '$3 == d && $2 ~ /^deployed-/ { found = 1 } END { exit !found }' "${STATE}"; then
  pass "  ...N-2 keeps no deployed- tag and resumes normal ageing"
else
  fail "  ...N-2 still carries a deployed- tag: $(tr '\n' ';' < "${STATE}")"
fi

run_cmd -- "${SCRIPT}" --image "${SERVER}@${D3}" --env dev
check "re-run of the same deploy succeeds" 0 "already names ${D3} (a re-run)"
check_tag "  ...and does not overwrite N-1 with N" "${SERVER}" deployed-dev-previous "${D2}"
check_tag "  ...deployed-dev still holds N" "${SERVER}" deployed-dev "${D3}"
check_adds "  ...and only deployed-dev is rewritten" 1

echo "tag-deployed-digest: failures are loud and resumable"
registry "${SERVER}" deployed-dev "${D1}" "${SERVER}" deployed-dev-previous "${F1}"
run_cmd FAIL_LIST=1 -- "${SCRIPT}" --image "${SERVER}@${D2}" --env dev
check "a refused tag listing fails the step" 1 "::error::could not list the tags of ${SERVER}"
check_adds "  ...before any tag is written" 0
check_tag "  ...deployed-dev stays on the serving digest" "${SERVER}" deployed-dev "${D1}"

run_cmd FAIL_ADD_TAG=deployed-dev-previous -- "${SCRIPT}" --image "${SERVER}@${D2}" --env dev
check "a refused N-1 tag fails the step" 1 "::error::could not tag the outgoing ${SERVER}@${D1} as deployed-dev-previous"
check_tag "  ...and deployed-dev is not moved off the outgoing digest" "${SERVER}" deployed-dev "${D1}"
run_cmd -- "${SCRIPT}" --image "${SERVER}@${D2}" --env dev
check "  ...a re-run completes the move" 0 "Tagging ${SERVER}@${D2} as ${SERVER}:deployed-dev"
check_tag "  ...N-1 anchored" "${SERVER}" deployed-dev-previous "${D1}"
check_tag "  ...N in place" "${SERVER}" deployed-dev "${D2}"

registry "${SERVER}" deployed-dev "${D1}"
run_cmd FAIL_ADD_TAG=deployed-dev -- "${SCRIPT}" --image "${SERVER}@${D2}" --env dev
check "a refused deployed-dev move fails the step" 1 "::error::could not move deployed-dev onto ${SERVER}@${D2}"
check_tag "  ...the outgoing digest is already anchored as N-1" "${SERVER}" deployed-dev-previous "${D1}"
run_cmd -- "${SCRIPT}" --image "${SERVER}@${D2}" --env dev
check_tag "  ...a re-run keeps N-1 on the outgoing digest" "${SERVER}" deployed-dev-previous "${D1}"
check_tag "  ...and moves deployed-dev" "${SERVER}" deployed-dev "${D2}"

registry "${SERVER}" deployed-dev "${D1}"
run_cmd "LIST_OVERRIDE=deployed-dev\tprojects/p/locations/l/repositories/r/packages/server/versions/${D1}" -- \
  "${SCRIPT}" --image "${SERVER}@${D2}" --env dev
check "a listing that does not name a digest fails rather than guessing" 1 "::error::deployed-dev on ${SERVER} reads as"
check_adds "  ...before any tag is written" 0

run_cmd -- "${SCRIPT}" --image "${SERVER}:latest" --env dev
check "an image that is not digest-pinned is refused" 1 "--image must be digest-pinned"
check_no_gcloud "  ...before gcloud is called"
run_cmd -- "${SCRIPT}" --image "${SERVER}@${D2}"
check "a missing --env is refused" 1 "--env is required"
check_no_gcloud "  ...before gcloud is called"
run_cmd -- "${SCRIPT}" --image "${SERVER}@${D2}" --env dev-previous
check "an --env that would collide with the N-1 tag is refused" 1 "--env must be one lowercase word"
check_no_gcloud "  ...before gcloud is called"

# ---------------------------------------------------------------------------
# The wiring, and the lift of the two calling steps. KNOWN names every job that
# moves a deployed- tag; the scan must find each, and nothing else may write one.
# ---------------------------------------------------------------------------
echo "tag-deployed-digest: workflows"
if SCAN_OUT=$(python3 - "${WORK}" <<'PY' 2>&1
import copy, glob, os, re, shutil, sys, tempfile, yaml

work = sys.argv[1]
SCRIPT = "scripts/ci/tag-deployed-digest.sh"
# An invocation, not a mention (the fast-gate's shellcheck line names the path):
# the path followed by its flags.
INVOKE = re.compile(r"scripts/ci/tag-deployed-digest\.sh\"?\s+--")
CALL = re.compile(r"\./scripts/ci/tag-deployed-digest\.sh\s+--image\s+\S+\s+--env\s+dev\s*$", re.M)
# A deployed- tag in a command, but not the script's own name (tag-deployed-digest).
TAG_REF = re.compile(r"(?<![\w-])deployed-")
KNOWN = {
    "publish-images.yml:tag-deployed-dev": "publish-step.sh",
    "deploy-satellite-cloud-run.yml:tag-deployed": "satellite-step.sh",
}

def code(run):
    return "\n".join(l for l in run.splitlines() if not l.lstrip().startswith("#"))

def scan(wf_dir):
    problems, lifted = [], {}
    # The workflow files are the data under test: which jobs move a deployed-
    # tag, and in what step order, is declared nowhere else. The steps found here
    # are also lifted and run below, so the text checks guard only the wiring.
    for path in sorted(glob.glob(os.path.join(wf_dir, "*.yml"))):
        doc = yaml.safe_load(open(path)) or {}
        for job_id, job in (doc.get("jobs") or {}).items():
            where = f"{os.path.basename(path)}:{job_id}"
            steps = job.get("steps") or []
            for i, step in enumerate(steps):
                run = step.get("run") or ""
                body = code(run)
                label = f"{where} step '{step.get('name')}'"
                if TAG_REF.search(body) and "gcloud artifacts" in body:
                    problems.append(f"{label} writes a deployed- tag itself; only {SCRIPT} may move them")
                if not INVOKE.search(body):
                    continue
                if where in lifted:
                    problems.append(f"{where} calls {SCRIPT} from more than one step")
                lifted[where] = run
                if not CALL.search(body):
                    problems.append(f"{label} must call ./{SCRIPT} --image <digest-pinned image> --env dev")
                if "${{" in run:
                    problems.append(f"{label} interpolates an expression into its script; move it to env")
                uses = [s.get("uses") or "" for s in steps[:i]]
                checkouts = [j for j, u in enumerate(uses) if u.startswith("actions/checkout@")]
                auths = [j for j, u in enumerate(uses) if u.startswith("google-github-actions/auth@")]
                if not auths:
                    problems.append(f"{label} runs with no google-github-actions/auth before it")
                if not checkouts:
                    problems.append(f"{label} has no actions/checkout before it, so {SCRIPT} is not on the runner")
                elif auths and max(checkouts) > min(auths):
                    problems.append(f"{where} checks out after auth, which deletes the credentials file auth wrote")
                for j in checkouts:
                    sparse = str((steps[j].get("with") or {}).get("sparse-checkout") or "")
                    if sparse and "scripts/ci" not in sparse.split():
                        problems.append(f"{where} sparse checkout '{sparse.strip()}' does not include scripts/ci")
    for where in sorted(set(KNOWN) - set(lifted)):
        problems.append(f"{where} no longer calls {SCRIPT}: the scan found nothing to check there")
    for where in sorted(set(lifted) - set(KNOWN)):
        problems.append(f"{where} calls {SCRIPT} but is not in KNOWN: add it, and a run scenario for it")
    return problems, lifted

problems, lifted = scan(".github/workflows")
for where, out in KNOWN.items():
    if where in lifted:
        open(os.path.join(work, out), "w").write(lifted[where])

# Every tag the script writes must stay under a prefix the registry's keep rule
# holds. A parity check between two artefacts nothing at runtime joins, so it
# reads both sides. One side is the tags the move scenarios above made the script
# write, as the stub registry recorded them, never a list restated here. The
# other is infra/artifacts' effective release_tag_prefixes: the tfvars value when
# set, else the variable's default. A rename on either side passes every other
# check and surfaces only when the registry reaps a rollback target.
def prefixes():
    tfvars = open("infra/artifacts/terraform.tfvars").read()
    m = re.search(r"^\s*release_tag_prefixes\s*=\s*\[([^\]]*)\]", tfvars, re.M)
    if not m:
        var = re.search(r'variable\s+"release_tag_prefixes"\s*\{(.*?)\n\}', open("infra/artifacts/variables.tf").read(), re.S)
        m = var and re.search(r"^\s*default\s*=\s*\[([^\]]*)\]", var.group(1), re.M)
    return re.findall(r'"([^"]*)"', m.group(1)) if m else []

def unkept(tags, kept):
    return [t for t in tags if not any(t.startswith(p) for p in kept)]

written_path = os.path.join(work, "written-tags")
written = sorted(set(open(written_path).read().split())) if os.path.exists(written_path) else []
kept = prefixes()
if not written:
    problems.append("the move scenarios wrote no tag, so the keep-prefix check has nothing to compare")
if not kept:
    problems.append("could not read release_tag_prefixes from infra/artifacts")
for tag in unkept(written, kept):
    problems.append(f"{tag} is under no keep-referenced-tags prefix {kept}: the cleanup policy would reap it")

for p in problems:
    print(f"PROBLEM {p}")

# The scan against broken copies: each mutation must produce a problem.
def mutate(fn):
    tmp = tempfile.mkdtemp(dir=work)
    for path in glob.glob(".github/workflows/*.yml"):
        shutil.copy(path, tmp)
    fn(tmp)
    found, _ = scan(tmp)
    shutil.rmtree(tmp)
    return found

def edit(tmp, name, fn):
    path = os.path.join(tmp, name)
    doc = yaml.safe_load(open(path))
    fn(doc)
    yaml.safe_dump(doc, open(path, "w"), sort_keys=False)

def step(doc, job, needle):
    return next(s for s in doc["jobs"][job]["steps"] if needle in (s.get("run") or ""))

def pub(fn): return lambda tmp: edit(tmp, "publish-images.yml", fn)
def sat(fn): return lambda tmp: edit(tmp, "deploy-satellite-cloud-run.yml", fn)

def checkout_after_auth(doc):
    steps = doc["jobs"]["tag-deployed"]["steps"]
    co = next(s for s in steps if (s.get("uses") or "").startswith("actions/checkout@"))
    steps.remove(co)
    steps.insert(len(steps) - 1, co)

def third_writer(tmp):
    doc = {"name": "x", "on": "push", "jobs": {"rogue": {"runs-on": "ubuntu-latest", "steps": [
        {"run": 'gcloud artifacts docker tags add "${IMAGE}" "${IMAGE%@*}:deployed-dev" --quiet'}]}}}
    yaml.safe_dump(doc, open(os.path.join(tmp, "rogue.yml"), "w"))

MUTATIONS = {
    "publish-images moves deployed-dev itself": pub(lambda d: step(d, "tag-deployed-dev", SCRIPT).update(run=
        'for IMAGE in "${SERVER_IMAGE}"; do\n  gcloud artifacts docker tags add "${IMAGE}" "${IMAGE%@*}:deployed-dev" --quiet\ndone\n')),
    "publish-images interpolates the image into the script": pub(lambda d: step(d, "tag-deployed-dev", SCRIPT).update(run=
        './scripts/ci/tag-deployed-digest.sh --image "${{ needs.publish-server.outputs.image }}" --env dev\n')),
    "the satellite tail tags another environment": sat(lambda d: step(d, "tag-deployed", SCRIPT).update(run=
        './scripts/ci/tag-deployed-digest.sh --image "${IMAGE}" --env prod\n')),
    "the satellite tail has no checkout": sat(lambda d: d["jobs"]["tag-deployed"].update(steps=[
        s for s in d["jobs"]["tag-deployed"]["steps"] if not (s.get("uses") or "").startswith("actions/checkout@")])),
    "the satellite tail checks out after auth": sat(checkout_after_auth),
    "the satellite tail's sparse checkout misses scripts/ci": sat(lambda d: next(
        s for s in d["jobs"]["tag-deployed"]["steps"] if (s.get("uses") or "").startswith("actions/checkout@")
    )["with"].update({"sparse-checkout": "docker"})),
    "a third workflow writes deployed-dev directly": third_writer,
}
for name, fn in MUTATIONS.items():
    print(f"{'CAUGHT' if mutate(fn) else 'MISSED'} {name}")
# The keep-prefix check against a prefix list that has lost deployed-.
without = [p for p in kept if p != "deployed-"]
print(f"{'CAUGHT' if written and unkept(written, without) else 'MISSED'} the keep prefixes lose deployed-")
sys.exit(1 if problems else 0)
PY
); then
  pass "both deploy paths move the deployed- tags only through ${SCRIPT}, checked out before auth"
  pass "every tag the script wrote sits under a keep-referenced-tags prefix"
else
  grep '^PROBLEM ' <<<"${SCAN_OUT}" | sed 's/^PROBLEM /  FAIL /' || true
  grep -v '^PROBLEM \|^CAUGHT \|^MISSED ' <<<"${SCAN_OUT}" | sed 's/^/       /' || true
  FAILURES=$(( FAILURES + 1 ))
fi
CAUGHT=$(grep -c '^CAUGHT ' <<<"${SCAN_OUT}" || true)
while IFS= read -r line; do
  case "${line}" in
    "CAUGHT "*) pass "  ...the scan fails on a copy where ${line#CAUGHT }" ;;
    "MISSED "*) fail "  ...the scan passes a copy where ${line#MISSED }" ;;
  esac
done <<<"${SCAN_OUT}"
[ "${CAUGHT}" -gt 0 ] || fail "the scan ran against no broken copy, so it proves nothing"

[ -s "${WORK}/publish-step.sh" ] && [ -s "${WORK}/satellite-step.sh" ] \
  || { echo "could not lift the tag steps out of the workflows"; exit 1; }

# The runner executes `run:` as `bash -e -o pipefail <file>` from the checkout root.
echo "tag-deployed-digest: publish-images.yml tag-deployed-dev, as the runner runs it"
registry "${SERVER}" deployed-dev "${D1}" "${SERVER}" latest "${D1}" "${FRONTEND}" deployed-dev "${F1}"
run_cmd SERVER_IMAGE="${SERVER}@${D2}" FRONTEND_IMAGE="" -- bash -e -o pipefail "${WORK}/publish-step.sh"
check "server shipped, frontend skipped" 0 "Tagging ${SERVER}@${D2} as ${SERVER}:latest"
check_tag "  ...server N-1 anchored" "${SERVER}" deployed-dev-previous "${D1}"
check_tag "  ...server deployed-dev on N" "${SERVER}" deployed-dev "${D2}"
check_tag "  ...server latest on N" "${SERVER}" latest "${D2}"
check_tag "  ...the skipped frontend keeps its deployed-dev" "${FRONTEND}" deployed-dev "${F1}"
check_tag "  ...and gains no deployed-dev-previous" "${FRONTEND}" deployed-dev-previous ""

run_cmd SERVER_IMAGE="${SERVER}@${D3}" FRONTEND_IMAGE="${FRONTEND}@${F2}" -- bash -e -o pipefail "${WORK}/publish-step.sh"
check "server and frontend shipped" 0 "Tagging ${FRONTEND}@${F2} as ${FRONTEND}:latest"
check_tag "  ...server N-1 is the digest deployed-dev left" "${SERVER}" deployed-dev-previous "${D2}"
check_tag "  ...frontend N-1 anchored" "${FRONTEND}" deployed-dev-previous "${F1}"
check_tag "  ...frontend deployed-dev on N" "${FRONTEND}" deployed-dev "${F2}"
check_tag "  ...frontend latest on N" "${FRONTEND}" latest "${F2}"

registry "${SERVER}" deployed-dev "${D1}" "${SERVER}" latest "${D1}"
run_cmd SERVER_IMAGE="${SERVER}@${D2}" FRONTEND_IMAGE="" FAIL_ADD_TAG=deployed-dev-previous -- \
  bash -e -o pipefail "${WORK}/publish-step.sh"
check "a refused N-1 tag fails the step" 1 "::error::could not tag the outgoing"
check_tag "  ...and latest is not moved past the failure" "${SERVER}" latest "${D1}"

echo "tag-deployed-digest: deploy-satellite-cloud-run.yml tag-deployed, as the runner runs it"
registry "${REG}/sciotte" deployed-dev "${D1}" "${REG}/sciotte" v0.9.0 "${D1}"
run_cmd IMAGE="${REG}/sciotte@${D2}" -- bash -e -o pipefail "${WORK}/satellite-step.sh"
check "sciotte: the outgoing digest is anchored" 0 "Tagging the outgoing ${REG}/sciotte@${D1}"
check_tag "  ...sciotte N-1 anchored" "${REG}/sciotte" deployed-dev-previous "${D1}"
check_tag "  ...sciotte deployed-dev on N" "${REG}/sciotte" deployed-dev "${D2}"
registry
run_cmd IMAGE="${REG}/photograveur@${D1}" -- bash -e -o pipefail "${WORK}/satellite-step.sh"
check "photograveur's first deploy tags deployed-dev alone" 0 "its first deploy"
check_tag "  ...photograveur deployed-dev on N" "${REG}/photograveur" deployed-dev "${D1}"
run_cmd IMAGE="${REG}/sciotte@${D2}" FAIL_LIST=1 -- bash -e -o pipefail "${WORK}/satellite-step.sh"
check "a refused listing fails the satellite step" 1 "::error::could not list the tags"

echo
[ "${FAILURES}" -eq 0 ] && { echo "all tag-deployed-digest scenarios pass"; exit 0; }
echo "${FAILURES} scenario(s) failed"; exit 1
