// ABOUTME: PostgreSQL-backed FederatedIdentityRepository, emitted from the shared implementation in repositories/federated_identities.rs
// ABOUTME: user_id is a native uuid column here, so the uuid binds unwrapped and reads back through a ::text cast

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::federated_identities::{
    impl_federated_identity_repository, user_for_subject_sql, LINK_SUBJECT_SQL,
    SUBJECT_FOR_USER_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::FederatedIdentityRepository;

impl_federated_identity_repository!(PostgresDatabase, NativeUuid::bind, "::text");
