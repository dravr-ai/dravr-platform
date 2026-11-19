#!/usr/bin/env bash
# ABOUTME: Pins terraform-drift.yml — a leg for every Terraform root a workflow can apply, each planning as its root's plan identity
# ABOUTME: Then runs the lifted quiesce, plan, classify and Slack steps against stubbed terraform/gcloud/curl, offline
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The drift monitor is only as good as its coverage, and coverage fails without
# a sound. infra/artifacts had no leg, so the dravr-images cleanup policy was
# replaced with gcloud and ran unseen for twelve days (carnet#137). Three things
# are pinned here:
#
#   1. Coverage. Every root that a workflow job can apply (a `terraform apply`
#      in a job that is not `if: false`) has a drift leg. Prod is excluded by
#      that same rule, not by name: its apply job is disarmed scaffolding.
#   2. Identity. Each leg authenticates as the identity its root's own plan job
#      uses. For infra/artifacts that is the read-only planner; the runner can
#      grant itself roles/owner and must never be minted by an ungated cron.
#   3. Behaviour. The plan and classify steps the runner executes are lifted out
#      and run against a stub terraform: drift (exit 2), the setup-terraform
#      wrapper quirk (exit 0 over a non-empty plan) and a failed plan all red the
#      leg, only "exit 0 and No changes" passes, and the published summary
#      carries action lines but never raw attribute values (public repo). The
#      Slack step is run the same way.
#   4. Settling (carnet#1383). A deploy's canary pins the API's traffic while
#      it ramps, and a plan landing then reads it as drift. A settle job waits
#      for dev Cloud Run to quiesce before the drift job plans, and the dev leg
#      re-plans once after a shorter wait when the API's Cloud Run service is
#      the only resource the first plan changes; only that re-plan is
#      classified, and drift beside it reds at once, without the wait. A wait that times out
#      only warns (the script's ::error lines downgraded to ::warning), so the
#      plan still decides the verdict, and a pin still there after the settle
#      still reds. The quiesce script is stubbed.
#   5. Locking. Only the dev leg's plan holds terraform.yml's terraform-state
#      group: the settle job's long wait holds no group (a run waiting in that
#      group is replaced by the next arrival, carnet#1116), the artifacts leg
#      plans with -lock=false and queues behind nothing, and a failed settle
#      never skips the plan.
#
# The coverage and identity checks run twice more against mutated copies of the
# workflow — a leg removed, a leg minting the artifacts runner — and must fail
# both: a gate that cannot fail is not a gate.
#
# Usage: scripts/ci/terraform-drift.test.sh — bash, python3 (PyYAML) and jq, no network, no GCP.

set -euo pipefail
cd "$(dirname "$0")/../.."
WORK=$(mktemp -d); trap 'rm -rf "${WORK}"' EXIT

