# ABOUTME: Variables for the centralized dravr-artifacts Terraform configuration
# ABOUTME: Populate env_app_sa_emails after applying dev/prod environment Terraform

project_id = "dravr-artifacts"

# Enforce the image cleanup policy (default is dry-run = log-only). The 4-rule
# policy keeps release-tagged + the 20 most-recent versions and deletes stale
# tagged builds (>30d) and orphaned untagged images (>3d). Reconciles the
# agent-set manual policy that only swept untagged and let tagged CI builds
# accumulate (~287GB). Deployed digests are protected by keep-recent-versions.
cleanup_policy_dry_run = false

env_app_sa_emails = [
  # Populate after applying dev/prod environments:
  "dravr-mcp-server-app@dravr-dev.iam.gserviceaccount.com",
  "service-865150413606@serverless-robot-prod.iam.gserviceaccount.com",
  "terraform-runner@dravr-dev.iam.gserviceaccount.com",
  # "pierre-mcp-server-app@dravr-prod.iam.gserviceaccount.com",
]
