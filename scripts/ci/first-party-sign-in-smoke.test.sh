#!/usr/bin/env bash
# ABOUTME: Tests for first-party-sign-in-smoke.sh against a local stub of /oauth2/authorize
# ABOUTME: Proves a refused origin fails and is named, a serving-only origin fails unrequested, and gcloud failures fail
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The stub answers the way the server does (crates/pierre-routes-identity
# src/oauth2.rs handle_authorization): a redirect_uri on an accepted origin, or
# the mobile callback, gets 303 to a relative /oauth2/login?client_id=…; any
# other redirect_uri gets the 400 "Invalid redirect_uri" page. MODE switches it
# to the failures a probe must not mistake for a pass. A stub `gcloud` on PATH
# stands in for `run services describe`, so the default path (origins read
# from the live api and frontend services) runs too.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SMOKE="$SCRIPT_DIR/first-party-sign-in-smoke.sh"
TMP="$(mktemp -d)"
STUB_PID=""
cleanup() {
    if [ -n "$STUB_PID" ]; then
        kill "$STUB_PID" 2>/dev/null
        wait "$STUB_PID" 2>/dev/null
    fi
    rm -rf "$TMP"
}
trap cleanup EXIT

PASSED=0
FAILED=0

cat > "$TMP/stub.py" <<'PY'
import os
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlsplit

state_dir = sys.argv[1]
ACCEPTED = {"http://accepted-one.test", "http://accepted-two.test", "http://accepted-three.test"}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        query = parse_qs(urlsplit(self.path).query)
        client = query.get("client_id", [""])[0]
        redirect = query.get("redirect_uri", [""])[0]
        with open(os.path.join(state_dir, "requests"), "a", encoding="utf-8") as log:
            log.write(f"{client} {redirect}\n")
        with open(os.path.join(state_dir, "mode"), encoding="utf-8") as handle:
            mode = handle.read().strip()

        if mode == "unavailable":
            self.reply(503, "upstream unavailable")
            return
        if mode == "ok-page":
            self.reply(200, "<html>an SPA fallback page</html>")
            return
        if mode == "wrong-redirect":
            self.send_response(303)
            self.send_header("Location", "/login-error")
            self.end_headers()
            return

        origin = redirect[: -len("/auth/callback")] if redirect.endswith("/auth/callback") else None
        accepted = (client == "dravr-web" and origin in ACCEPTED) or (
            client == "dravr-mobile" and redirect == "dravr://auth/callback"
        )
        if accepted:
            self.send_response(303)
            self.send_header("Location", f"/oauth2/login?client_id={client}&redirect_uri=x")
            self.end_headers()
        else:
            self.reply(400, "<h1>Connection Failed</h1><p>Invalid redirect_uri</p>")

    def reply(self, status, body):
        data = body.encode()
        self.send_response(status)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


server = HTTPServer(("127.0.0.1", 0), Handler)
with open(os.path.join(state_dir, "port"), "w", encoding="utf-8") as handle:
    handle.write(str(server.server_address[1]))
server.serve_forever()
PY

echo accept > "$TMP/mode"
python3 -I "$TMP/stub.py" "$TMP" &
STUB_PID=$!
for _ in $(seq 1 50); do
    [ -s "$TMP/port" ] && break
    sleep 0.1
done
if [ ! -s "$TMP/port" ]; then
    echo "❌ the stub server did not start"
    exit 1
fi
ENDPOINT="http://127.0.0.1:$(cat "$TMP/port")"

# The stub gcloud prints the fixture for the service it is asked about, from
# $TMP/gcloud/<service>.json, and exits with $TMP/gcloud/exit (0 by default).
# The api fixture carries a secret reference whose name must never be printed.
mkdir -p "$TMP/bin" "$TMP/gcloud"
cat > "$TMP/bin/gcloud" <<'SH'
#!/usr/bin/env bash
dir="$(dirname "$0")/../gcloud"
[ "$1 $2 $3" = "run services describe" ] || { echo "stub gcloud: unexpected $*" >&2; exit 64; }
cat "$dir/$4.json" 2>/dev/null || { echo "stub gcloud: no service $4" >&2; exit 1; }
exit "$(cat "$dir/exit" 2>/dev/null || echo 0)"
SH
chmod +x "$TMP/bin/gcloud"
cat > "$TMP/gcloud/stub-api.json" <<'JSON'
{"spec": {"template": {"spec": {"containers": [{"env": [
  {"name": "FRONTEND_URL", "value": "http://accepted-one.test"},
  {"name": "BASE_URL", "value": "http://accepted-one.test/"},
  {"name": "ALLOWED_MOBILE_REDIRECT_ORIGINS", "value": "http://accepted-two.test,http://accepted-three.test"},
  {"name": "JWT_SECRET", "valueFrom": {"secretKeyRef": {"name": "stub-secret-ref-name", "key": "latest"}}}
]}]}}}}
JSON
# frontend_fixture <url>... : the frontend service, served on those URLs
frontend_fixture() {
    python3 -I -c 'import json, sys; print(json.dumps({"metadata": {"annotations": {"run.googleapis.com/urls": json.dumps(sys.argv[1:])}}, "status": {"url": (sys.argv[1:] or [""])[0]}}))' "$@" \
        > "$TMP/gcloud/stub-frontend.json"
}

fail() {
    echo "❌ $1"
    printf '%s\n' "$2" | sed 's/^/     /'
    FAILED=$((FAILED + 1))
}

# run_smoke <mode> <origins> -> sets OUT and STATUS
run_smoke() {
    echo "$1" > "$TMP/mode"
    : > "$TMP/requests"
    OUT="$(SIGN_IN_SMOKE_ENDPOINT="$ENDPOINT" SIGN_IN_SMOKE_ORIGINS="$2" \
        SIGN_IN_SMOKE_ATTEMPTS=3 SIGN_IN_SMOKE_RETRY_DELAY=0 "$SMOKE" 2>&1)"
    STATUS=$?
}

# expect <name> <want exit> <want text> [<unwanted text>]
expect() {
    local name="$1" want_exit="$2" want_text="$3" unwanted="${4:-}"
    if [ "$STATUS" -ne "$want_exit" ]; then
        fail "$name: expected exit $want_exit, got $STATUS" "$OUT"
        return
    fi
    if ! grep -qF -- "$want_text" <<<"$OUT"; then
        fail "$name: output missing '$want_text'" "$OUT"
        return
    fi
    if [ -n "$unwanted" ] && grep -qF -- "$unwanted" <<<"$OUT"; then
        fail "$name: output unexpectedly contains '$unwanted'" "$OUT"
        return
    fi
    echo "✅ $name"
    PASSED=$((PASSED + 1))
}

# run_live <mode>: the default path, origins read through the stub gcloud
run_live() {
    echo "$1" > "$TMP/mode"
    : > "$TMP/requests"
    OUT="$(PATH="$TMP/bin:$PATH" SIGN_IN_SMOKE_ENDPOINT="$ENDPOINT" SIGN_IN_SMOKE_SERVICE=stub-api \
        SIGN_IN_SMOKE_ATTEMPTS=3 SIGN_IN_SMOKE_RETRY_DELAY=0 "$SMOKE" 2>&1)"
    STATUS=$?
}

expect_requests() {
    local name="$1" pattern="$2" want="$3" got
    got="$(grep -c -- "$pattern" "$TMP/requests" || true)"
    if [ "$got" -ne "$want" ]; then
        fail "$name: expected $want request(s) matching '$pattern', got $got" "$(cat "$TMP/requests")"
        return
    fi
    echo "✅ $name"
    PASSED=$((PASSED + 1))
}

run_smoke accept "http://accepted-one.test, http://accepted-two.test/ http://ACCEPTED-ONE.test:80"
expect "every accepted origin passes, duplicates probed once" 0 "3/3 sign-in checks passed (2 web origin(s) + mobile, 0 serving origin(s)"
expect_requests "mobile probe sends the app callback" "^dravr-mobile dravr://auth/callback$" 1

# The 2026-10-07 regression: one origin the app is served from, refused.
run_smoke accept "http://accepted-one.test http://refused.test"
expect "a refused origin fails the run and is named" 1 "dravr-web on http://refused.test: 400 Invalid redirect_uri"
expect "the fix names the terraform variable" 1 "frontend_previous_origins"
expect_requests "a 400 is never retried" "refused.test" 1

run_smoke unavailable "http://accepted-one.test"
expect "a 5xx fails after its retries" 1 "HTTP 503 from the server"
expect_requests "a 5xx is retried up to the attempt budget" "accepted-one.test" 3

run_smoke ok-page "http://accepted-one.test"
expect "a 200 page is not a sign-in redirect" 1 "HTTP 200, expected 303"

run_smoke wrong-redirect "http://accepted-one.test"
expect "a 303 elsewhere than the login page fails" 1 "303 to '${ENDPOINT}/login-error'"

run_smoke accept "not-a-url ftp://nope.test"
expect "no usable origin fails rather than passing vacuously" 1 "No origin to probe" "passed"

# The default path: accepted origins from the api's env (FRONTEND_URL, BASE_URL
# as the issuer, the comma-separated previous origins), serving origins from
# the frontend's run.googleapis.com/urls.
frontend_fixture "http://accepted-two.test" "http://accepted-three.test"
run_live accept
expect "live env: every origin read and probed, no secret printed" 0 "6/6 sign-in checks passed (3 web origin(s) + mobile, 2 serving origin(s)" "stub-secret-ref-name"
expect_requests "live env: FRONTEND_URL and the issuer are one origin, probed once" "^dravr-web http://accepted-one.test/auth/callback$" 1
expect_requests "live env: each previous origin is probed" "^dravr-web http://accepted-three.test/auth/callback$" 1

# carnet#1152: an origin the frontend serves that the api does not accept.
frontend_fixture "http://accepted-two.test" "http://served-only.test"
run_live accept
expect "a serving origin missing from the accept-list fails by name" 1 "dravr-web on http://served-only.test: serves the app but is not an accepted origin"
expect_requests "a serving-only origin is never requested" "served-only.test" 0

# gcloud failing must fail the run with its own message, even when what it
# printed parses.
frontend_fixture "http://accepted-two.test"
echo 1 > "$TMP/gcloud/exit"
run_live accept
expect "a failing gcloud fails the run though its output parses" 1 "gcloud run services describe stub-api failed" "passed"
expect_requests "a failing gcloud sends no request" "." 0
rm -f "$TMP/gcloud/exit"

rm -f "$TMP/gcloud/stub-frontend.json"
run_live accept
expect "a frontend that cannot be described fails the run" 1 "gcloud run services describe stub-frontend failed" "passed"

frontend_fixture
run_live accept
expect "a frontend with no URL fails rather than passing vacuously" 1 "No serving origin" "passed"

echo
echo "first-party-sign-in-smoke: ${PASSED} passed, ${FAILED} failed"
[ "$FAILED" -eq 0 ]
