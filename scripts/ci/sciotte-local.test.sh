#!/usr/bin/env bash
# ABOUTME: Fixture test for scripts/sciotte-local.sh — what the harness sends a local sciotte server
# ABOUTME: Pins X-Session-Id on every per-athlete read, flow_id on otp/2fa, and refusal when neither is known
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# One sciotte instance serves every athlete, so it answers a per-athlete read
# only for the session named in X-Session-Id and resumes a login only by the
# flow_id its first step minted (carnet#566): a request naming neither gets a
# 401 or a 400, never the latest session. The harness drives that server one
# command per process, so what it remembers between commands and what it puts
# on the wire is the whole contract. curl is a stub on PATH that records its
# argv and stdin and answers a canned body; no server runs.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/../sciotte-local.sh}"

failures=0
pass() { echo "  ✅ $1"; }
fail() {
    echo "  ❌ $1"
    failures=$((failures + 1))
}

expect() { # label actual expected
    if [ "$2" = "$3" ]; then pass "$1"; else
        fail "$1"
        echo "      got:      $2"
        echo "      expected: $3"
    fi
}

expect_contains() { # label haystack needle
    case "$2" in *"$3"*) pass "$1" ;; *)
        fail "$1"
        echo "      output:   $2"
        echo "      missing:  $3"
        ;;
    esac
}

echo "sciotte-local.sh fixture tests"

if ! command -v jq >/dev/null 2>&1; then
    fail "jq is required by the script under test"
    echo "❌ 1 sciotte-local case(s) failed"
    exit 1
fi

root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT
stub_bin="$root/bin"
stub_dir="$root/calls"
run_dir="$root/logs"
mkdir -p "$stub_bin" "$stub_dir"

# The stub records one call: argv one per line, stdin verbatim. It answers
# $STUB_RESPONSE and exits $STUB_EXIT, so a refused connection is a case too.
cat >"$stub_bin/curl" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$@" >"$STUB_DIR/argv"
cat >"$STUB_DIR/stdin"
printf '%s' "$STUB_RESPONSE"
exit "${STUB_EXIT:-0}"
STUB
chmod +x "$stub_bin/curl"

PASSWORD="pw-sentinel-not-real"

# Run one harness command as a fresh process, the way a developer does.
# Sets rc and out (stdout + stderr); leaves the stub's record of the call.
run() { # response exit args...
    local response="$1" code="$2"
    shift 2
    rm -f "$stub_dir/argv" "$stub_dir/stdin"
    rc=0
    out="$(PATH="$stub_bin:$PATH" DEV_RUN_DIR="$run_dir" STUB_DIR="$stub_dir" \
        STUB_RESPONSE="$response" STUB_EXIT="$code" SCIOTTE_TEST_PASSWORD="$PASSWORD" \
        "$UNDER_TEST" "$@" </dev/null 2>&1)" || rc=$?
}

called() { [ -e "$stub_dir/argv" ]; }
sent_url() { grep '^http' "$stub_dir/argv" || true; }
sent_body() { jq -c . "$stub_dir/stdin"; }
sent_stdin() { cat "$stub_dir/stdin"; }
argv_has() { grep -qF -- "$1" "$stub_dir/argv"; }

BASE="http://127.0.0.1:8091"

echo "Per-athlete reads with no session known"
run '{}' 0 status
expect "status with no session exits 1" "$rc" "1"
expect_contains "status names what is missing" "$out" "no session_id known"
if called; then fail "status with no session sends nothing"; else pass "status with no session sends nothing"; fi
run '{}' 0 activities
expect "activities with no session exits 1" "$rc" "1"
if called; then fail "activities with no session sends nothing"; else pass "activities with no session sends nothing"; fi

echo "A session named as the argument"
run '{"authenticated":true,"session_id":"sess-arg"}' 0 status sess-arg
expect "status exits 0" "$rc" "0"
expect "status reads /auth/status" "$(sent_url)" "$BASE/auth/status"
expect "status names the argument's session" "$(sent_stdin)" "X-Session-Id: sess-arg"
if argv_has "sess-arg"; then fail "the session id stays out of curl's argv"; else pass "the session id stays out of curl's argv"; fi

echo "Continuations with no flow known"
run '{}' 0 otp 123456
expect "otp with no flow exits 1" "$rc" "1"
expect_contains "otp names what is missing" "$out" "no flow_id known"
if called; then fail "otp with no flow sends nothing"; else pass "otp with no flow sends nothing"; fi
run '{}' 0 2fa sms
expect "2fa with no flow exits 1" "$rc" "1"
if called; then fail "2fa with no flow sends nothing"; else pass "2fa with no flow sends nothing"; fi

echo "A login that is challenged, then continued by its flow_id"
run '{"status":"otp_required","flow_id":"flow-1","provider":"garmin"}' 0 login athlete@example.test garmin
expect "login exits 0" "$rc" "0"
expect "login posts the credential route" "$(sent_url)" "$BASE/auth/login-with-credentials"
expect "login body carries email, password, method, provider" "$(sent_body)" \
    "{\"email\":\"athlete@example.test\",\"password\":\"$PASSWORD\",\"method\":\"email\",\"provider\":\"garmin\"}"
