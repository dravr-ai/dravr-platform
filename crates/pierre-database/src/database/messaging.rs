// ABOUTME: SQLite-backed MessagingRepository, emitted from the shared implementation in repositories/messaging.rs
// ABOUTME: Every id column is TEXT here: a user id binds and reads as the hyphenated text the caller holds
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::messaging::MessagingError;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::messaging_link_states as link_states;
use super::messaging_reactions as reactions;
use super::Database;
use crate::repositories::messaging::{
    CreateChannelLinkParams, CreateLinkStateParams, CreateSessionParams, InsertMessageParams,
    MessagingRepository, ReactionFeedbackTarget, UpsertChannelConfigParams,
    ACTIVE_OTP_LINK_STATE_SQL, AGENT_PROPOSAL_SENT_SQL, ALL_PENDING_OUTBOUND_SQL,
    CHANNEL_IDENTITY_CLAIMED_SQL, CHANNEL_LINK_LOCALE_SQL, CHANNEL_LINK_TENANT_SQL,
    CLAIM_BACKFILL_PUSH_SQL, CONFIGS_BY_CHANNEL_TYPE_SQL, CREATE_CHANNEL_LINK_SQL,
    CREATE_SESSION_SQL, DELETE_CHANNEL_CONFIG_SQL, DELETE_CHANNEL_LINK_SQL, ENQUEUE_OUTBOUND_SQL,
    GET_CHANNEL_CONFIG_SQL, GET_CHANNEL_LINK_SQL, INCREMENT_OTP_ATTEMPTS_SQL,
    INSERT_DELIVERY_RECEIPT_SQL, INSERT_MESSAGE_SQL, INVALIDATE_OTP_LINK_STATES_SQL,
    LIST_CHANNEL_CONFIGS_SQL, LIST_USER_CHANNEL_LINKS_SQL, LOGOUT_DELETE_LINK_SQL,
    LOGOUT_INVALIDATE_STATES_SQL, MARK_AGENT_PROPOSAL_SENT_SQL, PENDING_OUTBOUND_SQL,
    PROPOSED_AGENT_IDS_SQL, SESSION_BY_CHANNEL_IDENTITY_SQL, SESSION_BY_CONVERSATION_SQL,
    SESSION_MESSAGES_SQL, SET_CHANNEL_LINK_LOCALE_SQL, SET_OTP_ON_LINK_STATE_SQL,
    SET_SESSION_CONVERSATION_SQL, SET_SIGNUP_PENDING_SQL, TOUCH_SESSION_SQL,
    UPDATE_OUTBOUND_STATUS_SQL, UPSERT_CHANNEL_CONFIG_SQL,
};
use crate::repositories::messaging_backend::{
    impl_messaging_repository, instant_from_rfc3339, messaging_column_error,
};
use crate::repositories::uuid_columns::TextUuid;

impl_messaging_repository!(Database, SqliteRow, TextUuid);
