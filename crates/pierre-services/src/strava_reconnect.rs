// ABOUTME: What a Strava reconnect owes the grants it replaces: the token lands only over the row it read
// ABOUTME: A move to another app revokes the old grants once the new token is stored; a lost store revokes its own

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The Strava half of an OAuth callback's store.
//!
//! Strava counts an athlete against every application that holds their
//! grant, and the token row is one per user, tenant and provider. A reconnect
//! that lands on another app than the one that issued a stored token would
//! leave the old grant authorized at Strava with nothing here to revoke it by,
//! so the callback reads the tokens it replaces before the code exchange and
//! settles them after:
//!
//! - The new token is stored only over the row that was read
//!   ([`StorePrecondition`]). Two reconnects racing for one athlete cannot
//!   both land: the second store finds a row it did not read and refuses,
//!   and nothing is overwritten.
//! - Once the new token is stored, every grant it supersedes on another app
//!   is revoked under the client that issued it
//!   ([`ReplacedGrants::revoke_superseded`]): the token it replaced in this
//!   tenant, and the athlete's tokens in other tenants whose grant is dead
//!   by the seat counts' rule (one Strava may still count). A live grant in another tenant is left alone, and so is every
//!   token on its app: Strava holds one grant per athlete and app, so
//!   revoking through a dead token on that app would withdraw the live one
//!   too. The authorize path keeps a seat-holding athlete on its app, so a
//!   live grant exists on another app only while that app is being drained.
//! - A store that did not land leaves a fresh grant at Strava that nothing
//!   stores, which [`ReplacedGrants::abandon`] revokes unless the token now
//!   stored is on the same app, since that would be the same grant, or the
//!   athlete holds a live grant on that app in another tenant.
//! - When the athlete's tokens cannot be listed, whether a grant on some app
//!   is live in another tenant is unknown, so nothing is revoked: a grant left
//!   authorized costs a seat until an operator withdraws it, and revoking a
//!   live one disconnects the athlete wherever it is stored.
//!
//! Each revocation's outcome is logged. One Strava did not confirm for a grant
//! that was live here (by the seat counts' rule, so a `needs_reauth` over our
//! own client credentials included) is logged at ERROR, the level the alert
//! policy watches: the athlete then counts against two apps until an operator
//! withdraws it. A dead grant here most often died at Strava too, so its
//! refused revocation is a WARN.

use std::collections::{BTreeMap, BTreeSet};

use pierre_auth::oauth2_client::OAuth2Token;
use pierre_auth::strava_pool::{same_strava_app, strava_client_id};
use pierre_core::constants::oauth_providers;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{StravaTokenApp, TenantId, UserOAuthToken};
use pierre_database::backends::OAuthTokenRepository;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::oauth_flow::OAuthService;
use crate::provider_revocation::{revoke_stored_grant, RevocationOutcome};

/// Whether the athlete holds a live grant on one app in another tenant, as
/// far as the listing of their tokens says.
enum LiveElsewhere<'a> {
    /// No live grant on the app elsewhere.
    Absent,
    /// The athlete's live grant in this other tenant is on the app.
    In(&'a str),
    /// The listing failed, so a live grant on the app may exist.
    Unlisted,
}

/// What the store of a callback's token must find in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorePrecondition {
    /// Nothing is checked: a provider whose tokens name no app, or a replaced
    /// row that could not be read.
    Unconditional,
    /// The user holds no token for the tenant and provider.
    Absent,
    /// The stored row is still the one with this `id`.
    Row(String),
}

/// Store a callback's `token` over the row `precondition` names.
///
/// # Errors
/// Returns a database error when the write fails, and `ResourceLocked` when
/// another connect's token landed first: nothing was written then, and the
/// token that stands is the other one.
pub async fn store_guarded(
    tokens: &dyn OAuthTokenRepository,
    token: &UserOAuthToken,
    precondition: StorePrecondition,
) -> AppResult<()> {
    let stored = match precondition {
        StorePrecondition::Unconditional => tokens.upsert_token(token).await.map(|()| true),
        StorePrecondition::Absent => tokens.replace_token_if_current(token, None).await,
        StorePrecondition::Row(id) => tokens.replace_token_if_current(token, Some(&id)).await,
    }
    .map_err(|e| AppError::database(format!("Failed to upsert OAuth token: {e}")))?;
    if stored {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::ResourceLocked,
        format!(
            "Another {} connection for this account finished first; it stands",
            token.provider
        ),
    ))
}

