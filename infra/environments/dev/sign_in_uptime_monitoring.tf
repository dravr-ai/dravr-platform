# ABOUTME: Cloud Monitoring uptime checks that walk first-party sign-in on every accepted origin and public hostname, plus their alert
# ABOUTME: Catches a refused /oauth2/authorize within minutes of a deploy, before any athlete has to hit it
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Background
# ----------
# The web and mobile apps sign in through the hosted login page: the app sends
# the athlete to `/oauth2/authorize?client_id=dravr-web|dravr-mobile&
# redirect_uri=<origin>/auth/callback`, and the server accepts only the
# deployment's own callbacks (`FirstPartyRedirects`, built from FRONTEND_URL,
# the issuer and ALLOWED_MOBILE_REDIRECT_ORIGINS). From 2026-10-07 the previous
# run.app origin was missing from that list and web sign-in from it was refused
# for 16 days, until an alpha athlete reported it (carnet#1145).
#
# Why a probe on top of first_party_sign_in_monitoring.tf
# --------------------------------------------------------
# That alert counts 400s in the request log, so it can only fire after a real
# athlete has already been turned away, and an origin nobody happens to use
# that day never fires it at all. These checks request the exact URL the apps
# build, from Google's checkers every five minutes, so a refused origin pages
# without any athlete involved. A refused probe also lands a 400 in the request
# log and fires that alert too; the overlap is intended.
#
# Which origins
# -------------
# Every hostname that serves the frontend is a first-party origin
# (carnet#1152), so the probe set is the accepted set: frontend_base_url and
# local.frontend_extra_origins (main.tf), which set FRONTEND_URL and
# ALLOWED_MOBILE_REDIRECT_ORIGINS. Every public_domains hostname reaches it
# through frontend_extra_origins, since the load balancer routes each one to
# the same nginx, and so does the frontend's deterministic run.app URL. Each
# check is therefore on an accepted origin by construction, so none is red for
# a missing entry; it turns red when the server stops honouring that config,
# nginx stops proxying /oauth2/, or the api is down.
# The frontend's hash run.app URL cannot be derived from module.frontend at
# plan time: it is probed only because frontend_previous_origins lists it.
# The precondition on the alert policy asserts that every run.app URL is
# accepted, and scripts/ci/first-party-sign-in-smoke.sh checks the live
# service's URLs after every deploy.
#
# What a check asserts
# --------------------
# Uptime checks FOLLOW redirects and judge the final response ("all URL
# redirects are followed and the final response received by the uptime check
# is used", Cloud Monitoring docs), so the server's 303 cannot be asserted
# directly. The check asserts what the 303 leads to instead:
#   - accepted: the 303 lands on GET /oauth2/login, a 200 login page;
#   - refused: the hosted "Invalid redirect_uri" page, a 400, fails the status;
#   - nginx no longer proxying /oauth2/: the SPA shell, a 200, fails the
#     content matcher.
# The matcher is the login form's action, not its title or copy: the page is
# localized from Accept-Language, the form action is not
# (crates/pierre-routes-identity/templates/oauth_login.html). Changing that
# action turns these checks red, which is the right direction to fail.
#
# Load on the api
# ---------------
# Each execution is two GETs (authorize, then login). Five checks from the six
# checker locations every 300s is ~60 requests per 5 minutes in total, a handful
# per checker address, against an authorize window of 60 per minute per client
# address (OAUTH_AUTHORIZE_RATE_LIMIT_RPM). GET /oauth2/login is not metered by
# the password-sign-in windows, which count only the POST. No session is
# created: the checker carries no cookie, so it never gets past the login page.
# Each accepted authorize logs one info line ("No authenticated session for
# OAuth authorization, redirecting to login") and one request-log row: about
# 8.6k of each per day at five checks, which is the price of the coverage.

