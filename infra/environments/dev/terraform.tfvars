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

# 2 vCPU. Two SEPARATE reasons historically pinned this: (a) the contremaitre
# boot-sync ran on the bind path and saturated a single core, so 1 vCPU missed
# Cloud Run's ~55s startup probe (rev 00465) — this is now FIXED (the sync runs
# off the bind path via init_contremaitre_registries -> the poll's first
# immediate tick); AND (b) the headless-Chrome sciotte scrape (Garmin + token-less
# Strava) is CPU-hungry, and a coaching turn on 1 vCPU risks starving it into a
# tool-loop timeout. (a) is resolved but (b) is NOT, so cpu=1 stays DEFERRED:
# keep 2 vCPU until a cold-cache Chrome-scrape load-test on 1 vCPU proves the
# scrape completes in time. cpu_idle stays false (ADR-019).
backend_cpu    = "2"
backend_memory = "2Gi"
# Scale to zero when idle. A warm floor (min=1) combined with cpu_idle=false
# bills 2 vCPU continuously (~$140/mo); the dev cost is not worth keeping the
# contremaitre push webhook off a cold start — the webhook retries and prompts
# also sync on container startup.
backend_min_instances = 0
# Capped at 3 by the DB connection budget (see the concurrency block below):
# max_instances × POSTGRES_MAX_CONNECTIONS must stay ≤ 18 on db-f1-micro. With
# concurrency=8 a single pod already absorbs a full ~13-request dashboard load,
# so 3 pods is ample at dev scale (was 15, which fed the connection-slot herd).
backend_max_instances = 3

# Request concurrency is 8, NOT 1. The old "one turn per pod or it OOMs" premise
# was stale: a coaching turn does NOT hold two fresh heavyweight children. The
# `copilot --acp` Node process is a single long-lived SINGLETON shared across all
# turns (built once at startup, kept warm), and the ACP transport mutex already
# serializes LLM turns onto it — raising concurrency cannot multiply Node procs.
# Headless Chrome only spawns on a cold-cache sciotte scrape and is independently
# capped by backend_sciotte_max_concurrent below, not by request concurrency. So
# a warm-cache turn holds no fresh child, and concurrency=1 only served to turn a
# ~13-request dashboard load into a 13-instance cold-start herd that then
# exhausted the Postgres connection slots (rev 00679, 2026-07-10 incident).
#
# The real binding constraint is the DB connection budget, not memory. The
# db-f1-micro Postgres allows max_connections=25 − 3 superuser-reserved = 22
# usable; leave ~4 for migrations / the sql-client + drift-check jobs → ~18 for
# the api service. INVARIANT: max_instances × POSTGRES_MAX_CONNECTIONS ≤ 18.
# Here 3 × 6 = 18. POSTGRES_MIN_CONNECTIONS=0 (main.tf) so idle/booting pods hold
# zero slots. Raising either knob requires raising the DB tier first.
backend_max_instance_request_concurrency = 8
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
