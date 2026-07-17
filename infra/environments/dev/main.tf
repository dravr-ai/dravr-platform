# ABOUTME: Orchestrates all Terraform modules for Dravr MCP Server infrastructure
# ABOUTME: Manages dependencies between modules with explicit depends_on

# -----------------------------------------------------------------------------
# Project APIs (must be first)
# -----------------------------------------------------------------------------

module "project" {
  source = "../../modules/project"

  project_id = var.project_id
}

# -----------------------------------------------------------------------------
# Networking (depends on APIs)
# -----------------------------------------------------------------------------

module "networking" {
  source = "../../modules/networking"

  project_id         = var.project_id
  region             = var.region
  vpc_name           = var.vpc_name
  subnet_cidr        = var.subnet_cidr
  vpc_connector_cidr = var.vpc_connector_cidr
  enable_database    = var.enable_database

  depends_on = [module.project]
}

# -----------------------------------------------------------------------------
# Secrets (depends on APIs)
# -----------------------------------------------------------------------------

module "secrets" {
  source = "../../modules/secrets"

  project_id   = var.project_id
  service_name = var.service_name
  labels       = var.labels

  depends_on = [module.project]
}

# -----------------------------------------------------------------------------
# Database (depends on networking and secrets)
# -----------------------------------------------------------------------------

module "database" {
  count  = var.enable_database ? 1 : 0
  source = "../../modules/database"

  project_id                = var.project_id
  region                    = var.region
  service_name              = var.service_name
  environment               = var.environment
  vpc_self_link             = module.networking.vpc_self_link
  private_vpc_connection_id = module.networking.private_vpc_connection_id
  database_version          = var.database_version
  database_tier             = var.database_tier
  database_name             = var.database_name
  database_user             = var.database_user
  database_password         = module.secrets.db_password
  deletion_protection       = var.database_deletion_protection
  backup_enabled            = var.database_backup_enabled
  backup_start_time         = var.database_backup_start_time
  enable_public_ip          = var.database_enable_public_ip
  authorized_networks       = var.database_authorized_networks
  labels                    = var.labels

  depends_on = [module.networking, module.secrets]
}

# -----------------------------------------------------------------------------
# Cache (optional, depends on networking)
# -----------------------------------------------------------------------------

module "cache" {
  count  = var.enable_cache ? 1 : 0
  source = "../../modules/cache"

  project_id           = var.project_id
  region               = var.region
  service_name         = var.service_name
  vpc_id               = module.networking.vpc_id
  redis_tier           = var.redis_tier
  redis_memory_size_gb = var.redis_memory_size_gb
  redis_version        = var.redis_version
  labels               = var.labels

  depends_on = [module.project, module.networking]
}

# -----------------------------------------------------------------------------
# BigQuery usage analytics (optional; depends on database + secrets)
# Pricing analysis reads BigQuery, never the live api service (no observer
# effect — see ADR-019 and "BigQuery + Looker pricing analytics — Plan").
# -----------------------------------------------------------------------------

module "bigquery_usage" {
  count  = var.enable_bigquery_usage ? 1 : 0
  source = "../../modules/bigquery_usage"

  project_id               = var.project_id
  location                 = var.region
  dataset_id               = var.bigquery_usage_dataset_id
  cloudsql_connection_name = module.database[0].connection_name
  cloudsql_database        = var.database_name
  cloudsql_user            = var.database_user
  cloudsql_password        = module.secrets.db_password
  labels                   = var.labels

  depends_on = [module.database, module.secrets]
}

# -----------------------------------------------------------------------------
# Service Accounts (depends on APIs)
# -----------------------------------------------------------------------------

module "service_accounts" {
  source = "../../modules/service_accounts"

  project_id           = var.project_id
  service_name         = var.service_name
  artifacts_project_id = var.artifacts_project_id
  tf_state_bucket      = "dravr-dev-terraform-f58b1"

  depends_on = [module.project]
}

# -----------------------------------------------------------------------------
# Workload Identity (depends on service accounts)
# -----------------------------------------------------------------------------

module "workload_identity" {
  source = "../../modules/workload_identity"

  project_id                            = var.project_id
  github_org                            = var.github_org
  github_repo                           = var.github_repo
  deployer_service_account_name         = module.service_accounts.deployer_service_account_name
  terraform_runner_service_account_name = module.service_accounts.terraform_runner_service_account_name
  # No sister repos need GCP access — the dravr-contremaitre prompt mirror was
  # removed (prompts now hot-reload directly from GitHub via the push webhook).
  additional_repositories = []

  depends_on = [module.service_accounts]
}

# -----------------------------------------------------------------------------
# Cloud KMS Key Encryption Key (KEK) for envelope encryption of the database DEK
# The KEK never leaves KMS; the runtime wraps/unwraps the DEK via KMS (ADR-017).
# -----------------------------------------------------------------------------

module "kms" {
  source = "../../modules/kms"

  project_id                    = var.project_id
  service_name                  = var.service_name
  location                      = var.region
  runtime_service_account_email = module.service_accounts.app_service_account_email
  labels                        = var.labels

  # module.project gates on the cloudkms API being enabled + propagated (time_sleep).
  depends_on = [module.project, module.service_accounts]
}

