#!/usr/bin/env bash
# ABOUTME: Fixture test for cargo-sweep-nightly.sh discovery — builds throwaway scan
# ABOUTME: roots and asserts which build trees are found, labelled, and addressed.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A repository can hold more than one build tree: `target/` plus whatever a side
# build pointed CARGO_TARGET_DIR at. Discovery matched the literal name `target`,
# so an alternate tree was never swept, never aged out, and never counted toward
# the fleet cap — invisible disk that only grew. These cases pin that it is found,
# that it carries its own label, and that cargo-sweep is pointed at the tree that
# was actually discovered rather than the manifest default.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="$SCRIPT_DIR/cargo-sweep-nightly.sh"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

[[ -x "$UNDER_TEST" ]] || { echo "not executable: $UNDER_TEST" >&2; exit 1; }

# A build tree as cargo leaves one, without paying for a compile. Discovery keys
# off CACHEDIR.TAG (or .rustc_info.json) plus a manifest beside the tree, so that
# is the whole shape a fixture needs.
make_tree() { # $1 = repo dir, $2 = target dir name
  mkdir -p "$1/$2/debug"
  printf 'Signature: 8a477f597d28d172789f06886806bc55\n' > "$1/$2/CACHEDIR.TAG"
  head -c 200000 /dev/zero > "$1/$2/debug/blob.bin"
}

make_repo() { # $1 = scan root, $2 = repo name
  mkdir -p "$1/$2"
  printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2021"\n' "$2" > "$1/$2/Cargo.toml"
  printf '%s\n' "$1/$2"
}

new_root() { mktemp -d "${TMPDIR:-/tmp}/cargo-sweep-test.XXXXXX"; }

# Labels as `status` prints them, one per line. The table runs from the line
# after the header to the rule above the fleet totals, and is empty when nothing
# was discovered — so the extraction has to stop at the rule, not run into the
# FLEET/CAP/HEADROOM footer below it.
labels_of() { # $1 = scan root
  # awk must drain its input rather than exit at the rule: closing the pipe
  # early sends SIGPIPE upstream, and pipefail would turn that into a failure.
  "$UNDER_TEST" status --root "$1" 2>/dev/null | awk '
    NR == 1 { next }
    /^-----/ { done = 1 }
    !done && NF { print $1 }
  '
}

echo "cargo-sweep-nightly.sh — discovery"

# An alternate tree alone must be found: before, `-name target` matched nothing
# here and the whole repository reported as having no build output at all.
root="$(new_root)"; repo="$(make_repo "$root" "alt-only")"
make_tree "$repo" "target-featurecheck"
got="$(labels_of "$root")"
if [[ "$got" == "alt-only[target-featurecheck]" ]]; then
  pass "an alternate build tree is discovered and labelled by its directory"
else
  fail "an alternate build tree is discovered and labelled by its directory (got: ${got:-<none>})"
fi
rm -rf "$root"

# An agent worktree builds at <repo>/.claude/worktrees/<name>/target, depth 5 under
# the root. The default depth must reach it: at three, such trees went uncounted.
root="$(new_root)"; repo="$(make_repo "$root" "host")"
nested="$(make_repo "$repo/.claude/worktrees" "agent-x")"
make_tree "$nested" "target"
got="$(labels_of "$root")"
if [[ "$got" == "agent-x" ]]; then
  pass "an agent worktree's target/ under .claude/worktrees is discovered by default"
else
  fail "an agent worktree's target/ under .claude/worktrees is discovered by default (got: ${got:-<none>})"
fi
rm -rf "$root"

# The plain tree keeps its bare repository label, so protected lists and cap
# ledgers written against the old name still match.
root="$(new_root)"; repo="$(make_repo "$root" "plain-only")"
make_tree "$repo" "target"
got="$(labels_of "$root")"
if [[ "$got" == "plain-only" ]]; then
  pass "a plain target/ keeps the bare repository label"
else
  fail "a plain target/ keeps the bare repository label (got: ${got:-<none>})"
fi
rm -rf "$root"

# Two trees in one repository must not collapse into one entry: the label keys
# the protected list, the cap ledger and the wholesale-reclaim staging path.
root="$(new_root)"; repo="$(make_repo "$root" "both")"
make_tree "$repo" "target"
make_tree "$repo" "target-featurecheck"
got="$(labels_of "$root" | sort | tr '\n' ' ')"
if [[ "$got" == "both both[target-featurecheck] " ]]; then
  pass "two trees in one repository get distinct labels"
