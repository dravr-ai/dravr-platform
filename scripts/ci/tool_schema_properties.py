# ABOUTME: Attributes declared properties and rejection sites to the tool that owns them
# ABOUTME: Shared by check-contremaitre-sync.sh Check 9 and check-declared-parameters.sh
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Two gates ask "which parameters does tool X have?" from opposite directions, and
# each grew its own answer. This is the one answer, so a third does not get written.
#
# THE SEGMENTATION
#
# These files are written definition-then-execute, per tool, throughout:
#
#     impl McpTool for ToolX {
#         fn definition(&self) -> Tool {
#             properties.insert("a", ...);          <- X declares
#             object_schema(properties, Some(vec!["a"]))
#             tool_definition("x", ...)             <- X is named HERE
#         }
#         async fn execute(...) {
#             args.get("a").ok_or_else(|| "Missing required parameter: a")
#         }                                          <- X rejects
#     }
#     impl McpTool for ToolY { ... }                 <- Y declares, Y is named, Y rejects
#
# So one cut at each `tool_definition("name")` literal yields both halves:
# everything BEFORE a literal (back to the previous one) is that tool's schema,
# and everything AFTER it (up to the next one) is that tool's handler. Callers
# get `properties`/`required` from the first and `rejections` from the second.
#
# WHY NOT PER FILE, AND WHY NOT PER MODULE
#
# Per file is wrong in both directions: one file defines many tools, so tool A's
# read looks like tool B's undeclared parameter (78 false positives in Check 9's
# first run), and a schema in `mod.rs` does not sit in the same file as its
# handler in `inner.rs`.
#
# Per module — the cheap fix check-declared-parameters.sh shipped with — is quiet
# but has a false negative that fails silent: a rejection in tool A is satisfied
# by a declaration in tool B in the same module, and the gate reports green. This
# retires it.
#
# THE ASSUMPTION IS ASSERTED, NOT INHERITED
#
# The segmentation depends on schema-before-definition ordering. That held for
# 32 of 32 files when this was written, and it is an assumption rather than an
# invariant: a file that ever builds its properties AFTER its `tool_definition`
# would read as declaring nothing, and both gates would pass on it in silence.
# `scan` raises on that rather than trusting it. It also counts every
# `tool_definition(` call site against the ones whose name it could read as a
# literal — 113 of 113 when written — so a computed name breaks the scan loudly
# instead of quietly shrinking its coverage.

import glob
import os
import re

TOOL_DEF = re.compile(r'tool_definition\(\s*\n?\s*"([a-z0-9_]+)"')
TOOL_DEF_ANY = re.compile(r"tool_definition\(")
PROP_INSERT = re.compile(r'properties\.insert\(\s*\n?\s*"([a-z0-9_]+)"')
PROP_TUPLE = re.compile(r'\(\s*"([a-z0-9_]+)"\.to_owned\(\)\s*,\s*PropertySchema')
REQUIRED_VEC = re.compile(r"Some\(vec!\[(.*?)\]\)", re.S)
STRING_LIT = re.compile(r'"([a-z0-9_]+)"')

# How a handler says "you did not give me X".
#
# Every phrasing the handlers actually use has to be here, because a phrasing
# the scan cannot see is a rejection it cannot check -- and the check then
# reports green over exactly the drift it exists to catch. The coach -> agent
# rename left three messages naming `coach_id` for parameters called
# `agent_id`, and this list matched none of them: two spell it
# "Missing required 'X' argument" and one "X is required to ...".
REJECTIONS = [
    re.compile(r"Missing required parameter:\s*([a-z0-9_]+)"),
    re.compile(r"Missing required '([a-z0-9_]+)' argument"),
    re.compile(r'"([a-z0-9_]+) is required\b'),
    re.compile(r'"([a-z0-9_]+) must be a UUID'),
    re.compile(r'missing_parameter\(\s*[^,]+,\s*"([a-z0-9_]+)"'),
    re.compile(r'require_string_field\(\s*&?args\s*,\s*"([a-z0-9_]+)"'),
]

# Accepted by every tool, added outside the per-tool schema.
UNIVERSAL = {"format"}


class ScanError(RuntimeError):
    """The scan could not stand behind its own result."""


def _module_of(path):
    """The tool package a file belongs to.

    `recipes/inner.rs` and `recipes/mod.rs` are one module; `admin.rs` is its
    own. A nested `inner/` directory is part of its parent package rather than a
    module of its own — `analytics/inner/compare.rs` rejects on parameters that
    `analytics/mod.rs` declares, and splitting them reported five analytics
    tools' `activity_id` as undeclared.
    """
    d, base = os.path.split(path)
    while os.path.basename(d) == "inner":
        d = os.path.dirname(d)
    parent = os.path.basename(d)
    if base in ("mod.rs", "inner.rs") or parent not in ("", "src"):
        return d
    return path


