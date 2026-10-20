#!/usr/bin/env bash
# ABOUTME: Maps pierre-server test suites to the binary that runs them: `tests/it` modules or a top-level file
# ABOUTME: One answer for CI shards, the changed-files lane, pre-push and coverage instead of one per script
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

# Most server suites are modules of one test binary, `tests/it/main.rs`; a suite
# that writes the process environment stays a top-level file and its own binary
# (`it/test_layout_test.rs` holds both sides of that rule). A suite is named by
# its file stem either way, so a list of suites, a changed path or a shard
# assignment reads the same; this script says how each one is built and run.
#
# Usage:
#   server-suites.sh list                 every suite: "<name>\t<it|bin>", sorted by name
#   server-suites.sh targets <suite>...   the cargo --test targets they need, deduplicated
#   server-suites.sh changed-targets      the --test targets a list of changed paths (stdin,
#                                         one per line) compiles into: `it` for anything under
#                                         tests/it/, tests/helpers/ or common.rs, a top-level
#                                         file's own name for it; other paths name none
#   server-suites.sh run [--threads N] <suite>... [-- <cargo args>]
#                                         run them: each top-level suite as its own
#                                         binary, every `it` module in one `--test it`
#                                         run filtered by `<module>::`
#
# A suite may be given as a name or as a path to its file. An unknown suite is
# an error: a caller that names a suite expects it to run.
#
# libtest filters by substring, so `goal_test::` also selects
# `season_goal_test::…`. That over-selects, which costs a little time and never
# skips a test.

set -euo pipefail

ROOT="${SERVER_SUITES_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
TESTS="$ROOT/crates/pierre-server/tests"

die() {
    echo "server-suites.sh: $*" >&2
    exit 2
}

list_suites() {
    {
        find "$TESTS" -maxdepth 1 -name '*.rs' ! -name common.rs -exec basename {} .rs \; \
            | sed 's/$/\tbin/'
        find "$TESTS/it" -maxdepth 1 -name '*.rs' ! -name main.rs -exec basename {} .rs \; \
            | sed 's/$/\tit/'
    } | LC_ALL=C sort
}

# Prints "it" or "bin" for one suite name or path; fails on an unknown one.
kind_of() {
    local name
    name="$(basename "$1" .rs)"
    if [ -f "$TESTS/it/$name.rs" ] && [ "$name" != main ]; then
        echo it
    elif [ -f "$TESTS/$name.rs" ] && [ "$name" != common ]; then
        echo bin
    else
        die "no pierre-server suite named '$name'"
    fi
}

targets() {
    local suite kind name
    for suite in "$@"; do
        kind="$(kind_of "$suite")"
        name="$(basename "$suite" .rs)"
        if [ "$kind" = it ]; then echo it; else echo "$name"; fi
    done | LC_ALL=C sort -u
}

# Name-based on purpose: a path the diff deleted still names the target it
# was part of, and the caller decides whether that target still exists.
changed_targets() {
    grep -E '^crates/pierre-server/tests/([^/]+|(it|helpers)/.+)\.rs$' \
        | sed -E 's#^crates/pierre-server/tests/((it|helpers)/.*|common\.rs)$#it#; s#.*/##; s#\.rs$##' \
        | LC_ALL=C sort -u || true
}

run() {
    local threads=4
    local suites=() cargo_args=()
    while [ $# -gt 0 ]; do
        case "$1" in
            --threads) threads="$2"; shift 2 ;;
            --) shift; cargo_args=("$@"); break ;;
            *) suites+=("$1"); shift ;;
        esac
    done
    [ "${#suites[@]}" -gt 0 ] || die "run: no suites named"

    local bins=() filters=() suite name
    for suite in "${suites[@]}"; do
        name="$(basename "$suite" .rs)"
        if [ "$(kind_of "$suite")" = it ]; then
            filters+=("${name}::")
        else
            bins+=("$name")
        fi
    done

    local failed=0
    for name in "${bins[@]+"${bins[@]}"}"; do
        group "$name"
        if ! cargo test "${cargo_args[@]+"${cargo_args[@]}"}" --test "$name" -- --test-threads="$threads"; then
            report "suite $name failed"
            failed=1
        fi
        endgroup
    done
    if [ "${#filters[@]}" -gt 0 ]; then
        group "it (${#filters[@]} suites)"
        if ! cargo test "${cargo_args[@]+"${cargo_args[@]}"}" --test it -- --test-threads="$threads" "${filters[@]}"; then
            report "a suite in the it binary failed: ${filters[*]}"
            failed=1
        fi
        endgroup
    fi
    return "$failed"
}

group() { if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::group::$1"; else echo "=== $1"; fi; }
endgroup() { if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::endgroup::"; fi; }
report() { if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::error::$1"; else echo "FAILED: $1" >&2; fi; }

command="${1:-}"
[ $# -gt 0 ] && shift
# Every command but changed-targets reads the tree; that one maps names only.
if [ "$command" != changed-targets ]; then
    [ -d "$TESTS/it" ] || die "$TESTS/it is missing"
fi
case "$command" in
    list) list_suites ;;
    targets) [ $# -gt 0 ] || die "targets: no suites named"; targets "$@" ;;
    changed-targets) changed_targets ;;
    run) run "$@" ;;
    *) die "usage: server-suites.sh list | targets <suite>... | changed-targets < paths | run [--threads N] <suite>... [-- <cargo args>]" ;;
esac
