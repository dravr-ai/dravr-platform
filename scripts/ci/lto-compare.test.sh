#!/usr/bin/env bash
# ABOUTME: Fixture test for lto-compare.py — drives every subcommand against a stand-in cargo and checks the table
# ABOUTME: Pins the shipped invocation, the cold-build bookkeeping, the interleaving, the arithmetic and the fail-closed paths
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The real experiment is a workflow_dispatch run of several hours that can only
# start once the workflow is on main, so this test is the only verification the
# driver gets before it meets a runner. It builds a fake repository around the
# REAL .github/actions/server-binaries/action.yml (the driver must read the
# shipped invocation from it, not from a copy) and puts a stand-in `cargo` on
# PATH. The stand-in is a mock in test code: it answers the four invocations the
# driver makes (--unit-graph, build, bench --no-run, and the bench executables
# it emits) in the shapes cargo 1.98.1 and Criterion 0.5 produce, which were
# captured from a real throwaway crate. It does not compile anything.
#
# What it proves, in the order it runs:
#   - the feature set, RUSTFLAGS and binaries come from the action file, and the
#     build is `--locked --offline` with a per-profile CARGO_TARGET_DIR
#   - a profile that does not resolve, a profile on the wrong side, or an action
#     file that no longer carries the invocation stops `describe` at once
#   - `bench-build` builds with --profile and drops the target dir
#   - thin and fat alternate which goes first, per suite and per repetition
#   - the table carries the right numbers: paired ratio, spread, range, verdict,
#     geometric means, and a zero divisor prints n/a rather than dividing
#   - a failed bench run, a missing repetition, a one-sided benchmark and an
#     empty results tree each make `report` exit non-zero instead of printing a
#     table that looks complete
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
UNDER_TEST="$SCRIPT_DIR/lto-compare.py"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

for tool in python3 size; do
  command -v "$tool" >/dev/null 2>&1 || { echo "  ❌ $tool is required"; exit 1; }