def _line_of(src, offset):
    return src[:offset].count("\n") + 1


def scan(pattern="crates/**/*.rs", skip_tests=True):
    """Attribute properties and rejections to the tool that owns them.

    Returns (tools, stats). `tools` maps a tool name to
    {"properties": set, "required": set, "rejections": {param: [site, ...]}}.

    Raises ScanError when the scan's own premise does not hold — an ordering
    violation, or a tool name it could not read.
    """
    tools = {}
    orphans = {}
    files_scanned = 0
    call_sites = 0
    named = 0
    ordering_violations = []

    for path in sorted(glob.glob(pattern, recursive=True)):
        if skip_tests and (os.sep + "tests" + os.sep) in path:
            continue
        try:
            src = open(path, encoding="utf-8").read()
        except OSError:
            continue
        if "tool_definition(" not in src:
            # A helper file beside the tools — recipes/inner.rs holds the
            # handlers whose schemas live in recipes/mod.rs. Its rejections
            # belong to *some* tool in this module and static analysis cannot
            # say which, so they are reported separately rather than dropped.
            # Callers fall back to module scope for these, knowingly.
            found = {}
            for rx in REJECTIONS:
                for r in rx.finditer(src):
                    found.setdefault(r.group(1), []).append(
                        f"{path}:{_line_of(src, r.start())}"
                    )
            if found:
                mod = _module_of(path)
                bucket = orphans.setdefault(mod, {})
                for param, sites in found.items():
                    bucket.setdefault(param, []).extend(sites)
            continue
        files_scanned += 1

        defs = list(TOOL_DEF.finditer(src))
        # Every call site, so a computed name cannot quietly shrink coverage.
        # The definition of tool_definition itself is not a call site.
        for m in TOOL_DEF_ANY.finditer(src):
            head = src.rfind("\n", 0, m.start())
            if "fn tool_definition" in src[max(0, head) : m.end()]:
                continue
            call_sites += 1
        named += len(defs)

        if not defs:
            continue

        # The premise: nothing declares a property after the last tool is named.
        tail = PROP_INSERT.findall(src[defs[-1].start() :])
        if tail:
            ordering_violations.append(
                f"{path}: {len(tail)} properties.insert after the final "
                f"tool_definition ({defs[-1].group(1)}): {sorted(set(tail))[:5]}"
            )

        for i, m in enumerate(defs):
            name = m.group(1)
            before = src[defs[i - 1].start() if i else 0 : m.start()]
            after = src[m.start() : defs[i + 1].start() if i + 1 < len(defs) else len(src)]

            entry = tools.setdefault(
                name,
                {
                    "properties": set(),
                    "required": set(),
                    "rejections": {},
                    "module": _module_of(path),
                },
            )
            entry["properties"] |= set(PROP_INSERT.findall(before))
            entry["properties"] |= set(PROP_TUPLE.findall(before))
            for blob in REQUIRED_VEC.findall(before):
                entry["required"] |= set(STRING_LIT.findall(blob))
            for rx in REJECTIONS:
                for r in rx.finditer(after):
                    entry["rejections"].setdefault(r.group(1), []).append(
                        f"{path}:{_line_of(src, m.start() + r.start())}"
                    )

    if ordering_violations:
        raise ScanError(
            "schema-before-definition ordering is violated, so these tools read "
            "as declaring nothing:\n  " + "\n  ".join(ordering_violations)
        )
    if call_sites != named:
        raise ScanError(
            f"{call_sites - named} tool_definition call site(s) have a name this "
            f"scan cannot read as a literal, so its coverage is unknown "
            f"({named} of {call_sites} resolved)"
        )
    if not tools:
        raise ScanError(f"no tools resolved from {pattern!r} — refusing to pass")

    # An orphan belongs to a tool only if its module has one. The branch above
    # takes every file without a tool_definition, which is right for a helper
    # sitting beside the tools it serves and wrong for a module that declares
    # no tools at all -- an HTTP route handler validating its own JSON body
    # names fields that no tool schema was ever meant to declare.
    tool_modules = {e["module"] for e in tools.values()}
    orphans = {m: v for m, v in orphans.items() if m in tool_modules}

    return tools, {
        "files": files_scanned,
        "tools": len(tools),
        "call_sites": call_sites,
        "orphan_rejections": orphans,
    }