/// A token a reconnect may supersede, and whether its grant was live.
#[derive(Debug)]
struct Superseded {
    token: UserOAuthToken,
    /// The token's grant is live by the seat counts' rule: one Strava still
    /// counts.
    live: bool,
}

/// The Strava tokens a callback's new token replaces, read before the code
/// exchange.
#[derive(Debug, Default)]
pub struct ReplacedGrants {
    /// The guard the store takes; `None` for a provider other than Strava.
    precondition: Option<StorePrecondition>,
    /// The token in the tenant the new one lands in, then the athlete's
    /// tokens in other tenants whose grant is dead.
    superseded: Vec<Superseded>,
    /// The app (by `client_id`) of each live grant the athlete holds in
    /// another tenant, with that tenant: none of these apps is revoked.
    /// `None` when the athlete's tokens could not be listed: then no app is
    /// known to be free of one, and none is revoked.
    live_elsewhere: Option<BTreeMap<Option<String>, String>>,
}

impl ReplacedGrants {
    /// Read what a Strava token stored under `tenant_id` would replace.
    /// Nothing for any other provider.
    pub async fn read(
        service: &OAuthService,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> Self {
        if !provider.eq_ignore_ascii_case(oauth_providers::STRAVA) {
            return Self::default();
        }
        let tokens = service.data.repos().oauth_tokens.as_ref();
        let apps = match tokens.list_strava_token_apps(user_id, tenant_id).await {
            Ok(apps) => apps,
            Err(e) => {
                warn!(user_id = %user_id, error = %e, "Could not list the athlete's Strava tokens; no grant this connect supersedes is revoked, since any may share its app with a live grant in another tenant");
                let (precondition, _) = read_replaced(tokens, user_id, tenant_id, true).await;
                return Self {
                    precondition: Some(precondition),
                    superseded: Vec::new(),
                    live_elsewhere: None,
                };
            }
        };
        let landing = tenant_id.to_string();
        let live_here = apps
            .iter()
            .find(|app| app.tenant_id == landing)
            .is_none_or(|app| app.grant_live);

        let (precondition, replaced) = read_replaced(tokens, user_id, tenant_id, live_here).await;
        let (dead_elsewhere, live_elsewhere) =
            read_elsewhere(tokens, user_id, &landing, &apps).await;
        Self {
            precondition: Some(precondition),
            superseded: replaced.into_iter().chain(dead_elsewhere).collect(),
            live_elsewhere: Some(live_elsewhere),
        }
    }

    /// Whether the athlete holds a live grant in another tenant on the app
    /// `client_id` names, as [`strava_client_id`] keys it.
    fn live_grant_elsewhere(&self, client_id: Option<&str>) -> LiveElsewhere<'_> {
        let Some(live) = &self.live_elsewhere else {
            return LiveElsewhere::Unlisted;
        };
        live.get(&client_id.map(str::to_owned))
            .map_or(LiveElsewhere::Absent, |tenant| LiveElsewhere::In(tenant))
    }

    /// Whether the fresh grant of a token that was not stored, issued by
    /// `attribution`'s app, must stay authorized: the athlete's live grant in
    /// another tenant is on that app (Strava holds one grant per athlete and
    /// app, so it is that grant), or the listing could not say. Logged either
    /// way; the unknown case at ERROR, since the athlete may then count on an
    /// app nothing here stores.
    fn keeps_fresh_grant(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        attribution: Option<&str>,
    ) -> bool {
        let app = attribution.unwrap_or("env");
        match self.live_grant_elsewhere(strava_client_id(attribution).as_deref()) {
            LiveElsewhere::Absent => false,
            LiveElsewhere::Unlisted => {
                error!(
                    user_id = %user_id,
                    tenant_id = %tenant_id,
                    app,
                    "A Strava token was not stored and the athlete's tokens could not be listed; its fresh grant is left authorized at Strava, since it may be a live grant another tenant holds"
                );
                true
            }
            LiveElsewhere::In(live_in) => {
                info!(
                    user_id = %user_id,
                    tenant_id = %tenant_id,
                    live_in = %live_in,
                    app,
                    "A Strava token was not stored; its fresh grant is the athlete's live grant in another tenant, and stays authorized"
                );
                true
            }
        }
    }