# -----------------------------------------------------------------------------
# Sciotte Scripts Bucket (hot-swappable JS for headless Chrome scraping)
# -----------------------------------------------------------------------------

resource "google_storage_bucket" "sciotte_scripts" {
  name          = "${var.project_id}-sciotte-scripts"
  project       = var.project_id
  location      = var.region
  force_destroy = true

  uniform_bucket_level_access = true

  labels = merge(var.labels, { component = "sciotte" })
}

# Grant the app service account read access to the scripts bucket
resource "google_storage_bucket_iam_member" "sciotte_scripts_reader" {
  bucket = google_storage_bucket.sciotte_scripts.name
  role   = "roles/storage.objectViewer"
  member = "serviceAccount:${module.service_accounts.app_service_account_email}"

  depends_on = [module.service_accounts]
}

# -----------------------------------------------------------------------------
# Backend API (always deployed)
# -----------------------------------------------------------------------------

module "backend" {
  source = "../../modules/cloud_run"

  project_id            = var.project_id
  region                = var.region
  service_name          = "${var.service_name}-api"
  container_image       = var.backend_image
  service_account_email = module.service_accounts.app_service_account_email

  container_port = 8081
  cpu            = var.backend_cpu
  memory         = var.backend_memory
  # Chat turns run a multi-step tool loop with detail-page provider enrichment
  # that exceeds the 300s default; kept in sync with the frontend nginx
  # proxy_read_timeout (docker/images/frontend/nginx.conf).
  request_timeout = "600s"
  # Keep CPU always-allocated at 2 vCPU: this service can't be trimmed cheaply.
  # cpu_idle=true throttles CPU between requests and kills long-lived subprocesses
  # — the Copilot ACP LLM runner (chat/insights/coach UI hung, rev 00464) and the
  # Discord Gateway. cpu=1 can't keep /health alive through the contremaitre boot
  # sync, so startup probes fail (rev 00465). Real cost fix = split the always-on
  # LLM/Discord subprocess into its own small service, then let api go request-based.
  # See ADR-019 (instance-based billing rationale + planned split).
  cpu_idle                         = false
  startup_cpu_boost                = true
  min_instances                    = var.backend_min_instances
  max_instances                    = var.backend_max_instances
  max_instance_request_concurrency = var.backend_max_instance_request_concurrency

  # Mount sciotte scripts bucket for hot-swappable JS extraction scripts
  gcs_volumes = {
    sciotte-scripts = {
      bucket     = google_storage_bucket.sciotte_scripts.name
      mount_path = "/sciotte-scripts"
      read_only  = true
    }
  }

  ingress                  = "INGRESS_TRAFFIC_INTERNAL_ONLY"
  allow_unauthenticated    = true
  vpc_connector_id         = module.networking.vpc_connector_id
  cloudsql_connection_name = var.enable_database ? module.database[0].connection_name : null