done
python3 -c 'import yaml' 2>/dev/null || { echo "  ❌ PyYAML is required"; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/bin"

# --- the fake repository ------------------------------------------------------
# Only the action file is real. The manifest path the stand-in reports is where
# the bench executables must be run from.
make_repo() { # $1 = directory
  mkdir -p "$1/.github/actions/server-binaries" "$1/crates/pierre-server"
  cp "$REPO_ROOT/.github/actions/server-binaries/action.yml" "$1/.github/actions/server-binaries/action.yml"
  : >"$1/crates/pierre-server/Cargo.toml"
}
REPO="$WORK/repo"
make_repo "$REPO"

# --- the stand-in cargo -------------------------------------------------------
cat >"$WORK/bin/cargo" <<'PY'
#!/usr/bin/env python3
import json
import os
import pathlib
import sys

args = sys.argv[1:]
with pathlib.Path(os.environ["FAKE_CARGO_LOG"]).open("a") as fh:
    fh.write(json.dumps({
        "args": args,
        "target_dir": os.environ.get("CARGO_TARGET_DIR"),
        "rustflags": os.environ.get("RUSTFLAGS"),
        "incremental": os.environ.get("CARGO_INCREMENTAL"),
        "bootstrap": os.environ.get("RUSTC_BOOTSTRAP"),
    }) + "\n")


def option(name):
    return args[args.index(name) + 1] if name in args else None


def values(name):
    return [args[i + 1] for i, a in enumerate(args) if a == name]


# Profile name -> (lto, codegen-units), as the repository's Cargo.toml defines them.
PROFILES = {
    "release": ("thin", 16),
    "release-lto": ("fat", 1),
    "lto-compare-thin": ("thin", 1),
}
if args == ["-V"]:
    print("cargo 1.98.1 (stand-in)")
    sys.exit(0)
profile = option("--profile")
if profile not in PROFILES:
    sys.stderr.write(f"error: profile `{profile}` is not defined\n")
    sys.exit(101)
lto, cgu = PROFILES[profile]
manifest = os.environ["FAKE_MANIFEST"]
MIB = 1024 * 1024
# Binary sizes the report must reproduce exactly: fat is 20% / 25% smaller.
SIZES = {
    "thin": {"pierre-mcp-server": 10 * MIB, "pierre-cli": 4 * MIB},
    "fat": {"pierre-mcp-server": 8 * MIB, "pierre-cli": 3 * MIB},
}
# Per-benchmark fat / thin factor; a list is per repetition. Thin is always 1.0.
FACTORS = {
    ("serde_bench", "serialize/single"): 0.8,
    ("serde_bench", "serialize/batch_100"): 1.25,
    ("serde_bench", "slash/name"): 0.5,
    ("fitness_bench", "training_load/ctl_atl"): 1.02,
    ("fitness_bench", "training_load/tss"): 0.97,
    ("fitness_bench", "degenerate/zero"): 1.0,
    ("fitness_bench", "mixed/case"): [0.8, 0.8, 1.2],
}
BASE_NS = {"serialize/single": 1000.0, "serialize/batch_100": 100000.0, "slash/name": 250.0,
           "training_load/ctl_atl": 50000.0, "training_load/tss": 20000.0, "mixed/case": 7000.0,
           "degenerate/zero": 0.0}
side = "thin" if lto == "thin" else "fat"


def artifact(name, kind, executable, test=False):
    return json.dumps({
        "reason": "compiler-artifact",
        "package_id": f"path+file:///repo/crates/pierre-server#{name}@0.3.0",
        "manifest_path": manifest,
        "target": {"name": name, "kind": [kind]},
        "profile": {"opt_level": "3", "debuginfo": 0, "test": test},
        "executable": executable,
    })


if "--unit-graph" in args:
    if os.environ.get("RUSTC_BOOTSTRAP") != "1":
        sys.stderr.write("error: the `--unit-graph` flag is unstable\n")
        sys.exit(1)
    bins = values("--bin")
    units = [{"target": {"name": "serde", "kind": ["lib"]}, "mode": "build",
              "profile": {"lto": "off", "codegen_units": 16}}]
    units += [{
        "target": {"name": b, "kind": ["bin"]}, "mode": "build",
        "profile": {"name": profile, "opt_level": "3", "lto": lto, "codegen_units": cgu, "debuginfo": 0,
                    "debug_assertions": False, "overflow_checks": False, "incremental": False,
                    "panic": "unwind", "strip": {"resolved": {"Named": "symbols"}}},
    } for b in bins]
    print(json.dumps({"version": 1, "units": units, "roots": list(range(1, len(units)))}))
    sys.exit(0)

target = pathlib.Path(os.environ["CARGO_TARGET_DIR"]) / profile
if args[0] == "build":
    target.mkdir(parents=True, exist_ok=True)
    print(artifact("serde", "lib", None))
    for name in values("--bin"):
        exe = target / name
        exe.write_bytes(pathlib.Path("/usr/bin/true").read_bytes())
        with exe.open("r+b") as fh:
            fh.truncate(SIZES[side][name])
        exe.chmod(0o755)
        print("this line is not JSON and must be skipped")
        print(artifact(name, "bin", str(exe)))
    print(json.dumps({"reason": "build-finished", "success": True}))
elif args[0] == "bench" and "--no-run" in args:
    deps = target / "deps"
    deps.mkdir(parents=True, exist_ok=True)
    suites = values("--bench") or ["serde_bench", "fitness_bench"]
    for suite in suites:
        if suite == "nonexistent_bench":
            sys.stderr.write(f"error: no bench target named `{suite}`\n")
            sys.exit(101)
        if suite == "skipped_bench":
            continue  # cargo skips a target whose required-features are off, without an error
        exe = deps / f"{suite}-0123456789abcdef"
        benches = {b: f for (s, b), f in FACTORS.items() if s == suite}
        exe.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, pathlib, sys\n"
            f"SUITE, PROFILE, SIDE, BASE, BENCHES = {suite!r}, {profile!r}, {side!r}, {BASE_NS!r}, {benches!r}\n"
            + EXECUTABLE_BODY
        )
        exe.chmod(0o755)
        print(artifact(suite, "bench", str(exe), test=True))
    # The bin's own test harness: carries `test: true`, kind bin, and must be ignored.
    print(artifact("pierre-mcp-server", "bin", str(deps / "pierre_mcp_server-feedface"), test=True))
    print(json.dumps({"reason": "build-finished", "success": True}))
