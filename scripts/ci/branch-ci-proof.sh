#!/usr/bin/env bash
# ABOUTME: The one definition of when branch CI already proved a pre-push compile tier on HEAD's exact git tree
# ABOUTME: check <tier> exits 0 only when a commit with HEAD's tree passed the covering CI job and step on a push run
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Usage:
#   branch-ci-proof.sh check <tier>            exit 0 when branch CI proved <tier> on HEAD's tree
#   branch-ci-proof.sh no-default-unit <crate>   exit 0 when CI's no-default-features profile
#                                              compiles <crate> with no feature on (Tier 1f scope)
#   branch-ci-proof.sh requirements [<tier>]     print the tier -> workflow | job | step table
#
# Why this exists:
#   A landing re-ran the whole local gate five times on 2026-11-24 (about 12 minutes and a
#   Rust compile each) because main moved under every run, while the branch it came from was
#   green on every CI lane. Two commits with one tree id hold the same bytes, so a tier whose
#   work a CI job already did on those bytes has nothing left to find on the laptop.
#
# What counts as proof, for one tier:
#   - a candidate commit whose tree is HEAD's committed tree (`git rev-parse HEAD^{tree}`,
#     never the working state). Candidates are found locally: the tip of every
#     refs/remotes/origin/* ref with that tree, plus the shas BRANCH_CI_PROOF_SHA names
#     (space-separated), each checked against HEAD's tree here before GitHub is asked anything;
#   - a push run of the tier's workflow file for that sha, completed, whose head commit's tree
#     is HEAD's tree (read back from the API too);
#   - in that run, every job the tier names concluded success, and every step the tier names
#     inside it concluded success. The step is what proves the work: clippy's job concludes
#     success having linted nothing on a push no Rust input touched, and on main it skips its
#     steps when a branch proved the tree (proven-on-branch.sh). A skipped job or step, an
#     absent one, or one still running proves nothing.
#   A covering job that failed or timed out in any run of HEAD's tree refuses the proof, even
#   when another run of the same tree passed it.
#
# The branch a run came from is not judged, unlike proven-on-branch.sh: that script skips
# whole lanes on main, and the fix/ and debug/ lanes run a smoke subset of some of them. The
# steps named here run the same command on every branch, so a step's success is the proof.
#
# Fail closed, always: no candidate, no network, an API error or rate limit, an answer that
# does not parse, a truncated job list, a budget spent: exit 1 with one line saying why, and
# the caller runs the tier itself. The lookup is bounded: the API calls share one budget,
# BRANCH_CI_PROOF_BUDGET seconds (default 6) of time spent waiting on GitHub, so a tier the
# gate compiles locally in between does not use it up. A caller that checks several tiers
# sets BRANCH_CI_PROOF_CACHE to a directory it owns, so the answers and the budget are read
# once per gate run; without it each call has its own.
#
# Exit status: 0 proven, 1 not proven (the line says why), 2 bad usage.

set -euo pipefail

REPO="${BRANCH_CI_PROOF_REPO:-${GITHUB_REPOSITORY:-dravr-ai/dravr-platform}}"
BUDGET="${BRANCH_CI_PROOF_BUDGET:-6}"
MAX_CANDIDATES=5

usage() { sed -n '7,11p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }

# tier -> workflow file, then one "job|step" line per requirement. Every name is the one
# the API reports: a matrix job carries its matrix value.
tier_requirements() {
    case "$1" in
        1e) printf '%s\n' ci-backend.yml \
            "Clippy Linting (full workspace)|Lint with Clippy (Zero Tolerance + Cognitive Complexity)" ;;
        1f) printf '%s\n' ci-backend.yml \
            "Feature Profile Check (no-default-features)|Check no-default-features" ;;
        4) printf '%s\n' frontend-tests.yml \
            "Shared Package Tests|Type check shared packages" \
            "Shared Package Tests|Lint shared packages" \
            "Shared Package Tests|Test shared packages" ;;
        # The local tier also runs the accessibility Playwright specs when browsers are
        # installed; CI runs them in the Playwright job, so that job is required too.
        5) printf '%s\n' frontend-tests.yml \
            "Frontend Test Suite|Lint code" \
            "Frontend Test Suite|Type check" \
            "Frontend Test Suite|Run tests" \
            "Playwright E2E Tests|Run Playwright E2E tests" ;;
        6) printf '%s\n' server-e2e.yml \
            "SDK Test Suite|Build SDK bridge" \
            "SDK Test Suite|Run SDK unit tests" ;;
        7) printf '%s\n' mobile-unit-tests.yml \
            "Typecheck, Lint & Tests|Type check" \
            "Typecheck, Lint & Tests|Lint" \
            "Typecheck, Lint & Tests|Run unit tests" \
            "Typecheck, Lint & Tests|Run app integration specs" ;;
        *) return 1 ;;
    esac
}
COVERED_TIERS="1e 1f 4 5 6 7"

ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || { echo "no proof: not inside a git checkout"; exit 1; }
cd "$ROOT"

