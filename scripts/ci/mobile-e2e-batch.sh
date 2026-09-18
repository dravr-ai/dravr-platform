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
#   - the batch is judged on its BEST attempt (most flows passed), that attempt's
#     junit becomes <output>, and the summary names which attempt it was.
# Exit: 0 every flow of the best attempt passed; 1 some flow failed on every
# attempt; 2 the environment is gone (Metro dead) — the caller stops the lane.
#
# The summary file is KEY=VALUE lines for the caller to `source`: TESTS, PASSED,
# FAILS, RC, BEST_ATTEMPT, ATTEMPTS_RUN, plus ATTEMPT_<n>="tests/passed/fails/rc".
set -uo pipefail

usage() {
    cat >&2 <<'EOF'
usage:
  mobile-e2e-batch.sh run --platform ios|android --label <name> --output <junit>
      --summary <file> [--device <udid>] [--attempts N] [--metro-url URL]
      [--deep-link URL] [--app-id ID] [--maestro PATH] [--settle SECS]
      -- <flow.yaml>...
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

# Writes a junit with one failed testcase so a driver that died before writing
# any result still shows up in the merged report and the step summary, instead of
# reading as a batch in which nothing went wrong.
write_synthetic_junit() {
    local file="$1" label="$2" rc="$3"
    cat > "$file" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<testsuites>
<testsuite name="$label" tests="1" failures="1">
<testcase name="$label: maestro exited $rc without recording a result" classname="$label">
<failure message="maestro exited $rc and wrote no testcase — the driver died before any flow ran"/>
</testcase>
</testsuite>
</testsuites>
EOF
}

# ---------------------------------------------------------------- judge

# judge_attempts <label> <output> <summary> <rc>:<junit>...
# Picks the attempt with the most passed flows; on a tie the earliest one, so a
# retry that changed nothing is reported as the first run it repeated.
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
        if [ "$passed" -gt "$best_passed" ]; then
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
        write_synthetic_junit "$output" "$label" "$best_rc"
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

# app_running <platform> <device> <app-id>
app_running() {
    case "$1" in
        ios)
            xcrun simctl spawn "$2" launchctl list 2>/dev/null | grep -q "UIKitApplication:$3"
            ;;
        android)
            [ -n "$(adb shell pidof "$3" 2>/dev/null | tr -d '[:space:]')" ]
            ;;
    esac
}

# reset_app <platform> <device> <app-id> <metro-url> <deep-link> <settle>
# Returns 2 when Metro is gone; the caller does not retry against a dead bundler.
reset_app() {
    local platform="$1" device="$2" app_id="$3" metro_url="$4" deep_link="$5" settle="$6"
    log "reset: terminating $app_id"
    case "$platform" in
        ios)
            # Exit 3 is "found nothing to terminate": the app already died, which
            # is the state a reset wants.
            xcrun simctl terminate "$device" "$app_id" >/dev/null 2>&1 || true
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
            # the same order helpers/launch-app.yaml uses.
            xcrun simctl launch "$device" "$app_id" >/dev/null 2>&1 || true
            xcrun simctl openurl "$device" "$deep_link" >/dev/null 2>&1 || true
            ;;
        android)
            adb shell am start -a android.intent.action.VIEW -d "$deep_link" >/dev/null 2>&1 || true
            ;;
    esac
    local i
    for i in $(seq 1 30); do
        if app_running "$platform" "$device" "$app_id"; then
            log "reset: $app_id is running again (after ~$((i * 2))s); settling ${settle}s for the bundle"
            sleep "$settle"
            return 0
        fi
        sleep 2
    done
    log "reset: $app_id did not come back within 60s; retrying anyway — the flow helper launches it itself"
    return 0
}

# ---------------------------------------------------------------- run

run_batch() {
    local platform="" device="" label="" output="" summary="" attempts=2
    local metro_url="" deep_link="" app_id="" maestro="${MAESTRO:-$HOME/.maestro/bin/maestro}" settle=15
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
            --) shift; break ;;
            *) usage ;;
        esac
    done
    [ -n "$platform" ] && [ -n "$label" ] && [ -n "$output" ] && [ -n "$summary" ] || usage
    [ $# -gt 0 ] || usage
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

    local n=1 rc attempt_file env_gone=0
    local specs=()
    while [ "$n" -le "$attempts" ]; do
        attempt_file="$attempt_dir/$base-attempt$n.xml"
        rm -f "$attempt_file"
        log ""
        log "=== $label attempt $n of $attempts ==="
        rc=0
        "$maestro" test ${device_args[@]+"${device_args[@]}"} "$@" --format junit --output "$attempt_file" || rc=$?
        specs+=("$rc:$attempt_file")
        if [ "$rc" -eq 0 ]; then
            log "$label attempt $n passed."
            break
        fi
        log "$label attempt $n failed (maestro rc=$rc)."
        if [ "$n" -lt "$attempts" ]; then
            if ! reset_app "$platform" "$device" "$app_id" "$metro_url" "$deep_link" "$settle"; then
                env_gone=1
                break
            fi
        fi
        n=$((n + 1))
    done

    log ""
    log "=== $label: attempts ==="
    local judged=0
    judge_attempts "$label" "$output" "$summary" "${specs[@]}" || judged=$?
    if [ "$env_gone" -eq 1 ]; then
        log "$label: stopped after attempt $n — Metro is dead, the remaining attempts were not run."
        return 2
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