  env_vars = merge(
    {
      RUST_LOG    = "info"
      HOST        = "0.0.0.0"
      MCP_PORT    = "8080"
      HTTP_PORT   = "8081"
      ENVIRONMENT = var.environment

      # Frontend proxies all traffic to backend (same origin), so CORS is only
      # needed for mobile clients which don't enforce it. Wildcard is safe here.
      CORS_ALLOWED_ORIGINS = "*"

      # Public URL for OAuth callbacks (frontend URL, since nginx proxies to backend)
      FRONTEND_URL = var.frontend_base_url
      BASE_URL     = var.frontend_base_url

      # Firebase project for Google Sign-In token validation
      FIREBASE_PROJECT_ID = var.firebase_project_id

      # Email sender configuration
      RESEND_FROM_EMAIL = "no-reply@dravr.ai"

      # Auto-approve: disabled globally, but dravr.ai emails bypass approval
      AUTO_APPROVE_USERS   = "false"
      AUTO_APPROVE_DOMAINS = "dravr.ai"

      # LLM provider configuration (copilot_headless via embacle + GitHub Copilot CLI).
      # Primary = claude-opus-4.8 (high-reasoning Opus variant, not the -fast SKU).
      # Fallback model = claude-sonnet-5 in case Opus is unavailable or rate-limited
      # (intra-provider model fallback, same Copilot session).
      # Runtime fallback chain = cross-provider failover to Cohere (Command A,
      # paid, 10k rpm chat via COHERE_API_KEY) when Copilot itself returns a
      # retryable error (auth, 5xx, transient, throttle). Built by
      # ChatProvider::Chain in crates/pierre-llm/src/provider.rs and gated on
      # PIERRE_LLM_RUNTIME_FALLBACK=true. Without this, a Copilot session token
      # refresh failure (e.g. transient GitHub rate-limit during the
      # api.github.com/copilot_internal token exchange) would surface as a
      # user-facing "Dravr temporairement indisponible". Switched off claude_code
      # on 2026-05-13: the direct Anthropic subscription burned through Opus
      # credits during every Copilot blip with no spend cap. Cohere's paid
      # production rate limit keeps the first fallback tier answering under load.
      # Provider health transitions still fire `llm.provider_unhealthy`
      # notify events for operator awareness, and the chat route returns 503
      # with retry-after when LlmHealthState reports Unhealthy.
      # Entitlement verified 2026-04-16 against the dev PAT (Copilot side).
      #
      # Tertiary = Gemini (Google free tier via GEMINI_API_KEY). Last-resort
      # tier (Copilot -> Cohere -> Gemini): the free tier 429s under sustained
      # load, so it sits behind the paid Cohere fallback and only catches
      # requests when both Copilot and Cohere return a retryable error.
      # PIERRE_LLM_TERTIARY_PROVIDER turns the secondary into a nested
      # Chain{Cohere, Gemini}, so retry classification cascades the same way
      # at each tier.
      PIERRE_LLM_PROVIDER = "copilot_headless"
      # Coaching model. Sonnet, not Opus: the coaching bench found raters could
      # not distinguish Opus output and it tied last on quality, while Opus is
      # the slowest model — slow enough that an Autopilot tool turn overruns the
      # ACP prompt timeout below. Sonnet is faster (keeps turns under budget) at
      # equal coaching quality. PIERRE_LLM_MODEL is the unified primary-model
      # override for all providers; PIERRE_LLM_DEFAULT_MODEL must match it so the
      # chain stamps the right primary model name. (COPILOT_HEADLESS_MODEL stays
      # on Opus — it only drives the sciotte vision-login fallback, not chat.)
      PIERRE_LLM_MODEL                   = "claude-sonnet-5"
      PIERRE_LLM_DEFAULT_MODEL           = "claude-sonnet-5"
      PIERRE_LLM_FALLBACK_MODEL          = "claude-sonnet-5"
      PIERRE_LLM_RUNTIME_FALLBACK        = "true"
      PIERRE_LLM_FALLBACK_PROVIDER       = "cohere"
      PIERRE_LLM_FALLBACK_PROVIDER_MODEL = "command-a-03-2025"
      PIERRE_LLM_TERTIARY_PROVIDER       = "gemini"
      PIERRE_LLM_TERTIARY_PROVIDER_MODEL = "gemini-flash-lite-latest"

      # Per-turn tool intent pre-filter (dravr-aiguilleur): narrows the
      # chat-callable tool set to what each message + coach needs before sending
      # it to the LLM. Default-off in code; enabled here so dev runs it.
      PIERRE_TOOL_INTENT_PREFILTER_ENABLED = "1"

      # Route Copilot-headless tool turns through native MCP tool calling: the
      # server hands Copilot an HTTP MCP server pointing at its own /mcp endpoint
      # (per-turn, tenant-scoped token), so the model calls Dravr tools natively
      # over ACP instead of fragile text-based <tool_call> simulation. This both
      # makes the headless provider advertise SDK_TOOL_CALLING and enables the
      # per-turn MCP-bridge token minting.
      #
      # TEMPORARILY DISABLED (2026-06-22): the native MCP-bridge path has two
      # open defects that break the messaging coach — (1) the originating
      # conversation id is not carried through the bridged tool calls, so a
      # chat-triggered historical-activity backfill is spawned untagged and the
      # completion push never fires; (2) the model echoes the raw tool-result
      # scaffolding instead of synthesizing, which the strip nukes to empty ->
      # "Hmm, je n'ai pas réussi". Falling back to the text <tool_call> loop
      # routes tool calls through the conversation-id-bound UniversalExecutor
      # (push works) and the working synthesis strip. Re-enable once both native
      # -path defects are fixed.
      COPILOT_HEADLESS_MCP_TOOL_CALLING = "false"

      # Copilot runs the MCP tool loop in Autopilot mode (call -> results ->
      # synthesize in one turn), which makes individual ACP messages run longer
      # than the 90s embacle default. Raise the per-message read timeout so the
      # turn isn't cut off mid-synthesis.
      EMBACLE_ACP_MESSAGE_TIMEOUT_SECS = "300"

      # Whole-turn ACP timeout. Must encompass a full Autopilot turn, which runs
      # the entire tool loop AND synthesis inside ONE ACP prompt — so this caps
      # total turn duration, not a single request/response. The old 150s cap was
      # justified by a "heaviest real coaching turn ~48s" figure measured in the
      # pre-Autopilot text-sim era (many short prompts); once Autopilot collapsed
      # the turn into one long prompt, healthy multi-tool turns hit 150s+ and got
      # guillotined mid-synthesis (then fell to a broken Cohere fallback -> generic
      # error). Set to 300 to match EMBACLE_ACP_MESSAGE_TIMEOUT_SECS above: the
      # message (idle) timeout already fails fast on a stalled "emitting nothing"
      # session, so this only needs to be wide enough for a legitimately long turn.
      EMBACLE_ACP_PROMPT_TIMEOUT_SECS = "300"

      # Disable backups in Cloud Run (ephemeral filesystem)
      BACKUP_ENABLED = "false"

      # Sciotte JS scripts override directory (GCS-mounted bucket)
      DRAVR_SCIOTTE_SCRIPTS_DIR = "/sciotte-scripts"

      # WhatsApp non-secret config (phone number ID is not sensitive)
      META_WHATSAPP_PHONE_NUMBER_ID = "997162370153116"
      META_WHATSAPP_VERIFY_TOKEN    = "5aec2c301a90cf03a31e5f5e638f9e38"

      # Messenger non-secret config (verify token is the App Secret, not sensitive here)
      META_MESSENGER_VERIFY_TOKEN = "5aec2c301a90cf03a31e5f5e638f9e38"

      # Admin email for messaging channel seeding (resolves tenant on startup)
      ADMIN_EMAIL = "admin@dravr.ai"

      # Slack ops notification channels (deploy events and user lifecycle events)
      SLACK_OPS_ENABLED         = tostring(var.slack_ops_enabled)
      SLACK_OPS_DEPLOYS_CHANNEL = var.slack_ops_deploys_channel
      SLACK_OPS_USERS_CHANNEL   = var.slack_ops_users_channel

      # Error notification layer (dravr-tronc ErrorNotificationLayer)
      SLACK_ERROR_CHANNEL         = var.slack_error_channel
      NOTIFY_BATCH_WINDOW_SECS    = tostring(var.notify_batch_window_secs)
      NOTIFY_MAX_MESSAGES_PER_MIN = tostring(var.notify_max_messages_per_min)
      NOTIFY_DEDUP_WINDOW_SECS    = tostring(var.notify_dedup_window_secs)
      NOTIFY_EMAIL_FROM           = var.notify_email_from
      NOTIFY_EMAIL_TO             = var.notify_email_to

      # Contremaitre prompt hot-reload — reads directly from the GitHub repo
      # (the source of truth) on the push webhook. The webhook does a
      # SELECTIVE sync of only the changed files (a handful per push), so it
      # does not threaten the 5000/hr GitHub API budget the way a full poll
      # would — which is why the GCS mirror (a billing-blocked single point of
      # failure) is no longer needed. Writes (admin coach-promotion) commit to
      # the repo via CONTREMAITRE_GITHUB_PAT.
      CONTREMAITRE_REPO   = "dravr-ai/dravr-contremaitre"
      CONTREMAITRE_BRANCH = "main"

      # Sciotte backpressure limiter — all seven knobs are required at
      # startup; the crate ships no numeric defaults. See
      # pierre-providers/src/sciotte_limiter.rs for the semantics of each.
      PIERRE_SCIOTTE_MAX_CONCURRENT           = tostring(var.backend_sciotte_max_concurrent)
      PIERRE_SCIOTTE_MAX_QUEUE                = tostring(var.backend_sciotte_max_queue)
      PIERRE_SCIOTTE_ACQUIRE_TIMEOUT_SECS     = tostring(var.backend_sciotte_acquire_timeout_secs)
      PIERRE_SCIOTTE_PERMIT_MAX_LIFETIME_SECS = tostring(var.backend_sciotte_permit_max_lifetime_secs)
      PIERRE_SCIOTTE_WATCHDOG_INTERVAL_SECS   = tostring(var.backend_sciotte_watchdog_interval_secs)
      PIERRE_SCIOTTE_RETRY_AFTER_HINT_SECS    = tostring(var.backend_sciotte_retry_after_hint_secs)
      PIERRE_SCIOTTE_CLOSED_RETRY_AFTER_SECS  = tostring(var.backend_sciotte_closed_retry_after_secs)

      # Sciotte scraper login-step timeouts (consumed by the dravr-sciotte
      # crate, not the limiter). The crate's compiled defaults (login 120s,
      # password-step 30s, phone-tap 60s) are too short for Strava's
      # number-match 2FA, where the user must read a number off the login
      # screen and tap it on their phone — a 3-4 minute interactive step.
      DRAVR_SCIOTTE_LOGIN_TIMEOUT         = tostring(var.backend_sciotte_login_timeout_secs)
      DRAVR_SCIOTTE_PASSWORD_STEP_TIMEOUT = tostring(var.backend_sciotte_password_step_timeout_secs)
      DRAVR_SCIOTTE_PHONE_TAP_TIMEOUT     = tostring(var.backend_sciotte_phone_tap_timeout_secs)

      # Sciotte vision login: Hybrid runs the fast CSS/JS selector path first
      # and only falls back to LLM screenshot reasoning when selectors fail
      # (e.g. a Strava login DOM change). COPILOT_HEADLESS_MODEL is the model
      # for that vision fallback only — PIERRE_LLM_MODEL shadows it for the chat
      # provider. Kept on claude-opus-4.8 for the heavier vision-reasoning task;
      # chat/coaching runs the cheaper, faster claude-sonnet-5 via PIERRE_LLM_MODEL.
      DRAVR_SCIOTTE_LOGIN_MODE = "hybrid"
      COPILOT_HEADLESS_MODEL   = "claude-opus-4.8"

      # ADR-021 remote toggle: when backend_sciotte_remote is on, sciotte
      # logins/scrapes route to the dedicated scraper service instead of
      # in-pod Chrome. Empty string = toggle off (the client treats it as
      # unset); the in-process path above stays live until Phase 4 deletes it.
      DRAVR_SCIOTTE_REMOTE_URL = var.backend_sciotte_remote ? module.sciotte[0].service_url : ""

      # Detail-page enrichment. The all-activities N+1 (navigate to each detail
      # page) ran ~4.5 min and timed out on a real coaching turn, handing the
      # coach 0 activities — so it's OFF by default. The list page already carries
      # type, date, distance, and elevation (dénivelé), and ambient temperature is
      # filled by the weather backfill, so coaching has what it needs without it.
      # Enrichment only adds precise UTC start-time + HR/power/cadence. With
      # dravr-sciotte v0.7.1, flipping ENRICH_DETAILS to "true" now enriches only
      # the most recent ENRICH_LIMIT activities (bounded), so it no longer runs
      # minutes — opt in once the list-only path is confirmed.
      PIERRE_SCIOTTE_ENRICH_DETAILS = "false"
      PIERRE_SCIOTTE_ENRICH_LIMIT   = "5"
    },
    # Cloud SQL components — entrypoint.sh assembles these into DATABASE_URL
    var.enable_database ? {
      DATABASE_HOST = "/cloudsql/${module.database[0].connection_name}"
      DATABASE_NAME = module.database[0].database_name
      DATABASE_USER = module.database[0].database_user

      # Per-pod sqlx pool bounds. The code default is max=10/min=2 (see
      # PostgresPoolConfig in crates/pierre-core/src/config/database.rs); without
      # these overrides every warm pod pre-opened 2 slots, so ~11 warm pods alone
      # exhausted the db-f1-micro's 22 usable connection slots (rev 00679 incident,
      # 2026-07-10). MIN=0 → idle/booting pods hold zero slots; MAX=6 keeps the
      # budget invariant max_instances(3) × 6 = 18 ≤ 22 (see terraform.tfvars).
      POSTGRES_MIN_CONNECTIONS = "0"
      POSTGRES_MAX_CONNECTIONS = "6"
      } : {
      # Fallback to ephemeral SQLite when Cloud SQL is disabled
      DATABASE_URL = "sqlite:./data/users.db"
    },
    var.enable_cache ? {
      REDIS_URL = module.cache[0].redis_url
    } : {},
    {
      # Cloud KMS KEK resource id (not a secret — key material never leaves KMS).
      # GcpKmsKekProvider wraps/unwraps the DEK against this key (ADR-017).
      PIERRE_KMS_KEY_RESOURCE = module.kms.key_resource
    },
  )

