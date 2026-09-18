// ABOUTME: PostgreSQL-backed AgentArtefactRepository, emitted from the shared implementation
// ABOUTME: Replace-the-set write in one transaction; read ordered by (kind, slug)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{AgentArtefact, PackageArtefact};
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::agent_artefacts::{
    artefact_from_row, impl_agent_artefact_repository, AgentArtefactRepository,
    CLEAR_AGENT_ARTEFACTS_SQL, INSERT_AGENT_ARTEFACT_SQL, LIST_AGENT_ARTEFACTS_SQL,
};

impl_agent_artefact_repository!(PostgresDatabase);