locals {
  # Probes exist only where the frontend does and has a public URL: with
  # enable_frontend off or frontend_base_url unset there is no sign-in page to
  # walk, and the regexes below would fail plan on the empty URL.
  sign_in_probe_enabled = var.enable_frontend && var.frontend_base_url != ""

  # Every origin the server accepts as first-party. The regex rejects a
  # non-https origin at plan time and drops a trailing slash.
  sign_in_accepted_origins = local.sign_in_probe_enabled ? distinct([
    for origin in concat([var.frontend_base_url], local.frontend_extra_origins) :
    "https://${regex("^https://([^/]+)/?$", origin)[0]}"
  ]) : []

  # One check per accepted host: only accepted origins are probed, and the
  # list above is already distinct, so `https://app.dravr.ai` and
  # `https://app.dravr.ai/` are one check.
  sign_in_probe_hosts = [for origin in local.sign_in_accepted_origins : trimprefix(origin, "https://")]

  sign_in_probe_base_host = local.sign_in_probe_enabled ? regex("^https://([^/]+)/?$", var.frontend_base_url)[0] : ""

  # A fixed, syntactically valid S256 challenge (43 unreserved characters). No
  # token is ever exchanged for it: the probe stops at the login page.
  sign_in_probe_code_challenge = "dravr-sign-in-uptime-probe-0000000000000000"

  # The authorize query packages/api-client beginSignIn builds, minus client_id
  # and redirect_uri: both apps always send prompt=login and a ui_locales.
  sign_in_probe_query_tail = "response_type=code&code_challenge=${local.sign_in_probe_code_challenge}&code_challenge_method=S256&state=uptime-probe&prompt=login&ui_locales=en"

  sign_in_probes = local.sign_in_probe_enabled ? merge(
    {
      for host in local.sign_in_probe_hosts : "web-${host}" => {
        host         = host
        app          = "web"
        display_name = "Sign-in probe: web on ${host}"
        path         = "/oauth2/authorize?client_id=dravr-web&redirect_uri=${urlencode("https://${host}/auth/callback")}&${local.sign_in_probe_query_tail}"
      }
    },
    {
      "mobile-${local.sign_in_probe_base_host}" = {
        host         = local.sign_in_probe_base_host
        app          = "mobile"
        display_name = "Sign-in probe: mobile on ${local.sign_in_probe_base_host}"
        path         = "/oauth2/authorize?client_id=dravr-mobile&redirect_uri=${urlencode("dravr://auth/callback")}&${local.sign_in_probe_query_tail}"
      }
    },
  ) : {}

  # The checks' ids, for the alert's filters: one condition covers them all,
  # so adding an origin adds no condition (a metric policy allows at most 6).
  sign_in_probe_check_ids = join(",", [for check in google_monitoring_uptime_check_config.sign_in : "\"${check.uptime_check_id}\""])
  sign_in_probe_filter    = "metric.type=\"monitoring.googleapis.com/uptime_check/check_passed\" AND resource.type=\"uptime_url\" AND metric.label.check_id=one_of(${local.sign_in_probe_check_ids})"
}

resource "google_monitoring_uptime_check_config" "sign_in" {
  for_each = local.sign_in_probes

  project      = var.project_id
  display_name = each.value.display_name
  period       = "300s"
  # Each check follows a redirect (authorize 303 → login) through nginx to an
  # internal api that may be cold-starting; Google asks for 30s or more on a
  # redirecting URL, so 10s would page while sign-in works.
  timeout = "30s"

  # A failing probe leaves its response in Cloud Logging, which the runbook's
  # first step reads.
  log_check_failures = true

  http_check {
    path           = each.value.path
    port           = 443
    use_ssl        = true
    validate_ssl   = true
    request_method = "GET"

    accepted_response_status_codes {
      status_class = "STATUS_CLASS_2XX"
    }
  }

  content_matchers {
    content = "action=\"/oauth2/login\""
    matcher = "CONTAINS_STRING"
  }

  monitored_resource {
    type = "uptime_url"
    labels = {
      project_id = var.project_id
      host       = each.value.host
    }
  }

  user_labels = {
    probe = "first-party-sign-in"
    app   = each.value.app
  }

  # A check is created only once the api revision carrying its origin in
  # ALLOWED_MOBILE_REDIRECT_ORIGINS is serving: a first probe on the previous
  # revision would draw a 400, and first_party_sign_in_monitoring.tf pages on
  # a single one.
  depends_on = [module.backend]
}

