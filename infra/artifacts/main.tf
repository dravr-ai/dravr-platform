# ABOUTME: Central Artifact Registry, Workload Identity, and IAM for image publishing
# ABOUTME: Shared across all environments; environment projects grant cross-project reader access

# -----------------------------------------------------------------------------
# Required APIs
# -----------------------------------------------------------------------------

resource "google_project_service" "artifactregistry" {
  project            = var.project_id
  service            = "artifactregistry.googleapis.com"
  disable_on_destroy = false
}

resource "google_project_service" "iam" {
  project            = var.project_id
  service            = "iam.googleapis.com"
  disable_on_destroy = false
}

resource "google_project_service" "iamcredentials" {
  project            = var.project_id
  service            = "iamcredentials.googleapis.com"
  disable_on_destroy = false
}

resource "google_project_service" "cloudresourcemanager" {
  project            = var.project_id
  service            = "cloudresourcemanager.googleapis.com"
  disable_on_destroy = false
}

# -----------------------------------------------------------------------------
# Artifact Registry
# -----------------------------------------------------------------------------

resource "google_artifact_registry_repository" "images" {
  location      = var.region
  project       = var.project_id
  repository_id = var.registry_name
  description   = "Central Docker repository for all Dravr platform images"
  format        = "DOCKER"

  # When true, deletions are logged only — flip to false to enforce.
  cleanup_policy_dry_run = var.cleanup_policy_dry_run

  # Keep policies win over delete policies: a version matched by both is kept.
  # Release-tagged and recent images below are therefore safe from the age rule.

  # Protect deploy / rollback anchors (e.g. semver tags) indefinitely.
  cleanup_policies {
    id     = "keep-release-tags"
    action = "KEEP"

    condition {
      tag_state    = "TAGGED"
      tag_prefixes = var.release_tag_prefixes
    }
  }

  # Rollback window: protect the newest versions per package regardless of age.
  cleanup_policies {
    id     = "keep-recent-versions"
    action = "KEEP"

    most_recent_versions {
      keep_count = var.recent_versions_keep_count
    }
  }

  # Expire stale tagged CI builds (SHA tags) once past the retention window.
  cleanup_policies {
    id     = "delete-stale-tagged"
    action = "DELETE"

    condition {
      tag_state  = "ANY"
      older_than = "${var.stale_tag_retention_days * 24 * 60 * 60}s"
    }
  }

  # Sweep orphaned untagged images left behind when a moving tag (e.g. latest) advances.
  cleanup_policies {
    id     = "delete-untagged"
    action = "DELETE"

    condition {
      tag_state  = "UNTAGGED"
      older_than = "${var.untagged_retention_days * 24 * 60 * 60}s"
    }
  }

  labels = var.labels

  depends_on = [google_project_service.artifactregistry]
}

# -----------------------------------------------------------------------------
# Image Publisher Service Account (used by GitHub Actions to push images)
# -----------------------------------------------------------------------------

resource "google_service_account" "image_publisher" {
  account_id   = "image-publisher"
  project      = var.project_id
  display_name = "Image Publisher"
  description  = "Service account used by GitHub Actions to push images to Artifact Registry"
}

resource "google_project_iam_member" "publisher_artifact_writer" {
  project = var.project_id
  role    = "roles/artifactregistry.writer"
  member  = "serviceAccount:${google_service_account.image_publisher.email}"
}

# -----------------------------------------------------------------------------
# Workload Identity Federation for GitHub Actions
# -----------------------------------------------------------------------------

resource "google_iam_workload_identity_pool" "github" {
  project                   = var.project_id
  workload_identity_pool_id = "github-pool"
  display_name              = "GitHub Actions Pool"
  description               = "Identity pool for GitHub Actions image publishing"
  disabled                  = false

  depends_on = [google_project_service.iam, google_project_service.iamcredentials]
}

resource "google_iam_workload_identity_pool_provider" "github" {
  project                            = var.project_id
  workload_identity_pool_id          = google_iam_workload_identity_pool.github.workload_identity_pool_id
  workload_identity_pool_provider_id = "github-provider"
  display_name                       = "GitHub Provider"
  description                        = "OIDC provider for GitHub Actions image publishing"

  attribute_mapping = {
    "google.subject"             = "assertion.sub"
    "attribute.actor"            = "assertion.actor"
    "attribute.repository"       = "assertion.repository"
    "attribute.repository_owner" = "assertion.repository_owner"
    "attribute.ref"              = "assertion.ref"
  }

  attribute_condition = "assertion.repository == '${var.github_org}/${var.github_repo}'"

  oidc {
    issuer_uri = "https://token.actions.githubusercontent.com"
  }
}

