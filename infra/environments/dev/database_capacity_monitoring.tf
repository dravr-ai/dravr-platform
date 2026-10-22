# ABOUTME: Alerts when Postgres runs out of connection slots or a boot cannot read its keys because the database is unavailable
# ABOUTME: Two log-based metrics (Cloud SQL's slot-exhaustion FATAL, the API's database.key_store_unavailable) on one Slack policy (carnet#1130)
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# On 2026-10-21 at 18:48 UTC the rollout of dravr-mcp-server-api-01426 started
# three instances while the outgoing revision still held its pool. Postgres
# logged 423 "remaining connection slots are reserved" FATALs in five minutes,
# four booting instances timed out reading the DEK, and the only page was the
# key-material alert telling on-call to restore the key from a point-in-time
# clone: the key was fine, the database was full (carnet#1129, #1130).
#
# A key-store read the database could not serve now logs
# `database.key_store_unavailable` (crates/pierre-database/src/backends/shared/key_material.rs),
# outside the `key_material.` namespace the key-loss policy matches; it lands
# here, beside Postgres's own account of the exhaustion.

resource "google_logging_metric" "postgres_connection_slots_exhausted" {
  project = var.project_id
  name    = "dravr-postgres-connection-slots-exhausted"

  description = "Counts Postgres refusing a connection because every slot is taken (SQLSTATE 53300: 'remaining connection slots are reserved' or 'too many clients already')."

  filter = <<-EOT
    resource.type="cloudsql_database"
    textPayload:("remaining connection slots are reserved" OR "too many clients already")
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"
  }
}

resource "google_logging_metric" "key_store_unavailable" {
  project = var.project_id
  name    = "dravr-key-store-unavailable"

  description = "Counts API boots that could not read the DEK or the signing keypair because the database could not serve the read (no free connection slot, closed pool, lost connection). The key is not in question; the instance refuses to boot and mints nothing."

  # The event name is pinned by a test in the platform repo
  # (crates/pierre-server/tests/it/key_material_alert_test.rs), which reads this
  # filter and checks the event the code emits against it.
  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    jsonPayload.event="database.key_store_unavailable"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"
  }
}

# The descriptors take up to ~10 minutes to reach Cloud Monitoring; a policy
# created in the same apply 404s without the wait (see key_material_monitoring.tf).
resource "time_sleep" "database_capacity_metric_propagation" {
  depends_on = [
    google_logging_metric.postgres_connection_slots_exhausted,
    google_logging_metric.key_store_unavailable,
  ]
  create_duration = "600s"
}

resource "google_monitoring_alert_policy" "database_capacity" {
  project      = var.project_id
  display_name = "dravr-database-capacity"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      Postgres ran out of connection slots, or an API instance could not read
      its keys because the database could not serve the read. Nothing is
      wrong with the keys: do NOT follow the key-material recovery procedure,
      and do not rotate, mint or delete anything.

      Most often this is a deploy: the incoming revision's instances open
      their pools while the outgoing revision still holds its own, and the
      two together exceed the instance's slots. Booting instances then time
      out on the DEK read, refuse to boot, and Cloud Run replaces them until
      the old revision drains (about five minutes on 2026-10-21).

      Check, in order:

        1. Was a revision rolling out? `gcloud run revisions list --service=${var.service_name}-api --region=${var.region} --project=${var.project_id} --limit=3`
        2. Did it settle? Look for "initialized at active DEK version" in the
           boot logs of the new revision after the alert.
        3. Who holds the slots: `max_instances × POSTGRES_MAX_CONNECTIONS`
           per revision, twice during a rollout, plus any Cloud Run job.

      If it recurs outside deploys, or a revision never settles, the budget
      in terraform.tfvars no longer fits the tier: carnet#1129.

      Read the lines:

        gcloud logging read 'resource.type="cloudsql_database" AND textPayload:"remaining connection slots are reserved"' --project=${var.project_id} --freshness=1h --limit=20
        gcloud logging read 'resource.labels.service_name="${var.service_name}-api" AND jsonPayload.event="database.key_store_unavailable"' --project=${var.project_id} --freshness=1h --limit=20
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Postgres connection slots exhausted in 5min window"

    condition_threshold {
      filter          = "resource.type=\"cloudsql_database\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.postgres_connection_slots_exhausted.name}\""
      duration        = "0s"
      comparison      = "COMPARISON_GT"
      threshold_value = 0

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_SUM"
        cross_series_reducer = "REDUCE_SUM"
      }

      trigger {
        count = 1
      }
    }
  }

  conditions {
    display_name = "API boot could not read its keys from the database in 5min window"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.key_store_unavailable.name}\""
      duration        = "0s"
      comparison      = "COMPARISON_GT"
      threshold_value = 0

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_SUM"
        cross_series_reducer = "REDUCE_SUM"
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

  depends_on = [time_sleep.database_capacity_metric_propagation]
}
