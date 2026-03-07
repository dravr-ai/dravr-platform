# Required
project_id = "dravr-prod" # TODO: replace with actual GCP project ID
# Initial image reference — CI/CD deploys by digest and Terraform lifecycle.ignore_changes prevents drift
backend_image        = "northamerica-northeast1-docker.pkg.dev/dravr-artifacts/dravr-images/server:placeholder-managed-by-cicd"
frontend_image       = "northamerica-northeast1-docker.pkg.dev/dravr-artifacts/dravr-images/frontend:placeholder-managed-by-cicd"
artifacts_project_id = "dravr-artifacts"

environment  = "production"
region       = "northamerica-northeast1"
service_name = "dravr-mcp-server"

# Database disabled until production migration from SQLite is ready
enable_database = false
enable_cache    = true
enable_frontend = true

backend_cpu           = "2"
backend_memory        = "1Gi"
backend_min_instances = 0
backend_max_instances = 3

# Database (uncomment when enable_database = true)
# database_tier                = "db-custom-2-7680"
# database_deletion_protection = true
# database_backup_enabled      = true

redis_tier           = "BASIC"
redis_memory_size_gb = 1

frontend_min_instances = 0
frontend_max_instances = 3

labels = {
  app         = "dravr"
  managed_by  = "terraform"
  environment = "production"
}
