# ABOUTME: Outputs from the Cloud Run v2 service module
# ABOUTME: Provides service URL, name, and ID for integration with other resources

output "service_url" {
  description = "URL of the deployed Cloud Run service"
  value       = google_cloud_run_v2_service.service.uri
}

output "service_name" {
  description = "Name of the Cloud Run service"
  value       = google_cloud_run_v2_service.service.name
}

output "service_id" {
  description = "Full resource ID of the Cloud Run service"
  value       = google_cloud_run_v2_service.service.id
}
