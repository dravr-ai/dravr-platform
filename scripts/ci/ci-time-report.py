#!/usr/bin/env python3
# ABOUTME: Measures a repo's CI time the way the Oct 7-20 2026 baseline did: job-hours, runner wait and the two plan targets
# ABOUTME: Lists runs and jobs through `gh api`, caches every response, and re-reports from the cache alone with --offline
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

"""Measures a repo's CI time the way the Oct 7-20 2026 baseline did.

carnet#1114's plan (vault: Work Log/2026-10/Platform CI Time Plan (2026-10-21))
set two goals against a baseline measured with throwaway scripts kept beside the
plan in ``ci-time-baseline/``. This is those scripts' method in one place, so a
re-measure after each phase is comparable with the baseline:

  job-hours    completed_at - started_at of every job that ran (``ran``: started,
               completed, not skipped, and held by a runner), over every attempt
               of the run (jobs?filter=all), summed per workflow and split
               Linux / macOS by runner label (c.py).
  runner wait  started_at - created_at of every job that ran, floored at 0 (b.py,
               bw.py). A dependent job's created_at is its dependency's
               completed_at, so this is how long a runnable job waited for a
               runner; the report re-checks that premise on the window.
  branch       "CI: Backend (Rust)" pushes to a non-main branch that concluded
  Backend      success on attempt 1: run updated_at - run created_at (h.py; the
               plan's 86 min baseline, goal 30). The success-or-failure variant
               the baseline also printed is shown beside it, not as the target.
  main         per commit pushed to main: from the first run the push created to
  verdict      the last updated_at of the verdict workflows (h.py; baseline 103,
               goal 45), plus the variant that also waits for mobile E2E (119).

Percentiles of run wall times use ``q`` (linear interpolation, common.py).
Weighted percentiles of job waits use ``weighted_quantile`` (the first value whose
cumulative weight reaches p, bw.py's ``wq``), so a census window and the
baseline print comparable numbers.

Runs are listed per half day (the runs endpoint stops at 1,000 results for one
``created`` filter, and the half-day split is what runs.py used). Jobs are
fetched for every run that was not skipped when the window holds at most
--census-max of them; otherwise for the runs inside each --census-window plus a
stratified sample of the rest (plan.py): strata are (workflow name normalised by
``norm``, event, main/other), each stratum gives min(n, max(2, round(rate * n)))
runs, drawn with random.Random(--seed) in listing order. A sampled run weighs
(non-census runs in its stratum) / (sampled runs in its stratum) and a census run
weighs 1, which without census windows is bw.py's cnt/fet.

That census-aware weight is what produced the baseline's published waits (0.8 /
22.6 / 60.7 min, Backend 3.2 / 41.0); bw.py as saved weighs every fetched run
cnt/fet and prints 2.2 / 28.3 / 81.2 on the same data, because it counts the
fully fetched busy census days as if they were a sample. c.py's 2,294 job-hours
used that cnt/fet weight too; the census-aware weight gives 2,240 on the same
data, and that is the baseline this script's job-hours compare against.

Every API response is cached as JSON under --cache-dir, so a rerun makes no API
call and --offline reports from the cache alone. A runs page holding a run that
has not completed, or a jobs page holding a job that has not, is not cached: the
next run fetches it again instead of reporting a stale snapshot.
"""

from __future__ import annotations

import argparse
import collections
import json
import os
import random
import re
import subprocess
import sys
import tempfile
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from datetime import date, datetime, timedelta
from pathlib import Path
from typing import Callable, Iterable

DEFAULT_REPO = "dravr-ai/dravr-platform"
DEFAULT_SAMPLE_RATE = 0.12
DEFAULT_SEED = 42
# A window with at most this many non-skipped runs is fetched in full.
DEFAULT_CENSUS_MAX = 300
PER_PAGE = 100
# The runs endpoint returns at most this many results for one `created` filter.
SEARCH_CAP = 1000
HALF_DAYS = (("00:00:00", "11:59:59"), ("12:00:00", "23:59:59"))
FETCH_THREADS = 8
GH_ATTEMPTS = 3
GH_RETRY_SECONDS = 2
# b.py: a job created more than this after its run's first job is a dependent
# one, and its dependency is a sibling that completed within this of it.
DEPENDENCY_SLACK_SECONDS = 3

