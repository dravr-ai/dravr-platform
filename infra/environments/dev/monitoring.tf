# ABOUTME: Cloud Run job failure alerting, abandoned-OAuth-launch alerting, and the Slack channel both route to
# ABOUTME: Job half added 2026-05-25 (c6630e46's silent exit-1); OAuth half 2026-09-11 (the blank-popup outage)
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
#        * pierre-cli check-drift agents failures (the contremaitre-vs-DB
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

# -----------------------------------------------------------------------------
# OAuth launches that expire without ever completing
# -----------------------------------------------------------------------------
#
# Background
# ----------
# On 2026-09-10 users reported that connecting Strava was broken — "the popup is
# blank". Every signal we kept said the server was healthy: it minted a valid
# authorize URL and answered HTTP 200. The flow simply never came back. No
# callback, no error, nothing logged.
#
# The gap was structural, not a missing log line. We measured the requests we
# ANSWERED and never the flows we were still WAITING ON, so a failure that lives
# entirely in the gap between the redirect going out and the callback coming
# back could not show up anywhere — not in error rates, not in latency, not in
# the 5xx count. Finding it cost a full day of correlating nginx access logs by
# hand.
#
# The server-side sweeper closes that gap: every 15 minutes it reaps OAuth
# launches that aged out without a callback and reports the count per provider
# (crates/pierre-services/src/oauth_launch_sweeper.rs). The resources below turn
# that WARN line into a paging signal, so the next occurrence of this failure
# class announces itself instead of waiting for a user to complain.

resource "google_logging_metric" "oauth_launches_abandoned" {
  project = var.project_id
  name    = "dravr-oauth-launches-abandoned"

  description = "Counts sweeper passes that found at least one OAuth launch expired without ever completing — a redirect went out to the provider and no callback ever came back. Each log line is one sweep, carrying how many launches it reaped and the per-provider breakdown. Added 2026-09-11 after the 2026-09-10 outage where connecting Strava returned a blank popup: the authorize URL was valid and answered 200, the flow never returned, and nothing we measured could see it."

  # Matched on the marker substring alone, deliberately. The two sibling metrics
  # in this directory also pin `labels."rust.target"`, which is tighter — but the
  # sweeper is landing in a parallel change and guessing its module path would
  # produce a filter that matches nothing while looking perfectly healthy. The
  # marker is unambiguous enough to stand on its own; tighten it once the
  # emitting module is real.
  #
  # Severity is not pinned either, following the identity-leak metric: the
  # message is definitive, and a metric that keeps counting after someone
  # re-levels the log line is worth more than one that silently stops.
  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    jsonPayload.message=~"oauth launches expired without completing"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "reaped"
      value_type  = "STRING"
      description = "How many launches this sweep reaped. Magnitude separates a trickle of ordinary drop-off from a flow that is failing for everyone at once."
    }

    labels {
      key         = "providers"
      value_type  = "STRING"
      description = "Per-provider breakdown of the reaped launches. The fastest triage key there is: the 2026-09-10 outage was Strava-only while every other provider connected fine, which is what told us to look at that provider's redirect configuration rather than at our own handler."
    }
  }

  label_extractors = {
    "reaped"    = "EXTRACT(jsonPayload.reaped)"
    "providers" = "EXTRACT(jsonPayload.providers)"
  }
}

# Cloud Logging creates the metric immediately, but its descriptor takes up to
# ~10 minutes to reach Cloud Monitoring, so an alert policy created in the same
# apply 404s with "Cannot find metric(s) that match type = ...". `depends_on`
# orders Terraform's graph, not GCP's eventual consistency, so it cannot help.
# Observed on the first apply of wire_shape_monitoring.tf, 2026-08-05; this
# metric and its policy are born in the same apply, which is exactly that shape.
# Costs ten minutes once, on create only.
resource "time_sleep" "oauth_abandonment_metric_propagation" {
  depends_on      = [google_logging_metric.oauth_launches_abandoned]
  create_duration = "600s"
}

