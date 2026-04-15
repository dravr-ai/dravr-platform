# ABOUTME: Defines all configurable variables for Dravr MCP Server infrastructure
# ABOUTME: Includes project settings, database config, and GitHub integration

# -----------------------------------------------------------------------------
# Project Configuration
# -----------------------------------------------------------------------------

variable "project_id" {
  description = "GCP project ID where resources will be created"
  type        = string

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{4,28}[a-z0-9]$", var.project_id))
    error_message = "Project ID must be 6-30 lowercase letters, digits, or hyphens."
  }
}

variable "region" {
  description = "GCP region for resource deployment"
  type        = string
  default     = "northamerica-northeast1"
}

variable "environment" {
  description = "Environment name (e.g., production, development)"
  type        = string
  default     = "development"

  validation {
    condition     = contains(["development", "production"], var.environment)
    error_message = "Environment must be development or production."
  }
}

# -----------------------------------------------------------------------------
# Service Configuration
# -----------------------------------------------------------------------------

variable "service_name" {
  description = "Name of the Cloud Run service"
  type        = string
  default     = "dravr-mcp-server"
}

# -----------------------------------------------------------------------------
# Database Configuration
# -----------------------------------------------------------------------------

variable "enable_database" {
  description = "Enable Cloud SQL PostgreSQL database (provisions instance and private networking)"
  type        = bool
  default     = false
}

variable "database_tier" {
  description = "Cloud SQL machine tier (e.g., db-f1-micro, db-custom-1-3840)"
  type        = string
  default     = "db-f1-micro"
}

variable "database_version" {
  description = "PostgreSQL version for Cloud SQL"
  type        = string
  default     = "POSTGRES_15"
}

variable "database_name" {
  description = "Name of the PostgreSQL database"
  type        = string
  default     = "dravr"
}

variable "database_user" {
  description = "Name of the PostgreSQL user"
  type        = string
  default     = "dravr"
}

variable "database_deletion_protection" {
  description = "Enable deletion protection for the database instance"
  type        = bool
  default     = true
}

variable "database_backup_enabled" {
  description = "Enable automated backups for the database"
  type        = bool
  default     = true
}

variable "database_backup_start_time" {
  description = "Start time for database backups (HH:MM format, UTC)"
  type        = string
  default     = "03:00"
}

variable "database_enable_public_ip" {
  description = "Enable public IP on Cloud SQL for local debugging via Auth Proxy (dev only)"
  type        = bool
  default     = false
}

variable "database_authorized_networks" {
  description = "Authorized networks for Cloud SQL public IP access (CIDR ranges)"
  type = list(object({
    name  = string
    value = string
  }))
  default = []
}

# -----------------------------------------------------------------------------
# Cache Configuration
# -----------------------------------------------------------------------------

variable "enable_cache" {
  description = "Enable Memorystore Redis cache (minimum 1GB BASIC tier ~$35/month)"
  type        = bool
  default     = false
}

variable "redis_tier" {
  description = "Redis tier: BASIC (no replication) or STANDARD_HA (with replica)"
  type        = string
  default     = "BASIC"
}

variable "redis_memory_size_gb" {
  description = "Redis memory size in GB (minimum 1)"
  type        = number
  default     = 1
}

variable "redis_version" {
  description = "Redis version for Memorystore (e.g., REDIS_7_0, REDIS_7_2)"
  type        = string
  default     = "REDIS_7_0"

  validation {
    condition     = can(regex("^REDIS_[0-9]+_[0-9]+$", var.redis_version))
    error_message = "Redis version must match REDIS_X_Y (e.g., REDIS_7_0)."
  }
}

# -----------------------------------------------------------------------------
# Networking Configuration
# -----------------------------------------------------------------------------

variable "vpc_name" {
  description = "Name of the VPC network"
  type        = string
  default     = "dravr-vpc"
}

