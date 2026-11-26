// ABOUTME: The consent grant a delegated token is admitted under: its standing, the ceiling it carries, its hourly budget
// ABOUTME: A token under a detailed grant always carries its ceiling, and never reads as a plain grant

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_auth::auth::Claims;
use pierre_auth::oauth2_server::authorization_details::athlete_zone;
use pierre_auth::oauth2_server::grant_ceiling::GrantCeiling;
use pierre_auth::rate_limiting::{calculate_grant_budget, grant_window_start, RequestBudget};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::User;
use tracing::{error, warn};
use uuid::Uuid;

use super::outcome::Refused;
use super::{GrantPolicy, McpAuthMiddleware};
use crate::rate_limiting::{enforce_request_budget, log_budget_refusal};

impl McpAuthMiddleware {
    /// Refuse a delegated token whose athlete has withdrawn the client's
    /// access, and resolve the ceiling of the grant it is admitted under.
    ///
    /// The JWT stays good until it expires, so this is what makes revocation
    /// take effect within seconds on the paths that take such a token. A
    /// token naming its consent grant (`client_grant_id`) is admitted only
    /// while that grant is live and unexpired; one naming none keeps the
    /// issued-at rule
    /// ([`ClientGrantStanding::admits_token`](pierre_core::models::ClientGrantStanding::admits_token)). Only
    /// the scoped paths take one, and the caller has refused a first-party
    /// session there before this runs; a token naming no client is not one a
    /// grant governs. A read that fails refuses the request, as a quota that
    /// cannot be read does.
    ///
    /// The ceiling is the detailed grant the token is held to
    /// ([`ClientGrantStanding::detailed_grant_for`](pierre_core::models::ClientGrantStanding::detailed_grant_for)):
    /// the grant it names, or for a token naming none the client's live grant
    /// for its scope and failing that any detailed one, so a token that lost
    /// its claim never reads as a plain grant beside a preset. `None` under a
    /// plain grant. Its daily window is read in the athlete's own zone.
    ///
    /// # Errors
    /// `AuthInvalid` when the consent was withdrawn, so a client re-authorizes,
    /// or when the grant's stored details cannot be resolved to a ceiling; a
    /// database error when the tenant or the grant cannot be read.
    pub(super) async fn require_unwithdrawn_grant(
        &self,
        claims: &Claims,
        user: &User,
        policy: GrantPolicy<'_>,
    ) -> AppResult<Option<GrantCeiling>> {
        let user_id = user.id;
        let GrantPolicy::ScopesEnforced { .. } = policy else {
            return Ok(None);
        };
        let Some(client_id) = claims.client_id.as_deref() else {
            return Ok(None);
        };
        let tenant_id = match claims.active_tenant_id.clone() {
            Some(tenant_id) => tenant_id,
            None => match self.repos.tenants.first_for_user(user_id).await? {
                Some(tenant_id) => tenant_id.to_string(),
                // No tenant, no grant the client could have been given here.
                None => return Ok(None),
            },
        };
        let standing = match &self.grant_standings {
            Some(standings) => standings.read(user_id, &tenant_id, client_id).await?,
            None => {
                self.repos
                    .oauth2_server
                    .client_grant_standing(&user_id.to_string(), &tenant_id, client_id)
                    .await?
            }
        };
        let client_grant_id = claims.client_grant_id.as_deref();
        if !standing.admits_token(claims.iat, client_grant_id, Timestamp::now()) {
            warn!(
                user_id = %user_id,
                client_id = %client_id,
                client_grant_id = ?client_grant_id,
                "Delegated token refused: the athlete withdrew this client's access, or its grant expired"
            );
            return Err(AppError::auth_invalid(
                "The athlete withdrew this application's access; authorize it again",
            ));
        }
        let Some(held) = standing.detailed_grant_for(client_grant_id, &claims.scope) else {
            return Ok(None);
        };
        let Some(document) = held.authorization_details.as_deref() else {
            return Ok(None);
        };
        let zone = athlete_zone(user.timezone.as_deref());
        // The grant's live step-up child, read whole or not at all: a child
        // whose details do not read refuses the token as its parent would.
        let child = held
            .child
            .as_ref()
            .filter(|child| {
                child
                    .expires_at
                    .is_none_or(|expires_at| Timestamp::now() < expires_at)
            })
            .map(|child| {
                GrantCeiling::from_document(&child.id, &child.authorization_details, &zone)
                    .map(|ceiling| (ceiling, child.terms.clone()))
            })
            .transpose();
        GrantCeiling::from_document(&held.id, document, &zone)
            .and_then(|ceiling| {
                Ok(match child? {
                    Some((child, terms)) => ceiling.with_child(child, terms),
                    None => ceiling,
                })
            })
            .map(Some)
            .map_err(|e| {
                error!(
                    user_id = %user_id,
                    client_id = %client_id,
                    client_grant_id = %held.id,
                    "Delegated token refused: its grant's details do not resolve to a ceiling: {e}"
                );
                AppError::auth_invalid(
                    "This application's access could not be read; authorize it again",
                )
            })
    }

    /// The hourly request budget of the grant `ceiling` names, refused when
    /// spent: the 429 its `Retry-After` comes from, as the monthly budget's.
    ///
    /// # Errors
    /// `Refused::OverBudget` when the grant's hour is spent; a database error
    /// when its usage cannot be read, which refuses the request.
    pub(super) async fn enforce_grant_budget(
        &self,
        user_id: Uuid,
        ceiling: &GrantCeiling,
        now: Timestamp,
    ) -> Result<RequestBudget, Refused> {
        let usage = self
            .repos
            .usage
            .get_grant_window_usage(user_id, &ceiling.client_grant_id, grant_window_start(now))
            .await?;
        let budget = calculate_grant_budget(ceiling.budgets.requests_per_hour, &usage, now);
        enforce_request_budget(budget, now).map_err(|error| {
            log_budget_refusal(
                budget,
                &user_id.to_string(),
                "grant_requests_per_hour",
                "http",
            );
            Refused::OverBudget {
                budget,
                error,
                caller: user_id,
            }
        })?;
        Ok(budget)
    }
}
