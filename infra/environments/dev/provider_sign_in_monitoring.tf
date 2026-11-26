# ABOUTME: Log-based alert that posts to ops Slack when an athlete is still not connected 10 min after a provider refused them
# ABOUTME: Matches the follow-up each refusal queues, not the refusal itself, so an athlete who retries and connects posts nothing
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Background
# ----------
# On 2026-11-24 at 17:01 Montreal an athlete could not connect Strava during
# onboarding. The api logged one line, `Sciotte remote login rejected by
# provider (user credentials)` (pierre_routes_auth::sciotte, the
# LoginOutcome::Failed arm), and answered 200 with `{"status":"failed"}`, so
# neither "Server errors (5xx) on the api or frontend" nor "Sign-in broken: a
# Dravr app was refused at /oauth2/authorize" fired. Nobody knew until the
# athlete said so.
#
# The first version of this alert matched that refusal line and posted once
# per refusal: an athlete who mistyped a password and connected through OAuth
# a minute later still woke ops, and one stuck athlete retrying posted several
# times (carnet#1837). A refusal is an event; what ops acts on is a state, an
# athlete still not connected. So every refusal now records itself as the
# athlete's last one (shared cache, keyed by the user-facing provider, 30 min)
# and queues a follow-up 10 min later on the turns queue, the way routine
# wakes are queued (turn_queue.tf: no queue or IAM of its own). Only the
# follow-up of the latest refusal acts: it reads whether any backend serving
# the provider (OAuth or sign-in) now holds a live connection, and logs
# `Provider sign-in still not connected after refusals` at WARN when none
# does, which this alert matches, or `Provider sign-in recovered after
# refusals` at INFO, which posts nothing. Each refusal line keeps its text and
# now carries attempt_id and refusal_ordinal, for the logs.
#
# Why a log-match alert and not a log-based metric
# ------------------------------------------------
# The point is to see one stuck athlete within minutes, with the provider's
# own words. A matched-log condition posts the line's extracted labels into
# the Slack message; a metric threshold would only say that a count went up,
# and PromQL over log metrics sees events, not whether the athlete connected.
#
# Matched on the message text and the tracing target, never on severity, so a
# change of the line's level never silences the alert. The line is logged at
# WARN by crates/pierre-routes-auth/src/sign_in_follow_up.rs, which pins its
# target to the one below, logs only ids, counts and masked values (the
# account and any address in the provider's reason are masked when the
# refusal is recorded), and holds the parity test against this filter and its
# label extractors, in both directions.

resource "google_monitoring_alert_policy" "provider_sign_in_rejected" {
  project      = var.project_id
  display_name = "Provider sign-in still not connected: an athlete was refused and has not connected since"
  combiner     = "OR"

  documentation {
    subject   = "Provider sign-in still not connected: $${log.extracted_label.provider} after $${log.extracted_label.refusals} refusal(s) — $${log.extracted_label.reason}"
    mime_type = "text/markdown"
    content   = <<-EOT
      The provider refused an athlete's sign-in $${log.extracted_label.refusals}
      time(s), and $${log.extracted_label.minutes} min after the last refusal no
      route to the provider is connected. Each refusal was answered 200 with
      `status: failed`, so no 5xx alert saw it.

      * Provider: `$${log.extracted_label.provider}` (sign-in backend `$${log.extracted_label.backend}`)
      * Refusals in the window: `$${log.extracted_label.refusals}`
      * Provider's last reason: `$${log.extracted_label.reason}`
      * Last sign-in method: `$${log.extracted_label.method}`
      * Account typed at the provider (masked): `$${log.extracted_label.account}`
      * Routes checked, none connected: `$${log.extracted_label.routes_checked}`
      * Minutes since the last refusal: `$${log.extracted_label.minutes}`
      * User: `$${log.extracted_label.user_id}`
      * Tenant: `$${log.extracted_label.tenant_id}`

      [This user's api logs, last hour](https://console.cloud.google.com/logs/query;query=resource.labels.service_name%3D%22${var.service_name}-api%22%0AjsonPayload.user_id%3D%22$${log.extracted_label.user_id}%22;duration=PT1H?project=${var.project_id})

      What to do: the athlete is still stuck at connect; reach out. The refusal lines in the logs above carry each attempt's attempt_id and refusal_ordinal.
    EOT
  }

  conditions {
    display_name = "Provider sign-in still not connected 10 min after the last refusal"

    condition_matched_log {
      filter = <<-EOT
        resource.type="cloud_run_revision"
        resource.labels.service_name="${var.service_name}-api"
        labels."rust.target"="pierre_routes_auth::sign_in_follow_up"
        jsonPayload.message:"Provider sign-in still not connected after refusals"
      EOT

      # The GcpFormatter (crates/pierre-logging/src/gcp.rs) writes each tracing
      # field as a jsonPayload key.
      label_extractors = {
        "user_id"   = "EXTRACT(jsonPayload.user_id)"
        "tenant_id" = "EXTRACT(jsonPayload.tenant_id)"
        "provider"  = "EXTRACT(jsonPayload.provider)"
        "backend"   = "EXTRACT(jsonPayload.backend)"
        "refusals"  = "EXTRACT(jsonPayload.refusals)"
        "minutes"   = "EXTRACT(jsonPayload.minutes)"
        "reason"    = "EXTRACT(jsonPayload.reason)"
        # method_used, not method: jsonPayload.method is the HTTP verb.
        "method"         = "EXTRACT(jsonPayload.method_used)"
        "account"        = "EXTRACT(jsonPayload.account)"
        "routes_checked" = "EXTRACT(jsonPayload.routes_checked)"
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    notification_rate_limit {
      period = var.alert_windows["provider_sign_in.rejected.notification_rate_limit"]
    }
    auto_close = var.alert_windows["provider_sign_in.rejected.auto_close"]
  }
}
