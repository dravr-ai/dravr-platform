# ABOUTME: Log-based alert that posts to ops Slack each time a provider refuses an athlete's credential sign-in
# ABOUTME: The refusal is answered HTTP 200 with status "failed", so no 5xx or authorize alert ever sees it
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
# Why a log-match alert and not a log-based metric
# ------------------------------------------------
# Every refusal is one athlete stuck at a sign-in, and the point is to see it
# within minutes with the provider's own words. A matched-log condition posts
# the log line's extracted labels (provider, reason) into the Slack message; a
# metric threshold would only say that a count went up.
#
# Matched on the message text and the tracing target, never on severity: the
# line was INFO when this landed and is being raised to WARN with `provider`
# and `reason` fields, and the alert must hold on both sides of that deploy.
# Before it, the provider and reason labels are empty; user_id is there on both.

resource "google_monitoring_alert_policy" "provider_sign_in_rejected" {
  project      = var.project_id
  display_name = "Provider sign-in refused: an athlete could not connect a provider"
  combiner     = "OR"

  documentation {
    subject   = "Provider sign-in refused: $${log.extracted_label.provider} ($${log.extracted_label.reason})"
    mime_type = "text/markdown"
    content   = <<-EOT
      An athlete's credential sign-in to a provider was refused by the provider.
      The api answered 200 with `status: failed`, so the athlete saw a sign-in
      error and is likely stuck (onboarding included).

      * Provider: `$${log.extracted_label.provider}`
      * Provider's reason: `$${log.extracted_label.reason}`
      * User: `$${log.extracted_label.user_id}`

      (Provider and reason are empty on a build that predates the WARN log
      line carrying them.)

      What to do:
      1. Find the line: Logs Explorer on the `${var.service_name}-api` service,
         `jsonPayload.message:"Sciotte remote login rejected by provider"`. The
         same trace holds the sciotte sign-in service's side.
      2. Wrong password or an expired account: reach the athlete and help them
         retry. A refusal text the athlete did not cause (captcha, blocked
         sign-in, a changed provider page) on several athletes means the
         provider changed: check the sciotte login canary.
      3. Workaround for the athlete meanwhile: connect through the sign-in
         extension or OAuth from the connections screen.
    EOT
  }

  conditions {
    display_name = "Sciotte remote login rejected by provider"

    condition_matched_log {
      filter = <<-EOT
        resource.type="cloud_run_revision"
        resource.labels.service_name="${var.service_name}-api"
        labels."rust.target"="pierre_routes_auth::sciotte"
        jsonPayload.message:"Sciotte remote login rejected by provider"
      EOT

      # The GcpFormatter (crates/pierre-logging/src/gcp.rs) writes each tracing
      # field as a jsonPayload key.
      label_extractors = {
        "provider" = "EXTRACT(jsonPayload.provider)"
        "reason"   = "EXTRACT(jsonPayload.reason)"
        "user_id"  = "EXTRACT(jsonPayload.user_id)"
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
