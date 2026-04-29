# Required
project_id = "dravr-dev" # TODO: replace with actual GCP project ID
# Baseline image for fresh resource creation — CI/CD deploys by digest and lifecycle.ignore_changes prevents drift
backend_image        = "northamerica-northeast1-docker.pkg.dev/dravr-artifacts/dravr-images/server:latest"
frontend_image       = "northamerica-northeast1-docker.pkg.dev/dravr-artifacts/dravr-images/frontend:latest"
artifacts_project_id = "dravr-artifacts"

environment  = "development"
region       = "northamerica-northeast1"
service_name = "dravr-mcp-server"

enable_database = true
enable_cache    = true
enable_frontend = true

# Frontend public URL (nginx proxies API traffic to backend; used for OAuth callbacks)
frontend_base_url = "https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app"

backend_cpu           = "2"
backend_memory        = "2Gi"
backend_min_instances = 0
backend_max_instances = 15

# database_tier                = "db-f1-micro"
# database_deletion_protection = false
# database_backup_enabled      = false

# Firebase Identity Platform (Google Sign-In)
# OAuth credentials stored in GCP Secret Manager (google-oauth-client-id, google-oauth-client-secret)
firebase_project_id = "dravr-dev-8d4a3"

labels = {
  app         = "dravr"
  managed_by  = "terraform"
  environment = "development"
}
