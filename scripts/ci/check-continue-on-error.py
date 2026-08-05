#!/usr/bin/env python3
# ABOUTME: Fails when a workflow step sets continue-on-error without being allowlisted
# ABOUTME: Matches on workflow:job:step names, so the allowlist cannot drift with line numbers
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
"""Check `continue-on-error: true` usage against the approved allowlist.

Why names and not line numbers
------------------------------
The allowlist previously pinned ``workflow_file:line_number``. Any edit above an
allowlisted step shifts it, so the entry silently stops matching and the check
fails on a step that was already approved — twice within twenty minutes on
2026-08-05, from nothing more than adding a comment. The TOML header has always
documented a ``workflow_file:step_name`` form, but the matcher compared the
allowlist against ``rg``'s ``file:line:content`` output, where the content is
``continue-on-error: true`` — so a name-based entry could never match and the
documented form did not work.

This parses the workflows instead: every step carrying the flag is identified by
``workflow.yml:job_id:Step Name``, which survives edits above it and says plainly
which step was approved.
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path

import yaml


def approved(patterns_file: Path) -> set[str]:
    with patterns_file.open("rb") as handle:
        config = tomllib.load(handle)
    section = config.get("ci_continue_on_error_allowlist", {})
    return {entry.strip() for entry in section.get("allowed", []) if entry.strip()}


def offenders(workflow_dir: Path) -> list[str]:
    """Every step with `continue-on-error: true`, as workflow:job:step keys."""
    found: list[str] = []
    for path in sorted(workflow_dir.glob("*.yml")) + sorted(workflow_dir.glob("*.yaml")):
        try:
            doc = yaml.safe_load(path.read_text(errors="ignore")) or {}
        except yaml.YAMLError as exc:
            print(f"  cannot parse {path.name}: {exc}")
            continue
        for job_id, job in (doc.get("jobs") or {}).items():
            if not isinstance(job, dict):
                continue
            # A job-level flag is far more dangerous than a step-level one: it
            # lets every step in the job fail silently. Report it the same way
            # so it must be approved explicitly too.
            if job.get("continue-on-error") is True:
                found.append(f"{path.name}:{job_id}:<job>")
            for step in job.get("steps") or []:
                if isinstance(step, dict) and step.get("continue-on-error") is True:
                    name = step.get("name") or step.get("uses") or "<unnamed>"
                    found.append(f"{path.name}:{job_id}:{name}")
    return found


def main() -> int:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    allowlist = approved(root / "scripts" / "ci" / "validation-patterns.toml")
    all_uses = offenders(root / ".github" / "workflows")

    unapproved = [use for use in all_uses if use not in allowlist]
    # An allowlist entry that no longer matches anything is stale — it would
    # quietly permit nothing while looking like active approval.
    stale = sorted(allowlist - set(all_uses))

    for entry in stale:
        print(f"  STALE allowlist entry (matches no step): {entry}")
    for use in unapproved:
        print(f"  FORBIDDEN continue-on-error: {use}")

    if unapproved:
        print("")
        print("continue-on-error lets a failing step pass silently. If this use is")
        print("genuinely justified — e.g. the outcome is captured and a later step")
        print("fails on it — add the exact key above to")
        print("scripts/ci/validation-patterns.toml [ci_continue_on_error_allowlist].")
    if stale:
        print("")
        print("Remove stale entries: approval that matches nothing is noise, and hides")
        print("whether the real use is still approved.")

    if unapproved or stale:
        return 1

    print(f"  ok   {len(all_uses)} continue-on-error use(s), all approved by name")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