  secret_env_vars = {
    DB_PASSWORD = module.secrets.secret_ids["db_password"]
    # The database KEK is Cloud KMS (PIERRE_KMS_KEY_RESOURCE in env_vars); the DEK is
    # wrapped by KMS, so the env-backed master key is no longer wired (ADR-017).
    STRAVA_CLIENT_ID     = module.secrets.secret_ids["strava_client_id"]
    STRAVA_CLIENT_SECRET = module.secrets.secret_ids["strava_client_secret"]
    # WHOOP central OAuth app credentials. Empty until secrets are populated
    # out-of-band (`gcloud secrets versions add ...`); Cloud Run sees the
    # placeholder string, the OAuth manager logs the "no credentials" warn,
    # and the platform falls back to BYO (web/mobile setup modals). Once a
    # real version lands, Whoop becomes 1-step identical to Strava.
    WHOOP_CLIENT_ID         = module.secrets.secret_ids["whoop_client_id"]
    WHOOP_CLIENT_SECRET     = module.secrets.secret_ids["whoop_client_secret"]
    USDA_API_KEY            = module.secrets.secret_ids["usda_api_key"]
    GEMINI_API_KEY          = module.secrets.secret_ids["gemini_api_key"]
    COHERE_API_KEY          = module.secrets.secret_ids["cohere_api_key"]
    COPILOT_GITHUB_TOKEN    = module.secrets.secret_ids["copilot_github_token"]
    CLAUDE_CODE_OAUTH_TOKEN = module.secrets.secret_ids["claude_code_oauth_token"]
    # Bearer the sciotte client presents to the dedicated scraper service
    # (ADR-021); same secret the service itself gates on.
    DRAVR_SCIOTTE_API_KEY = module.secrets.secret_ids["dravr_sciotte_api_key"]
    OPENWEATHER_API_KEY   = module.secrets.secret_ids["openweather_api_key"]
    RESEND_API_KEY        = module.secrets.secret_ids["resend_api_key"]
    POSTHOG_API_KEY       = module.secrets.secret_ids["posthog_api_key"]

    # Messaging channel credentials (seeded into DB on startup)
    SLACK_BOT_TOKEN                  = module.secrets.secret_ids["slack_bot_token"]
    SLACK_SIGNING_SECRET             = module.secrets.secret_ids["slack_signing_secret"]
    TELEGRAM_BOT_TOKEN               = module.secrets.secret_ids["telegram_bot_token"]
    TELEGRAM_WEBHOOK_SECRET          = module.secrets.secret_ids["telegram_webhook_secret"]
    META_WHATSAPP_APP_SECRET         = module.secrets.secret_ids["meta_whatsapp_app_secret"]
    META_WHATSAPP_ACCESS_TOKEN       = module.secrets.secret_ids["meta_whatsapp_access_token"]
    META_MESSENGER_APP_SECRET        = module.secrets.secret_ids["meta_messenger_app_secret"]
    META_MESSENGER_PAGE_ACCESS_TOKEN = module.secrets.secret_ids["meta_messenger_page_access_token"]
    DISCORD_BOT_TOKEN                = module.secrets.secret_ids["discord_bot_token"]
    DISCORD_PUBLIC_KEY               = module.secrets.secret_ids["discord_public_key"]
    DISCORD_APPLICATION_ID           = module.secrets.secret_ids["discord_application_id"]
    DISCORD_BOT_PERMISSIONS          = module.secrets.secret_ids["discord_bot_permissions"]

    # Contremaitre prompt hot-reload credentials
    CONTREMAITRE_GITHUB_PAT     = module.secrets.secret_ids["contremaitre_github_pat"]
    CONTREMAITRE_WEBHOOK_SECRET = module.secrets.secret_ids["contremaitre_webhook_secret"]
  }

