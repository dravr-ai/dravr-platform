# ABOUTME: Log entries the _Default sink never stores: Google's own request logs of a URL that carries a bearer secret
# ABOUTME: Our logs redact those secrets; the load balancer and Cloud Run request logs record the full URL and cannot be redacted (carnet#1641, carnet#1873)
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

# Garmin pushes to /webhooks/garmin/{token}/{summaryType}, and the token is the
# only secret a forged push must guess (carnet#1155). nginx logs the path with
# the token replaced by [redacted] and the API never logs it, but three logs
# Google writes itself store httpRequest.requestUrl whole: the load balancer's
# and the Cloud Run request log of the frontend and of the API. Anyone who can
# read logs in this project could read the token there and forge a push.
#
# Those entries are dropped before storage. Each push stays observable in the
# frontend's nginx access log (redacted path, status, upstream), and no
# log-based metric or alert reads the dropped entries.
resource "google_logging_project_exclusion" "garmin_push_request_urls" {
  project     = var.project_id
  name        = "garmin-push-request-urls"
  description = "Request logs of Garmin pushes: their URL carries the webhook token (carnet#1641)"
  filter      = "(resource.type=\"http_load_balancer\" OR log_id(\"run.googleapis.com/requests\")) AND httpRequest.requestUrl:\"/webhooks/garmin/\""
}

# The same three Google logs store every other URL that carries a bearer
# secret, and these cannot be redacted either (carnet#1873). Each resource
# below names its secret, how long it stays usable and why it is in the URL.
# Each filter is anchored on the host and the route, so the routes beside it
# stay logged: /providers/connect/import/{source}/... carries its token as a
# header and has no token= query, /api/messaging/link/* has no code in the
# path, /auth/callback and /oauth2/login/google/callback are kept because
# their code is worthless without a PKCE verifier only the browser holds, and
# /api/oauth/callback/{provider} is kept because its state is an opaque key to
# a server-side row and its code needs our client secret (the OAuth launch
# runbook in monitoring.tf pairs these callbacks with their authorize hits).
# An entry whose route is listed in the parity test in
# crates/pierre-middleware/src/redaction.rs (SECRET_BEARING_URLS) must keep a
# filter here that matches it.
#
# Hosted provider pages read a link-token from the `token` query: the connect
# token (60 min, never burned, any provider's OAuth for the athlete) on
# /providers/connect*, /api/providers/connect/oauth-init/{provider} and
# /providers/connect/import?token=, and the provider sign-in token (24 h, and
# a valid Bearer on the sign-in POSTs for all 24 h) on /providers/signin*,
# /providers/sciotte/login. One filter covers both families: every page sits
# under /providers/ with a token= query.
resource "google_logging_project_exclusion" "provider_link_token_request_urls" {
  project     = var.project_id
  name        = "provider-link-token-request-urls"
  description = "Request logs of hosted provider pages: their token= query is a connect (60 min) or sign-in (24 h) link-token (carnet#1873)"
  filter      = "(resource.type=\"http_load_balancer\" OR log_id(\"run.googleapis.com/requests\")) AND (httpRequest.requestUrl=~\"^https?://[^/]+(/api)?/providers/[^?]*[?](.*&)?token=\")"
}

# /r/{code} is a short link: the 32-hex code redirects, on every GET for 24 h,
# to a /providers URL carrying a connect (1 h) or sign-in (24 h) link-token,
# so the code alone yields the token.
resource "google_logging_project_exclusion" "short_link_request_urls" {
  project     = var.project_id
  name        = "short-link-request-urls"
  description = "Request logs of /r/{code} short links: the code resolves to a link-token URL for 24 h (carnet#1873)"
  filter      = "(resource.type=\"http_load_balancer\" OR log_id(\"run.googleapis.com/requests\")) AND (httpRequest.requestUrl=~\"^https?://[^/]+/r/[0-9a-f]{32}\")"
}

# The provider sign-in takeover token (10 min, a live view of and input into
# the athlete's parked sign-in browser) rides the socket's
# Sec-WebSocket-Protocol header, and the server refuses one in the query. A
# page loaded before that change still sends ?token=; the refusal does not
# keep its URL out of the request logs, so those entries are dropped.
resource "google_logging_project_exclusion" "takeover_socket_request_urls" {
  project     = var.project_id
  name        = "takeover-socket-request-urls"
  description = "Request logs of the sign-in takeover socket: its token= query is a 10 min takeover token (carnet#1873)"
  filter      = "(resource.type=\"http_load_balancer\" OR log_id(\"run.googleapis.com/requests\")) AND (httpRequest.requestUrl:\"/ws/providers/sciotte/takeover?\")"
}

