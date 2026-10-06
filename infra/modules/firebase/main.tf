# ABOUTME: Tracks GCP API enablements related to Firebase authentication
# ABOUTME: Also grants the runtime SA Firebase user deletion on the Firebase project (carnet#798)

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

# Account deletion also deletes the user's Google sign-in identity
# (carnet#798): the backend calls Identity Toolkit
# `projects/<firebase_project_id>/accounts:delete` with the runtime SA's ADC
# token. The Firebase project is separate from the platform project, so the
# grant lives there; firebaseauth.admin is the narrowest predefined role that
# holds firebaseauth.users.delete.
resource "google_project_iam_member" "runtime_firebaseauth_admin" {
  project = var.firebase_project_id
  role    = "roles/firebaseauth.admin"
  member  = "serviceAccount:${var.runtime_service_account_email}"
}

# The runner refreshes and changes the binding above, which sits in the
# Firebase project where its platform-project roles do not reach; without this
# every apply 403s reading that project's IAM policy. Like the KMS grant, it
# needs a one-time bootstrap by a Firebase project owner before the first
# apply can manage it:
#   gcloud projects add-iam-policy-binding <firebase_project_id> \
#     --member="serviceAccount:<terraform runner email>" \
#     --role="roles/resourcemanager.projectIamAdmin" --condition=None
resource "google_project_iam_member" "terraform_runner_firebase_iam_admin" {
  project = var.firebase_project_id
  role    = "roles/resourcemanager.projectIamAdmin"
  member  = "serviceAccount:${var.terraform_runner_service_account_email}"
}
