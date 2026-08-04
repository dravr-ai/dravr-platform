# ABOUTME: Variables for the centralized dravr-artifacts Terraform configuration
# ABOUTME: Populate env_app_sa_emails after applying dev/prod environment Terraform

project_id = "dravr-artifacts"

# Enforce the image cleanup policy (default is dry-run = log-only). The digest
# each environment is serving carries a deployed-<env> tag and is protected by
# keep-release-tags; release-tagged (v*) images are kept forever as prod rollback
# anchors.
cleanup_policy_dry_run = false

# Retention tuned for cost: hold the registry to roughly the last week of builds
# instead of a month. keep_count=2 is a recency floor over the two newest
# versions of each package — it bounds how fast the age rule can reach the head
# of a package, but it says nothing about which digest is deployed, which is why
# the deployed-<env> tag carries that guarantee instead. The 7-day stale window
# is the lever that actually prunes old SHA-tagged CI builds. Buildx cache
# manifests live in their own <image>-cache packages (publish-images.yml) so
# they cannot occupy an image package's recency floor, and the orphans a moving
# buildcache tag leaves behind are swept after 1 day. Reconciles the earlier
# manual policy that only swept untagged and let tagged CI builds accumulate
# (~287GB).
recent_versions_keep_count = 2
stale_tag_retention_days   = 7
untagged_retention_days    = 1

env_app_sa_emails = [
  # Populate after applying dev/prod environments:
  "dravr-mcp-server-app@dravr-dev.iam.gserviceaccount.com",
  "service-865150413606@serverless-robot-prod.iam.gserviceaccount.com",
  "terraform-runner@dravr-dev.iam.gserviceaccount.com",
  # "pierre-mcp-server-app@dravr-prod.iam.gserviceaccount.com",
]
