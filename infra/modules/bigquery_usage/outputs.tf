# ABOUTME: Outputs for the bigquery_usage module
# ABOUTME: Dataset/connection identifiers + the rollup service account for Looker IAM wiring

output "dataset_id" {
  description = "The BigQuery usage dataset ID."
  value       = google_bigquery_dataset.usage.dataset_id
}

output "dataset_self_link" {
  description = "Self link of the usage dataset."
  value       = google_bigquery_dataset.usage.self_link
}

output "connection_id" {
  description = "Federated Cloud SQL connection ID (for EXTERNAL_QUERY)."
  value       = google_bigquery_connection.cloudsql.id
}

output "rollup_service_account_email" {
  description = "SA running the scheduled rollup query (grant Looker read on the dataset separately)."
  value       = google_service_account.rollup.email
}