else:
    sys.stderr.write(f"stand-in cargo: unexpected invocation {args}\n")
    sys.exit(2)
PY

# The bench executable body, spliced into the stand-in above so the stand-in can
# stay one file. It writes what Criterion 0.5 writes (new/estimates.json and
# new/benchmark.json under CRITERION_HOME) and logs how it was run.
EXECUTABLE_BODY_FILE="$WORK/executable-body.py"
cat >"$EXECUTABLE_BODY_FILE" <<'PY'
if sys.argv[1:] != ["--bench", "--noplot"]:
    sys.stderr.write(f"unexpected arguments {sys.argv[1:]}\n")
    sys.exit(2)
home = pathlib.Path(os.environ["CRITERION_HOME"])
rep = int(home.parent.name.removeprefix("rep-"))
with open(os.environ["FAKE_ORDER_LOG"], "a") as fh:
    fh.write(f"{PROFILE} rep{rep} {SUITE} cwd={os.getcwd()}\n")
if os.environ.get("FAKE_FAIL") == f"{PROFILE}:{SUITE}":
    sys.exit(3)
drift = 1 + 0.1 * rep  # the runner slows down as the job goes on; both sides feel it
for bench, factor in BENCHES.items():
    if isinstance(factor, list):
        factor = factor[rep - 1]
    ns = BASE[bench] * drift * (factor if SIDE == "fat" else 1.0)
    # Criterion nests group/function and flattens a slash inside a single id.
    directory = home / ("slash_name" if bench == "slash/name" else bench)
    new = directory / "new"
    new.mkdir(parents=True)
    estimates = {k: {"point_estimate": ns, "standard_error": 1.0} for k in ("mean", "median", "median_abs_dev", "std_dev")}
    estimates["slope"] = None
    (new / "estimates.json").write_text(json.dumps(estimates))
    (new / "benchmark.json").write_text(json.dumps({"group_id": bench, "full_id": bench, "directory_name": bench}))
PY
python3 - "$WORK/bin/cargo" "$EXECUTABLE_BODY_FILE" <<'PY'
import sys
shim, body = sys.argv[1:]
text = open(shim).read().replace("EXECUTABLE_BODY", repr(open(body).read()))
open(shim, "w").write(text)
PY
chmod +x "$WORK/bin/cargo"

export PATH="$WORK/bin:$PATH"
export FAKE_CARGO_LOG="$WORK/cargo.log"
export FAKE_ORDER_LOG="$WORK/order.log"
export FAKE_MANIFEST="$REPO/crates/pierre-server/Cargo.toml"
export GITHUB_SHA="0123456789abcdef0123456789abcdef01234567"
unset GITHUB_STEP_SUMMARY GITHUB_RUN_ID

# --- helpers -----------------------------------------------------------------
lto() { python3 "$UNDER_TEST" --repo "$REPO" "$@"; }

jget() { # $1 = json file, $2 = python expression over `d`
  python3 -c 'import json,sys; d = json.load(open(sys.argv[1])); print(eval(sys.argv[2]))' "$1" "$2"
}

expect_eq() { # $1 = label, $2 = actual, $3 = wanted
  if [ "$2" = "$3" ]; then pass "$1"; else fail "$1 (got '$2', wanted '$3')"; fi
}

