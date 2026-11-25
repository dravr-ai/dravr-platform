// ABOUTME: PostgreSQL-backed OAuthStandingPolicyRepository, emitted from the shared body in repositories/standing_policies.rs
// ABOUTME: An athlete's "always allow this app this preset" policies, read where the consent screen would be skipped
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::OAuthStandingPolicy;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::standing_policies::{
    impl_standing_policy_repository, standing_policy_from_row, OAuthStandingPolicyRepository,
    INSERT_STANDING_POLICY_SQL, LIST_CLIENT_STANDING_POLICIES_SQL, LIST_STANDING_POLICIES_SQL,
    REVOKE_LIVE_STANDING_POLICY_SQL, REVOKE_STANDING_POLICY_SQL,
};

impl_standing_policy_repository!(PostgresDatabase);
