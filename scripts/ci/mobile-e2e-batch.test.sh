#!/usr/bin/env bash
# ABOUTME: Fixture test for mobile-e2e-batch.sh — the best-attempt judgement and the reset between attempts
# ABOUTME: Runs without a simulator: xcrun/adb/curl are PATH shims and Maestro is a scripted fake
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The shape this pins is the one carnet#443 measured on run 35211561989: a batch
# that passed 2 of 3 on attempt 1 and 0 of 3 on the retry was judged on the retry
# and fail-fasted the lane. Every case below runs the real runner loop against a
# fake `maestro` that writes a pre-decided junit per attempt, so the judgement,
# the reset order (terminate → Metro check → launch → openurl → wait) and the
# dead-Metro stop are all exercised the way the workflow calls them.
#
# The watchdog cases (carnet#842) run real processes on the real clock: a fake
# maestro that never finishes, with a child under it the way the JVM has them,
# is stopped at a 2 s cap, and the test then looks for anything of it still
# running. They take tens of seconds, most of it the KILL grace one case waits out.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/mobile-e2e-batch.sh}"

failures=0
pass() { echo "  ✅ $1"; }
fail() {
    echo "  ❌ $1"
    failures=$((failures + 1))
}

TMP="$(mktemp -d)"
# Every process a hanging fake starts carries this in its argv (exec -a), so a
# leftover is found by pgrep -f and cleaned up even when an assertion failed.
export HANG_MARKER="mobile-e2e-batch-test-hang-$$"
trap 'pkill -KILL -f "$HANG_MARKER" 2>/dev/null || true; rm -rf "$TMP"' EXIT
# Resolved before the shims exist: the hangs and the watchdog run on real time.
REAL_SLEEP="$(command -v sleep)"
export REAL_SLEEP

# ---------------------------------------------------------------- fixtures

# junit <file> <passed-names...> -- <failed-names...>
junit() {
    local file="$1"; shift
    local failed=0
    {
        echo '<?xml version="1.0" encoding="UTF-8"?>'
        echo '<testsuites><testsuite name="batch">'
        for name in "$@"; do
            if [ "$name" = "--" ]; then failed=1; continue; fi
            if [ "$failed" -eq 0 ]; then
                echo "<testcase name=\"$name\" classname=\"batch\"/>"
            else
                echo "<testcase name=\"$name\" classname=\"batch\">"
                echo '<failure message="Assertion is false: id: login-screen is visible"/>'
                echo '</testcase>'
            fi
        done
        echo '</testsuite></testsuites>'
    } > "$file"
}

# Shims. Each records its argv to $SHIM_LOG; the fake maestro copies the fixture
# named by $MAESTRO_PLAN's next line into --output and exits with that line's rc.
SHIMS="$TMP/shims"
mkdir -p "$SHIMS"

cat > "$SHIMS/maestro" <<'EOF'
#!/usr/bin/env bash
# fake maestro: consumes one line of $MAESTRO_PLAN per call —
#   "<rc> <fixture>"              copy the fixture to --output and exit rc
#                                 ("none" writes no junit, as a dead driver)
#   "slow <secs> <rc> <fixture>"  the same, after <secs> of real time
#   "hang"                        run 60 s (far past any cap here, short enough
#                                 that a broken watchdog fails the test rather
#                                 than stalls it), with a child process under it
#   "hang-stubborn"               the same, and the child ignores TERM
echo "maestro $*" >> "$SHIM_LOG"
out=""
while [ $# -gt 0 ]; do
    if [ "$1" = "--output" ]; then out="$2"; shift 2; else shift; fi
done
line=$(head -1 "$MAESTRO_PLAN")
sed -i.bak '1d' "$MAESTRO_PLAN"
case "$line" in
    hang | hang-stubborn)
        if [ "$line" = hang-stubborn ]; then
            (trap '' TERM; exec -a "fake-maestro-child-$HANG_MARKER" "$REAL_SLEEP" 60) &
        else
            (exec -a "fake-maestro-child-$HANG_MARKER" "$REAL_SLEEP" 60) &
        fi
        exec -a "fake-maestro-$HANG_MARKER" "$REAL_SLEEP" 60
        ;;
    "slow "*)
        rest="${line#slow }"
        "$REAL_SLEEP" "${rest%% *}"
        line="${rest#* }"
        ;;
