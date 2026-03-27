# ABOUTME: Tracks GCP API enablements related to Firebase authentication
# ABOUTME: APIs were auto-enabled during Firebase project creation; Terraform tracks state only

# Identity Toolkit API — auto-enabled when Firebase project was created
resource "google_project_service" "identity_toolkit" {
  project            = var.project_id
  service            = "identitytoolkit.googleapis.com"
  disable_on_destroy = false
}

# Cloud Identity-Aware Proxy API
resource "google_project_service" "iap" {
  project            = var.project_id
  service            = "iap.googleapis.com"
  disable_on_destroy = false
}

# OAuth credentials stored in Secret Manager for reference by other modules
data "google_secret_manager_secret_version" "google_oauth_client_id" {
  project = var.project_id
  secret  = "google-oauth-client-id"
}

data "google_secret_manager_secret_version" "google_oauth_client_secret" {
  project = var.project_id
  secret  = "google-oauth-client-secret"
}
