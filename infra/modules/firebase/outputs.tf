# ABOUTME: Outputs from Firebase Identity Platform module
# ABOUTME: Exposes authorized domains and project configuration

output "authorized_domains" {
  description = "List of authorized domains for Firebase Auth"
  value       = google_identity_platform_config.auth.authorized_domains
}

output "firebase_project_id" {
  description = "Firebase project ID"
  value       = var.firebase_project_id
}
