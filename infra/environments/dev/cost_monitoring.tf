# ABOUTME: Daily GCP cost alert — a scheduled query over the billing export posts to #gcp-cost-alerts on a breach
# ABOUTME: Added 2026-10-02 (carnet#731) so registry egress and total spend are watched in CAD, not inferred from build counts
#
# Background
# ----------
# The bill tracks build churn, not traffic: through September 2026 Artifact
# Registry internet egress to GitHub-hosted runners was ~CA$3.5–8 of a
# ~CA$10–14 day. The GitHub-side monitor (billing-drivers-monitor.yml) counts
# builds but cannot see cost, and giving it BigQuery access would mean a second
# WIF identity for a read this project can do natively. The monthly budget in
# billing.tf only emails, and GCP rejects Slack channels for budget alerts.
#
# How it works
# ------------
# A BigQuery scheduled query runs each morning and reads the previous UTC day
# from the billing export (complete ~5 h after midnight UTC). It upserts that
# day's figures into cost_monitor.daily, then RAISEs when a threshold is
# crossed or the export has no rows for the day. A RAISE fails the transfer
# run, Data Transfer logs the failure at ERROR, and the log-match policy below
# posts the RAISE message to Slack. Nothing runs outside GCP and nothing is
# deployed: the alert is the run failing.
#
# Two things in the export are easy to get wrong, and the query handles both:
#   * Registry egress is billed to dravr-artifacts, not dravr-dev, so the
#     query reads the whole billing account's export, not one project.
#   * The monthly invoice tax lands as a cost_type = 'tax' row on the 1st
#     (CA$45.94 on 2026-09-01); only 'regular' rows are counted.
#
# Verify or backfill a day by hand: start a manual run of the transfer config
# for run time D+1 (Console › BigQuery › Scheduled queries › Schedule backfill).
# @run_date drives the day, so a backfill re-evaluates history, including the
# alert. 2026-10-01 (55.6 GiB) breaches both default thresholds.
#
# PREREQUISITE — Slack: the channel named by slack_cost_channel must exist and
# the Dravr Slack app (slack_bot_token) must be a member, or GCP's deliveries
# are dropped by Slack without an error on this side.

# -----------------------------------------------------------------------------
# Tuning knobs
# -----------------------------------------------------------------------------

variable "slack_cost_channel" {
  description = "Slack channel NAME for the daily cost alert — must stay a #name for GCP Monitoring. The Dravr Slack app must be a member."
  type        = string
  default     = "#gcp-cost-alerts"

  validation {
    condition     = can(regex("^#?[a-z0-9][a-z0-9_-]{0,79}$", var.slack_cost_channel))
    error_message = "slack_cost_channel must be a Slack channel name (lowercase letters, digits, - and _, at most 80 characters), optionally prefixed with #, never a channel ID."
  }
}

variable "cost_alert_ar_egress_gib" {
  description = "Alert when Artifact Registry internet egress for a UTC day exceeds this many GiB. Before CI built binaries outside Docker (2026-10-02) every busy day was 25–55 GiB; after it a build pulls ~0.1 GiB, so 10 GiB means the cache pulls are back."
  type        = number
  default     = 10

  validation {
    condition     = var.cost_alert_ar_egress_gib > 0
    error_message = "cost_alert_ar_egress_gib must be greater than 0; at 0 or below every day with any registry egress alerts."
  }
}

variable "cost_alert_daily_cad" {
  description = "Alert when the billing account's net regular cost for a UTC day exceeds this many CAD (credits applied, tax excluded). September 2026 ran CA$8–14.5/day with ~CA$4–8 of registry egress; without that egress the floor is ~CA$4–9."
  type        = number
  default     = 12

  validation {
    condition     = var.cost_alert_daily_cad > 0
    error_message = "cost_alert_daily_cad must be greater than 0; at 0 or below every day with any spend alerts."
  }
}

