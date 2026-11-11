// ABOUTME: Which backend serves a connected provider card, judged by the read plan routing reads by
// ABOUTME: A session that is not revoked serves the card before its grant; a usr/pwd-only card may be asked to move

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The backend behind a connected card, by the athlete's read plan
//! (carnet#1302, carnet#1494), and the move a card served by usr/pwd alone
//! asks of the athlete (carnet#1454).

use std::collections::HashSet;

use pierre_auth::connect_order::ConnectOrder;
use pierre_auth::dto::auth::ProviderMigration;
use pierre_auth::garmin_seats::garmin_oauth_required;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ConnectionType, TenantId};
use pierre_providers::backend_resolver::{self, ReadPlan};
use tracing::warn;
use uuid::Uuid;

use crate::AuthRoutesContext;

/// The backend serving `provider_name`'s card for `user_id`, and the
/// athlete's read plan for its provider in `tenant_id` (`None` outside a
/// tenant).
///
/// The row behind the card is the plan's primary when the athlete holds it
/// ([`backend_resolver::read_plan`]): a session that is not revoked before
/// its grant, so the card's reconnect is the session's. When the primary is
/// not held — a dead grant alone, whose plan is the mirror's sign-in — the
/// first connected backend of the pair stands for the card, so the dead
/// grant's flag still reads as its reconnect.
pub(crate) async fn card_backend(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    provider_name: &str,
    connected_providers: &HashSet<String>,
) -> (Option<String>, Option<ReadPlan>) {
    let plan = match tenant_id {
        Some(tid) => Some(
            backend_resolver::read_plan(
                &resources.repos,
                user_id,
                TenantId::from_uuid(tid),
                provider_name,
            )
            .await,
        ),
        None => None,
    };
    let connected = |backend: &str| connected_providers.contains(backend);
    let backend = plan
        .as_ref()
        .map(|plan| plan.primary.clone())
        .filter(|primary| connected(primary))
        .or_else(|| {
            backend_resolver::serving_backends(provider_name)
                .into_iter()
                .find(|backend| connected(backend))
        });
    (backend, plan)
}

/// Whether the athlete's own grant serves `provider_name`'s card first: the
/// plan's primary is the provider's OAuth backend.
pub(crate) fn grant_serves(provider_name: &str, plan: Option<&ReadPlan>) -> bool {
    let user_facing = backend_resolver::user_facing_name(provider_name);
    backend_resolver::mirror_backend_for(user_facing).is_some()
        && plan.is_some_and(|plan| plan.primary == user_facing)
}

/// Whether the card asks for a reconnect, given `connected_backend`, the
/// backend serving it, and `reauth_providers`, the backends flagged for
/// reconnect.
///
/// Only the serving backend counts: its session died (a dead scrape, a
/// refused OAuth refresh), so the UI prompts a reconnect instead of a healthy
/// "Connected". A grant left behind a serving session is not the card's
/// backend, so its flag asks nothing of the athlete: the card reads
/// "Reconnect" only when the backend serving it is flagged.
pub(crate) fn reauth_flag(
    connected_backend: Option<&str>,
    reauth_providers: &HashSet<String>,
) -> bool {
    connected_backend.is_some_and(|backend| reauth_providers.contains(backend))
}

/// The move the Garmin card asks of an athlete it shows served by their own
/// usr/pwd session alone: [`ProviderMigration::OauthRequired`] while the
/// server's Garmin client has `garmin_seats_left` free and Garmin's OAuth
/// goes first (`garmin_order`), else `None`.
///
/// Only the `sciotte_garmin` card whose `connected_backend` is its usr/pwd
/// session, made by the athlete themselves (`connection_type` is not
/// `delegated`), and whose athlete holds no Garmin token row in this tenant
/// (`garmin_grant_held`), is asked: [`garmin_oauth_required`] is the rule.
/// With the pool full the session stays their way in, and the card asks
/// nothing until a seat frees.
pub(crate) fn card_migration(
    card: &str,
    connected_backend: Option<&str>,
    connection_type: Option<&str>,
    garmin_seats_left: u32,
    garmin_grant_held: bool,
    garmin_order: ConnectOrder,
) -> Option<ProviderMigration> {
    let usr_pwd_only = card == oauth_providers::SCIOTTE_GARMIN
        && connected_backend == Some(oauth_providers::SCIOTTE_GARMIN)
        && connection_type != Some(ConnectionType::Delegated.as_str());
    (usr_pwd_only && garmin_oauth_required(garmin_seats_left, garmin_grant_held, garmin_order))
        .then_some(ProviderMigration::OauthRequired)
}

