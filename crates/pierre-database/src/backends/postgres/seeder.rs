// ABOUTME: PostgreSQL-backed SeederRepository, emitted from the shared implementation in repositories/seeder.rs
// ABOUTME: uuids are native here, the usage tables mint a SERIAL key, and a2a_clients.capabilities is a TEXT[] array
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::mobility::{ActivityMuscleMapping, StretchingExercise, YogaPose};
use pierre_core::models::User;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::seeder::{
    insert_a2a_usage_sql, insert_api_key_usage_sql, A2A_CLIENT_ID_BY_NAME_SQL,
    ADMIN_USER_EMAIL_SQL, AGENT_AUTHOR_ID_SQL, AGENT_BY_SLUG_SQL, AGENT_DRIFT_INFO_SQL,
    API_KEY_ID_BY_NAME_SQL, CATALOGUE_AGENTS_SQL, CATALOGUE_SLUGS_SQL, COUNT_NON_ADMIN_USERS_SQL,
    DELETE_LLM_USAGE_BY_TENANT_SQL, DELETE_SYNTHETIC_BY_USER_SQL, DETACH_AGENT_CONVERSATIONS_SQL,
    INSERT_A2A_CLIENT_SQL, INSERT_AGENT_AUTHOR_SQL, INSERT_AGENT_RELATION_SQL, INSERT_AGENT_SQL,
    INSERT_API_KEY_SQL, INSERT_LLM_USAGE_SQL, INSERT_STORE_LISTING_SQL,
    INSERT_SYNTHETIC_ACTIVITY_SQL, INSERT_TENANT_USER_SQL, NON_ADMIN_USER_IDS_SQL,
    TAKE_CATALOGUE_OWNERSHIP_SQL, UPDATE_AGENT_SQL, UPDATE_USER_TENANT_SQL,
    UPSERT_ACTIVITY_MAPPING_SQL, UPSERT_AGENT_TRANSLATION_SQL, UPSERT_DEMO_USER_SQL,
    UPSERT_PROVIDER_CONNECTION_SQL, UPSERT_STRETCHING_EXERCISE_SQL, UPSERT_TENANT_SQL,
    UPSERT_YOGA_POSE_SQL, USER_ID_BY_EMAIL_SQL, USER_TENANT_SQL,
};
use crate::repositories::seeder_body::{
    column, id_and_slug, impl_seeder_repository, int_column, json_field, status_code_column,
    text_and_hash, uuid_text_column, NativeSeedColumns,
};
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::{
    SeedTable, SeederRepository, UserRepository, AGENT_INSTALL_COUNT_RESYNC, AGENT_POINTER_MERGES,
    AGENT_POINTER_REWRITES, AGENT_SLUG_REWRITES,
};
use crate::seed_models::{
    SeedA2AClient, SeedA2AUsage, SeedAgent, SeedAgentAuthor, SeedAgentRelation,
    SeedAgentTranslation, SeedApiKey, SeedApiKeyUsage, SeedDemoUser, SeedLlmUsageRecord,
    SeedProviderConnection, SeedStoreListing, SeedSyntheticActivity, SeedTenant,
};

impl_seeder_repository!(PostgresDatabase, NativeUuid, NativeSeedColumns, "", "");
