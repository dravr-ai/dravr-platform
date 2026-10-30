#!/usr/bin/env bash
# ABOUTME: Fixture test for server-suites.sh — which server test targets a changed path selects, an unreadable crate graph selecting
# ABOUTME: every target, and `run --binaries` execing pierre-server's prebuilt binaries as cargo would, failing on a missing one.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Each case builds a throwaway tree (SERVER_SUITES_ROOT) holding a pierre-server
# tests/ layout, a workspace manifest per crate and a Cargo.lock, which is
# where affected-crates.py reads the crate graph. What is under test is the
# mapping from a changed path to targets; the closure is the fixture's graph:
#   pierre_mcp_server -> pierre-core (normal), pierre-test-support (dev)
#   pierre-cli        -> pierre-core   (outside the server's closure)
# pierre-core embeds packages/i18n/src/locales/en/translation.json.
# A `cargo` on PATH fails every call: the run cases below must never reach it.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/server-suites.sh}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

TREE="$WORK/tree"
mkdir -p "$TREE/crates/pierre-server/tests/it" "$TREE/crates/pierre-core/src" \
    "$TREE/crates/pierre-cli/src" "$TREE/crates/pierre-test-support/src" "$WORK/bin" \
    "$TREE/packages/i18n/src/locales/en"
printf 'mod helpers;\nmod routes_test;\nmod goal_test;\n' >"$TREE/crates/pierre-server/tests/it/main.rs"
: >"$TREE/crates/pierre-server/tests/env_writer_test.rs"
: >"$TREE/crates/pierre-server/tests/common.rs"
# A top-level suite compiling the shared modules in, as eight real ones do.
printf 'mod common;\nmod helpers;\n' >"$TREE/crates/pierre-server/tests/shared_user_test.rs"
printf '[workspace]\nmembers = ["crates/*"]\n' >"$TREE/Cargo.toml"
for crate in pierre-server:pierre_mcp_server pierre-core:pierre-core pierre-cli:pierre-cli \
    pierre-test-support:pierre-test-support; do
    printf '[package]\nname = "%s"\n' "${crate#*:}" >"$TREE/crates/${crate%%:*}/Cargo.toml"
done
echo '{}' >"$TREE/packages/i18n/src/locales/en/translation.json"
echo 'const EN: &str = include_str!("../../../packages/i18n/src/locales/en/translation.json");' \
    >"$TREE/crates/pierre-core/src/lib.rs"
cat >"$TREE/Cargo.lock" <<'LOCK'
version = 4

[[package]]
name = "pierre-cli"
version = "0.1.0"
dependencies = ["pierre-core"]

[[package]]
name = "pierre-core"
version = "0.1.0"

[[package]]
name = "pierre-test-support"
version = "0.1.0"

[[package]]
name = "pierre_mcp_server"
version = "0.1.0"
dependencies = ["pierre-core", "pierre-test-support", "serde"]

[[package]]
name = "serde"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
LOCK
cp "$TREE/Cargo.lock" "$WORK/Cargo.lock.good"

cat >"$WORK/bin/cargo" <<'STUB'
#!/bin/sh
echo "stub cargo: unexpected call: $*" >&2
exit 101
STUB
chmod +x "$WORK/bin/cargo"

FULL="$(printf '%s\n' env_writer_test it shared_user_test)"
OUT="$WORK/out"
ERR="$WORK/err"

# $1 = label, $2 = expected stdout (newline-separated), $3.. = changed paths.
# Optional: MODE=--edited-only, FAIL=1 for a Cargo.lock that does not parse.
expect_targets() {
    local label="$1" want="$2" code=0
    shift 2
    if [ -n "${FAIL:-}" ]; then echo 'not [ toml' >"$TREE/Cargo.lock"; else cp "$WORK/Cargo.lock.good" "$TREE/Cargo.lock"; fi
    printf '%s\n' "$@" | env PATH="$WORK/bin:$PATH" SERVER_SUITES_ROOT="$TREE" \
        "$UNDER_TEST" changed-targets ${MODE:+"$MODE"} >"$OUT" 2>"$ERR" || code=$?
    if [ "$code" -ne 0 ]; then
        fail "$label (exit $code)"
        sed 's/^/      /' "$ERR"
    elif [ "$(cat "$OUT")" = "$want" ]; then
        pass "$label"
    else
        fail "$label"
        echo "      wanted: $(tr '\n' ' ' <<< "$want")"
        echo "      got:    $(tr '\n' ' ' <<< "$(cat "$OUT")")"
    fi
}

