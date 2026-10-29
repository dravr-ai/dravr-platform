# ABOUTME: Variables for the service_accounts module
# ABOUTME: Configures service account naming

variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "service_name" {
  description = "Name of the service (used as prefix for SA account IDs)"
  type        = string
}

variable "artifacts_project_id" {
  description = "GCP project ID of the centralized dravr-artifacts project (for cross-project image pull access)"
  type        = string
}

variable "tf_state_bucket" {
  description = "GCS bucket name used for Terraform state (grants terraform-runner objectAdmin on this bucket)"
  type        = string
}

variable "enable_cloud_trace" {
  description = "Grant the app service account roles/telemetry.tracesWriter, so the cloud_run module's OpenTelemetry Collector sidecar can write spans to Cloud Trace through the Telemetry API. Off by default."
  type        = bool
  default     = false
}