    /// The guard the store takes.
    #[must_use]
    pub fn precondition(&self) -> StorePrecondition {
        self.precondition
            .clone()
            .unwrap_or(StorePrecondition::Unconditional)
    }

    /// Revoke every grant the stored token supersedes on another app: the
    /// token it replaced in its tenant, and the athlete's dead tokens in
    /// other tenants. Each app's grant is revoked once, and none on an app the
    /// athlete holds a live grant on in another tenant: Strava keeps one grant
    /// per athlete and app, so the revocation would withdraw that one too.
    pub async fn revoke_superseded(&self, service: &OAuthService, new_attribution: Option<&str>) {
        let mut revoked: BTreeSet<Option<String>> = BTreeSet::new();
        for Superseded { token, live } in &self.superseded {
            let Some(tenant_id) = self.revocation_target(token, new_attribution, &mut revoked)
            else {
                continue;
            };
            let app = token.oauth_app_client_id.as_deref();
            info!(
                user_id = %token.user_id,
                tenant_id = %tenant_id,
                from_app = app.unwrap_or("env"),
                to_app = new_attribution.unwrap_or("env"),
                "Reconnect moved the athlete to another Strava app; revoking the previous app's grant"
            );
            let outcome = revoke_stored_grant(service, token.clone(), tenant_id).await;
            report(
                &outcome,
                token.user_id,
                app,
                *live,
                "a grant the reconnect superseded",
            );
        }
    }

    /// The tenant to revoke a superseded `token`'s grant under, or `None`
    /// when it is left as it is: it is on the new token's app, its app's grant
    /// was already revoked (`revoked`, which this extends), the athlete holds
    /// a live grant on its app in another tenant, or its tenant id is invalid.
    fn revocation_target(
        &self,
        token: &UserOAuthToken,
        new_attribution: Option<&str>,
        revoked: &mut BTreeSet<Option<String>>,
    ) -> Option<TenantId> {
        let app = token.oauth_app_client_id.as_deref();
        let client_id = strava_client_id(app);
        if same_strava_app(app, new_attribution) || !revoked.insert(client_id.clone()) {
            return None;
        }
        let live_in = match self.live_grant_elsewhere(client_id.as_deref()) {
            LiveElsewhere::Absent => None,
            LiveElsewhere::In(tenant) => Some(tenant),
            LiveElsewhere::Unlisted => Some("unknown: the athlete's tokens could not be listed"),
        };
        if let Some(live_in) = live_in {
            warn!(
                user_id = %token.user_id,
                tenant_id = %token.tenant_id,
                live_in = %live_in,
                from_app = app.unwrap_or("env"),
                to_app = new_attribution.unwrap_or("env"),
                "A superseded Strava grant is left authorized: the athlete's live grant in another tenant may be on the same app, and revoking one withdraws both; the athlete counts on both apps until it is withdrawn"
            );
            return None;
        }
        TenantId::parse_str(&token.tenant_id)
            .inspect_err(|_| {
                warn!(tenant_id = %token.tenant_id, "Stored Strava token carries an invalid tenant id");
            })
            .ok()
    }

