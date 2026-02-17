# ABOUTME: Creates Memorystore Redis instance for Pierre MCP Server caching
# ABOUTME: Connected to VPC for private access from Cloud Run services

resource "google_redis_instance" "cache" {
  name           = "${var.service_name}-redis"
  project        = var.project_id
  region         = var.region
  tier           = var.redis_tier
  memory_size_gb = var.redis_memory_size_gb
  redis_version  = var.redis_version

  authorized_network = var.vpc_id

  redis_configs = {
    maxmemory-policy = "allkeys-lru"
  }

  maintenance_policy {
    weekly_maintenance_window {
      day = "SUNDAY"
      start_time {
        hours   = 4
        minutes = 0
        seconds = 0
        nanos   = 0
      }
    }
  }

  labels = var.labels
}