expect_row() { # $1 = label, $2 = summary file, $3 = exact table row
  if grep -qxF -- "$3" "$2"; then pass "$1"; else
    fail "$1 (row not found: $3)"; grep -F -- "${3%% |*}" "$2" | sed 's/^/       found: /' || true
  fi
}

expect_fails() { # $1 = label, $2 = needle in the combined output, rest = command
  local label="$1" needle="$2" output code=0
  shift 2
  output="$("$@" 2>&1)" || code=$?
  if [ "$code" -ne 0 ] && grep -qF -- "$needle" <<<"$output"; then pass "$label"; else
    fail "$label (exit $code, wanted non-zero mentioning '$needle')"; sed 's/^/       /' <<<"$output" | tail -8
  fi
}

# --- describe: the shipped invocation and the two profiles ----------------------
echo "describe"
OUT="$WORK/out"
lto --out "$OUT" describe --thin-profile lto-compare-thin --fat-profile release-lto >/dev/null
expect_eq "features are read from the server-binaries action" \
  "$(jget "$OUT/meta.json" "d['shipped']['features']")" \
  "$(python3 -c 'import re,sys; print(re.search(r"--features \"([^\"]+)\"", open(sys.argv[1]).read()).group(1))' "$REPO/.github/actions/server-binaries/action.yml")"
expect_eq "the shipped binaries are the action's defaults" "$(jget "$OUT/meta.json" "' '.join(d['shipped']['bins'])")" "pierre-mcp-server pierre-cli"
expect_eq "RUSTFLAGS comes from the action" "$(jget "$OUT/meta.json" "d['shipped']['rustflags']")" "-C opt-level=3"
expect_eq "the effective profiles are read back from cargo" \
  "$(jget "$OUT/meta.json" "d['profiles']['thin']['effective']['lto'] + '/' + d['profiles']['fat']['effective']['lto']")" "thin/fat"
expect_eq "the default pair differs in lto alone" "$(jget "$OUT/meta.json" "d['profiles_differ_in']")" "['lto']"
bootstrap_only_for_unit_graph() { # prints True when RUSTC_BOOTSTRAP=1 appears on the unit-graph calls and nowhere else
  python3 -c 'import json,sys; print(all((json.loads(l)["bootstrap"] == "1") == ("--unit-graph" in json.loads(l)["args"]) for l in open(sys.argv[1])))' "$FAKE_CARGO_LOG"
}
expect_eq "RUSTC_BOOTSTRAP is set for the unit-graph query and for nothing else" "$(bootstrap_only_for_unit_graph)" "True"
expect_eq "the toolchain version is recorded" "$(jget "$OUT/meta.json" "d['toolchain']['cargo']")" "cargo 1.98.1 (stand-in)"

OUT_MIXED="$WORK/out-mixed"
lto --out "$OUT_MIXED" describe --thin-profile release --fat-profile release-lto >/dev/null 2>&1
expect_eq "a pair that differs in more than lto (release against release-lto) says so" "$(jget "$OUT_MIXED/meta.json" "d['profiles_differ_in']")" "['lto', 'codegen_units']"

expect_fails "a profile on the wrong side is refused" "is the thin side" \
  lto --out "$WORK/out-wrong" describe --thin-profile release-lto --fat-profile lto-compare-thin
expect_fails "a profile cargo cannot resolve is refused" "could not resolve profile" \
  lto --out "$WORK/out-unknown" describe --thin-profile no-such-profile --fat-profile release-lto

REPO_DRIFT="$WORK/repo-drift"
make_repo "$REPO_DRIFT"
sed -i 's/--no-default-features //' "$REPO_DRIFT/.github/actions/server-binaries/action.yml"
expect_fails "an action file that dropped --no-default-features is refused" "no longer passes --no-default-features" \
  python3 "$UNDER_TEST" --repo "$REPO_DRIFT" --out "$WORK/out-drift" describe --thin-profile lto-compare-thin --fat-profile release-lto