# ---------------------------------------------------------------------------
# Bounded API access
# ---------------------------------------------------------------------------
OWN_CACHE=""
if [ -n "${BRANCH_CI_PROOF_CACHE:-}" ]; then
    CACHE="$BRANCH_CI_PROOF_CACHE"
    mkdir -p "$CACHE"
else
    CACHE="$(mktemp -d)"
    OWN_CACHE="$CACHE"
fi
cleanup() { if [ -n "$OWN_CACHE" ]; then rm -rf "$OWN_CACHE"; fi; }
trap cleanup EXIT

# Seconds of the budget not yet spent waiting on the API.
remaining_budget() {
    local spent=0
    if [ -f "$CACHE/spent" ]; then spent="$(cat "$CACHE/spent")"; fi
    echo $(( BUDGET - spent ))
}

# Adds the seconds since $1 to the time spent waiting on the API.
charge_budget() {
    local started="$1" spent=0
    if [ -f "$CACHE/spent" ]; then spent="$(cat "$CACHE/spent")"; fi
    echo $(( spent + $(date +%s) - started )) >"$CACHE/spent"
}

# Runs "$@" for at most $1 seconds, portable to a macOS without coreutils' timeout.
# The watchdog's descriptors go to /dev/null so a command substitution never waits on it.
run_bounded() {
    local secs="$1" pid watchdog code=0
    shift
    "$@" &
    pid=$!
    ( sleep "$secs"; kill -TERM "$pid" 2>/dev/null ) >/dev/null 2>&1 </dev/null &
    watchdog=$!
    wait "$pid" || code=$?
    kill "$watchdog" 2>/dev/null || true
    wait "$watchdog" 2>/dev/null || true
    return "$code"
}

client() {
    if [ -n "${BRANCH_CI_PROOF_CLIENT:-}" ]; then echo "$BRANCH_CI_PROOF_CLIENT"
    elif command -v gh >/dev/null 2>&1; then echo gh
    else echo curl; fi
}

# api <endpoint> <cache-key>: the JSON body on stdout, or exit 1 with the reason on stdout.
# A failure is cached as well, so every later tier of the same gate run reports it at once.
api() {
    local endpoint="$1" key="$2" secs body err started code=0
    if [ -f "$CACHE/$key.json" ]; then cat "$CACHE/$key.json"; return 0; fi
    if [ -f "$CACHE/$key.err" ]; then cat "$CACHE/$key.err"; return 1; fi
    secs="$(remaining_budget)"
    if [ "$secs" -le 0 ]; then
        echo "the ${BUDGET}s lookup budget is spent" | tee "$CACHE/$key.err"
        return 1
    fi
    body="$CACHE/$key.body"
    err="$CACHE/$key.stderr"
    started="$(date +%s)"
    case "$(client)" in
        gh) run_bounded "$secs" gh api "$endpoint" >"$body" 2>"$err" || code=$? ;;
        curl) run_bounded "$secs" curl -fsS --max-time "$secs" -H 'Accept: application/vnd.github+json' \
                  "https://api.github.com/$endpoint" >"$body" 2>"$err" || code=$? ;;
        *) echo "unknown API client '$(client)'" >"$err"; code=1 ;;
    esac
    charge_budget "$started"
    if [ "$code" -ne 0 ]; then
        if grep -qi 'rate limit' "$err" "$body" 2>/dev/null; then
            echo "GitHub API rate limit reached" >"$CACHE/$key.err"
        elif [ "$code" -eq 143 ] || [ "$code" -eq 28 ]; then
            echo "the GitHub API did not answer within the ${BUDGET}s lookup budget" >"$CACHE/$key.err"
        else
            echo "GitHub API error ($(client) exit $code): $(head -c 200 "$err" | tr '\n' ' ')" >"$CACHE/$key.err"
        fi
        cat "$CACHE/$key.err"
        return 1
    fi
    if ! jq -e 'type == "object"' "$body" >/dev/null 2>&1; then
        echo "GitHub API answer for $key is not a JSON object" | tee "$CACHE/$key.err"
        return 1
    fi
    mv "$body" "$CACHE/$key.json"
    cat "$CACHE/$key.json"
}

# ---------------------------------------------------------------------------
# Candidates: commits holding HEAD's tree, found without asking GitHub
# ---------------------------------------------------------------------------
HEAD_TREE=""
CANDIDATES=""   # lines "sha ref"
REFUSED=""      # why a named sha was not a candidate

