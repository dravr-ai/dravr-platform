# ABOUTME: Cloud Run Job failure alerting — fires on any dravr-mcp-server-* job non-zero exit
# ABOUTME: Added 2026-05-25 after c6630e46 silently exit-1'd seed-coaches for 3.5 weeks
#
# Background
# ----------
# On 2026-05-25 we discovered the `dravr-mcp-server-seed-coaches` Cloud Run
# job had been exit-1 on every execution since 2026-05-01 (commit c6630e46
# flipped the seeder to clone-contremaitre-per-run but the terraform locals
# never got the required env vars). No alert fired because we had no
# monitoring on Cloud Run job non-zero exits.
#
# This file plugs that gap with three resources:
#   1. google_logging_metric — counts ERROR-or-above log lines emitted by
#      any Cloud Run job whose name matches the dravr-mcp-server-* family.
#      Cloud Run automatically writes an ERROR log line when a job's task
#      exits with a non-zero status code, so this metric trips on:
#        * pierre-cli seed-* failures (the c6630e46 class of bug)
#        * pierre-cli check-drift coaches failures (the contremaitre-vs-DB
#          drift signal, see drift_check.tf)
#        * any future *-cron / *-job we add following the same naming
#   2. google_monitoring_notification_channel — Slack channel pointing at
#      #dev-dravr-errors. The auth_token is read from the existing
#      slack_bot_token secret (same one Pierre uses for outbound messaging),
#      so there's no second token to load. Grant the existing Slack app
#      chat:write scope on the alerts channel.
#   3. google_monitoring_alert_policy — fires when the log metric exceeds
#      0 in any 5-minute window, routing to the Slack channel above.

# -----------------------------------------------------------------------------
# Log-Based Metric — count ERROR+ logs from dravr-mcp-server-* Cloud Run jobs
# -----------------------------------------------------------------------------

resource "google_logging_metric" "cloud_run_job_failures" {
  project = var.project_id
  name    = "dravr-cloud-run-job-failures"

  description = "Counts ERROR-or-above log lines from dravr-mcp-server-* Cloud Run jobs (seed-*, drift-check-*, *-cron, *-job). Added 2026-05-25 after the 3.5-week silent seed-coaches exit-1 regression caused by c6630e46."

  # The regex below intentionally matches every dravr-mcp-server-* job that
  # could fail in a way operators care about — the c6630e46 outage was a
  # seed-* job, but Cloud Run jobs we add later (drift-check-*, *-cron,
  # *-job) should fall under the same alert without further wiring.
  filter = <<-EOT
    resource.type="cloud_run_job"
    resource.labels.job_name=~"^${var.service_name}-(seed-.*|drift-check-.*|.*-cron|.*-job)$"
    severity>=ERROR
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "job_name"
      value_type  = "STRING"
      description = "Name of the Cloud Run job whose execution emitted the ERROR log line"
    }
  }

  label_extractors = {
    "job_name" = "EXTRACT(resource.labels.job_name)"
  }
}

# -----------------------------------------------------------------------------
# Slack Notification Channel
# -----------------------------------------------------------------------------
# Reuses the existing slack_bot_token secret (the same one Pierre uses for
# outbound messaging). GCP-Slack notification channels accept any Slack bot
# token that has chat:write scope on the target channel — load that scope
# into the existing app rather than maintain a second token / secret.
data "google_secret_manager_secret_version" "slack_bot_token" {
  project = var.project_id
  secret  = module.secrets.secret_ids["slack_bot_token"]
}

resource "google_monitoring_notification_channel" "slack_alerts" {
  project      = var.project_id
  display_name = "Dravr Slack Alerts (#${trimprefix(var.slack_error_channel, "#")})"
  type         = "slack"
  description  = "Routes Cloud Run job failure alerts to the dev-dravr-errors Slack channel."

  labels = {
    # Slack expects a channel name with a leading '#'.
    "channel_name" = startswith(var.slack_error_channel, "#") ? var.slack_error_channel : "#${var.slack_error_channel}"
  }

  sensitive_labels {
    auth_token = data.google_secret_manager_secret_version.slack_bot_token.secret_data
  }

  # The auth_token rotates independently of terraform — keep `terraform
  # apply` from clobbering an operator-rotated value.
  lifecycle {
    ignore_changes = [sensitive_labels[0].auth_token]
  }
}

# -----------------------------------------------------------------------------
# Alert Policy — fires when any dravr-mcp-server-* job emits an ERROR
# -----------------------------------------------------------------------------

resource "google_monitoring_alert_policy" "job_failures" {
  project      = var.project_id
  display_name = "dravr-mcp-server-job-failures"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      One or more Cloud Run jobs in the dravr-mcp-server-* family emitted an
      ERROR log line in the last 5 minutes. Most commonly this is a
      non-zero task exit. Check the job's execution history:

        gcloud run jobs executions list --job=<job_name> --region=${var.region}

      Background: this alert exists because commit c6630e46 (2026-05-01)
      silently broke the seed-coaches job for 3.5 weeks. Every execution
      exited 1 with no alert. Don't let that happen again.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Cloud Run job ERROR log line in 5min window"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_job\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.cloud_run_job_failures.name}\""
      duration        = "0s"
      comparison      = "COMPARISON_GT"
      threshold_value = 0

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_SUM"
        cross_series_reducer = "REDUCE_SUM"
        group_by_fields      = ["metric.label.job_name"]
      }

      trigger {
        count = 1
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    # Auto-close the incident after 30 minutes with no further ERROR logs —
    # most job failures are one-shot (exit on first iteration); we don't
    # want a stale incident hanging around once the next successful run
    # passes.
    auto_close = "1800s"
  }
}