variable "subnet_cidr" {
  description = "CIDR range for the VPC subnet"
  type        = string
  default     = "10.0.0.0/24"
}

variable "vpc_connector_cidr" {
  description = "CIDR range for the serverless VPC connector"
  type        = string
  default     = "10.8.0.0/28"
}

# -----------------------------------------------------------------------------
# GitHub Integration
# -----------------------------------------------------------------------------

variable "github_org" {
  description = "GitHub organization or username"
  type        = string
  default     = "dravr-ai"
}

variable "github_repo" {
  description = "GitHub repository name"
  type        = string
  default     = "dravr-platform"
}

# -----------------------------------------------------------------------------
# Artifact Registry (centralized in dravr-artifacts project)
# -----------------------------------------------------------------------------

variable "artifacts_project_id" {
  description = "GCP project ID of the centralized dravr-artifacts project (used for cross-project image pull IAM)"
  type        = string

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{4,28}[a-z0-9]$", var.artifacts_project_id))
    error_message = "Artifacts project ID must be 6-30 lowercase letters, digits, or hyphens."
  }
}

# -----------------------------------------------------------------------------
# Backend Cloud Run Configuration
# -----------------------------------------------------------------------------

variable "backend_image" {
  description = "Container image for the backend API (e.g., region-docker.pkg.dev/project/repo/server:latest)"
  type        = string
}

variable "backend_base_url" {
  description = "Deprecated — use frontend_base_url instead (backend is behind nginx proxy)"
  type        = string
  default     = ""
}

variable "frontend_base_url" {
  description = "Public URL of the frontend (nginx proxies API traffic to backend; used for OAuth callbacks)"
  type        = string
  default     = ""
}

variable "backend_cpu" {
  description = "CPU allocation for backend instances. 2 vCPU gives Chrome-driven sciotte scraping the headroom it needs alongside the Pierre server itself."
  type        = string
  default     = "2"
}

variable "backend_memory" {
  description = "Memory allocation for backend instances. 2Gi fits ~4 concurrent Chrome processes (each ~250Mi) plus Pierre's working set with headroom."
  type        = string
  default     = "2Gi"
}

variable "backend_min_instances" {
  description = "Minimum backend instances (0 for scale-to-zero)"
  type        = number
  default     = 0
}

variable "backend_max_instances" {
  description = "Maximum backend instances. 15 × 4-concurrency = 60 concurrent Chrome slots at fleet peak — enough to absorb a 50-user onboarding burst with room to spare."
  type        = number
  default     = 15
}

variable "backend_max_instance_request_concurrency" {
  description = "Maximum concurrent requests per backend container. Set to 4 to match PIERRE_SCIOTTE_MAX_CONCURRENT so Cloud Run spreads sciotte traffic across instances instead of piling onto one pod (default 80 causes OOM under burst load)."
  type        = number
  default     = 4
}

variable "backend_sciotte_max_concurrent" {
  description = "Hard cap on concurrent Chrome-driven sciotte scrapes per instance. Used as PIERRE_SCIOTTE_MAX_CONCURRENT env var. Must match backend_max_instance_request_concurrency."
  type        = number
  default     = 4
}

variable "backend_sciotte_max_queue" {
  description = "Combined cap on running + waiting sciotte requests per instance before fast-rejecting with 503. Used as PIERRE_SCIOTTE_MAX_QUEUE env var."
  type        = number
  default     = 20
}

variable "backend_sciotte_acquire_timeout_secs" {
  description = "Maximum seconds a sciotte request waits for a permit before being rejected. Used as PIERRE_SCIOTTE_ACQUIRE_TIMEOUT_SECS env var."
  type        = number
  default     = 10
}

variable "backend_sciotte_permit_max_lifetime_secs" {
  description = "Maximum seconds a sciotte permit can stay parked across a multi-step OTP/2FA flow before the watchdog evicts it. Used as PIERRE_SCIOTTE_PERMIT_MAX_LIFETIME_SECS env var."
  type        = number
  default     = 300
}

