# ABOUTME: Manages Firebase Identity Platform for authentication (Google Sign-In, Email/Password)
# ABOUTME: Configures authorized domains, sign-in providers; reads OAuth credentials from Secret Manager

# Enable required APIs
resource "google_project_service" "identity_toolkit" {
  project            = var.project_id
  service            = "identitytoolkit.googleapis.com"
  disable_on_destroy = false
}

resource "google_project_service" "firebase_auth" {
  project            = var.project_id
  service            = "firebaseauth.googleapis.com"
  disable_on_destroy = false
}

resource "google_project_service" "iap" {
  project            = var.project_id
  service            = "iap.googleapis.com"
  disable_on_destroy = false
}

# Read Google OAuth credentials from Secret Manager
data "google_secret_manager_secret_version" "google_oauth_client_id" {
  project = var.project_id
  secret  = "google-oauth-client-id"
}

data "google_secret_manager_secret_version" "google_oauth_client_secret" {
  project = var.project_id
  secret  = "google-oauth-client-secret"
}

# Identity Platform configuration (authorized domains, sign-in settings)
resource "google_identity_platform_config" "auth" {
  project = var.project_id

  authorized_domains = concat(
    [
      "localhost",
      "${var.firebase_project_id}.firebaseapp.com",
      "${var.firebase_project_id}.web.app",
    ],
    var.authorized_domains,
  )

  sign_in {
    allow_duplicate_emails = false

    email {
      enabled           = true
      password_required = true
    }
  }

  depends_on = [
    google_project_service.identity_toolkit,
    google_project_service.firebase_auth,
  ]
}

# Google Sign-In provider configuration
resource "google_identity_platform_default_supported_idp_config" "google" {
  project = var.project_id
  idp_id  = "google.com"
  enabled = true

  client_id     = data.google_secret_manager_secret_version.google_oauth_client_id.secret_data
  client_secret = data.google_secret_manager_secret_version.google_oauth_client_secret.secret_data

  depends_on = [google_identity_platform_config.auth]
}