echo "server-suites.sh changed-targets fixture tests"

expect_targets "a closure crate's src selects every target" "$FULL" crates/pierre-core/src/lib.rs
expect_targets "a dev-dependency's src selects every target" "$FULL" crates/pierre-test-support/src/lib.rs
expect_targets "the server's own src selects every target" "$FULL" crates/pierre-server/src/main.rs
expect_targets "a closure crate's manifest selects every target" "$FULL" crates/pierre-core/Cargo.toml
expect_targets "a closure crate's build.rs selects every target" "$FULL" crates/pierre-core/build.rs
expect_targets "a crate outside the closure selects none" "" crates/pierre-cli/src/main.rs
expect_targets "a closure crate's own test selects none" "" crates/pierre-core/tests/core_test.rs
expect_targets "a closure crate's README selects none" "" crates/pierre-core/README.md
expect_targets "an it module selects it" "it" crates/pierre-server/tests/it/routes_test.rs
expect_targets "a helper selects it and every top-level suite declaring helpers" \
    "$(printf '%s\n' it shared_user_test)" crates/pierre-server/tests/helpers/fixtures.rs
expect_targets "common.rs selects it and every top-level suite declaring common" \
    "$(printf '%s\n' it shared_user_test)" crates/pierre-server/tests/common.rs
expect_targets "a top-level suite selects its own binary" "env_writer_test" crates/pierre-server/tests/env_writer_test.rs
expect_targets "a deleted suite still names its target" "gone_test" crates/pierre-server/tests/gone_test.rs
expect_targets "a server test fixture selects every target" "$FULL" crates/pierre-server/tests/fixtures/a.json
expect_targets "the lockfile selects every target" "$FULL" Cargo.lock
expect_targets "the toolchain pin selects every target" "$FULL" rust-toolchain.toml
expect_targets "cargo config selects every target" "$FULL" .cargo/config.toml
expect_targets "a migration selects every target" "$FULL" migrations/20261020000000_x.sql
expect_targets "a PostgreSQL migration selects every target" "$FULL" migrations_pg/20261020000000_x.sql
expect_targets "a locale a closure crate embeds selects every target" "$FULL" packages/i18n/src/locales/en/translation.json
expect_targets "a client package file no crate reads selects none" "" packages/i18n/src/index.ts \
    packages/shared-constants/src/surfaces.ts infra/environments/dev/variables.tf
expect_targets "an unclassified root path selects every target" "$FULL" mystery/file.txt
expect_targets "a path under crates/ in no member selects every target" "$FULL" crates/removed-crate/src/lib.rs
expect_targets "docs, clients and CI select none" "" docs/guide.md frontend/src/App.tsx \
    frontend-mobile/src/App.tsx packages/api-client/src/index.ts .github/workflows/ci.yml README.md \
    scripts/ci/x.sh
expect_targets "one closure path among docs still selects every target" "$FULL" docs/a.md crates/pierre-core/src/x.rs
expect_targets "a test and a closure path select every target" "$FULL" \
    crates/pierre-server/tests/gone_test.rs crates/pierre-core/src/x.rs
expect_targets "no paths select none" ""

FAIL=1 expect_targets "an unreadable crate graph selects every target" "$FULL" frontend/src/App.tsx

MODE=--edited-only expect_targets "--edited-only ignores a closure crate's src" "" crates/pierre-core/src/lib.rs
MODE=--edited-only expect_targets "--edited-only maps test sources" "$(printf '%s\n' env_writer_test it)" \
    crates/pierre-server/tests/it/goal_test.rs crates/pierre-server/tests/env_writer_test.rs \
    crates/pierre-core/src/lib.rs
FAIL=1 MODE=--edited-only expect_targets "--edited-only never reads the crate graph" \
    "$(printf '%s\n' it shared_user_test)" crates/pierre-server/tests/common.rs

echo ""
echo "server-suites.sh run --binaries fixture tests"