# Allow GitHub Actions from dravr-ai/dravr-platform to impersonate the image-publisher SA
resource "google_service_account_iam_member" "workload_identity_binding" {
  service_account_id = google_service_account.image_publisher.name
  role               = "roles/iam.workloadIdentityUser"
  member             = "principalSet://iam.googleapis.com/${google_iam_workload_identity_pool.github.name}/attribute.repository/${var.github_org}/${var.github_repo}"
}

# -----------------------------------------------------------------------------
# Cross-project reader access for environment app service accounts
# -----------------------------------------------------------------------------

# Grant each environment's Cloud Run app SA read access to pull images
resource "google_project_iam_member" "env_app_reader" {
  for_each = toset(var.env_app_sa_emails)

  project = var.project_id
  role    = "roles/artifactregistry.reader"
  member  = "serviceAccount:${each.value}"
}

# -----------------------------------------------------------------------------
# Terraform Runner Service Account
# -----------------------------------------------------------------------------
# Impersonated by GitHub Actions (terraform-artifacts.yml) to plan/apply THIS
# config. Distinct from image-publisher, which only pushes images
# (artifactregistry.writer) and therefore cannot read state or manage the
# registry policy — the reason the workflow's Terraform Init 403'd. The runner
# needs admin on every resource type this config manages, scoped to the
# single-purpose dravr-artifacts project (blast radius = this one registry
# project). Its state-bucket access is a bootstrap grant applied out-of-band
# (see below), deliberately NOT managed here.

resource "google_service_account" "terraform_runner" {
  account_id   = "terraform-runner"
  project      = var.project_id
  display_name = "Terraform Runner"
  description  = "Runs terraform plan/apply for infra/artifacts from GitHub Actions (terraform-artifacts.yml)"
}

# One predefined role per resource type this config manages. Kept minimal — no
# owner/editor. Each entry documents which resource it is required for.
resource "google_project_iam_member" "terraform_runner_roles" {
  for_each = toset([
    "roles/artifactregistry.admin",          # google_artifact_registry_repository.images
    "roles/iam.serviceAccountAdmin",         # google_service_account.* + their IAM bindings
    "roles/iam.workloadIdentityPoolAdmin",   # google_iam_workload_identity_pool[_provider].github
    "roles/resourcemanager.projectIamAdmin", # google_project_iam_member.* project bindings
    "roles/serviceusage.serviceUsageAdmin",  # google_project_service.* API enablement
  ])

  project = var.project_id
  role    = each.value
  member  = "serviceAccount:${google_service_account.terraform_runner.email}"
}

# State-bucket access is a BOOTSTRAP grant applied out-of-band by an owner, NOT a
# resource here: roles/storage.objectAdmin gives storage.objects.* (list
# workspaces + read/write state) but not storage.buckets.getIamPolicy, so a
# google_storage_bucket_iam_member owned by this config would 403 when the runner
# refreshes it — and would be a circular dep (the config owning the grant that
# unlocks its own state). One-time, run by a project owner:
#   gcloud storage buckets add-iam-policy-binding gs://dravr-artifacts-terraform-7d896 \
#     --member="serviceAccount:terraform-runner@dravr-artifacts.iam.gserviceaccount.com" \
#     --role="roles/storage.objectAdmin"

# Let GitHub Actions impersonate the runner ONLY from refs/heads/main. The runner
# holds projectIamAdmin (→ can self-grant owner), so repo-wide impersonation would
# let any feature-branch workflow holding the secrets escalate to project owner —
# a reach the Environment reviewer gate (a per-job control) does not stop. Branch
# scoping relies on attribute.ref in the provider mapping above; the provider's
# attribute_condition still pins the repo. (image-publisher stays repo-scoped: it
# is writer-only and also runs on v* tags, which are not refs/heads/main.)
resource "google_service_account_iam_member" "terraform_runner_wif" {
  service_account_id = google_service_account.terraform_runner.name
  role               = "roles/iam.workloadIdentityUser"
  member             = "principalSet://iam.googleapis.com/${google_iam_workload_identity_pool.github.name}/attribute.ref/refs/heads/main"
}
