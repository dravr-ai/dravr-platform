# ABOUTME: Cloud Run cost alerting — fires when the backend api service never scales to zero
# ABOUTME: Added 2026-06-11 after an idle /ws browser tab pinned a 2-vCPU instance 24/7 for weeks
#
# Background
# ----------
# The backend api service runs with CPU always-allocated (cpu_idle = false, see
# main.tf) so that the Copilot ACP LLM runner and Discord Gateway survive between
# requests. Under always-allocated CPU, any single long-lived inbound connection
# pins a full 2-vCPU + 2Gi instance for its entire duration — and Cloud Run never
# scales it down while the connection is held.
#
# In June 2026 a single idle browser tab held the (now-removed) /ws WebSocket open,
# reconnecting every ~10 minutes around the clock. That one tab kept exactly one
# instance "active" 24/7 for weeks — ~86,400 instance-seconds/day, ~$137/mo of pure
# idle waste — billing even overnight with zero users. Nothing alerted; we found it
# only by reverse-engineering the billable_instance_time metric off the cost report.
#
# This alert encodes the failure signature directly: "the instance floor stayed
# ABOVE THE CONFIGURED FLOOR for N hours straight." A sustained excess means
# something is holding an extra instance open — a new /ws-style pin, a stuck
# streaming request, or an always-on subprocess that escaped its budget.
#
# It compares against `backend_min_instances`, not against zero. The service now
# runs a warm floor of 1 (registre#109: a detached messaging turn makes an idle
# scaledown able to land on a live coaching turn), and an alert hard-coded to
# zero would fire the moment that floor was raised and never stop — which is
# precisely how an alert channel stops being read. At min=0 this is identical to
# the original "never scaled to zero" test; at min=1 it asks the same question
# one instance up.

# -----------------------------------------------------------------------------
# Tuning knobs
# -----------------------------------------------------------------------------

variable "instance_floor_alert_hours" {
  description = "Fire the idle-floor alert when the backend api instance count stays above backend_min_instances continuously for this many hours (i.e. an instance beyond the configured floor never went away). Real dev traffic is bursty and returns to the floor between bursts, so this targets a stuck/pinned instance, not active use. Lower = more sensitive (and more likely to fire during a genuinely busy stretch)."
  type        = number
  default     = 4
}

# -----------------------------------------------------------------------------
# Alert Policy — backend api never scaled to zero over the configured window
# -----------------------------------------------------------------------------
# Reuses the Slack notification channel defined in monitoring.tf (same #dev-dravr-errors
# channel the Cloud Run job-failure alert routes to) — operators already watch it.

resource "google_monitoring_alert_policy" "instance_never_idle" {
  project      = var.project_id
  display_name = "dravr-mcp-server-api held instances above its floor"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      The backend `${var.service_name}-api` Cloud Run service kept more than
      ${var.backend_min_instances} instance(s) running continuously for
      ${var.instance_floor_alert_hours}+ hours without ever returning to its
      configured floor.

      Because this service runs with CPU always-allocated (cpu_idle = false), a
      pinned instance bills a full 2 vCPU + 2Gi the entire time (~$137/mo per
      instance) even with no users. This alert is about instances ABOVE the
      floor: the floor itself is bought deliberately (see terraform.tfvars) and
      is not what fired this. A surplus that never drains almost always means
      something is holding an extra instance open:

        * a long-lived inbound connection (a WebSocket / SSE / streaming request
          held by an idle browser tab — the June 2026 /ws incident)
        * a stuck request that never completes
        * an always-on subprocess that escaped its intended budget

      Investigate what is holding the instance open:

        # What inbound connections are alive right now (look for status 101 = WS upgrade)?
        gcloud logging read 'resource.type="cloud_run_revision" AND \
          resource.labels.service_name="${var.service_name}-api" AND httpRequest.status=101' \
          --project ${var.project_id} --freshness=1h --limit 20 \
          --format="value(timestamp, httpRequest.requestUrl, httpRequest.userAgent)"

        # Confirm the floor from the billed metric:
        #   run.googleapis.com/container/billable_instance_time  (~86,400/day == 1 instance 24/7)

      Background: this alert exists because an idle /ws browser tab pinned an
      instance 24/7 for weeks in June 2026, billing ~$137/mo silently. Don't let
      that happen again.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "api instance count stayed above its floor for ${var.instance_floor_alert_hours}h"

    condition_threshold {
      # Per 30-minute bucket, take the MINIMUM instance count (ALIGN_MIN). If that
      # minimum exceeds the configured floor, the service never returned to the
      # floor in that bucket. The condition must hold continuously for
      # instance_floor_alert_hours, so a single gap back down to the floor breaks
      # the streak and suppresses the alert.
      filter = <<-EOT
        resource.type = "cloud_run_revision"
        AND resource.labels.service_name = "${var.service_name}-api"
        AND metric.type = "run.googleapis.com/container/instance_count"
      EOT

      duration   = "${var.instance_floor_alert_hours * 3600}s"
      comparison = "COMPARISON_GT"
      # The configured warm floor, not zero. Hard-coding zero here would make
      # this alert fire forever the moment a floor was bought (registre#109).
      threshold_value = var.backend_min_instances

      aggregations {
        alignment_period     = "1800s"
        per_series_aligner   = "ALIGN_MIN"
        cross_series_reducer = "REDUCE_SUM"
      }

      trigger {
        count = 1
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    # Auto-close once the count finally returns to the floor and the metric
    # stops satisfying the condition. Generous window so a brief blip doesn't
    # prematurely resolve a genuinely-stuck instance.
    auto_close = "3600s"
  }
}