else
  fail "two trees in one repository get distinct labels (got: ${got:-<none>})"
fi
rm -rf "$root"

# Widening the glob must not hand a destructive tool a directory that merely
# starts with "target". No CACHEDIR.TAG means it is not build output.
root="$(new_root)"; repo="$(make_repo "$root" "decoy")"
mkdir -p "$repo/targets/fixtures"
head -c 200000 /dev/zero > "$repo/targets/fixtures/data.bin"
got="$(labels_of "$root")"
if [[ -z "$got" ]]; then
  pass "a targets/ directory that is not build output is left alone"
else
  fail "a targets/ directory that is not build output is left alone (got: $got)"
fi
rm -rf "$root"

# Build output with no manifest beside it stays out of the swept set: cargo-sweep
# resolves through a manifest and cannot act on it.
root="$(new_root)"
mkdir -p "$root/orphaned"
make_tree "$root/orphaned" "target-featurecheck"
got="$(labels_of "$root")"
report="$("$UNDER_TEST" status --root "$root" 2>&1 || true)"
if [[ -z "$got" ]] && printf '%s' "$report" | grep -q "^orphan .*orphaned:"; then
  pass "an alternate tree with no Cargo.toml is reported as orphan, not swept"
else
  fail "an alternate tree with no Cargo.toml is reported as orphan, not swept (got: ${got:-<none>})"
fi
rm -rf "$root"

echo "cargo-sweep-nightly.sh — sweep addressing"

# cargo-sweep resolves the build tree from the manifest, which finds `target/`
# and nothing else. Unless the discovered tree is named through CARGO_TARGET_DIR,
# an alternate tree is reported as swept while keeping every byte.
if ! command -v cargo >/dev/null 2>&1 || ! command -v cargo-sweep >/dev/null 2>&1; then
  fail "sweep addressing requires cargo and cargo-sweep on PATH (cargo install cargo-sweep)"
else
  root="$(new_root)"; repo="$(make_repo "$root" "addressed")"
  mkdir -p "$repo/src"
  echo 'fn main() {}' > "$repo/src/main.rs"
  ( cd "$repo" && CARGO_TARGET_DIR=target-featurecheck cargo build -q ) >/dev/null 2>&1
  out="$(CARGO_SWEEP_DAYS=0 "$UNDER_TEST" sweep --root "$root" --no-cap --dry-run 2>&1 || true)"
  if printf '%s' "$out" | grep -qF "$repo/target-featurecheck"; then
    pass "cargo-sweep is pointed at the discovered tree, not the manifest default"
  else
    fail "cargo-sweep is pointed at the discovered tree, not the manifest default"
  fi
  rm -rf "$root"
fi

echo "cargo-sweep-nightly.sh — disk guard (carnet#799)"

# A build tree whose last build was $2 hours ago, by the sentinel cargo rewrites.
age_tree() { # $1 = target dir, $2 = hours ago
  : > "$1/.rustc_info.json"
  touch -t "$(date -v-"$2"H +%Y%m%d%H%M.%S)" "$1/.rustc_info.json"
}

# A run that skips the slow age pass, so a fixture needs no buildable crate: a
# scheduled run whose age pass is fresh does only merged worktrees, cap and floor.
quick_sweep() { # $1 = scan root, rest = extra args
  local root="$1" state; shift
  state="$(mktemp -d "${TMPDIR:-/tmp}/cargo-sweep-state.XXXXXX")"
  : > "$state/last-age-sweep"
  CARGO_SWEEP_STATE_DIR="$state" "$UNDER_TEST" sweep --root "$root" --scheduled --dry-run "$@" 2>&1 || true
  rm -rf "$state"
}

hook_input() { printf '{"tool_name":"Bash","tool_input":{"command":"%s"}}' "$1"; }

root="$(new_root)"
rc=0; hook_input "cd crates && cargo test --test foo" | "$UNDER_TEST" guard --hard-floor 999TiB >/dev/null 2>&1 || rc=$?
[[ $rc -eq 2 ]] && pass "guard refuses a cargo build under the hard floor (exit 2)" \
  || fail "guard refuses a cargo build under the hard floor (exit $rc)"
