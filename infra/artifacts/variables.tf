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

# -----------------------------------------------------------------------------
# Image retention / cleanup policy
# -----------------------------------------------------------------------------

variable "cleanup_policy_dry_run" {
  description = "When true, cleanup policies are evaluated and logged but no images are deleted. Set false to enforce."
  type        = bool
  default     = true
}

variable "recent_versions_keep_count" {
  description = "Number of most-recent versions to protect from deletion per package (rollback window)."
  type        = number
  default     = 20

  validation {
    condition     = var.recent_versions_keep_count >= 1
    error_message = "recent_versions_keep_count must be at least 1."
  }
}

variable "stale_tag_retention_days" {
  description = "Age after which tagged images are deleted, unless protected by a keep policy (recent window or release tags)."
  type        = number
  default     = 30

  validation {
    condition     = var.stale_tag_retention_days >= 1
    error_message = "stale_tag_retention_days must be at least 1."
  }
}

variable "untagged_retention_days" {
  description = "Age after which untagged (orphaned) images are deleted."
  type        = number
  default     = 3

  validation {
    condition     = var.untagged_retention_days >= 1
    error_message = "untagged_retention_days must be at least 1."
  }
}

variable "release_tag_prefixes" {
  description = "Tag prefixes for images kept indefinitely (deploy / rollback anchors: semver releases, plus the moving deployed-<env> tag each Cloud Run deploy applies to the digest it ships)."
  type        = list(string)
  default     = ["v", "deployed-"]
}
