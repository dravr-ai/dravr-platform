# ABOUTME: Variables for the generic Cloud Run v2 service module
# ABOUTME: Configurable container, scaling, networking, env vars, and health checks

variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "region" {
  description = "GCP region"
  type        = string
}

variable "service_name" {
  description = "Name of the Cloud Run service"
  type        = string
}

variable "container_image" {
  description = "Container image to deploy (e.g., region-docker.pkg.dev/project/repo/image:tag)"
  type        = string
}

variable "service_account_email" {
  description = "Service account email for the Cloud Run service"
  type        = string
}

# -----------------------------------------------------------------------------
# Container Configuration
# -----------------------------------------------------------------------------

variable "container_port" {
  description = "Port the container listens on"
  type        = number
  default     = 8080
}

variable "cpu" {
  description = "CPU allocation (e.g., '1', '2', '4')"
  type        = string
  default     = "1"
}

variable "memory" {
  description = "Memory allocation (e.g., '512Mi', '1Gi', '2Gi')"
  type        = string
  default     = "512Mi"
}

variable "cpu_idle" {
  description = "Allow CPU to be throttled when no requests (enables scale-to-zero cost savings)"
  type        = bool
  default     = true
}

variable "startup_cpu_boost" {
  description = "Temporarily allocate extra CPU during startup"
  type        = bool
  default     = true
}

# -----------------------------------------------------------------------------
# Scaling Configuration
# -----------------------------------------------------------------------------

variable "min_instances" {
  description = "Minimum number of instances (0 for scale-to-zero)"
  type        = number
  default     = 0
}

variable "max_instances" {
  description = "Maximum number of instances"
  type        = number
  default     = 10
}

# -----------------------------------------------------------------------------
# Networking Configuration
# -----------------------------------------------------------------------------

variable "ingress" {
  description = "Ingress traffic setting (INGRESS_TRAFFIC_ALL, INGRESS_TRAFFIC_INTERNAL_ONLY, INGRESS_TRAFFIC_INTERNAL_LOAD_BALANCER)"
  type        = string
  default     = "INGRESS_TRAFFIC_ALL"
}

variable "allow_unauthenticated" {
  description = "Allow unauthenticated access (public endpoint)"
  type        = bool
  default     = false
}

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
# Health Checks
# -----------------------------------------------------------------------------

variable "health_check_path" {
  description = "HTTP path for startup and liveness probes"
  type        = string
  default     = "/health"
}

variable "startup_probe_initial_delay" {
  description = "Initial delay in seconds before startup probe begins"
  type        = number
  default     = 5
}

# -----------------------------------------------------------------------------
# Execution Environment
# -----------------------------------------------------------------------------

variable "execution_environment" {
  description = "Cloud Run execution environment (EXECUTION_ENVIRONMENT_GEN1 or EXECUTION_ENVIRONMENT_GEN2). Gen2 provides better memory management and is recommended for stateful workloads."
  type        = string
  default     = "EXECUTION_ENVIRONMENT_GEN2"
}

# -----------------------------------------------------------------------------
# Labels
# -----------------------------------------------------------------------------

variable "labels" {
  description = "Labels to apply to the Cloud Run service"
  type        = map(string)
  default     = {}
}
