# ABOUTME: Variables for the Cloud Run v2 Job module
# ABOUTME: Configurable container, networking, env vars, and execution settings

variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "region" {
  description = "GCP region"
  type        = string
}

variable "job_name" {
  description = "Name of the Cloud Run job"
  type        = string
}

variable "container_image" {
  description = "Container image to run (e.g., region-docker.pkg.dev/project/repo/image:tag)"
  type        = string
}

variable "service_account_email" {
  description = "Service account email for the Cloud Run job"
  type        = string
}

# -----------------------------------------------------------------------------
# Container Configuration
# -----------------------------------------------------------------------------

variable "command" {
  description = "Entrypoint command (overrides Dockerfile ENTRYPOINT)"
  type        = list(string)
  default     = []
}

variable "args" {
  description = "Arguments passed to the entrypoint command"
  type        = list(string)
  default     = []
}

variable "cpu" {
  description = "CPU allocation (e.g., '1', '2')"
  type        = string
  default     = "1"
}

variable "memory" {
  description = "Memory allocation (e.g., '512Mi', '1Gi')"
  type        = string
  default     = "512Mi"
}

# -----------------------------------------------------------------------------
# Execution Configuration
# -----------------------------------------------------------------------------

variable "max_retries" {
  description = "Maximum number of retries per task (0 = no retries)"
  type        = number
  default     = 1
}

variable "timeout" {
  description = "Maximum execution time per task (e.g., '300s', '600s')"
  type        = string
  default     = "300s"
}

variable "execution_environment" {
  description = "Cloud Run execution environment (EXECUTION_ENVIRONMENT_GEN1 or EXECUTION_ENVIRONMENT_GEN2)"
  type        = string
  default     = "EXECUTION_ENVIRONMENT_GEN2"
}

# -----------------------------------------------------------------------------
# Networking Configuration
# -----------------------------------------------------------------------------

variable "vpc_connector_id" {
  description = "Serverless VPC connector ID for private network access (null to skip)"
  type        = string
  default     = null
}

variable "cloudsql_connection_name" {
  description = "Cloud SQL connection name for unix socket mounting (null to skip)"
  type        = string
  default     = null
}

# -----------------------------------------------------------------------------
# Environment Variables
# -----------------------------------------------------------------------------

variable "env_vars" {
  description = "Plain environment variables as key-value map"
  type        = map(string)
  default     = {}
}

variable "secret_env_vars" {
  description = "Secret Manager environment variables as map of env_name => secret_name (uses latest version)"
  type        = map(string)
  default     = {}
}

# -----------------------------------------------------------------------------
# Labels
# -----------------------------------------------------------------------------

variable "labels" {
  description = "Labels to apply to the Cloud Run job"
  type        = map(string)
  default     = {}
}