  health_check_path           = "/health"
  startup_probe_initial_delay = 10

  labels = merge(var.labels, { component = "backend" })

  depends_on = [module.networking, module.secrets, module.service_accounts]
}

# -----------------------------------------------------------------------------
# Seed Jobs (Cloud Run Jobs for database seeding)
# -----------------------------------------------------------------------------

locals {
  seed_env_vars = var.enable_database ? {
    DATABASE_HOST       = "/cloudsql/${module.database[0].connection_name}"
    DATABASE_NAME       = module.database[0].database_name
    DATABASE_USER       = module.database[0].database_user
    RUST_LOG            = "info"
    CONTREMAITRE_REPO   = "dravr-ai/dravr-contremaitre"
    CONTREMAITRE_BRANCH = "main"
  } : {}

  seed_secret_env_vars = {
    DB_PASSWORD             = module.secrets.secret_ids["db_password"]
    CONTREMAITRE_GITHUB_PAT = module.secrets.secret_ids["contremaitre_github_pat"]
  }

  seed_common = {
    project_id               = var.project_id
    region                   = var.region
    container_image          = var.backend_image
    service_account_email    = module.service_accounts.app_service_account_email
    vpc_connector_id         = module.networking.vpc_connector_id
    cloudsql_connection_name = var.enable_database ? module.database[0].connection_name : null
    cpu                      = "1"
    memory                   = "512Mi"
    max_retries              = 1
    timeout                  = "300s"
  }
}

