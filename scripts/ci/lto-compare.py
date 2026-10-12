#!/usr/bin/env python3
# ABOUTME: Drives the thin-vs-fat LTO experiment of carnet#499: cold builds, binary sizes, interleaved Criterion runs
# ABOUTME: One subcommand per workflow step; every number lands under --out and `report` turns it into the table and JSON
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

"""Drives the thin-vs-fat LTO experiment of carnet#499: cold builds, binary sizes, interleaved Criterion runs.

The question is whether the image should ship fat LTO (`release-lto`) or go back
to thin, and it has to be answered on one commit, one runner and one job, with
only the LTO mode changing. The steps, in the order the workflow runs them:

  describe     the runner, the toolchain, the shipped invocation, and the effective
               settings of the two profiles (read back from cargo, not assumed)
  build        wipe the label's target dir, build the shipped binaries from empty,
               record wall clock, CPU time, peak RSS, and each binary's size
  bench-build  build the Criterion benches under the SAME profile (`cargo bench
               --profile`, because the default `bench` profile measures neither),
               keep the executables, then drop the target dir to free disk
  bench        run every bench executable of both profiles, several repetitions,
               thin and fat back to back per suite with the order alternating, so
               runner drift lands on both sides instead of on one
  report       one table (build, size, every benchmark, thin vs fat, % difference)
               to $GITHUB_STEP_SUMMARY plus lto-compare.json

The shipped feature set, RUSTFLAGS and binary list are read from
.github/actions/server-binaries/action.yml, the action the image build and the
size gate both call, so the experiment follows the invocation instead of keeping
a third copy of it.

Everything that cannot be trusted to have run is a failure: a bench that exits
non-zero is recorded and fails the step after the remaining runs finish, and
`report` exits non-zero on any missing, unequal or unparsable piece instead of
printing a table that looks complete.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import platform
import re
import resource
import shutil
import statistics
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

import yaml

ACTION_FILE = Path(".github/actions/server-binaries/action.yml")
# The package that owns every Criterion bench (benchmarks.yml scopes to it too).
BENCH_PACKAGE = "pierre_mcp_server"
# The binary the release-binary size gate measures.
GATED_BIN = "pierre-mcp-server"
LABELS = ("thin", "fat")
# What `lto` reads back as in cargo's unit graph for each role.
LTO_VALUES = {"thin": {"thin"}, "fat": {"fat", "true"}}
# Profile keys compared between the two sides, and shown in the report header.
PROFILE_KEYS = (
    "lto",
    "codegen_units",
    "opt_level",
    "panic",
    "strip",
    "debuginfo",
    "debug_assertions",
    "overflow_checks",
    "incremental",
)
DEFAULT_BENCH_TIMEOUT_SECONDS = 1800
DEFAULT_MATERIAL_PCT = 5.0


class Fail(Exception):
    """A condition the experiment cannot continue past or report as success."""


# ---------------------------------------------------------------------------
# Paths and small helpers
# ---------------------------------------------------------------------------


def out_dir(args: argparse.Namespace) -> Path:
    path = Path(args.out)
    path.mkdir(parents=True, exist_ok=True)
    return path


def read_json(path: Path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise Fail(f"{path} is missing") from exc
    except json.JSONDecodeError as exc:
        raise Fail(f"{path} is not valid JSON: {exc}") from exc


def write_json(path: Path, data) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def target_dir(out: Path, label: str) -> Path:
    return out / "target" / label


def dir_bytes(path: Path) -> int:
    return sum(p.stat().st_size for p in path.rglob("*") if p.is_file() and not p.is_symlink())


@dataclass(frozen=True)
class Timed:
    wall_seconds: float
    cpu_seconds: float
    peak_rss_mib: float
    returncode: int


def run_timed(cmd: list[str], env: dict[str, str], cwd: Path, stdout=None, stderr=None, timeout=None) -> Timed:
    """Run `cmd` and report wall clock, CPU time (user+sys over all descendants) and peak RSS.

    Each subcommand is its own process, so RUSAGE_CHILDREN starts at zero and
    ru_maxrss is the largest single process the step ran (rustc or the linker).
    """
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    started = time.monotonic()
    try:
        proc = subprocess.run(cmd, env=env, cwd=cwd, stdout=stdout, stderr=stderr, timeout=timeout, check=False)
        returncode = proc.returncode
    except subprocess.TimeoutExpired:
        returncode = 124
    wall = time.monotonic() - started
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    cpu = (after.ru_utime - before.ru_utime) + (after.ru_stime - before.ru_stime)
    # ru_maxrss is KiB on Linux.
    return Timed(wall, cpu, after.ru_maxrss / 1024, returncode)


def cargo_artifacts(messages_path: Path, kind: str) -> dict[str, Path]:
    """Executables of `kind` ("bin" or "bench") from cargo's JSON messages, by target name.

    Test-harness builds of a bin carry `profile.test == true`; only the real
    artifact counts for the "bin" kind.
    """
    found: dict[str, Path] = {}
    for line in messages_path.read_text(encoding="utf-8").splitlines():
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("reason") != "compiler-artifact" or not msg.get("executable"):
            continue
        target = msg["target"]
        if kind not in target["kind"]:
            continue
        if kind == "bin" and msg["profile"].get("test"):
            continue
        found[target["name"]] = Path(msg["executable"])
    return found


def manifest_dir_of(messages_path: Path, target_name: str) -> Path:
    for line in messages_path.read_text(encoding="utf-8").splitlines():
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("reason") == "compiler-artifact" and msg.get("target", {}).get("name") == target_name:
            return Path(msg["manifest_path"]).parent
    raise Fail(f"no compiler-artifact message for {target_name} in {messages_path}")


# ---------------------------------------------------------------------------
# The shipped invocation
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Shipped:
    features: str
    rustflags: str
    incremental: str
    bins: tuple[str, ...]
    profile: str


def read_shipped(repo: Path) -> Shipped:
    """The feature set, env and binaries `.github/actions/server-binaries` builds."""
    action = repo / ACTION_FILE
    try:
        doc = yaml.safe_load(action.read_text(encoding="utf-8"))
    except (OSError, yaml.YAMLError) as exc:
        raise Fail(f"cannot read the shipped invocation from {action}: {exc}") from exc
    builds = [s for s in doc["runs"]["steps"] if "cargo build" in str(s.get("run", ""))]
    if len(builds) != 1:
        raise Fail(f"{action} must hold exactly one `cargo build` step, found {len(builds)}")
    step = builds[0]
    run = step["run"]
    features = re.findall(r'--features "([^"]+)"', run)
    profile = re.findall(r"--profile (\S+)", run)
    if len(features) != 1 or len(profile) != 1:
        raise Fail(f"{action}: expected one --features and one --profile in the cargo build step")
    if "--no-default-features" not in run:
        raise Fail(f"{action}: the shipped build no longer passes --no-default-features; update this script")
    env = step.get("env") or {}
    for key in ("RUSTFLAGS", "CARGO_INCREMENTAL"):
        if key not in env:
            raise Fail(f"{action}: the cargo build step no longer sets {key}; update this script")
    bins = tuple(str(doc["inputs"]["bins"]["default"]).split())
    if GATED_BIN not in bins:
        raise Fail(f"{action}: default bins {bins} do not include {GATED_BIN}")
    return Shipped(features[0], str(env["RUSTFLAGS"]), str(env["CARGO_INCREMENTAL"]), bins, profile[0])


def bin_args(shipped: Shipped) -> list[str]:
    return [arg for name in shipped.bins for arg in ("--bin", name)]


def cargo_env(shipped: Shipped, target: Path | None) -> dict[str, str]:
    env = dict(os.environ)
    env["RUSTFLAGS"] = shipped.rustflags
    env["CARGO_INCREMENTAL"] = shipped.incremental
    env.setdefault("COPILOT_SKIP_CLI_DOWNLOAD", "1")
    if target is not None:
        env["CARGO_TARGET_DIR"] = str(target)
    return env


def shipped_flags(shipped: Shipped) -> list[str]:
    return ["--no-default-features", "--features", shipped.features]


def effective_profile(repo: Path, shipped: Shipped, profile: str) -> dict:
    """The settings cargo will actually compile `profile` with, for the shipped binaries.

    The unit graph is the only structured view of a resolved profile. It sits
    behind -Z unstable-options, so RUSTC_BOOTSTRAP=1 is set for this one query
    and never for a build. Reading it back means a typo in a profile name or an
    `inherits` chain that changes something else fails here, in seconds, not
    after two hours of compiling the wrong thing.
    """
    env = cargo_env(shipped, None)
    env["RUSTC_BOOTSTRAP"] = "1"
    cmd = [
        "cargo", "build", "--locked", "--offline", "--profile", profile,
        *shipped_flags(shipped), *bin_args(shipped),
        "--unit-graph", "-Z", "unstable-options",
    ]
    proc = subprocess.run(cmd, env=env, cwd=repo, capture_output=True, text=True, check=False)
    if proc.returncode != 0:
        raise Fail(f"cargo could not resolve profile `{profile}`:\n{proc.stderr.strip()[-2000:]}")
    graph = json.loads(proc.stdout)
    roots = [graph["units"][i] for i in graph["roots"]]
    gated = [u for u in roots if u["target"]["name"] == GATED_BIN]
    if len(gated) != 1:
        raise Fail(f"unit graph for `{profile}` has {len(gated)} root units named {GATED_BIN}, expected 1")
    resolved = gated[0]["profile"]
    return {key: resolved.get(key) for key in PROFILE_KEYS}


# ---------------------------------------------------------------------------
# describe
# ---------------------------------------------------------------------------


def runner_description() -> dict:
    cpu_model = ""
    try:
        for line in Path("/proc/cpuinfo").read_text(encoding="utf-8").splitlines():
            if line.startswith("model name"):
                cpu_model = line.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    mem_gib = None
    try:
        for line in Path("/proc/meminfo").read_text(encoding="utf-8").splitlines():
            if line.startswith("MemTotal"):
                mem_gib = round(int(line.split()[1]) / 1024 / 1024, 1)
                break
    except OSError:
        pass
    return {
        "cpus": os.cpu_count(),
        "cpu_model": cpu_model,
        "mem_gib": mem_gib,
        "platform": platform.platform(),
        # The hosted runner sets this variable under exactly this spelling.
        "image_version": os.environ.get("ImageVersion", ""),
    }


def tool_version(cmd: list[str]) -> str:
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
    except OSError:
        return ""
    return proc.stdout.strip() if proc.returncode == 0 else ""


def cmd_describe(args: argparse.Namespace) -> None:
    out = out_dir(args)
    repo = Path(args.repo).resolve()
    shipped = read_shipped(repo)
    profiles = {}
    for label, name in (("thin", args.thin_profile), ("fat", args.fat_profile)):
        effective = effective_profile(repo, shipped, name)
        if str(effective["lto"]) not in LTO_VALUES[label]:
            raise Fail(f"profile `{name}` is the {label} side but resolves to lto={effective['lto']!r}")
        profiles[label] = {"name": name, "effective": effective}
    differ = [k for k in PROFILE_KEYS if profiles["thin"]["effective"][k] != profiles["fat"]["effective"][k]]
    meta = {
        "commit": os.environ.get("GITHUB_SHA") or tool_version(["git", "-C", str(repo), "rev-parse", "HEAD"]),
        "run_url": run_url(),
        "runner": runner_description(),
        "toolchain": {"rustc": tool_version(["rustc", "-V"]), "cargo": tool_version(["cargo", "-V"])},
        "shipped": {
            "features": shipped.features,
            "no_default_features": True,
            "rustflags": shipped.rustflags,
            "cargo_incremental": shipped.incremental,
            "bins": list(shipped.bins),
            "profile": shipped.profile,
        },
        "profiles": profiles,
        "profiles_differ_in": differ,
    }
    write_json(out / "meta.json", meta)
    print(json.dumps(meta, indent=2, sort_keys=True))
    if differ != ["lto"]:
        print(f"::warning::the two profiles differ in {differ}, not in lto alone")


def run_url() -> str:
    server, repo, run = (os.environ.get(k) for k in ("GITHUB_SERVER_URL", "GITHUB_REPOSITORY", "GITHUB_RUN_ID"))
    return f"{server}/{repo}/actions/runs/{run}" if server and repo and run else ""


def load_meta(out: Path) -> dict:
    return read_json(out / "meta.json")


def shipped_from_meta(meta: dict) -> Shipped:
    s = meta["shipped"]
    return Shipped(s["features"], s["rustflags"], s["cargo_incremental"], tuple(s["bins"]), s["profile"])


# ---------------------------------------------------------------------------
# build and bench-build
# ---------------------------------------------------------------------------


def measure_binary(path: Path) -> dict:
    """`stat -c %s`, the Berkeley `size` triple, and `size -A` per section."""
    if not path.is_file():
        raise Fail(f"{path} was not built")
    berkeley = subprocess.run(["size", str(path)], capture_output=True, text=True, check=False)
    rows = berkeley.stdout.strip().splitlines()
    if berkeley.returncode != 0 or len(rows) < 2:
        raise Fail(f"`size {path}` failed: {berkeley.stderr.strip()}")
    fields = rows[1].split()
    try:
        text, data, bss = (int(fields[i]) for i in range(3))
    except (ValueError, IndexError) as exc:
        raise Fail(f"cannot parse `size` output for {path}: {rows[1]!r}") from exc
    sysv = subprocess.run(["size", "-A", str(path)], capture_output=True, text=True, check=False)
    if sysv.returncode != 0:
        raise Fail(f"`size -A {path}` failed: {sysv.stderr.strip()}")
    sections: dict[str, int] = {}
    for line in sysv.stdout.splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[1].isdigit() and parts[0] != "Total":
            sections[parts[0]] = int(parts[1])
    if ".text" not in sections:
        raise Fail(f"`size -A {path}` reported no .text section")
    return {
        "bytes": path.stat().st_size,
        "text": text,
        "data": data,
        "bss": bss,
        "text_section": sections[".text"],
        "sections": sections,
    }


def timed_dict(timed: Timed) -> dict:
    return {
        "wall_seconds": round(timed.wall_seconds, 1),
        "cpu_seconds": round(timed.cpu_seconds, 1),
        "peak_rss_mib": round(timed.peak_rss_mib, 1),
    }


# pierre-server's build script runs `npm install` and a TypeScript build when this
# file is missing and skips both when it exists. The cost does not depend on LTO,
# and the second cold build would find what the first one left, so it must be
# present (or absent) at the start of both builds alike. Nothing embeds the file.
SDK_MARKER = Path("sdk") / "dist" / "cli.js"


def cmd_build(args: argparse.Namespace) -> None:
    out = out_dir(args)
    repo = Path(args.repo).resolve()
    meta = load_meta(out)
    shipped = shipped_from_meta(meta)
    label = args.label
    profile = meta["profiles"][label]["name"]
    target = target_dir(out, label)
    if target.exists():
        shutil.rmtree(target)
    target.mkdir(parents=True)
    messages = out / "logs" / f"build-{label}.json"
    messages.parent.mkdir(parents=True, exist_ok=True)
    cmd = [
        "cargo", "build", "--locked", "--offline", "--profile", profile,
        *shipped_flags(shipped), *bin_args(shipped),
        "--message-format=json-render-diagnostics",
    ]
    print(f"cold build of `{profile}` into an empty {target}", flush=True)
    sdk_present = (repo / SDK_MARKER).exists()
    started_at = time.time()
    with messages.open("w", encoding="utf-8") as sink:
        timed = run_timed(cmd, cargo_env(shipped, target), repo, stdout=sink)
    if timed.returncode != 0:
        raise Fail(f"cargo build --profile {profile} exited {timed.returncode}")
    built = cargo_artifacts(messages, "bin")
    missing = [b for b in shipped.bins if b not in built]
    if missing:
        raise Fail(f"cargo reported no executable for {missing}")
    binaries = {name: measure_binary(built[name]) for name in shipped.bins}
    record = {
        "label": label,
        "profile": profile,
        "started_at": started_at,
        "sdk_dist_present_at_start": sdk_present,
        "build": timed_dict(timed),
        "target_bytes": dir_bytes(target),
        "binaries": binaries,
    }
    write_json(out / "build" / f"{label}.json", record)
    print(json.dumps(record, indent=2, sort_keys=True))


def cmd_bench_build(args: argparse.Namespace) -> None:
    out = out_dir(args)
    repo = Path(args.repo).resolve()
    meta = load_meta(out)
    shipped = shipped_from_meta(meta)
    label = args.label
    profile = meta["profiles"][label]["name"]
    target = target_dir(out, label)
    record_path = out / "build" / f"{label}.json"
    record = read_json(record_path)
    suites = args.suites.split()
    selection = [a for s in suites for a in ("--bench", s)] if suites else ["--benches"]
    messages = out / "logs" / f"bench-build-{label}.json"
    cmd = [
        "cargo", "bench", "--locked", "--offline", "-p", BENCH_PACKAGE, "--profile", profile,
        *shipped_flags(shipped), *selection, "--no-run",
        "--message-format=json-render-diagnostics",
    ]
    print(f"building the benches under `{profile}` (cargo bench --profile, not the bench profile)", flush=True)
    with messages.open("w", encoding="utf-8") as sink:
        timed = run_timed(cmd, cargo_env(shipped, target), repo, stdout=sink)
    if timed.returncode != 0:
        raise Fail(f"cargo bench --no-run --profile {profile} exited {timed.returncode}")
    built = cargo_artifacts(messages, "bench")
    if not built:
        raise Fail(f"cargo bench --no-run --profile {profile} produced no bench executable")
    if suites and sorted(built) != sorted(suites):
        raise Fail(f"asked for benches {sorted(suites)}, cargo built {sorted(built)}")
    dest = out / "bench-bin" / label
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    for name, exe in built.items():
        shutil.copy2(exe, dest / name)
    record["bench_build"] = {
        **timed_dict(timed),
        "suites": sorted(built),
        "cwd": str(manifest_dir_of(messages, sorted(built)[0])),
    }
    write_json(record_path, record)
    # The target dir has done its job; two of them would not fit beside each other.
    shutil.rmtree(target)
    print(json.dumps(record["bench_build"], indent=2, sort_keys=True))


# ---------------------------------------------------------------------------
# bench
# ---------------------------------------------------------------------------


def suite_order(rep: int, index: int) -> tuple[str, str]:
    """Thin first when (rep - 1 + suite index) is even, fat first when odd, so neither owns the first slot."""
    return LABELS if (rep - 1 + index) % 2 == 0 else (LABELS[1], LABELS[0])


def cmd_bench(args: argparse.Namespace) -> None:
    out = out_dir(args)
    suites = {label: sorted(p.name for p in (out / "bench-bin" / label).glob("*")) for label in LABELS}
    if not suites["thin"] or suites["thin"] != suites["fat"]:
        raise Fail(f"bench executables differ between profiles: thin={suites['thin']} fat={suites['fat']}")
    cwd = Path(read_json(out / "build" / "thin.json")["bench_build"]["cwd"])
    failures_path = out / "bench-failures.txt"
    failures_path.unlink(missing_ok=True)
    runs = []
    for rep in range(1, args.reps + 1):
        for index, suite in enumerate(suites["thin"]):
            for label in suite_order(rep, index):
                home = out / "criterion" / label / f"rep-{rep}" / suite
                home.mkdir(parents=True, exist_ok=True)
                env = dict(os.environ)
                env["CRITERION_HOME"] = str(home)
                log = out / "logs" / f"bench-{label}-rep{rep}-{suite}.log"
                with log.open("w", encoding="utf-8") as sink:
                    timed = run_timed(
                        [str(out / "bench-bin" / label / suite), "--bench", "--noplot"],
                        env, cwd, stdout=sink, stderr=subprocess.STDOUT, timeout=args.timeout,
                    )
                runs.append({"rep": rep, "suite": suite, "label": label, "seconds": round(timed.wall_seconds, 1), "exit": timed.returncode})
                print(f"rep {rep}/{args.reps} {suite:<24} {label:<4} {timed.wall_seconds:7.1f}s exit {timed.returncode}", flush=True)
                if timed.returncode != 0:
                    with failures_path.open("a", encoding="utf-8") as fh:
                        fh.write(f"{label} rep {rep} {suite} exit {timed.returncode} (log: {log.name})\n")
    write_json(out / "bench.json", {"reps": args.reps, "suites": suites["thin"], "runs": runs})
    if failures_path.exists():
        raise Fail(f"bench runs failed:\n{failures_path.read_text(encoding='utf-8')}")


# ---------------------------------------------------------------------------
# report
# ---------------------------------------------------------------------------


def collect_criterion(label_dir: Path) -> dict[tuple[str, str], dict[int, float]]:
    """{(suite, benchmark id): {rep: median ns}} from one label's CRITERION_HOME trees."""
    found: dict[tuple[str, str], dict[int, float]] = {}
    for rep_dir in sorted(label_dir.glob("rep-*")):
        rep = int(rep_dir.name.removeprefix("rep-"))
        for suite_dir in sorted(p for p in rep_dir.iterdir() if p.is_dir()):
            for estimates in sorted(suite_dir.rglob("new/estimates.json")):
                bench_dir = estimates.parent.parent
                ident = estimates.parent / "benchmark.json"
                if ident.is_file():
                    bench_id = str(read_json(ident)["full_id"])
                else:
                    bench_id = "/".join(bench_dir.relative_to(suite_dir).parts)
                median = read_json(estimates)["median"]["point_estimate"]
                found.setdefault((suite_dir.name, bench_id), {})[rep] = float(median)
    return found


