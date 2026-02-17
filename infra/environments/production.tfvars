environment = "production"

<<<<<<< Updated upstream
# Database disabled until production migration from SQLite is ready
=======
# Centralized Artifact Registry project (images are pushed here by GitHub Actions)
# Replace with the real dravr-artifacts GCP project ID after creating it in the Console
artifacts_project_id = "dravr-artifacts"

# All optional features enabled
>>>>>>> Stashed changes
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