    /// Settle a callback that failed at or after the store.
    ///
    /// When the token stored now is on the new token's app (the store landed
    /// and a later step failed, or the row it would have replaced shares its
    /// grant), the grants it supersedes are settled as after a clean store.
    /// Otherwise the exchange issued a fresh grant nothing stores, and it is
    /// revoked, unless the athlete's live grant in another tenant is on the
    /// same app (Strava holds one grant per athlete and app, so it is that
    /// grant) or the listing could not say.
    pub async fn abandon(
        &self,
        service: &OAuthService,
        user_id: Uuid,
        tenant_id: TenantId,
        token: &OAuth2Token,
        attribution: Option<&str>,
    ) {
        if self.precondition.is_none() {
            return;
        }
        match service
            .data
            .repos()
            .oauth_tokens
            .get_token(user_id, tenant_id, oauth_providers::STRAVA)
            .await
        {
            Ok(Some(stored))
                if same_strava_app(stored.oauth_app_client_id.as_deref(), attribution) =>
            {
                self.revoke_superseded(service, attribution).await;
                return;
            }
            Ok(_) => {}
            Err(e) => {
                error!(
                    user_id = %user_id,
                    tenant_id = %tenant_id,
                    error = %e,
                    "A Strava token was not stored and the stored one cannot be read; the fresh grant is left authorized at Strava"
                );
                return;
            }
        }
        if self.keeps_fresh_grant(user_id, tenant_id, attribution) {
            return;
        }
        let row = UserOAuthToken::new(
            user_id,
            tenant_id.to_string(),
            oauth_providers::STRAVA.to_owned(),
            token.access_token.clone(),
            token.refresh_token.clone(),
            token.expires_at,
            token.scope.clone(),
        )
        .with_oauth_app_client_id(attribution.map(str::to_owned));
        let outcome = revoke_stored_grant(service, row, tenant_id).await;
        report(
            &outcome,
            user_id,
            attribution,
            true,
            "the grant of a token that was not stored",
        );
    }
}

/// The token stored in the tenant a reconnect lands in, as a grant it
/// supersedes (`live` as the listing read it), and the guard its store takes.
async fn read_replaced(
    tokens: &dyn OAuthTokenRepository,
    user_id: Uuid,
    tenant_id: TenantId,
    live: bool,
) -> (StorePrecondition, Option<Superseded>) {
    match tokens
        .get_token(user_id, tenant_id, oauth_providers::STRAVA)
        .await
    {
        Ok(Some(token)) => (
            StorePrecondition::Row(token.id.clone()),
            Some(Superseded { token, live }),
        ),
        Ok(None) => (StorePrecondition::Absent, None),
        Err(e) => {
            error!(
                user_id = %user_id,
                tenant_id = %tenant_id,
                error = %e,
                "The Strava token a reconnect replaces cannot be read; it is overwritten unguarded and, if another app issued it, its grant stays authorized at Strava"
            );
            (StorePrecondition::Unconditional, None)
        }
    }
}

/// The athlete's Strava tokens in tenants other than `landing`, split by the
/// listing's liveness: the dead ones, which a move to another app revokes,
/// and the app (by `client_id`) of each live one, with its tenant, which it
/// leaves alone.
async fn read_elsewhere(
    tokens: &dyn OAuthTokenRepository,
    user_id: Uuid,
    landing: &str,
    apps: &[StravaTokenApp],
) -> (Vec<Superseded>, BTreeMap<Option<String>, String>) {
    let mut dead = Vec::new();
    let mut live = BTreeMap::new();
    for app in apps.iter().filter(|app| app.tenant_id != landing) {
        if app.grant_live {
            live.entry(strava_client_id(app.attribution.as_deref()))
                .or_insert_with(|| app.tenant_id.clone());
            continue;
        }
        let Ok(other) = TenantId::parse_str(&app.tenant_id) else {
            warn!(tenant_id = %app.tenant_id, "Stored Strava token carries an invalid tenant id");
            continue;
        };
        match tokens
            .get_token(user_id, other, oauth_providers::STRAVA)
            .await
        {
            Ok(Some(token)) => dead.push(Superseded { token, live: false }),
            Ok(None) => {}
            Err(e) => warn!(
                user_id = %user_id,
                tenant_id = %other,
                error = %e,
                "Could not read a dead Strava token in another tenant; its grant is left as it is"
            ),
        }
    }
    (dead, live)
}

/// Log one revocation's outcome: one Strava did not confirm for a live grant
/// at ERROR, for a dead one at WARN.
fn report(outcome: &RevocationOutcome, user_id: Uuid, app: Option<&str>, live: bool, what: &str) {
    let app = app.unwrap_or("env");
    let RevocationOutcome::Unconfirmed(reason) = outcome else {
        info!(user_id = %user_id, app, outcome = ?outcome, "Settled {what} at Strava");
        return;
    };
    if live {
        error!(
            user_id = %user_id,
            app,
            reason = %reason,
            "Strava did not confirm revoking {what}; the athlete counts against two apps until it is withdrawn"
        );
    } else {
        warn!(
            user_id = %user_id,
            app,
            reason = %reason,
            "Strava did not confirm revoking {what}, already dead here; it most likely died at Strava too"
        );
    }
}
