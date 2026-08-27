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
# A warm floor of 1. This is a bought cost, not a default: combined with
# cpu_idle=false it bills 2 vCPU continuously (~$140/mo against a 300 CAD/mo
# budget). min=0 was the surviving saving from the 2026-06-03 cost cut
# (2676ccfc7) and is being spent deliberately.
#
# What buys it is registre#109. A messaging turn is dispatched AFTER its webhook
# has returned 200, so Cloud Run sees zero in-flight requests and reads the
# instance as idle from the athlete's first second — an idle scaledown can
# therefore land on a live coaching turn at any time, with no deploy involved.
# On 2026-08-26 one took a group chart ask 40s from its answer. The in-flight
# turn tracker (services::turn_lifecycle) makes that survivable and visible
# rather than silent, but only a floor stops the scaledown happening at all.
#
# The cheaper alternative — leave min=0 and rely on the drain — was rejected
# because the drain can only ever convert a lost answer into an apology. A
# coaching turn the athlete waited two minutes for is worth more than the
# instance-hour that would have finished it.
#
# NOTE: the idle-floor alert (instance_floor_monitoring.tf) keys off THIS value,
# not off zero. Raising the floor without it would leave that alert firing
# permanently, which is how the real one stops being read.
backend_min_instances = 1
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
# cold-cache sciotte scrape, and admission control for that lives in the
# dedicated dravr-sciotte service (ADR-021 Phase 4), not in this pod.
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

# registre#36 close-out: Cloud Run IAM on the scraper (allow_unauthenticated
# = false + run.invoker for the app SA), so a reachable URL stops meaning an
# open one and any future route that forgets an app-level gate is still
# rejected before the container. Flipped 2026-08-21, after both the scraper
# (identity-token verifier, rev 00013) and the API (IdTokenSource caller,
# rev 00822) were serving the token auth this depends on.
backend_sciotte_iam = true

# Coach visuals: deploy the photograveur chart-press service (min=0,
# cpu_idle=true — ~$0 idle).
enable_photograveur_service = true

# Traffic flip: the API now holds the press URL, so messaging channels that
# render media can be offered charts. Nothing is offered yet — a coach only
# gets visuals once its contremaitre kind grants `visuals:`, which is the
# gate that actually turns this on for an athlete. Rollback = set false +
# apply; the app keeps rendering charts either way, since that geometry is
# resolved in-process and never touches this service.
backend_photograveur = true

# The moving tag, matching sciotte, because photograveur-bump.yml now owns when it
# moves: the image is built and :latest advanced only after main has merged the
# matching pin, and the deploy asserts the serving digest is the one it built and
# scanned. Terraform sets this at creation and then ignores it (the cloud_run module
# ignores image changes), so a fixed tag here would only describe the first revision
# ever created while claiming to govern the service.
photograveur_image = "northamerica-northeast1-docker.pkg.dev/dravr-artifacts/dravr-images/photograveur:latest"
