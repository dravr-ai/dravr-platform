#!/usr/bin/env bash
# ABOUTME: Fails when a workflow runs `cargo ... --test <name>` for a test target the workspace no longer has
# ABOUTME: Compile-free: resolves each name against crates/*/tests and [[test]] declarations, scoped by -p/--workspace
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

# A test file moved out of the workspace takes its target with it, and the
# workflow that names it by `--test` finds out only when it next runs. For a
# push lane that is the same push; for a cron lane it is the next morning, and
# then every morning after. `llm-live-cron.yml` went red on six consecutive
# scheduled runs (2026-09-19 onward, carnet#474) with
#   error: no test target named `llm_openai_compatible_test` in default-run packages
# because the commit that moved the OpenAI-compatible provider tests into
# dravr-embacle deleted the file and left the workflow's four `--test` flags
# alone. Nothing in the push that did it ran that workflow.
#
# This asks the question without compiling: for every `--test <name>` inside a
# `cargo` invocation in .github/workflows, does the package selection the
# command makes (its `-p`/`--package` flags, `--workspace`, or the workspace's
# default members) contain a test target of that name? A target is a
# `tests/<name>.rs` or `tests/<name>/main.rs` under a package that has not
# turned autotests off, or a `[[test]]` name declared in its Cargo.toml.
#
# Scope and deliberate exclusions:
#   - Comment lines are skipped; backslash continuations are joined, so a flag
#     on the fourth line of a `cargo test \` command is still read.
#   - A name built at runtime (`--test "$target"`, `${{ matrix.test }}`) cannot
#     be resolved statically. Those are counted and reported, never failed on.
#   - Arguments after `--` belong to the test binary, not to cargo, and are not
#     read (that is where `--test-threads` lives).
#   - Every command is read as run from the repository root. A step that runs
#     cargo from inside a member crate narrows cargo's default package set to
#     that crate, which this does not model; no workflow does so today.
#   - The scan asserts its own premise: no workflows, no test targets, or no
#     static reference resolved at all is a scan that verified nothing, and it
#     fails rather than reporting a clean pass.
#
# Usage: check-workflow-test-targets.sh [repo-root]   (default: this checkout)

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
REPO_ROOT="${1:-$( cd "$SCRIPT_DIR/../.." && pwd )}"

echo -e "${BLUE}==== Workflow Test Targets (cargo --test -> workspace targets) ====${NC}"

if [ ! -f "$REPO_ROOT/Cargo.toml" ]; then
    echo -e "${RED}❌ No Cargo.toml at $REPO_ROOT — nothing to resolve targets against.${NC}"
    exit 1
fi

# The exit status of the scan is judged on its own; its output is judged by a
# success marker afterwards. Neither stands in for the other: a crashed scan
# prints no UNRESOLVED line either.
if ! OUT="$(python3 - "$REPO_ROOT" <<'PY' 2>&1
import glob, os, re, shlex, sys, tomllib

root = sys.argv[1]


def load(path):
    with open(path, "rb") as handle:
        return tomllib.load(handle)


# ---- side 1: every test target the workspace declares, per package --------
root_manifest = load(os.path.join(root, "Cargo.toml"))
workspace = root_manifest.get("workspace", {})
excluded = {os.path.normpath(os.path.join(root, e)) for e in workspace.get("exclude", [])}

member_dirs = []
if "package" in root_manifest:
    member_dirs.append(root)
for pattern in workspace.get("members", []):
    for path in sorted(glob.glob(os.path.join(root, pattern))):
        path = os.path.normpath(path)
        if path in excluded or not os.path.isfile(os.path.join(path, "Cargo.toml")):
            continue
        if path not in member_dirs:
            member_dirs.append(path)

targets = {}      # package name -> set of test target names
dir_to_pkg = {}
for pkg_dir in member_dirs:
    manifest = load(os.path.join(pkg_dir, "Cargo.toml"))
    package = manifest.get("package")
    if not package or "name" not in package:
        continue
    name = package["name"]
    dir_to_pkg[os.path.normpath(pkg_dir)] = name
    names = set()
    claimed = set()
    for entry in manifest.get("test", []):
        if "name" in entry:
            names.add(entry["name"])
        if "path" in entry:
            claimed.add(os.path.normpath(os.path.join(pkg_dir, entry["path"])))
    if package.get("autotests", True):
        tests_dir = os.path.join(pkg_dir, "tests")
        for path in glob.glob(os.path.join(tests_dir, "*.rs")):
            if os.path.normpath(path) not in claimed:
                names.add(os.path.basename(path)[:-3])
        for path in glob.glob(os.path.join(tests_dir, "*", "main.rs")):
            if os.path.normpath(path) not in claimed:
                names.add(os.path.basename(os.path.dirname(path)))
    targets[name] = names

all_pkgs = set(targets)
if "default-members" in workspace:
    default_pkgs = {
        dir_to_pkg[os.path.normpath(p)]
        for pattern in workspace["default-members"]
        for p in glob.glob(os.path.join(root, pattern))
        if os.path.normpath(p) in dir_to_pkg
    }
elif "package" in root_manifest:
    default_pkgs = {dir_to_pkg[os.path.normpath(root)]}
else:
    default_pkgs = set(all_pkgs)

target_count = sum(len(v) for v in targets.values())

# ---- side 2: every --test a workflow's cargo invocation names -------------
workflow_files = sorted(
    glob.glob(os.path.join(root, ".github", "workflows", "*.yml"))
    + glob.glob(os.path.join(root, ".github", "workflows", "*.yaml"))
)


