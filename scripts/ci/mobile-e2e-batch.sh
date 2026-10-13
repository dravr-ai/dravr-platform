#!/usr/bin/env bash
# ABOUTME: Runs one Maestro batch with a real app reset between attempts and judges it on its best attempt
# ABOUTME: Subcommands run/judge — shared by the iOS and Android e2e lanes so the retry can be exercised locally
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this is a script and not a `run:` block. The retry used to live inline in
# mobile-e2e-ios.yml, where it re-opened the deep link, slept 15 s and re-ran the
# batch against whatever state Expo Go was in — no stop, no Metro check — and the
# fail-fast then judged the batch on that LAST attempt. carnet#443 (run
# 35211561989): attempt 1 passed 2 of 3 flows, the retry re-entered the wedged app
# and failed 3 of 3, and "0 passed" killed the lane on what was a 2-of-3 pass. A
# YAML block cannot be run on a desk simulator and cannot be unit-tested; this can.
#
# Contract of `run`:
#   - each attempt runs the whole batch and writes its own junit under
#     <output-dir>/maestro-attempts/, so every attempt stays in the artifact;
#   - between attempts the app is terminated, Metro is asked for its status and a
#     dead Metro ends the batch with exit 2 (a retry cannot resurrect a bundler,
#     and re-running the flows against it only burns the 120 s login wait per
#     flow), then the app is relaunched through the same deep link the flow
#     helper uses and the runner waits for its process before re-running;
#   - every simctl call of an iOS reset is bounded (perl's alarm, as in the iOS
#     smoke step's probes) and so is the reset as a whole: its calls and its wait
#     for the process share one RESET_PROCESS_WAIT_SECS window, and a probe that
#     is stopped counts as "not running";
#   - the batch is judged on its BEST attempt (most flows passed, then, between
#     attempts that recorded as many flows, fewest failures), that attempt's
#     junit becomes <output>, and the summary names which attempt it was;
#   - --attempt-timeout SECS caps each attempt; 0, the default, sets no
#     wall-clock cap. Both lanes' test steps pass it, the Android lane per flow
#     and the iOS lane per batch; the iOS smoke step is bounded by --deadline
#     alone. An attempt still running at the cap is stopped (TERM to maestro
#     and every process under it, KILL to whatever outlives --kill-grace SECS),
#     recorded as rc 124 with a junit naming the timeout, and the reset above
#     runs before the next attempt. carnet#842 (run 37779337862): three launches
#     each sat ~23 min in "Waiting for flows to complete…" before passing, and
#     the 90-minute step ran out with the retry never having fired;
#   - --exit-grace SECS (default 120; 0 turns it off) bounds what maestro may
#     spend after its report, on every attempt, capped or not. Once the
#     attempt's junit is complete (it holds </testsuites>), a maestro still
#     running that many seconds later is stopped the same way and the attempt
#     takes the report's status — 0 when it records a testcase and no failure,
#     else 1 — not 124. A cap or deadline that fires after the report is
#     complete does the same. carnet#869 (run 38402697100): batch 2's retry wrote
#     its whole junit and printed "2/3 Flows Failed" at 14:30:57, its JVM
#     shutdown teardown never returned, and the uncapped batch ran until the
#     90-minute step cap killed it unjudged and unsummarized;
#   - --deadline EPOCH bounds the whole batch: no attempt runs past it, none
#     starts after it, and no reset starts whose waits (the process wait plus
#     --settle) would carry it past the deadline, so a lane that hands every
#     batch its own step deadline ends inside that step with its report
#     written, and a failed attempt's last screen is still on the device.
# Exit: 0 every flow of the best attempt passed; 1 some flow failed on every
# attempt; 2 the environment is gone (Metro dead) — the caller stops the lane;
# 3 the deadline passed before the batch passed — the caller stops the lane.
#
# The summary file is KEY=VALUE lines for the caller to `source`: TESTS, PASSED,
# FAILS, RC, BEST_ATTEMPT, ATTEMPTS_RUN, plus ATTEMPT_<n>="tests/passed/fails/rc".
set -uo pipefail