def spread_pct(values: list[float]) -> float:
    centre = statistics.median(values)
    return (max(values) - min(values)) / centre * 100 if centre > 0 else 0.0


def judge(thin: dict[int, float], fat: dict[int, float], material_pct: float) -> dict:
    reps = sorted(set(thin) & set(fat))
    if not reps:
        return {"reps": [], "ratios": [], "delta_pct": None, "delta_range_pct": None, "verdict": "no data",
                "thin_ns": [], "fat_ns": [], "thin_median_ns": 0.0, "fat_median_ns": 0.0,
                "thin_spread_pct": 0.0, "fat_spread_pct": 0.0}
    ratios = [fat[r] / thin[r] for r in reps if thin[r] > 0]
    row = {
        "reps": reps,
        "thin_ns": [thin[r] for r in reps],
        "fat_ns": [fat[r] for r in reps],
        "thin_median_ns": statistics.median(thin[r] for r in reps),
        "fat_median_ns": statistics.median(fat[r] for r in reps),
        "thin_spread_pct": spread_pct([thin[r] for r in reps]),
        "fat_spread_pct": spread_pct([fat[r] for r in reps]),
        "ratios": ratios,
    }
    if len(ratios) != len(reps):
        row.update(delta_pct=None, delta_range_pct=None, verdict="no data")
        return row
    delta = (statistics.median(ratios) - 1) * 100
    row["delta_pct"] = delta
    row["delta_range_pct"] = [(min(ratios) - 1) * 100, (max(ratios) - 1) * 100]
    if all(r < 1 for r in ratios) and abs(delta) >= material_pct:
        row["verdict"] = "fat faster"
    elif all(r > 1 for r in ratios) and abs(delta) >= material_pct:
        row["verdict"] = "thin faster"
    else:
        row["verdict"] = "no clear difference"
    return row


