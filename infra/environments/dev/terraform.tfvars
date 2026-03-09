# Required
project_id = "dravr-dev" # TODO: replace with actual GCP project ID
# Initial image reference — CI/CD deploys by digest and Terraform lifecycle.ignore_changes prevents drift
backend_image        = "northamerica-northeast1-docker.pkg.dev/dravr-artifacts/dravr-images/server:56721491"
frontend_image       = "northamerica-northeast1-docker.pkg.dev/dravr-artifacts/dravr-images/frontend:56721491"
artifacts_project_id = "dravr-artifacts"

environment  = "development"
region       = "northamerica-northeast1"
service_name = "dravr-mcp-server"

enable_database = false
enable_cache    = false
enable_frontend = true

backend_cpu           = "1"
backend_memory        = "1Gi"
backend_min_instances = 0
backend_max_instances = 1

# database_tier                = "db-f1-micro"
# database_deletion_protection = false
# database_backup_enabled      = false

labels = {
  app         = "dravr"
  managed_by  = "terraform"
  environment = "development"
}