if argv_has "$PASSWORD"; then fail "the password stays out of curl's argv"; else pass "the password stays out of curl's argv"; fi
expect "the flow is remembered" "$(cat "$run_dir/sciotte-local.flow")" "flow-1"
expect "the remembered flow is private to the user" \
    "$(find "$run_dir/sciotte-local.flow" -perm 600)" "$run_dir/sciotte-local.flow"

run '{"status":"two_factor_choice","flow_id":"flow-1","options":[]}' 0 2fa sms
expect "2fa exits 0" "$rc" "0"
expect "2fa posts /auth/select-2fa" "$(sent_url)" "$BASE/auth/select-2fa"
expect "2fa names the remembered flow" "$(sent_body)" '{"option_id":"sms","flow_id":"flow-1"}'

run '{"status":"otp_required","flow_id":"flow-1"}' 0 otp 654321 flow-explicit
expect "otp posts /auth/submit-otp" "$(sent_url)" "$BASE/auth/submit-otp"
expect "otp names the flow its argument gives" "$(sent_body)" '{"code":"654321","flow_id":"flow-explicit"}'

run '{"status":"authenticated","session_id":"sess-9","cookie_count":3,"provider":"garmin"}' 0 otp 123456
expect "otp names the remembered flow" "$(sent_body)" '{"code":"123456","flow_id":"flow-1"}'
expect "the session login ends in is remembered" "$(cat "$run_dir/sciotte-local.session")" "sess-9"
if [ -e "$run_dir/sciotte-local.flow" ]; then fail "an authenticated flow is forgotten"; else pass "an authenticated flow is forgotten"; fi
run '{}' 0 otp 111111
expect "otp after the flow concluded exits 1" "$rc" "1"

echo "Per-athlete reads name the remembered session"
run '{"id":"1"}' 0 athlete
expect "athlete reads /api/athlete" "$(sent_url)" "$BASE/api/athlete"
expect "athlete names the remembered session" "$(sent_stdin)" "X-Session-Id: sess-9"
run '{"count":0,"activities":[]}' 0 activities
expect "activities reads /api/activities" "$(sent_url)" "$BASE/api/activities"
expect "activities names the remembered session" "$(sent_stdin)" "X-Session-Id: sess-9"
run '{"authenticated":true}' 0 status
expect "status names the remembered session" "$(sent_stdin)" "X-Session-Id: sess-9"
run '{"id":"2"}' 0 athlete sess-other
expect "an argument wins over the remembered session" "$(sent_stdin)" "X-Session-Id: sess-other"

echo "A failed flow and an abandoned one are forgotten"
run '{"status":"number_match","number":"42","flow_id":"flow-2"}' 0 login athlete@example.test
expect "login provider defaults to garmin" "$(jq -r .provider "$stub_dir/stdin")" "garmin"
expect_contains "number_match is continued by polling" "$out" "2fa poll"
run '{"status":"failed","reason":"wrong code"}' 0 otp 000000
expect "the failed step named the flow" "$(sent_body)" '{"code":"000000","flow_id":"flow-2"}'
if [ -e "$run_dir/sciotte-local.flow" ]; then fail "a failed flow is forgotten"; else pass "a failed flow is forgotten"; fi
run '{"status":"otp_required","flow_id":"flow-3"}' 0 login athlete@example.test strava
run '{"error":"queue_full","message":"busy"}' 0 login athlete@example.test strava
if [ -e "$run_dir/sciotte-local.flow" ]; then fail "a new login forgets the flow it abandons"; else pass "a new login forgets the flow it abandons"; fi
expect "the session survives a login that established none" "$(cat "$run_dir/sciotte-local.session")" "sess-9"

echo "A server that does not answer"
run '' 7 athlete
expect "an unanswered read exits 1" "$rc" "1"
expect_contains "an unanswered read says so" "$out" "no answer from $BASE/api/athlete"
run '' 7 login athlete@example.test
if [ "$rc" -ne 0 ]; then pass "an unanswered login exits non-zero"; else fail "an unanswered login exits non-zero"; fi
expect_contains "an unanswered login says so" "$out" "no answer from $BASE/auth/login-with-credentials"

echo "Retired and argument-carrying commands"
run '{}' 0 sessions
expect "sessions (GET /auth/sessions is gone) is an unknown command" "$rc" "1"
expect_contains "sessions is reported unknown" "$out" "unknown command: sessions"
if called; then fail "sessions sends nothing"; else pass "sessions sends nothing"; fi

# `serve <provider>` hands the provider's config to the binary; a fixture
# sciotte checkout whose binary echoes its argv stands in for the real one.
repo="$root/dravr-sciotte"
mkdir -p "$repo/target/debug" "$repo/providers"
: >"$repo/providers/garmin.toml"
printf '#!/usr/bin/env bash\nprintf "%%s\\n" "$*"\n' >"$repo/target/debug/dravr-sciotte-server"
chmod +x "$repo/target/debug/dravr-sciotte-server"
rc=0
out="$(SCIOTTE_REPO="$repo" DEV_RUN_DIR="$run_dir" "$UNDER_TEST" serve garmin </dev/null 2>&1)" || rc=$?
expect "serve garmin exits 0" "$rc" "0"
expect_contains "serve garmin serves only the garmin config" "$out" \
    "--provider $repo/providers/garmin.toml serve --host 127.0.0.1 --port 8091"

echo ""
if [ "$failures" -ne 0 ]; then
    echo "❌ $failures sciotte-local case(s) failed"
    exit 1
fi
echo "✅ all sciotte-local cases passed"
