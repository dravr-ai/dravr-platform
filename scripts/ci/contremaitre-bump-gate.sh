#!/usr/bin/env bash
# ABOUTME: The contremaitre bump lane's client-test gate: runs the web, shared-package and mobile unit tests
# ABOUTME: on a bump's new string copy, and parks a bump that breaks one on contremaitre-bump/<rev8> with a register issue
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   contremaitre-bump.yml copies the pinned contremaitre strings into
#   packages/i18n and pushes to main with GITHUB_TOKEN, which starts no run, so
#   the client lanes it dispatches afterwards judge a commit already on main.
#   On 2026-11-25 a reworded string (apiKeyRequired, carnet#1820) reached main
#   that way while two unit tests quoted the old sentence, and main was red
#   before anyone saw it. This gate runs those tests before the push.
#
#   Refusing to push a red bump would wedge the lane: the test cannot change
#   on main before the string arrives (it would fail there), and Check 8 of
#   check-contremaitre-sync.sh refuses a branch that moves the pin (carnet#826).
#   So a red bump is parked on contremaitre-bump/<rev8>, one register issue
#   says which tests broke and how to land it, and Check 8 lets exactly that
#   parked commit, with a test fix on top, through to main.
#
# Flakes. A failure counts against the bump only when it fails twice on the
# new copy and not on main's copy: a red suite is rerun on the new copy first,
# and a test that passes there is a flake, not the copy's doing. A test the
# copy really breaks that also flakes red on main's copy is excused, and the
# client lanes the lane dispatches after the push report it on main.
#
# Retries. A scheduled poll stops at a parked rev for good: rerunning ~40
# minutes of suites on every client commit to main only ever rereddened the
# same test, and the way out is a human squashing the stall branch. A newer
# contremaitre commit, or `gh workflow run contremaitre-bump.yml`, retries.
#
# Subcommands (run from the repository root, HEAD = the lane's bump commit
# for `test` and `park`, HEAD = main for `current-stall` and
# `pin-landed-by-hand`):
#   current-stall <rev>   exit 0 when origin/contremaitre-bump/<rev8> carries
#                         the lane's bump to <rev>, at its tip or under a
#                         human's commits: the scheduled poll stops there
#   test <dir>            runs the suites; exit 0 green (or only failures that
#                         are flakes or that main's own copy fails too), 1 red,
#                         2 could not judge. <dir>/failures.txt lists what the
#                         copy breaks
#   park <rev>            pushes HEAD to contremaitre-bump/<rev8>, replacing
#                         only a tip that is the lane's own bump (exit 2, and
#                         nothing pushed, over a human's commit), and deletes
#                         every other contremaitre-bump/* branch
#   prune                 deletes every contremaitre-bump/* branch
#   pin-landed-by-hand <hours>
#                         exit 0 when the commit that set HEAD's pin is not the
#                         lane's and is younger than <hours>: a stall squashed
#                         onto main, whose issue the lane closes even when the
#                         stall branch is already gone
#   file-stall <old> <new> <dir>
#                         opens, or edits, the one open stall issue. Env
#                         PARK_OUTCOME (the park step's result: parked or not parked) and
#                         CLIENTS_RESULT (the test job's result) shape the body
#   close-stall <why> [rev]
#                         closes the open stall issue, if any, saying why; with
#                         <rev>, only one whose title names the move to it
#
# Env: REMOTE (default origin), GH_TOKEN for the register, RUN_URL for the
# issue body. The register is registre.toml's tracker: this repo is public.
set -euo pipefail

REMOTE="${REMOTE:-origin}"
STALL_PREFIX="contremaitre-bump"
BOT="github-actions[bot] <41898282+github-actions[bot]@users.noreply.github.com>"
TITLE_PREFIX="[platform] dravr-contremaitre bump stalled"
LOCALES="packages/i18n/src/locales"
SUITES=(web packages mobile mobile-integration)

die() { echo "::error::$2" >&2; exit "$1"; }

lane_subject() { echo "chore(contremaitre): bump dravr-contremaitre rev to ${1:0:8}"; }

pin_rev() { # $1 = commit — the contremaitre rev Cargo.toml pins there
    git show "$1:Cargo.toml" 2>/dev/null \
        | sed -nE 's/^dravr-contremaitre = \{ git = "[^"]+", rev = "([a-f0-9]+)".*/\1/p'
}