# Seconds a stopped attempt's processes get to exit on TERM before KILL, unless
# --kill-grace says otherwise. Maestro is a JVM: TERM runs its shutdown hooks,
# which release the device session.
DEFAULT_KILL_GRACE_SECS=10

# Seconds maestro may keep running after its junit is complete, unless
# --exit-grace says otherwise. Its summary-to-exit gap (the JVM's shutdown
# teardown) measured 7.7-19.5 s on the 2026-10-13 iOS runs and as much as 72.4 s
# on earlier ones; one that never ended is carnet#869.
DEFAULT_EXIT_GRACE_SECS=120

# Seconds a reset may take — its simctl calls and the wait for the relaunched
# app's process, together — before re-running anyway. With --settle it bounds a
# reset, which is how the run loop tells a reset the deadline still leaves time
# after from one it would only spend.
RESET_PROCESS_WAIT_SECS=60

# Seconds one simctl call of an iOS reset may take before it is stopped. A
# wedged simulator hangs them without end: the iOS smoke step bounds its
# `launchctl list` probe to 15 s for that reason, and run 38402697100 still had
# a simctl process alive at the job's orphan cleanup. A launch gets twice as
# long because it waits on a process start, and Expo Go's starts stalled 24-36 s
# in that run.
SIMCTL_CALL_SECS=15
SIMCTL_LAUNCH_SECS=30

usage() {
    cat >&2 <<'EOF'
usage:
  mobile-e2e-batch.sh run --platform ios|android --label <name> --output <junit>
      --summary <file> [--device <udid>] [--attempts N] [--metro-url URL]
      [--deep-link URL] [--app-id ID] [--maestro PATH] [--settle SECS]
      [--attempt-timeout SECS] [--deadline EPOCH] [--kill-grace SECS]
      [--exit-grace SECS] -- <flow.yaml>...
  mobile-e2e-batch.sh judge --label <name> --output <junit> --summary <file>
      -- <rc>:<attempt-junit>...
EOF
    exit 64
}

log() { printf '%s\n' "$*"; }

# ---------------------------------------------------------------- junit reading

# count_tag <tag> <file> — grep -c exits 1 on zero matches, which under `||`
# would print a second "0" and break arithmetic; assign separately instead.
count_tag() {
    local n
    n=$(grep -c "$1" "$2" 2>/dev/null) || n=0
    printf '%s' "$n"
}

# write_failure_junit <file> <label> <testcase-name> <message> — one failed
# testcase, so a driver that died or was stopped before writing any result still
# shows up in the merged report and the step summary, instead of reading as a
# batch in which nothing went wrong.
write_failure_junit() {
    local file="$1" label="$2" name="$3" message="$4"
    cat > "$file" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<testsuites>
<testsuite name="$label" tests="1" failures="1">
<testcase name="$name" classname="$label">
<failure message="$message"/>
</testcase>
</testsuite>
</testsuites>
EOF
}

# report_complete <junit> — maestro writes the junit after its last flow, before
# its teardown; the closing tag is what says the whole report is on disk.
report_complete() {
    grep -q '</testsuites>' "$1" 2>/dev/null
}

# report_status <junit> — the attempt's status as its report states it: 0 when
# it records a testcase and no failure, else 1. A report naming no testcase
# verifies nothing, which judge_attempts counts as a failure too.
report_status() {
    [ "$(count_tag '<testcase' "$1")" -gt 0 ] && [ "$(count_tag '<failure' "$1")" -eq 0 ]
}

# ---------------------------------------------------------------- judge

