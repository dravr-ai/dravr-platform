#!/usr/bin/env bash
# ABOUTME: Fixture test for check-async-lock-guards.sh — what it refuses and what it must leave alone
# ABOUTME: Pins both catch cases, four no-false-positive cases, and the fail-closed empty scan
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A gate nobody has seen fail is a gate nobody knows works — the lockbud job
# reported green for nine months without once analysing this codebase
# (carnet#399). So every branch here is fired against a fixture tree: the two
# deadlock shapes must exit 1, the four correct shapes must exit 0, and a tree
# holding no async lock at all must exit 1 rather than pass having checked
# nothing.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-async-lock-guards.sh}"

failures=0
pass() { echo "  ✅ $1"; }
fail() {
    echo "  ❌ $1"
    failures=$((failures + 1))
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# fixture <name> <rust-body>  -> builds ROOT/crates/<name>/src/lib.rs
fixture() {
    local name="$1" body="$2"
    local root="$TMP/$name"
    mkdir -p "$root/crates/$name/src"
    printf '%s\n' "$body" > "$root/crates/$name/src/lib.rs"
    echo "$root"
}

# expect_exit <label> <root> <want>
expect_exit() {
    local label="$1" root="$2" want="$3" got=0
    "$UNDER_TEST" "$root" >"$TMP/out.txt" 2>&1 || got=$?
    if [ "$got" = "$want" ]; then
        pass "$label"
    else
        fail "$label"
        echo "      exit: got $got, expected $want"
        sed 's/^/      /' "$TMP/out.txt"
    fi
}

echo "check-async-lock-guards.sh fixture tests"

# ---------------------------------------------------------------- must FAIL
expect_exit "read guard live when write() is awaited on the same lock" \
  "$(fixture readwrite '
impl Cache {
    async fn get_or_insert(&self, v: u32) -> u32 {
        let read = self.configs.read().await;
        if let Some(f) = read.iter().find(|x| **x == v) {
            return *f;
        }
        let mut write = self.configs.write().await;
        write.push(v);
        v
    }
}')" 1

expect_exit "guard live across an awaited call on its own receiver" \
  "$(fixture reentrant '
impl Registry {
    async fn stats(&self) -> u32 {
        let guard = self.entries.read().await;
        let extra = self.recount().await;
        guard.len() as u32 + extra
    }
}')" 1

# ---------------------------------------------------------------- must PASS
expect_exit "explicit drop() before the second acquisition" \
  "$(fixture dropped '
impl Cache {
    async fn get_or_insert(&self, v: u32) -> u32 {
        let read = self.configs.read().await;
        if let Some(f) = read.iter().find(|x| **x == v) {
            return *f;
        }
        drop(read);
        let mut write = self.configs.write().await;
        write.push(v);
        v
    }
}')" 0

expect_exit "read scoped in its own block, write after it closes" \
  "$(fixture scoped '
impl Cache {
    async fn get_or_insert(&self, v: u32) -> u32 {
        {
            let read = self.configs.read().await;
            if let Some(f) = read.iter().find(|x| **x == v) {
                return *f;
            }
        }
        let mut write = self.configs.write().await;
        write.push(v);
        v
    }
}')" 0

expect_exit "temporary guard, dropped at end of statement" \
  "$(fixture temporary '
impl Cache {
    async fn sizes(&self) -> usize {
        let a = self.search.read().await.len();
        let b = self.details.read().await.len();
        a + b
    }
}')" 0

expect_exit "two DIFFERENT locks held at once is not flagged" \
  "$(fixture twolocks '
impl Service {
    async fn catalog(&self) -> usize {
        let cats = self.categories.read().await;
        let defs = self.definitions.read().await;
        cats.len() + defs.len()
    }
}')" 0

expect_exit "guard held across an await that is NOT on its receiver" \
  "$(fixture unrelated_await '
impl Service {
    async fn refresh(&self, client: &Client) -> usize {
        let cache = self.entries.read().await;
        let fetched = client.fetch_remote().await;
        cache.len() + fetched
    }
}')" 0

# ------------------------------------------------- must FAIL CLOSED (stale)
expect_exit "a tree with no async lock at all fails rather than passing" \
  "$(fixture nolocks '
impl Plain {
    fn total(&self) -> u32 {
        self.a + self.b
    }
}')" 1

expect_exit "a missing ROOT is a usage error, not a pass" \
  "$TMP/does-not-exist" 1

# ------------------------------------------- the real tree must still pass
REAL_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
expect_exit "the live workspace passes" "$REAL_ROOT" 0

echo ""
if [ "$failures" -eq 0 ]; then
    echo "✅ all check-async-lock-guards cases passed"
    exit 0
fi
echo "❌ $failures check-async-lock-guards case(s) failed"
exit 1
