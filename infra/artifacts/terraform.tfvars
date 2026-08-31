# ABOUTME: Variables for the centralized dravr-artifacts Terraform configuration
# ABOUTME: Populate env_app_sa_emails after applying dev/prod environment Terraform

project_id = "dravr-artifacts"

# Enforce the image cleanup policy (default is dry-run = log-only). The digest
# each environment is serving carries a deployed-<env> tag and is protected by
# keep-release-tags; release-tagged (v*) images are kept forever as prod rollback
# anchors.
cleanup_policy_dry_run = false

# Retention tuned for cost. keep_count=2 is a recency floor over the two newest
# versions of each package — it bounds how fast the age rule can reach the head
# of a package, but it says nothing about which digest is deployed, which is why
# the deployed-<env> tag carries that guarantee instead (applied by
# publish-images.yml, enforme-bump.yml and photograveur-bump.yml). The stale
# window is the lever that actually prunes superseded SHA-tagged CI builds.
#
# These values MIRROR THE LIVE REPOSITORY as verified 2026-08-31, rather than
# stating an intent the running policy does not have. The history matters:
# 450617a13 (2026-07-17) codified 7d/1d, but the live repo has never run that —
# `git log -S` finds no history anywhere in infra/ for the running policy ids
# (`delete-superseded`, `keep-referenced-tags`) or for `21600`, and the old
# day-granular arithmetic could not have produced 6h/1h at all. The live policy
# was authored outside Terraform, so applying this configuration used to be a
# destructive act: it would have relaxed retention 28x and dropped the
# latest/buildcache keep-prefixes. Codified here so a plan is a no-op and any
# future retention change is a reviewable diff instead of a console edit.
recent_versions_keep_count = 2
stale_tag_retention_hours  = 6
untagged_retention_hours   = 1

env_app_sa_emails = [
  # Populate after applying dev/prod environments:
  "dravr-mcp-server-app@dravr-dev.iam.gserviceaccount.com",
  "service-865150413606@serverless-robot-prod.iam.gserviceaccount.com",
  "terraform-runner@dravr-dev.iam.gserviceaccount.com",
  # "pierre-mcp-server-app@dravr-prod.iam.gserviceaccount.com",
]
