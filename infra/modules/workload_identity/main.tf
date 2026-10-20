# ABOUTME: Creates Workload Identity Federation for GitHub Actions
# ABOUTME: Allows GitHub to authenticate to GCP without service account keys

# -----------------------------------------------------------------------------
# Workload Identity Pool
# -----------------------------------------------------------------------------

resource "google_iam_workload_identity_pool" "github" {
  project                   = var.project_id
  workload_identity_pool_id = "github-pool"
  display_name              = "GitHub Actions Pool"
  description               = "Identity pool for GitHub Actions CI/CD"
  disabled                  = false
}

# -----------------------------------------------------------------------------
# Workload Identity Provider (OIDC)
# -----------------------------------------------------------------------------

resource "google_iam_workload_identity_pool_provider" "github" {
  project                            = var.project_id
  workload_identity_pool_id          = google_iam_workload_identity_pool.github.workload_identity_pool_id
  workload_identity_pool_provider_id = "github-provider"
  display_name                       = "GitHub Provider"
  description                        = "OIDC provider for GitHub Actions"

  attribute_mapping = {
    "google.subject"             = "assertion.sub"
    "attribute.actor"            = "assertion.actor"
    "attribute.repository"       = "assertion.repository"
    "attribute.repository_owner" = "assertion.repository_owner"
    # Repository and ref in one value, so the runner binding below names one
    # branch of one repository: a ref alone would also admit the main branch of
    # every sister repo in additional_repositories.
    "attribute.repository_ref" = "assertion.repository + '/' + assertion.ref"
  }

  # Allow the primary platform repo plus any extra sister repos under the
  # same org (e.g. dravr-contremaitre's prompt-mirror Action).
  attribute_condition = format(
    "assertion.repository_owner == '%s' && (%s)",
    var.github_org,
    join(" || ", [
      for r in concat([var.github_repo], var.additional_repositories) :
      format("assertion.repository == '%s/%s'", var.github_org, r)
    ])
  )

  oidc {
    issuer_uri = "https://token.actions.githubusercontent.com"
  }
}

# -----------------------------------------------------------------------------
# Allow GitHub to impersonate the deployer service account
# -----------------------------------------------------------------------------

resource "google_service_account_iam_member" "workload_identity_binding" {
  service_account_id = var.deployer_service_account_name
  role               = "roles/iam.workloadIdentityUser"
  member             = "principalSet://iam.googleapis.com/${google_iam_workload_identity_pool.github.name}/attribute.repository/${var.github_org}/${var.github_repo}"
}

# Let GitHub Actions impersonate the terraform runner ONLY from main of this
# repo. The runner holds projectIamAdmin (it can grant itself owner), so a
# repo-wide binding would let any branch's workflow mint owner-equivalent
# credentials: a branch can edit the workflow and drop the Environment reviewer
# gate, which is a per-job control. Same scoping as infra/artifacts'
# terraform_runner_wif.
# A plan or drift check dispatched from another branch is refused; both run
# from main on push and on schedule.
resource "google_service_account_iam_member" "terraform_runner_workload_identity_binding" {
  service_account_id = var.terraform_runner_service_account_name
  role               = "roles/iam.workloadIdentityUser"
  member             = "principalSet://iam.googleapis.com/${google_iam_workload_identity_pool.github.name}/attribute.repository_ref/${var.github_org}/${var.github_repo}/refs/heads/main"
}
