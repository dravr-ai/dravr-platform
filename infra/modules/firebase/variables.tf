# ABOUTME: Input variables for Firebase Identity Platform module
# ABOUTME: Accepts project config and authorized domains; OAuth creds read from Secret Manager

variable "project_id" {
  description = "GCP project ID where Firebase is configured"
  type        = string
}

variable "firebase_project_id" {
  description = "Firebase project ID (used for default authorized domains)"
  type        = string
}

variable "authorized_domains" {
  description = "Additional domains authorized for Firebase Auth (e.g., Cloud Run frontend URLs)"
  type        = list(string)
  default     = []
}
