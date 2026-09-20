// ABOUTME: PostgreSQL-backed StoreListingsRepository, emitted from the shared implementation in repositories/store_listings.rs
// ABOUTME: user ids are native uuids here, so the shell passes the native uuid codec; search matches with ILIKE
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::agents::{
    Agent, AgentCategory, AgentHandle, AgentVisibility, AgentWithListing, PublishStatus,
    StoreAdminStats, StoreListing,
};
use pierre_core::models::TenantId;
use pierre_core::pagination::{Cursor, CursorPage, StoreCursor, StoreSortOrder};
use uuid::Uuid;

use super::agents_rows::row_to_agent_pg;
use super::PostgresDatabase;
use crate::repositories::store_listings::{
    agent_columns_aliased, agent_with_listing_from_row, agent_with_listing_select, category_filter,
    column, contains_pattern, ensure_catalogue_handle, listing_columns_aliased, page_limit,
    page_offset, published_agents_sql, published_page_sql, search_published_sql,
    store_admin_stats_from_row, store_listing_from_row, token_count_column,
    StoreListingsRepository, AGENT_WITH_LISTING_SQL, APPROVE_LISTING_SQL, ASSIGN_HANDLE_SQL,
    AUTHOR_EMAIL_SQL, CATEGORY_COUNTS_SQL, DECREMENT_INSTALLS_SQL, DELETE_OWNED_AGENT_SQL,
    EXISTING_INSTALL_SQL, EXISTING_LISTING_SQL, GET_LISTING_SQL, HANDLE_OWNER_SQL,
    HANDLE_TAKEN_SQL, INCREMENT_INSTALLS_SQL, INSERT_DRAFT_LISTING_SQL, INSERT_PENDING_LISTING_SQL,
    INSTALLED_AGENTS_SQL, INSTALLED_AGENT_SQL, INSTALL_AGENT_SQL, MAX_HANDLE_ATTEMPTS,
    NEWEST_AFTER, NEWEST_ORDER, OWNED_AGENT_ORIGIN_SQL, OWNED_AGENT_SQL, PENDING_REVIEW_SQL,
    POPULAR_AFTER, POPULAR_ORDER, PUBLISHED_AGENT_SQL, PUBLISHED_BY_HANDLE_SQL, REJECTED_SQL,
    REJECT_LISTING_SQL, SELF_ASSIGN_SQL, STORE_ADMIN_STATS_SQL, SUBMIT_LISTING_SQL, TITLE_AFTER,
    TITLE_ORDER, TOUCH_AGENT_SQL, UNPUBLISH_LISTING_SQL,
};
use crate::repositories::store_listings_backend::impl_store_listings_repository;
use crate::repositories::uuid_columns::NativeUuid;

impl_store_listings_repository!(PostgresDatabase, NativeUuid, row_to_agent_pg, "ILIKE");
