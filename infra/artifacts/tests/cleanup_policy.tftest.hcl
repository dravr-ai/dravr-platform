# ABOUTME: Offline plan-mode tests for the dravr-images cleanup policy logic
# ABOUTME: Uses mock_provider so it runs with no GCP credentials (pre-push + local)

mock_provider "google" {}

variables {
  project_id = "dravr-artifacts"
}

# The first apply must never delete anything until an operator reviews the
# dry-run results and explicitly flips the flag.
run "dry_run_defaults_true" {
  command = plan

  assert {
    condition     = google_artifact_registry_repository.images.cleanup_policy_dry_run == true
    error_message = "cleanup_policy_dry_run must default to true so the first apply deletes nothing"
  }
}

# Guards the days -> seconds arithmetic: a unit slip here would silently ship a
# far-too-short (or too-long) retention that still applies cleanly.
run "stale_retention_days_convert_to_seconds" {
  command = plan

  assert {
    condition = anytrue([
      for p in google_artifact_registry_repository.images.cleanup_policies :
      p.condition[0].older_than == "2592000s"
      if p.id == "delete-stale-tagged"
    ])
    error_message = "stale_tag_retention_days=30 must compute to older_than=2592000s"
  }

  assert {
    condition = anytrue([
      for p in google_artifact_registry_repository.images.cleanup_policies :
      p.condition[0].older_than == "259200s"
      if p.id == "delete-untagged"
    ])
    error_message = "untagged_retention_days=3 must compute to older_than=259200s"
  }
}

# Non-default windows must convert correctly too, not just the defaults.
run "custom_retention_window_converts" {
  command = plan

  variables {
    stale_tag_retention_days = 14
  }

  assert {
    condition = anytrue([
      for p in google_artifact_registry_repository.images.cleanup_policies :
      p.condition[0].older_than == "1209600s"
      if p.id == "delete-stale-tagged"
    ])
    error_message = "stale_tag_retention_days=14 must compute to older_than=1209600s"
  }
}

# Release tags are the rollback anchors — the keep guard must carry the prefix.
run "release_tags_are_kept" {
  command = plan

  assert {
    condition = anytrue([
      for p in google_artifact_registry_repository.images.cleanup_policies :
      p.action == "KEEP" && contains(p.condition[0].tag_prefixes, "v")
      if p.id == "keep-release-tags"
    ])
    error_message = "keep-release-tags must KEEP tagged images with the configured release prefix"
  }
}

# Input guards reject nonsense windows before they reach the provider.
run "rejects_zero_stale_retention" {
  command = plan

  variables {
    stale_tag_retention_days = 0
  }

  expect_failures = [var.stale_tag_retention_days]
}

run "rejects_zero_keep_count" {
  command = plan

  variables {
    recent_versions_keep_count = 0
  }

  expect_failures = [var.recent_versions_keep_count]
}