variable "backend_sciotte_watchdog_interval_secs" {
  description = "Interval at which the sciotte watchdog scans for stale parked permits. Used as PIERRE_SCIOTTE_WATCHDOG_INTERVAL_SECS env var."
  type        = number
  default     = 15
}

variable "backend_sciotte_retry_after_hint_secs" {
  description = "`Retry-After` header value emitted on sciotte queue-full / timeout rejections. Used as PIERRE_SCIOTTE_RETRY_AFTER_HINT_SECS env var."
  type        = number
  default     = 5
}

variable "backend_sciotte_closed_retry_after_secs" {
  description = "`Retry-After` header value emitted when the sciotte limiter is closed (shutdown). Used as PIERRE_SCIOTTE_CLOSED_RETRY_AFTER_SECS env var."
  type        = number
  default     = 60
}

# -----------------------------------------------------------------------------
# Frontend Cloud Run Configuration
# -----------------------------------------------------------------------------

variable "enable_frontend" {
  description = "Enable the admin frontend Cloud Run service"
  type        = bool
  default     = false
}

variable "frontend_image" {
  description = "Container image for the admin frontend (required when enable_frontend is true)"
  type        = string
  default     = null
}

variable "frontend_min_instances" {
  description = "Minimum frontend instances (0 for scale-to-zero)"
  type        = number
  default     = 0
}

variable "frontend_max_instances" {
  description = "Maximum frontend instances"
  type        = number
  default     = 5
}

# -----------------------------------------------------------------------------
# Labels
# -----------------------------------------------------------------------------

variable "labels" {
  description = "Common labels to apply to all resources"
  type        = map(string)
  default = {
    app        = "dravr"
    managed_by = "terraform"
  }
}

# -----------------------------------------------------------------------------
# Firebase Identity Platform
# -----------------------------------------------------------------------------

variable "firebase_project_id" {
  description = "Firebase project ID (linked GCP project with Firebase enabled)"
  type        = string
  default     = "dravr-dev-8d4a3"
}

# -----------------------------------------------------------------------------
# Slack Ops Notifications
# -----------------------------------------------------------------------------

variable "slack_ops_enabled" {
  description = "Enable Slack ops notifications (set to false to use noop notifier)"
  type        = bool
  default     = true
}

variable "slack_ops_deploys_channel" {
  description = "Slack channel for deploy/restart notifications (channel ID or name, e.g. #dravr-dev-deploys)"
  type        = string
  default     = "#dravr-dev-deploys"
}

variable "slack_ops_users_channel" {
  description = "Slack channel for user lifecycle notifications (channel ID or name, e.g. #dravr-dev-users)"
  type        = string
  default     = "#dravr-dev-users"
}

variable "slack_error_channel" {
  description = "Slack channel for automatic ERROR-level log alerts (channel ID or name, empty to disable)"
  type        = string
  default     = "#dev-dravr-errors"
}

variable "notify_batch_window_secs" {
  description = "Seconds to batch error notifications before sending a digest (default: 5)"
  type        = number
  default     = 5
}

variable "notify_max_messages_per_min" {
  description = "Maximum error notification messages per minute across all channels (default: 10)"
  type        = number
  default     = 10
}

variable "notify_dedup_window_secs" {
  description = "Seconds to suppress duplicate error notifications for the same error (default: 30)"
  type        = number
  default     = 30
}

variable "notify_email_from" {
  description = "Sender address for error alert emails via Resend (e.g. 'Pierre Alerts <alerts@dravr.ai>')"
  type        = string
  default     = "Pierre Alerts <alerts@dravr.ai>"
}

variable "notify_email_to" {
  description = "Comma-separated recipient email addresses for error alerts"
  type        = string
  default     = "jf@dravr.ai,phil@dravr.ai"
}
