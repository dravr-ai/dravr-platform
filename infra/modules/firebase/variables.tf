# ABOUTME: Input variables for Firebase module
# ABOUTME: Accepts project IDs for API state tracking

variable "project_id" {
  description = "GCP project ID where Firebase is configured"
  type        = string
}

variable "firebase_project_id" {
  description = "Firebase project ID"
  type        = string
}
