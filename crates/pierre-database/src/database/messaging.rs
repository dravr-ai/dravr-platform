// ABOUTME: SQLite-backed MessagingRepository, emitted from the shared implementation in repositories/messaging_backend.rs
// ABOUTME: Every id column is TEXT here: a user id binds and reads as the hyphenated text the caller holds
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::messaging::MessagingError;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::messaging::is_routable_sender_id;
use pierre_core::models::TenantId;
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::messaging_link_states as link_states;
use super::messaging_reactions as reactions;
use super::Database;
use crate::repositories::messaging::sql::{
    ACTIVE_OTP_LINK_STATE_SQL, AGENT_PROPOSAL_SENT_SQL, CHANNEL_IDENTITY_CLAIMED_SQL,
    CHANNEL_LINK_LOCALE_SQL, CHANNEL_LINK_TENANT_SQL, CLAIM_BACKFILL_PUSH_SQL,
    CLAIM_DUE_OUTBOUND_SQL, CLEAR_PROPOSED_AGENT_IDS_SQL, CONFIGS_BY_CHANNEL_TYPE_SQL,
    CREATE_CHANNEL_LINK_SQL, CREATE_SESSION_SQL, DELETE_CHANNEL_CONFIG_SQL,
    DELETE_CHANNEL_LINK_SQL, ENQUEUE_OUTBOUND_SQL, GET_CHANNEL_CONFIG_SQL, GET_CHANNEL_LINK_SQL,
    INCREMENT_OTP_ATTEMPTS_SQL, INSERT_MESSAGE_SQL, INVALIDATE_OTP_LINK_STATES_SQL,
    LIST_CHANNEL_CONFIGS_SQL, LIST_CHANNEL_LINKS_FOR_USER_SQL, LIST_USER_CHANNEL_LINKS_SQL,
    LOGOUT_DELETE_LINK_SQL, LOGOUT_INVALIDATE_STATES_SQL, MARK_AGENT_PROPOSAL_SENT_SQL,
    MARK_CHANNEL_CONFIG_PLATFORM_SCOPE_SQL, PROPOSED_AGENT_IDS_SQL, REKEY_CHANNEL_LINK_SQL,
    REKEY_CHANNEL_SESSIONS_SQL, REKEY_RESUMABLE_TURNS_SQL, RESOLVE_CHANNEL_CONFIG_SQL,
    SESSION_BY_CHANNEL_IDENTITY_SQL, SESSION_BY_CONVERSATION_SQL, SESSION_MESSAGES_SQL,
    SET_CHANNEL_LINK_LOCALE_SQL, SET_OTP_ON_LINK_STATE_SQL, SET_SESSION_CONVERSATION_SQL,
    SET_SIGNUP_PENDING_SQL, TOUCH_SESSION_SQL, UPDATE_OUTBOUND_STATUS_SQL,
    UPSERT_CHANNEL_CONFIG_SQL, USER_CHANNEL_LINK_LOCALE_SQL,
};
use crate::repositories::messaging::{
    CreateChannelLinkParams, CreateLinkStateParams, CreateSessionParams, EnqueueOutboundParams,
    InsertMessageParams, MessagingRepository, ReactionFeedbackTarget, RekeyChannelIdentityParams,
    UpsertChannelConfigParams,
};
use crate::repositories::messaging_backend::{
    impl_messaging_repository, instant_from_rfc3339, messaging_column_error,
};
use crate::repositories::messaging_link_states::{
    HOLD_LINK_FOR_CONFIRMATION_SQL, RELEASE_SENDER_LINK_CONFIRMATIONS_SQL, SPEND_HELD_LINK_CODE_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_messaging_repository!(Database, SqliteRow, TextUuid);
