#!/usr/bin/env bash
# ABOUTME: Fixture test for affected-crates.py — which crates a commit reaches on a throwaway workspace, and that an
# ABOUTME: unknown change, a CI definition or a manifest change outside [workspace.dependencies] tests every crate.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A crate wrongly left out of a scope keeps its tests from running on a change
# that can break them: a green nobody earned. Every rule that narrows a scope
# has a case here, and so does every rule that must widen it to every crate.
#
# The fixture workspace:
#   pierre_mcp_server -> pierre-core, serde; dev: pierre-test-support
#   pierre-cli        -> pierre-core; embeds pierre-server's tests/fixtures/push.json
#   pierre-core       -> (none); embeds packages/i18n/en.json; its migrations/ is a symlink to ../../migrations
#   pierre-test-support -> (none)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/affected-crates.py}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
R="$WORK/repo"
mkdir -p "$R"
failures=0

git -C "$R" init -q
git -C "$R" config user.email test@example.com
git -C "$R" config user.name test
git -C "$R" config commit.gpgsign false

mkdir -p "$R/crates/pierre-server/src" "$R/crates/pierre-server/tests/it" "$R/crates/pierre-core/src" \
    "$R/crates/pierre-core/tests" "$R/crates/pierre-cli/src" "$R/crates/pierre-test-support/src" \
    "$R/crates/pierre-server/tests/fixtures" \
    "$R/packages/i18n" "$R/migrations" "$R/docs" "$R/.github/workflows"
cat >"$R/Cargo.toml" <<'TOML'
[workspace]
members = ["crates/*"]

[workspace.dependencies]
serde = "1.0.0"

[profile.release]
lto = "thin"
TOML
printf '[package]\nname = "pierre_mcp_server"\n\n[dependencies]\nserde = { workspace = true }\n' \
    >"$R/crates/pierre-server/Cargo.toml"
printf '[package]\nname = "pierre-core"\n' >"$R/crates/pierre-core/Cargo.toml"
printf '[package]\nname = "pierre-cli"\n' >"$R/crates/pierre-cli/Cargo.toml"
printf '[package]\nname = "pierre-test-support"\n' >"$R/crates/pierre-test-support/Cargo.toml"
lock() { # $1 = serde version
    cat <<LOCK
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
version = "$1"
source = "registry+https://github.com/rust-lang/crates.io-index"
LOCK
}
lock 1.0.0 >"$R/Cargo.lock"
echo '{}' >"$R/packages/i18n/en.json"
echo '{}' >"$R/packages/i18n/unread.json"
echo 'const EN: &str = include_str!("../../../packages/i18n/en.json");' >"$R/crates/pierre-core/src/lib.rs"
ln -s ../../migrations "$R/crates/pierre-core/migrations"
echo '{}' >"$R/crates/pierre-server/tests/fixtures/push.json"
echo 'const PUSH: &str = include_str!("../../pierre-server/tests/fixtures/push.json");' >"$R/crates/pierre-cli/src/main.rs"
echo 'select 1;' >"$R/migrations/0001_init.sql"
for f in crates/pierre-server/src/main.rs crates/pierre-server/tests/it/main.rs crates/pierre-core/tests/core_test.rs \
    crates/pierre-cli/src/main.rs crates/pierre-test-support/src/lib.rs docs/guide.md \
    .github/workflows/ci-backend.yml crates/pierre-core/README.md; do
    echo "// base" >>"$R/$f"
done
git -C "$R" add -A
git -C "$R" commit -qm base
BASE="$(git -C "$R" rev-parse HEAD)"

# $1 = label, $2 = expected "mode|tested|linked", then a shell snippet that edits the tree from BASE.
expect() {
    local label="$1" want="$2" edit="$3" out got
    git -C "$R" checkout -q --detach "$BASE"
    git -C "$R" reset -q --hard "$BASE"
    (cd "$R" && eval "$edit")
    git -C "$R" add -A
    git -C "$R" commit -qm change --allow-empty
    out="$("$UNDER_TEST" --root "$R" scope --base "$BASE" 2>"$WORK/err")"
    got="$(sed -n 's/^mode=//p' <<<"$out")|$(sed -n 's/^tested=//p' <<<"$out")|$(sed -n 's/^linked=//p' <<<"$out")"
    if [ "$got" = "$want" ]; then
        echo "  ✅ $label"
    else
        echo "  ❌ $label — got '$got', want '$want'"
        sed 's/^/      /' "$WORK/err"
        failures=$((failures + 1))
    fi
}

ALL="pierre-cli pierre-core pierre-test-support pierre_mcp_server"
FULL="full|$ALL|$ALL"
echo "=== affected-crates.py scope"
expect "the server's own src reaches the server alone" \
    "scoped|pierre_mcp_server|pierre_mcp_server" 'echo x >>crates/pierre-server/src/main.rs'
expect "a crate's src reaches it and every crate depending on it" \
    "scoped|pierre-cli pierre-core pierre_mcp_server|pierre-cli pierre-core pierre_mcp_server" \
    'echo x >>crates/pierre-core/src/lib.rs'
expect "a dev-dependency reaches the crates testing with it" \
    "scoped|pierre-test-support pierre_mcp_server|pierre-test-support pierre_mcp_server" \
    'echo x >>crates/pierre-test-support/src/lib.rs'