resource "google_monitoring_alert_policy" "sign_in_probe_failed" {
  count = local.sign_in_probe_enabled ? 1 : 0

  project      = var.project_id
  display_name = "Sign-in broken: the first-party sign-in probe is failing"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      An uptime check that opens Dravr's hosted sign-in exactly as the web or
      mobile app does has failed from at least two checker locations (not
      necessarily two regions: the USA region holds several) for ten minutes,
      or has stopped reporting. The incident's `host` and `check_id` labels
      say which origin and which check (the check's display name says which
      app). Athletes on that origin cannot sign in.

      What to do:
      1. Open the check in Monitoring > Uptime checks, and its failed
         responses in Cloud Logging: the status says which way it broke.
         - 400: the server refused the redirect_uri, the hosted
           "Invalid redirect_uri" page. The origin is missing from the
           first-party list: `FRONTEND_URL`, the issuer, or
           `ALLOWED_MOBILE_REDIRECT_ORIGINS` (terraform `frontend_base_url`
           and `local.frontend_extra_origins` in main.tf: the
           `frontend_previous_origins`, every `public_domains` hostname and
           the frontend's run.app URL). Every probed origin is in that list by
           construction, so a 400 means the api's env drifted from terraform
           (re-apply infra/environments/dev) or its FirstPartyRedirects
           regressed. The api logs `first-party redirect_uri refused` naming
           it.
         - 200 with the content check failed: the request no longer reaches
           the api's login page, most likely nginx serving the SPA for
           `/oauth2/` (docker/images/frontend/nginx.conf).
         - 429/503/5xx or timeout: the api is rate limiting, down or slow;
           follow the api's own alerts.
         - No data: a check was deleted or disabled outside terraform;
           re-apply infra/environments/dev.
      2. Reproduce without following redirects: `curl -sI` the check's URL
         must answer 303 with a Location on `/oauth2/login`.
      3. Workaround for athletes meanwhile: sign in at `frontend_base_url`.
    EOT
    mime_type = "text/markdown"
  }

  # ONE condition over every check, not one each: Cloud Monitoring caps a
  # metric policy at 6 conditions, so one per origin would fail plan as soon
  # as a sixth check existed. Grouped per check, each check is its own time
  # series and opens its own incident, labelled with its check_id and host.
  conditions {
    display_name = "A sign-in probe is failing from 2+ checker locations"

    condition_threshold {
      filter          = local.sign_in_probe_filter
      comparison      = "COMPARISON_GT"
      threshold_value = 1
      duration        = "600s"

      # Latest result per check and checker location (check_passed is one
      # series per location, and a region such as USA holds several), then
      # how many locations of each check report a failure. More than one,
      # held for two check periods: one flaky location or one slow probe does
      # not page, though a failure confined to one multi-location region can.
      aggregations {
        alignment_period     = "600s"
        per_series_aligner   = "ALIGN_NEXT_OLDER"
        cross_series_reducer = "REDUCE_COUNT_FALSE"
        group_by_fields      = ["metric.label.check_id", "resource.label.host"]
      }
    }
  }

  # The threshold above sees only reported failures: a check that stops
  # reporting at all would leave it quiet. One shared condition, grouped per
  # check.
  conditions {
    display_name = "A sign-in probe stopped reporting"

    condition_absent {
      filter   = local.sign_in_probe_filter
      duration = "1800s"

      aggregations {
        alignment_period     = "600s"
        per_series_aligner   = "ALIGN_NEXT_OLDER"
        cross_series_reducer = "REDUCE_COUNT"
        group_by_fields      = ["metric.label.check_id"]
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = "1800s"
  }

  lifecycle {
    # Cloud Run keeps serving the app on every run.app URL, and the web app
    # signs in from the one it was opened on: each must be accepted. Unknown
    # until the frontend exists, so on a fresh environment this is checked at
    # apply.
    precondition {
      condition = alltrue([
        for url in(var.enable_frontend ? module.frontend[0].service_urls : []) :
        contains(local.sign_in_accepted_origins, trimsuffix(url, "/"))
      ])
      error_message = "A run.app URL of the frontend service is not a first-party origin, so web sign-in there is refused (carnet#1145, carnet#1152). Add it to frontend_previous_origins or local.frontend_extra_origins in main.tf."
    }
  }
}
