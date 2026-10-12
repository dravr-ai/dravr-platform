# ABOUTME: Variables for the centralized dravr-artifacts Terraform configuration
# ABOUTME: Populate env_app_sa_emails after applying dev/prod environment Terraform

project_id = "dravr-artifacts"

# Enforce the image cleanup policy (default is dry-run = log-only). The digest
# each environment is serving carries a deployed-<env> tag, the digest it
# replaced carries deployed-<env>-previous, and keep-referenced-tags protects
# both; release-tagged (v*) images are kept forever as prod rollback anchors.
cleanup_policy_dry_run = false

# Retention tuned for cost (carnet#166). keep_count=2 is a recency floor over the
# two newest VERSIONS of each package, and a version is not a build: every push
# writes three (the OCI index carrying the tags, its linux/amd64 manifest and an
# attestation). The floor therefore holds less than one complete build. It keeps
# a quiet package from being emptied by the age rule; it is not a rollback
# window, and it says nothing about which digest is deployed, which is why the
# deployed-<env> tag carries the guarantee instead. Every dev deploy tail moves it
# through scripts/ci/tag-deployed-digest.sh (publish-images.yml's
# tag-deployed-dev; deploy-satellite-cloud-run.yml, the ship tail of the
# satellite-bump.yml lanes), which first tags the digest it replaces
# deployed-<env>-previous (carnet#137). The rollback window is therefore one
# deploy deep however long ago that deploy was: the serving digest and the one
# before it are both kept, at the cost of one more retained build per package
# and with keep_count left where carnet#166 put it. Two deploys back there is no
# anchor: the digest deployed-<env>-previous leaves keeps only its SHA tag, so
# delete-superseded reaps it on the first cleanup pass after it is
# stale_tag_retention_hours old (the very next pass, for a build that served
# longer than that), unless it also carries a v<version> release tag, as a
# satellite image built from its release ref does. The stale window is the lever
# that actually prunes superseded SHA-tagged CI builds.
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
# future retention change is a reviewable diff instead of a console edit; the
# drift leg for this root in terraform-drift.yml re-plans it nightly, so a
# console edit reds that monitor instead of running unseen.
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
