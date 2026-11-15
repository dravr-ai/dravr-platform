// ABOUTME: GET /api/me/coach-setup — a coach's « Préparer ton coaching » checklist: the seat, the athlete invited, a coaching platform
// ABOUTME: Each row is done from server state (coach access, a member who joined, a connection), so web and phone tick the same rows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coach's checklist (carnet#1529).
//!
//! A user who coaches others without coach access lands on a coach-shaped
//! Home, never the athlete's Getting started. This route is that Home's list,
//! shaped like Getting started's: rows in order, each done or not and the
//! first open one current, then the facts the rows were decided on, so the
//! clients say what the server holds — the groups the invite works on, the
//! athlete who joined and when, the coaching platform linked. The rules live
//! in [`pierre_services::first_run::coach`].

use std::sync::Arc;

use axum::{extract::State, Json};
use chrono::{DateTime, Utc};
use pierre_core::errors::AppError;
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_runtime_context::{resolve_tenant, tenant::require, TenantMode};
use pierre_services::first_run::coach::{
    coach_setup, CoachFacts, CoachGroup, CoachStep, JoinedAthlete,
};
use serde::Serialize;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// One row of `GET /api/me/coach-setup`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoachSetupRowResponse {
    /// `seat`, `invite` or `platform`.
    pub id: &'static str,
    /// The row's place in the list, from 1.
    pub order: usize,
    /// Done.
    pub done: bool,
    /// The first row not done.
    pub current: bool,
}

impl From<CoachStep> for CoachSetupRowResponse {
    fn from(step: CoachStep) -> Self {
        Self {
            id: step.id,
            order: step.order,
            done: step.done,
            current: step.current,
        }
    }
}

/// One of the coach's groups.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoachSetupGroupResponse {
    /// The group's id.
    pub id: Uuid,
    /// The group's name.
    pub name: String,
}

impl From<CoachGroup> for CoachSetupGroupResponse {
    fn from(group: CoachGroup) -> Self {
        Self {
            id: group.id,
            name: group.name,
        }
    }
}

/// The first person who joined one of the coach's groups.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoachSetupAthleteResponse {
    /// Their display name, else their email; `null` when neither reads.
    pub name: Option<String>,
    /// The group they joined.
    pub group_id: Uuid,
    /// When they joined it (RFC 3339).
    pub joined_at: DateTime<Utc>,
}

impl From<JoinedAthlete> for CoachSetupAthleteResponse {
    fn from(athlete: JoinedAthlete) -> Self {
        Self {
            name: athlete.name,
            group_id: athlete.group_id,
            joined_at: athlete.joined_at,
        }
    }
}

/// Body of `GET /api/me/coach-setup`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoachSetupResponse {
    /// The seat, the athlete invited, the coaching platform, in that order.
    pub rows: Vec<CoachSetupRowResponse>,
    /// Rows done.
    pub done_count: usize,
    /// Rows in all.
    pub total: usize,
    /// The coach's groups, oldest first: the coachless ones they own in this
    /// tenant, then the ones they coach. Empty until they make one, when the
    /// invite row offers to.
    pub groups: Vec<CoachSetupGroupResponse>,
    /// The first person who joined one of those groups; `null` until someone has.
    pub athlete: Option<CoachSetupAthleteResponse>,
    /// The coaching platform the coach linked (`sciotte_trainingpeaks` or
    /// `intervals_icu`); `null` until they link one.
    pub platform: Option<&'static str>,
}

/// `GET /api/me/coach-setup` — the caller's coach checklist.
///
/// # Errors
///
/// Returns the authentication error, a refusal for a caller with no tenant,
/// and the repository error when the account, the groups, their members or
/// the connections cannot be read: a list built on a partial read could tick
/// a row that is not done.
pub async fn get_coach_setup(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> Result<Json<CoachSetupResponse>, AppError> {
    let auth = auth.into_inner();
    let tenant = require(resolve_tenant(&resources, &auth, TenantMode::Required).await?)?;
    let facts = CoachFacts::read(&resources.common.repos, tenant, auth.user_id).await?;
    let setup = coach_setup(&facts);
    Ok(Json(CoachSetupResponse {
        rows: setup.steps.into_iter().map(Into::into).collect(),
        done_count: setup.done_count,
        total: setup.total,
        groups: facts.groups.into_iter().map(Into::into).collect(),
        athlete: facts.first_athlete.map(Into::into),
        platform: facts.platform,
    }))
}
