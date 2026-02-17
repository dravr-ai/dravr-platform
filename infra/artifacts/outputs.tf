# ABOUTME: Outputs from the centralized artifacts Terraform configuration
# ABOUTME: Provides values needed for GitHub secrets/variables and environment Terraform

output "registry_url" {
  description = "Artifact Registry URL for docker push/pull (set as GCP_ARTIFACTS_REGISTRY_URL GitHub variable)"
  value       = "${var.region}-docker.pkg.dev/${var.project_id}/${var.registry_name}"
}

output "workload_identity_provider" {
  description = "Workload Identity Provider name (set as GCP_ARTIFACTS_WIF_PROVIDER GitHub secret)"
  value       = google_iam_workload_identity_pool_provider.github.name
}

output "image_publisher_sa_email" {
  description = "Image publisher service account email (set as GCP_IMAGE_PUBLISHER_SA GitHub secret)"
  value       = google_service_account.image_publisher.email
}
