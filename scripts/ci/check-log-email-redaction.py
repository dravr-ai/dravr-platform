#!/usr/bin/env python3
# ABOUTME: Fails when an info!/warn!/error! macro interpolates an email-named value without mask_email
# ABOUTME: Reads whole macro invocations (multi-line, comments stripped), so a wrapped log line cannot hide
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
"""Log redaction gate for email addresses.

The policy (docs/coding-standards.md "Logging hygiene"): an email address is PII, so it is
DEBUG-level or redacted with ``pierre_middleware::redaction::mask_email`` at
INFO and above. The line-oriented ``rg`` checks beside this one in
``architectural-validation.sh`` cannot enforce that, because rustfmt wraps any
log call with more than a couple of arguments, which puts the ``user.email``
argument on a line that no longer starts with ``info!``.

So this parses each ``info!``/``warn!``/``error!`` invocation to its closing
parenthesis, with comments blanked out first, and splits it into top-level
arguments. An argument is a finding when, after every ``mask_email(...)`` call
in it is removed, it still names an identifier whose last word is ``email``
(``email``, ``user_email``, ``admin.email``; not ``email_verified``):

* a field value (``email = %user.email``), never the field name on the left;
* a positional format argument (``"{}", user_email``) or a shorthand field
  (``%email``, ``email``);
* an inline format capture in any string literal (``"{email}"``).

An identifier containing ``mask`` (``masked_email``) is already redacted, and
one immediately followed by ``.is_some()``/``.is_none()``/``.is_empty()`` or
``.len()`` logs a boolean or a length, not the address. ``debug!``/``trace!``
are the permitted home for a raw address and are not read.

What it cannot see: an address held in a variable whose name does not say
``email`` (``to``, ``recipient``) or folded into another value first. Those
are review territory; the gate covers the shape every leak it was written for
had.

Exit status: 0 clean, 1 findings, 2 when the scan read no log macro at all or
could not close one, since a scan that verified nothing is not a pass.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

MACRO = re.compile(r"(?<![A-Za-z0-9_])(?:(?:tracing|log)::)?(info|warn|error)!\s*\(")
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
CAPTURE = re.compile(r"\{([A-Za-z_][A-Za-z0-9_]*)(?::[^{}]*)?\}")
SCALAR_SUFFIX = re.compile(r"\s*\.\s*(?:is_some|is_none|is_empty|len)\s*\(\s*\)")
EMAIL_NAME = re.compile(r"(?:^|_)emails?$")
MASK_CALL = re.compile(r"(?<![A-Za-z0-9_])mask_email\s*\(")
RAW_OPEN = re.compile(r'r(#*)"')
CHAR_LITERAL = re.compile(r"'(?:\\u\{[0-9A-Fa-f]+\}|\\.|[^\\'])'")


def opens_literal(text: str, i: int) -> bool:
    """Whether a string, raw string or char literal (or a lifetime) starts at ``i``."""
    c = text[i]
    return c in "\"'" or (c == "r" and RAW_OPEN.match(text, i) is not None)


def blank_comments(source: str) -> str:
    """Replace every comment with spaces, keeping strings and line breaks intact."""
    out: list[str] = []
    i, n = 0, len(source)
    while i < n:
        c = source[i]
        nxt = source[i + 1] if i + 1 < n else ""
        if c == "/" and nxt == "/":
            end = source.find("\n", i)
            end = n if end == -1 else end
            out.append(" " * (end - i))
            i = end
        elif c == "/" and nxt == "*":
            depth, j = 1, i + 2
            while j < n and depth:
                if source.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif source.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            out.append(re.sub(r"[^\n]", " ", source[i:j]))
            i = j
        elif opens_literal(source, i):
            end = skip_literal(source, i)
            out.append(source[i:end])
            i = end
        else:
            out.append(c)
            i += 1
    return "".join(out)


def skip_literal(source: str, i: int) -> int:
    """Index just past the string, raw string or char literal at ``i``.

    A ``'`` that does not open a char literal is a lifetime and is consumed alone.
    """
    n = len(source)
    if source[i] == "r":
        raw = RAW_OPEN.match(source, i)
        if raw is None:
            return i + 1
        close = '"' + raw.group(1)
        end = source.find(close, raw.end())
        return n if end == -1 else end + len(close)
    if source[i] == "'":
        m = CHAR_LITERAL.match(source, i)
        return m.end() if m else i + 1
    j = i + 1
    while j < n:
        if source[j] == "\\":
            j += 2
        elif source[j] == '"':
            return j + 1
        else:
            j += 1
    return n


def macro_body(source: str, open_paren: int) -> int | None:
    """Index of the parenthesis closing the one at ``open_paren``, or None."""
    depth, i, n = 1, open_paren + 1, len(source)
    while i < n:
        c = source[i]
        if opens_literal(source, i):
            i = skip_literal(source, i)
            continue
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return None


def split_args(body: str) -> list[str]:
    """Top-level comma-separated arguments of a macro body."""
    args, depth, start, i, n = [], 0, 0, 0, len(body)
    while i < n:
        c = body[i]
        if opens_literal(body, i):
            i = skip_literal(body, i)
            continue
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif c == "," and depth == 0:
            args.append(body[start:i])
            start = i + 1
        i += 1
    args.append(body[start:])
    return [a.strip() for a in args if a.strip()]


def strip_mask_calls(arg: str) -> str:
    """Remove every ``mask_email(...)`` call, arguments included."""
    while (m := MASK_CALL.search(arg)) is not None:
        close = macro_body(arg, m.end() - 1)
        if close is None:
            return arg[: m.start()]
        arg = arg[: m.start()] + arg[close + 1 :]
    return arg


def literals_and_code(arg: str) -> tuple[list[str], str]:
    """Split an argument into its string literals and the code around them."""
    literals, code, i, n = [], [], 0, len(arg)
    while i < n:
        c = arg[i]
        if c == '"' or (c == "r" and RAW_OPEN.match(arg, i) is not None):
            end = skip_literal(arg, i)
            literals.append(arg[i:end])
            code.append(" ")
            i = end
        elif c == "'":
            end = skip_literal(arg, i)
            code.append(" ")
            i = end
        else:
            code.append(c)
            i += 1
    return literals, "".join(code)


def is_email_name(name: str) -> bool:
    """An identifier that holds an address: ``email``, ``user_email``, ``emails``.

    ``email`` must be its last word, so ``email_verified`` and ``email_service``
    are not addresses; one naming ``mask`` (``masked_email``) is already redacted.
    """
    lowered = name.lower()
    return EMAIL_NAME.search(lowered) is not None and "mask" not in lowered


def field_value(arg: str) -> str:
    """The value side of ``name = value``; the whole argument otherwise."""
    m = re.match(r"^[A-Za-z_][A-Za-z0-9_.]*\s*=(?![=>])", arg)
    return arg[m.end() :] if m else arg


def leaked_names(arg: str) -> list[str]:
    if re.match(r"^(?:target|parent|name)\s*:", arg):
        return []
    literals, code = literals_and_code(strip_mask_calls(field_value(arg)))
    names: list[str] = []
    for literal in literals:
        text = literal.replace("{{", "").replace("}}", "")
        names += [m.group(1) for m in CAPTURE.finditer(text) if is_email_name(m.group(1))]
    for m in IDENT.finditer(code):
        if is_email_name(m.group(0)) and not SCALAR_SUFFIX.match(code, m.end()):
            names.append(m.group(0))
    return names


def scan(root: Path) -> tuple[list[str], int, list[str]]:
    findings: list[str] = []
    unclosed: list[str] = []
    macros = 0
    for path in sorted(root.glob("crates/*/src/**/*.rs")):
        source = blank_comments(path.read_text(encoding="utf-8"))
        rel = path.relative_to(root)
        for m in MACRO.finditer(source):
            line = source.count("\n", 0, m.start()) + 1
            close = macro_body(source, m.end() - 1)
            if close is None:
                unclosed.append(f"{rel}:{line}")
                continue
            macros += 1
            for arg in split_args(source[m.end() : close]):
                for name in leaked_names(arg):
                    findings.append(
                        f"{rel}:{line}: {m.group(1)}! logs '{name}' without mask_email"
                    )
    return findings, macros, unclosed


def main() -> int:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    findings, macros, unclosed = scan(root)
    if unclosed:
        print("Could not find the end of these log macros, so they were not checked:")
        print("\n".join(f"  {u}" for u in unclosed))
        return 2
    if macros == 0:
        print(f"No info!/warn!/error! macro found under {root}/crates/*/src — nothing was verified.")
        return 2
    if findings:
        print("\n".join(findings))
        print(
            f"{len(findings)} email value(s) logged at INFO or above without "
            "pierre_middleware::redaction::mask_email (or move the field to debug!)."
        )
        return 1
    print(f"Checked {macros} info!/warn!/error! macros: no unmasked email value.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
