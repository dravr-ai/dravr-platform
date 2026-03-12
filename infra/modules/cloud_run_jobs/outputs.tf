# ABOUTME: Outputs from the Cloud Run v2 Job module
# ABOUTME: Provides job name and ID for CI/CD integration and monitoring

output "job_name" {
  description = "Name of the Cloud Run job"
  value       = google_cloud_run_v2_job.job.name
}

output "job_id" {
  description = "Full resource ID of the Cloud Run job"
  value       = google_cloud_run_v2_job.job.id
}
