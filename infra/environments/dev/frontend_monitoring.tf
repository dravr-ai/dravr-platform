# ABOUTME: Alerts when the frontend's nginx logs a proxy or server fault, which no other alert reads
# ABOUTME: Log-based metric on the frontend service's stderr, routed to the Slack channel monitoring.tf wires
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Every athlete request enters through the frontend service, whose nginx
# proxies to the internal-ingress API. From 2026-09-15 to 2026-09-26 nginx
# logged 1,057 "connect() to [2600:...]:443 failed (101: Network unreachable)"
# lines and 493 "upstream server temporarily disabled" warnings on dev (the
# static /ws proxy_pass fixed in 571b471dd), and nothing alerted:
#
#   - the Slack error channel's other feeds are the API's in-process error
#     layer and log metrics on the API service and Cloud Run jobs; none reads
#     the frontend service;
#   - nginx writes its error log as plain text on stderr, which Cloud Run
#     stores with no severity, so a severity>=ERROR filter never matches it;
#   - nginx retried the next address, so no request answered 5xx.
#
# The metric reads nginx's own level out of the text. It counts every line at
# [error] and above, plus the one [warn] that means nginx has stopped using an
# upstream address. A request for a static file that does not exist
# ("open() ... failed (2: No such file or directory)") is logged at [error]
# too; that is a 404, mostly scanners probing for /assets/.env, and is left
# out. The access log's JSON severity (docker/images/frontend/nginx.conf) is
# for reading the requests behind an alert, not a second count of them.

# A metric's label block is immutable in Cloud Monitoring and a metric an
# alert policy references cannot be deleted, so a change to what the label
# means is a new metric the policy is moved to, never an edit in place.
resource "google_logging_metric" "frontend_nginx_faults" {
  project = var.project_id
  name    = "dravr-frontend-nginx-faults"

  description = "Counts nginx error-log lines on the frontend service at [error] or above, plus 'upstream server temporarily disabled' warnings, excluding 404s for missing static files. Labelled by nginx's level."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-frontend"
    logName="projects/${var.project_id}/logs/run.googleapis.com%2Fstderr"
    (textPayload=~"\[(error|crit|alert|emerg)\]" OR textPayload:"upstream server temporarily disabled")
    NOT textPayload=~"open\(\) \".*\" failed \(2: No such file or directory\)"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "level"
      value_type  = "STRING"
      description = "nginx's level for the line: warn, error, crit, alert or emerg"
    }
  }

  label_extractors = {
    "level" = "REGEXP_EXTRACT(textPayload, \"\\\\[(warn|error|crit|alert|emerg)\\\\]\")"
  }
}

resource "google_monitoring_alert_policy" "frontend_nginx_faults" {
  project      = var.project_id
  display_name = "dravr-frontend-nginx-faults"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      The frontend's nginx logged a proxy or server fault in the last 5 minutes.
      Requests may still be succeeding: nginx retries the next upstream address,
      so a connect failure can serve a 200 at added latency. That is how a week of
      IPv6 connect failures stayed invisible until 2026-09-26.

      The error lines:

        gcloud logging read 'resource.labels.service_name="${var.service_name}-frontend" AND logName="projects/${var.project_id}/logs/run.googleapis.com%2Fstderr" AND textPayload=~"\[(warn|error|crit|alert|emerg)\]"' --project=${var.project_id} --freshness=30m --limit=20

      The requests they affected (the access log is JSON; WARNING = served after
      abandoning an address, ERROR = 5xx):

        gcloud logging read 'resource.labels.service_name="${var.service_name}-frontend" AND severity>=WARNING AND jsonPayload.path:*' --project=${var.project_id} --freshness=30m --limit=20

      The nginx config is docker/images/frontend/nginx.conf; scripts/ci/nginx-routing-check.sh
      runs it against a stub upstream.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "nginx fault on the frontend in 5min window"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.frontend_nginx_faults.name}\""
      duration        = "0s"
      comparison      = "COMPARISON_GT"
      threshold_value = 0

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_SUM"
        cross_series_reducer = "REDUCE_SUM"
        group_by_fields      = ["metric.label.level"]
      }

      trigger {
        count = 1
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    # A broken upstream keeps logging for as long as it is broken; the
    # incident stays open while lines keep coming and closes 30 minutes after.
    auto_close = "1800s"
  }
}
