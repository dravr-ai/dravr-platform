// ABOUTME: SQLite-backed CoachingGroupRepository, emitted from the shared implementation in repositories/coaching_groups.rs
// ABOUTME: uuid columns are hyphenated TEXT here, so the shared statements bind and read ids through the text codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupInvite, GroupInviteKind, GroupMember, GroupRespondMode,
    GroupRole, GroupSummary, GroupTranscriptEntry, NewGroupTranscriptEntry, TranscriptSpeaker,
    UpdateGroupRequest,
};
use pierre_core::models::TenantId;
use sqlx::sqlite::SqliteRow;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::coaching_groups::{
    column, impl_coaching_group_repository, instant, ARCHIVE_GROUP_SQL, CLAIM_GROUP_DIGEST_SQL,
    COUNT_GROUPS_FOR_OWNER_SQL, COUNT_MEMBERS_SQL, DEACTIVATE_GROUP_INVITES_SQL,
    DEACTIVATE_INVITE_SQL, FIND_GROUPS_FOR_USER_AND_AGENT_SQL, FINISH_GROUP_DIGEST_SQL,
    GET_GROUP_BY_CHANNEL_SQL, GET_GROUP_SQL, GET_INVITE_BY_CODE_SQL, GET_INVITE_SQL,
    GET_MEMBER_SQL, INCREMENT_INVITE_USE_COUNT_SQL, INSERT_GROUP_SQL, INSERT_INVITE_SQL,
    INSERT_MEMBER_SQL, INSERT_TRANSCRIPT_ENTRY_SQL, LIST_ACTIVE_GROUPS_FOR_TENANT_SQL,
    LIST_GROUPS_COACHED_BY_SQL, LIST_GROUPS_FOR_AGENT_SQL, LIST_GROUPS_FOR_USER_SQL,
    LIST_INVITES_SQL, LIST_MEMBERS_SQL, LIST_TRANSCRIPT_VISIBLE_TO_SQL, RELEASE_GROUP_MEMBERS_SQL,
    REMOVE_MEMBER_SQL, SET_GROUP_COACH_USER_SQL, UPDATE_GROUP_SQL, UPDATE_MEMBER_ROLE_SQL,
    UPDATE_PEER_SHARING_CONSENT_SQL,
};
use crate::repositories::uuid_columns::TextUuid;
use crate::repositories::CoachingGroupRepository;

impl_coaching_group_repository!(Database, SqliteRow, TextUuid);