locals {
  billing_export_table = "${var.project_id}.${google_bigquery_dataset.billing_export.dataset_id}.gcp_billing_export_v1_${replace(var.billing_account_id, "-", "_")}"
  cost_monitor_table   = "${var.project_id}.${google_bigquery_dataset.cost_monitor.dataset_id}.${google_bigquery_table.cost_monitor_daily.table_id}"

  # One UTC day per run, the day before @run_date. _PARTITIONTIME is the export
  # day, which trails usage by hours to days, so the partition window opens a
  # day early and closes five days late; usage_start_time picks the day.
  cost_monitor_sql = <<-SQL
    DECLARE day DATE DEFAULT DATE_SUB(@run_date, INTERVAL 1 DAY);
    DECLARE ar_gib FLOAT64;
    DECLARE ar_cad FLOAT64;
    DECLARE total_cad FLOAT64;
    DECLARE row_count INT64;
    -- Not named after the breaches column: inside the MERGE a column wins
    -- over a script variable of the same name.
    DECLARE breach_list ARRAY<STRING>;

    SET (ar_gib, ar_cad, total_cad, row_count) = (
      SELECT AS STRUCT
        IFNULL(SUM(IF(sku.description LIKE 'Artifact Registry Network Internet Egress%', usage.amount_in_pricing_units, 0)), 0),
        IFNULL(SUM(IF(sku.description LIKE 'Artifact Registry Network Internet Egress%', cost, 0)), 0),
        IFNULL(SUM(cost + IFNULL((SELECT SUM(c.amount) FROM UNNEST(credits) AS c), 0)), 0),
        COUNT(*)
      FROM `${local.billing_export_table}`
      WHERE cost_type = 'regular'
        AND DATE(usage_start_time) = day
        AND DATE(_PARTITIONTIME) BETWEEN DATE_SUB(day, INTERVAL 1 DAY) AND DATE_ADD(day, INTERVAL 5 DAY)
    );

    SET breach_list = ARRAY(
      SELECT b FROM UNNEST([
        IF(ar_gib > ${var.cost_alert_ar_egress_gib}, FORMAT('registry egress %.1f GiB > ${var.cost_alert_ar_egress_gib} GiB', ar_gib), NULL),
        IF(total_cad > ${var.cost_alert_daily_cad}, FORMAT('total CA$%.2f > ${var.cost_alert_daily_cad} CAD', total_cad), NULL)
      ]) AS b
      WHERE b IS NOT NULL
    );

    MERGE `${local.cost_monitor_table}` AS t
    USING (SELECT day AS day) AS s
    ON t.day = s.day
    WHEN MATCHED THEN UPDATE SET
      evaluated_at = CURRENT_TIMESTAMP(), billing_rows = row_count,
      ar_egress_gib = ar_gib, ar_egress_cad = ar_cad, total_net_cad = total_cad,
      breaches = ARRAY_TO_STRING(breach_list, '; ')
    WHEN NOT MATCHED THEN INSERT
      (day, evaluated_at, billing_rows, ar_egress_gib, ar_egress_cad, total_net_cad, breaches)
      VALUES (day, CURRENT_TIMESTAMP(), row_count, ar_gib, ar_cad, total_cad, ARRAY_TO_STRING(breach_list, '; '));

    IF row_count = 0 THEN
      RAISE USING MESSAGE = FORMAT('COST ALERT %t: the billing export has no rows for this day; the export is late or has stopped, so nothing was checked', day);
    END IF;

    IF ARRAY_LENGTH(breach_list) > 0 THEN
      RAISE USING MESSAGE = FORMAT(
        'COST ALERT %t: %s. Registry egress %.1f GiB = CA$%.2f, total CA$%.2f',
        day, ARRAY_TO_STRING(breach_list, ' and '), ar_gib, ar_cad, total_cad);
    END IF;
  SQL
}

# -----------------------------------------------------------------------------
# History table and the identity the scheduled query runs as
# -----------------------------------------------------------------------------

resource "google_bigquery_dataset" "cost_monitor" {
  project       = var.project_id
  dataset_id    = "cost_monitor"
  friendly_name = "Daily cost monitor"
  description   = "One row per UTC day written by the cost-monitor scheduled query (cost_monitoring.tf): registry egress, total net cost, and which thresholds it breached."
  location      = var.region

  labels = {
    app         = "dravr"
    managed_by  = "terraform"
    environment = "development"
    purpose     = "cost-monitor"
  }
}

resource "google_bigquery_table" "cost_monitor_daily" {
  project             = var.project_id
  dataset_id          = google_bigquery_dataset.cost_monitor.dataset_id
  table_id            = "daily"
  deletion_protection = false

  schema = jsonencode([
    { name = "day", type = "DATE", mode = "REQUIRED", description = "UTC usage day evaluated" },
    { name = "evaluated_at", type = "TIMESTAMP", mode = "REQUIRED", description = "When the scheduled query last evaluated this day" },
    { name = "billing_rows", type = "INT64", mode = "REQUIRED", description = "Regular-cost export rows found for the day; 0 means the export had not landed" },
    { name = "ar_egress_gib", type = "FLOAT64", mode = "REQUIRED", description = "Artifact Registry internet egress, GiB" },
    { name = "ar_egress_cad", type = "FLOAT64", mode = "REQUIRED", description = "Artifact Registry internet egress cost, CAD" },
    { name = "total_net_cad", type = "FLOAT64", mode = "REQUIRED", description = "Billing account regular cost with credits applied, CAD" },
    { name = "breaches", type = "STRING", mode = "NULLABLE", description = "Thresholds crossed, empty when none" },
  ])
}

resource "google_service_account" "cost_monitor" {
  project      = var.project_id
  account_id   = "cost-monitor"
  display_name = "Daily cost monitor scheduled query"
  description  = "Runs the cost-monitor BigQuery scheduled query: reads billing_export, writes cost_monitor."
}

resource "google_project_iam_member" "cost_monitor_job_user" {
  project = var.project_id
  role    = "roles/bigquery.jobUser"
  member  = "serviceAccount:${google_service_account.cost_monitor.email}"
}