# --- build, bench-build, bench ---------------------------------------------------
echo "build"
: >"$FAKE_CARGO_LOG"
for label in thin fat; do
  lto --out "$OUT" build --label "$label" >/dev/null
done
expect_eq "thin server binary size is stat -c %s" "$(jget "$OUT/build/thin.json" "d['binaries']['pierre-mcp-server']['bytes']")" "$((10 * 1024 * 1024))"
expect_eq "fat server binary size is stat -c %s" "$(jget "$OUT/build/fat.json" "d['binaries']['pierre-mcp-server']['bytes']")" "$((8 * 1024 * 1024))"
expect_eq "pierre-cli is measured too" "$(jget "$OUT/build/fat.json" "d['binaries']['pierre-cli']['bytes']")" "$((3 * 1024 * 1024))"
expect_eq "size -A yields a .text section" "$(jget "$OUT/build/thin.json" "d['binaries']['pierre-mcp-server']['text_section'] > 0 and '.text' in d['binaries']['pierre-mcp-server']['sections']")" "True"
expect_eq "the Berkeley text figure is recorded" "$(jget "$OUT/build/thin.json" "d['binaries']['pierre-mcp-server']['text'] > 0")" "True"
expect_eq "a build is never run with RUSTC_BOOTSTRAP" "$(bootstrap_only_for_unit_graph)" "True"
build_call="$(python3 -c 'import json,sys; print(" ".join(json.loads(open(sys.argv[1]).readline())["args"]))' "$FAKE_CARGO_LOG")"
expect_eq "the thin build is the image invocation under the thin profile" "$build_call" \
  "build --locked --offline --profile lto-compare-thin --no-default-features --features $(jget "$OUT/meta.json" "d['shipped']['features']") --bin pierre-mcp-server --bin pierre-cli --message-format=json-render-diagnostics"
expect_eq "each profile builds into its own target dir" \
  "$(python3 -c 'import json,sys; print([json.loads(l)["target_dir"] for l in open(sys.argv[1])] == ["'"$OUT"'/target/thin", "'"$OUT"'/target/fat"])' "$FAKE_CARGO_LOG")" "True"
expect_eq "the build runs with the action's RUSTFLAGS and CARGO_INCREMENTAL" \
  "$(python3 -c 'import json,sys; print({(json.loads(l)["rustflags"], json.loads(l)["incremental"]) for l in open(sys.argv[1])})' "$FAKE_CARGO_LOG")" "{('-C opt-level=3', '0')}"

echo "bench-build"
: >"$FAKE_CARGO_LOG"
for label in thin fat; do
  lto --out "$OUT" bench-build --label "$label" >/dev/null
done
expect_eq "a bench build is never run with RUSTC_BOOTSTRAP" "$(bootstrap_only_for_unit_graph)" "True"
bench_call="$(python3 -c 'import json,sys; print(" ".join(json.loads(open(sys.argv[1]).readline())["args"]))' "$FAKE_CARGO_LOG")"
expect_eq "benches are built with --profile, in the server package, over the shipped features" "$bench_call" \
  "bench --locked --offline -p pierre_mcp_server --profile lto-compare-thin --no-default-features --features $(jget "$OUT/meta.json" "d['shipped']['features']") --benches --no-run --message-format=json-render-diagnostics"
