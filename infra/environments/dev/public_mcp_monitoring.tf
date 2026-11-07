# ABOUTME: Log-based metrics and alerts for the public protocol doors: /mcp, /a2a and /oauth2, consent refused, tool quota and per-user token refusals
# ABOUTME: Reads nginx's access log on the frontend service and the api's refusal lines; sends no request, so it keeps no service awake
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

# A tool call refused at its quota (`mcp/tool_handlers.rs`). An athlete reaching a
# cap is ordinary, so only a rate means anything. Its words are pinned by
# `mcp_tool_quota_alert_test.rs`.
resource "google_logging_metric" "mcp_tool_quota_exceeded" {
  project = var.project_id
  name    = "dravr-mcp-tool-quota-exceeded"

  description = "Counts /mcp tool calls refused because the account reached a usage cap."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    log_id("run.googleapis.com/stdout")
    jsonPayload.message=~"^Tool call quota exceeded: "
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"
  }
}

# A tool call refused because its quota could not be read. The public surface
# fails closed, so while this counts, tool calls are refused. Its words are pinned
# by `mcp_tool_quota_alert_test.rs`.
resource "google_logging_metric" "mcp_tool_quota_check_failed" {
  project = var.project_id
  name    = "dravr-mcp-tool-quota-check-failed"

  description = "Counts /mcp tool calls refused because the usage counters could not be read."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    log_id("run.googleapis.com/stdout")
    jsonPayload.message=~"^Failed to check tool call quota, refusing the call: "
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"
  }
}

# A grant redemption refused by its user's window at /oauth2/token
# (`oauth2_server/endpoints/grant_checks.rs`). A connection refreshes hourly at
# most, so no legitimate client should reach this. Its words are pinned by
# `oauth2_token_user_limit_test.rs`.
resource "google_logging_metric" "oauth_token_user_limited" {
  project = var.project_id
  name    = "dravr-oauth-token-user-limited"

  description = "Counts /oauth2/token requests refused because one user window was full."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    log_id("run.googleapis.com/stdout")
    jsonPayload.message=~"^OAuth token request refused: per-user window full"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"
  }
}

# The metric descriptors take up to ~10 minutes to reach Cloud Monitoring; a
# policy created before that 404s (see wire_shape_monitoring.tf).
resource "time_sleep" "public_mcp_metric_propagation" {
  depends_on = [
    google_logging_metric.public_protocol_responses,
    google_logging_metric.consent_refused,
    google_logging_metric.mcp_tool_quota_exceeded,
    google_logging_metric.mcp_tool_quota_check_failed,
    google_logging_metric.oauth_token_user_limited,
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
      /oauth2/token also refuses per user; that refusal has its own alert below.

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

resource "google_monitoring_alert_policy" "mcp_tool_quota_exceeded" {
  project      = var.project_id
  display_name = "Tool quota refusals climbing on /mcp"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      More than 20 /mcp tool calls in 5 minutes were refused at a usage cap. One
      athlete reaching a cap is ordinary, and a client that keeps calling can reach
      this alone; many accounts doing it at once is farmed accounts, or a cap set
      too low for real use.

      What to do:
      1. Logs Explorer on the api service:
         `jsonPayload.message:"Tool call quota exceeded"`. `user_id` and `tenant_id`
         name the account, the message names the cap.
      2. One account looping: look at its client. Many new accounts: read the
         sign-up policy and its daily ceiling. Many established athletes: the cap.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "More than 20 tool quota refusals in 5 minutes"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.mcp_tool_quota_exceeded.name}\""
      comparison      = "COMPARISON_GT"
      threshold_value = 20
      duration        = "0s"

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_DELTA"
        cross_series_reducer = "REDUCE_SUM"
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = "1800s"
  }

  depends_on = [time_sleep.public_mcp_metric_propagation]
}

resource "google_monitoring_alert_policy" "mcp_tool_quota_check_failed" {
  project      = var.project_id
  display_name = "Public connector refusing calls: tool quota cannot be read"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      A /mcp tool call was refused because its usage counters could not be read.
      The surface fails closed, so athletes get "Usage limits could not be verified
      right now" until the read works again. Every occurrence is a tool call that
      did not run.

      What to do:
      1. Logs Explorer on the api service:
         `jsonPayload.message:"Failed to check tool call quota"`. The message carries
         the database error.
      2. Look at the database: the dravr-database-capacity alert for connection
         slots, then a migration or a rollout in progress.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Any tool call refused because its quota could not be read"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.mcp_tool_quota_check_failed.name}\""
      comparison      = "COMPARISON_GT"
      threshold_value = 0
      duration        = "0s"

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_DELTA"
        cross_series_reducer = "REDUCE_SUM"
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = "1800s"
  }

  depends_on = [time_sleep.public_mcp_metric_propagation]
}

