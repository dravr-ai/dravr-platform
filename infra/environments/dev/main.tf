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
  # Sister repos under the same GitHub org whose Actions also run against
  # GCP. Currently: dravr-contremaitre's mirror Action that uploads prompt
  # snapshots into the contremaitre_prompts bucket.
  additional_repositories = ["dravr-contremaitre"]

  depends_on = [module.service_accounts]
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
# Contremaitre Prompts Bucket — read-path source of truth for prompts/tools/
# strings/evidence/config files mirrored from the dravr-contremaitre GitHub
# repo. A GitHub Action on that repo `gsutil rsync`s into this bucket on
# every push to `main`. Pierre reads from here at runtime; this avoids the
# 5000/hr per-GitHub-user core API rate limit shared with the Copilot LLM
# provider (both use PATs owned by `jfarcand`).
# -----------------------------------------------------------------------------

resource "google_storage_bucket" "contremaitre_prompts" {
  name          = "${var.project_id}-contremaitre-prompts"
  project       = var.project_id
  location      = var.region
  force_destroy = true

  uniform_bucket_level_access = true

  # Versioning protects against a bad mirror push; we can roll back by
  # restoring an older generation rather than waiting for a re-push.
  versioning {
    enabled = true
  }

  lifecycle_rule {
    condition {
      num_newer_versions = 10
    }
    action {
      type = "Delete"
    }
  }

  labels = merge(var.labels, { component = "contremaitre" })
}

# Grant the Cloud Run app service account read access to the bucket so the
# Pierre server can `read_file` / `read_manifest` against it.
resource "google_storage_bucket_iam_member" "contremaitre_prompts_app_reader" {
  bucket = google_storage_bucket.contremaitre_prompts.name
  role   = "roles/storage.objectViewer"
  member = "serviceAccount:${module.service_accounts.app_service_account_email}"

  depends_on = [module.service_accounts]
}

# Service account the dravr-contremaitre GitHub Action assumes via WIF to
# upload the rendered repo tree on every push.
resource "google_service_account" "contremaitre_mirror" {
  project      = var.project_id
  account_id   = "contremaitre-mirror"
  display_name = "Contremaitre GCS Mirror"
  description  = "Used by dravr-contremaitre's GitHub Action to upload prompt files to GCS."
}

resource "google_storage_bucket_iam_member" "contremaitre_prompts_mirror_writer" {
  bucket = google_storage_bucket.contremaitre_prompts.name
  # `roles/storage.admin` at bucket scope (NOT project scope) — gives the
  # mirror SA the `storage.buckets.get` permission `gcloud storage rsync`
  # needs to look up bucket metadata before walking objects, plus the
  # object-level CRUD already covered by objectAdmin. Scoping to the
  # bucket keeps the SA from touching anything else in the project.
  role   = "roles/storage.admin"
  member = "serviceAccount:${google_service_account.contremaitre_mirror.email}"
}

