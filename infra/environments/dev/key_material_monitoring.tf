# ABOUTME: Alerts on key-material failures: a stored secret that will not decrypt, a DEK or signing key that cannot load or is missing beside ciphertext
# ABOUTME: Log-based metric on jsonPayload.event "key_material.*" routed to the same Slack channel as the error alert (carnet#703)
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# On 2026-09-30 the stored Database Encryption Key was replaced and every
# stored secret stopped decrypting (carnet#696). From 13:19 to 16:25 UTC the
# API logged 133 "Decryption failed" lines and a fresh DEK minted over a
# database that held one, all at WARNING or INFO, so the in-process error
# alert (ERROR only) never fired; the only Slack signal was two generic 500s.
#
# Every key-material failure now logs at ERROR with an `event` field under
# `key_material.` (crates/pierre-database/src/backends/shared/key_material.rs
# names them). The in-process alert covers a serving instance; this policy
# covers what it cannot:
#
#   - a failure that aborts boot (the DEK or the signing keypair cannot be
#     read or unwrapped, or is missing on a database that holds ciphertext):
#     the process exits before the in-process notifier's batch window
#     flushes;
#   - more than one instance: the in-process dedup is per instance, while
#     this is one incident per event and secret kind across the service,
#     open while the lines keep coming;
#   - a log level that regresses: the filter matches the event, never the
#     severity, so a repeat logged at WARNING still counts.

resource "google_logging_metric" "key_material_failures" {
  project = var.project_id
  name    = "dravr-key-material-failures"

  description = "Counts key-material failure lines on the API service (jsonPayload.event key_material.*): stored-secret decrypt failures, DEK read or unwrap failures, signing-keypair load failures, and a boot refused because the DEK or signing keypair is missing on a database that holds ciphertext. Labelled by event and, for decrypt failures, by the kind of secret."

  # The event prefix is pinned by a test in the platform repo
  # (crates/pierre-server/tests/key_material_alert_test.rs), which reads this
  # filter and checks every event the code emits against it.
  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    jsonPayload.event=~"^key_material[.]"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "event"
      value_type  = "STRING"
      description = "The key-material event, e.g. key_material.stored_secret_decrypt_failed"
    }

    labels {
      key         = "secret_kind"
      value_type  = "STRING"
      description = "For a stored-secret decrypt failure, which secret: oauth_access_token, oauth_refresh_token, rsa_private_key, tenant_oauth_client_secret, strava_pool_client_secret or llm_api_key"
    }
  }

  label_extractors = {
    "event"       = "EXTRACT(jsonPayload.event)"
    "secret_kind" = "EXTRACT(jsonPayload.secret_kind)"
  }
}

# Cloud Logging creates the metric immediately, but its descriptor takes up to
# ~10 minutes to reach Cloud Monitoring, so a policy created in the same apply
# 404s (see monitoring.tf, oauth_abandonment_metric_propagation). Costs ten
# minutes once, on create only.
resource "time_sleep" "key_material_metric_propagation" {
  depends_on      = [google_logging_metric.key_material_failures]
  create_duration = "600s"
}

resource "google_monitoring_alert_policy" "key_material_failures" {
  project      = var.project_id
  display_name = "dravr-key-material-failures"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      A key-material failure was logged on the API service in the last 5
      minutes. The event label says which:

      - `key_material.stored_secret_decrypt_failed` — a stored secret (the
        secret_kind label: OAuth tokens, signing key, tenant or Strava pool
        client secret, LLM API key) will not decrypt. Every read of that
        secret is failing: athletes cannot sync, sign-in may break.
      - `key_material.dek_read_failed` / `key_material.dek_unwrap_failed` —
        the Database Encryption Key could not be read or unwrapped; the
        revision refuses to boot.
      - `key_material.rsa_keypair_load_failed` — the JWT signing keypair could
        not be read or decrypted; the revision refuses to boot.
      - `key_material.dek_minted_over_ciphertext` /
        `key_material.rsa_keypair_minted_over_ciphertext` — the DEK row in
        system_secrets, or every rsa_keypairs row, is missing on a database
        that holds ciphertext. The instance refused to mint a replacement
        (a new key could open none of the data, or would sign out every
        session) and refuses to boot until the original row is back. Nothing
        was written: the data is intact, only its key row is gone.

      Do not rotate, mint or delete anything, and do not empty the encrypted
      tables to get a boot through. Recover the key that sealed the data with
      the carnet#696 recovery procedure (dravr-ai/dravr-carnet#696, the
      "Timeline and recovery" comment):

        1. Restore a point-in-time clone of the Cloud SQL instance from
           before the row went missing, and read the original row from it:
           the wrapped DEK (secret_type `database_encryption_key`, plus any
           `database_encryption_key_v<N>` and the active-version row) or the
           rsa_keypairs row.
        2. Check it opens the live ciphertext before writing: every stored
           value must decrypt under it (dry run first).
        3. Write the original row back on the live database (a Cloud Run
           job against the private IP, as dravr-dek-recovery did); keep any
           row it replaces as a stash row until the incident is closed.
        4. Roll the service and confirm "initialized at active DEK version"
           and the persisted RSA keypair load in the boot logs.

      Read the lines:

        gcloud logging read 'resource.labels.service_name="${var.service_name}-api" AND jsonPayload.event=~"^key_material[.]"' --project=${var.project_id} --freshness=1h --limit=50

      Background: the 2026-09-30 DEK replacement logged 133 decrypt failures
      at WARNING and nothing alerted for three hours (carnet#696, #703).
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Key-material failure in 5min window"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.key_material_failures.name}\""
      duration        = "0s"
      comparison      = "COMPARISON_GT"
      threshold_value = 0

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_SUM"
        cross_series_reducer = "REDUCE_SUM"
        # One incident per event and secret kind: while one kind's incident
        # is open, a second kind failing opens its own and notifies again.
        group_by_fields = ["metric.label.event", "metric.label.secret_kind"]
      }

      trigger {
        count = 1
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    # A lost key fails every read until it is restored; the incident stays
    # open while the lines keep coming and closes 30 minutes after they stop.
    auto_close = "1800s"
  }

  depends_on = [time_sleep.key_material_metric_propagation]
}
