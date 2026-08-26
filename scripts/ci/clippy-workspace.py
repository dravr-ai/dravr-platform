#!/usr/bin/env python3
# ABOUTME: Runs full-workspace clippy and names the crates its error list could not reach
# ABOUTME: A crate whose dependency failed is never linted, so a red run reports only part of the debt
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

"""Full-workspace clippy, plus an honest account of what it did not check.

``--keep-going`` makes cargo continue past a failing crate into every crate
that does not depend on it. What it cannot do — what nothing can do — is lint a
crate whose dependency failed to compile: there is no ``rmeta`` to compile
against. Those crates emit no diagnostics at all, which reads exactly like
having none.

That reading cost three sessions an evening on 2026-08-26. A single
``too_long_first_doc_paragraph`` in ``pierre-services`` was the only error the
job could reach; four more sat unevaluated in ``pierre-cli``,
``pierre-routes-admin`` and ``pierre-routes-web-admin``, and two more in
``pierre-chat-pipeline``. Each was found by fixing the visible one and watching
main go red again on "new" errors that had been there all along.

So this wrapper reports the difference between *clean* and *unevaluated*. It
runs the same lint command, streams the same diagnostics, exits with the same
status, and adds one block naming the crates cargo never got to and the failure
that blocked each one.

The CI job runs it unscoped, which is the case it is written for. Passing extra
cargo args (-p some-crate) works and is how the report is exercised by hand,
but note that a crate outside a scoped invocation which also depends on a failed
one is reported as blocked rather than as out of scope — true, but louder than
it needs to be for a local spot check.
"""

import json
import os
import subprocess
import sys

# Mirrors the job's contract: every target, every feature, zero tolerance.
CANONICAL_ARGS = ["--keep-going", "--all-targets", "--all-features"]


def cargo_metadata():
    """Resolve graph + workspace membership, in cargo's own id representation.

    Package ids are matched by string against the ids in the JSON diagnostic
    stream, so both must come from the same cargo — which they do, since this
    process invokes both.
    """
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(out.stdout)


def run_clippy(extra_args):
    """Stream clippy, rendering diagnostics live; collect what it reached.

    Returns ``(status, evaluated, failed)`` where ``evaluated`` is every package
    that produced at least one artifact (fresh units included — cargo still
    emits ``compiler-artifact`` for them, so a warm cache does not read as a
    skipped crate) and ``failed`` is every package that emitted an error.
    """
    cmd = [
        "cargo",
        "clippy",
        *CANONICAL_ARGS,
        *extra_args,
        "--message-format=json-diagnostic-rendered-ansi",
        "--",
        "-D",
        "warnings",
    ]
    # stderr is inherited: cargo's own progress and summary lines keep flowing
    # to the log in real time, unchanged from a bare `cargo clippy` run.
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, text=True, bufsize=1)

    evaluated, failed = set(), set()
    for line in proc.stdout:
        line = line.rstrip("\n")
        if not line.startswith("{"):
            if line:
                print(line, flush=True)
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            print(line, flush=True)
            continue

        reason = msg.get("reason")
        if reason == "compiler-artifact":
            evaluated.add(msg.get("package_id"))
        elif reason == "compiler-message":
            body = msg.get("message") or {}
            rendered = body.get("rendered")
            if rendered:
                print(rendered, end="", flush=True)
            if body.get("level") == "error":
                failed.add(msg.get("package_id"))

    return proc.wait(), evaluated, failed


def blockers_for(pid, deps, failed, memo):
    """Which failed packages this one transitively depends on."""
    if pid in memo:
        return memo[pid]
    memo[pid] = frozenset()  # cycles cannot happen in cargo, but never recurse forever
    found = set()
    for dep in deps.get(pid, ()):
        if dep in failed:
            found.add(dep)
        found |= blockers_for(dep, deps, failed, memo)
    memo[pid] = frozenset(found)
    return memo[pid]


def emit_summary(lines):
    """Mirror the report into the job summary when running under Actions."""
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    if not path:
        return
    with open(path, "a", encoding="utf-8") as handle:
        handle.write("\n".join(lines) + "\n")


def main(argv):
    meta = cargo_metadata()
    members = set(meta["workspace_members"])
    name_of = {p["id"]: p["name"] for p in meta["packages"]}
    deps = {
        node["id"]: [d["pkg"] for d in node.get("deps", [])]
        for node in meta["resolve"]["nodes"]
    }

    status, evaluated, failed = run_clippy(argv)

    memo = {}
    blocked, unbuilt = {}, set()
    for member in members:
        if member in evaluated or member in failed:
            continue
        causes = blockers_for(member, deps, failed, memo)
        if causes:
            blocked[member] = causes
        else:
            # No failed dependency, no artifacts: cargo was never asked to build
            # it. Normal for a scoped run, so it is stated rather than warned.
            unbuilt.add(member)

    label = lambda pid: name_of.get(pid, pid)  # noqa: E731 — one-line display helper

    if not blocked:
        if not unbuilt:
            print(f"\n✅ Clippy evaluated all {len(members)} workspace crates.")
        else:
            print(
                f"\n✅ Clippy evaluated {len(members) - len(unbuilt)} of "
                f"{len(members)} workspace crates; {len(unbuilt)} were outside "
                "this invocation's scope."
            )
        return status

    report = [
        "",
        f"⚠️  {len(blocked)} workspace crate(s) were NOT evaluated by this run.",
        "",
        "   Cargo cannot lint a crate whose dependency failed to compile, so the",
        "   errors above are a partial list. Fixing them will surface whatever",
        "   these crates are hiding — expect the next run to find more, not fewer.",
        "",
    ]
    for member in sorted(blocked, key=label):
        causes = ", ".join(sorted(label(c) for c in blocked[member]))
        report.append(f"   - {label(member)}  (blocked by {causes})")
    report.append("")
    report.append("   Lint them directly before pushing the fix:")
    for member in sorted(blocked, key=label):
        report.append(
            f"     cargo clippy -p {label(member)} --all-targets --all-features -- -D warnings"
        )
    report.append("")

    print("\n".join(report))
    emit_summary(report)
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
