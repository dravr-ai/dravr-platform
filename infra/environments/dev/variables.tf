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
  description = "CPU allocation for backend instances"
  type        = string
  default     = "1"
}

variable "backend_memory" {
  description = "Memory allocation for backend instances"
  type        = string
  default     = "512Mi"
}

variable "backend_min_instances" {
  description = "Minimum backend instances (0 for scale-to-zero)"
  type        = number
  default     = 0
}

variable "backend_max_instances" {
  description = "Maximum backend instances"
  type        = number
  default     = 10
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
