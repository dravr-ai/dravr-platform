# ABOUTME: Variables for the centralized dravr-artifacts Terraform configuration
# ABOUTME: Controls registry naming, GitHub integration, and cross-environment reader access

variable "project_id" {
  description = "GCP project ID for the centralized dravr-artifacts project"
  type        = string

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{4,28}[a-z0-9]$", var.project_id))
    error_message = "Project ID must be 6-30 lowercase letters, digits, or hyphens."
  }
}

variable "region" {
  description = "GCP region for the Artifact Registry"
  type        = string
  default     = "northamerica-northeast1"
}

variable "registry_name" {
  description = "Name of the Artifact Registry Docker repository"
  type        = string
  default     = "dravr-images"
}

variable "github_org" {
  description = "GitHub organization that owns the repository"
  type        = string
  default     = "dravr-ai"
}

variable "github_repo" {
  description = "GitHub repository name"
  type        = string
  default     = "dravr-platform"
}

variable "env_app_sa_emails" {
  description = "Emails of Cloud Run app service accounts across all environments that need read access to the registry"
  type        = list(string)
  default     = []
}

variable "labels" {
  description = "Common labels to apply to all resources"
  type        = map(string)
  default = {
    app        = "dravr"
    managed_by = "terraform"
  }
}
