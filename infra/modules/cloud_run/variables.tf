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

variable "max_instance_request_concurrency" {
  description = "Maximum number of concurrent HTTP requests a single container instance serves. Cloud Run's default is 80, which piles long-running requests (e.g. Chrome-driven scraping) onto one pod and triggers OOM. Lower values force the autoscaler to spread load across more instances sooner — set to the per-pod concurrency ceiling for RAM-constrained workloads."
  type        = number
  default     = 80
}

variable "request_timeout" {
  description = "Maximum duration a single request may run before Cloud Run terminates it. The default 300s is too short for chat turns that run a multi-step tool loop with detail-page provider enrichment (observed >300s on dev). Requests proxied by the frontend are bounded by its nginx proxy_read_timeout as well; requests delivered straight to the service (Cloud Tasks turns) are bounded by this alone, so it must cover the longest of them."
  type        = string
  default     = "300s"
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

variable "vpc_egress" {
  description = "VPC egress setting: PRIVATE_RANGES_ONLY or ALL_TRAFFIC"
  type        = string
  default     = "PRIVATE_RANGES_ONLY"
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

variable "startup_probe_failure_threshold" {
  description = "Failed startup probes (one every 5 s) before Cloud Run kills the booting instance"
  type        = number
  default     = 10
}

# -----------------------------------------------------------------------------
# GCS Volume Mounts
# -----------------------------------------------------------------------------

variable "gcs_volumes" {
  description = "GCS buckets to mount as volumes: map of volume_name => { bucket, mount_path, read_only }"
  type = map(object({
    bucket     = string
    mount_path = string
    read_only  = optional(bool, true)
  }))
  default = {}
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

variable "invoker_members" {
  description = "IAM principals granted roles/run.invoker, e.g. [\"serviceAccount:app@project.iam.gserviceaccount.com\"]. Inert when allow_unauthenticated is true, which disables the IAM check outright."
  type        = list(string)
  default     = []

  validation {
    condition     = alltrue([for m in var.invoker_members : can(regex("^(serviceAccount|user|group|domain|principal|principalSet):", m))])
    error_message = "Each invoker member must carry an IAM principal prefix (serviceAccount:, user:, group:, domain:, principal:, principalSet:)."
  }
}

variable "custom_audiences" {
  description = "Extra audiences this service accepts in an identity token, alongside its own URL. A stable string (e.g. \"dravr-photograveur-dev\") lets terraform configure caller and callee from one literal instead of depending on the URL Cloud Run generates at creation. Keep it environment-scoped, or a token minted for dev replays against prod."
  type        = list(string)
  default     = []
}

# -----------------------------------------------------------------------------
# OpenTelemetry Collector sidecar (Cloud Trace)
# -----------------------------------------------------------------------------

variable "otel_collector" {
  description = <<-EOT
    Google-Built OpenTelemetry Collector as a sidecar that forwards the app's
    OTLP traces to Cloud Trace (via the Telemetry API, telemetry.googleapis.com).
    Off by default: with enabled = false the service is exactly what it is
    without this variable. When on, the app container gets
    OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318, the
    parentbased_traceidratio sampler at trace_sample_ratio, and
    OTEL_METRICS_EXPORTER=none, so only traces leave the instance. The service
    account also needs roles/telemetry.tracesWriter and the Telemetry API on,
    which the project and service_accounts modules grant behind the same flag.
    The sidecar's cpu/memory are billed like the app's (always-on when the
    service sets cpu_idle = false).
  EOT
  type = object({
    enabled            = optional(bool, false)
    image              = optional(string, "us-docker.pkg.dev/cloud-ops-agents-artifacts/google-cloud-opentelemetry-collector/otelcol-google:0.163.1")
    trace_sample_ratio = optional(number, 0.1)
    cpu                = optional(string, "1")
    memory             = optional(string, "512Mi")
  })
  default = {}

  validation {
    condition     = var.otel_collector.trace_sample_ratio >= 0 && var.otel_collector.trace_sample_ratio <= 1
    error_message = "otel_collector.trace_sample_ratio is a probability: 0 keeps no new trace, 1 keeps every one."
  }
}
