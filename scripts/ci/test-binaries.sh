#!/usr/bin/env bash
# ABOUTME: Builds a lane's test binaries in one cargo invocation and packs them, or unpacks and runs them on a shard that runs without compiling
# ABOUTME: One pack format for the PostgreSQL shards and the nightly SQLite shards, plus the runner's peak memory during the build (carnet#1115)
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Usage:
#   test-binaries.sh build <dir> [--keep <package dir>]... <cargo test args>...
#       Runs `cargo test --no-run --message-format=json-render-diagnostics <args>`
#       and writes into <dir>:
#         test-binaries.tsv  "<target>\t<package dir>\t<executable>" per test binary,
#                            the absolute paths cargo reported; with --keep, only
#                            the binaries of those packages (a path relative to the
#                            workspace), since every crate's suites are one binary
#                            named `it` and `--test it` builds each selected one;
#                            a kept package with no test binary fails the build
#         workspace          the workspace path the binaries were built in
#         spawned-binaries.tsv  "<bin>\t<package dir>\t<executable>" per binary a
#                            packed package's tests spawn through
#                            `env!("CARGO_BIN_EXE_<bin>")`: that path is baked in
#                            at compile time, so the bin travels with the tests
#                            (a bin the tests name and the build did not produce
#                            fails the build)
#         binaries.tar.zst   every executable, relative to that workspace
#         cargo-git.tar.zst  $CARGO_HOME/git/checkouts, relative to $CARGO_HOME
#       It samples /proc/meminfo while cargo runs and reports the runner's peak
#       memory, failed build or not: an out-of-memory link is a failed build.
#   test-binaries.sh unpack <dir>
#       Restores both archives on a runner that has not compiled anything and
#       checks every binary in the map is there and executable.
#   test-binaries.sh run <map> <package dir> <target> [<test args>...]
#       Execs every binary <map> lists for <target> in <package dir>, as
#       `cargo test --test <target>` would, from the workspace root. A target
#       with no such binary fails rather than passing unrun.
#
# A shard used to build its own binaries, so all four compiled the same tree.
# Now one job compiles and the shards download, which only works because the
# runner's workspace path is the same on every job of a workflow: tests bake
# `env!("CARGO_MANIFEST_DIR")` in at compile time and read the source tree
# through it, so unpack refuses a runner whose workspace differs from the
# build's rather than letting those tests read nothing.
#
# The checkouts travel because a test reads one at run time:
# personas_endpoint_test opens the pinned dravr-contremaitre checkout under
# $CARGO_HOME/git/checkouts, which a runner that never ran cargo does not have.
#
# The profile's split-debuginfo = "unpacked" leaves the DWARF in target/'s
# object files, which do not travel: a backtrace on a shard names functions but
# not lines. A panic message still carries its file and line.

set -euo pipefail

die() {
    echo "test-binaries.sh: $*" >&2
    exit 2
}

SAMPLER_PID=""

# MemTotal and MemAvailable in kB, one line a second. Linux only: elsewhere the
# build runs unmeasured and says so.
start_sampler() {
    [ -r /proc/meminfo ] || return 0
    (
        while :; do
            awk '/^MemTotal:/ { t = $2 } /^MemAvailable:/ { a = $2 } END { print t, a }' /proc/meminfo
            sleep 1
        done
    ) >"$1" &
    SAMPLER_PID=$!
}

stop_sampler() {
    [ -n "$SAMPLER_PID" ] || return 0
    kill "$SAMPLER_PID" 2>/dev/null || true
    wait "$SAMPLER_PID" 2>/dev/null || true
    SAMPLER_PID=""
}

# Peak use is MemTotal less the lowest MemAvailable seen: what the build held
# that the kernel could not have handed back, the runner's own agent included.
report_memory() {
    local line
    if [ ! -s "$1" ]; then
        line="Peak memory during the build: not measured (no /proc/meminfo)"
    else
        line="$(awk 'NF == 2 { t = $1; if (min == "" || $2 < min) min = $2 }
            END { printf "Peak memory during the build: %.1f GiB of %.1f GiB (lowest MemAvailable %.1f GiB)", (t - min) / 1048576, t / 1048576, min / 1048576 }' "$1")"
    fi
    echo "$line"
    if [ -n "${GITHUB_ACTIONS:-}" ]; then
        echo "::notice::$line"
        [ -z "${GITHUB_STEP_SUMMARY:-}" ] || echo "$line" >>"$GITHUB_STEP_SUMMARY"
    fi
}