def logical_lines(path):
    """Yield (first physical line number, text) with comments dropped and
    backslash continuations joined."""
    with open(path, encoding="utf8") as handle:
        raw = handle.read().splitlines()
    buffer, start = "", None
    for number, line in enumerate(raw, 1):
        if line.lstrip().startswith("#"):
            continue
        if start is None:
            start = number
        stripped = line.rstrip()
        if stripped.endswith("\\"):
            buffer += stripped[:-1] + " "
            continue
        yield start, buffer + line
        buffer, start = "", None
    if buffer:
        yield start, buffer


def tokens(text):
    try:
        lexer = shlex.shlex(text, posix=True, punctuation_chars=True)
        lexer.whitespace_split = True
        return list(lexer)
    except ValueError:
        # An unbalanced quote in prose (an `echo "don't"`) is not a cargo
        # command; a whitespace split still reads any flags on the line.
        return [t.strip("'\"") for t in text.split()]


SHELL_BREAK = re.compile(r"^[;&|()<>]+$")
DYNAMIC = re.compile(r"[$*{}<>\[\]]")

refs, dynamic = [], []
for path in workflow_files:
    rel = os.path.relpath(path, root)
    for line_no, text in logical_lines(path):
        if "--test" not in text or "cargo" not in text:
            continue
        toks = tokens(text)
        i = 0
        while i < len(toks):
            if toks[i] != "cargo" and not toks[i].endswith("/cargo"):
                i += 1
                continue
            pkgs, pkgs_dynamic, whole_workspace, names = [], False, False, []
            i += 1
            while i < len(toks) and toks[i] != "--" and not SHELL_BREAK.match(toks[i]):
                tok = toks[i]
                value = None
                if tok in ("-p", "--package", "--test") and i + 1 < len(toks):
                    value = toks[i + 1]
                    i += 1
                elif tok.startswith("--package="):
                    tok, value = "--package", tok.split("=", 1)[1]
                elif tok.startswith("--test="):
                    tok, value = "--test", tok.split("=", 1)[1]
                elif tok.startswith("-p") and len(tok) > 2 and not tok.startswith("--"):
                    tok, value = "-p", tok[2:]
                elif tok in ("--workspace", "--all"):
                    whole_workspace = True
                if value is not None and tok in ("-p", "--package"):
                    if DYNAMIC.search(value):
                        pkgs_dynamic = True
                    else:
                        pkgs.append(value.split("@", 1)[0])
                elif value is not None and tok == "--test":
                    names.append(value)
                i += 1
            if whole_workspace or pkgs_dynamic:
                scope = set(all_pkgs)
            elif pkgs:
                scope = set(pkgs)
            else:
                scope = set(default_pkgs)
            for name in names:
                if DYNAMIC.search(name):
                    dynamic.append(f"{rel}:{line_no} --test {name}")
                else:
                    refs.append((rel, line_no, name, scope, pkgs))

unresolved = []
for rel, line_no, name, scope, pkgs in refs:
    unknown = [p for p in pkgs if p not in targets]
    if unknown:
        unresolved.append(f"UNRESOLVED {rel}:{line_no} --test {name} (package {', '.join(unknown)} is not in the workspace)")
        continue
    if not any(name in targets[p] for p in scope):
        owners = sorted(p for p, names in targets.items() if name in names)
        where = f"it is in {', '.join(owners)}, outside the -p selection" if owners else "no package has it"
        unresolved.append(f"UNRESOLVED {rel}:{line_no} --test {name} ({where})")

print(f"workflows scanned: {len(workflow_files)}")
print(f"packages: {len(targets)}, test targets: {target_count}")
print(f"static --test references: {len(refs)}")
print(f"runtime-built --test references (not checkable): {len(dynamic)}")
for line in dynamic:
    print(f"  dynamic {line}")
for line in unresolved:
    print(line)
if not workflow_files or not target_count or not refs:
    print("SCAN INCOMPLETE")
    raise SystemExit(1)
if unresolved:
    raise SystemExit(1)
print("WORKFLOW TEST TARGETS RESOLVED")
PY
)"; then
    echo "$OUT" | sed 's/^/    /'
    if echo "$OUT" | grep -q "SCAN INCOMPLETE"; then
        echo -e "${RED}❌ Scan verified nothing — no workflows, no test targets, or no --test reference.${NC}"
        echo -e "${RED}Fix the scan or its inputs; an empty scan is never a pass.${NC}"
    elif echo "$OUT" | grep -q "^UNRESOLVED"; then
        echo -e "${RED}❌ A workflow names a cargo test target the workspace does not have.${NC}"
        echo -e "${RED}cargo refuses the whole command, so every other --test on that line stops running too.${NC}"
        echo -e "${RED}Point the workflow at the target that exists, or drop the flag with the test it named.${NC}"
    else
        echo -e "${RED}❌ The scan itself failed.${NC}"
    fi
    exit 1
fi

# Grep for the success marker, never for the absence of a finding.
if ! echo "$OUT" | grep -q "WORKFLOW TEST TARGETS RESOLVED"; then
    echo "$OUT" | sed 's/^/    /'
    echo -e "${RED}❌ Scan did not report success — treating as failure.${NC}"
    exit 1
fi

echo "$OUT" | grep -v "WORKFLOW TEST TARGETS RESOLVED" | sed 's/^/    /'
echo -e "${GREEN}✅ Every static cargo --test in .github/workflows resolves to a workspace test target${NC}"
exit 0
