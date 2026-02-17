# ABOUTME: Variables for the Memorystore Redis cache module
# ABOUTME: Configures Redis instance tier, size, and network settings

variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "region" {
  description = "GCP region"
  type        = string
}

variable "service_name" {
  description = "Name of the service (used as prefix for instance name)"
  type        = string
}

variable "vpc_id" {
  description = "ID of the VPC network for authorized network access"
  type        = string
}

variable "redis_tier" {
  description = "Redis tier: BASIC (no replication) or STANDARD_HA (with replica)"
  type        = string
  default     = "BASIC"

  validation {
    condition     = contains(["BASIC", "STANDARD_HA"], var.redis_tier)
    error_message = "Redis tier must be BASIC or STANDARD_HA."
  }
}

variable "redis_memory_size_gb" {
  description = "Redis memory size in GB (minimum 1)"
  type        = number
  default     = 1

  validation {
    condition     = var.redis_memory_size_gb >= 1
    error_message = "Memorystore Redis minimum memory size is 1 GB."
  }
}

variable "redis_version" {
  description = "Redis version (e.g., REDIS_7_0, REDIS_7_2)"
  type        = string
  default     = "REDIS_7_0"

  validation {
    condition     = can(regex("^REDIS_[0-9]+_[0-9]+$", var.redis_version))
    error_message = "Redis version must match pattern REDIS_X_Y (e.g., REDIS_7_0)."
  }
}

variable "labels" {
  description = "Labels to apply to resources"
  type        = map(string)
  default     = {}
}