rc=0; hook_input "ls -la" | "$UNDER_TEST" guard --hard-floor 999TiB >/dev/null 2>&1 || rc=$?
[[ $rc -eq 0 ]] && pass "guard lets a non-build command through under the floor" \
  || fail "guard lets a non-build command through under the floor (exit $rc)"
rc=0; hook_input "cargo build" | "$UNDER_TEST" guard --hard-floor 1KiB >/dev/null 2>&1 || rc=$?
[[ $rc -eq 0 ]] && pass "guard lets a build through above the floor" \
  || fail "guard lets a build through above the floor (exit $rc)"
rc=0; hook_input "cargo build" | CARGO_SWEEP_HARD_FLOOR=lots "$UNDER_TEST" guard >/dev/null 2>&1 || rc=$?
[[ $rc -eq 0 ]] && pass "guard fails open on a malformed setting, never exit 2" \
  || fail "guard fails open on a malformed setting, never exit 2 (exit $rc)"
rc=0; printf 'not json' | CARGO_SWEEP_SCAN_ROOT=/nonexistent "$UNDER_TEST" guard --hard-floor 999TiB >/dev/null 2>&1 || rc=$?
[[ $rc -eq 0 ]] && pass "guard fails open on unreadable input and a missing scan root" \
  || fail "guard fails open on unreadable input and a missing scan root (exit $rc)"
rc=0; hook_input "cargo build" | CARGO_SWEEP_MAX_DEPTH=0 "$UNDER_TEST" guard --hard-floor 1KiB >/dev/null 2>&1 || rc=$?
[[ $rc -eq 0 ]] && pass "guard fails open on an out-of-range setting, never exit 2" \
  || fail "guard fails open on an out-of-range setting, never exit 2 (exit $rc)"
rc=0; hook_input "cargo build" | CARGO_SWEEP_HARD_FLOOR=08GiB "$UNDER_TEST" guard >/dev/null 2>&1 || rc=$?
[[ $rc -eq 0 ]] && pass "guard reads a zero-padded size as decimal, not octal" \
  || fail "guard reads a zero-padded size as decimal, not octal (exit $rc)"
rc=0; hook_input "cargo build" | TMPDIR=/nonexistent "$UNDER_TEST" guard --hard-floor 999TiB >/dev/null 2>&1 || rc=$?
[[ $rc -eq 2 ]] && pass "guard still answers with no writable temp dir (a full disk)" \
  || fail "guard still answers with no writable temp dir (a full disk) (exit $rc)"

# Build shapes the guard must recognise: a miss lets a build fill the disk.
for cmd in "cargo --locked build" "cargo build;" "(cd x; cargo test)" "/usr/bin/cargo build" \
           "bash -c 'cargo test'" "cargo build|tee log" "echo x &&cargo check" "cargo +1.98.1 clippy" \
           "cargo t" "cargo nextest run" "cargo --manifest-path a/Cargo.toml build" \
           "./scripts/ci/pre-push-validate.sh" "./bin/start-server.sh" \
           "./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh"; do
  rc=0; hook_input "$cmd" | "$UNDER_TEST" guard --hard-floor 999TiB >/dev/null 2>&1 || rc=$?
  [[ $rc -eq 2 ]] && pass "guard refuses: $cmd" || fail "guard refuses: $cmd (exit $rc)"
done
# ...and the commands that free space or only read must never be refused.
for cmd in "scripts/setup/cargo-sweep-nightly.sh sweep" "cargo sweep --time 30" "cargo clean" \
           "cargo tree -d" "git commit -m fix" "./bin/stop-server.sh"; do
  rc=0; hook_input "$cmd" | "$UNDER_TEST" guard --hard-floor 999TiB >/dev/null 2>&1 || rc=$?
  [[ $rc -eq 0 ]] && pass "guard lets through: $cmd" || fail "guard lets through: $cmd (exit $rc)"
done

out="$("$UNDER_TEST" check --root "$root" --min-free 1KiB 2>&1 || true)"
[[ -z "$out" ]] && pass "check is silent above the floor" || fail "check is silent above the floor (got: $out)"
out="$("$UNDER_TEST" check --root "$root" --min-free 999TiB 2>&1 || true)"
printf '%s' "$out" | grep -q "DISK: .* under the .* floor" && pass "check warns under the floor" \
  || fail "check warns under the floor (got: ${out:-<none>})"
