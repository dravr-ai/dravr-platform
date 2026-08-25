#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Compile-free check that eas.json submit profiles only put "$VAR" in
# ABOUTME: the fields EAS actually interpolates from the environment.

# WHY THIS EXISTS
# ---------------
# EAS substitutes environment variables into exactly four submit fields, and
# silently passes every other field through as a literal string. The list lives
# in @expo/eas-json, build/submit/types.js:
#
#   IosSubmitProfileFieldsToEvaluate     = [ascApiKeyPath, ascApiKeyId,
#                                           ascApiKeyIssuerId]
#   AndroidSubmitProfileFieldsToEvaluate = [serviceAccountKeyPath]
#
# Anything else — appleId, ascAppId, appleTeamId — reaches Apple's validator as
# the characters "${APPLE_ID}" and is rejected as a malformed credential, not as
# a missing one. That failure is expensive to read: it names the credential as
# invalid, so it looks like a wrong secret rather than a config that cannot work
# no matter what the secret holds.
#
# Recurrence: 21c5e6a94 (2026-05-06) wired appleId/ascAppId/appleTeamId to
# "${APPLE_ID}"-style placeholders and passed the matching secrets from CI. The
# secrets were correct and present. Both TestFlight dispatch runs on 2026-08-21
# (32488057011, 32490321542) still failed on all three fields, and the follow-up
# commit a38a161854ab diagnosed it as credentials never being passed — deleting
# a workflow on the strength of a root cause that was not the root cause.
#
# USAGE
#   check-eas-submit-config.sh [EAS_JSON_PATH]
#     defaults to frontend-mobile/eas.json relative to the repo root.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
EAS_JSON="${1:-$PROJECT_ROOT/frontend-mobile/eas.json}"

if [ ! -f "$EAS_JSON" ]; then
    echo "❌ eas.json not found at: $EAS_JSON"
    exit 1
fi

python3 - "$EAS_JSON" <<'PY'
import json
import re
import sys

# Mirrors @expo/eas-json build/submit/types.js. Re-check it against the pinned
# eas-cli when bumping: a field added there may be interpolated here too.
INTERPOLATED = {
    "ios": {"ascApiKeyPath", "ascApiKeyId", "ascApiKeyIssuerId"},
    "android": {"serviceAccountKeyPath"},
}

# env-string, which EAS uses, accepts both "$VAR" and "${VAR}" and honours a
# leading backslash as an escape. Match the unescaped forms only.
PLACEHOLDER = re.compile(r"(?<!\\)\$\{?[A-Za-z0-9_]+\}?")

path = sys.argv[1]
try:
    with open(path) as fh:
        eas = json.load(fh)
except json.JSONDecodeError as err:
    print(f"❌ {path} is not valid JSON: {err}")
    sys.exit(1)

violations = []
for profile_name, profile in eas.get("submit", {}).items():
    for platform in ("ios", "android"):
        for field, value in (profile.get(platform) or {}).items():
            if not isinstance(value, str):
                continue
            if field in INTERPOLATED[platform]:
                continue
            match = PLACEHOLDER.search(value)
            if match:
                violations.append((profile_name, platform, field, match.group(0)))

if violations:
    print("❌ eas.json submit profiles interpolate fields EAS does not substitute:")
    print()
    for profile_name, platform, field, placeholder in violations:
        allowed = ", ".join(sorted(INTERPOLATED[platform]))
        print(f'   submit.{profile_name}.{platform}.{field} = "{placeholder}"')
        print(f"      EAS interpolates only: {allowed}")
        print(f"      This value reaches the store API as the literal text "
              f'"{placeholder}" and is rejected as a malformed credential.')
        print()
    print("   Fix: write the value literally, or move the credential onto one of")
    print("   the interpolated fields (an App Store Connect API key for iOS).")
    sys.exit(1)

checked = sum(
    1
    for profile in eas.get("submit", {}).values()
    for platform in ("ios", "android")
    if profile.get(platform)
)
print(f"✅ eas.json submit config OK ({checked} platform blocks checked)")
PY
