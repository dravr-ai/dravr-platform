#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Tests for check-eas-submit-config.sh — proves it rejects the exact
# ABOUTME: eas.json that failed CI on 2026-08-21 and accepts the fixed one.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-eas-submit-config.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

PASSED=0
FAILED=0

fixture() {
    printf '%s' "$2" > "$TMP/$1.json"
}

fail() {
    echo "❌ $1"
    echo "$2" | sed 's/^/     /'
    FAILED=$((FAILED + 1))
}

expect() {
    local name="$1" fixture="$2" want_exit="$3" want_text="$4"
    local out status
    out="$("$CHECK" "$TMP/$fixture.json" 2>&1)"
    status=$?
    if [ "$status" -ne "$want_exit" ]; then
        fail "$name: expected exit $want_exit, got $status" "$out"
        return
    fi
    if ! grep -qF "$want_text" <<<"$out"; then
        fail "$name: output missing '$want_text'" "$out"
        return
    fi
    echo "✅ $name"
    PASSED=$((PASSED + 1))
}

expect_absent() {
    local name="$1" fixture="$2" unwanted="$3"
    local out
    out="$("$CHECK" "$TMP/$fixture.json" 2>&1)"
    if grep -qF "$unwanted" <<<"$out"; then
        fail "$name: output unexpectedly contains '$unwanted'" "$out"
        return
    fi
    echo "✅ $name"
    PASSED=$((PASSED + 1))
}

# The literal shape that shipped in 21c5e6a94 and failed both dispatch runs on
# 2026-08-21, with the secrets correctly set.
fixture broken '{
  "submit": {
    "production": {
      "ios": {
        "appleId": "${APPLE_ID}",
        "ascAppId": "${ASC_APP_ID}",
        "appleTeamId": "${APPLE_TEAM_ID}"
      },
      "android": {
        "serviceAccountKeyPath": "${GOOGLE_SERVICE_ACCOUNT_KEY_PATH}",
        "track": "internal"
      }
    }
  }
}'

fixture fixed '{
  "submit": {
    "preview": { "extends": "production" },
    "production": {
      "ios": { "ascAppId": "6803245011" },
      "android": {
        "serviceAccountKeyPath": "${GOOGLE_SERVICE_ACCOUNT_KEY_PATH}",
        "track": "internal"
      }
    }
  }
}'

# The three iOS fields EAS does interpolate must stay allowed, or the check
# would push people off the credential path Expo documents for CI.
fixture asc_api_key '{
  "submit": {
    "production": {
      "ios": {
        "ascAppId": "6803245011",
        "ascApiKeyPath": "${ASC_API_KEY_PATH}",
        "ascApiKeyId": "$ASC_API_KEY_ID",
        "ascApiKeyIssuerId": "${ASC_API_KEY_ISSUER_ID}"
      }
    }
  }
}'

# Bare "$VAR" is substituted by env-string exactly like "${VAR}", so it is
# equally broken on a non-interpolated field.
fixture bare_dollar '{
  "submit": {
    "production": {
      "ios": { "ascAppId": "$ASC_APP_ID" }
    }
  }
}'

fixture no_submit '{ "build": { "production": { "autoIncrement": true } } }'

# The build job rewrites eas.json with jq before building, so a malformed file
# is reachable and should read as one line, not a Python traceback.
fixture malformed 'not json'

expect        "rejects the 2026-08-21 config"        broken      1 "submit.production.ios.appleId"
expect        "names ascAppId as a violation too"    broken      1 "submit.production.ios.ascAppId"
expect        "names appleTeamId as a violation too" broken      1 "submit.production.ios.appleTeamId"
expect_absent "spares the interpolated Android path" broken        "submit.production.android"
expect        "accepts the fixed config"             fixed       0 "submit config OK"
expect        "accepts ASC API key placeholders"     asc_api_key 0 "submit config OK"
expect        "rejects the bare-dollar form"         bare_dollar 1 "submit.production.ios.ascAppId"
expect        "accepts eas.json with no submit"      no_submit   0 "0 platform blocks checked"
expect        "reports malformed JSON in one line"    malformed   1 "is not valid JSON"
expect_absent "reports it without a traceback"        malformed     "Traceback"

echo ""
echo "Passed: $PASSED  Failed: $FAILED"
[ "$FAILED" -eq 0 ]