expect "a crate's own test source reaches its tests alone" \
    "scoped|pierre-core|" 'echo x >>crates/pierre-core/tests/core_test.rs'
expect "a server test source reaches the server's tests alone" \
    "scoped|pierre_mcp_server|" 'echo x >>crates/pierre-server/tests/it/main.rs'
expect "a crate's README reaches nothing" "scoped||" 'echo x >>crates/pierre-core/README.md'
expect "docs reach nothing" "scoped||" 'echo x >>docs/guide.md'
expect "a file a crate embeds reaches that crate and its dependents" \
    "scoped|pierre-cli pierre-core pierre_mcp_server|pierre-cli pierre-core pierre_mcp_server" \
    'echo "{\"a\":1}" >packages/i18n/en.json'
expect "a member's file another member embeds reaches the embedding crate too" \
    "scoped|pierre-cli pierre_mcp_server|pierre-cli pierre_mcp_server" \
    'echo "{\"a\":1}" >crates/pierre-server/tests/fixtures/push.json'
expect "a client file no crate reads reaches nothing" "scoped||" 'echo "{\"a\":1}" >packages/i18n/unread.json'
expect "a migration reaches the crate symlinking it and its dependents" \
    "scoped|pierre-cli pierre-core pierre_mcp_server|pierre-cli pierre-core pierre_mcp_server" \
    'echo "select 2;" >migrations/0002_next.sql'
expect "a lockfile bump reaches the crates depending on the package" \
    "scoped|pierre_mcp_server|pierre_mcp_server" "lock 1.0.1 >Cargo.lock"
expect "a [workspace.dependencies] bump reaches the crates naming it" \
    "scoped|pierre_mcp_server|pierre_mcp_server" \
    "sed -i.bak 's/^serde = \"1.0.0\"/serde = \"1.0.1\"/' Cargo.toml && rm Cargo.toml.bak && lock 1.0.1 >Cargo.lock"
expect "a root manifest change outside [workspace.dependencies] reaches every crate" "$FULL" \
    "sed -i.bak 's/^lto = \"thin\"/lto = \"fat\"/' Cargo.toml && rm Cargo.toml.bak"
expect "a definition the scoped lanes run tests every crate" "$FULL" 'echo x >>.github/workflows/ci-backend.yml'
expect "the toolchain pin tests every crate" "$FULL" 'printf "[toolchain]\n" >rust-toolchain.toml'
expect "an unclassified root path tests every crate" "$FULL" 'mkdir -p mystery && echo x >mystery/file'
expect "a path under crates/ in no member tests every crate" "$FULL" \
    'mkdir -p crates/removed/src && echo x >crates/removed/src/lib.rs'
expect "a new crate depending on another reaches itself" \
    "scoped|pierre-new|pierre-new" \
    'mkdir -p crates/pierre-new/src && printf "[package]\nname = \"pierre-new\"\n" >crates/pierre-new/Cargo.toml && echo x >crates/pierre-new/src/lib.rs && printf "\n[[package]]\nname = \"pierre-new\"\nversion = \"0.1.0\"\ndependencies = [\"pierre-core\"]\n" >>Cargo.lock'
expect "no change reaches nothing" "scoped||" ':'

echo "=== affected-crates.py scope fails toward every crate"
out="$("$UNDER_TEST" --root "$R" scope --base 0000000000000000000000000000000000000000 2>"$WORK/err")" && code=0 || code=$?
if [ "$code" = 0 ] && grep -qx 'mode=full' <<<"$out" && grep -q 'Scope unknown' "$WORK/err"; then
    echo "  ✅ an unknown base answers mode=full with a warning, exit 0"
else
    echo "  ❌ an unknown base answers mode=full with a warning, exit 0 — exit $code, got: $out"
    failures=$((failures + 1))
fi
git -C "$R" checkout -q --detach "$BASE"
echo 'not [ toml' >"$R/Cargo.lock"
out="$("$UNDER_TEST" --root "$R" scope --base "$BASE" 2>"$WORK/err")"
if grep -qx 'mode=full' <<<"$out"; then
    echo "  ✅ a Cargo.lock that does not parse answers mode=full"
else
    echo "  ❌ a Cargo.lock that does not parse answers mode=full — got: $out"
    failures=$((failures + 1))
fi
git -C "$R" checkout -q -- Cargo.lock

echo "=== affected-crates.py classify-server"
got="$(printf '%s\n' crates/pierre-core/src/lib.rs crates/pierre-cli/src/main.rs \
    crates/pierre-server/tests/it/main.rs Cargo.lock docs/guide.md \
    | "$UNDER_TEST" --root "$R" classify-server | cut -f1 | tr '\n' ' ')"
if [ "$got" = "full none tests full none " ]; then
    echo "  ✅ server classification: closure src full, outside none, test source tests, lockfile with no base full, docs none"
else
    echo "  ❌ server classification — got '$got'"
    failures=$((failures + 1))
fi

echo ""
if [ "$failures" -gt 0 ]; then
    echo "❌ affected-crates.test.sh: $failures case(s) failed"
    exit 1
fi
echo "✅ affected-crates.test.sh: every case passed"