expect_eq "both suites are kept for each profile" "$(ls "$OUT/bench-bin/thin" | tr '\n' ' ')|$(ls "$OUT/bench-bin/fat" | tr '\n' ' ')" "fitness_bench serde_bench |fitness_bench serde_bench "
expect_eq "the bin's own test harness is not mistaken for a suite" "$(jget "$OUT/build/thin.json" "d['bench_build']['suites']")" "['fitness_bench', 'serde_bench']"
if [ -e "$OUT/target/thin" ] || [ -e "$OUT/target/fat" ]; then fail "target dirs were left on disk"; else pass "the target dirs are removed once the executables are kept"; fi
expect_eq "the bench working directory is the package's manifest dir" "$(jget "$OUT/build/thin.json" "d['bench_build']['cwd']")" "$REPO/crates/pierre-server"
lto --out "$WORK/out-subset" describe --thin-profile lto-compare-thin --fat-profile release-lto >/dev/null
lto --out "$WORK/out-subset" build --label thin >/dev/null
mkdir -p "$WORK/out-subset/target/thin/stale" && : >"$WORK/out-subset/target/thin/stale/leftover"
lto --out "$WORK/out-subset" build --label thin >/dev/null
if [ -e "$WORK/out-subset/target/thin/stale/leftover" ]; then fail "a leftover file survived the cold build"; else pass "a second build starts from an empty target dir"; fi
lto --out "$WORK/out-subset" bench-build --label thin --suites serde_bench >/dev/null
expect_eq "a suite selection builds exactly those suites" "$(ls "$WORK/out-subset/bench-bin/thin")" "serde_bench"
expect_fails "a bench target cargo rejects fails the build step" "exited 101" \
  lto --out "$WORK/out-subset" bench-build --label thin --suites "serde_bench nonexistent_bench"
expect_fails "a suite cargo skipped without an error is refused" "asked for benches" \
  lto --out "$WORK/out-subset" bench-build --label thin --suites "serde_bench skipped_bench"

echo "bench"
: >"$FAKE_ORDER_LOG"
lto --out "$OUT" bench --reps 3 >/dev/null
expect_eq "twelve runs: two suites, two profiles, three repetitions" "$(wc -l <"$FAKE_ORDER_LOG" | tr -d ' ')" "12"
expect_eq "thin and fat alternate which goes first, per suite and per repetition" \
  "$(head -8 "$FAKE_ORDER_LOG" | awk '{print $1 "/" $2 "/" $3}' | tr '\n' ' ')" \
  "lto-compare-thin/rep1/fitness_bench release-lto/rep1/fitness_bench release-lto/rep1/serde_bench lto-compare-thin/rep1/serde_bench release-lto/rep2/fitness_bench lto-compare-thin/rep2/fitness_bench lto-compare-thin/rep2/serde_bench release-lto/rep2/serde_bench "
expect_eq "benches run from the manifest dir" "$(awk '{print $4}' "$FAKE_ORDER_LOG" | sort -u)" "cwd=$REPO/crates/pierre-server"
expect_eq "each run has its own CRITERION_HOME" "$(find "$OUT/criterion" -name estimates.json | wc -l | tr -d ' ')" "42"

# --- report -------------------------------------------------------------------
echo "report"
# Real timings are milliseconds here, so pin the ones the table does arithmetic on.
python3 - "$OUT/build/thin.json" "$OUT/build/fat.json" <<'PY'
import json, sys
for path, wall, cpu in ((sys.argv[1], 1500.0, 0.0), (sys.argv[2], 2220.0, 5.0)):
    d = json.load(open(path))
    d["build"]["wall_seconds"], d["build"]["cpu_seconds"] = wall, cpu
    json.dump(d, open(path, "w"))
PY
SUMMARY="$WORK/summary.md"
: >"$SUMMARY"
lto --out "$OUT" report --summary-file "$SUMMARY" >/dev/null
expect_row "fat faster, every repetition, with the paired ratio" "$SUMMARY" \
  '| `serde_bench` `serialize/single` | 1.20 µs | 960.0 ns | -20.0% | -20.0% to -20.0% | 16.7% / 16.7% | fat faster |'
expect_row "thin faster is named too" "$SUMMARY" \
  '| `serde_bench` `serialize/batch_100` | 120.00 µs | 150.00 µs | +25.0% | +25.0% to +25.0% | 16.7% / 16.7% | thin faster |'
