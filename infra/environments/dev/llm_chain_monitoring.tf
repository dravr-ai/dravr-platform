# ABOUTME: Alerts on the LLM chain's health: a tier's quota or rate limit, a failed startup probe, and fallthroughs
# ABOUTME: Log-based metrics on the API service routed to the same Slack channel as the error alert (carnet#479)
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# On 2026-09-21 the API came up with a chat primary whose account had spent
# its monthly Copilot quota. The startup probe logged a WARNING at 07:49 and
# nothing alerted; the first thing anyone saw was an athlete's turn failing
# at 07:58. Three alerts close that gap, each one a log-based metric on the
# API service's structured LLM lines (every runner line carries
# jsonPayload.provider), routed to the Slack channel monitoring.tf already
# wires:
#
#   1. A tier's quota or rate limit — the account, not the athlete: the
#      athlete's own budget is refused by the ingress before dispatch and
#      never comes back from a runner.
#   2. The LLM startup probe failed — the revision is serving with a primary
#      that cannot answer; the chain, if any, is doing all the work.
#   3. The chain fell through — the primary is being bypassed; a burst of
#      these is the early signal of the next quota incident.

# -----------------------------------------------------------------------------
# 1. A tier's quota or rate limit
# -----------------------------------------------------------------------------

resource "google_logging_metric" "llm_tier_rate_limited" {
  project = var.project_id
  name    = "dravr-llm-tier-rate-limited"

  description = "Counts LLM runner lines on the API service whose error is a provider quota or rate limit (RateLimit, quota_exceeded, 429). Labelled by provider so the alert names the tier."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    jsonPayload.provider!=""
    jsonPayload.error=~"(?i)quota|rate.?limit"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "provider"
      value_type  = "STRING"
      description = "The LLM tier whose account answered with a quota or rate limit"
    }
  }

  label_extractors = {
    "provider" = "EXTRACT(jsonPayload.provider)"
  }
}

# The metric behind the policy below is `llm_tier_quota` — the first one,
# `llm_tier_rate_limited`, counted the chain's duplicate line and read the
# tier from the span's provider (the primary's), so Slack said "claude-code"
# for a Copilot quota. A metric's label block is immutable in Cloud Monitoring
# and a metric an alert policy references cannot be deleted, so the fix is a
# new metric the policy is moved to; the old resource leaves in the apply
# after that move.
resource "google_logging_metric" "llm_tier_quota" {
  project = var.project_id
  name    = "dravr-llm-tier-quota"

  description = "Counts LLM runner lines on the API service whose error is a provider quota or rate limit (RateLimit, quota_exceeded, 429), one per event, labelled by the tier named in the error."

  # One event, one line: inside a chain the same failure is logged twice —
  # by the runner ("<tier>: turn failed") and by the chain ("LLM tier failed
  # with a provider fault; falling back"). The runner's line is the one
  # counted, and the tier is read from the error text embacle writes
  # ("RateLimit: copilot-sdk: You have exceeded ..."), never from the span.
  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    jsonPayload.provider!=""
    jsonPayload.error=~"(?i)quota|rate.?limit"
    NOT jsonPayload.message:"falling back"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "tier"
      value_type  = "STRING"
      description = "The LLM tier whose account answered with a quota or rate limit, as named in the error text"
    }
  }

  label_extractors = {
    "tier" = "REGEXP_EXTRACT(jsonPayload.error, \"^RateLimit: ([^:]+):\")"
  }
}