module "seed_bootstrap" {
  source = "../../modules/cloud_run_jobs"

  project_id               = local.seed_common.project_id
  region                   = local.seed_common.region
  job_name                 = "${var.service_name}-seed-bootstrap"
  container_image          = local.seed_common.container_image
  service_account_email    = local.seed_common.service_account_email
  vpc_connector_id         = local.seed_common.vpc_connector_id
  cloudsql_connection_name = local.seed_common.cloudsql_connection_name
  cpu                      = local.seed_common.cpu
  memory                   = local.seed_common.memory
  max_retries              = local.seed_common.max_retries
  timeout                  = local.seed_common.timeout

  command = ["/app/seed-entrypoint.sh"]
  args    = ["bootstrap"]

  env_vars = merge(local.seed_env_vars, {
    ADMIN_EMAIL = "admin@dravr.ai"
  })

  secret_env_vars = merge(local.seed_secret_env_vars, {
    ADMIN_PASSWORD = module.secrets.secret_ids["admin_password"]
  })

  labels = merge(var.labels, { component = "seed-bootstrap" })

  depends_on = [module.networking, module.secrets, module.service_accounts]
}

module "seed_coaches" {
  source = "../../modules/cloud_run_jobs"

  project_id               = local.seed_common.project_id
  region                   = local.seed_common.region
  job_name                 = "${var.service_name}-seed-coaches"
  container_image          = local.seed_common.container_image
  service_account_email    = local.seed_common.service_account_email
  vpc_connector_id         = local.seed_common.vpc_connector_id
  cloudsql_connection_name = local.seed_common.cloudsql_connection_name
  cpu                      = local.seed_common.cpu
  memory                   = local.seed_common.memory
  max_retries              = local.seed_common.max_retries
  timeout                  = local.seed_common.timeout

  command = ["/app/seed-entrypoint.sh"]
  args    = ["coaches"]

  env_vars        = local.seed_env_vars
  secret_env_vars = local.seed_secret_env_vars

  labels = merge(var.labels, { component = "seed-coaches" })

  depends_on = [module.networking, module.secrets, module.service_accounts]
}

module "seed_mobility" {
  source = "../../modules/cloud_run_jobs"

  project_id               = local.seed_common.project_id
  region                   = local.seed_common.region
  job_name                 = "${var.service_name}-seed-mobility"
  container_image          = local.seed_common.container_image
  service_account_email    = local.seed_common.service_account_email
  vpc_connector_id         = local.seed_common.vpc_connector_id
  cloudsql_connection_name = local.seed_common.cloudsql_connection_name
  cpu                      = local.seed_common.cpu
  memory                   = local.seed_common.memory
  max_retries              = local.seed_common.max_retries
  timeout                  = local.seed_common.timeout

  command = ["/app/seed-entrypoint.sh"]
  args    = ["mobility"]

  env_vars        = local.seed_env_vars
  secret_env_vars = local.seed_secret_env_vars

