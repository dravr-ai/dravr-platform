// ABOUTME: PostgreSQL-backed HostProfileClaimRepository, emitted from the shared body in repositories/host_profile_claims.rs
// ABOUTME: The per-request Host Profile claim a step-up of a client with no metadata document stands on
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::HostProfileClaim;
use sqlx::Row;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::host_profile_claims::{
    impl_host_profile_claim_repository, HostProfileClaimRepository, GET_HOST_PROFILE_CLAIM_SQL,
    RECORD_HOST_PROFILE_CLAIM_SQL,
};

impl_host_profile_claim_repository!(PostgresDatabase);