find_candidates() {
    HEAD_TREE="$(git rev-parse --verify --quiet 'HEAD^{tree}')" || return 1
    local sha tree ref named
    while read -r sha tree ref; do
        [ "$tree" = "$HEAD_TREE" ] || continue
        CANDIDATES="${CANDIDATES}${sha} ${ref#refs/remotes/}"$'\n'
    done < <(git for-each-ref --format='%(objectname) %(tree) %(refname)' refs/remotes/origin)
    for named in ${BRANCH_CI_PROOF_SHA:-}; do
        if ! sha="$(git rev-parse --verify --quiet "${named}^{commit}")"; then
            REFUSED="${REFUSED}BRANCH_CI_PROOF_SHA ${named} is not a commit here; "
            continue
        fi
        tree="$(git rev-parse "${sha}^{tree}")"
        if [ "$tree" != "$HEAD_TREE" ]; then
            REFUSED="${REFUSED}BRANCH_CI_PROOF_SHA ${named:0:12} has tree ${tree:0:12}, not HEAD's ${HEAD_TREE:0:12}; "
            continue
        fi
        CANDIDATES="${CANDIDATES}${sha} BRANCH_CI_PROOF_SHA"$'\n'
    done
    CANDIDATES="$(printf '%s' "$CANDIDATES" | awk 'NF && !seen[$1]++' | head -n "$MAX_CANDIDATES")"
}

# ---------------------------------------------------------------------------
# check <tier>
# ---------------------------------------------------------------------------
# Judges one run against the tier's requirements. Prints "ok" or the reason; returns 0
# when the run proves the tier, 2 when it vetoes it (a covering job failed), 1 otherwise.
judge_run() {
    local run="$1" jobs_json="$2" req job step state
    shift 2
    # Every covering job is judged for a veto before any is judged for proof: a run that
    # proves nothing at its first requirement still refuses the proof with a failed later job.
    for req in "$@"; do
        job="${req%%|*}"
        state="$(jq -r --arg job "$job" '
            [.jobs[]? | select(.name == $job and .status == "completed")
             | select(.conclusion == "failure" or .conclusion == "timed_out") | .conclusion]
            | .[0] // ""' <<<"$jobs_json")" || state=""
        if [ -n "$state" ]; then
            echo "run $run: job '$job' concluded $state"
            return 2
        fi
    done
    for req in "$@"; do
        job="${req%%|*}"; step="${req#*|}"
        state="$(jq -r --arg job "$job" '
            [.jobs[]? | select(.name == $job)] as $m
            | if ($m | length) == 0 then "absent"
              elif $m[0].status != "completed" then "running:" + ($m[0].status // "unknown")
              else "done:" + ($m[0].conclusion // "none") end' <<<"$jobs_json")" || state="unparsable"
        case "$state" in
            done:success) ;;
            absent)
                if [ "$(jq -r '(.total_count // 0) > (.jobs | length)' <<<"$jobs_json")" = true ]; then
                    echo "run $run: job '$job' is not in the first page of its job list"
                else
                    echo "run $run: job '$job' is absent"
                fi
                return 1 ;;
            running:*) echo "run $run: job '$job' is ${state#running:}"; return 1 ;;
            done:*) echo "run $run: job '$job' concluded ${state#done:}"; return 1 ;;
            *) echo "run $run: the job list does not parse"; return 1 ;;
        esac
        state="$(jq -r --arg job "$job" --arg step "$step" '
            [.jobs[] | select(.name == $job) | .steps[]? | select(.name == $step)] as $s
            | if ($s | length) == 0 then "absent" else ($s[0].conclusion // ($s[0].status // "unknown")) end' \
            <<<"$jobs_json")" || state="unparsable"
        if [ "$state" != success ]; then
            echo "run $run: step '$job › $step' is $state"
            return 1
        fi
    done
    echo ok
}

