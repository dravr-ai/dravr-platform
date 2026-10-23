#!/usr/bin/env bash
# ABOUTME: Fixture test for test-binaries.sh — a build packs exactly the test executables cargo reports, with the cargo git checkouts,
# ABOUTME: an unpack restores them runnable, and a run execs one package's binaries as cargo would; each refuses what it cannot vouch for
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A `cargo` stub on PATH plays `cargo test --no-run`: it writes two test
# executables and a library under the fixture's target/ (or where
# CARGO_STUB_DEPS says), prints cargo's JSON messages for them, and fails or
# reports nothing when told to. What is under test is the selection, the pack,
# the restore and the run, never cargo itself.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/test-binaries.sh}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
# The workspace path is compared as text, so resolve the temp dir's symlinks
# (macOS's /var -> /private/var) the way `pwd` inside it reports them.
WORK="$(cd "$WORK" && pwd -P)"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

TREE="$WORK/tree"
mkdir -p "$TREE/crates/pierre-server" "$TREE/crates/pierre-database" "$WORK/bin" \
    "$WORK/cargo-home/git/checkouts/dravr-contremaitre-0a1b2c/5bc9599/config"
echo "contracts: []" >"$WORK/cargo-home/git/checkouts/dravr-contremaitre-0a1b2c/5bc9599/config/persona_contracts.yaml"

cat >"$WORK/bin/cargo" <<'STUB'
#!/bin/sh
[ -n "${CARGO_STUB_FAIL:-}" ] && { echo "error: linking with cc failed: signal 9" >&2; exit 101; }
[ "$1" = test ] || exit 1
deps="${CARGO_STUB_DEPS:-$PWD/target/debug/deps}"
mkdir -p "$deps"
[ -n "${CARGO_STUB_EMPTY:-}" ] && { echo '{"reason":"build-finished","success":true}'; exit 0; }
for exe in it-1111 smoke_test-2222; do
    printf '#!/bin/sh\necho "%s ran in $PWD with $*"\n' "$exe" >"$deps/$exe"
    chmod +x "$deps/$exe"
done
echo "not json: a build script's stray line"
printf '{"reason":"compiler-artifact","target":{"name":"pierre_core","kind":["lib"]},"manifest_path":"%s/crates/pierre-core/Cargo.toml","executable":null}\n' "$PWD"
printf '{"reason":"compiler-artifact","target":{"name":"it","kind":["test"]},"manifest_path":"%s/crates/pierre-server/Cargo.toml","executable":"%s/it-1111"}\n' "$PWD" "$deps"
printf '{"reason":"compiler-artifact","target":{"name":"smoke_test","kind":["test"]},"manifest_path":"%s/crates/pierre-database/Cargo.toml","executable":"%s/smoke_test-2222"}\n' "$PWD" "$deps"
echo '{"reason":"build-finished","success":true}'
STUB
chmod +x "$WORK/bin/cargo"

PACK="$WORK/pack"
OUT="$WORK/out"

# $1 = label, $2 = expected exit (0 or nonzero), $3 = directory to run in, $4.. = args.
# Optional: CARGO_STUB_FAIL / CARGO_STUB_EMPTY / CARGO_STUB_DEPS, HOME_DIR for CARGO_HOME.
# GITHUB_ACTIONS and GITHUB_STEP_SUMMARY are blanked: under CI the script
# would otherwise write the stub build's "peak memory" into the real job's
# summary.
expect() {
    local label="$1" want="$2" dir="$3" code=0
    shift 3
    (cd "$dir" && env PATH="$WORK/bin:$PATH" CARGO_HOME="${HOME_DIR:-$WORK/cargo-home}" \
        CARGO_STUB_FAIL="${CARGO_STUB_FAIL:-}" CARGO_STUB_EMPTY="${CARGO_STUB_EMPTY:-}" \
        CARGO_STUB_DEPS="${CARGO_STUB_DEPS:-}" GITHUB_ACTIONS= GITHUB_STEP_SUMMARY= \
        "$UNDER_TEST" "$@") >"$OUT" 2>&1 || code=$?
    if { [ "$want" = 0 ] && [ "$code" -eq 0 ]; } || { [ "$want" != 0 ] && [ "$code" -ne 0 ]; }; then
        pass "$label"
    else
        fail "$label (exit $code)"
        sed 's/^/      /' "$OUT"
    fi
}

