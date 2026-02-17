environment = "development"

# Optional features disabled in development
enable_database = false
enable_cache    = false
enable_frontend = true

# Backend (always deployed, minimal resources)
backend_cpu           = "1"
backend_memory        = "512Mi"
backend_min_instances = 0
backend_max_instances = 1

# Database (unused when enable_database=false, but set reasonable defaults)
database_tier                = "db-f1-micro"
database_deletion_protection = false
database_backup_enabled      = false

# Labels
labels = {
  app         = "dravr"
  managed_by  = "terraform"
  environment = "development"
}