# /messaging/link/{code} is the page a chat sender opens to bind their chat
# identity to an account; the 32-character code (10 min) plus any account
# does the binding. The form's own POST, /messaging/link/auth, and every
# /api/messaging/link/* route stay logged.
resource "google_logging_project_exclusion" "messaging_link_request_urls" {
  project     = var.project_id
  name        = "messaging-link-request-urls"
  description = "Request logs of /messaging/link/{code}: the code binds a chat identity for 10 min (carnet#1873)"
  filter      = "(resource.type=\"http_load_balancer\" OR log_id(\"run.googleapis.com/requests\")) AND (httpRequest.requestUrl=~\"^https?://[^/]+/messaging/link/[A-Za-z0-9]{32}([/?]|$)\")"
}

# /api/viz/{token}.png serves a chart from an athlete's conversation to a
# channel's media fetcher, which can only GET a URL; the HMAC token returns it
# to anyone for 15 min.
resource "google_logging_project_exclusion" "viz_request_urls" {
  project     = var.project_id
  name        = "viz-request-urls"
  description = "Request logs of /api/viz/{token}.png: the token serves an athlete's chart for 15 min (carnet#1873)"
  filter      = "(resource.type=\"http_load_balancer\" OR log_id(\"run.googleapis.com/requests\")) AND (httpRequest.requestUrl:\"/api/viz/\")"
}

# A group invite code joins its group, multi-use, until the creator's expiry
# or never; a coach signup invite (1-30 days) also approves the account. It
# rides /groups/join/{code} and the preview /api/invites/{code}. The SPA is
# served under Referrer-Policy strict-origin, so its requests name only the
# origin as their Referer; a client that still sends the full /groups/join
# page as its Referer is matched on that field too.
resource "google_logging_project_exclusion" "group_invite_request_urls" {
  project     = var.project_id
  name        = "group-invite-request-urls"
  description = "Request logs carrying a group invite code in the URL or the Referer: multi-use, long-lived (carnet#1873)"
  filter      = "(resource.type=\"http_load_balancer\" OR log_id(\"run.googleapis.com/requests\")) AND (httpRequest.requestUrl=~\"^https?://[^/]+(/api/invites|/groups/join)/\" OR httpRequest.referer=~\"^https?://[^/]+/groups/join/\")"
}

# /signup?code= and /register?code= carry a signup code that approves the
# account redeeming it (admin code: up to 365 days or never, multi-use; coach
# code: 1-30 days). The SPA reads it from #code=, which no request carries,
# and still accepts ?code= for links shared in that shape: those carry it in
# the URL, and in the Referer of a client that does not apply the SPA's
# strict-origin policy, so both fields are matched.
resource "google_logging_project_exclusion" "signup_code_request_urls" {
  project     = var.project_id
  name        = "signup-code-request-urls"
  description = "Request logs carrying a signup code in the URL or the Referer: it approves the account that redeems it (carnet#1873)"
  filter      = "(resource.type=\"http_load_balancer\" OR log_id(\"run.googleapis.com/requests\")) AND (httpRequest.requestUrl=~\"^https?://[^/]+/(signup|register)[?](.*&)?code=\" OR httpRequest.referer=~\"^https?://[^/]+/(signup|register)[?](.*&)?code=\")"
}

# A webhook subscription check sends hub.verify_token in the query: the
# channel's verify token, static until rotated, which a forged subscription
# handshake needs. The Meta handshake compares only a stored verify token,
# never the channel's webhook secret (the Meta app secret that signs inbound
# webhooks), but a misconfigured Meta dashboard still sends whatever it holds,
# so every verification, the Strava handshake included, is dropped.
resource "google_logging_project_exclusion" "webhook_verify_token_request_urls" {
  project     = var.project_id
  name        = "webhook-verify-token-request-urls"
  description = "Request logs of webhook verifications: hub.verify_token is a static channel secret (carnet#1873)"
  filter      = "(resource.type=\"http_load_balancer\" OR log_id(\"run.googleapis.com/requests\")) AND (httpRequest.requestUrl:\"hub.verify_token=\")"
}

# Firebase's sign-in redirect lands on /__/auth/handler with Google's
# authorization code and Firebase's state in the query; nginx proxies /__/ to
# Firebase Hosting. The code is single-use and short-lived, and Firebase
# exchanges it within the second, but its exchange runs on Google's side with
# Firebase's own client secret, so nothing here proves a copy of it is useless.
# Measured 2026-11-27: 25 such requests in a week, stored on the load balancer
# and the frontend. The handler's other pages carry no code.
resource "google_logging_project_exclusion" "firebase_auth_handler_request_urls" {
  project     = var.project_id
  name        = "firebase-auth-handler-request-urls"
  description = "Request logs of Firebase's sign-in redirect: its query carries Google's authorization code (carnet#1873)"
  filter      = "(resource.type=\"http_load_balancer\" OR log_id(\"run.googleapis.com/requests\")) AND (httpRequest.requestUrl=~\"^https?://[^/]+/__/auth/handler[?](.*&)?code=\")"
}