resource "google_bigquery_dataset_iam_member" "cost_monitor_reads_billing" {
  project    = var.project_id
  dataset_id = google_bigquery_dataset.billing_export.dataset_id
  role       = "roles/bigquery.dataViewer"
  member     = "serviceAccount:${google_service_account.cost_monitor.email}"

  # Dataset IAM needs the runner's bigquery.admin (modules/service_accounts).
  depends_on = [module.service_accounts]
}

resource "google_bigquery_dataset_iam_member" "cost_monitor_writes_history" {
  project    = var.project_id
  dataset_id = google_bigquery_dataset.cost_monitor.dataset_id
  role       = "roles/bigquery.dataEditor"
  member     = "serviceAccount:${google_service_account.cost_monitor.email}"

  # Dataset IAM needs the runner's bigquery.admin (modules/service_accounts).
  depends_on = [module.service_accounts]
}

# Creating a transfer config that runs as a service account requires actAs on
# it. The Data Transfer service agent already mints its tokens through the
# project-level roles/bigquerydatatransfer.serviceAgent binding.
resource "google_service_account_iam_member" "terraform_runner_can_act_as_cost_monitor" {
  service_account_id = google_service_account.cost_monitor.name
  role               = "roles/iam.serviceAccountUser"
  member             = "serviceAccount:${module.service_accounts.terraform_runner_service_account_email}"
}

# -----------------------------------------------------------------------------
# The scheduled query
# -----------------------------------------------------------------------------

resource "google_bigquery_data_transfer_config" "cost_monitor" {
  project              = var.project_id
  location             = var.region
  display_name         = "cost-monitor-daily"
  data_source_id       = "scheduled_query"
  schedule             = "every day 13:00"
  service_account_name = google_service_account.cost_monitor.email

  params = {
    query = local.cost_monitor_sql
  }

  depends_on = [
    google_project_iam_member.cost_monitor_job_user,
    google_bigquery_dataset_iam_member.cost_monitor_reads_billing,
    google_bigquery_dataset_iam_member.cost_monitor_writes_history,
    google_service_account_iam_member.terraform_runner_can_act_as_cost_monitor,
    module.service_accounts,
  ]
}

# -----------------------------------------------------------------------------
# Slack channel and the alert on a failed run
# -----------------------------------------------------------------------------

resource "google_monitoring_notification_channel" "slack_cost_alerts" {
  project      = var.project_id
  display_name = "Dravr Slack Cost Alerts (#${trimprefix(var.slack_cost_channel, "#")})"
  type         = "slack"
  description  = "Routes the daily cost-monitor alert to the cost channel."

  labels = {
    "channel_name" = startswith(var.slack_cost_channel, "#") ? var.slack_cost_channel : "#${var.slack_cost_channel}"
  }

  sensitive_labels {
    auth_token = data.google_secret_manager_secret_version.slack_bot_token.secret_data
  }

  lifecycle {
    ignore_changes = [sensitive_labels[0].auth_token]
  }
}

resource "google_monitoring_alert_policy" "daily_cost" {
  project      = var.project_id
  display_name = "GCP daily cost over threshold"
  combiner     = "OR"

  documentation {
    subject   = "GCP cost alert: $${log.extracted_label.detail}"
    mime_type = "text/markdown"
    content   = <<-EOT
      $${log.extracted_label.detail}

      The daily cost-monitor scheduled query (`cost-monitor-daily`, BigQuery in
      ${var.project_id}) found the previous UTC day over a threshold, or found
      no billing rows for it. Limits: registry egress
      ${var.cost_alert_ar_egress_gib} GiB, total ${var.cost_alert_daily_cad} CAD.

      * Registry egress over the limit: image builds are pulling layers out of
        Artifact Registry again. Check that publish-images.yml still builds with
        `BINARY_SOURCE=prebuilt` and `mode=min`, and count the day's builds in
        the "Monitor: Build Cost Drivers" workflow.
      * Total over the limit: break the day down by SKU with
        `gcp-floor.sh report` (vault runbook "GCP Cost Floor — Measure and
        Trim"), or query `${local.billing_export_table}`.
      * No billing rows: the export is late or stopped; check Billing › Billing
        export in the console.

      History: `${local.cost_monitor_table}`.
    EOT
  }

  conditions {
    display_name = "cost-monitor-daily run failed"

    condition_matched_log {
      filter = <<-EOT
        resource.type = "bigquery_dts_config"
        AND resource.labels.config_id = "${element(split("/", google_bigquery_data_transfer_config.cost_monitor.name), 5)}"
        AND severity >= ERROR
      EOT

      label_extractors = {
        "detail" = "REGEXP_EXTRACT(textPayload, \"(COST ALERT[^;\\\"]*)\")"
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_cost_alerts.id]

  alert_strategy {
    notification_rate_limit {
      period = "3600s"
    }
    auto_close = "86400s"
  }
}