BACKEND = "CI: Backend (Rust)"
# h.py: the workflows whose last completion is main's verdict on a push.
VERDICT_CI = frozenset(
    {
        "CI: Backend (Rust)",
        "CI: Backend (PostgreSQL)",
        "CI: Integration (HTTP/MCP)",
        "CI: TypeScript SDK",
        "CI: Web Frontend",
        "CI: Mobile Unit",
        "CI: Backend (Redis)",
        "CI: MCP Protocol Compliance",
        "CI: Design System",
        "CI: Examples Validation",
        "CI: Messaging Eval",
    }
)
VERDICT_E2E = frozenset({"CI: Mobile E2E (iOS)", "CI: Mobile E2E (Android)"})
# bw.py: the workflows whose wait the baseline reported one by one.
WAIT_WORKFLOWS = (
    "CI: Backend (Rust)",
    "CI: Backend (PostgreSQL)",
    "CI: Web Frontend",
    "CI: TypeScript SDK",
    "CI: Integration (HTTP/MCP)",
    "CI: Mobile E2E (Android)",
    "Deploy (sha)",
)
GOAL_BRANCH_BACKEND_P90 = 30.0
GOAL_MAIN_VERDICT_P90 = 45.0

CENSUS_WINDOW = re.compile(r"^\d{4}-\d{2}-\d{2}(T\d{2}(:\d{2}(:\d{2})?)?)?$")


class ReportError(Exception):
    """A condition the report cannot continue past or present as a measurement."""


# ---------------------------------------------------------------------------
# The baseline's definitions (common.py)
# ---------------------------------------------------------------------------


def ts(value: str | None) -> datetime | None:
    return datetime.fromisoformat(value.replace("Z", "+00:00")) if value else None


def minutes(start: str, end: str) -> float:
    return (ts(end) - ts(start)).total_seconds() / 60


def norm(name: str) -> str:
    """One stratum per deploy and per Dependabot update, not one per commit."""
    if re.match(r"Deploy [0-9a-f]{40}", name):
        return "Deploy (sha)"
    if " - Update #" in name:
        return "dependabot"
    return name


def branch_class(run: dict) -> str:
    return "main" if run["head_branch"] == "main" else "other"


def stratum(run: dict) -> tuple[str, str, str]:
    return (norm(run["name"]), run["event"], branch_class(run))


def ran(job: dict) -> bool:
    return bool(job["started_at"] and job["completed_at"] and job["conclusion"] != "skipped" and job.get("runner_name"))


def job_hours(job: dict) -> float:
    return minutes(job["started_at"], job["completed_at"]) / 60 if ran(job) else 0.0


def is_mac(job: dict) -> bool:
    return any("macos" in label.lower() for label in job.get("labels") or [])


def q(xs: Iterable[float], p: float) -> float:
    """Linear-interpolation percentile, exactly common.py's `q`. The caller guarantees xs is not empty."""
    xs = sorted(xs)
    k = (len(xs) - 1) * p
    i = int(k)
    return xs[i] if i + 1 >= len(xs) else xs[i] + (xs[i + 1] - xs[i]) * (k - i)


def weighted_quantile(pairs: Iterable[tuple[float, float]], p: float) -> float:
    """bw.py's `wq`: the first value, in sorted order, whose cumulative weight reaches p of the total."""
    pairs = sorted(pairs)
    total = sum(w for _, w in pairs)
    acc = 0.0
    for value, weight in pairs:
        acc += weight
        if acc >= p * total:
            return value
    return pairs[-1][0]


# ---------------------------------------------------------------------------
# The API and its cache
# ---------------------------------------------------------------------------


def read_json(path: Path) -> dict:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ReportError(f"cached response {path} is not valid JSON: {exc}") from exc