# judge_attempts <label> <output> <summary> <rc>:<junit>...
# Picks the attempt with the most passed flows. Between attempts that passed as
# many out of as many recorded, the one with fewer failures: an attempt whose
# only failure is an exit status its report did not name (a stopped or crashed
# maestro, counted below) loses to a clean retry. The recorded count has to
# match because an attempt that recorded nothing, a timeout or a dead driver,
# counts as one failure: by failures alone it would displace a 0-of-3 whose
# junit names all three, and the report would lose them. On any other tie the
# earliest, so a retry that changed nothing is reported as the first run it
# repeated.
judge_attempts() {
    local label="$1" output="$2" summary="$3"
    shift 3
    local n=0 best=0 best_passed=-1 best_fails=0 best_tests=0 best_rc=0 best_file=""
    : > "$summary"
    local spec rc file tests fails passed
    for spec in "$@"; do
        n=$((n + 1))
        rc="${spec%%:*}"
        file="${spec#*:}"
        tests=$(count_tag '<testcase' "$file")
        fails=$(count_tag '<failure' "$file")
        passed=$((tests - fails))
        if [ "$tests" -eq 0 ]; then
            # Nothing recorded verifies nothing, whatever the exit status said.
            log "  attempt $n: no testcase recorded (maestro rc=$rc) — counted as one failure"
            passed=0
            fails=1
        elif [ "$rc" -ne 0 ] && [ "$fails" -eq 0 ]; then
            # Maestro's exit status is the only signal separating a driver that
            # died mid-batch from a real pass, so it is counted as a failure the
            # junit did not name.
            log "  attempt $n: maestro exited $rc without recording a failure — counting it as one"
            fails=1
        fi
        log "  attempt $n: $tests tests, $passed passed, $fails failures (maestro rc=$rc)"
        printf 'ATTEMPT_%s="%s/%s/%s/%s"\n' "$n" "$tests" "$passed" "$fails" "$rc" >> "$summary"
        if [ "$passed" -gt "$best_passed" ] \
            || { [ "$passed" -eq "$best_passed" ] && [ "$tests" -eq "$best_tests" ] \
                && [ "$fails" -lt "$best_fails" ]; }; then
            best=$n; best_passed=$passed; best_fails=$fails; best_tests=$tests
            best_rc=$rc; best_file=$file
        fi
    done
    if [ "$n" -eq 0 ]; then
        log "judge: no attempt to judge"
        return 64
    fi
    if [ "$best_tests" -eq 0 ]; then
        best_tests=1
        write_failure_junit "$output" "$label" "$label: maestro exited $best_rc without recording a result" \
            "maestro exited $best_rc and wrote no testcase — the driver died before any flow ran"
    elif [ "$best_file" != "$output" ]; then
        cp "$best_file" "$output"
    fi
    {
        printf 'TESTS=%s\n' "$best_tests"
        printf 'PASSED=%s\n' "$best_passed"
        printf 'FAILS=%s\n' "$best_fails"
        printf 'RC=%s\n' "$best_rc"
        printf 'BEST_ATTEMPT=%s\n' "$best"
        printf 'ATTEMPTS_RUN=%s\n' "$n"
    } >> "$summary"
    if [ "$n" -gt 1 ]; then
        log "$label: judged on attempt $best of $n — $best_passed passed, $best_fails failures"
    else
        log "$label: $best_passed passed, $best_fails failures"
    fi
    [ "$best_fails" -eq 0 ]
}

# ---------------------------------------------------------------- reset

# metro_alive <url> — Metro's /status answers "packager-status:running"; the
# marker is what is checked, so an empty reply from a dying process fails.
metro_alive() {
    curl -sf --max-time 10 "$1/status" 2>/dev/null | grep -q 'packager-status:running'
}

# bounded <secs> <command>... — the command, ended by SIGALRM after secs (exit
# 142). Stock macOS has no `timeout`; the alarm survives perl's exec, which is
# the idiom the iOS smoke step's probes use. A command that cannot start exits
# non-zero with the reason.
bounded() {
    local secs="$1"
    shift
    perl -e 'alarm shift; exec @ARGV or die "cannot run $ARGV[0]: $!\n"' "$secs" "$@"
}

# reset_left <started> — seconds left of the RESET_PROCESS_WAIT_SECS window of a
# reset that began at the epoch <started>.
reset_left() {
    printf '%s' $((RESET_PROCESS_WAIT_SECS - ($(date +%s) - $1)))
}

