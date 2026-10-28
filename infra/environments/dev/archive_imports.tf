# ABOUTME: Import your own data (carnet#1218): dravr-barrage's blob bucket and the Cloud Tasks queue its steps run on
# ABOUTME: A bucket of its own (no versioning, no soft delete) so a deleted original is gone; one step at a time
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# An athlete imports their own history from a service's export (Strava first).
# dravr-barrage, compiled into the backend, spools the export into a blob
# store, keeps each recording file as the original, and imports the rows a
# chunk per Cloud Tasks step. Its records and jobs live in the `barrage`
# schema of the platform's own Cloud SQL database, on the backend's pool.
#
# The bucket is a separate resource from the module's app bucket, which keeps
# every version: a deleted original must be gone when the athlete deletes it
# or their account, so versioning stays off and soft delete keeps nothing.
# The app service account gets object read/write/delete on this bucket alone.
#
# Deploy order is free in both directions: without BARRAGE_GCS_BUCKET the
# binary runs on Postgres with imports disabled (it says so at boot and every
# import route answers 503), and a bucket and queue applied before the code
# sit unused. The manual apply (see the deploy-infra skill) is what turns
# imports on. Before it, the barrage schema's first migration on Cloud SQL is
# unverified (carnet#1216 §3): a refused CREATE SCHEMA leaves imports disabled
# and logged, never the boot broken.

resource "google_storage_bucket" "barrage" {
  name          = "${var.project_id}-barrage"
  project       = var.project_id
  location      = var.region
  storage_class = "STANDARD"

  uniform_bucket_level_access = true
  public_access_prevention    = "enforced"

  versioning {
    enabled = false
  }

  # Soft delete would keep every deleted object for seven days by default,
  # which is exactly what a deletion the athlete asked for must not do.
  soft_delete_policy {
    retention_duration_seconds = 0
  }

  labels = merge(var.labels, { component = "barrage" })

  depends_on = [module.project]
}

# Object read, write and delete on the barrage bucket only: the spool, the
# kept originals, and their deletion.
resource "google_storage_bucket_iam_member" "barrage_object_user" {
  bucket = google_storage_bucket.barrage.name
  role   = "roles/storage.objectUser"
  member = "serviceAccount:${module.service_accounts.app_service_account_email}"

  depends_on = [module.service_accounts]
}

resource "google_cloud_tasks_queue" "archive_import" {
  name     = "${var.service_name}-archive-import"
  location = var.region
  project  = var.project_id

  # A failed step (a store or projection error) is retried with backoff;
  # every write before its commit is idempotent, so a retry converges.
  retry_config {
    max_attempts       = 20
    max_retry_duration = "21600s"
    min_backoff        = "10s"
    max_backoff        = "600s"
    max_doublings      = 5
  }

  # One step at a time across every athlete: a step decodes one recording
  # at a time on the API instance that runs it, and this keeps an import's
  # load on the API to one such step.
  rate_limits {
    max_concurrent_dispatches = 1
    max_dispatches_per_second = 1
  }

  depends_on = [module.project, module.service_accounts]
}

# The app service account creates import steps on the queue. The OIDC
# bindings each task's token needs (the Cloud Tasks agent and the app account
# acting as the app account) are the turn queue's, in turn_queue.tf.
resource "google_cloud_tasks_queue_iam_member" "archive_import_enqueuer" {
  project  = var.project_id
  location = google_cloud_tasks_queue.archive_import.location
  name     = google_cloud_tasks_queue.archive_import.name
  role     = "roles/cloudtasks.enqueuer"
  member   = "serviceAccount:${module.service_accounts.app_service_account_email}"
}
