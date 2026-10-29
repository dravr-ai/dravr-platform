// ABOUTME: PostgreSQL-backed ChatRepository, emitted from the shared implementation in repositories/chat.rs
// ABOUTME: user_id, group_id and added_by are uuid columns and the timestamps TIMESTAMPTZ: the codec parses and renders
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::roster_display_name;
use pierre_core::models::{
    AddMessageParams, ConversationLastMessage, ConversationPage, ConversationParticipant,
    ConversationRecord, ConversationSummary, MessageFeedbackRecord, MessagePage, MessageRecord,
    ParticipantRole, TenantId, ThreadTopic, UpsertMessageFeedbackParams, CHANNEL_TYPE_WEB,
    COMMAND_FINISH_REASON, DIVIDER_FINISH_REASON,
};
use pierre_core::transport::TransportPolicy;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::chat::{
    chat_column_error, instant_from_rfc3339, owner_slot_lock_sql, stamp_column, AssistantMessage,
    ChatRepository, NewConversation, Reactivation, SlotClaim, ADD_MESSAGE_SQL,
    ADVANCE_READ_MARKER_SQL, ARCHIVED_SLOT_SQL, ARCHIVE_CONVERSATION_SQL,
    ASSISTANT_MESSAGES_AFTER_SQL, BUMP_CONVERSATION_TOKENS_SQL, CAS_ONBOARDING_STATE_SQL,
    CLEAR_READ_MARKER_SQL, CONTENT_HEAD_CHARS, COUNT_ACTIVE_SINCE_SQL, COUNT_CONVERSATIONS_SQL,
    COUNT_PARTICIPATING_SQL, CREATE_CONVERSATION_SQL, CREATE_OWNER_PARTICIPANT_SQL,
    DELETE_CONVERSATION_SQL, DESIGNATE_PERSONAL_SQL, FIND_GROUP_CONVERSATION_SQL,
    GET_CONVERSATION_SQL, GET_MESSAGES_SQL, GET_RECENT_MESSAGES_SQL, HAS_AGENT_INTRODUCTION_SQL,
    IS_ARCHIVED_SQL, LATEST_PERSONAL_CONVERSATION_SQL, LIST_CONVERSATIONS_SQL,
    LIST_ONBOARDING_STATES_SQL, PERSONAL_CONVERSATION_SQL, REACTIVATE_CONVERSATION_SQL,
    READ_TARGET_SQL, RECENT_CONVERSATIONS_ADMIN_SQL, RECORD_AGENT_INTRODUCTION_SQL,
    SET_AGENT_ID_SQL, SET_CHANNEL_SQL, SET_GROUP_ID_SQL, SET_ONBOARDING_STATE_SQL,
    SET_SESSION_ID_SQL, TOUCH_CONVERSATION_SQL, UPDATE_TITLE_SQL,
};
use crate::repositories::chat_backend::impl_chat_repository;
use crate::repositories::chat_feedback::{
    CONVERSATION_FEEDBACK_SQL, DELETE_FEEDBACK_SQL, GET_FEEDBACK_SQL, UPSERT_FEEDBACK_SQL,
};
use crate::repositories::chat_member_rows::{recent_prompt_messages_sql, MESSAGE_SUBJECTS_SQL};
use crate::repositories::chat_participants::{
    ADD_PARTICIPANT_SQL, GET_PARTICIPANT_SQL, LIST_PARTICIPANTS_SQL, REMOVE_PARTICIPANT_SQL,
};
use crate::repositories::chat_subject::{
    NewSubjectConversation, OpenedSubjectThread, CREATE_SUBJECT_CONVERSATION_SQL,
    REOPEN_SUBJECT_CONVERSATION_SQL, SUBJECT_CONVERSATION_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_chat_repository!(
    PostgresDatabase,
    PgRow,
    NativeUuid,
    "FOR NO KEY UPDATE",
    "BEGIN"
);
