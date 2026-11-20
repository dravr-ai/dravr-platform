// ABOUTME: One integration-test binary for pierre-database: every suite is a module here
// ABOUTME: Suites that mutate the process environment stay separate binaries in tests/
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Integration tests for the database crate, linked as one binary.
//!
//! A top-level file in `tests/` is a test binary of its own, linking its own
//! copy of the crate. Only a suite that writes the process environment stays
//! one, because the environment is shared by every test in a binary.

mod account_reset_test;
mod activity_cache_place_test;
mod activity_cache_samples_test;
mod admin_audit_test;
mod admin_operator_tokens_test;
mod agents_repository_test;
mod athlete_declared_home_test;
mod athlete_gear_test;
mod athlete_places_due_test;
mod check_in_preferences_test;
mod claim_verdict_grading_pool_test;
mod coaching_groups_repository_test;
mod data_access_requests_test;
mod date_decode_golden_sqlite_test;
mod date_round_trip_golden_test;
mod date_stamp_golden_sqlite_test;
mod date_storage_golden_sqlite_test;
mod date_stored_column_golden_sqlite_test;
mod debug_redaction_test;
mod dek_versioning_test;
mod federated_link_test;
mod first_run_progress_test;
mod fitness_config_repository_test;
mod guarded_status_write_test;
mod harness_memory_repository_test;
mod home_preferences_test;
mod impersonation_test;
mod last_active_admin_guard_test;
mod member_tenants_test;
mod oauth2_server_repository_test;
mod platform_post_claims_test;
mod pool_fault_test;
mod provider_connections_repository_test;
mod repository_registry_identity_test;
mod request_log_deletion_overlap_test;
mod seeder_repository_test;
mod smoke_test;
mod system_settings_test;
mod tenant_create_atomic_test;
mod test_db_factory_test;
mod test_db_survives_cancelled_query_test;
mod tool_selection_repository_test;
mod unit_preferences_test;
mod usage_repositories_test;
mod user_export_bundles_test;
mod user_export_test;
mod user_listing_tenant_scope_test;
mod user_mcp_tokens_repository_test;
mod user_oauth_tokens_repository_test;
mod user_preferences_test;
mod user_tier_overrides_test;
mod users_is_admin_follows_role_test;
mod weather_cache_test;
mod wellness_resume_day_test;
