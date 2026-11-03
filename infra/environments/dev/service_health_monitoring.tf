# ABOUTME: Generic service-health alerts (5xx count, median latency), the service-health dashboard, and Log Analytics on _Default
# ABOUTME: Added 2026-11-03 (observability posture assessment, P1-7/P1-8): every other policy here matches one known failure
#
# Background
# ----------
# The twenty log-based policies in this directory each match one failure that
# already happened once. Nothing watched the service as a whole: a new failure
# showed up only in post-deploy-verify's 60-minute window, or when someone
# noticed. These two policies read Cloud Run's built-in request metrics, which
# Cloud Run records at the front end whatever the app logs — reading them never
# reaches the service, so they cost nothing against scale-to-zero (unlike the
# uptime checks removed in 97e8666eb).
#
# Thresholds come from the week to 2026-11-03 (Monitoring API, api service):
#   - ~45,000 requests, zero 5xx. Any 5xx is unusual, so the 5xx policy counts
#     them (3 or more in 15 minutes) instead of taking a ratio, which at alpha
#     traffic swings to 50% on one failure out of two requests.
#   - The 15-minute median was 137 ms at its median and 452 ms at p90, but a
#     long chat turn or provider scrape alone can push a quiet window's median
#     past 55 s, and the metric has no route label to exclude those routes.
#     So the latency policy fires on a median above 5 s only while the window
#     holds at least 30 requests: broad slowness, not one long turn.
#
# The dashboard reads the same built-in metrics plus Cloud SQL's, lists every
# policy's incidents, and draws each single-threshold policy through an
# alertChart, which renders the policy's own condition (each metric is written
# against its own resource type, so a hand-written query per metric would
# drift from the policy it mirrors).
# Viewing it reads Cloud Monitoring, never the service.