esac
rc="${line%% *}"
fixture="${line#* }"
if [ "$fixture" != "none" ]; then cp "$fixture" "$out"; fi
exit "$rc"
EOF

cat > "$SHIMS/xcrun" <<'EOF'
#!/usr/bin/env bash
echo "xcrun $*" >> "$SHIM_LOG"
case "$*" in
    *"launchctl list"*)
        # The app is "running" once the shim has seen a launch.
        if grep -q "simctl launch" "$SHIM_LOG"; then
            echo "3481	0	UIKitApplication:host.exp.Exponent[34a8][rb-legacy]"
        fi
        ;;
    *"simctl terminate"*)
        echo "found nothing to terminate" >&2
        exit 3
        ;;
esac
exit 0
EOF

cat > "$SHIMS/adb" <<'EOF'
#!/usr/bin/env bash
echo "adb $*" >> "$SHIM_LOG"
case "$*" in
    *"pidof"*)
        if grep -q "am start" "$SHIM_LOG"; then echo "12345"; fi
        ;;
esac
exit 0
EOF

cat > "$SHIMS/curl" <<'EOF'
#!/usr/bin/env bash
echo "curl $*" >> "$SHIM_LOG"
if [ -f "$METRO_DEAD" ]; then exit 7; fi
echo "packager-status:running"
EOF

