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

  # Protect deploy / rollback anchors indefinitely: semver release tags, and the
  # moving deployed-<env> tag that publish-images.yml applies to each digest it
  # ships to Cloud Run. Dev deploys by digest and never carries a semver tag, so
  # that deploy tag is what makes the serving image immune to the age rule —
  # recency alone does not, since the newest-versions floor is a count of builds,
  # not a statement about what any environment is running.
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

# Moving the `deployed-dev` tag onto each newly shipped digest is a delete of
# the tag from the previous digest followed by a create, and roles/…writer
# grants only the create half — so the deploy tagged the first digest and then
# failed on every deploy after it, leaving the serving image unprotected from
# the retention policy the tag exists to satisfy.
#
# A custom role rather than roles/artifactregistry.repoAdmin: repoAdmin also
# carries packages.delete and versions.delete, which would let a compromised
# CI token erase published images outright. Tag deletion is the whole of the
# additional authority the publisher needs.
resource "google_project_iam_custom_role" "artifact_tag_mover" {
  project     = var.project_id
  role_id     = "artifactTagMover"
  title       = "Artifact Registry Tag Mover"
  description = "Retag an existing image, which requires removing the tag from the digest that held it"
  permissions = ["artifactregistry.tags.delete"]
}

resource "google_project_iam_member" "publisher_tag_mover" {
  project = var.project_id
  role    = google_project_iam_custom_role.artifact_tag_mover.id
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
# Impersonated by GitHub Actions (terraform-artifacts.yml) to apply THIS config.
# The write half of terraform only: the read-only terraform_planner below serves
# plan, so this owner-equivalent credential is minted by the write verb alone.
# Distinct from image-publisher, which only pushes images (artifactregistry.writer)
# and therefore cannot read state or manage the registry policy — the reason the
# workflow's Terraform Init 403'd. The runner needs admin on every resource type
# this config manages, scoped to the single-purpose dravr-artifacts project
# (blast radius = this one registry project). Its state-bucket access is a
# bootstrap grant applied out-of-band (see below), deliberately NOT managed here.

resource "google_service_account" "terraform_runner" {
  account_id   = "terraform-runner"
  project      = var.project_id
  display_name = "Terraform Runner"
  description  = "Runs terraform apply for infra/artifacts from GitHub Actions (terraform-artifacts.yml)"
}

# One predefined admin role per resource type this config manages. Kept minimal —
# no owner/editor. Each entry documents which resource it is required for.
resource "google_project_iam_member" "terraform_runner_roles" {
  for_each = toset([
    "roles/artifactregistry.admin",          # google_artifact_registry_repository.images
    "roles/iam.serviceAccountAdmin",         # google_service_account.* + their IAM bindings
    "roles/iam.workloadIdentityPoolAdmin",   # google_iam_workload_identity_pool[_provider].github
    "roles/iam.roleAdmin",                   # google_project_iam_custom_role.*
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

# -----------------------------------------------------------------------------
# Terraform Planner Service Account (read-only)
# -----------------------------------------------------------------------------
# The read half of terraform as its own identity. `terraform plan` refreshes and
# diffs and writes nothing, but running it as the runner above mints a credential
# that can self-grant roles/owner — so an ungated plan job would hand that
# credential out on demand, and a gated one puts a human approval in front of a
# read-only verb. This SA holds viewer roles on dravr-artifacts and objectViewer
# on the state bucket, with no setIamPolicy anywhere: there is nothing to escalate
# to, so terraform-artifacts.yml runs its plan job with no Environment gate and
# reserves that gate for apply.

resource "google_service_account" "terraform_planner" {
  account_id   = "terraform-planner"
  project      = var.project_id
  display_name = "Terraform Planner"
  description  = "Runs read-only terraform plan for infra/artifacts from GitHub Actions (terraform-artifacts.yml)"
}

# The read-only mirror of terraform_runner_roles: one predefined viewer role per
# resource type a plan refresh has to fetch. Each entry names the resource that
# requires it.
resource "google_project_iam_member" "terraform_planner_roles" {
  for_each = toset([
    "roles/artifactregistry.reader",         # google_artifact_registry_repository.images
    "roles/iam.roleViewer",                  # google_project_iam_custom_role.* definitions
    "roles/iam.serviceAccountViewer",        # google_service_account.* (also carries resourcemanager.projects.get)
    "roles/iam.workloadIdentityPoolViewer",  # google_iam_workload_identity_pool[_provider].github
    "roles/serviceusage.serviceUsageViewer", # google_project_service.* API enablement
  ])

  project = var.project_id
  role    = each.value
  member  = "serviceAccount:${google_service_account.terraform_planner.email}"
}

# The viewer roles above read each resource's own attributes, but none of them
# reads an IAM *policy* — and two of this config's resource types are policy
# bindings: google_project_iam_member refreshes through
# resourcemanager.projects.getIamPolicy, google_service_account_iam_member
# through iam.serviceAccounts.getIamPolicy. Without both, a plan 403s during
# refresh instead of producing a diff.
#
# No narrow predefined role carries those two permissions; the predefined answers
# are roles/viewer and roles/iam.securityReviewer, both project-wide read.
# roles/viewer also grants storage.objects.get/list, which would hand the planner
# read of the terraform state bucket implicitly — the opposite of the explicit,
# separately-revocable bucket grant documented below — and read of every future
# bucket and image in the project. roles/iam.securityReviewer grants getIamPolicy
# on every resource type this project ever gains. A custom role holding exactly
# the two permissions is the narrower option, on the same reasoning that made
# artifactTagMover a custom role rather than repoAdmin.
resource "google_project_iam_custom_role" "terraform_plan_iam_reader" {
  project     = var.project_id
  role_id     = "terraformPlanIamReader"
  title       = "Terraform Plan IAM Reader"
  description = "Read the project and service-account IAM policies a terraform plan refresh diffs against"
  permissions = [
    "iam.serviceAccounts.getIamPolicy",
    "resourcemanager.projects.getIamPolicy",
  ]
}

resource "google_project_iam_member" "terraform_planner_iam_reader" {
  project = var.project_id
  role    = google_project_iam_custom_role.terraform_plan_iam_reader.id
  member  = "serviceAccount:${google_service_account.terraform_planner.email}"
}

# State-bucket access is a BOOTSTRAP grant applied out-of-band by an owner for the
# same two reasons as the runner's: it is circular (the config owning the grant
# that unlocks its own state), and roles/storage.objectViewer carries
# storage.objects.get/list but not storage.buckets.getIamPolicy, so a
# google_storage_bucket_iam_member here would 403 refreshing itself. objectViewer
# rather than the runner's objectAdmin — the planner reads state and must never
# write it. One-time, run by a project owner:
#   gcloud storage buckets add-iam-policy-binding gs://dravr-artifacts-terraform-7d896 \
#     --member="serviceAccount:terraform-planner@dravr-artifacts.iam.gserviceaccount.com" \
#     --role="roles/storage.objectViewer"
#
# Read-only state has one consequence the workflow must honour: the GCS backend
# takes its lock by creating a .tflock object, which objectViewer cannot do, so
# `terraform plan` would 403 acquiring a lock before it ever refreshed. The plan
# job therefore runs with -lock=false (terraform-artifacts.yml) — sound because a
# plan writes no state, so there is nothing for the lock to serialize. Apply keeps
# its lock: the runner holds objectAdmin.

# Impersonation is repo-scoped rather than pinned to refs/heads/main like the
# runner's. The runner is pinned because projectIamAdmin lets it self-grant owner,
# so a feature branch holding the secrets could escalate; the planner holds only
# read roles plus objectViewer on state and has no such reach to contain. Planning
# a branch before it merges is the whole point of a read-only planner — main-only
# scoping would leave the plan verb usable only after the change had already
# landed. The provider's attribute_condition still pins the pool to this repo, so
# the widening is across branches of dravr-ai/dravr-platform, not across repos.
resource "google_service_account_iam_member" "terraform_planner_wif" {
  service_account_id = google_service_account.terraform_planner.name
  role               = "roles/iam.workloadIdentityUser"
  member             = "principalSet://iam.googleapis.com/${google_iam_workload_identity_pool.github.name}/attribute.repository/${var.github_org}/${var.github_repo}"
}
