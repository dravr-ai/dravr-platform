#!/usr/bin/env bash
# ABOUTME: Pins satellite-bump.yml's CI gate: a queue budget, a run budget, and runs matched by commit
# ABOUTME: The step is lifted out of the YAML and driven by a fake clock and scripted run timelines
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# "Poll until every gate reaches a terminal status" is what decides whether a
# satellite release lands on main. It had one 45-minute clock from the moment
# it started, so a saturated fleet spent the budget in the queue: on
# 2026-09-23 the canot 0.4.30 lane timed out with ci-backend still in
# progress and filed a stall about a tree nobody had finished building. It
# also matched runs by branch, which a retry of the same version reuses, so a
# previous attempt's green run could satisfy it. The text run here is the text
# the runner executes; a rename or re-indent of the step fails the lift.
#
# Usage: scripts/ci/satellite-bump-gate.test.sh — bash and awk, no network.

set -euo pipefail
cd "$(dirname "$0")/../.."
WORKFLOW=".github/workflows/satellite-bump.yml"
WORK=$(mktemp -d); trap 'rm -rf "${WORK}"' EXIT
FAILURES=0
pass() { printf '  ok   %s\n' "$1"; }
fail() { printf '  FAIL %s\n' "$1"; FAILURES=$(( FAILURES + 1 )); }

GATE="${WORK}/gate.sh"
awk -v name="      - name: Poll until every gate reaches a terminal status" '
  $0 == name { in_step = 1; next }
  in_step && /^        run: \|$/ { in_run = 1; next }
  in_run {
    if ($0 ~ /^          / || $0 ~ /^[[:space:]]*$/) { sub(/^          /, ""); print; next }
    exit
  }
' "${WORKFLOW}" > "${GATE}"
grep -q "QUEUE_DEADLINE" "${GATE}" || { echo "could not lift the gate step out of ${WORKFLOW} — its name or indentation changed"; exit 1; }

# ---------------------------------------------------------------------------
# Stand-ins. CLOCK holds fake epoch seconds; `sleep` advances it, `date +%s`
# reads it. TIMELINE rows are "<minute> <workflow> <status> <conclusion> [sha]";
# the gate asks for W's runs on one commit and gets the latest row for W on
# that commit at or before the current minute, or nothing (not started). A row
# with no sha is on the commit under test. satellite-pin.sh answers `gates`
# with $GATES, run from a scratch tree so the real one is never consulted.
# ---------------------------------------------------------------------------
BIN="${WORK}/bin"; mkdir -p "${BIN}" "${WORK}/tree/scripts/ci"
cat > "${BIN}/date" <<'SH'
#!/usr/bin/env bash
[ "$*" = "+%s" ] && { cat "${CLOCK}"; exit 0; }
exec /bin/date "$@"
SH
cat > "${BIN}/sleep" <<'SH'
#!/usr/bin/env bash
echo $(( $(cat "${CLOCK}") + $1 )) > "${CLOCK}"
SH
cat > "${BIN}/gh" <<'SH'
#!/usr/bin/env bash
if [ "$1" = "api" ] && [[ "$2" == *"/actions/workflows/"*"/runs?head_sha="* ]]; then
  wf="${2#*/actions/workflows/}"; wf="${wf%%/runs*}"
  sha="${2#*head_sha=}"; sha="${sha%%&*}"
  now_min=$(( ( $(cat "${CLOCK}") - START ) / 60 ))
  awk -v wf="${wf}" -v now="${now_min}" -v sha="${sha}" -v head="${HEAD_SHA}" '
    { row_sha = (NF >= 5) ? $5 : head }
    $2 == wf && $1 <= now && row_sha == sha { line = $3 " " $4 }
    END { if (line) print line }' "${TIMELINE}"