# Allow the dravr-contremaitre repo's GitHub Action to impersonate the
# mirror service account through the existing GitHub WIF pool. The pool
# itself is created by the workload_identity module above; we just bind
# this SA to a different repo's principalSet.
resource "google_service_account_iam_member" "contremaitre_mirror_workload_identity" {
  service_account_id = google_service_account.contremaitre_mirror.name
  role               = "roles/iam.workloadIdentityUser"
  member = format(
    "principalSet://iam.googleapis.com/%s/attribute.repository/%s/dravr-contremaitre",
    module.workload_identity.pool_name,
    var.github_org
  )
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
  # Disable CPU throttling so background tasks (Discord Gateway WebSocket) run between requests
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
      # Primary = claude-opus-4.7 (high-reasoning Opus variant, not the -fast SKU).
      # Fallback model = claude-sonnet-4.6 in case Opus is unavailable or rate-limited
      # (intra-provider model fallback, same Copilot session).
      # Runtime fallback chain = cross-provider failover to Gemini when Copilot
      # itself returns a retryable error (auth, 5xx, transient). Built by
      # ChatProvider::Chain in crates/pierre-llm/src/provider.rs and gated on
      # PIERRE_LLM_RUNTIME_FALLBACK=true. Without this, a Copilot session token
      # refresh failure (e.g. transient GitHub rate-limit during the
      # api.github.com/copilot_internal token exchange) surfaces as a
      # user-facing "Dravr temporairement indisponible" instead of seamlessly
      # serving the reply from Gemini. Wired live 2026-05-12 after that
      # exact failure mode took down chat for ~10 min.
      # Entitlement verified 2026-04-16 against the dev PAT; health probe
      # (copilot-health-probe.yml) exercises this model daily.
      PIERRE_LLM_PROVIDER          = "copilot_headless"
      PIERRE_LLM_MODEL             = "claude-opus-4.7"
      PIERRE_LLM_DEFAULT_MODEL     = "claude-opus-4.7"
      PIERRE_LLM_FALLBACK_MODEL    = "claude-sonnet-4.6"
      PIERRE_LLM_RUNTIME_FALLBACK  = "true"
      PIERRE_LLM_FALLBACK_PROVIDER = "gemini"

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

      # Contremaitre prompt hot-reload — reads from GCS at runtime so the
      # 5000/hr GitHub core API budget on `jfarcand` stays available for
      # the Copilot LLM provider. Writes (admin coach-promotion) still
      # commit to the repo via CONTREMAITRE_GITHUB_PAT.
      CONTREMAITRE_REPO       = "dravr-ai/dravr-contremaitre"
      CONTREMAITRE_BRANCH     = "main"
      CONTREMAITRE_GCS_BUCKET = google_storage_bucket.contremaitre_prompts.name

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
    },
    # Cloud SQL components — entrypoint.sh assembles these into DATABASE_URL
    var.enable_database ? {
      DATABASE_HOST = "/cloudsql/${module.database[0].connection_name}"
      DATABASE_NAME = module.database[0].database_name
      DATABASE_USER = module.database[0].database_user
      } : {
      # Fallback to ephemeral SQLite when Cloud SQL is disabled
      DATABASE_URL = "sqlite:./data/users.db"
    },
    var.enable_cache ? {
      REDIS_URL = module.cache[0].redis_url
    } : {},
  )

  secret_env_vars = {
    DB_PASSWORD                  = module.secrets.secret_ids["db_password"]
    PIERRE_MASTER_ENCRYPTION_KEY = module.secrets.secret_ids["encryption_key"]
    STRAVA_CLIENT_ID             = module.secrets.secret_ids["strava_client_id"]
    STRAVA_CLIENT_SECRET         = module.secrets.secret_ids["strava_client_secret"]
    USDA_API_KEY                 = module.secrets.secret_ids["usda_api_key"]
    GEMINI_API_KEY               = module.secrets.secret_ids["gemini_api_key"]
    COPILOT_GITHUB_TOKEN         = module.secrets.secret_ids["copilot_github_token"]
    OPENWEATHER_API_KEY          = module.secrets.secret_ids["openweather_api_key"]
    RESEND_API_KEY               = module.secrets.secret_ids["resend_api_key"]
    POSTHOG_API_KEY              = module.secrets.secret_ids["posthog_api_key"]

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
    DATABASE_HOST = "/cloudsql/${module.database[0].connection_name}"
    DATABASE_NAME = module.database[0].database_name
    DATABASE_USER = module.database[0].database_user
    RUST_LOG      = "info"
  } : {}

  seed_secret_env_vars = {
    DB_PASSWORD = module.secrets.secret_ids["db_password"]
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
  args    = ["coaches", "--coaches-dir", "/app/coaches"]

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

  env_vars        = local.seed_env_vars
  secret_env_vars = local.seed_secret_env_vars

  labels = merge(var.labels, { component = "seed-synthetic" })

  depends_on = [module.networking, module.secrets, module.service_accounts]
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

module "frontend" {
  count  = var.enable_frontend ? 1 : 0
  source = "../../modules/cloud_run"

  project_id            = var.project_id
  region                = var.region
  service_name          = "${var.service_name}-frontend"
  container_image       = var.frontend_image
  service_account_email = module.service_accounts.app_service_account_email

  container_port    = 8080
  cpu               = "1"
  memory            = "1Gi"
  cpu_idle          = true
  startup_cpu_boost = false
  min_instances     = var.frontend_min_instances
  max_instances     = var.frontend_max_instances

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
