#!/usr/bin/env bash
# ABOUTME: Fails when a Maestro flow can only pass — no assertion, or every assertion behind a when: guard
# ABOUTME: A guarded assertion is skipped when its precondition is absent, and a skipped flow reports success
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# carnet#364. Four of the nine store flows asserted nothing under any
# condition and two more asserted only inside `runFlow: when:` blocks, so a
# green run meant the runner reached the end of the file. Two of those four
# were in the Android nightly.
#
# The rule this enforces: every flow must carry at least one assertion that
# runs unconditionally. Guarding SETUP is legitimate — whether an agent is
# already installed depends on what a previous run left behind — but an
# assertion inside a guard is not a guard at all.
#
# `assertVisible`, `assertNotVisible` and `extendedWaitUntil` all count: each
# fails the flow when its subject never appears.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MAESTRO_DIR="${1:-$ROOT/frontend-mobile/.maestro}"

if [[ ! -d "$MAESTRO_DIR" ]]; then
    echo "❌ Maestro directory not found: $MAESTRO_DIR" >&2
    exit 1
fi

python3 - "$MAESTRO_DIR" <<'PY'
import sys, pathlib, yaml

ASSERTIONS = {"assertVisible", "assertNotVisible", "extendedWaitUntil", "assertTrue"}
root = pathlib.Path(sys.argv[1])

def count(node, guarded=False):
    """Return (unguarded, guarded) assertion counts under this node."""
    free = held = 0
    if isinstance(node, list):
        for item in node:
            f, h = count(item, guarded)
            free += f; held += h
    elif isinstance(node, dict):
        for key, value in node.items():
            if key == "runFlow" and isinstance(value, dict) and "when" in value:
                # Everything inside a conditional block can be skipped, so
                # nothing under it is ever unconditional however deep it sits.
                f, h = count(value.get("commands", []), True)
                held += f + h
            elif key in ASSERTIONS:
                if guarded:
                    held += 1
                else:
                    free += 1
            else:
                f, h = count(value, guarded)
                free += f; held += h
    return free, held

failures = []
checked = 0
# helpers/ are fragments included by other flows and are exempt: they carry the
# waits their callers rely on, and are never run on their own.
for path in sorted(root.rglob("*.yaml")):
    if path.parent.name == "helpers" or path.name == "config.yaml":
        continue
    try:
        docs = list(yaml.safe_load_all(path.read_text()))
    except yaml.YAMLError as exc:
        failures.append((path, f"does not parse: {exc}"))
        continue
    checked += 1
    free, held = count(docs)
    if free == 0:
        detail = (
            f"{held} assertion(s), all inside a when: guard"
            if held
            else "no assertions at all"
        )
        failures.append((path, detail))

rel = lambda p: p.relative_to(root.parent.parent) if root.parent.parent in p.parents else p

if failures:
    print("❌ Maestro flows that cannot fail:\n")
    for path, why in failures:
        print(f"   {rel(path)}")
        print(f"       {why}")
    print(
        "\n   A flow with no unconditional assertion passes by reaching the end of\n"
        "   the file. Guard the setup if the device state genuinely varies, then\n"
        "   assert outside the guard. See carnet#364."
    )
    sys.exit(1)

print(f"✅ Maestro assertions: all {checked} flows carry an unconditional assertion.")
PY