fi
SH
cat > "${WORK}/tree/scripts/ci/satellite-pin.sh" <<'SH'
#!/usr/bin/env bash
[ "$1" = "gates" ] && echo "${GATES}"
SH
chmod +x "${BIN}"/* "${WORK}/tree/scripts/ci/satellite-pin.sh"

START=1790000000
export HEAD_SHA="c0ffee0000000000000000000000000000000000"
run_gate() {  # run_gate <queue min> <run min> <workflows> <timeline rows...>
  local queue="$1" budget="$2" gates="$3"; shift 3
  TIMELINE="${WORK}/timeline"; printf '%s\n' "$@" > "${TIMELINE}"
  CLOCK="${WORK}/clock"; echo "${START}" > "${CLOCK}"
  set +e
  OUT=$(cd "${WORK}/tree" && PATH="${BIN}:${PATH}" CLOCK="${CLOCK}" START="${START}" TIMELINE="${TIMELINE}" \
        GATES="${gates}" SATELLITE="dravr-x" BRANCH="fix/dravr-x-9.9.9" SHA="${HEAD_SHA}" \
        GH_REPO="dravr-ai/dravr-platform" GATE_TIMEOUT="${budget}" GATE_QUEUE="${queue}" \
        bash "${GATE}" 2>&1)
  RC=$?
  set -e
  ELAPSED=$(( ( $(cat "${CLOCK}") - START ) / 60 ))
}
check() {  # check <name> <expected rc> <expected output fragment>
  if [ "${RC}" = "$2" ] && grep -qF -- "$3" <<<"${OUT}"; then pass "$1"
  else fail "$1 (rc=${RC}, want $2; output did not contain '$3')"; printf '%s\n' "${OUT}" | tail -3 | sed 's/^/       /'; fi
}

echo "satellite-bump gate"
run_gate 180 45 "ci-backend.yml" "0 ci-backend.yml queued -" "40 ci-backend.yml in_progress -" "70 ci-backend.yml completed success"
check "40m queued then 30m running is green — the 2026-09-23 canot case" 0 "all gates green"

run_gate 180 45 "ci-backend.yml" "0 ci-backend.yml queued -"
check "never picked up within the queue budget fails, naming the queue" 1 "no runner picked up"
[ "${ELAPSED}" -ge 180 ] && [ "${ELAPSED}" -le 182 ] && pass "  ...and only after the full 180m" || fail "  queue refusal came at ${ELAPSED}m"

run_gate 180 45 "ci-backend.yml" "0 ci-backend.yml in_progress -"
check "running past the run budget fails, naming the run budget" 1 "of running"
[ "${ELAPSED}" -ge 45 ] && [ "${ELAPSED}" -le 47 ] && pass "  ...at 45m of running" || fail "  run refusal came at ${ELAPSED}m"

run_gate 180 45 "ci-backend.yml" "0 ci-backend.yml in_progress -" "12 ci-backend.yml completed failure"
check "a red gate fails at once, not at a deadline" 1 "CI is not green"

run_gate 180 45 "ci-backend.yml" "0 ci-backend.yml in_progress -" "12 ci-backend.yml completed cancelled"
check "a cancelled gate validated nothing and is not green" 1 "ci-backend.yml(cancelled)"

run_gate 180 45 "a.yml b.yml" "0 a.yml in_progress -" "0 b.yml queued -" "100 b.yml in_progress -" "120 a.yml completed success" "130 b.yml completed success"
check "the run budget starts only when the LAST gate starts" 0 "all gates green"
grep -q "run budget starts now" <<<"${OUT}" && pass "  ...and says when it started" || fail "  never announced the run budget"

run_gate 180 45 "ci-backend.yml" "0 ci-backend.yml completed success deadbeef00000000000000000000000000000000"
check "a green run from a previous attempt on another commit does not satisfy the gate" 1 "no runner picked up"

run_gate 180 45 "ci-backend.yml" "0 ci-backend.yml completed success deadbeef00000000000000000000000000000000" "5 ci-backend.yml in_progress -" "20 ci-backend.yml completed failure"
check "  ...and this commit's own red is what decides, not the stale green" 1 "CI is not green"

run_gate 180 45 ""
check "a satellite with no gates has nothing pending and passes" 0 "all gates green"

echo
[ "${FAILURES}" -eq 0 ] && { echo "all satellite-bump gate scenarios pass"; exit 0; }
echo "${FAILURES} scenario(s) failed"; exit 1