# is_lane_bump <commit> <rev> — the commit is the lane's own bump to <rev>.
is_lane_bump() {
    [[ "$(git log -1 --format='%an <%ae>|%cn <%ce>|%s' "$1" 2>/dev/null || true)" \
        == "${BOT}|${BOT}|$(lane_subject "$2")" ]] && [[ "$(pin_rev "$1")" == "$2" ]]
}

cmd_current_stall() {
    local rev="${1:?current-stall needs the target rev}" tip c
    tip="$(git rev-parse -q --verify "refs/remotes/${REMOTE}/${STALL_PREFIX}/${rev:0:8}^{commit}" 2>/dev/null)" || return 1
    # The lane's commit is the tip, or sits under a human's test fix pushed
    # onto the branch itself; either way the bump is parked and a retry would
    # only redden the same test.
    for c in $(git rev-list --max-count=50 "$tip" --not HEAD 2>/dev/null); do
        if is_lane_bump "$c" "$rev"; then
            echo "origin/${STALL_PREFIX}/${rev:0:8} parks this bump (${c:0:8}); a dispatch or a newer contremaitre commit retries"
            return 0
        fi
    done
    return 1
}

cmd_pin_landed_by_hand() {
    local hours="${1:?pin-landed-by-hand needs a window in hours}" line who when
    line="$(git log -1 --format='%ct|%an <%ae>' -G '^dravr-contremaitre = ' HEAD -- Cargo.toml 2>/dev/null)" || return 1
    [[ -n "$line" ]] || return 1
    when="${line%%|*}" who="${line#*|}"
    [[ "$who" != "$BOT" ]] || return 1
    (( $(date +%s) - when < hours * 3600 ))
}

# run_suite <name> <log> — the commands the client lanes run: frontend-tests.yml's
# "Run tests" and "Test shared packages", mobile-unit-tests.yml's "Run unit
# tests" and "Run app integration specs". --forceExit because a jest worker
# left open after the run otherwise holds it until the job times out; it
# changes no verdict.
run_suite() {
    local rc=0
    case "$1" in
        web) (cd frontend && bun run test -- --run) ;;
        packages) bun run test:packages ;;
        mobile) (cd frontend-mobile && bun run test -- --forceExit) ;;
        mobile-integration) (cd frontend-mobile && bunx jest --config integration/jest.config.js --selectProjects app --forceExit) ;;
        *) die 2 "unknown suite $1" ;;
    esac >"$2" 2>&1 || rc=$?
    return "$rc"
}

# failure_ids <suite> <log>... — one line per failing test file and test, from
# vitest's "FAIL  file > test" lines and jest's "FAIL file" + "● test" pairs,
# prefixed with the suite and, under `bun run --filter`, the package.
failure_ids() {
    local suite="$1"
    shift
    python3 - "$suite" "$@" <<'PY'
import re, sys
suite, paths = sys.argv[1], sys.argv[2:]
ansi = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
fail = re.compile(r"^(?:(\S+) test: )?\s*FAIL\s+(\S.*)$")
bullet = re.compile(r"^(?:\S+ test: )?\s*●\s+(.+)$")
ids = set()
for path in paths:
    current = None
    with open(path, errors="replace") as log:
        for raw in log:
            line = ansi.sub("", raw.rstrip("\n"))
            m = fail.match(line)
            if m:
                where = f"{m.group(1)} " if m.group(1) else ""
                rest = m.group(2).strip()
                if " > " in rest:
                    ids.add(f"{suite}: {where}{rest}")
                    ids.add(f"{suite}: {where}{rest.split(' > ')[0].split()[0]}")
                    current = None
                else:
                    current = f"{where}{rest.split()[0]}"
                    ids.add(f"{suite}: {current}")
                continue
            m = bullet.match(line)
            if m and current and not m.group(1).startswith("Console"):
                ids.add(f"{suite}: {current} › {m.group(1).strip()}")
for i in sorted(ids):
    print(i)
PY
}