resource "google_monitoring_alert_policy" "oauth_token_user_limited" {
  project      = var.project_id
  display_name = "OAuth token refused: one athlete's grants redeemed too fast"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      /oauth2/token refused a request because one user's window was full
      (`OAUTH_TOKEN_USER_RATE_LIMIT_RPM`, 30 a minute by default). A real connection
      refreshes at most hourly, so this is a client stuck in a refresh loop or a
      stolen grant being replayed. The refused grant stays usable.

      What to do:
      1. Logs Explorer on the api service:
         `jsonPayload.message:"OAuth token request refused: per-user window full"`.
         `user_id`, `client_id` and `grant_type` name who.
      2. One client looping for one athlete: look at that client. Several clients or
         addresses on one athlete: treat the grant as stolen and revoke it.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Any token request refused by a user's window"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.oauth_token_user_limited.name}\""
      comparison      = "COMPARISON_GT"
      threshold_value = 0
      duration        = "0s"

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_DELTA"
        cross_series_reducer = "REDUCE_SUM"
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = "1800s"
  }

  depends_on = [time_sleep.public_mcp_metric_propagation]
}

# Cloud Armor refusals, off Google's own metric rather than a log-based one like
# those above: it is free, and Cloud Monitoring's descriptor endpoint answers 404
# for http_load_balancer, the resource type of the load balancer's log, so a
# policy on a log-based metric could not be checked before an apply. A rule in
# preview counts under previewed_request_count, so this stays silent until a rule
# is promoted. The metric has no route: every promoted rule counts here.
#
# Promoting a rule, after 7 days in preview:
# 1. For each rule N, read what it would have refused (1000 and 1100 are the WAF,
#    2000, 3500 and 4000 the rate rules; a line holds one preview match):
#      gcloud logging read 'resource.type="http_load_balancer"
#        jsonPayload.previewSecurityPolicy.priority=N
#        (jsonPayload.previewSecurityPolicy.outcome="DENY"
#         OR jsonPayload.previewSecurityPolicy.rateLimitAction.outcome:"EXCEED")
#        timestamp>="<7 days ago>"' --project=dravr-dev
#    Also confirm preview logging worked: the same query without the outcome clause
#    on rule 2000 or 4000 must list conforming requests. None means nothing was
#    measured, not that nothing would be refused.
# 2. A false positive is a real athlete, connector or webhook sender in those
#    lines, by path, address and preconfiguredExprIds. Fix that first (exclude
#    the signature or raise the limit), and restart the 7 days.
# 3. Set armor_waf_preview or armor_rate_rules_preview = false in terraform.tfvars,
#    land it, then gh workflow run terraform.yml -f environment=dev -f action=apply.
# 4. gcloud compute security-policies describe <policy> must show preview: false,
#    scripts/ci/first-party-sign-in-smoke.sh must pass, and this alert must fire
#    in a drill.
resource "google_monitoring_alert_policy" "armor_denied" {
  count = var.armor_enabled ? 1 : 0

  project      = var.project_id
  display_name = "Cloud Armor refusing requests at the load balancer"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      Cloud Armor refused more than 20 requests in 5 minutes on app.dravr.ai or
      mcp.dravr.ai. Only rules promoted out of preview count. It is either an attack
      or a limit that is too low for real traffic, and a refused client gets no
      Dravr error page and reports nothing to us.

      What to do:
      1. Logs Explorer: `resource.type="http_load_balancer"` and
         `jsonPayload.statusDetails="denied_by_security_policy"`. Group by
         `jsonPayload.enforcedSecurityPolicy.priority`, `httpRequest.remoteIp` and
         the path. 1000 and 1100 are the WAF, 2000 the per-token throttle, 3500 the
         anonymous throttle, 4000 the per-address ban.
      2. One address or token on paths nobody should call at that rate: abuse.
         A connector host, a webhook sender or many real athletes: put that rule
         back in preview (armor_waf_preview, armor_rate_rules_preview in
         terraform.tfvars, then the manual apply) and raise the limit.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "More than 20 requests refused by Cloud Armor in 5 minutes"

    condition_threshold {
      filter          = "resource.type=\"network_security_policy\" AND resource.labels.policy_name=\"${module.frontend_domain[0].security_policy_name}\" AND metric.type=\"networksecurity.googleapis.com/https/request_count\" AND metric.labels.blocked=\"true\""
      comparison      = "COMPARISON_GT"
      threshold_value = 20
      duration        = "0s"

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_DELTA"
        cross_series_reducer = "REDUCE_SUM"
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = "1800s"
  }
}
