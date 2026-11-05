# ABOUTME: Log-based metrics and alerts for the public protocol doors: /mcp, /a2a and /oauth2, and consent refused
# ABOUTME: Reads nginx's access log on the frontend service and the api's consent refusals; sends no request, so it keeps no service awake
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Marketplace clients reach Dravr through these paths. They do not report a failure to anyone: a connector that gets a 5xx shows its user
# a generic error, and a host refused with 429 simply stops refreshing. The
# frontend's nginx sees every such request, with its status, in the JSON access
# log (docker/images/frontend/nginx.conf), so these need no code to be deployed.
#
# /oauth2 is shared with Dravr's own sign-in: the web and mobile apps open
# /oauth2/authorize and /oauth2/login, and redeem at /oauth/token, which is not
# matched here. A 5xx on those is an athlete who cannot sign in, so it belongs
# here too. Nginx's line does not carry the client_id that would tell them apart.
#
# 401 and 400 are left out on purpose. A 401 is how every OAuth connection
# starts (it carries the challenge), and a 400 is a client's own mistake.
#
# User-defined log-based metrics are chargeable Cloud Monitoring custom metrics
# (Google Cloud Observability pricing). They read logs the services already write
# and send no request.

# A metric's label block is immutable and a metric a policy references cannot be
# deleted (see frontend_monitoring.tf), so a new meaning is a new metric.
resource "google_logging_metric" "public_protocol_responses" {
  project = var.project_id
  name    = "dravr-public-protocol-responses"

  description = "Counts 5xx and 429 answers on /mcp, /a2a and /oauth2 at the frontend's nginx, by route and status."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-frontend"
    log_id("run.googleapis.com/stdout")
    jsonPayload.path=~"^/(mcp|a2a|oauth2)(/|$)"
    (jsonPayload.status>=500 OR jsonPayload.status=429)
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "route"
      value_type  = "STRING"
      description = "mcp, a2a or oauth2"
    }
    labels {
      key         = "status"
      value_type  = "STRING"
      description = "The HTTP status nginx answered with"
    }
  }

  label_extractors = {
    "route"  = "REGEXP_EXTRACT(jsonPayload.path, \"^/(mcp|a2a|oauth2)\")"
    "status" = "EXTRACT(jsonPayload.status)"
  }
}

# Every refusal of an account the consent path logs (`oauth2/consent.rs`), by its
# reason; a stale consent form logs `OAuth consent refused` and is not one. The
# counter stays readable for the expected ones, an account still waiting on its
# confirmation link among them; only the reason that means a stranger cannot
# finish connecting pages.
resource "google_logging_metric" "consent_refused" {
  project = var.project_id
  name    = "dravr-oauth-consent-refused"

  description = "Counts OAuth authorizations the consent path refused because of the account, by reason."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    log_id("run.googleapis.com/stdout")
    jsonPayload.message=~"^OAuth authorization refused: "
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "reason"
      value_type  = "STRING"
      description = "The refusal, as the log line words it"
    }
  }

  label_extractors = {
    "reason" = "REGEXP_EXTRACT(jsonPayload.message, \"^OAuth authorization refused: (.*)$\")"
  }
}

# The metric descriptors take up to ~10 minutes to reach Cloud Monitoring; a
# policy created before that 404s (see wire_shape_monitoring.tf).
resource "time_sleep" "public_mcp_metric_propagation" {
  depends_on = [
    google_logging_metric.public_protocol_responses,
    google_logging_metric.consent_refused,
  ]
  create_duration = "600s"
}

resource "google_monitoring_alert_policy" "public_protocol_5xx" {
  project      = var.project_id
  display_name = "Public connector failing: 5xx on /mcp, /a2a or /oauth2"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      More than 2 requests in 5 minutes got a 5xx on one of the routes marketplace
      clients use. Their users see a generic connector error and nobody tells us.
      /oauth2 also carries Dravr's own sign-in, so there it can be an athlete who
      cannot sign in to the app.

      What to do:
      1. Logs Explorer on the frontend service: `jsonPayload.path=~"^/(mcp|a2a|oauth2)"`
         and `jsonPayload.status>=500`. `upstream_status` says whether the api
         answered it (a real 5xx) or nginx did with no upstream answer.
      2. With no upstream answer, look at the api's revision and at
         dravr-frontend-nginx-faults, then check for a rollout in progress.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "More than 2 public protocol 5xx in 5 minutes on one route"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.public_protocol_responses.name}\" AND metric.labels.status!=\"429\""
      comparison      = "COMPARISON_GT"
      threshold_value = 2
      duration        = "0s"

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_DELTA"
        cross_series_reducer = "REDUCE_SUM"
        group_by_fields      = ["metric.label.route"]
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = "1800s"
  }

  depends_on = [time_sleep.public_mcp_metric_propagation]
}

resource "google_monitoring_alert_policy" "public_protocol_429" {
  project      = var.project_id
  display_name = "Public connector throttled: 429 on /mcp, /a2a or /oauth2"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      More than 20 requests in 5 minutes were refused with 429 on one of the routes
      marketplace clients use. /oauth2/token is metered per address, and a connector
      host refreshes all its athletes from a few egress addresses, so a large host
      reaching `OAUTH_TOKEN_RATE_LIMIT_RPM` looks exactly like this. /oauth2/authorize,
      /oauth2/register and the /oauth2/login password form are metered per address too.

      What to do:
      1. Logs Explorer on the frontend service: `jsonPayload.status=429` for the
         path, and the load balancer's request log for `httpRequest.remoteIp`.
      2. One host's egress range refreshing normally: raise that limit. Many
         addresses, or a path nobody should be calling at that rate: abuse.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "More than 20 public protocol 429 in 5 minutes on one route"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.public_protocol_responses.name}\" AND metric.labels.status=\"429\""
      comparison      = "COMPARISON_GT"
      threshold_value = 20
      duration        = "0s"

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_DELTA"
        cross_series_reducer = "REDUCE_SUM"
        group_by_fields      = ["metric.label.route"]
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = "1800s"
  }

  depends_on = [time_sleep.public_mcp_metric_propagation]
}

resource "google_monitoring_alert_policy" "consent_refused_awaiting_approval" {
  project      = var.project_id
  display_name = "Connector blocked: consent refused an account awaiting approval"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      An athlete signed in from a connector and was refused before the consent
      screen because their account waits for an operator's approval. With self-serve
      sign-up on, that should not happen: a stranger cannot finish connecting,
      which is the first thing a directory reviewer would hit.

      What to do:
      1. Logs Explorer on the api service:
         `jsonPayload.message:"OAuth authorization refused: the account awaits approval"`.
         `user_id` names the account.
      2. Check why the account is Pending: the sign-up policy and its daily
         limit, then approve it if it is a real athlete.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Any consent refused for an account awaiting approval"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.consent_refused.name}\" AND metric.labels.reason=\"the account awaits approval\""
      comparison      = "COMPARISON_GT"
      threshold_value = 0
      duration        = "0s"

      aggregations {
        alignment_period   = "300s"
        per_series_aligner = "ALIGN_DELTA"
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = "1800s"
  }

  depends_on = [time_sleep.public_mcp_metric_propagation]
}