def geomean_delta_pct(rows: list[dict]) -> float | None:
    medians = [statistics.median(r["ratios"]) for r in rows if r["ratios"] and r["delta_pct"] is not None]
    if not medians or any(m <= 0 for m in medians):
        return None
    return (math.exp(sum(math.log(m) for m in medians) / len(medians)) - 1) * 100


def fmt_ns(ns: float) -> str:
    for unit, scale in (("s", 1e9), ("ms", 1e6), ("µs", 1e3)):
        if ns >= scale:
            return f"{ns / scale:.2f} {unit}"
    return f"{ns:.1f} ns"


def fmt_seconds(seconds: float) -> str:
    whole = int(round(seconds))
    return f"{whole // 3600}h {whole % 3600 // 60:02d}m {whole % 60:02d}s" if whole >= 3600 else f"{whole // 60}m {whole % 60:02d}s"


def fmt_mib(count: float) -> str:
    return f"{count / 1024 / 1024:.1f} MiB"


def fmt_pct(value: float | None) -> str:
    return "n/a" if value is None else f"{value:+.1f}%"


def pct_change(thin: float, fat: float) -> float | None:
    return (fat / thin - 1) * 100 if thin > 0 else None


def single_build_row(name: str, thin: float, fat: float, fmt) -> list[str]:
    return [name, fmt(thin), fmt(fat), fmt_pct(pct_change(thin, fat)), "one build", "-", "-"]


