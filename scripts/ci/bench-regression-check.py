#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

"""
ABOUTME: Gate criterion benchmark regressions — warn past --warn-pct, fail past --fail-pct
ABOUTME: Reads target/criterion/**/change/estimates.json, writes a table to GITHUB_STEP_SUMMARY
"""

import argparse
import json
import os
import sys
from pathlib import Path


def collect_changes(criterion_dir: Path):
    """Yield (bench_id, mean_pct, median_pct) for every baseline comparison.

    Criterion writes one change/estimates.json per compared benchmark, nested
    arbitrarily deep (group names containing '/' become subdirectories). The
    point estimates are relative fractions: 0.05 == +5%.
    """
    for estimates in sorted(criterion_dir.rglob("change/estimates.json")):
        rel = estimates.relative_to(criterion_dir)
        # criterion's HTML report tree lives under report/ — not a benchmark
        if rel.parts[0] == "report":
            continue
        bench_id = "/".join(rel.parts[:-2])
        try:
            data = json.loads(estimates.read_text())
            mean_pct = data["mean"]["point_estimate"] * 100
            median_pct = data["median"]["point_estimate"] * 100
        except (json.JSONDecodeError, KeyError, TypeError) as exc:
            print(f"::notice::skipping unparsable {estimates}: {exc}")
            continue
        yield bench_id, mean_pct, median_pct


def write_summary(lines):
    """Append markdown to the GitHub job summary, or stdout when run locally."""
    text = "\n".join(lines) + "\n"
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        with open(summary_path, "a", encoding="utf-8") as fh:
            fh.write(text)
    print(text)


def main():
    parser = argparse.ArgumentParser(
        description="Fail CI on severe criterion benchmark regressions"
    )
    parser.add_argument("--criterion-dir", type=Path, default=Path("target/criterion"))
    parser.add_argument("--warn-pct", type=float, default=10.0)
    parser.add_argument("--fail-pct", type=float, default=50.0)
    args = parser.parse_args()

    results = list(collect_changes(args.criterion_dir)) if args.criterion_dir.is_dir() else []

    if not results:
        write_summary(
            [
                "## Benchmark regression check",
                "",
                "No baseline comparison data (first run or baseline cache miss) — skipped.",
            ]
        )
        return 0

    severe = []
    rows = [
        "## Benchmark regression check",
        "",
        f"Thresholds: warn > +{args.warn_pct:g}%, fail > +{args.fail_pct:g}% (mean vs `main` baseline).",
        "",
        "| Benchmark | Mean change | Median change | Status |",
        "|---|---:|---:|---|",
    ]
    for bench_id, mean_pct, median_pct in results:
        if mean_pct > args.fail_pct:
            status = "❌ severe regression"
            severe.append((bench_id, mean_pct))
            print(f"::error::{bench_id} regressed {mean_pct:+.1f}% (mean), past the {args.fail_pct:g}% gate")
        elif mean_pct > args.warn_pct:
            status = "⚠️ regression"
            print(f"::warning::{bench_id} regressed {mean_pct:+.1f}% (mean)")
        elif mean_pct < -args.warn_pct:
            status = "🎉 improvement"
        else:
            status = "OK"
        rows.append(f"| {bench_id} | {mean_pct:+.1f}% | {median_pct:+.1f}% | {status} |")

    write_summary(rows)

    if severe:
        worst = max(severe, key=lambda item: item[1])
        print(
            f"FAIL: {len(severe)} benchmark(s) past the +{args.fail_pct:g}% gate "
            f"(worst: {worst[0]} at {worst[1]:+.1f}%)"
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
