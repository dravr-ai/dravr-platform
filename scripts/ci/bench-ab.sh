#!/usr/bin/env bash
# ABOUTME: Benches a base commit and the checked-out head on one machine, suite by suite, so Criterion compares code, not runners
# ABOUTME: Base runs from a detached worktree sharing the target dir; head then compares against the `base` baseline it saved
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Usage: bench-ab.sh <base-commit> [suite]
#   <base-commit>  what the head is measured against: the merge-base for a
#                  branch, main as it was a day ago for the nightly run
#   [suite]        one bench target of pierre_mcp_server; every one when omitted
#
# The base and the head run on the same runner, alternating one suite at a
# time so a slow stretch of the machine lands on both sides. A baseline another
# runner recorded on another day cannot gate: the spread between runners alone
# reaches the fail threshold.
#
# Both builds share one target directory, so the head build reuses every
# dependency the base build compiled. CRITERION_HOME pins where both write, so
# the head's run finds the `base` baseline and writes change/estimates.json for
# bench-regression-check.py.
#
# A suite the base does not have is benched on the head alone and reported as
# such; it has nothing to regress against. A suite the head lacks is not run.

set -euo pipefail

die() {
    echo "bench-ab.sh: $*" >&2
    exit 2
}

[ $# -ge 1 ] && [ $# -le 2 ] || die "usage: bench-ab.sh <base-commit> [suite]"
base_commit="$1"
only_suite="${2:-}"

root="$(git rev-parse --show-toplevel)"
benches="crates/pierre-server/benches"
[ -d "$root/$benches" ] || die "$benches is missing"
git -C "$root" rev-parse --verify --quiet "$base_commit^{commit}" >/dev/null \
    || die "base commit $base_commit is not in this clone (fetch with full history)"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"
export CRITERION_HOME="${CRITERION_HOME:-$CARGO_TARGET_DIR/criterion}"

base_tree="${BENCH_BASE_TREE:-${RUNNER_TEMP:-${TMPDIR:-/tmp}}/bench-base}"
rm -rf "$base_tree"
git -C "$root" worktree prune
git -C "$root" worktree add --detach --quiet "$base_tree" "$base_commit"
trap 'git -C "$root" worktree remove --force "$base_tree" >/dev/null 2>&1 || true' EXIT
# The base tree sits outside the checkout, so cargo would not find the
# checkout's .cargo/config.toml from it: the linker and code-alignment flags CI
# writes there. Both sides must build alike or the comparison measures flags.
if [ -f "$root/.cargo/config.toml" ]; then
    mkdir -p "$base_tree/.cargo"
    cp "$root/.cargo/config.toml" "$base_tree/.cargo/config.toml"
fi

suites=()
if [ -n "$only_suite" ]; then
    [ -f "$root/$benches/$only_suite.rs" ] || die "the head has no bench suite $only_suite"
    suites=("$only_suite")
else
    for file in "$root/$benches"/*.rs; do
        suites+=("$(basename "$file" .rs)")
    done
fi
[ "${#suites[@]}" -gt 0 ] || die "no bench suites under $benches"

echo "Benching ${#suites[@]} suite(s): base $(git -C "$root" rev-parse --short "$base_commit") against head $(git -C "$root" rev-parse --short HEAD)"
head_only=()
for suite in "${suites[@]}"; do
    if [ -f "$base_tree/$benches/$suite.rs" ]; then
        echo "=== $suite: base"
        (cd "$base_tree" && cargo bench -p pierre_mcp_server --bench "$suite" -- --save-baseline base)
        echo "=== $suite: head"
        (cd "$root" && cargo bench -p pierre_mcp_server --bench "$suite" -- --baseline-lenient base)
    else
        echo "=== $suite: head only, the base has no such suite"
        head_only+=("$suite")
        (cd "$root" && cargo bench -p pierre_mcp_server --bench "$suite")
    fi
done

if [ "${#head_only[@]}" -gt 0 ]; then
    echo "Suites with no base to compare against: ${head_only[*]}"
fi
