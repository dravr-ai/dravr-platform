// ABOUTME: PostgreSQL-backed UserPhysiologicalProfileRepository + DossierRepository, emitted from repositories/user_physiological_profiles.rs
// ABOUTME: Ids bind as native uuid and the zone-set JSON as jsonb, through the same binds SQLite uses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Dossier, TenantId, UserId, UserPhysiologicalProfile};
use pierre_memory::{FactKind, FactSource};
use uuid::Uuid;

use super::PostgresDatabase;
use crate::dossier_facts::{group_facts, FACT_BUNDLE_LIMIT};
use crate::repositories::user_physiological_profiles::{
    impl_user_physiological_profile_repository, profile_binds, profile_from_row, DossierRepository,
    UserPhysiologicalProfileRepository, GET_PHYSIOLOGICAL_PROFILE_SQL,
    UPSERT_PHYSIOLOGICAL_PROFILE_SQL,
};
use crate::repositories::{HarnessMemoryRepository, ProfileRepository};

impl_user_physiological_profile_repository!(PostgresDatabase);