def build_and_size_rows(thin: dict, fat: dict) -> list[list[str]]:
    rows = [
        single_build_row("Clean build, wall clock", thin["build"]["wall_seconds"], fat["build"]["wall_seconds"], fmt_seconds),
        single_build_row("Clean build, CPU time (user+sys)", thin["build"]["cpu_seconds"], fat["build"]["cpu_seconds"], fmt_seconds),
        single_build_row("Clean build, largest process RSS", thin["build"]["peak_rss_mib"], fat["build"]["peak_rss_mib"], lambda v: f"{v:.0f} MiB"),
        single_build_row("Target dir after the build", thin["target_bytes"], fat["target_bytes"], fmt_mib),
    ]
    # The binary the size gate measures first.
    for name in sorted(thin["binaries"], key=lambda n: (n != GATED_BIN, n)):
        a, b = thin["binaries"][name], fat["binaries"][name]
        rows.append(single_build_row(f"`{name}` size (`stat -c %s`)", a["bytes"], b["bytes"], fmt_mib))
        rows.append(single_build_row(f"`{name}` `.text` section", a["text_section"], b["text_section"], fmt_mib))
        rows.append(single_build_row(f"`{name}` `size` text (all code and rodata)", a["text"], b["text"], fmt_mib))
    if "bench_build" in thin and "bench_build" in fat:
        rows.append(single_build_row("Bench build, after the binaries", thin["bench_build"]["wall_seconds"], fat["bench_build"]["wall_seconds"], fmt_seconds))
    return rows


