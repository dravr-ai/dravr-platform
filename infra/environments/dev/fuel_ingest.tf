# ABOUTME: Weekly fuel-product ingestion job — files catalogue proposals from brands' Shopify labels, read by OCR
# ABOUTME: Cloud Run job + paused weekly Cloud Scheduler trigger + run.invoker IAM for `pierre-cli fuel-ingest` (carnet#155)
#
# Background
# ----------
# The fuel-product bank (carnet#155) fills its review queue from the labels
# the brands themselves publish. `pierre-cli fuel-ingest` lists each
# checked-in brand's Shopify `/products.json`, downloads the label photos,
# reads them through the platform's LLM chain (embacle, contremaitre prompt
# `fuel_panel_extraction`) and files one proposal per new or changed product.
# A proposal never reaches the catalogue until an operator approves it in the
# admin console, so a bad read costs review time, never an athlete's plan.
#
# Every read the job pays for is recorded in `fuel_ingest_reads` under a hash
# of its photos, the prompt text and the configured model, whatever it found
# (a proposal, an illegible label, a duplicate, an unchanged product, an
# answer that did not parse): an unchanged weekly run makes no LLM call, and a
# new photo, prompt or model reads again.
#
# The job's name ends in `-job`, so the dravr-mcp-server-job-failures alert
# (monitoring.tf) already counts its failures: the command exits non-zero when
# every label read failed, every candidate failed outside the read (database,
# downloads), or no brand storefront could be listed.

# -----------------------------------------------------------------------------
# Cloud Run Job — fuel-ingest-job
# -----------------------------------------------------------------------------
# Same image, Cloud SQL path and entrypoint as the seed jobs (local.seed_common
# in main.tf): seed-entrypoint.sh builds DATABASE_URL and execs
# `pierre-cli fuel-ingest`. It is the first job that carries LLM credentials,
# bound below exactly as the backend binds them.

module "fuel_ingest" {
  source = "../../modules/cloud_run_jobs"

  project_id               = local.seed_common.project_id
  region                   = local.seed_common.region
  job_name                 = "${var.service_name}-fuel-ingest-job"
  container_image          = local.seed_common.container_image
  service_account_email    = local.seed_common.service_account_email
  vpc_connector_id         = local.seed_common.vpc_connector_id
  cloudsql_connection_name = local.seed_common.cloudsql_connection_name
  cpu                      = local.seed_common.cpu
  # Up to eight label photos per package, at most 14 MB together, are held
  # and base64-encoded at once; the seed jobs' 512Mi leaves no room for that
  # beside the runtime.
  memory = "1Gi"

  # A rerun pays for no read twice (every read is recorded under its hash),
  # but it reads every new label on the LLM's quota: fail once, alert, and
  # let the next weekly run pick up.
  max_retries = 0
  # Four brands, each label read one LLM call of up to a minute, plus the
  # downloads: an hour leaves room for a full first run.
  timeout = "3600s"

  command = ["/app/seed-entrypoint.sh"]
  args    = ["fuel-ingest"]

  env_vars = merge(local.seed_env_vars, {
    # The backend's chain, verbatim (main.tf, the backend's PIERRE_LLM_CHAIN
    # and its rationale): JF's call is that the job reads labels through the
    # normal chain, not a separate vision-only one. The command discards an
    # answer from a tier that does not advertise vision, so a tier that drops
    # the images can never file invented values. Change both together.
    PIERRE_LLM_CHAIN         = "anthropic=claude-opus-5-5,anthropic#2=claude-haiku-5-5,anthropic#3=claude-haiku-5-5,copilot_sdk=claude-sonnet-5.5,claude_code=sonnet,gemini=gemini-flash-lite-latest"
    PIERRE_LLM_DEFAULT_MODEL = "gemini-flash-lite-latest"
    # No PIERRE_LLM_MODEL_FUEL_PANEL_EXTRACTION: labels are read by the head's
    # own model (Opus). Haiku put numbers on panels it called illegible in the
    # 2026-10-29 Näak dry run; with ~150 SKUs each read once, accuracy is worth
    # the credit. The model is part of the read hash, so changing it here
    # re-reads every label once.
    # The job never offers the Copilot runtime a tool or a file write.
    COPILOT_SDK_PERMISSION_POLICY = "deny_all"
  })

  secret_env_vars = merge(local.seed_secret_env_vars, {
    ANTHROPIC_API_KEY         = module.secrets.secret_ids["anthropic_api_key"]
    ANTHROPIC_API_KEY_2       = module.secrets.secret_ids["anthropic_api_key_2"]
    ANTHROPIC_API_KEY_3       = module.secrets.secret_ids["anthropic_api_key_3"]
    COPILOT_GITHUB_TOKEN      = module.secrets.secret_ids["copilot_github_token"]
    CLAUDE_CODE_OAUTH_TOKEN   = module.secrets.secret_ids["claude_code_oauth_token"]
    CLAUDE_CODE_OAUTH_TOKEN_2 = module.secrets.secret_ids["claude_code_oauth_token_2"]
    GEMINI_API_KEY            = module.secrets.secret_ids["gemini_api_key"]
  })

  labels = merge(var.labels, { component = "fuel-ingest" })

  depends_on = [module.networking, module.secrets, module.service_accounts]
}

# -----------------------------------------------------------------------------
# Cloud Scheduler — weekly, Monday 04:37 UTC
# -----------------------------------------------------------------------------
# Brands change a label a few times a year; a weekly pass is plenty, and a
# rerun only downloads photos for packages already read (no LLM call).
# Off the hour, like every scheduled lane here.
#
# Paused on purpose: the branch is tested on JF's local stack first, and an
# unrelated dev apply must not start spending LLM credit on label reads.
# Flip `paused` to false (and apply) to start the weekly runs.
#
# Idle-cost: none, it starts a Cloud Run job through the :run API once a week;
# the job reads the database, the brands' storefronts and the LLM providers,
# and wakes no service (check-keep-warm-traffic.sh).

resource "google_cloud_scheduler_job" "fuel_ingest_weekly" {
  project          = var.project_id
  region           = var.region
  name             = "${var.service_name}-fuel-ingest-weekly"
  description      = "Weekly fuel-product label ingestion (pierre-cli fuel-ingest). Non-zero exit triggers dravr-mcp-server-job-failures alert."
  schedule         = "37 4 * * 1"
  time_zone        = "Etc/UTC"
  attempt_deadline = "320s"
  paused           = true

  retry_config {
    # A failed run waits for next week; Scheduler must not start a second one.
    retry_count = 0
  }

  http_target {
    http_method = "POST"
    uri         = "https://${var.region}-run.googleapis.com/apis/run.googleapis.com/v1/namespaces/${var.project_id}/jobs/${module.fuel_ingest.job_name}:run"

    oauth_token {
      service_account_email = module.service_accounts.app_service_account_email
      scope                 = "https://www.googleapis.com/auth/cloud-platform"
    }
  }

  depends_on = [module.fuel_ingest]
}

# -----------------------------------------------------------------------------
# IAM — let the app service account invoke the fuel-ingest Cloud Run job
# -----------------------------------------------------------------------------
# Cloud Run v2 jobs require run.invoker on the job resource for the :run
# endpoint; the Scheduler's token is issued for the app service account.

resource "google_cloud_run_v2_job_iam_member" "fuel_ingest_invoker" {
  project  = var.project_id
  location = var.region
  name     = module.fuel_ingest.job_name
  role     = "roles/run.invoker"
  member   = "serviceAccount:${module.service_accounts.app_service_account_email}"

  depends_on = [module.fuel_ingest]
}
