#!/usr/bin/env bash
# ABOUTME: Pins terraform-drift.yml — a leg for every Terraform root a workflow can apply, each planning as its root's plan identity
# ABOUTME: Then runs the lifted plan, classify and Slack steps against stubbed terraform/gcloud/curl, offline
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
# Stand-ins. terraform prints the canned plan named by STUB_TF_OUT, exits
# STUB_TF_CODE and records its arguments; gcloud prints STUB_TOKEN as the Slack
# bot token; curl records the payload it was asked to post and answers ok.
# ---------------------------------------------------------------------------
mkdir -p "${WORK}/bin"
cat > "${WORK}/bin/terraform" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$@" > "${STUB_TF_ARGS}"
cat "${STUB_TF_OUT}"
exit "${STUB_TF_CODE}"
SH
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
ERRORED = "\nError: Failed to get existing workspaces: storage: object doesn't exist\n"

def behaviour(work, legs, job):
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
    scenarios = [
        # name, plan exit, plan text, classify passes, marker the classify output must carry
        ("an empty plan", 0, CLEAN, True, "No drift"),
        ("drift (exit 2)", 2, DRIFTED, False, "Terraform drift detected"),
        ("exit 0 over a non-empty plan (wrapper quirk)", 0, DRIFTED, False, "Terraform drift detected"),
        ("a failed plan (exit 1)", 1, ERRORED, False, "Terraform plan failed"),
    ]
    for leg in legs:
        root = leg["root"]
        for name, code, text, passes, marker in scenarios:
            cwd = os.path.join(work, "root"); os.makedirs(cwd, exist_ok=True)
            out, summary, args = (os.path.join(work, f) for f in ("output", "summary", "tf-args"))
            for f in (out, summary):
                open(f, "w").close()
            open(os.path.join(work, "plan.txt"), "w").write(text)
            stub = {"STUB_TF_OUT": os.path.join(work, "plan.txt"), "STUB_TF_CODE": str(code),
                    "STUB_TF_ARGS": args, "GITHUB_OUTPUT": out, "GITHUB_STEP_SUMMARY": summary}
            try:
                rc, log = run_step(plan, dict(resolve_env(plan, leg, {}, {}), **stub), cwd, work)
                outputs = dict(l.split("=", 1) for l in open(out).read().splitlines() if "=" in l)
                crc, clog = run_step(classify, resolve_env(classify, leg, outputs, {}), cwd, work)
            except (KeyError, ValueError) as e:
                bad(f"{root}: {e}")
                return
            label = f"{root}: {name}"
            if rc != 0:
                bad(f"{label} — the plan step itself failed (exit {rc}); it must leave the verdict to classify:\n{log}")
                continue
            if (crc == 0) != passes or marker not in clog:
                bad(f"{label} — classify exited {crc}, expected {'0' if passes else 'non-zero'} with '{marker}':\n{clog}")
                continue
            published = open(summary).read()
            if SECRET in published or SECRET in log:
                bad(f"{label} — raw plan text reached the log or job summary (public repo)")
                continue
            if code == 2 and not ("# google_artifact_registry_repository.images will be updated in-place" in published
                                  and "Plan: 0 to add, 1 to change, 0 to destroy." in published):
                bad(f"{label} — the summary lost the action line or the Plan: line:\n{published}")
                continue
            if code == 1 and "Failed to get existing workspaces" not in published:
                bad(f"{label} — the summary lost the error tail:\n{published}")
                continue
            argv = open(args).read().split("\n")
            want_lock = f"-lock={leg['_want_lock']}"
            if "plan" not in argv or "-detailed-exitcode" not in argv or want_lock not in argv:
                bad(f"{label} — terraform was called as {argv}; want plan -detailed-exitcode {want_lock}")
                continue
            ok(f"{label} -> {'green' if passes else 'red'}, summary condensed, {want_lock}")

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
    behaviour(work, legs, job)
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
