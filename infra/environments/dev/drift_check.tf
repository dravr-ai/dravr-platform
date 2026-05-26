# ABOUTME: Daily contremaitre->prod DB drift check — surfaces silent prompt divergence
# ABOUTME: Added 2026-05-25 so prompt edits in contremaitre that never reach prod get caught
#
# Background
# ----------
# Even with a working seed-coaches job, contremaitre prompt edits don't
# reach prod until the next manual `gcloud run jobs execute seed-coaches`
# trigger. There was no signal that surfaced when the two fell out of
# sync. On 2026-05-25 we discovered we had been running with stale
# coach content for 3.5 weeks and had no way to know.
#
# This file adds a daily drift gate:
#   1. A new Cloud Run Job (drift-check-coaches) that clones contremaitre,
#      recomputes the canonical content hash for every coach file, and
#      compares against the `coaches.content_hash` column. Hash mismatches
#      exit non-zero.
#   2. A Cloud Scheduler trigger that fires the job at 06:00 UTC daily.
#   3. The job's naming (drift-check-coaches) matches the alert filter in
#      monitoring.tf, so non-zero exits automatically page the
#      dev-dravr-errors Slack channel via the dravr-mcp-server-job-failures
#      alert policy.

# -----------------------------------------------------------------------------
# Cloud Run Job — drift-check-coaches
# -----------------------------------------------------------------------------
# Reuses the same image, env vars, and Cloud SQL access path as the existing
# seed_coaches job (see local.seed_common in main.tf) — the binary's
# `check-drift coaches` subcommand connects to the DB, clones contremaitre,
# and compares hashes.

module "drift_check_coaches" {
  source = "../../modules/cloud_run_jobs"

  project_id               = local.seed_common.project_id
  region                   = local.seed_common.region
  job_name                 = "${var.service_name}-drift-check-coaches"
  container_image          = local.seed_common.container_image
  service_account_email    = local.seed_common.service_account_email
  vpc_connector_id         = local.seed_common.vpc_connector_id
  cloudsql_connection_name = local.seed_common.cloudsql_connection_name
  cpu                      = local.seed_common.cpu
  memory                   = local.seed_common.memory

  # Don't retry on failure — a drift signal SHOULD bubble up to the alert.
  # Retrying would mask intermittent contremaitre clone failures (transient
  # GitHub 5xx) too, but for the alerting story we'd rather have a noisy
  # false-positive once a quarter than miss a real drift.
  max_retries = 0
  timeout     = local.seed_common.timeout

  command = ["/app/seed-entrypoint.sh"]
  args    = ["check-drift", "coaches"]

  env_vars        = local.seed_env_vars
  secret_env_vars = local.seed_secret_env_vars

  labels = merge(var.labels, { component = "drift-check-coaches" })

  depends_on = [module.networking, module.secrets, module.service_accounts]
}

# -----------------------------------------------------------------------------
# Cloud Scheduler — daily 06:00 UTC drift check
# -----------------------------------------------------------------------------
# 06:00 UTC = 02:00 EDT = low-traffic window. Cloud Scheduler authenticates
# to the Cloud Run Job's :run endpoint with an OIDC token issued for the
# app service account (which already has run.invoker on its own jobs).

resource "google_cloud_scheduler_job" "drift_check_coaches_daily" {
  project          = var.project_id
  region           = var.region
  name             = "${var.service_name}-drift-check-coaches-daily"
  description      = "Daily contremaitre->DB drift check at 06:00 UTC. Non-zero exit triggers dravr-mcp-server-job-failures alert."
  schedule         = "0 6 * * *"
  time_zone        = "Etc/UTC"
  attempt_deadline = "320s"

  retry_config {
    # The job itself already classifies retryable vs non-retryable failures;
    # don't let Scheduler double-trigger drift alerts.
    retry_count = 0
  }

  http_target {
    http_method = "POST"
    uri         = "https://${var.region}-run.googleapis.com/apis/run.googleapis.com/v1/namespaces/${var.project_id}/jobs/${module.drift_check_coaches.job_name}:run"

    oauth_token {
      service_account_email = module.service_accounts.app_service_account_email
      scope                 = "https://www.googleapis.com/auth/cloud-platform"
    }
  }

  depends_on = [module.drift_check_coaches]
}

# -----------------------------------------------------------------------------
# IAM — let the app service account invoke its own drift-check Cloud Run job
# -----------------------------------------------------------------------------
# Cloud Run v2 jobs require run.invoker on the specific job resource to
# trigger via the :run endpoint. The app SA needs this permission so the
# Scheduler-issued OIDC token (impersonating the app SA) succeeds.

resource "google_cloud_run_v2_job_iam_member" "drift_check_invoker" {
  project  = var.project_id
  location = var.region
  name     = module.drift_check_coaches.job_name
  role     = "roles/run.invoker"
  member   = "serviceAccount:${module.service_accounts.app_service_account_email}"

  depends_on = [module.drift_check_coaches]
}