# consistent_ids <suite> <log1> <log2> — the tests that failed in both runs;
# a suite red both times with no test named in either is one unnamed entry.
consistent_ids() {
    failure_ids "$1" "$2" | sort -u >"$2.ids"
    failure_ids "$1" "$3" | sort -u >"$3.ids"
    if [[ ! -s "$2.ids" && ! -s "$3.ids" ]]; then
        echo "$1: (red, no failing test named)"
    else
        comm -12 "$2.ids" "$3.ids"
    fi
}

# base_ids <suite> <log> <rc> — what main's copy fails, named as consistent_ids
# names it, so an unnamed red that main shares cancels out too.
base_ids() {
    local named
    named="$(failure_ids "$1" "$2")"
    if [[ -n "$named" ]]; then
        printf '%s\n' "$named"
    elif [[ "$3" -ne 0 ]]; then
        echo "$1: (red, no failing test named)"
    fi
}

cmd_test() {
    local out="${1:?test needs an output directory}" suite rc red=() again=()
    mkdir -p "$out"
    : >"$out/failures.txt"
    # Only the two deliberate verdicts exit 0 or 1. Anything else that fails
    # here (the failure-name parser, a checkout, comm) is a gate that could not
    # judge, never a red that parks the bump under an issue naming no test.
    set -E
    trap 'echo "::error::contremaitre-bump-gate.sh test failed unexpectedly at line ${LINENO}; it cannot judge the copy" >&2; exit 2' ERR
    git diff --quiet HEAD -- || die 2 "the tree differs from HEAD before the tests ran"
    git rev-parse -q --verify HEAD~1 >/dev/null || die 2 "HEAD has no parent to compare the copy against"
    # `bun run --filter` over a pattern that matches no package exits 0, so an
    # empty shared-package selection would read as green: frontend-tests.yml
    # counts the same files before it trusts that suite.
    [[ -n "$(find packages -path '*__tests__*' -name '*.test.ts*' -print -quit 2>/dev/null)" ]] \
        || die 2 "no test files under packages/*/__tests__/; the shared-package suite would pass having run nothing"

    for suite in "${SUITES[@]}"; do
        echo "::group::${suite} on the bumped copy"
        if run_suite "$suite" "$out/${suite}.new.log"; then
            echo "${suite}: green"
        else
            echo "${suite}: red"
            red+=("$suite")
        fi
        tail -n 40 "$out/${suite}.new.log"
        echo "::endgroup::"
    done
    git diff --quiet HEAD -- || die 2 "the tests rewrote tracked files"
    if [[ ${#red[@]} -eq 0 ]]; then
        echo "verdict: green"
        exit 0
    fi

    # A red that does not reproduce on the same copy is a flake.
    for suite in "${red[@]}"; do
        echo "::group::${suite} again on the bumped copy"
        if run_suite "$suite" "$out/${suite}.new2.log"; then
            echo "${suite}: green on the rerun, so its first red was a flake"
        else
            echo "${suite}: red again"
            again+=("$suite")
        fi
        tail -n 20 "$out/${suite}.new2.log"
        echo "::endgroup::"
    done
    git diff --quiet HEAD -- || die 2 "the tests rewrote tracked files"
    if [[ ${#again[@]} -eq 0 ]]; then
        echo "::warning::every red client suite passed on its rerun on the same copy, so none is the bump's: ${red[*]}"
        echo "verdict: green (flaky reds)"
        exit 0
    fi
    for suite in "${again[@]}"; do
        consistent_ids "$suite" "$out/${suite}.new.log" "$out/${suite}.new2.log"
    done | sort -u >"$out/new.ids"
    if [[ ! -s "$out/new.ids" ]]; then
        echo "::warning::no client test failed on both runs of the bumped copy, so none is the bump's"
        echo "verdict: green (flaky reds)"
        exit 0
    fi

    # Main's own copy, through the same suites: a failure it shares is not the
    # bump's, and parking the bump over a red main would wedge every string
    # behind somebody else's test.
    trap 'git checkout -q HEAD -- "$LOCALES"' EXIT
    git checkout -q HEAD~1 -- "$LOCALES"
    : >"$out/base.ids"
    for suite in "${again[@]}"; do
        echo "::group::${suite} on main's copy"
        rc=0
        run_suite "$suite" "$out/${suite}.base.log" || rc=$?
        tail -n 20 "$out/${suite}.base.log"
        echo "::endgroup::"
        base_ids "$suite" "$out/${suite}.base.log" "$rc" >>"$out/base.ids"
    done
    git checkout -q HEAD -- "$LOCALES"
    trap - EXIT
    sort -u -o "$out/base.ids" "$out/base.ids"
    git diff --quiet HEAD -- || die 2 "the tests rewrote tracked files"

    comm -23 "$out/new.ids" "$out/base.ids" >"$out/failures.txt"
    if [[ -s "$out/failures.txt" ]]; then
        # An unnamed red hands the issue its log's last lines instead.
        for suite in "${again[@]}"; do
            if grep -qxF "${suite}: (red, no failing test named)" "$out/failures.txt"; then
                echo "${suite}: last lines:"
                tail -n 15 "$out/${suite}.new2.log" | sed 's/^/    /'
            fi
        done >"$out/tails.txt"
        cat "$out/tails.txt" >>"$out/failures.txt"
        echo "The bumped copy breaks:"
        sed 's/^/  /' "$out/failures.txt"
        echo "verdict: red"
        exit 1
    fi
    echo "::warning::every client test failure here also fails on main's own copy, so the bump adds none: $(tr '\n' ';' <"$out/new.ids")"
    echo "verdict: green (failures pre-exist on main)"
    exit 0
}

stall_branches() {
    git ls-remote --heads "$REMOTE" "refs/heads/${STALL_PREFIX}/*" | awk '{ sub("refs/heads/", "", $2); print $2 }'
}

prune_except() { # $1 = a branch to keep, or empty
    local b branches
    # A listing that failed is not an empty one: say so rather than report a
    # clean prune that read nothing.
    branches="$(stall_branches)" || die 2 "could not list ${STALL_PREFIX}/* on ${REMOTE}"
    for b in $branches; do
        [[ "$b" == "$1" ]] && continue
        git push -q "$REMOTE" --delete "$b"
        echo "deleted superseded ${b}"
    done
}

cmd_park() {
    local rev="${1:?park needs the target rev}" branch tip
    branch="${STALL_PREFIX}/${rev:0:8}"
    is_lane_bump HEAD "$rev" || die 2 "HEAD is not the lane's bump commit to ${rev:0:8}; refusing to park it"
    # Replace only the lane's own commit. A human who pushed a test fix onto
    # the stall branch itself keeps it: the lease makes a push that raced in
    # after this read fail rather than vanish.
    tip="$(git ls-remote "$REMOTE" "refs/heads/${branch}" | cut -f1)" || die 2 "could not read ${branch} on ${REMOTE}"
    if [[ -n "$tip" ]]; then
        git fetch -q "$REMOTE" "refs/heads/${branch}" || die 2 "could not fetch ${branch} to see who made its tip"
        is_lane_bump "$tip" "$rev" \
            || die 2 "${branch} ends in ${tip:0:8}, which the lane did not make; left as it is. Land it from there, or delete the branch and dispatch the lane to re-park"
    fi
    git push -q --force-with-lease="refs/heads/${branch}:${tip}" "$REMOTE" "HEAD:refs/heads/${branch}" \
        || die 2 "could not push ${branch}"
    echo "parked $(git rev-parse --short HEAD) on ${branch}"
    prune_except "$branch"
}

tracker() {
    grep -oE '^tracker[[:space:]]*=[[:space:]]*"[^"]+"' registre.toml | grep -oE '[^"]+/[^"]+'
}

open_stall_issue() { # $1 = tracker, $2 = a target rev or empty — prints the open stall issue's number, if any
    local to=""
    [[ -z "${2:-}" ]] || to=" -> ${2:0:8})"
    gh issue list --repo "$1" --state open --search "dravr-contremaitre bump stalled in:title" \
        --json number,title -q ".[] | select(.title | startswith(\"${TITLE_PREFIX}\")) | select(.title | contains(\"${to}\")) | .number" | head -n 1
}

cmd_file_stall() {
    local old="${1:?}" new="${2:?}" out="${3:?}" t title body branch existing what notes=""
    t="$(tracker)"
    branch="${STALL_PREFIX}/${new:0:8}"
    title="${TITLE_PREFIX} (${old:0:8} -> ${new:0:8}): copy breaks unit tests"
    what="the new string copy fails client unit tests twice that pass on main's copy"
    if [[ -n "${CLIENTS_RESULT:-}" && "${CLIENTS_RESULT}" != "success" ]]; then
        title="${TITLE_PREFIX} (${old:0:8} -> ${new:0:8}): client tests reached no verdict"
        what="the client unit test job reached no verdict (${CLIENTS_RESULT}): a crash, a timeout or a failed install, so no test is named. Read the run; a dispatch retries"
    fi
    if [[ -n "${PARK_OUTCOME:-}" && "${PARK_OUTCOME}" != "parked" ]]; then
        notes+="- **The bump could not be parked** (${PARK_OUTCOME}): either \`${branch}\` ends in a commit the lane did not make, which it never overwrites, or the push was refused. The run's \"Park the bump\" step says which."$'\n'
    fi
    body="$(cat <<EOF
The contremaitre bump lane moved the pin from \`${old:0:8}\` to \`${new:0:8}\`, and ${what}. It did not push to main.

**What breaks** (from the vitest/jest output):

\`\`\`
$(head -n 40 "$out/failures.txt" 2>/dev/null || true)
\`\`\`

- Bump commit: branch \`${branch}\` (bot-authored). Do not delete it or push to it: the lane deletes it once main carries the pin, and Check 8 reads it until then.
- Run: ${RUN_URL:-unknown}
${notes}

**To land it** — the tests change, never the copy (Check 6 holds it to contremaitre byte for byte):

\`\`\`
git fetch origin main ${branch}
git switch -c feature/contremaitre-bump-${new:0:8} origin/${branch}
# update the tests above to the new copy, then:
./scripts/ci/pre-push-validate.sh
git push origin feature/contremaitre-bump-${new:0:8}   # branch CI; Check 8 allows the lane's bump here
# once that branch is green, squash it onto main and push
\`\`\`

The lane closes this issue on its next poll after the squash lands. If it has not within the hour, close it by hand.

If the wording itself is wrong, fix it in dravr-contremaitre instead: the next bump supersedes this one. If a named test looks unrelated to the copy, it failed twice on the new copy and passed on main's, but it may still be a flake: \`gh workflow run contremaitre-bump.yml\` retries. Scheduled polls never retry a parked rev.

A newer contremaitre commit supersedes this stall: the lane retries from main, deletes \`${branch}\`, and edits this issue to the new revs, or closes it when the newer copy passes. Rebase onto the new stall branch if one replaces this one. While stalled, main keeps the previous pin and copy, and nothing ships.

_Filed by \`contremaitre-bump.yml\` through \`scripts/ci/contremaitre-bump-gate.sh\`._
EOF
)"
    # A search that failed files anyway: a second issue is recoverable, a stall
    # on no issue at all is what this step exists to prevent.
    existing="$(open_stall_issue "$t")" \
        || { echo "::warning::could not search ${t} for an open stall issue; opening one" >&2; existing=""; }
    if [[ -n "$existing" ]]; then
        gh issue edit "$existing" --repo "$t" --title "$title" --body "$body" >/dev/null
        echo "updated ${t}#${existing}"
    else
        gh issue create --repo "$t" --title "$title" --body "$body" --label dravr-platform
        echo "opened a stall issue on ${t}"
    fi
}

cmd_close_stall() {
    local why="${1:?close-stall needs a reason}" rev="${2:-}" t existing
    t="$(tracker)"
    existing="$(open_stall_issue "$t" "$rev")" || die 2 "could not search ${t} for an open stall issue"
    [[ -z "$existing" ]] && { echo "no open stall issue"; return 0; }
    gh issue close "$existing" --repo "$t" --reason completed --comment "${why} (${RUN_URL:-the bump lane})" >/dev/null
    echo "closed ${t}#${existing}"
}

case "${1:-}" in
    current-stall) shift; cmd_current_stall "$@" ;;
    test) shift; cmd_test "$@" ;;
    park) shift; cmd_park "$@" ;;
    prune) prune_except "" ;;
    pin-landed-by-hand) shift; cmd_pin_landed_by_hand "$@" ;;
    file-stall) shift; cmd_file_stall "$@" ;;
    close-stall) shift; cmd_close_stall "$@" ;;
    failure-ids) shift; failure_ids "$@" ;;
    *) die 2 "usage: $0 current-stall|test|park|prune|pin-landed-by-hand|file-stall|close-stall|failure-ids …" ;;
esac
