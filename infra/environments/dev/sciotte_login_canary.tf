# ABOUTME: Garmin credential-login canary — a fresh login through the deployed sciotte service every 6 h
# ABOUTME: Added 2026-10-16 (carnet#1078) after a Garmin captcha blocked every new athlete for a week unnoticed
#
# Background
# ----------
# From about 2026-10-09 until 2026-10-16 no new athlete could connect Garmin:
# Garmin put a captcha in front of the sign-in and sciotte waited out its
# whole budget behind it (carnet#1035). The only signal was athletes failing.
#
# This job signs a dedicated canary account in through the deployed
# dravr-mcp-server-sciotte service — the revision, flags and Cloud Run egress
# athletes get — with `pierre-cli canary sciotte-login --provider garmin`,
# and stops at the first decision point (a code step passes; a captcha, a
# stall or a refusal fails). A failed run logs at ERROR, which the
# dravr-mcp-server-job-failures alert counts: the job name ends in `-job` so
# the alert's filter (monitoring.tf) already covers it.
#
# Every run is one real Garmin login from prod's egress on one account, so
# the cadence stays at 6 h: a faster canary would itself feed Garmin's risk
# score.
#
# The image is not pinned here (the module ignores it); publish-images.yml
# re-points this job at each new server image, like the seed jobs.

# -----------------------------------------------------------------------------
# The canary account (filled by hand: never in terraform state)
# -----------------------------------------------------------------------------

resource "google_secret_manager_secret" "sciotte_canary_email" {
  project   = var.project_id
  secret_id = "${var.service_name}-sciotte-canary-email"
  labels    = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "sciotte_canary_email_placeholder" {
  secret      = google_secret_manager_secret.sciotte_canary_email.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

resource "google_secret_manager_secret" "sciotte_canary_password" {
  project   = var.project_id
  secret_id = "${var.service_name}-sciotte-canary-password"
  labels    = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "sciotte_canary_password_placeholder" {
  secret      = google_secret_manager_secret.sciotte_canary_password.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

# -----------------------------------------------------------------------------
# Cloud Run Job — sciotte-login-canary-job
# -----------------------------------------------------------------------------
# Runs as the app service account, which the sciotte service already admits
# (DRAVR_SCIOTTE_ALLOWED_CALLERS) and which reads every project secret. It
# needs no database: no Cloud SQL socket.

module "sciotte_login_canary" {
  count  = var.backend_sciotte_remote ? 1 : 0
  source = "../../modules/cloud_run_jobs"

  project_id               = var.project_id
  region                   = var.region
  job_name                 = "${var.service_name}-sciotte-login-canary-job"
  container_image          = var.backend_image
  service_account_email    = module.service_accounts.app_service_account_email
  vpc_connector_id         = module.networking.vpc_connector_id
  cloudsql_connection_name = null
  cpu                      = "1"
  memory                   = "512Mi"

  # A retry would be a second Garmin login inside the same minute: the
  # verdict of one run is the signal.
  max_retries = 0
  # SciotteClient gives a login step up to 330 s (above the service's own
  # 320 s request deadline); the task outlasts it so every run prints a verdict.
  timeout = "360s"

  command = ["/app/pierre-cli"]
  args    = ["canary", "sciotte-login", "--provider", "garmin"]

  env_vars = {
    DRAVR_SCIOTTE_REMOTE_URL = module.sciotte[0].service_url
    DRAVR_SCIOTTE_AUDIENCE   = local.sciotte_audience
    RUST_LOG                 = "info"
  }

  secret_env_vars = {
    SCIOTTE_CANARY_EMAIL    = google_secret_manager_secret.sciotte_canary_email.secret_id
    SCIOTTE_CANARY_PASSWORD = google_secret_manager_secret.sciotte_canary_password.secret_id
  }

  labels = merge(var.labels, { component = "sciotte-login-canary" })

  depends_on = [
    module.networking,
    module.service_accounts,
    google_secret_manager_secret_version.sciotte_canary_email_placeholder,
    google_secret_manager_secret_version.sciotte_canary_password_placeholder,
  ]
}

# -----------------------------------------------------------------------------
# Cloud Scheduler — every 6 h, off the hour
# -----------------------------------------------------------------------------

resource "google_cloud_scheduler_job" "sciotte_login_canary" {
  count            = var.backend_sciotte_remote ? 1 : 0
  project          = var.project_id
  region           = var.region
  name             = "${var.service_name}-sciotte-login-canary"
  description      = "Garmin credential-login canary every 6 h (carnet#1078). A stopped login fails the job and fires dravr-mcp-server-job-failures."
  schedule         = "17 */6 * * *"
  time_zone        = "Etc/UTC"
  attempt_deadline = "320s"

  # No retry_config block: a retried trigger would be a second real login, and
  # one verdict per slot is the point. Zero retries is Scheduler's default, and
  # a block holding only `retry_count = 0` is an empty message the API does not
  # store, so declaring it made every drift check plan to add it back.

  http_target {
    http_method = "POST"
    uri         = "https://${var.region}-run.googleapis.com/apis/run.googleapis.com/v1/namespaces/${var.project_id}/jobs/${module.sciotte_login_canary[0].job_name}:run"

    oauth_token {
      service_account_email = module.service_accounts.app_service_account_email
      scope                 = "https://www.googleapis.com/auth/cloud-platform"
    }
  }

  depends_on = [module.sciotte_login_canary]
}

resource "google_cloud_run_v2_job_iam_member" "sciotte_login_canary_invoker" {
  count    = var.backend_sciotte_remote ? 1 : 0
  project  = var.project_id
  location = var.region
  name     = module.sciotte_login_canary[0].job_name
  role     = "roles/run.invoker"
  member   = "serviceAccount:${module.service_accounts.app_service_account_email}"

  depends_on = [module.sciotte_login_canary]
}
