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

  cleanup_policies {
    id     = "keep-recent-versions"
    action = "KEEP"

    most_recent_versions {
      keep_count = 10
    }
  }

  cleanup_policies {
    id     = "delete-old-untagged"
    action = "DELETE"

    condition {
      tag_state  = "UNTAGGED"
      older_than = "604800s" # 7 days
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
  }

  attribute_condition = "assertion.repository_owner == '${var.github_org}'"

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
