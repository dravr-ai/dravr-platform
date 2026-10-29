// ABOUTME: Handler for GET /api/chat/athletes/{athlete_user_id}/plan — a group's coach reads a coached athlete's live plan, read-only
// ABOUTME: Group info's "View plan": the coach gates and the athlete's tenant allowing the plan read, no thread opened

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1244. Group info and the roster show a coached athlete's published
//! plan read-only from here, outside the plan thread (carnet#1070) where the
//! coach drafts and publishes it: the plan the athlete sees, in the shape
//! `GET /api/me/training-plan` answers with, and no draft. Nothing is opened
//! or written.
//!
//! The gates are those every coach read of a plan runs
//! ([`plan_scope::coached_plan_read_access`]): the caller is the active
//! group's human coach, the athlete a live member who still shares with them
//! and has one home tenant, and that tenant allows `get_training_plan`. A
//! plan derived from first-party-only data is refused over an external
//! transport, as on every surface that shows it (carnet#769).

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    Json,
};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_middleware::AuthenticatedUser;
use pierre_services::athlete_clock;
use pierre_services::locale::user_locale;
use pierre_services::personas;
use pierre_services::plan_card::{self, PlanSubject};
use pierre_tool_runtime::implementations::plan_scope::{self, CoachedPlanRefusal};
use serde_json::json;
use tracing::info;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::tools::runtime_adapter::into_runtime;

use super::dto::{AthletePlanQuery, AthletePlanResponse};

/// The refusal as an HTTP error, with the `details.reason` a client words:
/// the coaching gates' own, or 403 `plans_disabled` when the athlete's
/// tenant turned the plan read off.
fn refusal_error(refused: CoachedPlanRefusal) -> AppError {
    match refused {
        CoachedPlanRefusal::Coaching(gate) => gate.into_error(),
        CoachedPlanRefusal::PlansDisabled { athlete } => {
            let mut error = AppError::new(
                ErrorCode::PermissionDenied,
                format!("Training plans are turned off in {athlete}'s workspace"),
            );
            error.details = Some(Box::new(json!({ "reason": "plans_disabled" })));
            error
        }
    }
}

/// `GET /api/chat/athletes/{athlete_user_id}/plan?group_id=`: the athlete's
/// live plan projected on their today, in the coach's locale (or
/// `?locale=`), with `live: null` when they have none.
///
/// A refusal carries `details.reason`: 403 `not_group_coach`, 404
/// `not_a_member`, 403 `coach_sharing_revoked`, 409 `several_tenants` or
/// `no_tenant`, 403 `plans_disabled`.
pub(super) async fn get_athlete_plan(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(athlete_user_id): Path<String>,
    Query(query): Query<AthletePlanQuery>,
) -> Result<Json<AthletePlanResponse>, AppError> {
    let athlete = Uuid::parse_str(athlete_user_id.trim())
        .map_err(|_| AppError::invalid_input("athlete_user_id must be a UUID"))?;
    let coach = auth.user_id;
    let repos = &resources.common.repos;
    let access = plan_scope::coached_plan_read_access(
        &into_runtime(&resources),
        coach,
        athlete,
        query.group_id,
    )
    .await?
    .map_err(refusal_error)?;

    let today = athlete_clock::athlete_today(repos, athlete).await;
    let coach_user = repos.users.get_global(coach).await?;
    let locale = personas::resolve_persona_locale(
        query.locale.as_deref(),
        Some(&user_locale(coach_user.as_ref())),
    );
    let live = plan_card::try_load_plan_card(
        repos,
        access.tenant,
        athlete,
        today,
        &resources.mcp.messaging_strings_registry,
        &locale,
    )
    .await?;
    info!(
        coach_user_id = %coach,
        athlete_user_id = %athlete,
        group_id = %access.group.id,
        has_plan = live.is_some(),
        "coach read a coached athlete's live plan"
    );
    Ok(Json(AthletePlanResponse {
        athlete: PlanSubject {
            user_id: athlete,
            display_name: access.display_name,
        },
        today,
        live,
    }))
}