def bench_row(suite: str, bench_id: str, row: dict) -> list[str]:
    rng = row["delta_range_pct"]
    return [
        f"`{suite}` `{bench_id}`".replace("|", "\\|"),
        fmt_ns(row["thin_median_ns"]),
        fmt_ns(row["fat_median_ns"]),
        fmt_pct(row["delta_pct"]),
        "n/a" if rng is None else f"{rng[0]:+.1f}% to {rng[1]:+.1f}%",
        f"{row['thin_spread_pct']:.1f}% / {row['fat_spread_pct']:.1f}%",
        row["verdict"],
    ]


def profile_line(label: str, profile: dict) -> str:
    eff = profile["effective"]
    strip = eff["strip"]
    # Cargo prints the resolved setting as {"resolved": {"Named": "symbols"}}.
    strip = strip.get("resolved") if isinstance(strip, dict) else strip
    strip = strip.get("Named", strip) if isinstance(strip, dict) else strip
    return (
        f"- **{label}**: `{profile['name']}`: lto={eff['lto']}, codegen-units={eff['codegen_units']}, "
        f"opt-level={eff['opt_level']}, panic={eff['panic']}, strip={strip}"
    )


def render_header(meta: dict) -> list[str]:
    runner = meta["runner"]
    shipped = meta["shipped"]
    lines = [
        "## Thin vs fat LTO (carnet#499)",
        "",
        f"- Commit `{meta['commit'][:12]}`" + (f", [run]({meta['run_url']})" if meta["run_url"] else ""),
        f"- Runner: {runner['cpus']} CPUs, {runner['mem_gib']} GiB, {runner['cpu_model']}; {runner['platform']}"
        + (f"; image {runner['image_version']}" if runner["image_version"] else ""),
        f"- Toolchain: {meta['toolchain']['rustc']}, {meta['toolchain']['cargo']}",
        f"- Shipped invocation (the image builds `{shipped['profile']}`): `--no-default-features --features \"{shipped['features']}\"`, "
        f"RUSTFLAGS `{shipped['rustflags']}`, CARGO_INCREMENTAL={shipped['cargo_incremental']}",
        profile_line("thin", meta["profiles"]["thin"]),
        profile_line("fat", meta["profiles"]["fat"]),
    ]
    differ = meta["profiles_differ_in"]
    if differ == ["lto"]:
        lines.append("- The two profiles differ in `lto` and nothing else.")
    else:
        names = ", ".join(f"`{k}`" for k in differ) or "nothing"
        lines.append(f"- **The two profiles differ in {names}, not in `lto` alone**: read every difference below as the combined effect.")
    return lines + [""]