  labels = merge(var.labels, { component = "seed-mobility" })

  depends_on = [module.networking, module.secrets, module.service_accounts]
}

module "seed_synthetic_activities" {
  source = "../../modules/cloud_run_jobs"

  project_id               = local.seed_common.project_id
  region                   = local.seed_common.region
  job_name                 = "${var.service_name}-seed-synthetic"
  container_image          = local.seed_common.container_image
  service_account_email    = local.seed_common.service_account_email
  vpc_connector_id         = local.seed_common.vpc_connector_id
  cloudsql_connection_name = local.seed_common.cloudsql_connection_name
  cpu                      = local.seed_common.cpu
  memory                   = local.seed_common.memory
  max_retries              = local.seed_common.max_retries
  timeout                  = local.seed_common.timeout

  command = ["/app/seed-entrypoint.sh"]
  args    = ["synthetic-activities", "--email", "alice@demo.pierre.dev", "--count", "100", "--days", "90"]

  # seed-synthetic runs the full KeyManager to write encrypted oauth tokens, so it
  # needs the KMS KEK resource and the gcp-kms binary to wrap/unwrap the DEK (ADR-017).
  env_vars = merge(local.seed_env_vars, {
    PIERRE_KMS_KEY_RESOURCE = module.kms.key_resource
  })
  secret_env_vars = local.seed_secret_env_vars

  labels = merge(var.labels, { component = "seed-synthetic" })

  depends_on = [module.networking, module.secrets, module.service_accounts, module.kms]
}

# -----------------------------------------------------------------------------
# SQL Client Job (for local debugging via gcloud run jobs execute)
# -----------------------------------------------------------------------------

module "sql_client" {
  count  = var.enable_database ? 1 : 0
  source = "../../modules/cloud_run_jobs"

  project_id               = var.project_id
  region                   = var.region
  job_name                 = "${var.service_name}-sql-client"
  container_image          = "postgres:15-alpine"
  service_account_email    = module.service_accounts.app_service_account_email
  vpc_connector_id         = module.networking.vpc_connector_id
  cloudsql_connection_name = module.database[0].connection_name
  cpu                      = "1"
  memory                   = "512Mi"
  max_retries              = 0
  timeout                  = "60s"

  command = ["psql"]
  args    = ["-c", "SELECT 1"]

  env_vars = {
    PGHOST     = "/cloudsql/${module.database[0].connection_name}"
    PGDATABASE = module.database[0].database_name
    PGUSER     = module.database[0].database_user
  }

  secret_env_vars = {
    PGPASSWORD = module.secrets.secret_ids["db_password"]
  }

  labels = merge(var.labels, { component = "sql-client" })

  depends_on = [module.networking, module.secrets, module.service_accounts]
}

# -----------------------------------------------------------------------------
# Admin Frontend (optional)
# -----------------------------------------------------------------------------

# -----------------------------------------------------------------------------
# Sciotte scraper service (ADR-021) — the dedicated headless-Chrome service the
# API routes sciotte logins/scrapes to when DRAVR_SCIOTTE_REMOTE_URL is set.
# One multi-provider instance (garmin+strava, bare `serve`), scale-to-zero:
# min=0 + cpu_idle=true make idle cost ~$0 (per-request billing), unlike the
# API pod's ADR-019 floor. maxInstances=1 until the affinity-cookie scale-out
# lands (parked 2FA browsers are instance-bound; scrapes re-import and are not).
# -----------------------------------------------------------------------------
module "sciotte" {
  count  = var.enable_sciotte_service ? 1 : 0
  source = "../../modules/cloud_run"

  project_id            = var.project_id
  region                = var.region
  service_name          = "${var.service_name}-sciotte"
  container_image       = var.sciotte_image
  service_account_email = module.service_accounts.app_service_account_email

  container_port = 3000
  # 2 vCPU / 2Gi sized for DRAVR_SCIOTTE_MAX_CONCURRENT=2 headless Chromes
  # (~300-500MB each) plus the vision LLM subprocess headroom.
  cpu               = "2"
  memory            = "2Gi"
  cpu_idle          = true
  startup_cpu_boost = true
  min_instances     = 0
  max_instances     = 1
  # Scrapes are long-lived; keep the per-instance request budget near the
  # queue depth so overload sheds as app-level 503+Retry-After, not as a
  # Cloud Run queue pile-up.
  max_instance_request_concurrency = 10
  # Must outlast the slowest step the platform client waits on (330s parked
  # 2FA window; first Strava scrape measured >120s with pagination + N+1).
  request_timeout = "600s"

  # App-level bearer (DRAVR_SCIOTTE_API_KEY) is the auth boundary: the API pod
  # egresses PRIVATE_RANGES_ONLY, so an internal-only ingress here would drop
  # its calls. Revisit to IAM ID-token auth or internal ingress + ALL_TRAFFIC
  # backend egress if the posture needs tightening.
  ingress               = "INGRESS_TRAFFIC_ALL"
  allow_unauthenticated = true

  # Hot-swappable extraction scripts / prompt overrides, same bucket the API
  # pod mounts today (both keep it until the Phase 4 in-process cutover).
  gcs_volumes = {
    sciotte-scripts = {
      bucket     = google_storage_bucket.sciotte_scripts.name
      mount_path = "/sciotte-scripts"
      read_only  = true
    }
  }

