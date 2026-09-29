# ABOUTME: Outputs from Firebase module
# ABOUTME: Exposes project ID for reference

output "firebase_project_id" {
  description = "Firebase project ID"
  value       = var.firebase_project_id
}

# The Google OAuth web client's secrets, by Secret Manager id. The backend
# reads them as GOOGLE_OAUTH_CLIENT_ID / GOOGLE_OAUTH_CLIENT_SECRET for
# "Continue with Google" on the hosted OAuth login page (carnet#652); only the
# ids leave this module, never the payloads.
output "google_oauth_secret_ids" {
  description = "Secret Manager ids of the Google OAuth web client's id and secret"
  value = {
    client_id     = data.google_secret_manager_secret_version.google_oauth_client_id.secret
    client_secret = data.google_secret_manager_secret_version.google_oauth_client_secret.secret
  }
}