resource "google_monitoring_alert_policy" "llm_tier_rate_limited" {
  project      = var.project_id
  display_name = "dravr-llm-tier-rate-limited"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      An LLM tier answered with a quota or rate limit in the last 5 minutes.
      The tier label names it, read from the error text. This is the platform's account for that
      provider, not an athlete's budget.

      With a runtime chain configured the next tier is serving the turns;
      without one, athletes are failing right now — flip PIERRE_LLM_PROVIDER
      (infra/environments/dev/main.tf, then `terraform.yml` dev/apply) or lift
      the quota on the account.

      Read the lines:

        gcloud logging read 'resource.labels.service_name="${var.service_name}-api" AND jsonPayload.error=~"(?i)quota|rate.?limit"' --project=${var.project_id} --freshness=30m --limit=20

      Background: on 2026-09-21 the Copilot account's monthly quota was spent
      and nothing alerted until an athlete's turn failed (carnet#478, #479).
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "LLM tier quota or rate limit in 5min window"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.llm_tier_quota.name}\""
      duration        = "0s"
      comparison      = "COMPARISON_GT"
      threshold_value = 0

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_SUM"
        cross_series_reducer = "REDUCE_SUM"
        group_by_fields      = ["metric.label.tier"]
      }

      trigger {
        count = 1
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    # A spent quota keeps refusing for hours; the incident stays open while
    # the lines keep coming and closes 30 minutes after they stop.
    auto_close = "1800s"
  }
}

# -----------------------------------------------------------------------------
# 2. The LLM startup probe failed
# -----------------------------------------------------------------------------

resource "google_logging_metric" "llm_startup_probe_failed" {
  project = var.project_id
  name    = "dravr-llm-startup-probe-failed"

  description = "Counts 'LLM startup probe failed' lines on the API service: a revision started with a chat primary that could not complete its first turn."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    jsonPayload.message:"LLM startup probe failed"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "provider"
      value_type  = "STRING"
      description = "The primary provider whose startup probe failed"
    }
  }

  label_extractors = {
    "provider" = "EXTRACT(jsonPayload.provider)"
  }
}

resource "google_monitoring_alert_policy" "llm_startup_probe_failed" {
  project      = var.project_id
  display_name = "dravr-llm-startup-probe-failed"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      A new API revision came up and its chat primary failed the startup
      probe (provider label). Every turn now depends on the runtime chain;
      if the chain is empty or also failing, athletes are dead-ending.

        gcloud logging read 'resource.labels.service_name="${var.service_name}-api" AND jsonPayload.message:"startup probe"' --project=${var.project_id} --freshness=30m --limit=10

      The probe's error line right above it names the cause (a spent quota,
      a rejected token, an unreachable runtime).
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "LLM startup probe failed in 5min window"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.llm_startup_probe_failed.name}\""
      duration        = "0s"
      comparison      = "COMPARISON_GT"
      threshold_value = 0

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_SUM"
        cross_series_reducer = "REDUCE_SUM"
        group_by_fields      = ["metric.label.provider"]
      }

      trigger {
        count = 1
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = "1800s"
  }
}

# -----------------------------------------------------------------------------
# 3. The chain fell through
# -----------------------------------------------------------------------------

resource "google_logging_metric" "llm_chain_fallthrough" {
  project = var.project_id
  name    = "dravr-llm-chain-fallthrough"

  description = "Counts 'Runtime LLM fallback engaged' lines on the API service: a tier failed and the next one took the turn. Labelled by the tier that was bypassed."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    jsonPayload.message:"Runtime LLM fallback engaged"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "provider"
      value_type  = "STRING"
      description = "The tier that failed and was bypassed"
    }
  }

  label_extractors = {
    "provider" = "EXTRACT(jsonPayload.provider)"
  }
}

resource "google_monitoring_alert_policy" "llm_chain_fallthrough" {
  project      = var.project_id
  display_name = "dravr-llm-chain-fallthrough"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      The runtime chain bypassed a tier (provider label) more than three
      times in 5 minutes. Turns are still being served by the tier behind
      it, so this is the early warning, not the outage: read the bypassed
      tier's errors and act before the next tier runs out too.

        gcloud logging read 'resource.labels.service_name="${var.service_name}-api" AND jsonPayload.message:"fallback"' --project=${var.project_id} --freshness=30m --limit=30
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "More than 3 chain fallthroughs in 5min window"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.llm_chain_fallthrough.name}\""
      duration        = "0s"
      comparison      = "COMPARISON_GT"
      threshold_value = 3

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_SUM"
        cross_series_reducer = "REDUCE_SUM"
        group_by_fields      = ["metric.label.provider"]
      }

      trigger {
        count = 1
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = "1800s"
  }
}
