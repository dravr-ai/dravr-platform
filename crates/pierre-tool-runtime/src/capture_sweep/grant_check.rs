// ABOUTME: The daily sweep's grant check — every live push-only grant is walked through the token path
// ABOUTME: A refresh the provider refuses flags the grant, so "Reconnect" shows for an athlete who records nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The grant check (carnet#1386).
//!
//! A push-only provider (Garmin's Developer Program API) spends its token
//! only when a push offers a file to download, so an athlete who records
//! nothing never meets a refused refresh, and their dead grant would read
//! "Connected" until they did. The daily sweep closes that gap: each live
//! push-only grant goes through [`AuthService::get_valid_token`], the token
//! path every read takes, which refreshes a token that is due and flags the
//! connection, with its one notice, when the provider refuses the refresh.
//!
//! A lookup that hands back no token flags the grant here too
//! ([`flag_connection`]): nothing can renew it, so reconnecting is the only
//! remedy, and the flag's notice claim keeps it to one notice however many
//! paths saw it. A refresh that failed without a refusal (a rate limit, a
//! 5xx, a store hiccup) says nothing about the grant and flags nothing.

use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use pierre_core::models::TenantId;
use pierre_database::repositories::CaptureFreshness;
use pierre_providers::registry::ProviderRegistry;
use tracing::warn;
use uuid::Uuid;

use super::{flag_connection, ConnectionRefresh, RefreshOutcome, Walk};
use crate::protocol::AuthService;
use crate::runtime::ToolRuntime;

/// The live push-only grants of `live`, the ones [`check_grants`] walks.
pub(super) fn pushed_grants(
    registry: &ProviderRegistry,
    live: &[CaptureFreshness],
) -> Vec<CaptureFreshness> {
    live.iter()
        .filter(|connection| registry.is_push_only(&connection.provider))
        .cloned()
        .collect()
}

/// Walk each of `grants` that holds a token row through the token path and
/// add its line to the report, returning how many lines it added. Stops
/// starting checks once the time budget is spent; the rest are reported
/// skipped, and counted.
pub(super) async fn check_grants(walk: &mut Walk<'_>, grants: Vec<CaptureFreshness>) -> usize {
    let mut checked = 0;
    for grant in grants {
        let outcome = if walk.exhausted() {
            walk.tally.skipped += 1;
            RefreshOutcome::SkippedBudgetExhausted
        } else {
            let Some(outcome) = check_grant(walk.runtime, &grant).await else {
                continue;
            };
            match outcome {
                RefreshOutcome::Flagged { .. } => walk.tally.flagged += 1,
                RefreshOutcome::Failed { .. } => walk.tally.failed += 1,
                _ => {}
            }
            outcome
        };
        checked += 1;
        walk.connections.push(ConnectionRefresh {
            tenant_id: grant.tenant_id,
            user_id: grant.user_id,
            provider: grant.provider,
            outcome,
        });
    }
    checked
}

/// Take `grant`'s token through the token path: a token it hands back is a
/// grant its provider honours, none is a grant nothing can renew, flagged.
///
/// `None` for a connection with no token row at all: it holds no grant to
/// check, so the sweep neither reports nor flags it.
async fn check_grant(
    runtime: &Arc<dyn ToolRuntime>,
    grant: &CaptureFreshness,
) -> Option<RefreshOutcome> {
    let Ok(user_id) = Uuid::from_str(&grant.user_id) else {
        return Some(RefreshOutcome::Failed {
            error: "unparseable user_id on connection".to_owned(),
        });
    };
    let Ok(tenant) = TenantId::parse_str(&grant.tenant_id) else {
        return Some(RefreshOutcome::Failed {
            error: "unparseable tenant_id on connection".to_owned(),
        });
    };
    if matches!(
        runtime
            .repos()
            .oauth_tokens
            .get_token(user_id, tenant, &grant.provider)
            .await,
        Ok(None)
    ) {
        return None;
    }
    // Taken before the token is read, so a reconnect that lands while the
    // refresh is in flight is newer than the refusal it reports.
    let attempt_started_at = Utc::now();
    let outcome = match AuthService::new(Arc::clone(runtime))
        .get_valid_token(user_id, &grant.provider, Some(&grant.tenant_id))
        .await
    {
        Ok(Some(_)) => RefreshOutcome::GrantValid,
        Ok(None) => {
            flag_connection(
                runtime,
                &grant.tenant_id,
                user_id,
                &grant.provider,
                attempt_started_at,
            )
            .await
        }
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = %grant.provider,
                error = %e,
                "Capture sweep: the grant's token could not be checked; left as it is"
            );
            RefreshOutcome::Failed {
                error: e.to_string(),
            }
        }
    };
    Some(outcome)
}
