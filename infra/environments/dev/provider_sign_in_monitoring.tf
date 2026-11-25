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
# Matched on the message text and the tracing target, never on severity, so a
# change of the line's level never silences the alert. The line is logged at
# WARN by crates/pierre-routes-auth/src/sciotte_rejection.rs, which pins its
# target to the one below, masks the account and any address in the reason,
# and holds the parity test against this filter and its label extractors.

resource "google_monitoring_alert_policy" "provider_sign_in_rejected" {
  project      = var.project_id
  display_name = "Provider sign-in refused: an athlete could not connect a provider"
  combiner     = "OR"

  documentation {
    subject   = "Provider sign-in refused: $${log.extracted_label.provider} — $${log.extracted_label.reason}"
    mime_type = "text/markdown"
    content   = <<-EOT
      An athlete's credential sign-in was refused by the provider. The api
      answered 200 with `status: failed`, so no 5xx alert saw it.

      * Provider: `$${log.extracted_label.provider}`
      * Provider's reason: `$${log.extracted_label.reason}`
      * Sign-in method: `$${log.extracted_label.method}`
      * Account typed at the provider (masked): `$${log.extracted_label.account}`
      * User: `$${log.extracted_label.user_id}`
      * Tenant: `$${log.extracted_label.tenant_id}`
      * Request id: `$${log.extracted_label.http_request_id}`

      [This user's api logs, last hour](https://console.cloud.google.com/logs/query;query=resource.labels.service_name%3D%22${var.service_name}-api%22%0AjsonPayload.user_id%3D%22$${log.extracted_label.user_id}%22;duration=PT1H?project=${var.project_id})

      What to do: the athlete is stuck at connect; the window now offers OAuth — check they got through.
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
        "provider"        = "EXTRACT(jsonPayload.provider)"
        "reason"          = "EXTRACT(jsonPayload.reason)"
        "user_id"         = "EXTRACT(jsonPayload.user_id)"
        "tenant_id"       = "EXTRACT(jsonPayload.tenant_id)"
        "http_request_id" = "EXTRACT(jsonPayload.http_request_id)"
        # method_used, not method: jsonPayload.method is the HTTP verb.
        "method"  = "EXTRACT(jsonPayload.method_used)"
        "account" = "EXTRACT(jsonPayload.account)"
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