echo "test-binaries.sh fixture tests"

expect "a build packs the lane" 0 "$TREE" build "$PACK" --features postgresql --test it
if [ "$(cut -f1 "$PACK/test-binaries.tsv" | tr '\n' ' ')" = "it smoke_test " ]; then
    pass "the map holds the test targets only, library and stray lines left out"
else
    fail "the map holds the test targets only"
    sed 's/^/      /' "$PACK/test-binaries.tsv"
fi
if [ "$(cut -f2 "$PACK/test-binaries.tsv" | head -1)" = "$TREE/crates/pierre-server" ]; then
    pass "the map names each binary's package directory"
else
    fail "the map names each binary's package directory"
fi
if [ "$(cat "$PACK/workspace")" = "$TREE" ]; then pass "the pack records the build's workspace"; else
    fail "the pack records the build's workspace"
fi
if grep -q "Peak memory during the build" "$OUT"; then pass "the build reports its peak memory"; else
    fail "the build reports its peak memory"
    sed 's/^/      /' "$OUT"
fi

rm -rf "$TREE/target"
HOME_DIR="$WORK/fresh-home" expect "an unpack restores the binaries" 0 "$TREE" unpack "$PACK"
if [ -x "$TREE/target/debug/deps/it-1111" ] && [ -x "$TREE/target/debug/deps/smoke_test-2222" ]; then
    pass "the restored binaries are executable"
else
    fail "the restored binaries are executable"
fi
if [ "$(cd "$TREE/crates/pierre-server" && "$TREE/target/debug/deps/it-1111" --test-threads=4)" = \
    "it-1111 ran in $TREE/crates/pierre-server with --test-threads=4" ]; then
    pass "a restored binary runs"
else
    fail "a restored binary runs"
fi
if [ -f "$WORK/fresh-home/git/checkouts/dravr-contremaitre-0a1b2c/5bc9599/config/persona_contracts.yaml" ]; then
    pass "the cargo git checkouts land in the shard's CARGO_HOME"
else
    fail "the cargo git checkouts land in the shard's CARGO_HOME"
fi

# Run: prebuilt binaries that record how they were started, keyed by package
# as the map is. pierre-core owns a smoke_test too, as every crate with suites
# owns an `it` for real.
RUNLOG="$WORK/runlog"
RUNBIN="$WORK/runbin"
mkdir -p "$RUNBIN" "$TREE/crates/pierre-server/tests/it"
: >"$TREE/crates/pierre-server/tests/it/main.rs"
for name in it smoke other; do
    printf '#!/bin/sh\necho "%s cwd=$PWD home=$CARGO_HOME args=$*" >>"%s"\n' "$name" "$RUNLOG" >"$RUNBIN/$name"