  env_vars = {
    # Backpressure limiter — fail-fast required set (no crate defaults).
    # max_concurrent=2 matches the 2Gi memory sizing above.
    DRAVR_SCIOTTE_MAX_CONCURRENT          = "2"
    DRAVR_SCIOTTE_MAX_QUEUE               = "8"
    DRAVR_SCIOTTE_QUEUE_TIMEOUT_SECS      = "10"
    DRAVR_SCIOTTE_PARKED_PERMIT_TTL_SECS  = "300"
    DRAVR_SCIOTTE_WATCHDOG_INTERVAL_SECS  = "15"
    DRAVR_SCIOTTE_RETRY_AFTER_HINT_SECS   = "5"
    DRAVR_SCIOTTE_CLOSED_RETRY_AFTER_SECS = "60"

    # Interactive-login windows — one source of truth with the API pod's
    # in-process path during the migration.
    DRAVR_SCIOTTE_LOGIN_TIMEOUT         = tostring(var.backend_sciotte_login_timeout_secs)
    DRAVR_SCIOTTE_PASSWORD_STEP_TIMEOUT = tostring(var.backend_sciotte_password_step_timeout_secs)
    DRAVR_SCIOTTE_PHONE_TAP_TIMEOUT     = tostring(var.backend_sciotte_phone_tap_timeout_secs)

    # Hybrid login: selectors first, vision (Copilot screenshot reasoning) on
    # failure — required for the Strava/Google OAuth path (validated live).
    DRAVR_SCIOTTE_LOGIN_MODE  = "hybrid"
    COPILOT_HEADLESS_MODEL    = "claude-opus-4.8"
    DRAVR_SCIOTTE_SCRIPTS_DIR = "/sciotte-scripts"
  }

  secret_env_vars = {
    # Bearer every REST/MCP request is gated on (the API pod presents it).
    DRAVR_SCIOTTE_API_KEY = module.secrets.secret_ids["dravr_sciotte_api_key"]
    # Vision LLM credential (Copilot CLI), same secret the API pod uses.
    COPILOT_GITHUB_TOKEN = module.secrets.secret_ids["copilot_github_token"]
  }

  health_check_path           = "/health"
  startup_probe_initial_delay = 5

  labels = merge(var.labels, { component = "sciotte" })

  depends_on = [module.service_accounts, module.secrets]
}

module "frontend" {
  count  = var.enable_frontend ? 1 : 0
  source = "../../modules/cloud_run"

  project_id            = var.project_id
  region                = var.region
  service_name          = "${var.service_name}-frontend"
  container_image       = var.frontend_image
  service_account_email = module.service_accounts.app_service_account_email

  container_port = 8080
  cpu            = "1"
  memory         = "1Gi"
  # Instance-based billing: this nginx service is the always-up reverse-proxy
  # front door — every API, OAuth callback, and Firebase /__/ request flows
  # through it — so allocated-CPU's lower per-second rate beats per-request
  # (cpu_idle=true) billing. GCP Active Assist measured ~$14/mo cheaper here.
  # Unlike ADR-019's ACP-runner idle floor, min_instances=0 keeps this at $0 when idle.
  cpu_idle          = false
  startup_cpu_boost = false
  min_instances     = var.frontend_min_instances
  max_instances     = var.frontend_max_instances
  # Browser->nginx leg must outlast the proxied backend turn (600s) so the
  # frontend Cloud Run service doesn't cut the request before nginx does.
  request_timeout = "600s"

  ingress               = "INGRESS_TRAFFIC_ALL"
  allow_unauthenticated = true
  vpc_connector_id      = module.networking.vpc_connector_id
  vpc_egress            = "ALL_TRAFFIC"

  env_vars = {
    # Backend URL for nginx reverse proxy (injected via envsubst at container start)
    BACKEND_URL = module.backend.service_url
    # Firebase project ID for self-hosted auth handler (nginx proxies /__/ to firebaseapp.com)
    FIREBASE_PROJECT_ID = var.firebase_project_id
  }

  health_check_path           = "/health"
  startup_probe_initial_delay = 3

  labels = merge(var.labels, { component = "frontend" })

  depends_on = [module.networking, module.service_accounts, module.backend]
}

# -----------------------------------------------------------------------------
# Storage (optional, depends on APIs)
# -----------------------------------------------------------------------------

module "storage" {
  source = "../../modules/storage"

  project_id                    = var.project_id
  region                        = var.region
  service_name                  = var.service_name
  create_app_bucket             = false
  create_terraform_state_bucket = false
  labels                        = var.labels

  depends_on = [module.project]
}

# -----------------------------------------------------------------------------
# Firebase Identity Platform (authentication)
# -----------------------------------------------------------------------------

module "firebase" {
  source = "../../modules/firebase"

  project_id          = var.project_id
  firebase_project_id = var.firebase_project_id

  depends_on = [module.project, module.secrets]
}


# Secret backup: run `./scripts/sync-secrets-to-github.sh` to mirror GCP secrets to GitHub
# Intentionally NOT in Terraform to avoid secrets in tfstate
