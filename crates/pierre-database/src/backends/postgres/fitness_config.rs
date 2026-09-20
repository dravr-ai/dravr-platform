// ABOUTME: PostgreSQL-backed FitnessConfigRepository, emitted from the shared implementation in repositories/fitness_config.rs
// ABOUTME: user_id is a native uuid column here, so the shared statements bind the user id parsed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::config::FitnessConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::fitness_config::{
    configuration_names_from_rows, fitness_config_from_row, impl_fitness_config_repository,
    DELETE_TENANT_FITNESS_CONFIG_SQL, DELETE_USER_FITNESS_CONFIG_SQL,
    GET_TENANT_FITNESS_CONFIG_SQL, GET_USER_FITNESS_CONFIG_SQL, LIST_TENANT_FITNESS_CONFIGS_SQL,
    LIST_USER_FITNESS_CONFIGS_SQL, SAVE_TENANT_FITNESS_CONFIG_SQL, SAVE_USER_FITNESS_CONFIG_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::FitnessConfigRepository;

impl_fitness_config_repository!(PostgresDatabase, NativeUuid);
