// ABOUTME: Whether a push-fed grant's history is in, read from the rows held under its history keys
// ABOUTME: One rule for the provider card, the lead's first welcome and the agent's prompt (carnet#1386, carnet#1623)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A push-fed grant's history
//!
//! carnet#1386, carnet#1494, carnet#1623. A push-fed grant (Garmin) delivers
//! only what the athlete records after connecting, so whether what came
//! before is in is read off the rows: held under the sign-in's key or an
//! imported archive's
//! ([`HistoryState::Imported`]), or not ([`HistoryState::Needed`]). A sign-in
//! makes the session the provider's primary source, whose own reads bring the
//! history in. Nothing new is stored for it: the rows under the history keys
//! say it.
//!
//! The provider card reads it to offer its Add history action, and the chat
//! reads the same rule: the lead's first welcome says the past is missing and
//! points at that action, and the agent's prompt carries it so a
//! recommendation that leans on past load says what it cannot see.

use std::iter;

use pierre_auth::dto::auth::HistoryState;
use pierre_core::constants::oauth::providers::companion_keys;
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver::{self, ReadPlan};
use pierre_providers::registry::ProviderRegistry;
use uuid::Uuid;

/// Whether the athlete's own grant serves `provider`'s card first: the plan's
/// primary is the provider's OAuth backend.
#[must_use]
pub fn grant_serves(provider: &str, plan: Option<&ReadPlan>) -> bool {
    let user_facing = backend_resolver::user_facing_name(provider);
    backend_resolver::mirror_backend_for(user_facing).is_some()
        && plan.is_some_and(|plan| plan.primary == user_facing)
}

/// A push-fed grant's history, as [`grant_history`] reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrantHistory {
    /// Where the history stands.
    pub state: HistoryState,
    /// The sign-in that brings the history in (the provider's usr/pwd login
    /// backend), for a history it can bring in; `None` when the state is
    /// `not_applicable`.
    pub sign_in_target: Option<&'static str>,
}

/// The history of `card` for `user_id` in `tenant`, whose own grant serves it
/// first when `grant_serves` ([`grant_serves`]).
///
/// Rows held under the sign-in's key or an imported archive's read
/// `imported`: an athlete whose session the one-time import retired keeps
/// what it stored. Otherwise `needed`: rows the grant pushed itself start at
/// the day it connected, so they never stand for what came before it.
///
/// A card whose provider is not push-fed, or that no grant of the user's
/// serves first (a session beside it reads its own history), or read outside
/// a tenant, reads `not_applicable`.
pub async fn grant_history(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: Option<TenantId>,
    card: &str,
    grant_serves: bool,
) -> GrantHistory {
    let user_facing = backend_resolver::user_facing_name(card);
    let sign_in = backend_resolver::mirror_backend_for(user_facing);
    let (Some(tenant), Some(sign_in), true) = (
        tenant,
        sign_in,
        grant_serves && backend_resolver::pushed_backend_for(user_facing) == Some(user_facing),
    ) else {
        return GrantHistory {
            state: HistoryState::NotApplicable,
            sign_in_target: None,
        };
    };
    GrantHistory {
        state: history_state(repos, user_id, tenant, sign_in, user_facing).await,
        sign_in_target: Some(sign_in),
    }
}

/// [`grant_history`]'s state for a card a push-fed grant serves: `imported`
/// when rows are held under the sign-in's key or an imported archive's, else
/// `needed`.
async fn history_state(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
    sign_in: &str,
    grant_key: &str,
) -> HistoryState {
    let keys = iter::once(sign_in).chain(companion_keys(grant_key).iter().copied());
    for key in keys {
        if backend_resolver::holds_activities(repos, user_id, tenant, key).await {
            return HistoryState::Imported;
        }
    }
    HistoryState::Needed
}

/// The push-fed providers among `connected` whose grant serves alone and whose
/// history reads `needed` for `user_id` in `tenant`, each named once by its
/// user-facing name (`garmin`), in the order first met.
///
/// `connected` names the athlete's connection rows, either half of a pair.
/// A provider that is not push-fed costs no read.
pub async fn histories_needed<'a>(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
    connected: impl IntoIterator<Item = &'a str>,
) -> Vec<&'static str> {
    let mut pushed: Vec<&'static str> = Vec::new();
    for provider in connected {
        if let Some(fed) = backend_resolver::pushed_backend_for(provider) {
            if !pushed.contains(&fed) {
                pushed.push(fed);
            }
        }
    }
    let mut needed = Vec::with_capacity(pushed.len());
    for provider in pushed {
        let plan = backend_resolver::read_plan(repos, user_id, tenant, provider).await;
        let serves = grant_serves(provider, Some(&plan));
        let history = grant_history(repos, user_id, Some(tenant), provider, serves).await;
        if history.state == HistoryState::Needed {
            needed.push(provider);
        }
    }
    needed
}

/// `provider`, one of [`histories_needed`]'s, named as the athlete knows it
/// (`Garmin`): the brand its sign-in shows, else its own display name, else
/// the slug. The welcome and the agent's prompt name it alike.
#[must_use]
pub fn athlete_name(registry: &ProviderRegistry, provider: &'static str) -> &'static str {
    backend_resolver::brand_name(registry, provider)
        .or_else(|| registry.get_display_name(provider))
        .unwrap_or(provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::constants::oauth_providers;

    fn plan(primary: &str) -> ReadPlan {
        ReadPlan {
            primary: primary.to_owned(),
            fallback: None,
        }
    }

    #[test]
    fn the_grant_serves_only_when_it_is_the_plans_primary() {
        let garmin = oauth_providers::GARMIN;
        assert!(grant_serves(garmin, Some(&plan(garmin))));
        let mirror = backend_resolver::mirror_backend_for(garmin).unwrap_or_default();
        assert!(
            !grant_serves(garmin, Some(&plan(mirror))),
            "a session serves"
        );
        assert!(!grant_serves(garmin, None), "outside a tenant");
    }
}
