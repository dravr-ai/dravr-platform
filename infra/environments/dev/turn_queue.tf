# ABOUTME: The Cloud Tasks queue that delivers a messaging turn to the backend as a request Cloud Run can see
# ABOUTME: Queue, its retry policy, the three IAM bindings the enqueue-and-dispatch loop needs, and the target URL
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A messaging turn runs after its webhook answered 200, so Cloud Run reads the
# instance as idle from the athlete's first second and an idle scaledown can
# take a live coaching turn with it (carnet#126). The backend enqueues each
# turn here instead of running it detached; Cloud Tasks delivers it as
# `POST /internal/turns/{id}/run` at the backend's own run.app URL, and a turn
# that runs inside a request is one Cloud Run waits for before it shuts the
# instance down. Cloud Tasks traffic counts as internal for the backend's
# INGRESS_TRAFFIC_INTERNAL_ONLY when it targets the default run.app URL of a
# service in the same project, which is why the target below is that URL and
# never the custom domain. The first million operations a month are free and a
# turn costs two or three, so the queue adds nothing to the bill; the
# warm-instance alternative (backend_min_instances = 1, about $140/month) is
# what this replaces.
#
# Deploy order is free in both directions. The binary reads PIERRE_TURN_RUNNER
# and runs turns in-process when it is absent, so code deployed before this is
# applied keeps today's behaviour; a queue applied before the code lands sits
# unused. The manual apply (see the deploy-infra skill) is what arms it.

locals {
  # The backend's own run.app URL, spelled out rather than read from
  # module.backend: the value feeds module.backend's env vars, and a module
  # referencing its own output is a cycle Terraform refuses. Cloud Run's
  # deterministic form is `<service>-<project number>.<region>.run.app`.
  backend_run_url = "https://${var.service_name}-api-${data.google_project.current.number}.${var.region}.run.app"
}

resource "google_cloud_tasks_queue" "turns" {
  name     = "${var.service_name}-turns"
  location = var.region
  project  = var.project_id

  # A blocked claim — an older turn in the same conversation still running, or
  # an instance draining — answers 409, and the retry is what "resume" means on
  # this path. Unlimited attempts inside an hour, backing off to a minute, so no
  # follow-up message is ever dropped by attempt exhaustion; 429 and 503 are
  # never returned by the worker because Cloud Tasks throttles the whole queue
  # on them.
  retry_config {
    max_attempts       = -1
    max_retry_duration = "3600s"
    min_backoff        = "5s"
    max_backoff        = "60s"
    max_doublings      = 3
  }

  # Turns are few and long; the ceiling is headroom against a burst after a
  # rollout, not a throttle.
  rate_limits {
    max_concurrent_dispatches = 20
    max_dispatches_per_second = 5
  }

  depends_on = [module.project]
}

# The service agent Cloud Tasks mints OIDC tokens through. Declaring it makes
# the agent exist before the binding below names it; enabling the API alone
# creates it lazily, and an IAM member for a principal that does not exist yet
# fails the apply.
resource "google_project_service_identity" "cloudtasks" {
  provider = google-beta
  project  = var.project_id
  service  = "cloudtasks.googleapis.com"

  depends_on = [module.project]
}

# The app service account creates tasks on the queue.
resource "google_cloud_tasks_queue_iam_member" "turns_enqueuer" {
  project  = var.project_id
  location = google_cloud_tasks_queue.turns.location
  name     = google_cloud_tasks_queue.turns.name
  role     = "roles/cloudtasks.enqueuer"
  member   = "serviceAccount:${module.service_accounts.app_service_account_email}"
}

# Each task carries an OIDC token minted as the app service account; the Cloud
# Tasks service agent is what mints it and must be allowed to act as that
# account.
resource "google_service_account_iam_member" "tasks_agent_acts_as_app" {
  service_account_id = module.service_accounts.app_service_account_name
  role               = "roles/iam.serviceAccountUser"
  member             = "serviceAccount:${google_project_service_identity.cloudtasks.email}"
}

# tasks.create with an oidcToken naming a service account requires the caller
# to hold iam.serviceAccounts.actAs on it — even when the caller and the named
# account are the same identity.
resource "google_service_account_iam_member" "app_acts_as_itself" {
  service_account_id = module.service_accounts.app_service_account_name
  role               = "roles/iam.serviceAccountUser"
  member             = "serviceAccount:${module.service_accounts.app_service_account_email}"
}
