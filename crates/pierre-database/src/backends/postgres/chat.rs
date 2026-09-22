// ABOUTME: PostgreSQL-backed ChatRepository, emitted from the shared implementation in repositories/chat.rs
// ABOUTME: user_id, group_id and added_by are uuid columns and the timestamps TIMESTAMPTZ: the codec parses and renders
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    AddMessageParams, ConversationLastMessage, ConversationPage, ConversationParticipant,
    ConversationRecord, ConversationSummary, MessageFeedbackRecord, MessageRecord, ParticipantRole,
    TenantId, UpsertMessageFeedbackParams, CHANNEL_TYPE_WEB,
};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::chat::{
    chat_column_error, instant_from_rfc3339, stamp_column, ChatRepository, ADD_MESSAGE_SQL,
    ADD_PARTICIPANT_SQL, ADVANCE_READ_MARKER_SQL, BUMP_CONVERSATION_TOKENS_SQL,
    CAS_ONBOARDING_STATE_SQL, CLEAR_READ_MARKER_SQL, CONTENT_HEAD_CHARS, CONVERSATION_FEEDBACK_SQL,
    COUNT_ACTIVE_SINCE_SQL, COUNT_CONVERSATIONS_SQL, COUNT_PARTICIPATING_SQL,
    CREATE_CONVERSATION_SQL, CREATE_OWNER_PARTICIPANT_SQL, DELETE_CONVERSATION_SQL,
    DELETE_FEEDBACK_SQL, DELETE_USER_CONVERSATIONS_SQL, GET_CONVERSATION_SQL, GET_FEEDBACK_SQL,
    GET_MESSAGES_SQL, GET_PARTICIPANT_SQL, GET_RECENT_MESSAGES_SQL, HAS_AGENT_INTRODUCTION_SQL,
    LIST_CONVERSATIONS_SQL, LIST_ONBOARDING_STATES_SQL, LIST_PARTICIPANTS_SQL, MESSAGE_COUNT_SQL,
    READ_TARGET_SQL, RECENT_CONVERSATIONS_ADMIN_SQL, RECORD_AGENT_INTRODUCTION_SQL,
    REMOVE_PARTICIPANT_SQL, SET_AGENT_ID_SQL, SET_CHANNEL_SQL, SET_GROUP_ID_SQL,
    SET_ONBOARDING_STATE_SQL, SET_SESSION_ID_SQL, TOUCH_CONVERSATION_SQL, UPDATE_TITLE_SQL,
    UPSERT_FEEDBACK_SQL,
};
use crate::repositories::chat_backend::impl_chat_repository;
use crate::repositories::uuid_columns::NativeUuid;

impl_chat_repository!(PostgresDatabase, PgRow, NativeUuid);
