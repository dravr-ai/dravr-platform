# ABOUTME: Inputs for the bigquery_usage module (dataset, federated Cloud SQL connection, scheduled rollup)
# ABOUTME: Source data is the live usage/cost tables; consumed by Looker Studio for pricing analysis

variable "project_id" {
  description = "GCP project ID."
  type        = string
}

variable "location" {
  description = "BigQuery dataset + connection location (e.g. northamerica-northeast1)."
  type        = string
}

variable "dataset_id" {
  description = "BigQuery dataset ID for usage/pricing rollups."
  type        = string
  default     = "dravr_usage"
}

variable "labels" {
  description = "Resource labels."
  type        = map(string)
  default     = {}
}

variable "table_expiration_ms" {
  description = "Default table expiration in ms (0 = never expire). Rollups are small; default to no expiry."
  type        = number
  default     = 0
}

variable "cloudsql_connection_name" {
  description = "Cloud SQL instance connection name (project:region:instance) — from the database module's connection_name output."
  type        = string
}

variable "cloudsql_database" {
  description = "Postgres database name to federate."
  type        = string
}

variable "cloudsql_user" {
  description = "Read user for the federated connection."
  type        = string
}

variable "cloudsql_password" {
  description = "Password for the federated read user (sensitive)."
  type        = string
  sensitive   = true
}

variable "rollup_schedule" {
  description = "Scheduled-query cadence (BigQuery Data Transfer schedule syntax)."
  type        = string
  default     = "every day 05:00"
}

variable "rollup_table" {
  description = "Destination rollup table name template in the dataset."
  type        = string
  default     = "usage_daily"
}

variable "federated_sql" {
  description = <<-DESC
    The Postgres SELECT run via EXTERNAL_QUERY each day. **Its columns MUST match
    the live usage schema** — tune at apply time against `llm_usage_daily`. The
    default pulls the last two days of per-tenant/model token+cost aggregates
    (WRITE_APPEND + a 2-day lookback tolerates a missed run without gaps; dedupe
    downstream in the Looker view or a follow-up MERGE).
  DESC
  type        = string
  default     = <<-SQL
    SELECT day, tenant_id, COALESCE(model, '') AS model,
           SUM(input_tokens)  AS input_tokens,
           SUM(output_tokens) AS output_tokens,
           SUM(cost_usd)      AS cost_usd
    FROM llm_usage_daily
    WHERE day >= CURRENT_DATE - INTERVAL '2 days'
    GROUP BY day, tenant_id, model
  SQL
}