cmd_check() {
    local tier="${1:-}" spec workflow reqs=() sha ref runs_json run_lines id status tree jobs_json verdict code
    [ -n "$tier" ] || usage
    if ! spec="$(tier_requirements "$tier")"; then
        echo "no proof: no CI job covers tier $tier (covered: $COVERED_TIERS)"
        return 1
    fi
    workflow="$(head -1 <<<"$spec")"
    mapfile -t reqs < <(tail -n +2 <<<"$spec")

    if ! find_candidates; then
        echo "no proof: git cannot read HEAD's tree"
        return 1
    fi
    if [ -z "$CANDIDATES" ]; then
        echo "no proof: no origin/* ref has HEAD's tree ${HEAD_TREE:0:12} (${REFUSED}push the branch, or name the commit in BRANCH_CI_PROOF_SHA)"
        return 1
    fi

    # failed: the lookup itself failed, which refuses the proof outright, since a run it did
    # not read could hold a failed covering job. reason: why the last run read proved nothing.
    local reason="" proof="" veto="" failed=""
    while read -r sha ref; do
        if ! runs_json="$(api "repos/${REPO}/actions/runs?head_sha=${sha}&event=push&per_page=100" "runs-$sha")"; then
            failed="$runs_json"
            break
        fi
        # The query's own filters are checked again: a parameter the API ignored must not
        # let a pull_request run (which tests a merge commit's tree) or another sha in.
        if ! run_lines="$(jq -r --arg sha "$sha" --arg wf ".github/workflows/$workflow" '
                [.workflow_runs[]?
                 | select(.event == "push" and .head_sha == $sha)
                 | select(((.path // "") | split("@")[0]) == $wf)]
                | sort_by(.created_at) | reverse | .[]
                | "\(.id) \(.status) \(.head_commit.tree_id // "none")"' <<<"$runs_json")"; then
            failed="the run list for ${sha:0:12} does not parse"
            break
        fi
        if [ -z "$run_lines" ]; then
            reason="${reason:-$workflow has no push run for ${sha:0:12} ($ref)}"
            continue
        fi
        while read -r id status tree; do
            if [ "$tree" != "$HEAD_TREE" ]; then
                reason="run $id is for tree ${tree:0:12}, not HEAD's ${HEAD_TREE:0:12}"
                continue
            fi
            if [ "$status" != completed ]; then
                reason="run $id ($workflow, ${sha:0:12}) is $status"
                continue
            fi
            if ! jobs_json="$(api "repos/${REPO}/actions/runs/${id}/jobs?filter=latest&per_page=100" "jobs-$id")"; then
                failed="$jobs_json"
                break 2
            fi
            code=0
            verdict="$(judge_run "$id" "$jobs_json" "${reqs[@]}")" || code=$?
            case "$code" in
                0) [ -n "$proof" ] || proof="${sha:0:12} ($ref) run $id" ;;
                2) veto="$verdict" ;;
                *) reason="$verdict" ;;
            esac
        done <<<"$run_lines"
    done <<<"$CANDIDATES"

    if [ -n "$veto" ]; then
        echo "no proof: $veto"
        return 1
    fi
    if [ -n "$failed" ]; then
        echo "no proof: $failed"
        return 1
    fi
    if [ -n "$proof" ]; then
        local names
        names="$(printf '%s\n' "${reqs[@]}" | awk -F'|' '!seen[$1]++ { printf "%s%s", (n++ ? ", " : ""), $1 }')"
        echo "proven by branch CI: $proof, $workflow › $names"
        return 0
    fi
    echo "no proof: ${reason:-no run judged the tree of HEAD}"
    return 1
}

# ---------------------------------------------------------------------------
# no-default-unit <crate>: Tier 1f's scope
# ---------------------------------------------------------------------------
# CI's no-default-features job runs `cargo check -p pierre_mcp_server --no-default-features`.
# A crate in that graph with no feature on is compiled exactly as Tier 1f's per-crate probe
# compiles it; an optional dependency is absent from it, and a crate the graph turns a
# feature on for is a different unit. Read offline from the lockfile and the manifests.
cmd_no_default_unit() {
    local crate="${1:-}" tree_out features
    [ -n "$crate" ] || usage
    if ! tree_out="$(cargo tree --offline -p pierre_mcp_server --no-default-features -e normal,build \
            --prefix none -f '{p}|{f}' 2>/dev/null)"; then
        echo "not covered: cargo tree cannot read the no-default-features graph offline"
        return 1
    fi
    [ -n "$tree_out" ] || { echo "not covered: cargo tree printed no graph"; return 1; }
    features="$(awk -F'|' -v c="$crate" '{ split($1, p, " ") } p[1] == c { sub(/ \(\*\)$/, "", $2); print $2 }' \
        <<<"$tree_out" | sort -u)"
    if [ -z "$(awk -F'|' -v c="$crate" '{ split($1, p, " ") } p[1] == c' <<<"$tree_out")" ]; then
        echo "not covered: $crate is not in CI's no-default-features graph"
        return 1
    fi
    case "$features" in
        ""|default) echo "covered: CI's no-default-features job compiles $crate with no feature on" ;;
        *) echo "not covered: CI's no-default-features graph turns on $crate's features: $(tr '\n' ' ' <<<"$features")"
           return 1 ;;
    esac
}

cmd_requirements() {
    local tier spec
    for tier in ${1:-$COVERED_TIERS}; do
        spec="$(tier_requirements "$tier")" || { echo "no CI job covers tier $tier" >&2; return 1; }
        tail -n +2 <<<"$spec" | sed "s/^/$tier	$(head -1 <<<"$spec")	/; s/|/	/"
    done
}

case "${1:-}" in
    check) shift; cmd_check "$@" ;;
    no-default-unit) shift; cmd_no_default_unit "$@" ;;
    requirements) shift; cmd_requirements "$@" ;;
    *) usage ;;
esac
