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
# concurrency=80 a single pod absorbs a whole user's ~13-request dashboard load
# (one user = one pod), so 3 pods is ample at dev scale — it caps concurrent
# coaching turns (ACP-mutex-serialized, 1 per pod) at 3 (was 15, which combined
# with concurrency=1 fed the connection-slot herd).
backend_max_instances = 3

# Request concurrency is 80 (Cloud Run's default), NOT 1. The old "one turn per
# pod or it OOMs" premise was stale: a coaching turn does NOT hold two fresh
# heavyweight children. The `copilot --acp` Node process is a single long-lived
# SINGLETON shared across all turns (built once, kept warm), and the ACP transport
# mutex already serializes LLM turns onto it — so concurrent chat turns per pod are
# capped by that mutex, NOT by this setting. Headless Chrome only spawns on a
# cold-cache sciotte scrape and is capped by backend_sciotte_max_concurrent below.
# concurrency=1 was pure harm: it forced one pod per request, so a single user's
# ~13-request parallel dashboard load cold-started ~13 pods (a herd) that then
# exhausted the Postgres connection slots (rev 00679, 2026-07-10 incident).
#
# Key point: request concurrency and the DB connection budget are DECOUPLED. A pod
# accepts up to 80 concurrent HTTP requests but funnels them through a small sqlx
# pool (POSTGRES_MAX_CONNECTIONS=6) — each request holds a connection only for its
# ~1ms query, then releases. So per-pod DB use is bounded by the POOL, not by
# concurrency, and one pod serves a whole user's dashboard (one user = one pod).
# INVARIANT: the budget is max_instances × POSTGRES_MAX_CONNECTIONS (NOT ×
# concurrency) ≤ 18 (22 usable − ~4 for migrations / sql-client + drift jobs).
# Here 3 × 6 = 18. POSTGRES_MIN_CONNECTIONS=0 (main.tf) so idle/booting pods hold
# zero slots. Raising max_instances or the pool requires a bigger DB tier first;
# raising concurrency is free (pool-bounded).
backend_max_instance_request_concurrency = 80
# 2 (was 1): one in-flight login legitimately holds a Chrome slot for the full
# DRAVR_SCIOTTE_LOGIN_TIMEOUT (240s, sized for number-match 2FA phone-tap), so at
# a single slot one user's 2FA wait shed a 503 on every other sciotte op on the
# pod (rev 00687, 2026-07-11). Two slots ≈ 500Mi of the 2Gi budget (~4-Chrome
# headroom, see backend_memory), halving in-pod starvation. Chrome count is
# capped here independently of the 80 HTTP-request concurrency above (see the
# block comment) — the "must match request concurrency" note in variables.tf
# predates the one-user-one-pod model and no longer holds (80 != 1).
backend_sciotte_max_concurrent = 2

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

# ADR-021: deploy the dedicated sciotte scraper service (min=0, cpu_idle=true —
# ~$0 idle). Deploy-only: backend_sciotte_remote stays default-false, so the
# API keeps its in-process path until the service is validated on dev.
enable_sciotte_service = true

# ADR-021 traffic flip: dev API routes sciotte logins/scrapes to the dedicated
# service. Rollback = set false + apply (in-process path stays compiled in
# until Phase 4).
backend_sciotte_remote = true