# simctl_call <started> <secs> <out-file> <simctl-args>... — one simctl call of a
# reset begun at <started>, bounded by <secs> and by what is left of the reset's
# window, so the calls and the process wait together stay inside it. Its stdout
# goes to <out-file>, never a pipe: when the alarm stops simctl, a process it
# started in the simulator can still hold a pipe's write end, and a reader would
# wait for an EOF that never comes. Maestro reads its own `launchctl list` probe
# to EOF before its 3 s bound applies, the likeliest place its teardown hung in
# run 38402697100. Its stderr, with the shell's "Alarm clock" notice, is dropped
# for the one line logged here. A call with no window left is not made; both
# that and a stopped call are status 142.
simctl_call() {
    local started="$1" secs="$2" out="$3" left rc=0
    shift 3
    left=$(reset_left "$started")
    if [ "$left" -le 0 ]; then
        log "reset: simctl $1 not run — the ${RESET_PROCESS_WAIT_SECS}s the reset may take are spent"
        return 142
    fi
    [ "$left" -lt "$secs" ] && secs="$left"
    bounded "$secs" xcrun simctl "$@" > "$out" 2>/dev/null || rc=$?
    if [ "$rc" -eq 142 ]; then
        log "reset: simctl $1 did not return within ${secs}s and was stopped — the simulator is not answering"
    fi
    return "$rc"
}

# app_running <platform> <device> <app-id> <reset-started>
# On iOS a probe that is stopped counts as "not running": the reset's window
# then decides how long it keeps asking.
app_running() {
    case "$1" in
        ios)
            local listing found=1
            listing=$(mktemp "${TMPDIR:-/tmp}/mobile-e2e-launchctl.XXXXXX") || return 1
            if simctl_call "$4" "$SIMCTL_CALL_SECS" "$listing" spawn "$2" launchctl list \
                && grep -q "UIKitApplication:$3" "$listing"; then
                found=0
            fi
            rm -f "$listing"
            return "$found"
            ;;
        android)
            [ -n "$(adb shell pidof "$3" 2>/dev/null | tr -d '[:space:]')" ]
            ;;
    esac
}

# reset_app <platform> <device> <app-id> <metro-url> <deep-link> <settle>
# Returns 2 when Metro is gone; the caller does not retry against a dead bundler.
# Takes at most RESET_PROCESS_WAIT_SECS plus <settle>, give or take the second
# the clock is read in, which is what the run loop's deadline check assumes. On
# iOS every simctl call is bounded to hold that; on Android it holds while adb
# answers, as adb's calls carry no bound of their own.
reset_app() {
    local platform="$1" device="$2" app_id="$3" metro_url="$4" deep_link="$5" settle="$6"
    local started left
    started=$(date +%s)
    log "reset: terminating $app_id"
    case "$platform" in
        ios)
            # Exit 3 is "found nothing to terminate": the app already died, which
            # is the state a reset wants.
            simctl_call "$started" "$SIMCTL_CALL_SECS" /dev/null terminate "$device" "$app_id" || true
            ;;
        android)
            adb shell am force-stop "$app_id" >/dev/null 2>&1 || true
            ;;
    esac
    if ! metro_alive "$metro_url"; then
        log "reset: Metro at $metro_url no longer answers /status — a retry cannot recover from a dead bundler."
        return 2
    fi
    log "reset: Metro at $metro_url is serving; relaunching $app_id via $deep_link"
    case "$platform" in
        ios)
            # Launch first, then hand over the URL: an already-running Expo Go
            # takes the deep link without the "Open in Expo Go?" sheet, which is
            # the same order helpers/launch-app.yaml uses. A launch stopped at
            # its bound may still complete; the wait below looks for it.
            simctl_call "$started" "$SIMCTL_LAUNCH_SECS" /dev/null launch "$device" "$app_id" || true
            simctl_call "$started" "$SIMCTL_CALL_SECS" /dev/null openurl "$device" "$deep_link" || true
            ;;
        android)
            adb shell am start -a android.intent.action.VIEW -d "$deep_link" >/dev/null 2>&1 || true
            ;;
    esac
    # The wait is measured on the clock, not counted in probes: a bounded iOS
    # probe can itself take SIMCTL_CALL_SECS, and thirty of them would hold the
    # reset for minutes past the window the deadline arithmetic gave it.
    while :; do
        if app_running "$platform" "$device" "$app_id" "$started"; then
            log "reset: $app_id is running again ($(($(date +%s) - started))s into the reset); settling ${settle}s for the bundle"
            sleep "$settle"
            return 0
        fi
        left=$(reset_left "$started")
        [ "$left" -gt 0 ] || break
        sleep $((left < 2 ? left : 2))
        [ "$(reset_left "$started")" -gt 0 ] || break
    done
    log "reset: $app_id did not come back within ${RESET_PROCESS_WAIT_SECS}s; retrying anyway — the flow helper launches it itself"
    return 0
}