def render_runtime_summary(benches: list[tuple[tuple[str, str], dict]], reps: int, material_pct: float) -> list[str]:
    rows = [row for _, row in benches]
    counts = {v: sum(1 for r in rows if r["verdict"] == v) for v in ("fat faster", "thin faster", "no clear difference", "no data")}
    lines = [
        f"{reps} repetitions per benchmark. Thin and fat ran back to back for each suite, alternating which went first, on the same runner. "
        "A benchmark's figure is the median over repetitions of Criterion's per-repetition median; *spread* is (max - min) / median across repetitions. "
        "*Fat vs thin* is the median over repetitions of fat / thin - 1 taken per repetition, so negative means fat is faster; "
        "*range* is the smallest and largest of those per-repetition differences. "
        f"A verdict names a side only when every repetition agrees in sign and the median difference is at least {material_pct:g}%.",
        "",
        f"- **{counts['fat faster']}** fat faster, **{counts['thin faster']}** thin faster, **{counts['no clear difference']}** no clear difference"
        + (f", **{counts['no data']}** no data" if counts["no data"] else "")
        + f" of {len(rows)} benchmarks.",
        f"- Geometric mean difference, all benchmarks: **{fmt_pct(geomean_delta_pct(rows))}**.",
    ]
    for suite in sorted({s for (s, _), _ in benches}):
        suite_rows = [row for (s, _), row in benches if s == suite]
        lines.append(f"- `{suite}`: {fmt_pct(geomean_delta_pct(suite_rows))} over {len(suite_rows)} benchmarks.")
    return lines + [""]


