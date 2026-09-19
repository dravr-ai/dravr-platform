// ABOUTME: SQLite-backed SubscriptionsRepository, emitted from the shared implementation in repositories/subscriptions.rs
// ABOUTME: Ids bind as hyphenated text, timestamps as RFC3339 text and metadata as TEXT, through the same binds Postgres uses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Subscription, SubscriptionStatus, TenantId, UserId};
use uuid::Uuid;

use super::Database;
use crate::repositories::subscriptions::{
    impl_subscriptions_repository, subscription_from_row, SubscriptionsRepository,
    COUNT_BILLING_EVENT_SQL, GET_SUBSCRIPTION_BY_PROVIDER_CUSTOMER_ID_SQL,
    GET_SUBSCRIPTION_BY_PROVIDER_SUBSCRIPTION_ID_SQL, GET_SUBSCRIPTION_BY_TENANT_SQL,
    GET_SUBSCRIPTION_BY_USER_SQL, LIST_SUBSCRIPTIONS_BY_STATUS_SQL, MARK_BILLING_EVENT_SQL,
    UPSERT_SUBSCRIPTION_SQL,
};
use crate::uuid_column::UuidColumn;

impl_subscriptions_repository!(Database);