# ---------------------------------------------------------------- watchdog
#
# Pure bash 3.2, the macOS default this script supports (see device_args below):
# no `wait -n`, and macOS ships no GNU `timeout`.

# descendants <pid> — every live process under pid, each before its own children.
descendants() {
    local child
    for child in $(pgrep -P "$1" 2>/dev/null); do
        printf '%s\n' "$child"
        descendants "$child"
    done
}

# stop_tree <pid> <grace-secs> — TERM the process and everything under it, then
# KILL what is still alive grace-secs later. The tree is listed before the first
# signal: a child whose parent dies is re-parented and `pgrep -P` no longer finds it.
stop_tree() {
    local grace="$2" pids p alive waited=0
    pids="$1 $(descendants "$1" | tr '\n' ' ')"
    for p in $pids; do
        kill -TERM "$p" 2>/dev/null
    done
    while [ "$waited" -lt "$grace" ]; do
        alive=""
        for p in $pids; do
            if kill -0 "$p" 2>/dev/null; then
                alive="$alive $p"
            fi
        done
        [ -n "$alive" ] || return 0
        sleep 1
        waited=$((waited + 1))
    done
    log "watchdog: still running ${grace}s after TERM, sending KILL to:$alive"
    for p in $alive; do
        kill -KILL "$p" 2>/dev/null
    done
    return 0
}

# run_watched <cap-secs> <kill-grace> <exit-grace> <report> <command>... — runs
# the command in the background and polls it once a second:
#   - still running <exit-grace> seconds after <report> became complete (0: never
#     stopped for that), it is stopped with stop_tree and the status is the
#     report's (report_status);
#   - still running at the cap (0: no cap), it is stopped the same way, and the
#     status is the report's when the report was complete by then, else 124,
#     GNU timeout's;
#   - otherwise the command's own status. An exit 0 that races a stop is kept:
#     the attempt finished.
run_watched() {
    local cap="$1" kill_grace="$2" exit_grace="$3" report="$4"
    local pid started now reported=0 stop=0 rc
    shift 4
    "$@" &
    pid=$!
    started=$(date +%s)
    while kill -0 "$pid" 2>/dev/null; do
        now=$(date +%s)
        if [ "$reported" -eq 0 ] && report_complete "$report"; then
            reported=$now
        fi
        if [ "$reported" -gt 0 ] && [ "$exit_grace" -gt 0 ] && [ $((now - reported)) -ge "$exit_grace" ]; then
            log "watchdog: maestro wrote its report $((now - reported))s ago and has not exited — its teardown hung; stopping it, and the attempt is judged on that report."
            stop=1
            break
        fi
        if [ "$cap" -gt 0 ] && [ $((now - started)) -ge "$cap" ]; then
            if [ "$reported" -gt 0 ]; then
                log "watchdog: the ${cap}s cap was reached $((now - reported))s after maestro wrote its report; stopping it, and the attempt is judged on that report."
            fi
            stop=1
            break
        fi
        sleep 1
    done
    if [ "$stop" -eq 0 ]; then
        wait "$pid"
        return
    fi
    stop_tree "$pid" "$kill_grace"
    rc=0
    wait "$pid" 2>/dev/null || rc=$?
    [ "$rc" -eq 0 ] && return 0
    if [ "$reported" -gt 0 ]; then
        report_status "$report"
        return
    fi
    return 124
}