resource "google_monitoring_alert_policy" "oauth_launches_abandoned" {
  project      = var.project_id
  display_name = "OAuth launches are expiring without ever completing"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      The OAuth sweeper reported expired-without-completing launches on more
      than one separate pass in the last hour. Users are starting provider
      connections that never come back.

      ## Why this alert exists

      On 2026-09-10, connecting Strava was broken and presented as "the popup is
      blank". The server looked perfectly healthy from every angle we monitored:
      it issued a valid authorize URL and returned HTTP 200. The flow just never
      returned — no callback, no error, nothing logged.

      That is the whole problem this alert solves. We measured the requests we
      answered and never the flows we were still waiting on, so a failure living
      in the gap between the redirect leaving and the callback arriving was
      invisible to error rates, latency and status codes alike. It took a full
      day of correlating nginx access logs by hand to find. It should take this
      alert instead.

      ## Why "more than one pass in an hour" and not the first one

      Some abandonment is completely normal and will never stop: a user opens
      the consent screen, changes their mind, closes the window, and their launch
      expires. That is a single sweeper line in a single pass, followed by
      silence. Paging on one occurrence would page on ordinary drop-off, and this
      channel would be muted inside a week.

      A real breakage differs in SHAPE, not in size. When the flow itself is
      broken, every launch started during the outage expires, so the sweeper
      reports on pass after pass for as long as the outage lasts. The
      discriminator is therefore repetition across sweeps.

      The sweeper runs every 15 minutes and emits at most one line per pass, so
      an hour holds at most four of them. "More than one per hour" is therefore
      two separate passes, at least 15 minutes apart, that each found a dead
      launch — while a burst from one user retrying inside a single 15-minute
      window collapses into one line and stays silent. It trips at 2 against a
      ceiling of 4, so it has room to fire without needing a flawless outage.

      That headroom is the point. This threshold is calibrated against the
      incident itself: 2026-09-10 produced four dead launches across two people
      in total. A threshold demanding a high per-hour count would have missed
      the very outage this exists to catch — what gave that outage away was
      that launches kept dying pass after pass, not that many died at once.

      Two consequences worth knowing. A mass outage still waits for the second
      pass, so detection lags up to ~30 minutes. And the metric counts PASSES,
      not launches, because a counter log-metric increments once per matching
      line however large `reaped` is; magnitude lives in the label, for triage
      after the page rather than for deciding to page.

      If the sweeper's cadence ever changes, revisit this number with it — the
      two are coupled, and `SWEEP_INTERVAL` in
      crates/pierre-services/src/oauth_launch_sweeper.rs is the source of truth.

      ## First step

      Read the sweeper lines. `providers` says which provider is dying and
      `reaped` says how fast:

          gcloud logging read \
            'resource.type="cloud_run_revision" AND
             resource.labels.service_name="${var.service_name}-api" AND
             jsonPayload.message=~"oauth launches expired without completing"' \
            --project ${var.project_id} --freshness=1h --limit 20 \
            --format="value(timestamp, jsonPayload.reaped, jsonPayload.providers)"

      Then check for the 2026-09-10 signature directly — authorize requests
      landing while callbacks do not:

          gcloud logging read \
            'resource.type="cloud_run_revision" AND
             resource.labels.service_name="${var.service_name}-api" AND
             httpRequest.requestUrl:"/api/oauth/"' \
            --project ${var.project_id} --freshness=1h --limit 50 \
            --format="value(timestamp, httpRequest.status, httpRequest.requestUrl)"

      A healthy provider shows `/api/oauth/authorize/<provider>` followed by
      `/api/oauth/callback/<provider>`. Authorize hits with no matching callback
      is the outage reproducing, and it means the break is downstream of us: the
      redirect is leaving and nothing is coming back. Look outside the handler —
      the redirect URI registered with the provider, `BASE_URL` on the running
      revision, and the consent screen itself.

      If a single provider is reaped while the others stay clean, that provider's
      configuration is the suspect. If every provider is reaped at once, suspect
      something shared: `BASE_URL`, the callback route, or ingress.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "More than one sweep in an hour found abandoned OAuth launches"

    condition_threshold {
      # >1, not >0: one abandoned launch an hour is a user changing their mind,
      # and paging on it would train everyone to ignore this channel. Two
      # separate passes reporting inside one hour is the outage shape. The
      # sweeper's 15-minute cadence caps an hour at four lines, so this trips at
      # 2 with real headroom rather than demanding a perfect run of passes.
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.oauth_launches_abandoned.name}\""
      comparison      = "COMPARISON_GT"
      threshold_value = 1
      duration        = "0s"

      aggregations {
        # Summed across series rather than grouped by provider. `providers` is a
        # breakdown string, so grouping on it would split one outage into several
        # low-count series, none of which reaches the threshold — the alert would
        # get quieter exactly as the breakage got broader. The labels are for
        # triage after the page, not for deciding whether to page.
        alignment_period     = "3600s"
        per_series_aligner   = "ALIGN_DELTA"
        cross_series_reducer = "REDUCE_SUM"
      }

      trigger {
        count = 1
      }
    }
  }

  # Same Slack channel as every other policy in this directory — an OAuth flow
  # that never returns produces no other operator-visible signal, which is the
  # whole reason 2026-09-10 went unnoticed until users complained.
  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    # Closes once launches start completing again, so a fixed outage stops
    # nagging without someone acknowledging it by hand.
    auto_close = "1800s"
  }

  depends_on = [time_sleep.oauth_abandonment_metric_propagation]
}
