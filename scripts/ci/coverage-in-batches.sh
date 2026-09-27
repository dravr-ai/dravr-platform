#!/usr/bin/env bash
# ABOUTME: Runs the workspace's tests under cargo-llvm-cov in batches, deleting each batch's test executables
# ABOUTME: Disk peaks at one batch of instrumented binaries instead of all ~880, and one lcov file merges every batch
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why batches. Every integration-test file is its own executable, and each one
# statically links the workspace with coverage counters compiled in. A single
# `cargo llvm-cov --tests` links all of them before running any, so the job's
# disk grows with the number of test files: 616 files in pierre-server fitted
# the runner's 113 GB on 2026-09-20, and 660 did not a week later (run
# 36308946454 died with "No space left on device"). Here the dependencies and
# workspace libraries are compiled once, and each batch's executables are
# deleted once the batch has run and been reported.
#
# How the pieces fit:
# - Every batch selects `--workspace`, so cargo unifies features over the same
#   package set each time and no dependency is compiled twice. Batches differ
#   only in the targets they name.
# - `--no-report` skips cargo-llvm-cov's clean step (the two flags are mutually
#   exclusive), so a batch reuses everything the previous one compiled.
# - `cargo llvm-cov report` maps the batch's profraw files onto the executables
#   on disk. The profraw files go with the executables, so no batch reports
#   another's counts, and Codecov merges the concatenated records per file.
# - Test executables are never uplifted, so they have one link. A bin target is
#   hard-linked into target/.../debug/ on Linux, which gives its deps/ copy two
#   links: `-links 1` keeps the workspace bins (pierre-cli's tests spawn theirs
#   through CARGO_BIN_EXE) instead of relinking them every batch. Where cargo
#   copies rather than links (macOS), a bin is deleted too and cargo relinks it
#   when a later batch needs it.
#
# The test targets are the ones `--tests` would build: every [[test]] target
# cargo marks as tested (`test = true`, the default) whose required-features
# the workspace's resolved features satisfy. `--tests` skips the others
# silently, while naming one with `--test` runs a `test = false` target and is
# a hard error for unsatisfied features, so the same filter keeps the two
# equivalent; each skipped target is printed. An example or bench with
# `test = true` would also be tested by `--tests`, and none exists, so the run
# refuses one rather than dropping its coverage.
#
# Usage:  coverage-in-batches.sh <lcov output path> [-- <test binary args>...]
# Env:    COVERAGE_BATCH_SIZE   test targets per batch (default 150)

set -euo pipefail

if [ "$#" -lt 1 ] || [ "$1" = "--" ]; then
    echo "usage: coverage-in-batches.sh <lcov output path> [-- <test binary args>...]" >&2
    exit 2
fi
OUT="$1"
shift
[ "${1:-}" = "--" ] && shift
TEST_ARGS=("$@")
BATCH_SIZE="${COVERAGE_BATCH_SIZE:-150}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

cargo metadata --format-version 1 >"$WORK/metadata.json"
# Profraw files land in the target dir, executables in the build dir, which is
# the target dir unless cargo's build-dir is configured.
TARGET_ROOT="${CARGO_LLVM_COV_TARGET_DIR:-$(jq -r '.target_directory' "$WORK/metadata.json")/llvm-cov-target}"
BUILD_ROOT="${CARGO_LLVM_COV_BUILD_DIR:-$(jq -r '.build_directory // .target_directory' "$WORK/metadata.json")/llvm-cov-target}"
DEPS="$BUILD_ROOT/debug/deps"

# shellcheck disable=SC2016 # the $names are jq's variables, never the shell's
MEMBER_TARGETS='
  (.resolve.nodes | map({key: .id, value: .features}) | from_entries) as $resolved
  | .workspace_members as $members
  | .packages[]
  | .id as $id
  | select($members | index($id))
  | .name as $pkg
  | .targets[]'

TESTED_OTHERS="$(jq -r "$MEMBER_TARGETS"'
  | select((.kind == ["example"] or .kind == ["bench"]) and .test)
  | "\($pkg)/\(.name) (\(.kind[0]))"' "$WORK/metadata.json")"
if [ -n "$TESTED_OTHERS" ]; then
    echo "::error::test = true on targets this batcher does not select, so their coverage would drop:"
    printf '  %s\n' "$TESTED_OTHERS"
    exit 1
fi

# One row per [[test]] target: name, package, and why it is skipped (empty when it runs).
jq -r "$MEMBER_TARGETS"'
  | select(.kind == ["test"])
  | ((.["required-features"] // []) - ($resolved[$id] // [])) as $missing
  | [.name, $pkg,
     (if .test == false then "test = false"
      elif ($missing | length) > 0 then "features " + ($missing | join(","))
      else "" end)]
  | @tsv' "$WORK/metadata.json" >"$WORK/targets.tsv"

awk -F'\t' '$3 != "" { printf "skipped: %s/%s has %s\n", $2, $1, $3 }' "$WORK/targets.tsv"
# Two packages may name a test target alike; one `--test <name>` builds both.
mapfile -t TARGETS < <(awk -F'\t' '$3 == "" { print $1 }' "$WORK/targets.tsv" | sort -u)
TARGET_COUNT="$(awk -F'\t' '$3 == ""' "$WORK/targets.tsv" | wc -l | tr -d ' ')"
if [ "${#TARGETS[@]}" -eq 0 ]; then
    echo "::error::No test targets selected — refusing to report green."
    exit 1
fi

run_batch() {
    local label="$1"
    shift
    echo "::group::coverage batch ${label}"
    cargo llvm-cov --no-report --workspace "$@" -- ${TEST_ARGS[@]+"${TEST_ARGS[@]}"}
    cargo llvm-cov report --lcov --output-path "$WORK/lcov-${label}.info"
    if [ ! -d "$DEPS" ]; then
        echo "::error::${DEPS} does not exist — cargo's build layout moved; point DEPS at the test executables"
        exit 1
    fi
    # An executable's split debuginfo and dep-info files share its name as a prefix.
    find "$DEPS" -maxdepth 1 -type f -perm -u+x ! -name '*.*' -links 1 \
        -exec sh -c 'rm -f -- "$1" "$1".*' _ {} \;
    # Not `cargo llvm-cov clean --profraw-only`: 0.6.18 also removes the
    # workspace rlibs and fingerprints, so every batch would recompile them.
    find "$TARGET_ROOT" -maxdepth 1 -type f -name '*.profraw' -delete
    df -h "$TARGET_ROOT" | tail -1
    echo "::endgroup::"
}

# A restored cache may hold workspace artifacts from another commit, which the
# plain `cargo llvm-cov` this replaces removed on its own.
cargo llvm-cov clean --workspace

run_batch units --lib --bins
BATCHES=$(( (${#TARGETS[@]} + BATCH_SIZE - 1) / BATCH_SIZE ))
echo "=== ${TARGET_COUNT} test targets under ${#TARGETS[@]} names, in ${BATCHES} batch(es) of up to ${BATCH_SIZE} names ==="
for (( b = 0; b < BATCHES; b++ )); do
    FLAGS=()
    for name in "${TARGETS[@]:$(( b * BATCH_SIZE )):$BATCH_SIZE}"; do
        FLAGS+=(--test "$name")
    done
    run_batch "tests-$(( b + 1 ))" "${FLAGS[@]}"
done

# Not `cat`: cargo-llvm-cov ends each report without a newline, so `cat` would
# glue a batch's first SF: record onto the previous batch's end_of_record.
awk 1 "$WORK"/lcov-*.info >"$OUT"
echo "coverage for $(grep -c '^SF:' "$OUT") file records written to $OUT"
