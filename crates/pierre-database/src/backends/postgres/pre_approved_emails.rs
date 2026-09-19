// ABOUTME: PostgreSQL-backed PreApprovedEmailRepository, emitted from the shared implementation in repositories/pre_approved_emails.rs
// ABOUTME: allowed_by is a native uuid column here, so the uuid binds unwrapped and reads back through a ::text cast

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::PreApprovedEmail;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::pre_approved_emails::{
    entry_from_row, get_email_sql, impl_pre_approved_email_repository, list_emails_sql,
    pre_approved_email_columns, ALLOW_EMAIL_SQL, REMOVE_EMAIL_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::PreApprovedEmailRepository;

impl_pre_approved_email_repository!(PostgresDatabase, NativeUuid::bind, "::text");
