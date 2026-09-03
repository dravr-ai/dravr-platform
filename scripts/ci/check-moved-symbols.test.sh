#!/usr/bin/env bash
# ABOUTME: Fixture test for check-moved-symbols.sh — builds throwaway repos and asserts
# ABOUTME: each verdict, including the method-deletion and `const fn` shapes the gate must get right.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# An unverified gate is worse than no gate. Each case builds a base commit with
# a library module and an importer, applies one change as HEAD, and asserts the
# exit code the gate must produce against HEAD~1.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-moved-symbols.sh}"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

make_repo() {
  local dir
  dir="$(mktemp -d)"
  git -C "$dir" init -q
  git -C "$dir" config user.email test@example.com
  git -C "$dir" config user.name test
  mkdir -p "$dir/crates/demo/src" "$dir/crates/demo/tests"
  printf 'pub mod util;\npub mod helpers;\n' >"$dir/crates/demo/src/lib.rs"
  printf 'pub fn bar() {}\n' >"$dir/crates/demo/src/helpers.rs"
  printf '%s\n' "$dir"
}

commit_all() {
  git -C "$1" add -A
  git -C "$1" commit -qm "$2"
}

run_gate() {
  local dir="$1" code=0
  ( cd "$dir" && "$UNDER_TEST" HEAD~1 >/tmp/moved-symbols-test.out 2>&1 ) || code=$?
  echo "$code"
}

expect() { # $1 = label, $2 = actual, $3 = expected
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, expected $3)"
    sed 's/^/      /' /tmp/moved-symbols-test.out
  fi
}

echo "check-moved-symbols.sh fixture tests"

# 1. A column-0 pub fn leaves util.rs while a test still imports util::foo.
dir="$(make_repo)"
printf 'pub fn foo() {}\n' >"$dir/crates/demo/src/util.rs"
printf 'use demo::util::foo;\n#[test]\nfn t() { foo(); }\n' >"$dir/crates/demo/tests/foo_test.rs"
commit_all "$dir" base
printf '' >"$dir/crates/demo/src/util.rs"
printf 'pub fn bar() {}\npub fn foo() {}\n' >"$dir/crates/demo/src/helpers.rs"
commit_all "$dir" "move foo"
expect "moved fn with a stranded importer fails" "$(run_gate "$dir")" 1

# 2. The same move with the importer repointed passes.
dir="$(make_repo)"
printf 'pub fn foo() {}\n' >"$dir/crates/demo/src/util.rs"
printf 'use demo::util::foo;\n#[test]\nfn t() { foo(); }\n' >"$dir/crates/demo/tests/foo_test.rs"
commit_all "$dir" base
printf '' >"$dir/crates/demo/src/util.rs"
printf 'pub fn bar() {}\npub fn foo() {}\n' >"$dir/crates/demo/src/helpers.rs"
printf 'use demo::helpers::foo;\n#[test]\nfn t() { foo(); }\n' >"$dir/crates/demo/tests/foo_test.rs"
commit_all "$dir" "move foo and repoint"
expect "moved fn with the importer repointed passes" "$(run_gate "$dir")" 0

# 3. A re-export at the old path keeps the importer valid.
dir="$(make_repo)"
printf 'pub fn foo() {}\n' >"$dir/crates/demo/src/util.rs"
printf 'use demo::util::foo;\n#[test]\nfn t() { foo(); }\n' >"$dir/crates/demo/tests/foo_test.rs"
commit_all "$dir" base
printf 'pub use crate::helpers::foo;\n' >"$dir/crates/demo/src/util.rs"
printf 'pub fn bar() {}\npub fn foo() {}\n' >"$dir/crates/demo/src/helpers.rs"
commit_all "$dir" "move foo behind a re-export"
expect "moved fn re-exported at the old path passes" "$(run_gate "$dir")" 0

# 4. Deleting an indented method strands nobody, even when an importer of the
#    module names the same bare word (`Vec::new`).
dir="$(make_repo)"
printf 'pub struct Thing;\nimpl Thing {\n    pub fn new() -> Self {\n        Self\n    }\n}\n' >"$dir/crates/demo/src/util.rs"
printf 'use demo::util::Thing;\n#[test]\nfn t() { let _t = Thing; let _v: Vec<u8> = Vec::new(); }\n' >"$dir/crates/demo/tests/thing_test.rs"
commit_all "$dir" base
printf 'pub struct Thing;\n' >"$dir/crates/demo/src/util.rs"
commit_all "$dir" "drop the constructor"
expect "deleting an indented method is not a module move" "$(run_gate "$dir")" 0

