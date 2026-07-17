# ABOUTME: Variables for the centralized dravr-artifacts Terraform configuration
# ABOUTME: Populate env_app_sa_emails after applying dev/prod environment Terraform

project_id = "dravr-artifacts"

# Enforce the image cleanup policy (default is dry-run = log-only). Deployed
# digests are protected by keep-recent-versions; release-tagged (v*) images are
# kept forever as prod rollback anchors.
cleanup_policy_dry_run = false

# Retention tuned for cost: hold the registry to roughly the last week of builds
# instead of a month. keep_count=2 is a hard floor — the live image plus one
# rollback are never deleted, even during a quiet week with no merges. The 7-day
# stale window is the lever that actually prunes old SHA-tagged CI builds, and
# orphaned untagged (buildcache layer) blobs are swept after 1 day. Reconciles
# the earlier manual policy that only swept untagged and let tagged CI builds
# accumulate (~287GB).
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
