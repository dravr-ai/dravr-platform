// ABOUTME: PostgreSQL-backed EngagementRepository, emitted from the shared body in repositories/engagement.rs
// ABOUTME: The admin Engagement screen's reads, each in one admin scope
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use pierre_core::admin::scope::AdminScope;
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::agents::AGENT_USE_TOTALS_SQL;
use crate::repositories::engagement::{
    impl_engagement_repository, EngagementRepository, PUBLISHED_LISTINGS_SQL, USERS_LAST_ACTIVE_SQL,
};

impl_engagement_repository!(PostgresDatabase);
