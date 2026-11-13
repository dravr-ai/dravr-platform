// ABOUTME: An external tools listing is per audience: the coach-only arguments and tools go to a group's human coach alone
// ABOUTME: One predicate (the roster tool's: coach access and an active group) and one pass over a built listing, shared by MCP and A2A

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Who is offered the coach's arguments (carnet#1426)
//!
//! `athlete=` is how a coach names a coached athlete on a read or a plan
//! call, and the tools that only a coach can use (`publish_training_plan`)
//! serve nobody else. Listed to everyone they cost every athlete's MCP client
//! a hundred bytes on each of about fifty tools and offer a call that can
//! only be refused. So an external listing (`tools/list` on `/mcp`, the A2A
//! extended card) carries them for a coach, as defined below, and for no one
//! else.
//!
//! The listing is advice, never the gate. A call that names `athlete` or a
//! coach-only tool runs the coaching gates of its own (`resolve_read_subject`,
//! `resolve_plan_scope`) and refuses a non-coach in band, whatever was listed.
//!
//! "Coach" is the roster tool's rule (carnet#1511), read through
//! `pierre_services::coach_roster::roster_access` rather than restated: coach
//! access (`manages_roster`, or platform admin) and at least one active group
//! whose `coach_user_id` is the user. It is read on every listing and kept
//! nowhere: `/mcp` holds no session, a per-process copy would be stale on the
//! other replicas, and the listing carries `ttlMs: 0` with
//! `cacheScope: private`, so a client asks again.

use pierre_database::RepositoryRegistry;
use pierre_mcp_schema::ToolSchema;
use pierre_services::coach_roster::{self, RosterAccess};
use tracing::warn;
use uuid::Uuid;

use crate::read_subject::ATHLETE_ARGUMENT;
use crate::registry::ToolRegistry;

/// Whether `user_id` is a coach by the roster tool's rule: coach access
/// (`manages_roster`, or platform admin) and at least one active group they
/// coach.
///
/// A failed lookup answers `false`: the listing then omits what a coach would
/// see, which the next listing restores, rather than offering a non-coach what
/// the call would refuse.
pub async fn is_human_coach(repos: &RepositoryRegistry, user_id: Uuid) -> bool {
    match coach_roster::roster_access(repos, user_id).await {
        Ok(RosterAccess::Granted(_)) => true,
        Ok(RosterAccess::NotACoach) => false,
        Err(error) => {
            warn!(%user_id, %error, "coach lookup for a tools listing failed; listing as a non-coach");
            false
        }
    }
}

/// Narrow a built listing to its audience.
///
/// A coach is listed everything. Anyone else loses the tools that declare
/// themselves coach-only and the `athlete` property of every tool left,
/// whether the registry added it to a training or wellness read or the tool
/// declares its own (the plan and calendar tools).
pub fn retain_for_audience(registry: &ToolRegistry, schemas: &mut Vec<ToolSchema>, is_coach: bool) {
    if is_coach {
        return;
    }
    schemas.retain(|schema| !registry.is_coach_only(&schema.name));
    for schema in schemas.iter_mut() {
        if let Some(properties) = &mut schema.input_schema.properties {
            properties.remove(ATHLETE_ARGUMENT);
        }
        if let Some(required) = &mut schema.input_schema.required {
            required.retain(|name| name != ATHLETE_ARGUMENT);
        }
    }
}