def write_json(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    partial = path.with_name(path.name + ".partial")
    partial.write_text(json.dumps(data), encoding="utf-8")
    os.replace(partial, path)


def gh_api(endpoint: str) -> dict:
    stderr = ""
    for attempt in range(GH_ATTEMPTS):
        try:
            proc = subprocess.run(["gh", "api", endpoint], capture_output=True, text=True, check=False)
        except FileNotFoundError as exc:
            raise ReportError("the gh CLI is not on PATH; install it, or rerun with --offline against a filled cache") from exc
        if proc.returncode == 0:
            try:
                return json.loads(proc.stdout)
            except json.JSONDecodeError as exc:
                raise ReportError(f"gh api {endpoint} returned something that is not JSON: {exc}") from exc
        stderr = proc.stderr.strip()
        if attempt + 1 < GH_ATTEMPTS:
            time.sleep(GH_RETRY_SECONDS)
    raise ReportError(f"gh api {endpoint} failed {GH_ATTEMPTS} times: {stderr[:300]}")


class Api:
    """`gh api` GETs, each answered from the cache when it holds them."""

    def __init__(self, repo: str, cache_dir: Path, offline: bool) -> None:
        self.repo = repo
        self.root = cache_dir / repo
        self.offline = offline
        self.calls = 0
        self._lock = threading.Lock()

    def get(self, endpoint: str, cache_file: Path, cacheable: Callable[[dict], bool]) -> dict:
        if cache_file.is_file():
            return read_json(cache_file)
        if self.offline:
            raise ReportError(f"{endpoint} is not in the cache ({cache_file}) and --offline forbids fetching it")
        data = gh_api(endpoint)
        with self._lock:
            self.calls += 1
        if cacheable(data):
            write_json(cache_file, data)
        return data

    def pages(self, endpoint: str, key: str, cache_key: Path, cacheable: Callable[[dict], bool]) -> list[dict]:
        """Every item of a paginated list endpoint whose items live under `key`."""
        items: list[dict] = []
        page = 1
        while True:
            url = f"{endpoint}&per_page={PER_PAGE}&page={page}"
            data = self.get(url, self.root / cache_key / f"page-{page}.json", cacheable)
            if "total_count" not in data or key not in data:
                raise ReportError(f"{url} answered without total_count/{key}: {str(data)[:200]}")
            if data["total_count"] > SEARCH_CAP and key == "workflow_runs":
                raise ReportError(f"{url} reports {data['total_count']} runs, over the API's {SEARCH_CAP}-result cap, so the list would be cut short")
            items += data[key]
            if len(items) >= data["total_count"] or not data[key]:
                return items
            page += 1


def all_completed(key: str) -> Callable[[dict], bool]:
    return lambda data: all(item.get("status") == "completed" for item in data.get(key, []))


def list_runs(api: Api, since: date, until: date) -> list[dict]:
    """Every run created in [since, until], in listing order (half days ascending, newest first within one)."""
    runs: list[dict] = []
    seen: set[int] = set()
    day = since
    while day <= until:
        for lo, hi in HALF_DAYS:
            window = f"{day.isoformat()}T{lo}Z..{day.isoformat()}T{hi}Z"
            listed = api.pages(
                f"repos/{api.repo}/actions/runs?created={window}",
                "workflow_runs",
                Path("runs") / window,
                all_completed("workflow_runs"),
            )
            for run in listed:
                if run["id"] not in seen:
                    seen.add(run["id"])
                    runs.append(run)
        day += timedelta(days=1)
    first, last = since.isoformat(), (until + timedelta(days=1)).isoformat()
    return [r for r in runs if first <= r["created_at"] < last]


def list_jobs(api: Api, run_id: int) -> list[dict]:
    return api.pages(
        f"repos/{api.repo}/actions/runs/{run_id}/jobs?filter=all",
        "jobs",
        Path("jobs") / str(run_id),
        all_completed("jobs"),
    )


# ---------------------------------------------------------------------------
# Which runs get their jobs fetched, and what each one weighs
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Selection:
    census: frozenset[int]
    sampled: frozenset[int]
    strata: int
    population: int  # non-census runs the sample stands for

    def fetched(self) -> frozenset[int]:
        return self.census | self.sampled


def in_window(created_at: str, windows: list[tuple[str, str]]) -> bool:
    return any(lo <= created_at < hi for lo, hi in windows)


def select(runs: list[dict], census_max: int, windows: list[tuple[str, str]], rate: float, seed: int) -> Selection:
    """plan.py's census and stratified sample over the non-skipped `runs`, in their listing order."""
    if len(runs) <= census_max:
        return Selection(frozenset(r["id"] for r in runs), frozenset(), 0, 0)
    census = frozenset(r["id"] for r in runs if in_window(r["created_at"], windows))
    strata: dict[tuple[str, str, str], list[dict]] = collections.defaultdict(list)
    for run in runs:
        if run["id"] not in census:
            strata[stratum(run)].append(run)
    rng = random.Random(seed)
    sampled: list[int] = []
    for members in strata.values():
        k = min(len(members), max(2, round(rate * len(members))))
        sampled += [r["id"] for r in rng.sample(members, k)]
    return Selection(census, frozenset(sampled), len(strata), sum(len(m) for m in strata.values()))


def weights(runs: list[dict], selection: Selection) -> dict[int, float]:
    """1 for a census run; (non-census runs) / (sampled runs) of its stratum for a sampled one."""
    population = collections.Counter(stratum(r) for r in runs if r["id"] not in selection.census)
    drawn = collections.Counter(stratum(r) for r in runs if r["id"] in selection.sampled)
    out: dict[int, float] = {}
    for run in runs:
        if run["id"] in selection.census:
            out[run["id"]] = 1.0
        elif run["id"] in selection.sampled:
            out[run["id"]] = population[stratum(run)] / drawn[stratum(run)]
    return out


# ---------------------------------------------------------------------------
# The measurements
# ---------------------------------------------------------------------------


def hours_by_workflow(runs: list[dict], jobs: dict[int, list[dict]], weight: dict[int, float]) -> dict[str, list[float]]:
    """workflow -> [Linux job-hours, macOS job-hours], each job scaled by its run's weight."""
    out: dict[str, list[float]] = collections.defaultdict(lambda: [0.0, 0.0])
    for run in runs:
        if run["id"] not in weight:
            continue
        for job in jobs[run["id"]]:
            hours = job_hours(job)
            if hours:
                out[norm(run["name"])][1 if is_mac(job) else 0] += hours * weight[run["id"]]
    return dict(out)


@dataclass(frozen=True)
class Wait:
    minutes: float
    weight: float
    workflow: str
    mac: bool


def waits(runs: list[dict], jobs: dict[int, list[dict]], weight: dict[int, float]) -> list[Wait]:
    out: list[Wait] = []
    for run in runs:
        if run["id"] not in weight:
            continue
        for job in jobs[run["id"]]:
            if ran(job):
                out.append(Wait(max(0.0, minutes(job["created_at"], job["started_at"])), weight[run["id"]], norm(run["name"]), is_mac(job)))
    return out


def dependency_check(runs: list[dict], jobs: dict[int, list[dict]]) -> tuple[int, int]:
    """b.py: (dependent jobs created within the slack of a sibling's completion, dependent jobs)."""
    matched = dependent = 0
    for run in runs:
        if run["id"] not in jobs:
            continue
        attempt = [j for j in jobs[run["id"]] if j.get("run_attempt") == run["run_attempt"]]
        if not attempt:
            continue
        first = min(ts(j["created_at"]) for j in attempt)
        for job in attempt:
            if job["conclusion"] == "skipped" or not job["started_at"] or not job["completed_at"]:
                continue
            created = ts(job["created_at"])
            if created <= first + timedelta(seconds=DEPENDENCY_SLACK_SECONDS):
                continue
            dependent += 1
            if any(p["completed_at"] and abs((ts(p["completed_at"]) - created).total_seconds()) <= DEPENDENCY_SLACK_SECONDS for p in attempt):
                matched += 1
    return matched, dependent


def branch_backend_walls(runs: list[dict], conclusions: tuple[str, ...]) -> list[float]:
    return [
        minutes(r["created_at"], r["updated_at"])
        for r in runs
        if r["name"] == BACKEND and r["event"] == "push" and r["head_branch"] != "main" and r["conclusion"] in conclusions and r["run_attempt"] == 1
    ]


@dataclass(frozen=True)
class Verdicts:
    ci: list[float]
    with_e2e: list[float]
    unfinished: int  # main commits left out because a verdict run had not completed


def main_verdicts(runs: list[dict]) -> Verdicts:
    """h.py: per commit pushed to main, first run created -> last verdict workflow updated."""
    by_sha: dict[str, list[dict]] = collections.defaultdict(list)
    for run in runs:
        if run["event"] == "push" and run["head_branch"] == "main":
            by_sha[run["head_sha"]].append(run)
    ci: list[float] = []
    with_e2e: list[float] = []
    unfinished = 0
    for pushed in by_sha.values():
        verdict = [r for r in pushed if r["name"] in VERDICT_CI]
        if not verdict:
            continue
        everything = [r for r in pushed if r["name"] in VERDICT_CI | VERDICT_E2E]
        if any(r["status"] != "completed" for r in everything):
            unfinished += 1
            continue
        start = min(r["created_at"] for r in pushed)
        ci.append(minutes(start, max(r["updated_at"] for r in verdict)))
        with_e2e.append(minutes(start, max(r["updated_at"] for r in everything)))
    return Verdicts(ci, with_e2e, unfinished)


# ---------------------------------------------------------------------------
# The report
# ---------------------------------------------------------------------------


def wall_summary(xs: list[float]) -> str:
    return f"n={len(xs)} p50={q(xs, .5):.1f} p90={q(xs, .9):.1f} p99={q(xs, .99):.1f}"


def wait_summary(rows: list[Wait]) -> str:
    if not rows:
        return "jobs=0 (no job ran)"
    pairs = [(w.minutes, w.weight) for w in rows]
    p50, p90, p99 = (weighted_quantile(pairs, p) for p in (0.5, 0.9, 0.99))
    return f"jobs={len(rows)} p50={p50:.1f} p90={p90:.1f} p99={p99:.1f}"


def target_line(label: str, xs: list[float], goal: float) -> str:
    if not xs:
        return f"  {label}: NO DATA (no qualifying run in the window, so nothing to compare with the {goal:.0f} min goal)"
    p90 = q(xs, 0.9)
    verdict = "PASS" if p90 <= goal else "MISS"
    return f"  {label}: p90={p90:.1f} min, goal <= {goal:.0f} min: {verdict} ({wall_summary(xs)})"


def report(args: argparse.Namespace) -> list[str]:
    api = Api(args.repo, Path(args.cache_dir), args.offline)
    listed = list_runs(api, args.since, args.until)
    if not listed:
        raise ReportError(f"no workflow run of {args.repo} was created between {args.since} and {args.until}; refusing to report an empty window")
    runs = [r for r in listed if r["conclusion"] != "skipped"]
    if not runs:
        raise ReportError(f"every one of the {len(listed)} runs between {args.since} and {args.until} was skipped; there is nothing to measure")

    selection = select(runs, args.census_max, args.census_window, args.sample_rate, args.seed)
    todo = [r["id"] for r in runs if r["id"] in selection.fetched()]
    print(f"fetching jobs for {len(todo)} runs ({api.calls} API calls so far)", file=sys.stderr)
    with ThreadPoolExecutor(FETCH_THREADS) as pool:
        jobs = dict(zip(todo, pool.map(lambda rid: list_jobs(api, rid), todo)))
    weight = weights(runs, selection)

    unfinished = sum(1 for r in listed if r["status"] != "completed")
    lines = [
        f"CI time report: {args.repo}, runs created {args.since}..{args.until} (UTC)",
        f"  runs: {len(listed)} listed, {len(listed) - len(runs)} skipped, {unfinished} not completed",
    ]
    if not selection.sampled:
        lines.append(f"  jobs: census, all {len(todo)} runs that were not skipped (at most --census-max {args.census_max})")
    else:
        lines.append(
            f"  jobs: {len(selection.census)} census + {len(selection.sampled)} sampled of {selection.population} other runs "
            f"(strata {selection.strata}, rate {args.sample_rate}, seed {args.seed}); a sampled run weighs its stratum's share"
        )
    lines.append(f"  API calls: {api.calls} (cache: {api.root})")

    hours = hours_by_workflow(runs, jobs, weight)
    linux = sum(v[0] for v in hours.values())
    mac = sum(v[1] for v in hours.values())
    total = linux + mac
    lines += ["", "1. Job-hours by workflow (every attempt; sampled runs scaled by weight)", f"  {'workflow':<44} {'linux':>9} {'macOS':>8} {'total':>9} {'share':>6}"]
    for name, (lx, mc) in sorted(hours.items(), key=lambda kv: (-(kv[1][0] + kv[1][1]), kv[0])):
        share = (lx + mc) / total if total else 0.0
        lines.append(f"  {name:<44} {lx:>9.2f} {mc:>8.2f} {lx + mc:>9.2f} {share:>6.1%}")
    lines.append(f"  {'TOTAL':<44} {linux:>9.2f} {mac:>8.2f} {total:>9.2f}")

    rows = waits(runs, jobs, weight)
    matched, dependent = dependency_check(runs, jobs)
    lines += [
        "",
        "2. Runner wait, started_at - created_at per job that ran (minutes, weighted)",
        "  A dependent job's created_at is when its dependency completed, so this is the wait of a runnable job for a runner:",
        f"  {matched} of {dependent} dependent jobs were created within {DEPENDENCY_SLACK_SECONDS} s of a sibling's completion.",
        f"  {'all jobs':<28} {wait_summary(rows)}",
        f"  {'Linux':<28} {wait_summary([w for w in rows if not w.mac])}",
        f"  {'macOS':<28} {wait_summary([w for w in rows if w.mac])}",
    ]
    for name in WAIT_WORKFLOWS:
        lines.append(f"  {name:<28} {wait_summary([w for w in rows if w.workflow == name])}")

    verdicts = main_verdicts(listed)
    either = branch_backend_walls(listed, ("success", "failure"))
    lines += [
        "",
        "3. Plan targets (run wall minutes, created_at -> updated_at)",
        target_line(f'branch "{BACKEND}" (push to a non-main branch, success, attempt 1)', branch_backend_walls(listed, ("success",)), GOAL_BRANCH_BACKEND_P90),
        f"    success or failure (printed by the baseline too, not the target): {wall_summary(either) if either else 'n=0'}",
        target_line("main verdict (main push -> last verdict CI workflow)", verdicts.ci, GOAL_MAIN_VERDICT_P90),
        f"    including mobile E2E (iOS, Android), no goal of its own: {wall_summary(verdicts.with_e2e) if verdicts.with_e2e else 'n=0'}",
    ]
    if verdicts.unfinished:
        lines.append(f"    {verdicts.unfinished} main commit(s) left out: a verdict run had not completed")
    return lines


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def iso_day(value: str) -> date:
    try:
        return date.fromisoformat(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"{value!r} is not a YYYY-MM-DD date") from exc


def census_window(value: str) -> tuple[str, str]:
    lo, sep, hi = value.partition("..")
    if not sep or not CENSUS_WINDOW.match(lo) or not CENSUS_WINDOW.match(hi) or not lo < hi:
        raise argparse.ArgumentTypeError(f"{value!r} is not FROM..TO with FROM < TO, each a UTC timestamp prefix such as 2026-10-11T20")
    return lo, hi


def default_cache_dir() -> str:
    job_dir = os.environ.get("CLAUDE_JOB_DIR")
    return str(Path(job_dir) / "tmp" / "ci-time-report") if job_dir else str(Path(tempfile.gettempdir()) / "ci-time-report")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__.splitlines()[0], epilog=__doc__.split("\n", 2)[2], formatter_class=argparse.RawDescriptionHelpFormatter)
    root.add_argument("--since", type=iso_day, required=True, help="first UTC day of the window (YYYY-MM-DD)")
    root.add_argument("--until", type=iso_day, required=True, help="last UTC day of the window, inclusive (YYYY-MM-DD)")
    root.add_argument("--repo", default=DEFAULT_REPO, help=f"owner/name (default: {DEFAULT_REPO})")
    root.add_argument("--cache-dir", default=default_cache_dir(), help="response cache (default: $CLAUDE_JOB_DIR/tmp/ci-time-report, else the system temp dir)")
    root.add_argument("--offline", action="store_true", help="read the cache only; a response it lacks is an error")
    root.add_argument("--sample-rate", type=float, default=DEFAULT_SAMPLE_RATE, help=f"share of each stratum sampled (default: {DEFAULT_SAMPLE_RATE})")
    root.add_argument("--seed", type=int, default=DEFAULT_SEED, help=f"sample seed (default: {DEFAULT_SEED})")
    root.add_argument("--census-max", type=int, default=DEFAULT_CENSUS_MAX, help=f"fetch every run's jobs when the window has at most this many non-skipped runs (default: {DEFAULT_CENSUS_MAX})")
    root.add_argument(
        "--census-window",
        type=census_window,
        action="append",
        default=[],
        metavar="FROM..TO",
        help="when sampling, fetch every run created in [FROM, TO) as well (repeatable; the baseline: 2026-10-11T20..2026-10-13 and 2026-10-18T20..2026-10-20)",
    )
    return root


def main() -> int:
    args = parser().parse_args()
    if args.until < args.since:
        print(f"ci-time-report: error: --until {args.until} is before --since {args.since}", file=sys.stderr)
        return 2
    if not 0 < args.sample_rate <= 1 or args.census_max < 0:
        print("ci-time-report: error: --sample-rate must be in (0, 1] and --census-max at least 0", file=sys.stderr)
        return 2
    try:
        lines = report(args)
    except ReportError as exc:
        print(f"ci-time-report: error: {exc}", file=sys.stderr)
        return 1
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