done
printf '#!/bin/sh\nexit 101\n' >"$RUNBIN/red"
chmod +x "$RUNBIN"/*
RUNMAP="$WORK/run-map.tsv"
printf 'it\t%s\t%s\nsmoke_test\t%s\t%s\nsmoke_test\t%s\t%s\n' \
    "$TREE/crates/pierre-server" "$RUNBIN/it" \
    "$TREE/crates/pierre-database" "$RUNBIN/smoke" \
    "$TREE/crates/pierre-core" "$RUNBIN/other" >"$RUNMAP"

: >"$RUNLOG"
HOME_DIR=/fixture/cargo-home expect "a run execs the package's binary of the target" 0 "$TREE" \
    run "$RUNMAP" "$TREE/crates/pierre-database" smoke_test --test-threads=4
if grep -qxF "smoke cwd=$TREE/crates/pierre-database home=/fixture/cargo-home args=--test-threads=4" "$RUNLOG"; then
    pass "it runs from the package directory with CARGO_HOME and the test arguments"
else
    fail "it runs from the package directory with CARGO_HOME and the test arguments"
    sed 's/^/      /' "$RUNLOG"
fi
if grep -q '^other ' "$RUNLOG"; then fail "another package's binary of the same name stays unrun"; else
    pass "another package's binary of the same name stays unrun"
fi
# refresh-pg-shard-weights.sh times a suite from cargo's own `Running` line.
if grep -qF "Running crates/pierre-database/tests/smoke_test.rs (" "$OUT"; then
    pass "a file target prints cargo's Running line for its source"
else
    fail "a file target prints cargo's Running line for its source"
    sed 's/^/      /' "$OUT"
fi
expect "a run of a directory target passes" 0 "$TREE" run "$RUNMAP" "$TREE/crates/pierre-server" it
if grep -qF "Running crates/pierre-server/tests/it/main.rs (" "$OUT"; then
    pass "a directory target prints cargo's Running line for its main.rs"
else
    fail "a directory target prints cargo's Running line for its main.rs"
    sed 's/^/      /' "$OUT"
fi
expect "a run of a target the package has no binary for fails" 1 "$TREE" \
    run "$RUNMAP" "$TREE/crates/pierre-server" smoke_test
if grep -qF "no binary for smoke_test in $TREE/crates/pierre-server" "$OUT"; then
    pass "the failure names the target and the package"
else
    fail "the failure names the target and the package"
    sed 's/^/      /' "$OUT"
fi
printf 'smoke_test\t%s\t%s\n' "$TREE/crates/pierre-database" "$RUNBIN/red" >"$WORK/red-map.tsv"
expect "a run whose binary fails fails" 1 "$TREE" run "$WORK/red-map.tsv" "$TREE/crates/pierre-database" smoke_test
expect "a run over an empty map is refused" 1 "$TREE" run "$WORK/empty-map.tsv" "$TREE/crates/pierre-database" smoke_test

# A binary libtest says has a test past 60 s, which then never reports it.
cat >"$RUNBIN/hung" <<'HUNG'
#!/bin/sh
echo "running 2 tests"
echo "test slow::finishes has been running for over 60 seconds"
echo "test slow::hangs has been running for over 60 seconds"
echo "test slow::finishes ... ok"
exec sleep 30
HUNG
printf '#!/bin/sh\nexec sleep 30\n' >"$RUNBIN/silent"
chmod +x "$RUNBIN/hung" "$RUNBIN/silent"
printf 'smoke_test\t%s\t%s\n' "$TREE/crates/pierre-database" "$RUNBIN/hung" >"$WORK/hung-map.tsv"
printf 'smoke_test\t%s\t%s\n' "$TREE/crates/pierre-database" "$RUNBIN/silent" >"$WORK/silent-map.tsv"
started=$SECONDS
TEST_BINARY_DEADLINE_SECS=2 expect "a run whose binary passes its deadline fails" 1 "$TREE" \
    run "$WORK/hung-map.tsv" "$TREE/crates/pierre-database" smoke_test
if [ $((SECONDS - started)) -lt 20 ]; then pass "the deadline stops the binary instead of waiting it out"; else
    fail "the deadline stops the binary instead of waiting it out ($((SECONDS - started))s)"
fi
if grep -qF "ran past its 2s deadline and was stopped; still running: slow::hangs" "$OUT" \
    && ! grep -qF "still running: slow::finishes" "$OUT" && ! grep -qF " slow::hangs slow::finishes" "$OUT"; then
    pass "the failure names the test that never finished, and only it"
else
    fail "the failure names the test that never finished, and only it"
    sed 's/^/      /' "$OUT"
fi
if grep -qF "test slow::finishes ... ok" "$OUT"; then pass "the binary's own output still streams through"; else
    fail "the binary's own output still streams through"
    sed 's/^/      /' "$OUT"
fi
TEST_BINARY_DEADLINE_SECS=1 expect "a binary that hangs outside any test fails" 1 "$TREE" \
    run "$WORK/silent-map.tsv" "$TREE/crates/pierre-database" smoke_test
if grep -qF "no test had run for over 60 seconds, so the binary itself hung" "$OUT"; then
    pass "a hang with no slow test says the binary hung"
else
    fail "a hang with no slow test says the binary hung"
    sed 's/^/      /' "$OUT"
fi
TEST_BINARY_DEADLINE_SECS=soon expect "a deadline that is not a number of seconds is refused" 1 "$TREE" \
    run "$RUNMAP" "$TREE/crates/pierre-database" smoke_test

mkdir -p "$WORK/elsewhere"
expect "an unpack in another workspace path is refused" 1 "$WORK/elsewhere" unpack "$PACK"
if grep -q "were built in $TREE" "$OUT"; then pass "the refusal names the build's workspace"; else
    fail "the refusal names the build's workspace"
fi

printf 'ghost\t%s/crates/pierre-server\t%s/target/debug/deps/ghost-3333\n' "$TREE" "$TREE" >>"$PACK/test-binaries.tsv"
expect "an unpack whose map names a binary the archive lacks fails" 1 "$TREE" unpack "$PACK"

expect "an unpack of a directory holding no pack fails" 1 "$TREE" unpack "$WORK/elsewhere"
CARGO_STUB_FAIL=1 expect "a failed build fails" 1 "$TREE" build "$WORK/pack-failed" --test it
if grep -q "Peak memory during the build" "$OUT"; then pass "a failed build still reports its peak memory"; else
    fail "a failed build still reports its peak memory"
fi
CARGO_STUB_EMPTY=1 expect "a build that produced no test binary fails" 1 "$TREE" build "$WORK/pack-empty" --test it
expect "a build with no cargo arguments is refused" 1 "$TREE" build "$WORK/pack-noargs"

expect "a build keeping one package packs only its binaries" 0 "$TREE" \
    build "$WORK/pack-kept" --keep crates/pierre-server/ --test it
if [ "$(cut -f1,2 "$WORK/pack-kept/test-binaries.tsv")" = "it	$TREE/crates/pierre-server" ] \
    && [ "$(zstd -q -dc "$WORK/pack-kept/binaries.tar.zst" | tar -tf - | tr '\n' ' ')" = "target/debug/deps/it-1111 " ]; then
    pass "the kept map and archive hold the kept package's binary alone"
else
    fail "the kept map and archive hold the kept package's binary alone"
    sed 's/^/      /' "$WORK/pack-kept/test-binaries.tsv"
fi
expect "a build keeping a package that built nothing fails" 1 "$TREE" \
    build "$WORK/pack-kept-none" --keep crates/pierre-core --test it
if grep -qF -- "--keep crates/pierre-core matched no test binary" "$OUT"; then
    pass "the refusal names the kept package"
else
    fail "the refusal names the kept package"
    sed 's/^/      /' "$OUT"
fi
expect "a build keeping a package that built nothing beside one that built fails" 1 "$TREE" \
    build "$WORK/pack-kept-one-none" --keep crates/pierre-server --keep crates/pierre-core --test it
if grep -qF -- "--keep crates/pierre-core matched no test binary" "$OUT"; then
    pass "the refusal names the package that built nothing"
else
    fail "the refusal names the package that built nothing"
    sed 's/^/      /' "$OUT"
fi
expect "a --keep naming no package is refused" 1 "$TREE" build "$WORK/pack-kept-bare" --keep
if grep -qF -- "--keep names no package directory" "$OUT"; then
    pass "the refusal says --keep named no package"
else
    fail "the refusal says --keep named no package"
    sed 's/^/      /' "$OUT"
fi
CARGO_STUB_DEPS="$WORK/outside/deps" expect "a build whose binary lands outside the workspace is refused" 1 \
    "$TREE" build "$WORK/pack-outside" --test it
if grep -qF "is outside the workspace $TREE" "$OUT"; then pass "the refusal names the workspace"; else
    fail "the refusal names the workspace"
    sed 's/^/      /' "$OUT"
fi

echo ""
if [ "$failures" -gt 0 ]; then
    echo "❌ $failures case(s) failed"
    exit 1
fi
echo "✅ all test-binaries.sh cases passed"