/// Whether `user_id` holds a Garmin token row in `tenant_id`, for
/// [`card_migration`]. A lookup that fails reads as held, so the card asks no
/// move it cannot be sure of.
pub(crate) async fn garmin_grant_held(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
) -> bool {
    let Some(tenant) = tenant_id.map(TenantId::from_uuid) else {
        return true;
    };
    match resources
        .repos
        .oauth_tokens
        .get_token(user_id, tenant, oauth_providers::GARMIN)
        .await
    {
        Ok(token) => token.is_some(),
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "Garmin grant lookup failed; the card asks no move");
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: Option<&str> = Some(oauth_providers::SCIOTTE_GARMIN);
    const MANUAL: Option<&str> = Some("manual");

    #[test]
    fn a_usr_pwd_only_garmin_card_is_asked_to_move_while_a_seat_is_free() {
        assert_eq!(
            card_migration(
                oauth_providers::SCIOTTE_GARMIN,
                SESSION,
                MANUAL,
                5,
                false,
                ConnectOrder::OAuthFirst
            ),
            Some(ProviderMigration::OauthRequired),
            "bug: a usr/pwd-only Garmin athlete with a seat free is asked to move"
        );
    }

    #[test]
    fn a_full_pool_asks_no_move() {
        assert_eq!(
            card_migration(
                oauth_providers::SCIOTTE_GARMIN,
                SESSION,
                MANUAL,
                0,
                false,
                ConnectOrder::OAuthFirst
            ),
            None,
            "bug: with no seat the usr/pwd session stays the way in"
        );
    }

    #[test]
    fn a_grant_a_delegation_or_another_card_asks_no_move() {
        assert_eq!(
            card_migration(
                oauth_providers::SCIOTTE_GARMIN,
                SESSION,
                MANUAL,
                5,
                true,
                ConnectOrder::OAuthFirst
            ),
            None,
            "bug: a grant holder is on the OAuth already"
        );
        assert_eq!(
            card_migration(
                oauth_providers::SCIOTTE_GARMIN,
                Some(oauth_providers::GARMIN),
                Some("oauth"),
                5,
                false,
                ConnectOrder::OAuthFirst
            ),
            None,
            "bug: a card the grant serves asks no move"
        );
        assert_eq!(
            card_migration(
                oauth_providers::SCIOTTE_GARMIN,
                SESSION,
                Some(ConnectionType::Delegated.as_str()),
                5,
                false,
                ConnectOrder::OAuthFirst
            ),
            None,
            "bug: a coach's view of a member is not the athlete's to move"
        );
        assert_eq!(
            card_migration(
                oauth_providers::SCIOTTE_GARMIN,
                None,
                None,
                5,
                false,
                ConnectOrder::OAuthFirst
            ),
            None,
            "bug: a disconnected card has nothing to move"
        );
        assert_eq!(
            card_migration(
                oauth_providers::SCIOTTE_COROS,
                Some(oauth_providers::SCIOTTE_COROS),
                MANUAL,
                5,
                false,
                ConnectOrder::OAuthFirst
            ),
            None,
            "bug: COROS has no OAuth to move to"
        );
    }

    /// With the sign-in first (carnet#1472), a usr/pwd-only Garmin athlete is
    /// never asked to move, seats or not.
    #[test]
    fn a_sign_in_first_order_asks_no_move() {
        assert_eq!(
            card_migration(
                oauth_providers::SCIOTTE_GARMIN,
                SESSION,
                MANUAL,
                5,
                false,
                ConnectOrder::SignInFirst
            ),
            None,
            "bug: the sign-in-first order still asks a usr/pwd-only athlete to move"
        );
    }
}
