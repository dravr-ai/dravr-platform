// ABOUTME: SQLite-backed ProviderDataRepository, emitted from the shared implementation in repositories/provider_data.rs
// ABOUTME: Every id column the purge compares is TEXT here, so the shared statements bind hyphenated ids

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::provider_data::{
    impl_provider_data_repository, ProviderDataPurge, ProviderDataRepository, PROVIDER_PURGE_SQL,
    USER_PROVIDER_PURGE_SQL,
};

impl_provider_data_repository!(Database);
