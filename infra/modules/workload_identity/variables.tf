# ABOUTME: Variables for the workload_identity module
# ABOUTME: Configures GitHub organization, repo, and service account binding

variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "github_org" {
  description = "GitHub organization or username"
  type        = string
}

variable "github_repo" {
  description = "GitHub repository name"
  type        = string
}

variable "deployer_service_account_name" {
  description = "Full resource name of the deployer service account"
  type        = string
}

variable "terraform_runner_service_account_name" {
  description = "Full resource name of the terraform runner service account (bound to the WIF pool for plan/apply)"
  type        = string
}