expect_row "a benchmark id with a slash reads from benchmark.json" "$SUMMARY" \
  '| `serde_bench` `slash/name` | 300.0 ns | 150.0 ns | -50.0% | -50.0% to -50.0% | 16.7% / 16.7% | fat faster |'
expect_row "a difference under the material threshold is not named" "$SUMMARY" \
  '| `fitness_bench` `training_load/ctl_atl` | 60.00 µs | 61.20 µs | +2.0% | +2.0% to +2.0% | 16.7% / 16.7% | no clear difference |'
expect_row "a consistent difference under the material threshold is not named either" "$SUMMARY" \
  '| `fitness_bench` `training_load/tss` | 24.00 µs | 23.28 µs | -3.0% | -3.0% to -3.0% | 16.7% / 16.7% | no clear difference |'
expect_row "a benchmark with a zero thin median has no ratio and says so" "$SUMMARY" \
  '| `fitness_bench` `degenerate/zero` | 0.0 ns | 0.0 ns | n/a | n/a | 0.0% / 0.0% | no data |'
expect_row "repetitions that disagree in sign are not named however large the median, and the range shows it" "$SUMMARY" \
  '| `fitness_bench` `mixed/case` | 8.40 µs | 6.72 µs | -20.0% | -20.0% to +20.0% | 16.7% / 70.8% | no clear difference |'
expect_row "the build section says which profile was built first" "$SUMMARY" \
  '| **Build and size** (one cold build per profile, thin first) |  |  |  |  |  |  |'
expect_row "the server binary size and its percentage" "$SUMMARY" \
  '| `pierre-mcp-server` size (`stat -c %s`) | 10.0 MiB | 8.0 MiB | -20.0% | one build | - | - |'
expect_row "pierre-cli is in the table" "$SUMMARY" \
  '| `pierre-cli` size (`stat -c %s`) | 4.0 MiB | 3.0 MiB | -25.0% | one build | - | - |'
expect_row "a clean build time and its percentage" "$SUMMARY" \
  '| Clean build, wall clock | 25m 00s | 37m 00s | +48.0% | one build | - | - |'
expect_row "a zero thin figure prints n/a instead of dividing" "$SUMMARY" \
  '| Clean build, CPU time (user+sys) | 0m 00s | 0m 05s | n/a | one build | - | - |'
for needle in \
  '**2** fat faster, **1** thin faster, **3** no clear difference, **1** no data of 7 benchmarks.' \
  'Geometric mean difference, all benchmarks: **-14.3%**.' \
  '`serde_bench`: -20.6% over 3 benchmarks.' \
  '`fitness_bench`: -7.5% over 4 benchmarks.' \
  'The two profiles differ in `lto` and nothing else.' \
  '- **fat**: `release-lto`: lto=fat, codegen-units=1, opt-level=3, panic=unwind, strip=symbols'; do
  if grep -qF -- "$needle" "$SUMMARY"; then pass "summary says: $needle"; else fail "summary lacks: $needle"; fi
done
expect_eq "the JSON artifact carries every benchmark" "$(jget "$OUT/lto-compare.json" "len(d['benchmarks'])")" "7"
expect_eq "the JSON artifact keeps the raw ratios" "$(jget "$OUT/lto-compare.json" "[round(r, 6) for b in d['benchmarks'] if b['id'] == 'mixed/case' for r in b['ratios']]")" "[0.8, 0.8, 1.2]"
expect_eq "a clean run reports no problems" "$(jget "$OUT/lto-compare.json" "d['problems']")" "[]"

echo "sdk state"
expect_eq "the SDK marker state is recorded at the start of each build" "$(jget "$OUT/build/thin.json" "d['sdk_dist_present_at_start']")/$(jget "$OUT/build/fat.json" "d['sdk_dist_present_at_start']")" "False/False"
OUT_SDK="$WORK/out-sdk"
cp -r "$OUT" "$OUT_SDK"
python3 - "$OUT_SDK/build/fat.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
d["sdk_dist_present_at_start"] = True  # the first build ran the SDK build, the second found its output
json.dump(d, open(sys.argv[1], "w"))
PY
expect_fails "builds that started with different SDK state are not comparable" "existed at the start of one cold build and not the other" \
  lto --out "$OUT_SDK" report --summary-file "$WORK/summary-sdk.md"