def render(meta: dict, builds: dict, benches: list[tuple[tuple[str, str], dict]], reps: int, material_pct: float, problems: list[str]) -> list[str]:
    lines = render_header(meta)
    if benches:
        lines += render_runtime_summary(benches, reps, material_pct)
    table: list[list[str]] = []
    if len(builds) == 2:
        first = min(builds, key=lambda label: builds[label]["started_at"])
        table.append([f"**Build and size** (one cold build per profile, {first} first)", "", "", "", "", "", ""])
        table += build_and_size_rows(builds["thin"], builds["fat"])
    if benches:
        table.append(["**Runtime** (time per iteration)", "", "", "", "", "", ""])
        table += [bench_row(suite, bench_id, row) for (suite, bench_id), row in benches]
    if table:
        lines += ["| Metric | Thin | Fat | Fat vs thin | Range over repetitions | Spread thin / fat | Verdict |", "|---|---:|---:|---:|---:|---:|---|"]
        lines += ["| " + " | ".join(cells) + " |" for cells in table]
        lines.append("")
    if problems:
        lines += ["### Problems", ""] + [f"- {p}" for p in problems] + [""]
    return lines


def cmd_report(args: argparse.Namespace) -> None:
    out = out_dir(args)
    problems: list[str] = []
    meta = read_json(out / "meta.json")

    builds: dict[str, dict] = {}
    for label in LABELS:
        try:
            builds[label] = read_json(out / "build" / f"{label}.json")
        except Fail as exc:
            problems.append(str(exc))

    if len(builds) == 2 and builds["thin"].get("sdk_dist_present_at_start") != builds["fat"].get("sdk_dist_present_at_start"):
        problems.append(
            f"`{SDK_MARKER}` existed at the start of one cold build and not the other: pierre-server's build script "
            "runs an npm install and a TypeScript build when it is missing, a cost the other build skipped, so the build times are not comparable"
        )

    reps = 0
    try:
        reps = int(read_json(out / "bench.json")["reps"])
    except Fail as exc:
        problems.append(str(exc))
    failures = out / "bench-failures.txt"
    if failures.exists():
        problems += [f"bench run failed: {line}" for line in failures.read_text(encoding="utf-8").splitlines() if line]

    per_label = {label: collect_criterion(out / "criterion" / label) for label in LABELS}
    benches: list[tuple[tuple[str, str], dict]] = []
    if not per_label["thin"] and not per_label["fat"]:
        problems.append("no Criterion estimates were found for either profile")
    for key in sorted(set(per_label["thin"]) | set(per_label["fat"])):
        thin, fat = per_label["thin"].get(key), per_label["fat"].get(key)
        if thin is None or fat is None:
            problems.append(f"`{key[0]}` `{key[1]}` was measured for {'fat' if thin is None else 'thin'} only")
            continue
        if reps and (len(thin) != reps or len(fat) != reps):
            problems.append(f"`{key[0]}` `{key[1]}` has {len(thin)} thin and {len(fat)} fat repetitions, expected {reps}")
        benches.append((key, judge(thin, fat, args.material_pct)))

    lines = render(meta, builds, benches, reps, args.material_pct, problems)
    text = "\n".join(lines) + "\n"
    summary = args.summary_file or os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as fh:
            fh.write(text)
    print(text)
    write_json(
        out / "lto-compare.json",
        {
            "meta": meta,
            "builds": builds,
            "repetitions": reps,
            "material_pct": args.material_pct,
            "benchmarks": [{"suite": s, "id": i, **row} for (s, i), row in benches],
            "problems": problems,
        },
    )
    if problems:
        raise Fail(f"{len(problems)} problem(s) in the results; the table above is incomplete")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__.splitlines()[0], formatter_class=argparse.RawDescriptionHelpFormatter)
    default_out = os.environ.get("LTO_COMPARE_OUT") or str(Path(os.environ.get("RUNNER_TEMP", ".")) / "lto-compare")
    root.add_argument("--out", default=default_out, help="results directory (default: $LTO_COMPARE_OUT, else $RUNNER_TEMP/lto-compare)")
    root.add_argument("--repo", default=".", help="repository root (default: the current directory)")
    sub = root.add_subparsers(dest="command", required=True)

    describe = sub.add_parser("describe", help="record the runner and resolve both profiles")
    describe.add_argument("--thin-profile", required=True)
    describe.add_argument("--fat-profile", required=True)
    describe.set_defaults(run=cmd_describe)

    for name, func, help_text in (("build", cmd_build, "cold-build the shipped binaries"), ("bench-build", cmd_bench_build, "build the benches under the profile")):
        step = sub.add_parser(name, help=help_text)
        step.add_argument("--label", required=True, choices=LABELS)
        if name == "bench-build":
            step.add_argument("--suites", default="", help="space-separated bench targets (default: all)")
        step.set_defaults(run=func)

    bench = sub.add_parser("bench", help="run every bench executable of both profiles, interleaved")
    bench.add_argument("--reps", type=int, required=True)
    bench.add_argument("--timeout", type=int, default=DEFAULT_BENCH_TIMEOUT_SECONDS, help="seconds before one bench run is killed")
    bench.set_defaults(run=cmd_bench)

    report = sub.add_parser("report", help="write the table and lto-compare.json")
    report.add_argument("--material-pct", type=float, default=DEFAULT_MATERIAL_PCT)
    report.add_argument("--summary-file", default="", help="file to append the markdown to (default: $GITHUB_STEP_SUMMARY)")
    report.set_defaults(run=cmd_report)
    return root


def main() -> int:
    args = parser().parse_args()
    if getattr(args, "reps", 1) < 1:
        print("::error::--reps must be at least 1", file=sys.stderr)
        return 2
    try:
        args.run(args)
    except Fail as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