rm -rf "$root"

# Under the floor, whole trees go least-recently-built first, and a run stops
# once the projected free space would clear it.
root="$(new_root)"
old="$(make_repo "$root" "old")"; make_tree "$old" "target"; age_tree "$old/target" 48
new="$(make_repo "$root" "new")"; make_tree "$new" "target"; age_tree "$new/target" 1
out="$(quick_sweep "$root" --min-free 999TiB --keep 0)"
order="$(printf '%s\n' "$out" | sed -n 's/^floor: would reclaim \([a-z]*\) .*/\1/p' | tr '\n' ' ')"
[[ "$order" == "old new " ]] && pass "the floor reclaims least-recently-built first" \
  || fail "the floor reclaims least-recently-built first (got: ${order:-<none>})"
out="$(quick_sweep "$root" --min-free 999TiB --keep 1)"
printf '%s\n' "$out" | grep -q "^floor: keeping new " && pass "the floor keeps the --keep newest trees" \
  || fail "the floor keeps the --keep newest trees"
out="$(quick_sweep "$root" --min-free 1KiB)"
printf '%s\n' "$out" | grep -q "^floor: would reclaim" && fail "the floor reclaims nothing above it" \
  || pass "the floor reclaims nothing above it"
rm -rf "$root"

# A linked worktree whose content is on main gives up its build tree once it has
# sat unbuilt; one with unlanded work, or one built recently, keeps it.
root="$(new_root)"; repo="$(make_repo "$root" "host")"
(
  cd "$repo"
  git init -q -b main . && printf 'target*\n' > .gitignore
  git add -A && git -c user.email=t@t -c user.name=t commit -qm init
  git update-ref refs/remotes/origin/main HEAD
  git worktree add -q "$repo/.claude/worktrees/landed" -b landed
  git worktree add -q "$repo/.claude/worktrees/squashed" -b squashed
  git worktree add -q "$repo/.claude/worktrees/unlanded" -b unlanded
  git worktree add -q "$repo/.claude/worktrees/fresh" -b fresh
  # squashed: its commit reaches main as a different, squashed commit.
  ( cd "$repo/.claude/worktrees/squashed" && echo a > a.txt && git add a.txt && git -c user.email=t@t -c user.name=t commit -qm a )
  git checkout -q --detach && echo a > a.txt && git add a.txt && git -c user.email=t@t -c user.name=t commit -qm "squash of a"
  git update-ref refs/remotes/origin/main HEAD && git checkout -q main
  ( cd "$repo/.claude/worktrees/unlanded" && echo b > b.txt && git add b.txt && git -c user.email=t@t -c user.name=t commit -qm b )
  # dirty: on main, but holding an uncommitted file.
  git worktree add -q "$repo/.claude/worktrees/dirty" -b dirty
  echo c > "$repo/.claude/worktrees/dirty/c.txt"
) >/dev/null 2>&1
# The primary checkout sits on main too, and must never be a candidate.
make_tree "$repo" "target"; age_tree "$repo/target" 8
for wt in landed squashed unlanded fresh dirty; do
  make_tree "$repo/.claude/worktrees/$wt" "target"
done
for wt in landed squashed unlanded dirty; do age_tree "$repo/.claude/worktrees/$wt/target" 8; done
age_tree "$repo/.claude/worktrees/fresh/target" 1
out="$(quick_sweep "$root" --min-free 0)"
merged="$(printf '%s\n' "$out" | sed -n 's/^would   reclaim \([a-z]*\) .*already on main.*/\1/p' | sort | tr '\n' ' ')"
[[ "$merged" == "landed squashed " ]] \
  && pass "landed and squash-merged worktrees give up their build trees; unlanded, dirty, fresh and primary keep them" \
  || fail "landed and squash-merged worktrees give up their build trees; unlanded, dirty, fresh and primary keep them (got: ${merged:-<none>})"
rm -rf "$root"

echo ""
if [[ "$failures" -eq 0 ]]; then
  echo "✅ all cargo-sweep-nightly discovery cases passed"
else
  echo "❌ $failures case(s) failed"
fi
exit "$((failures > 0))"