locals {
  api_service      = "${var.service_name}-api"
  frontend_service = "${var.service_name}-frontend"

  # Cloud Run request metrics, PromQL-named, for one service or both.
  run_requests_api  = "run_googleapis_com:request_count{monitored_resource=\"cloud_run_revision\",service_name=\"${local.api_service}\"}"
  run_requests_both = "run_googleapis_com:request_count{monitored_resource=\"cloud_run_revision\",service_name=~\"${local.api_service}|${local.frontend_service}\",response_code_class=\"5xx\"}"
  run_latency_api   = "run_googleapis_com:request_latencies_bucket{monitored_resource=\"cloud_run_revision\",service_name=\"${local.api_service}\"}"

  # Log-based and instance policies with a single metric-threshold condition,
  # each drawn on the dashboard through its own condition (an alertChart).
  charted_policies = [
    google_monitoring_alert_policy.turns_abandoned_at_shutdown,
    google_monitoring_alert_policy.messaging_turns_interrupted,
    google_monitoring_alert_policy.outbound_dead_lettered,
    google_monitoring_alert_policy.cloud_tasks_refused_turns,
    google_monitoring_alert_policy.resume_sweep_unreadable,
    google_monitoring_alert_policy.llm_tier_rate_limited,
    google_monitoring_alert_policy.llm_startup_probe_failed,
    google_monitoring_alert_policy.llm_chain_fallthrough,
    google_monitoring_alert_policy.first_party_authorize_refused,
    google_monitoring_alert_policy.claim_verification_panicked,
    google_monitoring_alert_policy.claim_verification_failed,
    google_monitoring_alert_policy.coach_identity_leak_survived_reask,
    google_monitoring_alert_policy.wire_shape_multi_system,
    google_monitoring_alert_policy.frontend_nginx_faults,
    google_monitoring_alert_policy.oauth_launches_abandoned,
    google_monitoring_alert_policy.job_failures,
    google_monitoring_alert_policy.instance_never_idle,
  ]

  # Policies an alertChart is not known to draw — several conditions, a
  # PromQL condition, a matched-log condition — listed in the incident list
  # only, so a widget the API refuses cannot fail the whole dashboard.
  incident_only_policies = [
    google_monitoring_alert_policy.service_5xx,
    google_monitoring_alert_policy.api_slow,
    google_monitoring_alert_policy.daily_cost,
    google_monitoring_alert_policy.database_capacity,
    google_monitoring_alert_policy.key_material_failures,
    google_monitoring_alert_policy.chat_tapped_share,
  ]

  # Dashboard grid: 48 columns; charts are half width, policy tiles a quarter.
  chart_width  = 24
  chart_height = 16
  tile_width   = 12
  tile_height  = 12

  service_charts = [
    {
      title = "API requests per second, by response class"
      queries = {
        "{{response_code_class}}" = "sum by (response_code_class) (rate(${local.run_requests_api}[5m]))"
      }
    },
    {
      title = "5xx responses per 15 minutes (api, frontend)"
      queries = {
        "{{service_name}}" = "sum by (service_name) (increase(${local.run_requests_both}[15m]))"
      }
    },
    {
      title = "API latency (ms): p50, p95, p99"
      queries = {
        "p50" = "histogram_quantile(0.5, sum by (le) (rate(${local.run_latency_api}[15m])))"
        "p95" = "histogram_quantile(0.95, sum by (le) (rate(${local.run_latency_api}[15m])))"
        "p99" = "histogram_quantile(0.99, sum by (le) (rate(${local.run_latency_api}[15m])))"
      }
    },
    {
      title = "Instances by state (api, frontend)"
      queries = {
        "{{service_name}} {{state}}" = "sum by (service_name, state) (run_googleapis_com:container_instance_count{monitored_resource=\"cloud_run_revision\",service_name=~\"${local.api_service}|${local.frontend_service}\"})"
      }
    },
    {
      title = "Billable instance seconds per hour (scale-to-zero health)"
      queries = {
        "{{service_name}}" = "sum by (service_name) (increase(run_googleapis_com:container_billable_instance_time{monitored_resource=\"cloud_run_revision\",service_name=~\"${local.api_service}|${local.frontend_service}\"}[1h]))"
      }
    },
    {
      title = "Cloud SQL: open connections and CPU utilization"
      queries = {
        "connections" = "sum(cloudsql_googleapis_com:database_postgresql_num_backends{monitored_resource=\"cloudsql_database\"})"
        "cpu (x100)"  = "100 * max(cloudsql_googleapis_com:database_cpu_utilization{monitored_resource=\"cloudsql_database\"})"
      }
    },
  ]

  service_chart_tiles = [
    for i, chart in local.service_charts : {
      xPos   = (i % 2) * local.chart_width
      yPos   = local.chart_height + floor(i / 2) * local.chart_height
      width  = local.chart_width
      height = local.chart_height
      widget = {
        title = chart.title
        xyChart = {
          chartOptions = { mode = "COLOR" }
          dataSets = [
            for legend, query in chart.queries : {
              plotType           = "LINE"
              legendTemplate     = legend
              targetAxis         = "Y1"
              timeSeriesQuery    = { prometheusQuery = query }
              minAlignmentPeriod = "60s"
            }
          ]
        }
      }
    }
  ]

  policies_top = local.chart_height + ceil(length(local.service_charts) / 2) * local.chart_height

  policy_tiles = [
    for i, policy in local.charted_policies : {
      xPos   = (i % 4) * local.tile_width
      yPos   = local.policies_top + floor(i / 4) * local.tile_height
      width  = local.tile_width
      height = local.tile_height
      widget = {
        title      = policy.display_name
        alertChart = { name = policy.name }
      }
    }
  ]
}

