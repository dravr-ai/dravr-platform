#!/usr/bin/env bash
# ABOUTME: Fails when .github/workflows/actions.lock does not cover every workflow as it stands, a locked commit left
# ABOUTME: its ref, or a job image has no digest — gh-actions-lock --verify, failing closed on a missing tool or report
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   Every `uses:` in .github resolves a tag or a branch, and a moved tag runs
#   new code beside the deploy credentials (the tj-actions/changed-files
#   incident). actions.lock records the commit each workflow's actions resolved
#   to, and the runner runs that commit instead of whatever the tag points at.
#   A workflow or composite whose `uses:` changed without a relock makes the
#   runner reject the job, and an upstream force-push that takes a locked
#   commit out of its ref's history does the same. This gate finds both before
#   a run does: on the push that changes .github, and weekly for upstream.
#
# What fails:
#   - any error finding from `gh actions-lock --verify`: a dependency the lock
#     does not record for that workflow (not-pinned), a workflow missing from
#     the lock, a `./` local action (use `$/.github/actions/<name>`), a locked
#     commit no longer in its ref's history, a moved full tag, a repository
#     whose id changed under its name
#   - a container-image finding: a job container or service image without an
#     @sha256 digest. The extension reports it as a warning and exits 0, but
#     the lock pins action commits only, so the digest is the image's pin.
#   - a tracked symlink whose target the repository archive does not hold. A
#     `$/` step makes the runner download and extract that archive, and the
#     extraction fails at "Set up job" on a dangling link — every job using a
#     composite would. Leave such a link out of archives in .gitattributes
#     (`<path> export-ignore`), as .claude/skills and its siblings are.
#
# What passes and is printed:
#   - ref-moved and other warnings: a floating ref (v6, main) that upstream has
#     moved past the locked commit. The runner keeps running the locked commit;
#     taking the new one is a reviewed relock, never an automatic one.
#
# Exit 0  every workflow verified, no error finding, every job image digested
# Exit 1  a finding; the extension missing or not the pinned version; no
#         lockfile, no workflow or no tracked file; the extension crashed,
#         printed no report, printed one that disagrees with its exit status,
#         or covered fewer workflows than .github/workflows holds — a check
#         that verified nothing must not report success
#
# Usage: check-actions-lock.sh [REPO_ROOT]
#   GH  the gh binary to run (default: gh on PATH)
set -euo pipefail

# The one pin. actions-lock.yml installs exactly this version, and
# check-actions-lock.test.sh fails when the two disagree: another release can
# write or judge the lock differently (the format itself is versioned).
GH_ACTIONS_LOCK_VERSION="v0.4.1"

if [[ "${1:-}" == "--pinned-version" ]]; then
    echo "$GH_ACTIONS_LOCK_VERSION"
    exit 0
fi

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
GH="${GH:-gh}"
INSTALL_HINT="gh extension install github/gh-actions-lock --pin ${GH_ACTIONS_LOCK_VERSION}"
RELOCK="gh actions-lock --no-narrow"

if ! command -v "$GH" >/dev/null 2>&1; then
    echo "❌ check-actions-lock: gh is not on PATH; install it, then: ${INSTALL_HINT}"
    exit 1
fi

version_out=""
if ! version_out="$("$GH" actions-lock --version 2>&1)"; then
    echo "❌ check-actions-lock: the gh-actions-lock extension is not installed: ${INSTALL_HINT}"
    exit 1
fi
installed="${version_out##* }"
if [[ "$installed" != "$GH_ACTIONS_LOCK_VERSION" ]]; then
    echo "❌ check-actions-lock: gh-actions-lock ${GH_ACTIONS_LOCK_VERSION} is required, found '${version_out}'."
    echo "   gh extension remove actions-lock && ${INSTALL_HINT}"
    exit 1
fi

