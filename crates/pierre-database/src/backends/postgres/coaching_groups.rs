// ABOUTME: PostgreSQL-backed CoachingGroupRepository, emitted from the shared implementation in repositories/coaching_groups.rs
// ABOUTME: uuid columns are native here, so the shared statements bind and read ids through the native codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, ConnectGroupChat, CorrectedReply, GroupChatConnectCode,
    GroupChatConnectOutcome, GroupChatConnection, GroupDigestMode, GroupInvite, GroupInviteKind,
    GroupMember, GroupRespondMode, GroupRole, GroupSetupTimeline, GroupSummary,
    GroupTranscriptEntry, NewGroupTranscriptEntry, ReleasedGroupChat, RoomCorrector, RoomEntryBody,
    RoomTranscriptEntry, SharedRoomEntry, TranscriptSpeaker, UpdateGroupRequest,
};
use pierre_core::models::TenantId;
use pierre_core::transport::TransportPolicy;
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::coaching_groups::{
    column, impl_coaching_group_repository, instant, ARCHIVE_GROUP_SQL, COUNT_GROUPS_FOR_OWNER_SQL,
    DEACTIVATE_GROUP_INVITES_SQL, FIND_GROUPS_FOR_USER_AND_AGENT_SQL, GET_GROUP_BY_CHANNEL_SQL,
    GET_GROUP_SQL, INSERT_GROUP_SQL, LIST_ACTIVE_GROUPS_FOR_TENANT_SQL,
    LIST_ATHLETES_COACHED_BY_SQL, LIST_COACHLESS_GROUPS_OWNED_BY_SQL, LIST_GROUPS_COACHED_BY_SQL,
    LIST_GROUPS_FOR_USER_SQL, RELEASE_GROUP_MEMBERS_SQL, SET_GROUP_COACH_USER_SQL,
    UPDATE_GROUP_SQL,
};
use crate::repositories::group_chat_connect::{
    impl_group_chat_connect_repository, GroupChatConnectRepository, CONNECT_GROUP_CHAT_SQL,
    DISCONNECT_GROUP_CHAT_SQL, FORGET_GROUP_CHAT_RELEASE_SQL, GROUP_CHAT_BINDING_SQL,
    GROUP_CHAT_RELEASED_SQL, INSERT_CHAT_CONNECT_CODE_SQL, LIVE_CHAT_CONNECT_CODE_SQL,
    RECORD_CHAT_INVITE_PROMPT_SQL, RECORD_GROUP_CHAT_RELEASE_SQL, RELEASE_GROUP_CHAT_FOR_MOVE_SQL,
    SPEND_CHAT_CONNECT_CODE_SQL,
};
use crate::repositories::group_digest_sql::{CLAIM_GROUP_DIGEST_SQL, FINISH_GROUP_DIGEST_SQL};
use crate::repositories::group_invite_sql::{
    CLAIM_SIGNUP_INVITE_SQL, DEACTIVATE_INVITE_SQL, FIND_INVITE_BY_CODE_SQL,
    GET_INVITE_BY_CODE_SQL, GET_INVITE_BY_ID_SQL, GET_INVITE_SQL, INCREMENT_INVITE_USE_COUNT_SQL,
    INSERT_INVITE_SQL, LIST_INVITES_SQL, LIST_SIGNUP_INVITES_SQL,
};
use crate::repositories::group_member_sql::{
    ANSWER_ASSISTANT_NOTICE_SQL, COUNT_MEMBERS_SQL, GET_ASSISTANT_NOTICE_ANSWERED_SQL,
    GET_MEMBER_SQL, GROUP_SETUP_TIMELINE_SQL, INSERT_MEMBER_SQL, LIST_MEMBERS_SQL,
    REMOVE_MEMBER_SQL, SET_COACH_ASSISTANT_READ_SQL, SET_COACH_CONSENTS_SQL,
    UPDATE_COACH_SHARING_CONSENT_SQL, UPDATE_MEMBER_ROLE_SQL, UPDATE_PEER_SHARING_CONSENT_SQL,
};
use crate::repositories::group_transcript::{
    GET_ROOM_TRANSCRIPT_ENTRY_SQL, INSERT_TRANSCRIPT_ENTRY_SQL, LIST_ROOM_TRANSCRIPT_SQL,
    LIST_TRANSCRIPT_VISIBLE_TO_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::CoachingGroupRepository;

impl_coaching_group_repository!(PostgresDatabase, PgRow, NativeUuid);
impl_group_chat_connect_repository!(PostgresDatabase, NativeUuid);
