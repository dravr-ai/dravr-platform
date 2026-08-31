# ABOUTME: Offline plan-mode tests for the dravr-images cleanup policy logic
# ABOUTME: Uses mock_provider so it runs with no GCP credentials (pre-push + local)

mock_provider "google" {}

variables {
  project_id = "dravr-artifacts"
}

# Production enforces the cleanup policy: terraform.tfvars sets
# cleanup_policy_dry_run = false so the 4-rule policy actually prunes. Guards
# against an accidental revert to dry-run that would silently stop pruning and
# let the registry grow again. (The variable still DEFAULTS to true in
# variables.tf, so a fresh apply with no tfvars stays safe — an operator
# reviews the dry-run before flipping enforcement on.)
run "cleanup_policy_is_enforced" {
  command = plan

  assert {
    condition     = google_artifact_registry_repository.images.cleanup_policy_dry_run == false
    error_message = "terraform.tfvars must keep cleanup_policy_dry_run = false so the policy prunes"
  }
}

# The dry-run safety mode must still be reachable: operators flip it on to
# preview a policy change before enforcing.
run "dry_run_mode_is_available" {
  command = plan

  variables {
    cleanup_policy_dry_run = true
  }

  assert {
    condition     = google_artifact_registry_repository.images.cleanup_policy_dry_run == true
    error_message = "setting cleanup_policy_dry_run = true must put the policy in log-only mode"
  }
}

# Guards the hours -> seconds arithmetic against the enforced tfvars values: a
# unit slip here would silently ship a far-too-short (or too-long) retention that
# still applies cleanly. terraform.tfvars sets stale=6h, untagged=1h, which are
# the windows the live repository runs — these two assertions are what keep this
# configuration honest about production, so a plan stays a no-op.
run "stale_retention_hours_convert_to_seconds" {
  command = plan

  assert {
    condition = anytrue([
      for p in google_artifact_registry_repository.images.cleanup_policies :
      p.condition[0].older_than == "21600s"
      if p.id == "delete-superseded"
    ])
    error_message = "stale_tag_retention_hours=6 must compute to older_than=21600s (the live window)"
  }

  assert {
    condition = anytrue([
      for p in google_artifact_registry_repository.images.cleanup_policies :
      p.condition[0].older_than == "3600s"
      if p.id == "delete-untagged"
    ])
    error_message = "untagged_retention_hours=1 must compute to older_than=3600s (the live window)"
  }
}

# Non-default windows must convert correctly too, not just the defaults. Uses a
# sub-day value on purpose: day-granular arithmetic could not express the live
# policy, which is how the running config drifted out of Terraform in the first
# place, so the test pins that hours actually reach the provider.
run "custom_retention_window_converts" {
  command = plan

  variables {
    stale_tag_retention_hours = 36
  }

  assert {
    condition = anytrue([
      for p in google_artifact_registry_repository.images.cleanup_policies :
      p.condition[0].older_than == "129600s"
      if p.id == "delete-superseded"
    ])
    error_message = "stale_tag_retention_hours=36 must compute to older_than=129600s"
  }
}

# Release tags are the rollback anchors — the keep guard must carry the prefix.
run "release_tags_are_kept" {
  command = plan

  assert {
    condition = anytrue([
      for p in google_artifact_registry_repository.images.cleanup_policies :
      p.action == "KEEP" && contains(p.condition[0].tag_prefixes, "v")
      if p.id == "keep-referenced-tags"
    ])
    error_message = "keep-referenced-tags must KEEP tagged images with the configured release prefix"
  }
}

# The digest each environment is serving is tagged deployed-<env> by the deploy
# workflow (publish-images.yml, job tag-deployed-dev). That prefix must stay in
# the keep guard: dev deploys by digest and never carries a semver tag, so
# dropping it would leave the serving image protected by nothing but its position
# in the recent-versions window — which a handful of merges erases.
run "deployed_digest_tags_are_kept" {
  command = plan

  assert {
    condition = anytrue([
      for p in google_artifact_registry_repository.images.cleanup_policies :
      p.action == "KEEP" && contains(p.condition[0].tag_prefixes, "deployed-")
      if p.id == "keep-referenced-tags"
    ])
    error_message = "keep-referenced-tags must KEEP the deployed-<env> tag the deploy workflow applies to the serving digest"
  }
}

# The buildcache prefix is not cosmetic. The server image is a cargo-chef
# multi-stage build published with registry buildcache mode=max; the cache
# manifests carry a buildcache tag and are otherwise ordinary tagged images, so
# without this prefix the delete-superseded rule reaps them on the same 6-hour
# window as a stale CI build and the next run pays a full dependency recompile
# (~+10-20min). `latest` rides along for the same reason on the deploy side: it
# is the reference the Cloud Run services actually hold.
run "buildcache_and_latest_tags_are_kept" {
  command = plan

  assert {
    condition = anytrue([
      for p in google_artifact_registry_repository.images.cleanup_policies :
      p.action == "KEEP" && contains(p.condition[0].tag_prefixes, "buildcache")
      if p.id == "keep-referenced-tags"
    ])
    error_message = "keep-referenced-tags must KEEP buildcache tags — reaping them costs a full dep recompile"
  }

  assert {
    condition = anytrue([
      for p in google_artifact_registry_repository.images.cleanup_policies :
      p.action == "KEEP" && contains(p.condition[0].tag_prefixes, "latest")
      if p.id == "keep-referenced-tags"
    ])
    error_message = "keep-referenced-tags must KEEP the latest tag the Cloud Run services reference"
  }
}

# The recent-versions floor keeps the two newest versions of each package through
# a quiet week with no merges, when the age rule would otherwise reap the whole
# package. It is a build-count window, not a statement about what any environment
# is running — deployed_digest_tags_are_kept above covers that. tfvars pins it to
# 2 for cost — guard against a revert to a large window that would let the
# registry grow again.
run "recent_versions_keep_count_is_two" {
  command = plan

  assert {
    condition = anytrue([
      for p in google_artifact_registry_repository.images.cleanup_policies :
      p.most_recent_versions[0].keep_count == 2
      if p.id == "keep-recent-versions"
    ])
    error_message = "recent_versions_keep_count must be 2 (live image + one rollback floor)"
  }
}

# Input guards reject nonsense windows before they reach the provider.
run "rejects_zero_stale_retention" {
  command = plan

  variables {
    stale_tag_retention_hours = 0
  }

  expect_failures = [var.stale_tag_retention_hours]
}

run "rejects_zero_keep_count" {
  command = plan

  variables {
    recent_versions_keep_count = 0
  }

  expect_failures = [var.recent_versions_keep_count]
}