# Logged, then really slept: the reset only ever sleeps --settle 0 here (the
# app shims answer "running" on the first look), and the watchdog's once-a-second
# poll needs a clock that moves.
cat > "$SHIMS/sleep" <<'EOF'
#!/usr/bin/env bash
echo "sleep $*" >> "$SHIM_LOG"
exec "$REAL_SLEEP" "$@"
EOF
chmod +x "$SHIMS"/*

junit "$TMP/two-of-three.xml" 01-show-login-screen 06-successful-login -- 02-show-email-password-inputs
junit "$TMP/none-of-three.xml" -- 01-show-login-screen 02-show-email-password-inputs 06-successful-login
junit "$TMP/all-three.xml" 01-show-login-screen 02-show-email-password-inputs 06-successful-login
junit "$TMP/one-of-three.xml" 01-show-login-screen -- 02-show-email-password-inputs 06-successful-login

# run_case <name> <platform> <plan-lines...> ; sets CASE_RC, CASE_OUT, CASE_SUM,
# CASE_STDOUT, CASE_SECS (wall time), SHIM_LOG. Runner flags set in EXTRA_ARGS
# before the call are passed after the defaults (so a second --attempts wins)
# and cleared after it.
EXTRA_ARGS=()
run_case() {
    local name="$1" platform="$2"; shift 2
    local dir="$TMP/case-$name"
    mkdir -p "$dir"
    export SHIM_LOG="$dir/shims.log" MAESTRO_PLAN="$dir/plan" METRO_DEAD="$dir/metro-dead"
    : > "$SHIM_LOG"
    printf '%s\n' "$@" > "$MAESTRO_PLAN"
    CASE_OUT="$dir/results.xml"
    CASE_SUM="$dir/summary.env"
    CASE_RC=0
    local device_args=()
    local started
    [ "$platform" = ios ] && device_args=(--device SIM-UDID)
    started=$(date +%s)
    PATH="$SHIMS:$PATH" "$UNDER_TEST" run --platform "$platform" ${device_args[@]+"${device_args[@]}"} \
        --label "Batch 1" --attempts 2 --output "$CASE_OUT" --summary "$CASE_SUM" \
        --maestro "$SHIMS/maestro" --settle 0 ${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"} \
        -- .maestro/login/01.yaml .maestro/login/02.yaml .maestro/login/06.yaml \
        > "$dir/stdout" 2>&1 || CASE_RC=$?
    CASE_SECS=$(($(date +%s) - started))
    CASE_STDOUT="$dir/stdout"
    EXTRA_ARGS=()
}

expect() {
    local label="$1" got="$2" want="$3"
    if [ "$got" = "$want" ]; then pass "$label"; else fail "$label (got '$got', want '$want')"; fi
}

summary_value() { sed -n "s/^$1=//p" "$CASE_SUM" | tr -d '"'; }

echo "mobile-e2e-batch.sh fixture tests"

# ------------------------------------------------ the carnet#443 shape
echo "2-of-3 then 0-of-3 (the run that fail-fasted):"
run_case regression ios "1 $TMP/two-of-three.xml" "1 $TMP/none-of-three.xml"
expect "exit 1: a flow failed on every attempt" "$CASE_RC" 1
expect "judged on attempt 1, not the retry" "$(summary_value BEST_ATTEMPT)" 1
expect "PASSED is the best attempt's 2" "$(summary_value PASSED)" 2
expect "FAILS is the best attempt's 1" "$(summary_value FAILS)" 1
expect "TESTS is 3" "$(summary_value TESTS)" 3
expect "both attempts ran" "$(summary_value ATTEMPTS_RUN)" 2
expect "the output junit is attempt 1's" "$(grep -c '<failure' "$CASE_OUT")" 1
expect "each attempt's junit is kept" "$(ls "$(dirname "$CASE_OUT")/maestro-attempts" | wc -l | tr -d ' ')" 2
if grep -q 'judged on attempt 1 of 2 — 2 passed, 1 failures' "$CASE_STDOUT"; then
    pass "the report names the attempt it judged"
else
    fail "the report names the attempt it judged"; sed 's/^/      /' "$CASE_STDOUT"
fi

# The reset between the two attempts, in order. Each shim line is reduced to
# one token by a case statement rather than a sed alternation: GNU sed takes
# the first alternative that matches and BSD sed the longest, so the same
# regex names the launchctl line differently on a Linux runner and a Mac.
reset_order() {
    local line order=""
    while IFS= read -r line; do
        case "$line" in
            "xcrun simctl terminate "*) order="$order terminate" ;;
            "curl "*) order="$order metro-status" ;;
            "xcrun simctl launch "*) order="$order launch" ;;
            "xcrun simctl openurl "*) order="$order openurl" ;;
            *"launchctl list"*) order="$order wait-for-process" ;;
        esac
    done < "$SHIM_LOG"
    printf '%s\n' "$order" | tr -s ' ' '\n' | sed '/^$/d' | uniq | tr '\n' ' '
}
expect "reset order: terminate, Metro status, launch, openurl, wait for the process" \
    "$(reset_order)" "terminate metro-status launch openurl wait-for-process "
expect "Metro is asked for /status, not the root" "$(grep -c '8082/status' "$SHIM_LOG")" 1
expect "the deep link is the flow helper's" "$(grep -c 'openurl SIM-UDID exp://127.0.0.1:8082' "$SHIM_LOG")" 1
expect "maestro is given the simulator" "$(grep -c 'maestro test --device SIM-UDID' "$SHIM_LOG")" 2

# ------------------------------------------------ retry recovers
echo "1-of-3 then 3-of-3 (the reset worked):"
run_case recovers ios "1 $TMP/one-of-three.xml" "0 $TMP/all-three.xml"
expect "exit 0" "$CASE_RC" 0
expect "judged on attempt 2" "$(summary_value BEST_ATTEMPT)" 2
expect "no failures reported" "$(summary_value FAILS)" 0
expect "the output junit is attempt 2's" "$(grep -c '<failure' "$CASE_OUT")" 0

# ------------------------------------------------ first attempt passes
echo "3-of-3 first time:"
run_case clean ios "0 $TMP/all-three.xml"
expect "exit 0" "$CASE_RC" 0
expect "one attempt only" "$(summary_value ATTEMPTS_RUN)" 1
expect "no reset was run" "$(grep -c 'simctl terminate' "$SHIM_LOG")" 0

# ------------------------------------------------ Metro dies
echo "Metro dead after attempt 1:"
mkdir -p "$TMP/case-metro-dead" && touch "$TMP/case-metro-dead/metro-dead"
# (the plan's second line is never consumed)
run_case metro-dead ios "1 $TMP/two-of-three.xml" "0 $TMP/all-three.xml"
expect "exit 2: the environment is gone" "$CASE_RC" 2
expect "the retry was not run" "$(summary_value ATTEMPTS_RUN)" 1
expect "attempt 1 is still reported" "$(summary_value PASSED)" 2
expect "no relaunch against a dead Metro" "$(grep -c 'simctl launch' "$SHIM_LOG")" 0
if grep -q 'Metro at http://127.0.0.1:8082 no longer answers' "$CASE_STDOUT"; then
    pass "the stop names Metro"
else
    fail "the stop names Metro"; sed 's/^/      /' "$CASE_STDOUT"
fi

# ------------------------------------------------ driver died, no junit
echo "maestro exits without writing junit:"
run_case no-junit ios "1 none" "1 none"
expect "exit 1" "$CASE_RC" 1
expect "counted as one failure" "$(summary_value FAILS)" 1
expect "counted as one test, so 0-passed fail-fast fires" "$(summary_value TESTS)" 1
expect "a synthetic failure is written for the report" "$(grep -c '<failure' "$CASE_OUT")" 1

# ------------------------------------------------ rc without a recorded failure
echo "maestro exits 1 with a junit naming no failure:"
run_case rc-only ios "1 $TMP/all-three.xml"
expect "exit 1" "$CASE_RC" 1
expect "the unrecorded failure is counted" "$(summary_value FAILS)" 1

# ------------------------------------------------ android reset
echo "Android: one flow, force-stop and am start between attempts:"
run_case android android "1 $TMP/one-of-three.xml" "0 $TMP/all-three.xml"
expect "exit 0" "$CASE_RC" 0
expect "force-stop of the Expo Go package" "$(grep -c 'am force-stop host.exp.exponent' "$SHIM_LOG")" 1
expect "relaunch through the emulator host alias" "$(grep -c 'am start -a android.intent.action.VIEW -d exp://10.0.2.2:8082' "$SHIM_LOG")" 1
expect "waits on pidof" "$(grep -c 'pidof host.exp.exponent' "$SHIM_LOG")" 1
expect "no --device on android" "$(grep -c 'maestro test --device' "$SHIM_LOG")" 0

# ------------------------------------------------ watchdog (carnet#842)
# The --kill-grace the two hang cases run under: not the runner's default, so the
# KILL log naming it shows the flag reached the watchdog, and long enough that a
# stop which waited it out is told apart from one at the cap.
KILL_GRACE=12

# The Android reset as one line of tokens, attempts included.
android_sequence() {
    local line order=""
    while IFS= read -r line; do
        case "$line" in
            "maestro "*) order="$order maestro" ;;
            "adb shell am force-stop "*) order="$order force-stop" ;;
            "curl "*) order="$order metro-status" ;;
            "adb shell am start "*) order="$order relaunch" ;;
            "adb shell pidof "*) order="$order wait-for-process" ;;
        esac
    done < "$SHIM_LOG"
    printf '%s\n' "${order# }"
}

# Any process of a hanging fake that is still alive.
orphans() { pgrep -f "$HANG_MARKER" | tr '\n' ' '; }

# within <secs> — "yes" when the last case's wall time was at most <secs>.
within() {
    if [ "$CASE_SECS" -le "$1" ]; then echo yes; else echo "no: took ${CASE_SECS}s"; fi
}

echo "Watchdog: attempt 1 hangs past a 2 s cap, the retry passes:"
EXTRA_ARGS=(--attempt-timeout 2 --kill-grace "$KILL_GRACE")
run_case hang-recovers android "hang" "0 $TMP/all-three.xml"
expect "exit 0" "$CASE_RC" 0
expect "judged on attempt 2" "$(summary_value BEST_ATTEMPT)" 2
expect "attempt 1 is one failure with rc 124" "$(summary_value ATTEMPT_1)" "1/0/1/124"
if grep -q 'attempt 1 timed out after 2s' "$(dirname "$CASE_OUT")/maestro-attempts/results-attempt1.xml"; then
    pass "attempt 1's junit names the timeout"
else
    fail "attempt 1's junit names the timeout"
fi
if grep -q 'attempt 1 timed out after 2s (the 2s attempt cap)' "$CASE_STDOUT"; then
    pass "the log names the timeout and its cap"
else
    fail "the log names the timeout and its cap"; sed 's/^/      /' "$CASE_STDOUT"
fi
expect "the reset runs between the stopped attempt and the retry" "$(android_sequence)" \
    "maestro force-stop metro-status relaunch wait-for-process maestro"
expect "nothing of the stopped attempt is left running" "$(orphans)" ""
expect "everything exited on TERM, no KILL needed" "$(grep -c 'sending KILL' "$CASE_STDOUT")" 0
# The 2 s cap, a second of polling on each side of the TERM, the reset: ~4 s.
# A stop that waited out the KILL grace (a zombie read as alive, say) would
# take 2 + KILL_GRACE and more; the bound sits between the two.
expect "stopped at the cap, not after the KILL grace" "$(within $((2 + KILL_GRACE - 3)))" yes

echo "Watchdog: every attempt hangs, and a child ignores TERM:"
EXTRA_ARGS=(--attempt-timeout 2 --kill-grace "$KILL_GRACE")
run_case hang-always android "hang-stubborn" "hang"
expect "exit 1: no attempt passed" "$CASE_RC" 1
expect "both attempts ran" "$(summary_value ATTEMPTS_RUN)" 2
expect "attempt 1 is one failure with rc 124" "$(summary_value ATTEMPT_1)" "1/0/1/124"
expect "attempt 2 is one failure with rc 124" "$(summary_value ATTEMPT_2)" "1/0/1/124"
expect "the report names the timeout" "$(grep -c 'timed out after 2s' "$CASE_OUT")" 1
expect "the child that ignored TERM was sent KILL after the grace" \
    "$(grep -c "still running ${KILL_GRACE}s after TERM, sending KILL" "$CASE_STDOUT")" 1
expect "nothing of either attempt is left running" "$(orphans)" ""
# Two capped attempts with their polling, one KILL grace, and slack for a loaded
# runner; an unbounded attempt would run the fake's full 60 s.
expect "bounded by attempts x cap plus one KILL grace" "$(within $((2 * (2 + 2) + KILL_GRACE + 6)))" yes

echo "No --attempt-timeout (the iOS lane's calls): a slow attempt is waited for:"
run_case uncapped android "slow 3 0 $TMP/all-three.xml"
expect "exit 0" "$CASE_RC" 0
expect "one attempt" "$(summary_value ATTEMPTS_RUN)" 1
expect "no watchdog at work" "$(grep -c -e 'timed out' -e 'watchdog' "$CASE_STDOUT")" 0
expect "it ran its full 3 s" "$([ "$CASE_SECS" -ge 3 ] && echo yes || echo "no: took ${CASE_SECS}s")" yes
expect "maestro's own junit is the report" "$(grep -c '<testcase' "$CASE_OUT")" 3

echo "Watchdog armed, the attempt finishes well under the cap:"
EXTRA_ARGS=(--attempt-timeout 20)
run_case under-cap android "slow 1 0 $TMP/all-three.xml"
expect "exit 0" "$CASE_RC" 0
expect "returned when maestro did, not at the 20 s cap" "$(within 10)" yes
expect "maestro's own junit is the report" "$(grep -c '<testcase' "$CASE_OUT")" 3

# ------------------------------------------------ deadline
echo "Deadline: attempt 1 hangs into it — no reset, no retry, exit 3:"
EXTRA_ARGS=(--attempts 3 --deadline "$(($(date +%s) + 2))")
run_case deadline android "hang" "0 $TMP/all-three.xml" "0 $TMP/all-three.xml"
expect "exit 3: the lane is out of time" "$CASE_RC" 3
expect "one attempt ran" "$(summary_value ATTEMPTS_RUN)" 1
expect "maestro was run once" "$(grep -c '^maestro ' "$SHIM_LOG")" 1
expect "no reset after the deadline" "$(grep -c 'am force-stop' "$SHIM_LOG")" 0
if grep -q 'timed out after [0-9]*s (the lane deadline)' "$CASE_STDOUT"; then
    pass "the log names the deadline as the cap"
else
    fail "the log names the deadline as the cap"; sed 's/^/      /' "$CASE_STDOUT"
fi
expect "nothing of the stopped attempt is left running" "$(orphans)" ""

echo "Deadline closer than a reset: attempt 1 fails, no reset, exit 3:"
EXTRA_ARGS=(--deadline "$(($(date +%s) + 30))")
run_case deadline-no-reset ios "1 $TMP/two-of-three.xml" "0 $TMP/all-three.xml"
expect "exit 3: the lane is out of time" "$CASE_RC" 3
expect "one attempt ran" "$(summary_value ATTEMPTS_RUN)" 1
expect "no reset: the app was not terminated" "$(grep -c 'simctl terminate' "$SHIM_LOG")" 0
expect "returned at once, not after a reset's wait" "$([ "$CASE_SECS" -lt 10 ] && echo yes || echo "no: took ${CASE_SECS}s")" yes
expect "judged on the attempt that ran" "$(summary_value PASSED)/$(summary_value TESTS)" "2/3"
if grep -q 'the lane deadline leaves less than a reset (60s)' "$CASE_STDOUT"; then
    pass "the log names the reset the deadline cannot fit"
else
    fail "the log names the reset the deadline cannot fit"; sed 's/^/      /' "$CASE_STDOUT"
fi

echo "Deadline with room for a reset: attempt 1 fails, the retry runs:"
EXTRA_ARGS=(--deadline "$(($(date +%s) + 300))")
run_case deadline-room ios "1 $TMP/two-of-three.xml" "0 $TMP/all-three.xml"
expect "exit 0: the retry passed" "$CASE_RC" 0
expect "two attempts ran" "$(summary_value ATTEMPTS_RUN)" 2
expect "the reset terminated the app once" "$(grep -c 'simctl terminate' "$SHIM_LOG")" 1

echo "Deadline already passed: nothing runs, the report says so:"
EXTRA_ARGS=(--deadline "$(($(date +%s) - 1))")
run_case deadline-passed android "0 $TMP/all-three.xml"
expect "exit 3" "$CASE_RC" 3
expect "maestro was never run" "$(grep -c '^maestro ' "$SHIM_LOG")" 0
expect "no attempt ran" "$(summary_value ATTEMPTS_RUN)" 0
expect "counted as one failed test" "$(summary_value TESTS)/$(summary_value FAILS)" "1/1"
expect "the report names the flow as not run" "$(grep -c 'not run — the lane deadline had passed' "$CASE_OUT")" 1

# ------------------------------------------------ judge alone
echo "judge subcommand over recorded attempts:"
SUM="$TMP/judge.env"; OUT="$TMP/judge.xml"
rc=0
"$UNDER_TEST" judge --label smoke --output "$OUT" --summary "$SUM" \
    -- "1:$TMP/none-of-three.xml" "1:$TMP/two-of-three.xml" "1:$TMP/two-of-three.xml" > /dev/null || rc=$?
expect "exit 1" "$rc" 1
expect "the first best attempt wins a tie" "$(sed -n 's/^BEST_ATTEMPT=//p' "$SUM")" 2
expect "usage on no attempts" "$("$UNDER_TEST" judge --label x --output "$OUT" --summary "$SUM" 2>/dev/null; echo $?)" 64

echo "run refuses a cap, a deadline or a kill grace that is not whole seconds:"
for bad in "--attempt-timeout 10m" "--attempt-timeout -5" "--deadline soon" "--kill-grace 1s"; do
    # shellcheck disable=SC2086 # the flag and its value are split on purpose
    expect "usage on $bad" "$("$UNDER_TEST" run --platform android --label x --output "$OUT" --summary "$SUM" \
        $bad -- a.yaml 2>/dev/null; echo $?)" 64
done

echo ""
if [ "$failures" -gt 0 ]; then
    echo "$failures assertion(s) failed"
    exit 1
fi
echo "all assertions passed"