# ---------------------------------------------------------------------------
# Stand-ins. terraform's n-th call prints the n-th canned plan named in
# STUB_TF_OUT and exits the n-th code in STUB_TF_CODE (both space-separated; the
# last one repeats), counts its calls in STUB_TF_CALLS and records its
# arguments; the quiesce script records each call in STUB_WAIT_LOG and exits
# STUB_WAIT_CODE, printing '::error::' to stderr on failure as its die() does;
# gcloud prints STUB_TOKEN as the Slack bot token; curl records
# the payload it was asked to post and answers ok.
# ---------------------------------------------------------------------------
mkdir -p "${WORK}/bin" "${WORK}/ws/scripts/ci"
cat > "${WORK}/bin/terraform" <<'SH'
#!/usr/bin/env bash
n=$(( $(cat "${STUB_TF_CALLS}" 2>/dev/null || echo 0) + 1 ))
echo "${n}" > "${STUB_TF_CALLS}"
read -ra outs <<< "${STUB_TF_OUT}"
read -ra codes <<< "${STUB_TF_CODE}"
i=$(( n < ${#outs[@]} ? n - 1 : ${#outs[@]} - 1 ))
printf '%s\n' "$@" > "${STUB_TF_ARGS}"
cat "${outs[${i}]}"
exit "${codes[${i}]}"
SH
cat > "${WORK}/ws/scripts/ci/wait-cloud-run-service-ready.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${STUB_WAIT_LOG}"
echo "Waiting until no dev Cloud Run service is mid-rollout"
code="${STUB_WAIT_CODE:-0}"
[ "${code}" = 0 ] || echo "::error::stub quiesce timed out" >&2
exit "${code}"
SH
chmod +x "${WORK}/ws/scripts/ci/wait-cloud-run-service-ready.sh"
cat > "${WORK}/bin/gcloud" <<'SH'
#!/usr/bin/env bash
printf '%s' "${STUB_TOKEN}"
SH
cat > "${WORK}/bin/curl" <<'SH'
#!/usr/bin/env bash
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--data" ]; then printf '%s' "$2" > "${STUB_CURL_PAYLOAD}"; fi
  shift
done
printf '{"ok":true}'
SH
chmod +x "${WORK}/bin/terraform" "${WORK}/bin/gcloud" "${WORK}/bin/curl"

cat > "${WORK}/harness.py" <<'PY'
import copy, glob, os, re, subprocess, sys, yaml

DRIFT_WF = "terraform-drift.yml"
PLAN_STEP = "Terraform Plan (drift check)"
INIT_STEP = "Terraform Init"
QUIESCE_STEP = "Wait until no dev Cloud Run service is mid-rollout"
QUIESCE_TIMEOUT = "--timeout-seconds 1800"
REPLAN_TIMEOUT = "--timeout-seconds 900"
STATE_GROUP = "terraform-state"
CLOUD_RUN = "module.backend.google_cloud_run_v2_service.service"
CLASSIFY_STEP = "Classify drift result"
SLACK_STEP = "Post to Slack"
TF_CMD = re.compile(r"(?m)^\s*terraform\s+(?:-chdir=(\S+)\s+)?(plan|apply)\b")
SECRET_REF = re.compile(r"secrets\.([A-Za-z0-9_]+)|secrets\[matrix\.([A-Za-z0-9_]+)\]")

failures = 0
def ok(msg): print(f"  ok   {msg}")
def bad(msg):
    global failures
    failures += 1
    print(f"  FAIL {msg}")

def load(path):
    return yaml.safe_load(open(path))

def runs(job):
    return "\n".join(s.get("run", "") for s in job.get("steps") or [])

def disarmed(job):
    # PyYAML reads `if: false` as the boolean.
    return job.get("if") in (False, "false", "${{ false }}")

def job_root(job, chdir):
    if chdir:
        return chdir.strip("'\"")
    return ((job.get("defaults") or {}).get("run") or {}).get("working-directory")

def auth_secrets(job, leg=None):
    """(provider secret, service-account secret) named by the job's GCP auth step."""
    for step in job.get("steps") or []:
        if str(step.get("uses", "")).startswith("google-github-actions/auth@"):
            names = []
            for key in ("workload_identity_provider", "service_account"):
                m = SECRET_REF.search(str((step.get("with") or {}).get(key, "")))
                if not m:
                    names.append(None)
                elif m.group(1):
                    names.append(m.group(1))
                else:
                    names.append((leg or {}).get(m.group(2)))
            return tuple(names)
    return (None, None)

def drift_job(wf):
    for job_id, job in (wf.get("jobs") or {}).items():
        if any(s.get("name") == PLAN_STEP for s in job.get("steps") or []):
            return job_id, job
    return None, None

def check_contract(workflows_dir):
    """Coverage and identity problems for the drift workflow in workflows_dir."""
    problems = []
    # The workflow files are the data under test here: which roots a job can
    # apply and which identity plans each root are declared nowhere else, and
    # no runtime joins the drift monitor to the apply lanes.
    applyable, plan_ids = {}, {}
    for path in sorted(glob.glob(os.path.join(workflows_dir, "*.yml"))):
        if os.path.basename(path) == DRIFT_WF:
            continue
        for job_id, job in (load(path).get("jobs") or {}).items():
            where = f"{os.path.basename(path)}:{job_id}"
            cmds = TF_CMD.findall(runs(job))
            verbs = {verb for _, verb in cmds}
            for chdir, verb in cmds:
                root = job_root(job, chdir)
                if root is None:
                    problems.append(f"{where} runs terraform {verb} with no resolvable root directory")
                    continue
                if verb == "apply" and not disarmed(job) and where not in applyable.get(root, []):
                    applyable.setdefault(root, []).append(where)
                if verb == "plan" and "apply" not in verbs:
                    lock = "false" if re.search(r"(?m)^\s*terraform\s+plan\b.*-lock=false", runs(job)) else "true"
                    plan_ids.setdefault(root, set()).add(auth_secrets(job) + (lock,))
    if not applyable:
        problems.append("no workflow job applies any Terraform root — the scan found nothing to check")

    wf = load(os.path.join(workflows_dir, DRIFT_WF))
    job_id, job = drift_job(wf)
    if job is None:
        return problems + [f"{DRIFT_WF} has no job with a '{PLAN_STEP}' step"], [], None
    legs = ((job.get("strategy") or {}).get("matrix") or {}).get("include") or []
    covered = {leg.get("root") for leg in legs}
    for root, jobs in sorted(applyable.items()):
        if root not in covered:
            problems.append(f"{root} can be applied ({', '.join(jobs)}) but {DRIFT_WF} has no leg planning it")
    for leg in legs:
        root = leg.get("root")
        ids = plan_ids.get(root, set())
        if len(ids) != 1:
            problems.append(f"leg {root}: expected one plan job identity for the root, found {sorted(map(str, ids)) or 'none'}")
            continue
        want_provider, want_sa, want_lock = next(iter(ids))
        provider, sa = auth_secrets(job, leg)
        if (provider, sa) != (want_provider, want_sa):
            problems.append(f"leg {root} authenticates as ({provider}, {sa}); the root's plan job uses ({want_provider}, {want_sa})")
        leg["_want_lock"] = want_lock
    return problems, legs, (job_id, job, wf)

def resolve_env(step, leg, outputs, given):
    env = {}
    for name, value in (step.get("env") or {}).items():
        value = str(value)
        m = re.fullmatch(r"\$\{\{\s*(\S+)\s*\}\}", value)
        if "${{" not in value:
            env[name] = value
        elif m and m.group(1).startswith("matrix."):
            env[name] = str(leg[m.group(1)[len("matrix."):]])
        elif m and m.group(1).startswith("steps.plan.outputs."):
            env[name] = outputs.get(m.group(1)[len("steps.plan.outputs."):], "")
        elif name in given:
            env[name] = given[name]
        else:
            raise KeyError(f"step '{step.get('name')}' env {name}={value} has no stand-in")
    return env

def run_step(step, env, cwd, work):
    if "${{" in step["run"]:
        raise ValueError(f"step '{step.get('name')}' interpolates an expression into its script; move it to env")
    script = os.path.join(work, "step.sh")
    open(script, "w").write(step["run"])
    full = dict(os.environ, **env)
    full["PATH"] = os.path.join(work, "bin") + os.pathsep + os.environ["PATH"]
    # GitHub's default `run` shell when none is named: bash -e, no pipefail.
    proc = subprocess.run(["bash", "-e", script], cwd=cwd, env=full, capture_output=True, text=True)
    return proc.returncode, proc.stdout + proc.stderr

CLEAN = "\nNo changes. Your infrastructure matches the configuration.\n"
SECRET = "s3cr3t-value-in-plan"
DRIFTED = f"""
Terraform will perform the following actions:

  # google_artifact_registry_repository.images will be updated in-place
  ~ resource "google_artifact_registry_repository" "images" {{
      ~ description = "{SECRET}" -> "Central Docker repository"
    }}

Plan: 0 to add, 1 to change, 0 to destroy.
"""
CLOUD_RUN_DRIFTED = f"""
Terraform will perform the following actions:

  # {CLOUD_RUN} will be updated in-place
  ~ resource "google_cloud_run_v2_service" "service" {{
      ~ traffic {{
          ~ percent  = 99 -> 100
          - revision = "{SECRET}" -> null
        }}
    }}

Plan: 0 to add, 1 to change, 0 to destroy.
"""
# The Cloud Run move beside a change no settle undoes: the action lines of both, one Plan: line.
CLOUD_RUN_AND_DRIFTED = (CLOUD_RUN_DRIFTED.rsplit("Plan:", 1)[0]
                         + DRIFTED.split("actions:\n", 1)[1].replace("1 to change", "2 to change"))
ERRORED = "\nError: Failed to get existing workspaces: storage: object doesn't exist\n"
# The dev vars a quiesce wait reads, standing in for ${{ vars.* }}.
DEV_VARS = {"REGION": "us-test1", "PREFIX": "dravr-test"}

def stub_env(work, plans, wait_code):
    """Fresh stand-in state; plans is [(exit code, plan text)], one per terraform call."""
    names = ("output", "summary", "tf-args", "tf-calls", "wait-log")
    out, summary, args, calls, wait_log = (os.path.join(work, f) for f in names)
    for f in (out, summary, wait_log):
        open(f, "w").close()
    if os.path.exists(calls):
        os.remove(calls)
    files = []
    for n, (_, text) in enumerate(plans):
        f = os.path.join(work, f"plan{n}.txt")
        open(f, "w").write(text)
        files.append(f)
    return {"STUB_TF_OUT": " ".join(files), "STUB_TF_CODE": " ".join(str(c) for c, _ in plans),
            "STUB_TF_ARGS": args, "STUB_TF_CALLS": calls, "STUB_WAIT_LOG": wait_log,
            "STUB_WAIT_CODE": str(wait_code), "GITHUB_OUTPUT": out, "GITHUB_STEP_SUMMARY": summary,
            "GITHUB_WORKSPACE": os.path.join(work, "ws")}

def stub_calls(stub):
    tf = int(open(stub["STUB_TF_CALLS"]).read().strip()) if os.path.exists(stub["STUB_TF_CALLS"]) else 0
    waits = [l for l in open(stub["STUB_WAIT_LOG"]).read().splitlines() if l]
    return tf, waits

def run_leg(work, leg, plan, classify, plans, wait_code=0):
    """Run the plan then the classify step as the runner would; raises on an unresolvable env."""
    cwd = os.path.join(work, "root"); os.makedirs(cwd, exist_ok=True)
    stub = stub_env(work, plans, wait_code)
    rc, log = run_step(plan, dict(resolve_env(plan, leg, {}, DEV_VARS), **stub), cwd, work)
    outputs = dict(l.split("=", 1) for l in open(stub["GITHUB_OUTPUT"]).read().splitlines() if "=" in l)
    crc, clog = run_step(classify, resolve_env(classify, leg, outputs, {}), cwd, work)
    tf, waits = stub_calls(stub)
    return {"rc": rc, "log": log, "crc": crc, "clog": clog, "summary": open(stub["GITHUB_STEP_SUMMARY"]).read(),
            "argv": open(stub["STUB_TF_ARGS"]).read().split("\n"), "tf": tf, "waits": waits}

def strip_expr(value):
    return re.sub(r"^\$\{\{\s*|\s*\}\}$", "", str(value).strip())

def leg_group(group, leg):
    """The concurrency group a leg resolves to, for the one expression shape this lane uses:
    `matrix.label == '<label>' && '<group>' || <fallback>`; a plain string is every leg's group."""
    expr = strip_expr(group)
    if "${{" not in str(group):
        return str(group)
    m = re.fullmatch(r"matrix\.label == '([^']+)' && '([^']+)' \|\| (.+)", expr)
    if not m:
        return None
    if leg.get("label") == m.group(1):
        return m.group(2)
    # The fallback must be unique per run, so a leg outside the state group never
    # queues behind (or replaces) another run's.
    return f"<per-run {m.group(3)}>" if "github.run_id" in m.group(3) else m.group(3)

def warn_only(log, where):
    """A wait that only warns must leave no ::error annotation on a run that may go green."""
    if "::error" in log:
        bad(f"{where}: the quiesce script's ::error reached the log of a warn-only wait:\n{log}")
        return False
    return True

def quiesce_pins(work, legs, wf, drift_id, job):
    """A settle job outside terraform-state waits for Cloud Run before the plan; the wait only warns."""
    jobs = wf.get("jobs") or {}
    if "concurrency" in wf:
        bad(f"the workflow-level concurrency {wf['concurrency']!r} holds the group for every job, settle's wait included")
    else:
        ok("no workflow-level concurrency: only a job that plans holds a group")
    settles = [(jid, j) for jid, j in jobs.items() if any(s.get("name") == QUIESCE_STEP for s in j.get("steps") or [])]
    if len(settles) != 1 or settles[0][0] == drift_id:
        bad(f"expected one job other than {drift_id} with a '{QUIESCE_STEP}' step, found {[j for j, _ in settles]}")
        return
    settle_id, settle = settles[0]
    if "concurrency" in settle or "strategy" in settle:
        bad(f"{settle_id} must be one job outside any concurrency group, so its wait holds no lock")
    else:
        ok(f"{settle_id} is one job in no concurrency group")
    needs = job.get("needs")
    needs = [needs] if isinstance(needs, str) else (needs or [])
    cond = strip_expr(job.get("if", ""))
    if settle_id in needs and cond in ("!cancelled()", "always()"):
        ok(f"{drift_id} runs after {settle_id} whatever its result (if: {cond})")
    else:
        bad(f"{drift_id} must need {settle_id} and run on `if: ${{{{ !cancelled() }}}}`, has needs={needs} if={job.get('if')!r}")
    group = (job.get("concurrency") or {}).get("group")
    cancel = (job.get("concurrency") or {}).get("cancel-in-progress")
    groups = {l.get("label"): leg_group(group, l) for l in legs}
    others = [g for lbl, g in groups.items() if lbl != "dev"]
    if groups.get("dev") == STATE_GROUP and cancel is False and all(
            g and g != STATE_GROUP and g.startswith("<per-run ") for g in others):
        ok(f"only the dev leg queues in {STATE_GROUP}, never cancelling; the others get a per-run group: {groups}")
    else:
        bad(f"the dev leg alone must hold {STATE_GROUP} (cancel-in-progress: false), the others a per-run group; "
            f"got {groups}, cancel-in-progress={cancel!r}")
    drift_timeout, settle_timeout = job.get("timeout-minutes"), settle.get("timeout-minutes")
    if isinstance(drift_timeout, int) and 20 <= drift_timeout <= 30:
        ok(f"{drift_id}'s {drift_timeout}-minute timeout covers one 15-minute re-plan wait and two plans")
    else:
        bad(f"{drift_id} timeout-minutes {drift_timeout!r}: want 20-30, one 15-minute wait and two plans")
    if isinstance(settle_timeout, int) and 31 <= settle_timeout <= 40:
        ok(f"{settle_id}'s {settle_timeout}-minute timeout covers one 30-minute wait")
    else:
        bad(f"{settle_id} timeout-minutes {settle_timeout!r}: want 31-40, one 30-minute wait")
    dev = next((l for l in legs if l.get("label") == "dev"), None)
    if dev is None:
        bad("no dev leg to settle for")
        return
    step = next(s for s in settle["steps"] if s.get("name") == QUIESCE_STEP)
    for wait_code, name in ((0, "a settled dev"), (1, "a quiesce timeout")):
        stub = stub_env(work, [(0, CLEAN)], wait_code)
        try:
            rc, log = run_step(step, dict(resolve_env(step, dev, {}, DEV_VARS), **stub), work, work)
        except (KeyError, ValueError) as e:
            bad(f"quiesce: {e}")
            return
        _, waits = stub_calls(stub)
        want = f"quiesce --region {DEV_VARS['REGION']} --prefix {DEV_VARS['PREFIX']} {QUIESCE_TIMEOUT}"
        if waits != [want]:
            bad(f"quiesce on {name}: called the wait as {waits}, want [{want!r}] (no --db-headroom: a plan rolls nothing)")
        elif rc != 0:
            bad(f"quiesce on {name}: the step exited {rc}; a wait that fails must leave the verdict to the plan:\n{log}")
        elif "Waiting until" not in log:
            bad(f"quiesce on {name}: the script's progress (stdout) never reached the log:\n{log}")
        elif wait_code and "::warning::stub quiesce timed out" not in log:
            bad(f"quiesce on {name}: the script's own message was lost; it must come back as a warning:\n{log}")
        elif warn_only(log, f"quiesce on {name}"):
            ok(f"quiesce on {name}: waits {QUIESCE_TIMEOUT}, then {'warns (no ::error) and ' if wait_code else ''}lets the plan run")

def behaviour(work, legs, wf, drift_id, job):
    steps = {s.get("name"): s for s in job.get("steps") or []}
    plan, classify = steps.get(PLAN_STEP), steps.get(CLASSIFY_STEP)
    if not plan or not classify:
        bad(f"the drift job lacks '{PLAN_STEP}' or '{CLASSIFY_STEP}'")
        return
    setup = next((s for s in job["steps"] if str(s.get("uses", "")).startswith("hashicorp/setup-terraform@")), {})
    if str((setup.get("with") or {}).get("terraform_wrapper")).lower() == "false":
        ok("setup-terraform runs without its wrapper, so exit code 2 reaches the plan step")
    else:
        bad("setup-terraform must set terraform_wrapper: false — the wrapper turns exit 2 into 0")
    quiesce_pins(work, legs, wf, drift_id, job)
    scenarios = [
        # name, [(plan exit, plan text) per terraform call], quiesce exit, legs,
        # classify passes, marker the classify output must carry, terraform calls, waits in the plan step
        ("an empty plan", [(0, CLEAN)], 0, None, True, "No drift", 1, 0),
        ("drift (exit 2)", [(2, DRIFTED)], 0, None, False, "Terraform drift detected", 1, 0),
        ("exit 0 over a non-empty plan (wrapper quirk)", [(0, DRIFTED)], 0, None, False, "Terraform drift detected", 1, 0),
        ("a failed plan (exit 1)", [(1, ERRORED)], 0, None, False, "Terraform plan failed", 1, 0),
        ("Cloud Run drift, then a clean re-plan", [(2, CLOUD_RUN_DRIFTED), (0, CLEAN)], 0, {"dev"}, True, "No drift", 2, 1),
        ("Cloud Run drift in both plans", [(2, CLOUD_RUN_DRIFTED), (2, CLOUD_RUN_DRIFTED)], 0, {"dev"}, False, CLOUD_RUN, 2, 1),
        ("Cloud Run drift, a quiesce timeout, then a clean re-plan", [(2, CLOUD_RUN_DRIFTED), (0, CLEAN)], 1, {"dev"}, True, "No drift", 2, 1),
        # Drift a settle cannot undo beside the Cloud Run move: red at once, no wait held in terraform-state.
        ("Cloud Run drift beside other drift, never re-planned", [(2, CLOUD_RUN_AND_DRIFTED), (0, CLEAN)], 0, {"dev"}, False, "Terraform drift detected", 1, 0),
        ("Cloud Run drift, a re-plan that fails (exit 1)", [(2, CLOUD_RUN_DRIFTED), (1, ERRORED)], 0, {"dev"}, False, "Terraform plan failed", 2, 1),
        # Only the dev root holds the API's Cloud Run service; the other legs never wait or re-plan.
        ("Cloud Run drift, never re-planned off the dev leg", [(2, CLOUD_RUN_DRIFTED), (0, CLEAN)], 0, "not dev", False, CLOUD_RUN, 1, 0),
    ]
    for leg in legs:
        root = leg["root"]
        for name, plans, wait_code, only, passes, marker, want_tf, want_waits in scenarios:
            if only == "not dev":
                if leg.get("label") == "dev":
                    continue
            elif only is not None and leg.get("label") not in only:
                continue
            label = f"{root}: {name}"
            try:
                r = run_leg(work, leg, plan, classify, plans, wait_code)
            except (KeyError, ValueError) as e:
                bad(f"{root}: {e}")
                return
            if r["rc"] != 0:
                bad(f"{label} — the plan step itself failed (exit {r['rc']}); it must leave the verdict to classify:\n{r['log']}")
                continue
            if (r["crc"] == 0) != passes or marker not in r["clog"]:
                bad(f"{label} — classify exited {r['crc']}, expected {'0' if passes else 'non-zero'} with '{marker}':\n{r['clog']}")
                continue
            if (r["tf"], len(r["waits"])) != (want_tf, want_waits):
                bad(f"{label} — terraform ran {r['tf']} time(s) and the plan step waited {len(r['waits'])} time(s); "
                    f"want {want_tf} and {want_waits}")
                continue
            if want_waits and r["waits"] != [f"quiesce --region {DEV_VARS['REGION']} --prefix {DEV_VARS['PREFIX']} {REPLAN_TIMEOUT}"]:
                bad(f"{label} — the re-plan waited as {r['waits']}; want a {REPLAN_TIMEOUT} quiesce without --db-headroom")
                continue
            published = r["summary"]
            if SECRET in published or SECRET in r["log"]:
                bad(f"{label} — raw plan text reached the log or job summary (public repo)")
                continue
            replanned = want_tf == 2
            if replanned and not ("Re-planned:" in published and CLOUD_RUN in published
                                  and f"# {CLOUD_RUN} will be updated in-place" in published):
                bad(f"{label} — the summary does not say the plan was re-planned and why:\n{published}")
                continue
            if not replanned and "Re-planned:" in published:
                bad(f"{label} — the summary claims a re-plan that never ran:\n{published}")
                continue
            if wait_code and replanned and "::warning::stub quiesce timed out" not in r["log"]:
                bad(f"{label} — the failed re-plan wait lost the script's message; it must come back as a warning:\n{r['log']}")
                continue
            if not warn_only(r["log"], label):
                continue
            last_code, last_text = plans[min(want_tf, len(plans)) - 1]
            if last_code == 2 and last_text is DRIFTED and not (
                    "# google_artifact_registry_repository.images will be updated in-place" in published
                    and "Plan: 0 to add, 1 to change, 0 to destroy." in published):
                bad(f"{label} — the summary lost the action line or the Plan: line:\n{published}")
                continue
            if last_code == 1 and "Failed to get existing workspaces" not in published:
                bad(f"{label} — the summary lost the error tail:\n{published}")
                continue
            want_lock = f"-lock={leg['_want_lock']}"
            if "plan" not in r["argv"] or "-detailed-exitcode" not in r["argv"] or want_lock not in r["argv"]:
                bad(f"{label} — terraform was called as {r['argv']}; want plan -detailed-exitcode {want_lock}")
                continue
            ok(f"{label} -> {'green' if passes else 'red'}, {want_tf} plan(s), summary condensed, {want_lock}")

def slack(work, wf, drift_id):
    jobs = wf.get("jobs") or {}
    found = [(jid, j) for jid, j in jobs.items()
             if any(s.get("name") == SLACK_STEP for s in j.get("steps") or [])]
    if len(found) != 1:
        bad(f"expected exactly one job with a '{SLACK_STEP}' step, found {len(found)}")
        return
    jid, job = found[0]
    needs = job.get("needs")
    needs = [needs] if isinstance(needs, str) else (needs or [])
    if drift_id in needs and "strategy" not in job:
        ok(f"{jid} posts once, after every drift leg ({jid} needs {drift_id})")
    else:
        bad(f"{jid} must be a single job that needs {drift_id}, so one run posts once")
    step = next(s for s in job["steps"] if s.get("name") == SLACK_STEP)
    cases = [
        ("drift", {"DRIFT_RESULT": "failure", "IS_TEST": ""}, "tok", "Terraform drift detected"),
        ("a test dispatch", {"DRIFT_RESULT": "success", "IS_TEST": "true"}, "tok", "test alert"),
        ("drift on a test dispatch", {"DRIFT_RESULT": "failure", "IS_TEST": "true"}, "tok", "Terraform drift detected"),
        ("an unreadable token", {"DRIFT_RESULT": "failure", "IS_TEST": ""}, "", None),
    ]
    for name, given, token, want in cases:
        payload = os.path.join(work, "payload")
        if os.path.exists(payload):
            os.remove(payload)
        given = dict(given, RUN_URL="https://github.test/run/1")
        try:
            env = dict(resolve_env(step, {}, {}, given), STUB_TOKEN=token, STUB_CURL_PAYLOAD=payload)
            rc, log = run_step(step, env, work, work)
        except (KeyError, ValueError) as e:
            bad(f"slack: {e}")
            return
        posted = open(payload).read() if os.path.exists(payload) else None
        if rc != 0:
            bad(f"slack on {name}: the post exited {rc}; it must never fail the run:\n{log}")
        elif want is None and posted is None and "could not read" in log:
            ok(f"slack on {name}: warns, posts nothing, exits 0")
        elif want is not None and posted and want in posted and "#dev-dravr-errors" in posted:
            ok(f"slack on {name}: posts '{want}' to #dev-dravr-errors")
        else:
            bad(f"slack on {name}: posted {posted!r}, log:\n{log}")

def mutated(work, name, mutate):
    """A copy of .github/workflows with the drift workflow mutated in memory."""
    d = os.path.join(work, name); os.makedirs(d)
    for path in glob.glob(".github/workflows/*.yml"):
        if os.path.basename(path) != DRIFT_WF:
            os.symlink(os.path.abspath(path), os.path.join(d, os.path.basename(path)))
    wf = load(os.path.join(".github/workflows", DRIFT_WF))
    _, job = drift_job(wf)
    mutate(job["strategy"]["matrix"]["include"])
    yaml.safe_dump(wf, open(os.path.join(d, DRIFT_WF), "w"))
    return d

def main(work):
    print("terraform-drift: coverage and identity")
    problems, legs, found = check_contract(".github/workflows")
    for p in problems:
        bad(p)
    if not problems:
        ok("every applyable root has a leg: " + ", ".join(sorted(l["root"] for l in legs)))
        ok("each leg plans as its root's plan-job identity")
    if not legs:
        bad(f"{DRIFT_WF} declares no matrix legs, so nothing below can run")
        return

    print("terraform-drift: the contract check fails on a broken workflow")
    no_artifacts = mutated(work, "no-artifacts", lambda legs: legs.__setitem__(
        slice(None), [l for l in legs if l["root"] != "infra/artifacts"]))
    if any("infra/artifacts can be applied" in p for p in check_contract(no_artifacts)[0]):
        ok("a missing infra/artifacts leg is reported")
    else:
        bad("removing the infra/artifacts leg went unreported")
    runner = mutated(work, "runner-identity", lambda legs: [l.__setitem__(
        "service_account_secret", "GCP_ARTIFACTS_TF_SA") for l in legs if l["root"] == "infra/artifacts"])
    if any("leg infra/artifacts authenticates as" in p for p in check_contract(runner)[0]):
        ok("an artifacts leg minting the owner-capable runner is reported")
    else:
        bad("an artifacts leg authenticating as the runner went unreported")

    if found is None:
        return
    drift_id, job, wf = found
    print("terraform-drift: plan + classify, per leg")
    behaviour(work, legs, wf, drift_id, job)
    print("terraform-drift: Slack post")
    slack(work, wf, drift_id)

main(sys.argv[1])
print()
if failures:
    print(f"{failures} terraform-drift check(s) failed")
    sys.exit(1)
print("all terraform-drift checks pass")
PY

python3 "${WORK}/harness.py" "${WORK}"