shopt -s nullglob
workflow_files=("$ROOT"/.github/workflows/*.yml "$ROOT"/.github/workflows/*.yaml)
shopt -u nullglob
if [[ "${#workflow_files[@]}" -eq 0 ]]; then
    echo "❌ check-actions-lock: no workflow under $ROOT/.github/workflows; the check verified nothing"
    exit 1
fi
if [[ ! -f "$ROOT/.github/workflows/actions.lock" ]]; then
    echo "❌ check-actions-lock: $ROOT/.github/workflows/actions.lock is missing: run ${RELOCK} and commit it"
    exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# The archive a `$/` step extracts holds the tracked tree minus export-ignore,
# so every symlink in it must land on a path it holds. Offline, from the index:
# the tracked paths, their link targets and the .gitattributes rules alike.
if ! git -C "$ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "❌ check-actions-lock: $ROOT is not a git work tree, so the archive a \$/ action extracts cannot be judged"
    exit 1
fi
if ! python3 - "$ROOT" <<'PY'
import os
import subprocess
import sys

root = sys.argv[1]


def git(*args, stdin=None):
    return subprocess.run(["git", "-C", root, *args], input=stdin, capture_output=True, check=True).stdout


entries = [e for e in git("ls-files", "-s", "-z").split(b"\0") if e]
if not entries:
    print(f"❌ check-actions-lock: the index at {root} tracks no file, so the archive check judged nothing")
    sys.exit(1)
files, links = set(), set()
for entry in entries:
    meta, path = entry.split(b"\t", 1)
    path = path.decode()
    files.add(path)
    if meta.split()[0] == b"120000":
        links.add(path)
# git archive drops a path marked export-ignore and, for a marked directory,
# everything under it. check-attr matches a rule against the path it is given
# and never looks at the disk: neither `docs` nor `docs/` reaches the files
# under docs, and `docs/` matches the directory only when it is asked for as
# `docs/`. So every ancestor directory is asked for with its trailing slash.
ancestors = set()
for path in files:
    parent = os.path.dirname(path)
    while parent:
        ancestors.add(parent)
        parent = os.path.dirname(parent)
queries = sorted(files) + sorted(f"{d}/" for d in ancestors)
attrs = git("check-attr", "--cached", "--stdin", "-z", "export-ignore", stdin="\0".join(queries).encode() + b"\0").split(b"\0")
marked = {attrs[i].decode().rstrip("/") for i in range(0, len(attrs) - 2, 3) if attrs[i + 2] == b"set"}


def ignored(path):
    while path:
        if path in marked:
            return True
        path = os.path.dirname(path)
    return False


archived = {p for p in files if not ignored(p)}
dirs = {os.path.dirname(p) for p in archived}
for d in list(dirs):
    while d:
        d = os.path.dirname(d)
        dirs.add(d)


def lands(path, depth=0):
    if depth > 20:
        return False
    if path in links and path in archived:
        target = git("cat-file", "-p", f":{path}").decode()
        nxt = os.path.normpath(os.path.join(os.path.dirname(path), target))
        return not nxt.startswith("..") and not os.path.isabs(nxt) and lands(nxt, depth + 1)
    return path in archived or path in dirs


dangling = sorted(p for p in links if p in archived and not lands(p))
for path in dangling:
    target = git("cat-file", "-p", f":{path}").decode()
    print(f"❌ symlink {path} -> {target} dangles in the repository archive a $/ action extracts")
if dangling:
    print("   ↳ leave it out of archives: add `<path> export-ignore` to .gitattributes")
    sys.exit(1)
PY
then
    exit 1
fi

# The extension resolves workflow paths against the working directory, and
# GH_ACTIONS_LOCK_REPO_ROOT pins the root it resolves `$/` actions against.
code=0
(
    cd "$ROOT"
    GH_ACTIONS_LOCK_REPO_ROOT="$ROOT" "$GH" actions-lock --no-interactive --verify --json=valid,findings,workflows
) >"$WORK/report.json" 2>"$WORK/stderr" || code=$?

python3 - "$WORK/report.json" "$code" "${#workflow_files[@]}" "$WORK/stderr" "$GH_ACTIONS_LOCK_VERSION" "$RELOCK" <<'PY'
import json
import sys

report_path, code, expected, stderr_path, version, relock = sys.argv[1:7]
code, expected = int(code), int(expected)


def stderr_tail():
    try:
        lines = open(stderr_path, encoding="utf-8", errors="replace").read().splitlines()
    except OSError:
        return
    for line in lines[-15:]:
        print(f"   | {line}")


def die(message):
    print(f"❌ check-actions-lock: {message}")
    stderr_tail()
    sys.exit(1)


if code not in (0, 1):
    die(f"gh actions-lock exited {code}: it crashed or refused to run, so it verified nothing")
try:
    with open(report_path, encoding="utf-8") as handle:
        report = json.load(handle)
except (OSError, ValueError):
    die(f"gh actions-lock exited {code} and printed no JSON report; it verified nothing")
if not isinstance(report, dict):
    die("the report is not a JSON object; it verified nothing")

valid = report.get("valid")
workflows = report.get("workflows")
findings = report.get("findings")
if not isinstance(valid, bool) or not isinstance(workflows, list) or not isinstance(findings, list):
    die("the report lacks a boolean 'valid', a 'workflows' list or a 'findings' list")
if valid != (code == 0):
    die(f"the report says valid={str(valid).lower()} but the extension exited {code}")
if len(workflows) != expected:
    die(f"the report covers {len(workflows)} workflows but .github/workflows holds {expected}")

errors = [f for f in findings if f.get("severity") == "error"]
images = [f for f in findings if f.get("severity") != "error" and f.get("category") == "container-image"]
notes = [f for f in findings if f.get("severity") != "error" and f.get("category") != "container-image"]

for finding in notes:
    where = " ".join(x for x in (finding.get("workflow"), finding.get("dependency")) if x)
    print(f"⚠️  {finding.get('category', '?')}: {where} — {finding.get('detail', '')}")
for finding in errors + images:
    where = " ".join(x for x in (finding.get("workflow"), finding.get("dependency")) if x)
    print(f"❌ {finding.get('category', '?')}: {where}")
    print(f"   {finding.get('detail', '')}")
    if finding.get("remediation"):
        print(f"   ↳ {finding['remediation']}")

if errors or images:
    print("")
    print("How to fix (scripts/ci/README.md, \"Workflow lockfile\"):")
    print(f"  - a workflow or composite changed a uses: line → {relock}, and commit")
    print("    .github/workflows/actions.lock in the same commit")
    print("  - a composite is referenced as ./.github/actions/<name> → use $/.github/actions/<name>")
    print(f"  - actions.lock conflicts in a rebase → take either side, rerun {relock} on the result")
    print("  - a locked commit left its ref's history → read the upstream history before relocking")
    print("  - an image without a digest → image: <name>:<tag>@sha256:<multi-arch index digest>")
    print("  Never edit actions.lock by hand.")
    sys.exit(1)
if not valid:
    die("the report is invalid but names no error finding")

moved = f", {len(notes)} warning(s) above" if notes else ""
print(f"✅ check-actions-lock: {len(workflows)} workflows verified against actions.lock (gh-actions-lock {version}{moved})")
PY