resource "google_monitoring_alert_policy" "service_5xx" {
  project      = var.project_id
  display_name = "Server errors (5xx) on the api or frontend"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      Three or more 5xx responses in 15 minutes on `${local.api_service}` or
      `${local.frontend_service}`. The week this policy was written had none
      in ~45,000 requests, so even a few mean something broke.

      ## First step

      Read the failing requests, newest first. The app's failure line carries
      the request's user, tenant, `http_request_id` and trace, so the trace links
      to every line the request logged:

          gcloud logging read \
            'resource.type="cloud_run_revision" AND
             resource.labels.service_name="${local.api_service}" AND
             jsonPayload.message="response failed" AND
             jsonPayload.http_status>=500' \
            --project ${var.project_id} --freshness=1h --limit 50 \
            --format="value(timestamp, jsonPayload.http_status, jsonPayload.http_path, jsonPayload.user_id, trace)"

      Then open one line's trace in Logs Explorer ("Show entries for this
      trace") to see the whole request. A 5xx on the frontend only, with the
      api clean, is nginx itself (see the nginx-faults policy).
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "3 or more 5xx in 15 minutes, per service"

    condition_prometheus_query_language {
      query               = "sum by (service_name) (increase(${local.run_requests_both}[15m])) >= 3"
      duration            = "0s"
      evaluation_interval = "60s"
    }
  }

  # Same Slack channel as every other policy in this directory.
  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = "3600s"
  }
}

resource "google_monitoring_alert_policy" "api_slow" {
  project      = var.project_id
  display_name = "API median latency above 5 s under real traffic"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      The median request to `${local.api_service}` took longer than 5 s over
      15 minutes holding at least 30 requests. A single long chat turn
      cannot do this: half of all requests have to be slow, which points at
      something every route shares — the database, the connection pool, an
      instance starved of CPU, or the LLM chain blocking request threads.

      ## First step

      Check the dashboard "Dravr — Service health": Cloud SQL connections
      and CPU, and whether instances are pinned at the max. Then read the
      slowest requests of the window:

          gcloud logging read \
            'resource.type="cloud_run_revision" AND
             resource.labels.service_name="${local.api_service}" AND
             logName="projects/${var.project_id}/logs/run.googleapis.com%2Frequests" AND
             httpRequest.latency>="5s"' \
            --project ${var.project_id} --freshness=30m --limit 50 \
            --format="value(timestamp, httpRequest.latency, httpRequest.requestUrl, trace)"

      Cloud Run's request line carries the trace; every app line the request
      wrote carries the same one, with its user and route.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Median latency above 5000 ms, with at least 30 requests in 15 minutes"

    condition_prometheus_query_language {
      query               = <<-EOT
        histogram_quantile(0.5, sum by (le) (rate(${local.run_latency_api}[15m]))) > 5000
        and
        sum(increase(${local.run_requests_api}[15m])) >= 30
      EOT
      duration            = "300s"
      evaluation_interval = "60s"
    }
  }

  # Same Slack channel as every other policy in this directory.
  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = "3600s"
  }
}

resource "google_monitoring_dashboard" "service_health" {
  project = var.project_id

  dashboard_json = jsonencode({
    displayName = "Dravr — Service health"
    mosaicLayout = {
      columns = 48
      tiles = concat(
        [
          {
            xPos   = 0
            yPos   = 0
            width  = 48
            height = local.chart_height
            widget = {
              title = "Open and recent incidents"
              incidentList = {
                # The incident list names a policy without its project
                # prefix (`alertPolicies/<id>`), unlike the alert charts below.
                policyNames = [
                  for policy in concat(local.incident_only_policies, local.charted_policies) :
                  regex("alertPolicies/[^/]+$", policy.name)
                ]
              }
            }
          },
        ],
        local.service_chart_tiles,
        local.policy_tiles,
      )
    }
  })
}

# Log Analytics on the _Default bucket: SQL over the logs in Logs Explorer's
# Log Analytics page, at no added charge for the upgrade or for queries run
# there (a linked BigQuery dataset, which this does not create, is what bills).
# ONE-WAY: Cloud Logging cannot downgrade a bucket once upgraded.
#
# Retention stays at 30 days on purpose: the published privacy policy says
# server logs "expire on their own within about a month"
# (dravr-website src/content/legal/privacy.md). Raising it is a policy change
# first, then a one-line change here.
#
# The _Default bucket already exists; this resource adopts and updates it in
# place rather than creating one, and removing it only drops it from state.
resource "google_logging_project_bucket_config" "default_log_analytics" {
  project          = var.project_id
  location         = "global"
  bucket_id        = "_Default"
  retention_days   = 30
  enable_analytics = true
}
