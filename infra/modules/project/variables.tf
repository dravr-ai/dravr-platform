# ABOUTME: Variables for the project module
# ABOUTME: Configures which GCP project to enable APIs in

variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "enable_cloud_trace" {
  description = "Enable the Telemetry (OTLP) and Cloud Trace APIs that the cloud_run module's OpenTelemetry Collector sidecar writes traces through. Off by default; an API left enabled after turning it off costs nothing (disable_on_destroy = false)."
  type        = bool
  default     = false
}
