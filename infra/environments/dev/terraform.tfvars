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

backend_cpu    = "2"
backend_memory = "2Gi"
# Scale to zero when idle. A warm floor (min=1) combined with cpu_idle=false
# bills 2 vCPU continuously (~$140/mo); the dev cost is not worth keeping the
# contremaitre push webhook off a cold start — the webhook retries and prompts
# also sync on container startup.
backend_min_instances = 0
backend_max_instances = 15

# Concurrency capped at 1 for the OOM-prone shape of a coaching turn: each turn
# holds TWO heavyweight children for its full duration — a `copilot --acp` Node
# subprocess (COPILOT_HEADLESS_MCP_TOOL_CALLING=true) AND a headless Chrome
# (sciotte scrape on a cold cache). The 2Gi budget only sized Chrome (~250Mi ×4),
# not the Node process, so 4 concurrent turns (≈4×Chrome + 4×Node + Rust heap)
# blow past 2Gi and OOM-crash-loop. One turn per pod ≈ 1 Chrome + 1 Node ≈ 700Mi
# + Rust, comfortably under 2Gi; backend_max_instances=15 spreads load. Raise
# both back toward 4 once the activity cache + SWR removes the per-turn scrape.
backend_max_instance_request_concurrency = 1
backend_sciotte_max_concurrent           = 1

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
