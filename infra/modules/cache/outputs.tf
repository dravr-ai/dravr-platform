# ABOUTME: Outputs from the Memorystore Redis cache module
# ABOUTME: Provides connection details for Cloud Run environment variables

output "host" {
  description = "Hostname of the Redis instance"
  value       = google_redis_instance.cache.host
}

output "port" {
  description = "Port of the Redis instance"
  value       = google_redis_instance.cache.port
}

output "redis_url" {
  description = "Redis connection URL (redis://host:port)"
  value       = "redis://${google_redis_instance.cache.host}:${google_redis_instance.cache.port}"
}