# Prebuilt binaries record how they were run; the cargo stub fails any
# `cargo test`, so a run that reached cargo instead of the map fails too.
EXE="$WORK/exe"
RUNLOG="$WORK/runlog"
mkdir -p "$EXE"
for name in it env_writer foreign; do
    printf '#!/bin/sh\necho "%s cwd=$PWD home=$CARGO_HOME args=$*" >>"%s"\n' "$name" "$RUNLOG" >"$EXE/$name"
done
printf '#!/bin/sh\nexit 101\n' >"$EXE/red"
chmod +x "$EXE"/*
SERVER="$TREE/crates/pierre-server"
MAP="$WORK/test-binaries.tsv"
printf 'it\t%s\t%s\nenv_writer_test\t%s\t%s\nenv_writer_test\t%s\t%s\n' \
    "$SERVER" "$EXE/it" "$SERVER" "$EXE/env_writer" "$TREE/crates/pierre-cli" "$EXE/foreign" >"$MAP"

# $1 = label, $2 = expected exit (0 or nonzero), $3.. = run arguments.
expect_run() {
    local label="$1" want="$2" code=0
    shift 2
    : >"$RUNLOG"
    env PATH="$WORK/bin:$PATH" SERVER_SUITES_ROOT="$TREE" \
        CARGO_HOME=/fixture/cargo-home "$UNDER_TEST" run "$@" >"$OUT" 2>"$ERR" || code=$?
    if { [ "$want" = 0 ] && [ "$code" -eq 0 ]; } || { [ "$want" != 0 ] && [ "$code" -ne 0 ]; }; then
        pass "$label"
    else
        fail "$label (exit $code)"
        sed 's/^/      /' "$OUT" "$ERR"
    fi
}

expect_run "a top-level suite and an it module run from the map" 0 --binaries "$MAP" env_writer_test goal_test
# refresh-pg-shard-weights.sh times a suite from cargo's own `Running` line.
if grep -qF "Running crates/pierre-server/tests/env_writer_test.rs (" "$OUT" \
    && grep -qF "Running crates/pierre-server/tests/it/main.rs (" "$OUT"; then
    pass "each run prints cargo's Running line for its source"
else
    fail "each run prints cargo's Running line for its source"
    sed 's/^/      /' "$OUT"
fi
if grep -qxF "env_writer cwd=$SERVER home=/fixture/cargo-home args=--test-threads=4" "$RUNLOG"; then
    pass "a top-level binary runs from pierre-server's directory with CARGO_HOME"
else
    fail "a top-level binary runs from pierre-server's directory with CARGO_HOME"
    sed 's/^/      /' "$RUNLOG"
fi
if grep -qxF "it cwd=$SERVER home=/fixture/cargo-home args=--test-threads=4 goal_test::" "$RUNLOG"; then
    pass "the it binary runs filtered to the named module"
else
    fail "the it binary runs filtered to the named module"
    sed 's/^/      /' "$RUNLOG"
fi
if grep -q '^foreign ' "$RUNLOG"; then fail "another crate's binary of the same name stays unrun"; else
    pass "another crate's binary of the same name stays unrun"
fi
expect_run "--threads reaches the binaries" 0 --binaries "$MAP" --threads 2 routes_test
if grep -qxF "it cwd=$SERVER home=/fixture/cargo-home args=--test-threads=2 routes_test::" "$RUNLOG"; then
    pass "the thread count is the one asked for"
else
    fail "the thread count is the one asked for"
fi

grep -v '^it	' "$MAP" >"$WORK/no-it.tsv"
expect_run "an it module whose binary the map lacks fails" 1 --binaries "$WORK/no-it.tsv" goal_test
if grep -qF "no binary for it in $SERVER" "$ERR"; then pass "the failure names the missing binary"; else
    fail "the failure names the missing binary"
    sed 's/^/      /' "$ERR"
fi
printf 'it\t%s\t%s\n' "$SERVER" "$EXE/red" >"$WORK/red.tsv"
expect_run "a failing binary fails the run" 1 --binaries "$WORK/red.tsv" goal_test
expect_run "cargo arguments with --binaries are refused" 1 --binaries "$MAP" goal_test -- --features postgresql
expect_run "an empty map is refused" 1 --binaries "$WORK/empty.tsv" goal_test

echo ""
if [ "$failures" -gt 0 ]; then
    echo "❌ $failures case(s) failed"
    exit 1
fi
echo "✅ all server-suites.sh changed-targets and run --binaries cases passed"
