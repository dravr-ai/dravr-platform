# ABOUTME: Input variables for Firebase module
# ABOUTME: Accepts project IDs and the service accounts granted on the Firebase project

variable "project_id" {
  description = "GCP project ID where Firebase is configured"
  type        = string
}

variable "firebase_project_id" {
  description = "Firebase project ID"
  type        = string
}

variable "runtime_service_account_email" {
  description = "Email of the Cloud Run runtime service account, which deletes Firebase identities on account deletion"
  type        = string
}

variable "terraform_runner_service_account_email" {
  description = "Email of the terraform runner service account, which manages IAM on the Firebase project"
  type        = string
}
