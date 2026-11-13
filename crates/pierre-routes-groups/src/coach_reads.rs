// ABOUTME: GET /api/groups/{id}/members/me/coach-reads — the member's own record of their human coach reading their data
// ABOUTME: The caller's rows only, under each tenant they belong to, newest first, beside their coach sharing (carnet#1445)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A member's record of their coach's reads
//!
//! A group's human coach reads a coached athlete's data through their agent
//! under the athlete's coach sharing, and each read is recorded once
//! (`coach_data_reads`). This route lists the caller's own record in one
//! group: who read, which kind of data, through which tool, and when, kept
//! [`COACH_READ_RETENTION_MONTHS`] months. There is no path to anyone else's
//! record: the athlete is the caller, and each tenant read is one the caller
//! belongs to ([`own_coach_reads`], the listing `/group reads` uses too).

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use pierre_core::errors::AppError;
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{GroupsCtx, MiddlewareCtx};
use pierre_services::athlete_display_name::fetch_user_display_name;
use pierre_services::coach_read_retention::{own_coach_reads, COACH_READ_RETENTION_MONTHS};

/// Reads one page lists when the caller names no `limit`.
const DEFAULT_READS: u32 = 50;

/// Most reads one page lists.
const MAX_READS: u32 = 200;

/// One read of the caller's data by the group's human coach.
#[derive(Debug, Serialize)]
pub struct CoachReadBody {
    /// The coach who read, by display name; `None` once their account is gone.
    pub coach_name: Option<String>,
    /// The tool the coach's agent read the data through.
    pub tool: String,
    /// `training` or `wellness`.
    pub data_class: &'static str,
    /// When the coach read it (RFC 3339).
    pub read_at: String,
}

/// The caller's record of their coach's reads in one group.
#[derive(Debug, Serialize)]
pub struct CoachReadsBody {
    /// Newest first.
    pub reads: Vec<CoachReadBody>,
    /// How long a read stays listed before it is purged.
    pub retention_months: u32,
}

/// How many reads to list.
#[derive(Debug, Deserialize)]
pub struct CoachReadsQuery {
    /// At most this many, clamped to `1..=200`; 50 when omitted.
    pub limit: Option<u32>,
}

/// The member's record of their coach's reads.
#[derive(Debug)]
pub struct CoachReadRoutes;

impl CoachReadRoutes {
    /// The route, mounted beside [`crate::groups::GroupRoutes::routes`] under
    /// `/api/groups`.
    pub fn routes<C: GroupsCtx + MiddlewareCtx>(resources: Arc<C>) -> Router {
        Router::new()
            .route(
                "/api/groups/{group_id}/members/me/coach-reads",
                get(Self::handle_list::<C>),
            )
            .with_state(resources)
    }

    /// GET `/api/groups/:group_id/members/me/coach-reads` — the caller's own
    /// record of the group's human coach reading their data, newest first.
    ///
    /// The caller must be a live member of the group; the rows read are the
    /// caller's own, under each tenant they belong to: a read is recorded
    /// under the athlete's home tenant at the time, which need not be the
    /// tenant this session is in.
    async fn handle_list<C: GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
        Query(query): Query<CoachReadsQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let group = Uuid::parse_str(&group_id)
            .map_err(|_| AppError::not_found("You are not a member of this group"))?;
        let repos = resources.repos();
        repos
            .groups
            .get_member(&group_id, auth.user_id)
            .await?
            .ok_or_else(|| AppError::not_found("You are not a member of this group"))?;

        let limit = query.limit.unwrap_or(DEFAULT_READS).clamp(1, MAX_READS);
        let rows = own_coach_reads(repos, auth.user_id, group, limit).await?;
        // A coach is named once per page, however many reads they made.
        let mut names: HashMap<Uuid, String> = HashMap::new();
        let mut reads = Vec::with_capacity(rows.len());
        for row in rows {
            let coach_name = match row.coach {
                Some(coach) => Some(match names.entry(coach) {
                    Entry::Occupied(known) => known.get().clone(),
                    Entry::Vacant(slot) => slot
                        .insert(fetch_user_display_name(repos, coach).await)
                        .clone(),
                }),
                None => None,
            };
            reads.push(CoachReadBody {
                coach_name,
                tool: row.tool,
                data_class: row.class.as_str(),
                read_at: row.read_at.to_rfc3339(),
            });
        }
        let body = CoachReadsBody {
            reads,
            retention_months: COACH_READ_RETENTION_MONTHS,
        };
        Ok((StatusCode::OK, Json(body)).into_response())
    }
}
