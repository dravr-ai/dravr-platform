# ABOUTME: Log-based metric + alert that pages when a provider app's request budget refuses a read an athlete waits on
# ABOUTME: Work nobody waits on is held to its background share, so a spent foreground budget is never expected
#
# Background
# ----------
# Every request to a provider whose app has a published limit (Strava, Wahoo,
# Intervals.icu, WHOOP, Garmin, Terra) is admitted against that app's windows,
# counted in the database so every instance shares them
# (`pierre_providers::request_budget`). Work nobody waits on runs
# `in_background` and may take only `BACKGROUND_SHARE_PERCENT` (25%) of each
# window; a read an athlete waits on may take all of it. A refusal is logged:
# `debug` for background work, which is the share doing its job, and `warn`
# "Provider request budget spent" for a foreground read.
#
# On 2026-11-10 a backfill loop (carnet#1478) spent the Strava app's day by
# 12:00 UTC, so every athlete's live reads were refused until midnight. The only
# page was the head-read ERROR (`Live activity head read failed`), for one
# athlete, five hours after the budget ran out; the limiter's own warn reached
# nobody. Then the capture sweep and the scheduled sync were found reading at the
# full share (carnet#1491), which made a foreground spend look routine. Both now
# run in the background share, so this warn means an athlete was refused.
#
# Why an ALERT
# ------------
# A foreground spend refuses every athlete the app signs for, not just one,
# until the window resets (up to 24 h for a daily window), and the chat answers
# from the cache meanwhile. Something spent the share athletes need: a loop, a
# job missing `in_background`, or real growth past the provider's limit.

resource "google_logging_metric" "provider_budget_spent" {
  project = var.project_id
  name    = "dravr-provider-budget-spent"

  description = "Counts provider requests that our own limiter refused because the app's request budget was spent, for a read an athlete waits on (the 'Provider request budget spent' warn of pierre_providers::request_budget). Background work is held to its share and logs at debug, so the expected steady state is zero (carnet#1491)."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    labels."rust.target"="pierre_providers::request_budget"
    jsonPayload.message="Provider request budget spent"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "provider"
      value_type  = "STRING"
      description = "The provider whose budget refused the read (strava, intervals_icu, wahoo, ...)."
    }

    labels {
      key         = "app"
      value_type  = "STRING"
      description = "The signing OAuth app's client id; empty for a personal API key's own windows."
    }
  }

  label_extractors = {
    "provider" = "EXTRACT(jsonPayload.provider)"
    "app"      = "EXTRACT(jsonPayload.app)"
  }
}

# The metric descriptor takes up to ~10 minutes to reach Cloud Monitoring; an
# alert policy created before that 404s (see wire_shape_monitoring.tf).
resource "time_sleep" "provider_budget_spent_metric_propagation" {
  depends_on      = [google_logging_metric.provider_budget_spent]
  create_duration = "600s"
}

resource "google_monitoring_alert_policy" "provider_budget_spent" {
  project      = var.project_id
  display_name = "Provider request budget spent: athletes' live reads are refused"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      Our own limiter refused a read an athlete was waiting on, because the
      provider app's request budget is spent. Every athlete that app signs for
      is served from the cache until the window resets: the daily window at
      00:00 UTC. The `provider` and `app` labels name which.

      What to do:
      1. Find who spent it: Logs Explorer on the api service,
         `labels."rust.target"="pierre_providers::utils"` and
         `jsonPayload.message:"Starting <provider> API request"`, grouped by
         `jsonPayload.user_id`. One athlete with hundreds of requests is a loop
         (carnet#1478: a backfill's warm starting backfills on the athlete's
         other providers).
      2. Find which job: the lines around those requests name their target
         (`activity_backfill`, `capture_sweep`, `provider_refresh::scheduled`,
         `personal_bests`). Work nobody waits on must run `in_background`; one
         that does not spends the athletes' share (carnet#1491).
      3. The windows are counted in `usage_counters` under
         `provider_requests:<provider>:<app>`. Never reset them by hand: the
         provider counts the same requests and refuses the app once it is over
         its own limit.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Any foreground request refused by a spent provider budget"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.provider_budget_spent.name}\""
      comparison      = "COMPARISON_GT"
      threshold_value = 0
      duration        = "0s"

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_DELTA"
        cross_series_reducer = "REDUCE_SUM"
        group_by_fields      = ["metric.label.provider", "metric.label.app"]
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = "1800s"
  }

  depends_on = [time_sleep.provider_budget_spent_metric_propagation]
}
