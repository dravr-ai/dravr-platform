#!/usr/bin/env python3
# ABOUTME: Fails when a struct or enum deriving Debug holds a secret-named field typed String
# ABOUTME: Such a field prints in full under {:?}; the type that cannot is secrecy::SecretString
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
"""Type-level gate for secrets in Debug-printing types.

A ``#[derive(Debug)]`` type prints every field under ``{:?}``, so one holding a
password, token, API key or client secret as a plain ``String`` leaks it into
any log line, panic message or error that formats the value, in any form:
inline capture, positional argument, a ``tracing`` field. A
``secrecy::SecretString`` prints a redaction, has no ``Display``, and gives its
value up only through ``expose_secret()``, so the fix is the field's type.

A field is secret-named when an ``_``-separated word of its name is
``secret``, ``token``, ``password``, ``passwd`` or ``apikey``, or the name
contains ``api_key`` or ``private_key``. Names that describe a secret without
holding one are not: a last word of ``url``, ``uri``, ``endpoint``, ``type``,
``kind``, ``id``, ``ids``, ``prefix``, ``hash``, ``fingerprint``, ``name``,
``label``, ``encrypted``, ``count``, ``len``, ``length``, ``at``, ``ttl``,
``expiry``, ``hint`` (``token_url``, ``api_key_id``, ``password_hash``), and
the pagination cursors ``page_token``, ``next_token``, ``next_page_token``.
The connection strings ``redis_url``, ``database_url`` and
``connection_string`` carry the password they connect with, so they count.

A type with a hand-written ``Debug`` impl is not read: it decides what it
prints. Fields typed ``String`` or ``Option<String>`` are findings.

Usage: ``check-secret-fields.py <repo root>``. Exit status: 0 clean (prints a
success line with the counts it verified), 1 findings, 2 when the scan read no
Debug-deriving type or no ``SecretString`` field at all, since a scan that
verified nothing is not a pass.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

SECRET_WORDS = {"secret", "token", "password", "passwd", "apikey", "pat"}
SECRET_PARTS = ("api_key", "private_key")
# Whole names that hold an OAuth or device-flow credential: the authorization
# code, its PKCE verifier, the device-flow codes and the CSRF state.
SECRET_NAMES = {
    "code", "auth_code", "authorization_code", "oauth_code", "code_verifier",
    "user_code", "device_code", "state", "oauth_state", "csrf_state",
}
DESCRIPTIVE_LAST = {
    "url", "uri", "endpoint", "type", "kind", "id", "ids", "prefix", "hash",
    "fingerprint", "name", "label", "encrypted", "count", "len", "length",
    "at", "ttl", "expiry", "hint",
}
CURSORS = {"page_token", "next_token", "next_page_token"}
# Connection strings that embed the password they connect with.
CREDENTIAL_URLS = {"redis_url", "database_url", "connection_string"}

TYPE_DECL = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(struct|enum)\s+(\w+)")
DERIVE = re.compile(r"#\[derive\(([^)]*)\)\]")
# Owned or borrowed text and bytes print in full under Debug, bare or in an Option.
TEXT_TYPE = (
    r"(?:String|Arc<\s*str\s*>|Rc<\s*str\s*>|Box<\s*str\s*>|Cow<\s*'\w+\s*,\s*str\s*>"
    r"|&\s*(?:'\w+\s+)?str|Vec<\s*u8\s*>|Bytes)"
)
FIELD = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?([a-z_][a-z0-9_]*)\s*:\s*"
    r"(Option<\s*" + TEXT_TYPE + r"\s*>|" + TEXT_TYPE + r")\s*,?\s*(?://.*)?$"
)
SECRET_TYPED = re.compile(r":\s*(?:Option<\s*)?SecretString\b")


def is_secret_name(name: str) -> bool:
    if name in CREDENTIAL_URLS:
        return True
    if name in CURSORS:
        return False
    if name in SECRET_NAMES:
        return True
    words = name.split("_")
    if words[-1] in DESCRIPTIVE_LAST:
        return False
    return bool(SECRET_WORDS.intersection(words)) or any(p in name for p in SECRET_PARTS)


def strip_comments(line: str) -> str:
    """The line without a trailing `//` comment (string contents are not parsed)."""
    at = line.find("//")
    return line if at < 0 else line[:at]


def scan_file(path: Path, root: Path, findings: list[str]) -> tuple[int, int]:
    lines = path.read_text(encoding="utf-8").split("\n")
    debug_types = 0
    secret_fields = 0
    pending_attrs: list[str] = []
    i = 0
    while i < len(lines):
        line = lines[i]
        stripped = line.strip()
        if stripped.startswith("#["):
            # an attribute may span lines (`#[derive(\n    Debug,\n)]`)
            attr = stripped
            while attr.count("[") > attr.count("]") and i + 1 < len(lines):
                i += 1
                attr += " " + lines[i].strip()
            pending_attrs.append(attr)
            i += 1
            continue
        if stripped.startswith("///") or stripped.startswith("//") or not stripped:
            i += 1
            continue
        decl = TYPE_DECL.match(line)
        if not decl:
            pending_attrs = []
            i += 1
            continue
        derives = " ".join(m.group(1) for a in pending_attrs for m in DERIVE.finditer(a))
        pending_attrs = []
        derives_debug = re.search(r"\bDebug\b", derives) is not None
        # walk the body to its closing brace (a unit or tuple type has none)
        depth = 0
        opened = False
        j = i
        while j < len(lines):
            code = strip_comments(lines[j])
            if opened and depth >= 1:
                field = FIELD.match(lines[j])
                if SECRET_TYPED.search(code):
                    secret_fields += 1
                if derives_debug and field and is_secret_name(field.group(1)):
                    rel = path.relative_to(root)
                    findings.append(
                        f"{rel}:{j + 1}: {decl.group(2)}.{field.group(1)}: {field.group(2)} "
                        f"in a Debug-deriving {decl.group(1)}"
                    )
            depth += code.count("{") - code.count("}")
            if "{" in code:
                opened = True
            if (opened and depth <= 0) or (not opened and code.rstrip().endswith(";")):
                break
            j += 1
        if derives_debug:
            debug_types += 1
        i = j + 1
    return debug_types, secret_fields


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: check-secret-fields.py <repo root>", file=sys.stderr)
        return 2
    root = Path(sys.argv[1]).resolve()
    files = sorted(root.glob("crates/*/src/**/*.rs"))
    findings: list[str] = []
    debug_types = 0
    secret_fields = 0
    for path in files:
        d, s = scan_file(path, root, findings)
        debug_types += d
        secret_fields += s
    if findings:
        for finding in findings:
            print(finding)
        print(f"{len(findings)} secret-named String field(s) in Debug-deriving types")
        return 1
    if not files or debug_types == 0 or secret_fields == 0:
        print(
            f"scan verified nothing: {len(files)} file(s), {debug_types} Debug-deriving type(s), "
            f"{secret_fields} SecretString field(s)"
        )
        return 2
    print(
        f"secret fields OK: {debug_types} Debug-deriving type(s) in {len(files)} file(s), "
        f"{secret_fields} SecretString field(s)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
