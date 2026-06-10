# ABOUTME: BigQuery usage-analytics dataset + federated Cloud SQL read for pricing analysis
# ABOUTME: A daily scheduled query rolls usage/cost out of Postgres into BQ; Looker reads BQ, never the live api service

# Dataset that holds the usage/cost rollups Looker Studio queries for pricing
# analysis. Reading it never touches the cpu_idle=false api Cloud Run service
# (no observer effect — see ADR-019).
resource "google_bigquery_dataset" "usage" {
  project                     = var.project_id
  dataset_id                  = var.dataset_id
  friendly_name               = "Dravr usage & pricing analytics"
  description                 = "Daily usage/cost rollups from Cloud SQL for pricing analysis. Read by Looker Studio; never queries the live backend."
  location                    = var.location
  labels                      = var.labels
  default_table_expiration_ms = var.table_expiration_ms
}

# Federated read connection to the Cloud SQL Postgres instance, so a scheduled
# query can EXTERNAL_QUERY the live tables directly (daily aggregates only — no
# CDC infra). The connection runs as a Google-managed service account.
resource "google_bigquery_connection" "cloudsql" {
  project       = var.project_id
  location      = var.location
  connection_id = "${var.dataset_id}-cloudsql"
  friendly_name = "Cloud SQL (usage) federated read"

  cloud_sql {
    instance_id = var.cloudsql_connection_name
    database    = var.cloudsql_database
    type        = "POSTGRES"
    credential {
      username = var.cloudsql_user
      password = var.cloudsql_password
    }
  }
}

# The federated connection's managed SA needs Cloud SQL client to read Postgres.
resource "google_project_iam_member" "connection_cloudsql_client" {
  project = var.project_id
  role    = "roles/cloudsql.client"
  member  = "serviceAccount:${google_bigquery_connection.cloudsql.cloud_sql[0].service_account_id}"
}

# Service account that the scheduled rollup query runs as.
resource "google_service_account" "rollup" {
  project      = var.project_id
  account_id   = "${var.dataset_id}-rollup"
  display_name = "BigQuery usage rollup scheduled query"
}

resource "google_bigquery_dataset_iam_member" "rollup_editor" {
  project    = var.project_id
  dataset_id = google_bigquery_dataset.usage.dataset_id
  role       = "roles/bigquery.dataEditor"
  member     = "serviceAccount:${google_service_account.rollup.email}"
}

resource "google_project_iam_member" "rollup_job_user" {
  project = var.project_id
  role    = "roles/bigquery.jobUser"
  member  = "serviceAccount:${google_service_account.rollup.email}"
}

# Wrap the operator-tunable Postgres query in an EXTERNAL_QUERY against the
# federated connection. `federated_sql` selects daily aggregates from the live
# usage table; its columns MUST match the live schema (see variables.tf).
locals {
  rollup_query = <<-SQL
    SELECT * FROM EXTERNAL_QUERY(
      "${google_bigquery_connection.cloudsql.id}",
      """${var.federated_sql}"""
    )
  SQL
}

# Daily scheduled query: append the latest rollup rows into the destination table.
resource "google_bigquery_data_transfer_config" "daily_rollup" {
  project                = var.project_id
  location               = var.location
  display_name           = "usage-daily-rollup"
  data_source_id         = "scheduled_query"
  schedule               = var.rollup_schedule
  destination_dataset_id = google_bigquery_dataset.usage.dataset_id
  service_account_name   = google_service_account.rollup.email

  params = {
    query                           = local.rollup_query
    destination_table_name_template = var.rollup_table
    write_disposition               = "WRITE_APPEND"
  }
}
