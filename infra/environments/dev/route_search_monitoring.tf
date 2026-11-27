# ABOUTME: Log-based metric and alert for discover_routes finding no public OpenStreetMap (Overpass) server answering
# ABOUTME: Reads the api's one warning per unavailable search; sends no request, so it keeps no service awake
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# discover_routes asks the public Overpass instances for named routes
# (dravr-arpenteur, src/discovery/overpass.rs). When none answers
# within the walk's budget, the athlete is told the route search is down
# (overloaded when every instance said it was saturated) and gets only their
# own loops, or none. Nobody else hears of it:
# the daily live-apis lane catches a day-long outage (carnet#1705), not an
# afternoon of athletes asking for routes and getting none.
#
# The walk logs one warning per unavailable search, with
# event="route_search_unavailable", which this metric counts; the test
# `route_search_alert_test.rs` reads the filter here and pins that line. For a
# minute after such a walk that asked every instance, the same instances are
# not asked again and a search logs event="route_search_unavailable_held"
# instead, so an agent retrying within one turn counts once. The count is still per walk, not per athlete:
# the line carries no caller.
#
# User-defined log-based metrics are chargeable Cloud Monitoring custom metrics
# (Google Cloud Observability pricing). This one reads a line the api already
# writes.

# A metric's label block is immutable and a metric a policy references cannot be
# deleted (see frontend_monitoring.tf), so a new meaning is a new metric.
resource "google_logging_metric" "route_search_unavailable" {
  project = var.project_id
  name    = "dravr-route-search-unavailable"

  description = "Counts discover_routes searches that found no public OpenStreetMap (Overpass) server answering."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    log_id("run.googleapis.com/stdout")
    jsonPayload.event="route_search_unavailable"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"
  }
}

# The metric descriptor takes up to ~10 minutes to reach Cloud Monitoring; a
# policy created before that 404s (see wire_shape_monitoring.tf).
resource "time_sleep" "route_search_metric_propagation" {
  depends_on      = [google_logging_metric.route_search_unavailable]
  create_duration = "600s"
}

# The alert text states its window in minutes, read from the setting the
# condition aligns on, so the text and the condition cannot disagree.
locals {
  route_search_window_minutes = tonumber(trimsuffix(var.alert_windows["route_search.unavailable.alignment_period"], "s")) / 60
}

resource "google_monitoring_alert_policy" "route_search_unavailable" {
  project      = var.project_id
  display_name = "Route search down: no public OpenStreetMap server answering"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      More than ${var.alert_thresholds["route_search.unavailable.threshold"]} route searches in ${local.route_search_window_minutes} minutes found no public OpenStreetMap
      (Overpass) server answering. Athletes asking for a route get only their own
      loops, or none, with a message that the route search is down and that
      trying again later may work.

      First read `cause` on the logged lines. `overloaded` means every instance
      said it was saturated (busy, rate limited, hung, or stopping the query).
      That is a known, accepted dependency: on 2026-11-27 ChefFamille chose not
      to self-host an Overpass instance or pay for one, so there is nothing to
      deploy or roll back, and this alert says how long athletes have been
      without map routes. `failed` means at least one instance refused the
      query, answered something unreadable, or could not be reached. The fault
      may be ours: a query or parser regression in the last deploy, or DNS or
      egress from Cloud Run. Check the recent deploys before blaming the servers.

      What to check:
      1. Logs Explorer on the api service:
         `jsonPayload.event="route_search_unavailable"`. `cause` is overloaded
         or failed, and `outcomes` names each instance and why it failed (busy,
         timeout, stopped the query, refused, unreadable, connection failed).
      2. The main instance's own status page: https://overpass-api.de/api/status
         (free slots, queries running). The other two are overpass.private.coffee
         and maps.mail.ru.
      3. Every instance busy or hung for days: raise it with ChefFamille again;
         the remedy is an instance of our own, which he declined on 2026-11-27.

      The count is over a rolling ${local.route_search_window_minutes} minutes. The incident closes as soon as that
      count falls back to ${var.alert_thresholds["route_search.unavailable.threshold"]} or fewer, which can happen while searches still fail.
      During a long outage with little traffic it can close and reopen, each time
      in Slack; read a reopen within hours as the same outage.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "More than ${var.alert_thresholds["route_search.unavailable.threshold"]} unavailable route searches in ${local.route_search_window_minutes} minutes"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.route_search_unavailable.name}\""
      comparison      = "COMPARISON_GT"
      threshold_value = var.alert_thresholds["route_search.unavailable.threshold"]
      duration        = var.alert_windows["route_search.unavailable.duration"]

      aggregations {
        alignment_period     = var.alert_windows["route_search.unavailable.alignment_period"]
        per_series_aligner   = "ALIGN_DELTA"
        cross_series_reducer = "REDUCE_SUM"
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = var.alert_windows["route_search.unavailable.auto_close"]
  }

  depends_on = [time_sleep.route_search_metric_propagation]
}