# past <epoch> — true once the clock has reached it.
past() {
    [ "$(date +%s)" -ge "$1" ]
}

# ---------------------------------------------------------------- run

run_batch() {
    local platform="" device="" label="" output="" summary="" attempts=2
    local metro_url="" deep_link="" app_id="" maestro="${MAESTRO:-$HOME/.maestro/bin/maestro}" settle=15
    local attempt_timeout=0 deadline=0 kill_grace="$DEFAULT_KILL_GRACE_SECS" exit_grace="$DEFAULT_EXIT_GRACE_SECS"
    while [ $# -gt 0 ]; do
        case "$1" in
            --platform) platform="$2"; shift 2 ;;
            --device) device="$2"; shift 2 ;;
            --label) label="$2"; shift 2 ;;
            --output) output="$2"; shift 2 ;;
            --summary) summary="$2"; shift 2 ;;
            --attempts) attempts="$2"; shift 2 ;;
            --metro-url) metro_url="$2"; shift 2 ;;
            --deep-link) deep_link="$2"; shift 2 ;;
            --app-id) app_id="$2"; shift 2 ;;
            --maestro) maestro="$2"; shift 2 ;;
            --settle) settle="$2"; shift 2 ;;
            --attempt-timeout) attempt_timeout="$2"; shift 2 ;;
            --deadline) deadline="$2"; shift 2 ;;
            --kill-grace) kill_grace="$2"; shift 2 ;;
            --exit-grace) exit_grace="$2"; shift 2 ;;
            --) shift; break ;;
            *) usage ;;
        esac
    done
    [ -n "$platform" ] && [ -n "$label" ] && [ -n "$output" ] && [ -n "$summary" ] || usage
    [ $# -gt 0 ] || usage
    local secs
    for secs in "$attempt_timeout" "$deadline" "$kill_grace" "$exit_grace"; do
        case "$secs" in
            '' | *[!0-9]*) usage ;;
        esac
    done
    case "$platform" in
        ios)
            [ -n "$device" ] || usage
            : "${metro_url:=http://127.0.0.1:8082}"
            : "${deep_link:=exp://127.0.0.1:8082}"
            : "${app_id:=host.exp.Exponent}"
            ;;
        android)
            : "${metro_url:=http://127.0.0.1:8082}"
            : "${deep_link:=exp://10.0.2.2:8082}"
            : "${app_id:=host.exp.exponent}"
            ;;
        *) usage ;;
    esac

    local out_dir attempt_dir base
    out_dir=$(dirname "$output")
    base=$(basename "$output" .xml)
    attempt_dir="$out_dir/maestro-attempts"
    mkdir -p "$attempt_dir"

    # bash 3.2 (the macOS default) treats an empty array as unset under `set -u`,
    # hence the ${arr[@]+...} expansion below.
    local device_args=()
    [ -n "$device" ] && device_args=(--device "$device")

    local n=1 ran=0 rc attempt_file env_gone=0 out_of_time=0 cap cap_by remaining
    local specs=()
    while [ "$n" -le "$attempts" ]; do
        # The cap this attempt runs under: the per-attempt one, cut short by the
        # deadline when less time than that is left; 0 when neither is set.
        cap="$attempt_timeout"
        cap_by="the ${attempt_timeout}s attempt cap"
        if [ "$deadline" -gt 0 ]; then
            remaining=$((deadline - $(date +%s)))
            if [ "$remaining" -le 0 ]; then
                log "$label: the lane deadline has passed — attempt $n of $attempts is not started."
                out_of_time=1
                break
            fi
            if [ "$cap" -eq 0 ] || [ "$remaining" -lt "$cap" ]; then
                cap="$remaining"
                cap_by="the lane deadline"
            fi
        fi
        attempt_file="$attempt_dir/$base-attempt$n.xml"
        rm -f "$attempt_file"
        log ""
        log "=== $label attempt $n of $attempts ==="
        rc=0
        # Watched even with no cap (cap 0), so the exit grace applies to every
        # attempt, capped or not.
        run_watched "$cap" "$kill_grace" "$exit_grace" "$attempt_file" \
            "$maestro" test ${device_args[@]+"${device_args[@]}"} "$@" \
            --format junit --output "$attempt_file" || rc=$?
        ran=$((ran + 1))
        if [ "$cap" -gt 0 ] && [ "$rc" -eq 124 ]; then
            log "$label attempt $n timed out after ${cap}s ($cap_by) — maestro was stopped."
            # A stopped maestro writes no junit; the one written here names the
            # timeout so the report says why the attempt has no flow result.
            if [ "$(count_tag '<testcase' "$attempt_file")" -eq 0 ]; then
                write_failure_junit "$attempt_file" "$label" "$label: attempt $n timed out after ${cap}s" \
                    "maestro was still running at $cap_by (${cap}s) and was stopped before recording a result"
            fi
        fi
        specs+=("$rc:$attempt_file")
        if [ "$rc" -eq 0 ]; then
            log "$label attempt $n passed."
            break
        fi
        log "$label attempt $n failed (maestro rc=$rc)."
        if [ "$deadline" -gt 0 ] && past "$deadline"; then
            # No reset either: it would only spend time the lane no longer has.
            if [ "$n" -lt "$attempts" ]; then
                log "$label: the lane deadline has passed — the remaining attempts are not run."
            fi
            out_of_time=1
            break
        fi
        # A reset the deadline cannot fit is skipped too: it would end at or
        # past the deadline, so no attempt follows it, and it would replace the
        # screen the failed attempt left with a freshly launched app.
        if [ "$n" -lt "$attempts" ] && [ "$deadline" -gt 0 ] \
            && [ $((deadline - $(date +%s))) -le $((RESET_PROCESS_WAIT_SECS + settle)) ]; then
            log "$label: the lane deadline leaves less than a reset ($((RESET_PROCESS_WAIT_SECS + settle))s) — the remaining attempts are not run."
            out_of_time=1
            break
        fi
        if [ "$n" -lt "$attempts" ]; then
            if ! reset_app "$platform" "$device" "$app_id" "$metro_url" "$deep_link" "$settle"; then
                env_gone=1
                break
            fi
        fi
        n=$((n + 1))
    done

    if [ "$ran" -eq 0 ]; then
        # Only the deadline gets here. The judge needs an attempt, so the report
        # and the summary are written directly.
        write_failure_junit "$output" "$label" "$label: not run — the lane deadline had passed" \
            "the lane deadline passed before attempt 1 could start"
        printf 'TESTS=1\nPASSED=0\nFAILS=1\nRC=124\nBEST_ATTEMPT=0\nATTEMPTS_RUN=0\n' > "$summary"
        return 3
    fi

    log ""
    log "=== $label: attempts ==="
    local judged=0
    judge_attempts "$label" "$output" "$summary" "${specs[@]}" || judged=$?
    if [ "$env_gone" -eq 1 ]; then
        log "$label: stopped after attempt $n — Metro is dead, the remaining attempts were not run."
        return 2
    fi
    if [ "$out_of_time" -eq 1 ]; then
        log "$label: stopped after attempt $ran — the lane deadline has passed."
        return 3
    fi
    return "$judged"
}

judge_cmd() {
    local label="" output="" summary=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --label) label="$2"; shift 2 ;;
            --output) output="$2"; shift 2 ;;
            --summary) summary="$2"; shift 2 ;;
            --) shift; break ;;
            *) usage ;;
        esac
    done
    [ -n "$label" ] && [ -n "$output" ] && [ -n "$summary" ] || usage
    [ $# -gt 0 ] || usage
    judge_attempts "$label" "$output" "$summary" "$@"
}

case "${1:-}" in
    run) shift; run_batch "$@" ;;
    judge) shift; judge_cmd "$@" ;;
    *) usage ;;
esac