mkdir -p "$REPO/sdk/dist" && : >"$REPO/sdk/dist/cli.js"
lto --out "$WORK/out-sdk-present" describe --thin-profile lto-compare-thin --fat-profile release-lto >/dev/null
lto --out "$WORK/out-sdk-present" build --label thin >/dev/null
expect_eq "a present SDK marker is recorded as such" "$(jget "$WORK/out-sdk-present/build/thin.json" "d['sdk_dist_present_at_start']")" "True"
rm -r "$REPO/sdk"

echo "build order"
OUT_ORDER="$WORK/out-order"
cp -r "$OUT" "$OUT_ORDER"
python3 - "$OUT_ORDER/build/thin.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
d["started_at"] += 10000  # thin now started after fat
json.dump(d, open(sys.argv[1], "w"))
PY
lto --out "$OUT_ORDER" report --summary-file "$WORK/summary-order.md" >/dev/null
expect_row "a fat-first run is labelled fat first" "$WORK/summary-order.md" \
  '| **Build and size** (one cold build per profile, fat first) |  |  |  |  |  |  |'

# --- failures must not read as results -------------------------------------------
echo "failing runs"
OUT_FAIL="$WORK/out-fail"
cp -r "$OUT" "$OUT_FAIL"
rm -rf "$OUT_FAIL/criterion" "$OUT_FAIL/bench-failures.txt" "$OUT_FAIL/bench.json"
FAKE_FAIL="release-lto:serde_bench" expect_fails "a bench run that exits non-zero fails the bench step" "bench runs failed" \
  lto --out "$OUT_FAIL" bench --reps 2
expect_eq "the failure names the profile, repetition and suite" "$(head -1 "$OUT_FAIL/bench-failures.txt" | cut -d' ' -f1-4)" "fat rep 1 serde_bench"
expect_eq "the remaining runs still happened" "$(find "$OUT_FAIL/criterion/thin" -name estimates.json | wc -l | tr -d ' ')" "14"
SUMMARY_FAIL="$WORK/summary-fail.md"
expect_fails "report refuses a run with a failed bench" "problem(s) in the results" \
  lto --out "$OUT_FAIL" report --summary-file "$SUMMARY_FAIL"
if grep -qF 'bench run failed: fat rep 1 serde_bench' "$SUMMARY_FAIL"; then pass "the failed run is in the summary's Problems"; else fail "the summary does not list the failed run"; fi

OUT_GAP="$WORK/out-gap"
cp -r "$OUT" "$OUT_GAP"
rm -rf "$OUT_GAP/criterion/fat/rep-2/serde_bench"
expect_fails "a repetition missing on one side is a problem" "has 3 thin and 2 fat repetitions" \
  lto --out "$OUT_GAP" report --summary-file "$WORK/summary-gap.md"

OUT_ONE_SIDED="$WORK/out-one-sided"
cp -r "$OUT" "$OUT_ONE_SIDED"
rm -rf "$OUT_ONE_SIDED/criterion/fat"
expect_fails "a benchmark measured for one profile only is a problem" "was measured for thin only" \
  lto --out "$OUT_ONE_SIDED" report --summary-file "$WORK/summary-one-sided.md"

OUT_EMPTY="$WORK/out-empty"
mkdir -p "$OUT_EMPTY" && cp "$OUT/meta.json" "$OUT_EMPTY/"
expect_fails "a results tree with nothing in it is not a pass" "no Criterion estimates were found" \
  lto --out "$OUT_EMPTY" report --summary-file "$WORK/summary-empty.md"

if [ "$failures" -ne 0 ]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "all lto-compare checks passed"
