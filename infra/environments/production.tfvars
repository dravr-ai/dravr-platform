environment = "production"

# All optional features enabled
enable_database = false
enable_cache    = true
enable_frontend = true

# Backend (always-on with headroom)
backend_cpu           = "2"
backend_memory        = "1Gi"
backend_min_instances = 0
backend_max_instances = 3

# Database (HA tier, deletion protection, backups enabled)
# database_tier                = "db-custom-2-7680"
# database_deletion_protection = true
# database_backup_enabled      = true

# Cache (HA tier, 2GB for production workloads)
redis_tier           = "BASIC"
redis_memory_size_gb = 1

# Frontend (always-on)
frontend_min_instances = 0
frontend_max_instances = 3

# Labels
labels = {
  app         = "dravr"
  managed_by  = "terraform"
  environment = "production"
}
