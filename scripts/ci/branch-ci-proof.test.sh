#!/usr/bin/env bash
# ABOUTME: Fixture test for branch-ci-proof.sh — a stub gh (or curl, or cargo) on PATH serves canned answers,
# ABOUTME: and each case asserts a tier is proven only by a completed push run of HEAD's tree whose covering steps passed
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The pre-push gate skips a compile tier on `check` exiting 0, so a proof that passes
# wrongly drops that tier from the gate with nobody having run it on those bytes. Every
# rule that keeps a run, a job or a step out of the proof has a case here, and so does
# every way the lookup itself can fail.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/branch-ci-proof.sh}"
WORKFLOWS="${WORKFLOWS:-$SCRIPT_DIR/../../.github/workflows}"

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT
FIX="$ROOT/fixtures"
mkdir -p "$FIX" "$ROOT/bin"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

# The stub answers the two endpoints the script calls from files in $FIX. $FIX/fail names
# an endpoint key that answers HTTP 500, $FIX/ratelimit makes every call a rate-limit
# refusal, $FIX/sleep holds every answer that many seconds, $FIX/calls counts the calls.
cat >"$ROOT/bin/gh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[ "$1" = api ] || { echo "stub gh: unexpected $*" >&2; exit 1; }
url="$2"
echo "$url" >>"$FIX/calls"
case "$url" in
  */actions/runs\?head_sha=*) sha="${url#*head_sha=}"; sha="${sha%%&*}"; key="runs-$sha" ;;
  */actions/runs/*/jobs\?*) id="${url#*/actions/runs/}"; id="${id%%/*}"; key="jobs-$id" ;;
  *) echo "stub gh: unknown endpoint $url" >&2; exit 1 ;;
esac
if [ -f "$FIX/sleep" ]; then sleep "$(cat "$FIX/sleep")"; fi
if [ -f "$FIX/ratelimit" ]; then echo "gh: API rate limit exceeded for installation (HTTP 403)" >&2; exit 1; fi
if [ -f "$FIX/fail" ] && grep -qx "$key" "$FIX/fail"; then echo "gh: Server Error (HTTP 500)" >&2; exit 1; fi
if [ -f "$FIX/$key.json" ]; then cat "$FIX/$key.json"; else echo '{"total_count":0,"workflow_runs":[],"jobs":[]}'; fi
STUB
chmod +x "$ROOT/bin/gh"

# curl takes the same files: the script calls https://api.github.com/<endpoint>.
cat >"$ROOT/bin/curl" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
url="${*: -1}"
exec "$(dirname "$0")/gh" api "${url#https://api.github.com/}"
STUB
chmod +x "$ROOT/bin/curl"

# A repo whose branch is pushed to a bare origin, so refs/remotes/origin/feature/work
# holds HEAD's tree, the way a pushed branch does.
make_repo() {
  local dir origin
  origin="$(mktemp -d "$ROOT/origin-XXXXXX")"
  git -C "$origin" init -q --bare
  dir="$(mktemp -d "$ROOT/case-XXXXXX")"
  git -C "$dir" init -q -b feature/work
  git -C "$dir" config user.email test@example.com
  git -C "$dir" config user.name test
  git -C "$dir" config commit.gpgsign false
  printf 'work\n' >"$dir/work.txt"
  git -C "$dir" add -A
  git -C "$dir" commit -qm work
  git -C "$dir" remote add origin "$origin"
  # push.negotiate (set globally on some machines) fails against a local bare repo.
  git -C "$dir" -c push.negotiate=false push -q origin feature/work
  printf '%s\n' "$dir"
}

reset_fixtures() { rm -rf "$FIX"; mkdir -p "$FIX"; }

runs() { # $1 = sha, then each run as id:status:tree[:event[:workflow]]
  local sha="$1" items=() id status tree event wf
  shift
  for r in "$@"; do
    IFS=: read -r id status tree event wf <<<"$r"
    items+=("{\"id\":$id,\"event\":\"${event:-push}\",\"status\":\"$status\",\"head_sha\":\"$sha\",\"created_at\":\"2026-11-25T00:00:${id: -2}Z\",\"path\":\".github/workflows/${wf:-ci-backend.yml}\",\"head_commit\":{\"tree_id\":\"$tree\"}}")
  done
  local IFS=,
  printf '{"total_count":%d,"workflow_runs":[%s]}\n' "$#" "${items[*]}" >"$FIX/runs-$sha.json"
}

jobs() { # $1 = run id, then name=conclusion, each optionally followed by |step=conclusion entries
  local id="$1" items=() pair job name concl status steps step rest
  shift
  for pair in "$@"; do
    job="${pair%%|*}"; name="${job%=*}"; concl="${job##*=}"; steps=()
    if [ "$job" != "$pair" ]; then
      IFS='|' read -ra rest <<<"${pair#*|}"
      for step in "${rest[@]}"; do steps+=("{\"name\":\"${step%=*}\",\"conclusion\":\"${step##*=}\"}"); done
    fi
    status=completed
    if [ "$concl" = in_progress ]; then status=in_progress; concl=null; else concl="\"$concl\""; fi
    local IFS=,
    items+=("{\"name\":\"$name\",\"status\":\"$status\",\"conclusion\":$concl,\"steps\":[${steps[*]}]}")
    unset IFS
  done
  local IFS=,
  printf '{"total_count":%d,"jobs":[%s]}\n' "$#" "${items[*]}" >"$FIX/jobs-$id.json"
}

CLIPPY="Clippy Linting (full workspace)"
LINT="Lint with Clippy (Zero Tolerance + Cognitive Complexity)"

# Runs the script in $dir with the stubs first on PATH; sets OUT and CODE.
check() { # $1 = dir, then the script's arguments
  local dir="$1"
  shift
  CODE=0
  OUT="$(cd "$dir" && PATH="$ROOT/bin:$PATH" FIX="$FIX" BRANCH_CI_PROOF_REPO=o/r "$UNDER_TEST" "$@" 2>&1)" || CODE=$?
}

expect() { # $1 = label, $2 = expected exit, $3 = literal the output must carry
  if [ "$CODE" != "$2" ]; then fail "$1 (exit $CODE, expected $2: $OUT)"
  elif [[ "$OUT" != *"$3"* ]]; then fail "$1 (output '$OUT' is missing: $3)"
  else pass "$1"; fi
}

echo "branch-ci-proof.sh"

dir="$(make_repo)"
SHA="$(git -C "$dir" rev-parse HEAD)"
TREE="$(git -C "$dir" rev-parse 'HEAD^{tree}')"
OTHER_TREE=0000000000000000000000000000000000000001

echo "  case 1: a completed push run of HEAD's tree passed the covering job and step"
reset_fixtures
runs "$SHA" "101:completed:$TREE"
jobs 101 "$CLIPPY=success|$LINT=success"
check "$dir" check 1e
expect "the tier is proven and the line names sha, ref, run and job" 0 \
  "proven by branch CI: ${SHA:0:12} (origin/feature/work) run 101, ci-backend.yml › $CLIPPY"

echo "  case 2: the covering job failed"
reset_fixtures
runs "$SHA" "102:completed:$TREE"
jobs 102 "$CLIPPY=failure|$LINT=failure"
check "$dir" check 1e
expect "a failed job proves nothing" 1 "no proof: run 102: job '$CLIPPY' concluded failure"

echo "  case 2b: a failed run of the same tree refuses a green one"
reset_fixtures
runs "$SHA" "110:completed:$TREE" "111:completed:$TREE"
jobs 110 "$CLIPPY=success|$LINT=success"
jobs 111 "$CLIPPY=failure|$LINT=failure"
check "$dir" check 1e
expect "any failure of a covering job on this tree vetoes the proof" 1 "run 111: job '$CLIPPY' concluded failure"

echo "  case 2c: a later covering job failed in a run whose first requirement proves nothing"
reset_fixtures
runs "$SHA" "112:completed:$TREE:push:frontend-tests.yml" "113:completed:$TREE:push:frontend-tests.yml"
jobs 112 "Frontend Test Suite=success|Lint code=skipped|Type check=success|Run tests=success" \
  "Playwright E2E Tests=failure|Run Playwright E2E tests=failure"
jobs 113 "Frontend Test Suite=success|Lint code=success|Type check=success|Run tests=success" \
  "Playwright E2E Tests=success|Run Playwright E2E tests=success"
check "$dir" check 5
expect "a failed covering job vetoes whatever order the requirements are judged in" 1 \
  "run 112: job 'Playwright E2E Tests' concluded failure"

echo "  case 3: the covering job was skipped"
reset_fixtures
runs "$SHA" "103:completed:$TREE"
jobs 103 "Feature Profile Check (no-default-features)=skipped"
check "$dir" check 1f
expect "a skipped job proves nothing" 1 "job 'Feature Profile Check (no-default-features)' concluded skipped"

echo "  case 3b: the job passed but its work step was skipped"
reset_fixtures
runs "$SHA" "104:completed:$TREE"
jobs 104 "$CLIPPY=success|Detect file changes=success|$LINT=skipped"
check "$dir" check 1e
expect "a job green over a skipped work step proves nothing" 1 "step '$CLIPPY › $LINT' is skipped"

echo "  case 3c: the covering job is absent from the run"
reset_fixtures
runs "$SHA" "105:completed:$TREE"
jobs 105 "Fast Gate (fmt + architecture + secrets + surfaces)=success"
check "$dir" check 1e
expect "an absent job proves nothing" 1 "job '$CLIPPY' is absent"

echo "  case 4: the run is still in progress"
reset_fixtures
runs "$SHA" "106:in_progress:$TREE"
check "$dir" check 1e
expect "an incomplete run proves nothing" 1 "run 106 (ci-backend.yml, ${SHA:0:12}) is in_progress"
if grep -q 'jobs' "$FIX/calls"; then fail "an incomplete run's jobs were read"; else pass "an incomplete run's jobs are never read"; fi

echo "  case 4b: a completed run whose covering job is still running"
reset_fixtures
runs "$SHA" "107:completed:$TREE"
jobs 107 "$CLIPPY=in_progress"
check "$dir" check 1e
expect "a running job proves nothing" 1 "job '$CLIPPY' is in_progress"

echo "  case 5: the API says the run tested another tree"
reset_fixtures
runs "$SHA" "108:completed:$OTHER_TREE"
jobs 108 "$CLIPPY=success|$LINT=success"
check "$dir" check 1e
expect "a run for another tree proves nothing" 1 "run 108 is for tree ${OTHER_TREE:0:12}, not HEAD's ${TREE:0:12}"

echo "  case 5b: HEAD moved after the push"
reset_fixtures
runs "$SHA" "109:completed:$TREE"
jobs 109 "$CLIPPY=success|$LINT=success"
moved="$(make_repo)"
printf 'edit after the push\n' >>"$moved/work.txt"
git -C "$moved" commit -qam "edit after the push"
check "$moved" check 1e
expect "no ref holds the new tree, so nothing is asked" 1 "no proof: no origin/* ref has HEAD's tree"
if [ -s "$FIX/calls" ]; then fail "GitHub was asked about a tree no candidate holds"; else pass "no API call without a candidate"; fi

echo "  case 5c: a named commit with another tree is refused before GitHub is asked"
reset_fixtures
CODE=0
OUT="$(cd "$moved" && PATH="$ROOT/bin:$PATH" FIX="$FIX" BRANCH_CI_PROOF_REPO=o/r BRANCH_CI_PROOF_SHA="$(git -C "$moved" rev-parse HEAD~1)" "$UNDER_TEST" check 1e 2>&1)" || CODE=$?
expect "BRANCH_CI_PROOF_SHA must hold HEAD's tree" 1 "has tree $(git -C "$moved" rev-parse 'HEAD~1^{tree}' | cut -c1-12), not HEAD's"

echo "  case 5d: a named commit with HEAD's tree is a candidate without any ref"
reset_fixtures
named="$(mktemp -d "$ROOT/named-XXXXXX")"
git -C "$named" init -q -b feature/work
git -C "$named" config user.email test@example.com
git -C "$named" config user.name test
printf 'work\n' >"$named/work.txt"
git -C "$named" add -A
git -C "$named" commit -qm work
NAMED_SHA="$(git -C "$named" rev-parse HEAD)"
git -C "$named" commit -q --amend -m "same tree, another commit"
runs "$NAMED_SHA" "112:completed:$TREE"
jobs 112 "$CLIPPY=success|$LINT=success"
CODE=0
OUT="$(cd "$named" && PATH="$ROOT/bin:$PATH" FIX="$FIX" BRANCH_CI_PROOF_REPO=o/r BRANCH_CI_PROOF_SHA="$NAMED_SHA" "$UNDER_TEST" check 1e 2>&1)" || CODE=$?
expect "the named commit's green run proves HEAD's identical tree" 0 "proven by branch CI: ${NAMED_SHA:0:12} (BRANCH_CI_PROOF_SHA) run 112"

echo "  case 5e: an uncommitted edit does not change the tree judged"
reset_fixtures
runs "$SHA" "113:completed:$TREE"
jobs 113 "$CLIPPY=success|$LINT=success"
printf 'uncommitted\n' >>"$dir/work.txt"
check "$dir" check 1e
expect "the proof is for HEAD's committed tree" 0 "proven by branch CI"
git -C "$dir" checkout -q -- work.txt

echo "  case 6: no remote ref and no named commit"
reset_fixtures
lonely="$(mktemp -d "$ROOT/lonely-XXXXXX")"
git -C "$lonely" init -q
git -C "$lonely" config user.email test@example.com
git -C "$lonely" config user.name test
printf 'x\n' >"$lonely/x"
git -C "$lonely" add -A
git -C "$lonely" commit -qm x
check "$lonely" check 5
expect "no candidate ref proves nothing" 1 "no proof: no origin/* ref has HEAD's tree"

echo "  case 6b: the candidate has no run of the tier's workflow"
reset_fixtures
runs "$SHA" "114:completed:$TREE:push:frontend-tests.yml"
check "$dir" check 1e
expect "a run of another workflow proves nothing" 1 "ci-backend.yml has no push run for ${SHA:0:12}"

echo "  case 6c: a pull_request run is never a proof"
reset_fixtures
runs "$SHA" "115:completed:$TREE:pull_request"
jobs 115 "$CLIPPY=success|$LINT=success"
check "$dir" check 1e
expect "a pull_request run (a merge commit's tree) is ignored" 1 "ci-backend.yml has no push run"

echo "  case 7: the API answers an error"
reset_fixtures
echo "runs-$SHA" >"$FIX/fail"
check "$dir" check 1e
expect "an API error proves nothing" 1 "no proof: GitHub API error (gh exit 1): gh: Server Error (HTTP 500)"

echo "  case 7b: the jobs call fails after the run list answered"
reset_fixtures
runs "$SHA" "116:completed:$TREE"
jobs 116 "$CLIPPY=success|$LINT=success"
echo "jobs-116" >"$FIX/fail"
check "$dir" check 1e
expect "a failed job lookup proves nothing" 1 "no proof: GitHub API error"

echo "  case 7c: rate limited"
reset_fixtures
touch "$FIX/ratelimit"
check "$dir" check 1e
expect "a rate limit proves nothing and says so" 1 "no proof: GitHub API rate limit reached"

echo "  case 7d: an answer that is not JSON"
reset_fixtures
printf 'gateway timeout page\n' >"$FIX/runs-$SHA.json"
check "$dir" check 1e
expect "an unparsable answer proves nothing" 1 "is not a JSON object"

echo "  case 7e: the API does not answer within the budget"
reset_fixtures
echo 30 >"$FIX/sleep"
start="$(date +%s)"
CODE=0
OUT="$(cd "$dir" && PATH="$ROOT/bin:$PATH" FIX="$FIX" BRANCH_CI_PROOF_REPO=o/r BRANCH_CI_PROOF_BUDGET=2 "$UNDER_TEST" check 1e 2>&1)" || CODE=$?
elapsed=$(( $(date +%s) - start ))
expect "a hung API proves nothing" 1 "did not answer within the 2s lookup budget"
if [ "$elapsed" -le 5 ]; then pass "the lookup gave up in ${elapsed}s"; else fail "the lookup took ${elapsed}s on a 2s budget"; fi

echo "  case 7f: one gate run shares one cache and one lookup budget across tiers"
reset_fixtures
runs "$SHA" "117:completed:$TREE"
jobs 117 "$CLIPPY=success|$LINT=success" "Feature Profile Check (no-default-features)=success|Check no-default-features=success"
cache="$(mktemp -d "$ROOT/cache-XXXXXX")"
CODE=0
for tier in 1e 1f; do
  OUT="$(cd "$dir" && PATH="$ROOT/bin:$PATH" FIX="$FIX" BRANCH_CI_PROOF_REPO=o/r BRANCH_CI_PROOF_CACHE="$cache" "$UNDER_TEST" check "$tier" 2>&1)" || CODE=$?
done
expect "the second tier is proven from the same run" 0 "proven by branch CI: ${SHA:0:12} (origin/feature/work) run 117, ci-backend.yml › Feature Profile Check"
calls="$(wc -l <"$FIX/calls" | tr -d ' ')"
if [ "$calls" = 2 ]; then pass "two tiers cost two API calls"; else fail "two tiers cost $calls API calls, expected 2"; fi
reset_fixtures
echo "runs-$SHA" >"$FIX/fail"
cache="$(mktemp -d "$ROOT/cache-XXXXXX")"
for tier in 1e 1f 4; do
  CODE=0
  OUT="$(cd "$dir" && PATH="$ROOT/bin:$PATH" FIX="$FIX" BRANCH_CI_PROOF_REPO=o/r BRANCH_CI_PROOF_CACHE="$cache" "$UNDER_TEST" check "$tier" 2>&1)" || CODE=$?
done
expect "a cached failure still refuses the later tiers" 1 "no proof: GitHub API error"
calls="$(wc -l <"$FIX/calls" | tr -d ' ')"
if [ "$calls" = 1 ]; then pass "a failed lookup is not retried by every tier"; else fail "a failed lookup was asked $calls times"; fi

echo "  case 7g: the curl client, when gh is absent"
reset_fixtures
runs "$SHA" "118:completed:$TREE:push:server-e2e.yml"
jobs 118 "SDK Test Suite=success|Build SDK bridge=success|Run SDK unit tests=success"
CODE=0
OUT="$(cd "$dir" && PATH="$ROOT/bin:$PATH" FIX="$FIX" BRANCH_CI_PROOF_REPO=o/r BRANCH_CI_PROOF_CLIENT=curl "$UNDER_TEST" check 6 2>&1)" || CODE=$?
expect "curl reads the same answers" 0 "proven by branch CI: ${SHA:0:12} (origin/feature/work) run 118, server-e2e.yml › SDK Test Suite"
reset_fixtures
touch "$FIX/ratelimit"
CODE=0
OUT="$(cd "$dir" && PATH="$ROOT/bin:$PATH" FIX="$FIX" BRANCH_CI_PROOF_REPO=o/r BRANCH_CI_PROOF_CLIENT=curl "$UNDER_TEST" check 6 2>&1)" || CODE=$?
expect "curl's rate limit proves nothing" 1 "rate limit"

echo "  case 7h: a tier with two jobs needs both"
reset_fixtures
runs "$SHA" "119:completed:$TREE:push:frontend-tests.yml"
jobs 119 "Frontend Test Suite=success|Lint code=success|Type check=success|Run tests=success" "Playwright E2E Tests=cancelled"
check "$dir" check 5
expect "Tier 5 is not proven while the Playwright job is not green" 1 "job 'Playwright E2E Tests' concluded cancelled"

echo "  case 8: a tier no CI job covers"
reset_fixtures
check "$dir" check 1o
expect "an uncovered tier is never proven" 1 "no proof: no CI job covers tier 1o"
if [ -s "$FIX/calls" ]; then fail "an uncovered tier asked GitHub"; else pass "an uncovered tier asks nothing"; fi
check "$dir" frobnicate
expect "a bad subcommand is a usage error" 2 "check <tier>"

echo "  case 9: Tier 1f scope, read from cargo's no-default-features graph"
cat >"$ROOT/bin/cargo" <<'STUB'
#!/usr/bin/env bash
[ -f "$FIX/cargo-fails" ] && { echo "error: no matching package" >&2; exit 101; }
cat <<'TREE'
pierre_mcp_server v0.3.0 (/w/crates/pierre-server)|
pierre-auth v0.3.0 (/w/crates/pierre-auth)|default
pierre-auth v0.3.0 (/w/crates/pierre-auth)|default (*)
pierre-tool-runtime v0.3.0 (/w/crates/pierre-tool-runtime)| (*)
pierre-services v0.3.0 (/w/crates/pierre-services)|client-messaging,default
TREE
STUB
chmod +x "$ROOT/bin/cargo"
reset_fixtures
check "$dir" no-default-unit pierre-auth
expect "a crate with only its (empty) default on is covered" 0 "covered: CI's no-default-features job compiles pierre-auth"
check "$dir" no-default-unit pierre-tool-runtime
expect "a crate with no feature on is covered" 0 "covered"
check "$dir" no-default-unit pierre-routes-agents
expect "an optional dependency absent from the graph is not covered" 1 "not covered: pierre-routes-agents is not in CI's no-default-features graph"
check "$dir" no-default-unit pierre-services
expect "a crate the graph turns a feature on for is not covered" 1 "turns on pierre-services's features: client-messaging,default"
touch "$FIX/cargo-fails"
check "$dir" no-default-unit pierre-auth
expect "a cargo failure covers nothing" 1 "not covered: cargo tree cannot read"
rm -f "$ROOT/bin/cargo"

echo "  case 10: every job and step the table names exists in its workflow"
# A parity check between two artefacts nothing at runtime joins: the requirement table and
# the workflow files the API names its jobs from. A renamed step would make every lookup
# fail closed, silently costing the local compile it exists to save.
if missing="$("$UNDER_TEST" requirements | WORKFLOWS="$WORKFLOWS" python3 -c '
import os, sys, yaml
missing = []
for line in sys.stdin:
    tier, wf, job, step = line.rstrip("\n").split("\t")
    doc = yaml.safe_load(open(os.path.join(os.environ["WORKFLOWS"], wf)))
    found = False
    for spec in (doc.get("jobs") or {}).values():
        names = []
        template = spec.get("name", "")
        include = ((spec.get("strategy") or {}).get("matrix") or {}).get("include") or [{}]
        for entry in include:
            expand = lambda s: s.replace("${{ matrix.name }}", str(entry.get("name", "")))
            if expand(template) == job and any(expand(s.get("name", "")) == step for s in spec.get("steps", [])):
                found = True
    if not found:
        missing.append(f"tier {tier}: {wf} › {job} › {step}")
print("\n".join(missing))
')" && [ -z "$missing" ]; then
  pass "the table matches the workflow definitions"
else
  fail "the table names jobs or steps the workflows do not: $missing"
fi

echo ""
if [ "$failures" -ne 0 ]; then
  echo "❌ branch-ci-proof.test.sh: $failures case(s) failed"
  exit 1
fi
echo "✅ branch-ci-proof.test.sh: every case passed"