# 5. A `pub const fn` is named by its identifier, not by `fn`, so stranding
#    it is caught even when the importer contains no `fn` at all.
dir="$(make_repo)"
printf 'pub const fn answer() -> u32 {\n    42\n}\n' >"$dir/crates/demo/src/util.rs"
printf 'use demo::util::answer;\npub const X: u32 = answer();\n' >"$dir/crates/demo/tests/answer_test.rs"
commit_all "$dir" base
printf '' >"$dir/crates/demo/src/util.rs"
printf 'pub fn bar() {}\npub const fn answer() -> u32 { 42 }\n' >"$dir/crates/demo/src/helpers.rs"
commit_all "$dir" "move answer"
expect "moved const fn with a stranded importer fails" "$(run_gate "$dir")" 1

# 6. Removing a pub use re-export strands its importer.
dir="$(make_repo)"
printf 'pub use crate::helpers::bar;\n' >"$dir/crates/demo/src/util.rs"
printf 'use demo::util::bar;\n#[test]\nfn t() { bar(); }\n' >"$dir/crates/demo/tests/bar_test.rs"
commit_all "$dir" base
printf '' >"$dir/crates/demo/src/util.rs"
commit_all "$dir" "drop the re-export"
expect "removed re-export with a stranded importer fails" "$(run_gate "$dir")" 1

# 7. A module whose leaf name is common (`coaches`, `types`, `user`) must not
#    match another crate's module of the same name. This is the shape that made
#    the gate cry wolf: a file importing `pierre_core::models::coaches::X` and
#    separately naming the moved item was read as a stranded importer.
dir="$(make_repo)"
mkdir -p "$dir/crates/demo/src/coaches" "$dir/crates/other/src/models"
printf 'pub mod coaches;\n' >"$dir/crates/demo/src/lib.rs"
printf 'pub fn resolve_locale() -> String { String::new() }\n' >"$dir/crates/demo/src/coaches/mod.rs"
printf 'pub mod models;\n' >"$dir/crates/other/src/lib.rs"
printf 'pub mod coaches;\n' >"$dir/crates/other/src/models/mod.rs"
printf 'pub struct Category;\n' >"$dir/crates/other/src/models/coaches.rs"
# The innocent bystander: it uses another crate's `coaches` module and happens
# to mention the moved name, because it now imports it from its new home.
printf 'use other::models::coaches::Category;\nuse shared::resolve_locale;\nfn f(_c: Category) { let _ = resolve_locale(); }\n' \
  >"$dir/crates/other/src/models/user.rs"
commit_all "$dir" base
printf '' >"$dir/crates/demo/src/coaches/mod.rs"
commit_all "$dir" "move resolve_locale out of demo::coaches"
expect "a same-named module in another crate is not a stranded importer" "$(run_gate "$dir")" 0

# 8. The owning crate's own `crate::<module>::` importers are still caught.
dir="$(make_repo)"
mkdir -p "$dir/crates/demo/src/coaches"
printf 'pub mod coaches;\npub mod caller;\n' >"$dir/crates/demo/src/lib.rs"
printf 'pub fn resolve_locale() -> String { String::new() }\n' >"$dir/crates/demo/src/coaches/mod.rs"
printf 'use crate::coaches::resolve_locale;\npub fn f() -> String { resolve_locale() }\n' \
  >"$dir/crates/demo/src/caller.rs"
commit_all "$dir" base
printf '' >"$dir/crates/demo/src/coaches/mod.rs"
commit_all "$dir" "move resolve_locale, stranding a crate:: importer"
expect "an intra-crate crate:: importer is still caught" "$(run_gate "$dir")" 1

echo ""
if [ "$failures" -ne 0 ]; then
  echo "❌ $failures moved-symbols fixture case(s) failed"
  exit 1
fi
echo "✅ all moved-symbols fixture cases passed"