# Total bytes of the files named on stdin, one per line.
sum_bytes() {
    local file total=0 size
    while IFS= read -r file; do
        size="$(wc -c <"$file")"
        total=$((total + size))
    done
    echo "$total"
}

mib() { awk -v b="$1" 'BEGIN { printf "%.0f MiB", b / 1048576 }'; }

build() {
    local dir="$1"
    shift
    local keep=()
    while [ "${1:-}" = --keep ]; do
        [ $# -ge 2 ] || die "build: --keep names no package directory"
        keep+=("$PWD/${2%/}")
        shift 2
    done
    [ $# -gt 0 ] || die "build: no cargo test arguments"
    mkdir -p "$dir"
    local json="$dir/cargo-artifacts.json" map="$dir/test-binaries.tsv" list="$dir/executables.txt"
    local workspace="$PWD" cargo_home="${CARGO_HOME:-$HOME/.cargo}" status=0

    trap stop_sampler EXIT
    start_sampler "$dir/meminfo.log"
    # json-render-diagnostics names the executable cargo produced for each test
    # target while still rendering compile errors to stderr; plain `json` would
    # bury them in the redirected file.
    cargo test --no-run --message-format=json-render-diagnostics "$@" >"$json" || status=$?
    stop_sampler
    report_memory "$dir/meminfo.log"
    [ "$status" -eq 0 ] || die "cargo test --no-run failed (exit $status)"

    jq -R -r '
        fromjson?
        | select(.reason == "compiler-artifact" and .executable != null)
        | select(.target.kind | index("test") != null)
        | [.target.name, (.manifest_path | rtrimstr("/Cargo.toml")), .executable]
        | @tsv
    ' <"$json" >"$map"
    if [ "${#keep[@]}" -gt 0 ]; then
        printf '%s\n' "${keep[@]}" >"$dir/keep.txt"
        awk -F'\t' 'NR == FNR { keep[$0] = 1; next } $2 in keep' "$dir/keep.txt" "$map" >"$map.kept"
        mv "$map.kept" "$map"
        # Per package, not only overall: one misspelled --keep beside a good
        # one would otherwise pack a lane without that package's binaries.
        local package
        for package in "${keep[@]}"; do
            awk -F'\t' -v p="$package" '$2 == p { found = 1 } END { exit !found }' "$map" \
                || die "--keep ${package#"$workspace"/} matched no test binary — refusing to pack a lane without it"
        done
    fi
    [ -s "$map" ] || die "the build produced no test binary — refusing to pack an empty lane"

    # A test that spawns one of its package's binaries finds it through
    # `env!("CARGO_BIN_EXE_<bin>")`, the absolute path cargo built it at: on a
    # shard that path names nothing unless the bin is packed too (carnet#1544,
    # the lab PDF child every upload test runs).
    local spawned="$dir/spawned-binaries.tsv" package name exe
    : >"$spawned"
    while IFS= read -r package; do
        [ -d "$package/tests" ] || continue
        for name in $(grep -rhoE 'CARGO_BIN_EXE_[A-Za-z0-9_-]+' "$package/tests" | sed 's/^CARGO_BIN_EXE_//' | LC_ALL=C sort -u); do
            exe="$(jq -R -r --arg n "$name" --arg p "$package" '
                fromjson?
                | select(.reason == "compiler-artifact" and .executable != null)
                | select(.target.kind | index("bin") != null)
                | select(.target.name == $n and (.manifest_path | rtrimstr("/Cargo.toml")) == $p)
                | .executable
            ' <"$json" | tail -1)"
            [ -n "$exe" ] || die "${package#"$workspace"/}'s tests spawn $name (CARGO_BIN_EXE_$name), which the build did not produce"
            printf '%s\t%s\t%s\n' "$name" "$package" "$exe" >>"$spawned"
        done
    done < <(cut -f2 "$map" | LC_ALL=C sort -u)

    local target executable
    : >"$list"
    while IFS=$'\t' read -r target _ executable; do
        case "$executable" in
            "$workspace"/*) echo "${executable#"$workspace"/}" >>"$list" ;;
            *) die "$target's binary $executable is outside the workspace $workspace" ;;
        esac
    done < <(cat "$map" "$spawned")
    LC_ALL=C sort -u -o "$list" "$list"
    echo "$workspace" >"$dir/workspace"

    tar -cf - -C "$workspace" -T "$list" | zstd -q -T0 -3 -o "$dir/binaries.tar.zst"
    if [ -d "$cargo_home/git/checkouts" ]; then
        tar -cf - -C "$cargo_home" git/checkouts | zstd -q -T0 -3 -o "$dir/cargo-git.tar.zst"
    fi

    local raw packed
    raw="$(cd "$workspace" && sum_bytes <"$list")"
    packed="$(wc -c <"$dir/binaries.tar.zst")"
    echo "Packed $(wc -l <"$list" | tr -d ' ') test binaries: $(mib "$raw") raw, $(mib "$packed") compressed"
    if [ -f "$dir/cargo-git.tar.zst" ]; then
        echo "Packed $cargo_home/git/checkouts: $(mib "$(wc -c <"$dir/cargo-git.tar.zst")") compressed"
    fi
}

unpack() {
    local dir="$1" built cargo_home="${CARGO_HOME:-$HOME/.cargo}"
    [ -f "$dir/workspace" ] && [ -f "$dir/test-binaries.tsv" ] && [ -f "$dir/binaries.tar.zst" ] \
        || die "unpack: $dir holds no test-binaries pack"
    built="$(cat "$dir/workspace")"
    [ "$built" = "$PWD" ] \
        || die "unpack: the binaries were built in $built but this workspace is $PWD; their compile-time paths would name nothing here"

    zstd -q -dc "$dir/binaries.tar.zst" | tar -xf - -C "$PWD"
    if [ -f "$dir/cargo-git.tar.zst" ]; then
        mkdir -p "$cargo_home"
        zstd -q -dc "$dir/cargo-git.tar.zst" | tar -xf - -C "$cargo_home"
    fi

    local target executable count=0
    while IFS=$'\t' read -r target _ executable; do
        [ -x "$executable" ] || die "unpack: $target's binary $executable is missing or not executable"
        count=$((count + 1))
    done <"$dir/test-binaries.tsv"
    local spawned=0
    if [ -f "$dir/spawned-binaries.tsv" ]; then
        while IFS=$'\t' read -r target _ executable; do
            [ -x "$executable" ] || die "unpack: the bin $target the tests spawn, $executable, is missing or not executable"
            spawned=$((spawned + 1))
        done <"$dir/spawned-binaries.tsv"
    fi
    echo "Unpacked $count test binaries and $spawned bin(s) they spawn, built at $built"
}

# Runs one test binary from <dir> with the remaining arguments, stopping it
# once it prints nothing for <idle> seconds (it hung) or once <deadline>
# seconds pass (a backstop below the step's own bound). libtest prints a line
# as each test completes and flags a test past 60 s, so a binary still
# completing tests is never taken for a hung one. Its output streams through
# unchanged and is kept, so a stopped run can name the tests libtest saw hang.
# The exit status is the binary's, or 124 when the watchdog stopped it.
run_with_deadline() {
    local deadline="$1" idle="$2" target="$3" dir="$4" exe="$5"
    shift 5
    local log stopped pid watchdog status=0
    log="$(mktemp)"
    stopped="$log.stopped"
    # exec, so the background job's pid is the test binary itself.
    (cd "$dir" && CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" exec "$exe" "$@") > >(tee "$log") 2>&1 &
    pid=$!
    (
        trap 'exit 0' TERM
        started=$SECONDS
        quiet_since=$SECONDS
        size=0
        while :; do
            sleep 1
            now_size=$(wc -c <"$log")
            if [ "$now_size" -ne "$size" ]; then
                size=$now_size
                quiet_since=$SECONDS
            fi
            if [ $((SECONDS - quiet_since)) -ge "$idle" ]; then
                echo "idle $((SECONDS - started))" >"$stopped"
                break
            fi
            if [ $((SECONDS - started)) -ge "$deadline" ]; then
                echo "deadline $((SECONDS - quiet_since))" >"$stopped"
                break
            fi
        done
        kill -TERM "$pid" 2>/dev/null || exit 0
        sleep 10
        kill -KILL "$pid" 2>/dev/null || true
    ) &
    watchdog=$!
    wait "$pid" || status=$?
    kill -TERM "$watchdog" 2>/dev/null || true
    wait "$watchdog" 2>/dev/null || true
    # tee drains after the binary exits; let it finish before reading the log.
    wait 2>/dev/null || true
    if [ -f "$stopped" ]; then
        status=124
        report_stopped "$target" "$deadline" "$idle" "$log" "$(cat "$stopped")"
    fi
    rm -f "$log" "$stopped"
    return "$status"
}

# Says why the watchdog stopped <target>: "idle <elapsed>" when it printed
# nothing for <idle> seconds, "deadline <quiet>" when it ran past <deadline>
# with its last output <quiet> seconds before the stop. Either names the
# tests libtest flagged with "has been running for over 60 seconds" that never
# printed a result line: the ones still running when it was stopped.
report_stopped() {
    local target="$1" deadline="$2" idle="$3" log="$4" why="$5" slow hung=""
    while IFS= read -r slow; do
        grep -qE "^test $(printf '%s' "$slow" | sed 's/[][\.*^$/]/\\&/g') \.\.\. " "$log" || hung="$hung $slow"
    done < <(sed -nE 's/^test (.+) has been running for over 60 seconds$/\1/p' "$log" | sort -u)
    local message
    case "$why" in
        idle\ *)
            message="test binary $target printed nothing for ${idle}s and was stopped ${why#idle }s in"
            if [ -n "$hung" ]; then
                message="$message; still running:$hung"
            else
                message="$message; no test had run for over 60 seconds, so the binary itself hung"
            fi
            ;;
        *)
            message="test binary $target ran past its ${deadline}s deadline and was stopped while still completing tests (its last output ${why#deadline }s before the stop): it is slow, not hung"
            [ -z "$hung" ] || message="$message; still running:$hung"
            ;;
    esac
    if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::error::$message"; else echo "test-binaries.sh: $message" >&2; fi
}

# Execs every binary <map> lists for <target> in <package dir>, from that
# directory, with the remaining arguments. cargo runs a test from its package
# directory and hands it CARGO_HOME, and tests read both (contremaitre_test
# resolves src/ against the working directory, personas_endpoint_test reads
# CARGO_HOME), so an exec supplies both. stdin is the map, so a test never
# inherits it. The `Running` line is cargo's own, relative to the workspace
# root this runs from, which refresh-pg-shard-weights.sh reads a suite's start
# from.
#
# Each binary runs under a watchdog. Without it a hung test holds its binary
# until the job's step timeout, which ends the run `cancelled` and names
# nothing. A binary that prints nothing for TEST_BINARY_IDLE_SECS (300 s by
# default; libtest prints a line per completed test, and the slowest suite's
# test takes about 120 s) hung, and is stopped. TEST_BINARY_DEADLINE_SECS
# (1500 s by default) is the backstop under the 30-minute step that runs a
# PostgreSQL shard: its `it` binary takes 650 s on a fast runner and over
# 1200 s on a slow one (the same pierre-database suite runs 32 s on one and
# 62 s on the other), so a deadline near 1200 s stops binaries still
# completing tests. Either way the run fails, and the error says which bound
# stopped it and names every test libtest flagged as running for over 60
# seconds that never reported a result: the hung ones.
run() {
    local map="$1" package_dir="$2" target="$3"
    shift 3
    [ -s "$map" ] || die "run: $map is missing or empty"
    local deadline="${TEST_BINARY_DEADLINE_SECS:-1500}" idle="${TEST_BINARY_IDLE_SECS:-300}"
    case "$deadline" in '' | *[!0-9]* | 0) die "run: TEST_BINARY_DEADLINE_SECS must be a positive whole number of seconds, got '$deadline'" ;; esac
    case "$idle" in '' | *[!0-9]* | 0) die "run: TEST_BINARY_IDLE_SECS must be a positive whole number of seconds, got '$idle'" ;; esac
    local source="tests/$target.rs" ran=0 failed=0 name dir exe
    if [ -f "$package_dir/tests/$target/main.rs" ]; then source="tests/$target/main.rs"; fi
    while IFS=$'\t' read -r name dir exe; do
        [ "$name" = "$target" ] && [ "$dir" = "$package_dir" ] || continue
        ran=1
        echo "     Running ${dir#"$PWD"/}/$source (${exe#"$PWD"/})"
        if ! run_with_deadline "$deadline" "$idle" "$target" "$dir" "$exe" "$@" </dev/null; then
            failed=1
        fi
    done <"$map"
    if [ "$ran" -eq 0 ]; then
        echo "test-binaries.sh: no binary for $target in $package_dir in $map — refusing to report green over a test that did not run" >&2
        return 1
    fi
    return "$failed"
}

command="${1:-}"
[ $# -gt 0 ] && shift
case "$command" in
    build) [ $# -ge 1 ] || die "build: no output directory"; build "$@" ;;
    unpack) [ $# -eq 1 ] || die "unpack: name the pack directory"; unpack "$1" ;;
    run) [ $# -ge 3 ] || die "run: name the map, the package directory and the target"; run "$@" ;;
    *) die "usage: test-binaries.sh build <dir> [--keep <package dir>]... <cargo test args>... | unpack <dir> | run <map> <package dir> <target> [<test args>...]" ;;
esac
