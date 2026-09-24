// ABOUTME: Operator removal of a user, and of one provider grant, through the disconnect chokepoint
// ABOUTME: Every grant is revoked at the provider before the account goes; group and operator rows refuse it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Removing a user completely, from an operator surface.
//!
//! A user delete used to be `DELETE FROM users` and whatever the foreign keys
//! did with it. The token rows cascaded, so our own seat count dropped, but the
//! grant at the provider was never withdrawn: the athlete stayed authorized on
//! the Strava application and kept counting against Strava's athlete cap. And a
//! user referenced by a row that does not cascade — a group they own, an invite
//! they created — failed the delete outright, surfaced as a 500.
//!
//! This is the operator half of the disconnect chokepoint
//! (`OAuthService::disconnect_provider`), and it never re-implements a
//! disconnect. The admin routes reach the chokepoint through
//! [`ProviderDisconnector`], which [`OAuthService`] implements, so an operator's
//! disconnect revokes upstream, deletes the token and connection rows, purges
//! the provider-derived cache and raises `provider.disconnected` exactly as the
//! athlete's own does. What the provider answered is reported per grant: the
//! local rows go either way, so "disconnected" never stands in for "revoked".
//!
//! The account delete then runs in one transaction that also clears every row
//! the user owns in a table no foreign key cascades to, and refuses to commit
//! while one survives. A disconnect or delete that fails comes back as an
//! [`Interruption`] naming what was already done, a failed disconnect's own
//! grant included, since it may have been revoked before the failure.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, UserReference};
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver;
use serde::Serialize;
use tracing::{info, warn};
use uuid::Uuid;

use crate::oauth_flow::OAuthService;
use crate::provider_revocation::{DisconnectReason, RevocationOutcome};

/// How many blocking references a refusal names before summarising the rest.
const BLOCKERS_NAMED: usize = 10;

/// The disconnect chokepoint, as an operator surface reaches it.
///
/// The admin route crate holds neither the `DataContext` nor the server config
/// the chokepoint needs, so the composition root injects this seam instead —
/// the same shape as the approval notifier beside it.
#[async_trait]
pub trait ProviderDisconnector: Send + Sync {
    /// Disconnect `provider` for `user_id` within `tenant_id`, exactly as the
    /// user's own disconnect does: the grant is revoked at the provider, the
    /// token and connection rows (for both halves of a coalesced pair) are
    /// deleted, the provider-derived cache is purged, and success is refused
    /// while any row survives. `reason` names who asked on the
    /// `provider.disconnected` event. Returns what the provider said about
    /// the grant, since the local deletion never waits on it.
    async fn disconnect(
        &self,
        user_id: Uuid,
        provider: &str,
        tenant_id: TenantId,
        reason: DisconnectReason,
    ) -> AppResult<RevocationOutcome>;

    /// Whether this server can disconnect `provider` at all. A provider this
    /// build does not register has no revocation path here; the account
    /// delete clears its token and connection rows itself.
    fn supports(&self, provider: &str) -> bool;
}

#[async_trait]
impl ProviderDisconnector for OAuthService {
    async fn disconnect(
        &self,
        user_id: Uuid,
        provider: &str,
        tenant_id: TenantId,
        reason: DisconnectReason,
    ) -> AppResult<RevocationOutcome> {
        self.disconnect_provider(user_id, provider, Some(tenant_id.as_uuid()), reason)
            .await
    }

    fn supports(&self, provider: &str) -> bool {
        self.data.provider_registry().is_supported(provider)
    }
}

/// One provider a user holds in one tenant, named as the athlete sees it
/// (`strava` for a `sciotte` mirror row), since that is the name the
/// disconnect chokepoint clears a whole coalesced pair by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HeldProvider {
    /// Tenant the token or connection row lives under.
    pub tenant_id: TenantId,
    /// User-facing provider name.
    pub provider: String,
}

impl HeldProvider {
    /// `strava (tenant …)`, as an operator reads it.
    #[must_use]
    pub fn describe(&self) -> String {
        format!("{} (tenant {})", self.provider, self.tenant_id)
    }
}

/// One provider disconnected through the chokepoint, and what the provider
/// said about its grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DisconnectedProvider {
    /// Tenant the token or connection row lived under.
    pub tenant_id: TenantId,
    /// User-facing provider name.
    pub provider: String,
    /// Whether the provider confirmed the revocation. Anything but `revoked`
    /// means the grant may still be authorized there, which an operator
    /// freeing a seat needs to know.
    pub revocation: RevocationOutcome,
}

impl DisconnectedProvider {
    /// The provider and tenant with the revocation's outcome, as an operator
    /// reads it.
    #[must_use]
    pub fn describe(&self) -> String {
        let held = format!("{} (tenant {})", self.provider, self.tenant_id);
        match &self.revocation {
            RevocationOutcome::Revoked => format!("{held} revoked at the provider"),
            RevocationOutcome::NoGrant => format!("{held} held no grant at the provider"),
            RevocationOutcome::Unconfirmed(reason) => format!(
                "{held} disconnected here, but the provider did not confirm the revocation ({reason}); the grant may still be authorized there"
            ),
        }
    }
}

/// Everything a user delete did, for the operator's response.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RemovalReport {
    /// Providers disconnected through the chokepoint, each with what the
    /// provider said about its grant.
    pub disconnected: Vec<DisconnectedProvider>,
    /// Providers this build cannot disconnect: nothing was revoked upstream
    /// for them, and the account delete cleared their token and connection
    /// rows itself.
    pub not_revocable: Vec<HeldProvider>,
    /// Coaching-group membership rows deleted.
    pub memberships_removed: u64,
    /// Every row the account delete removed itself, by table: the tables no
    /// foreign key cascades to on both engines.
    pub rows_removed: BTreeMap<String, u64>,
}

/// A removal that failed part-way.
///
/// A disconnect the chokepoint accepted runs its revocation before the local
/// deletes, so one that fails may already have revoked its grant at the
/// provider: any failed disconnect is one of these, never a plain error that
/// would read as "nothing happened". A provider this build cannot disconnect
/// is refused before the chokepoint is called, so no failure here is one it
/// turned away up front.
#[derive(Debug, Clone)]
pub struct Interruption {
    /// Providers disconnected before the failure; their grants stay withdrawn.
    pub disconnected: Vec<DisconnectedProvider>,
    /// The provider whose disconnect failed; `None` when every disconnect
    /// completed and the account delete itself failed (it rolled back whole).
    pub failed: Option<HeldProvider>,
    /// The failing step's error.
    pub error: AppError,
}

impl Interruption {
    /// What already happened and what did not, as an operator reads it;
    /// `cause` is the failure as the caller may show it.
    #[must_use]
    pub fn describe(&self, cause: &str) -> String {
        let done = if self.disconnected.is_empty() {
            if self.failed.is_some() {
                "No other provider was disconnected".to_owned()
            } else {
                "No provider was disconnected".to_owned()
            }
        } else {
            let each: Vec<String> = self
                .disconnected
                .iter()
                .map(DisconnectedProvider::describe)
                .collect();
            format!("Already disconnected: {}", each.join("; "))
        };
        self.failed.as_ref().map_or_else(
            || {
                format!(
                    "The account delete failed: {cause}. {done}. The account, its memberships and its other rows were left in place."
                )
            },
            |failed| {
                format!(
                    "Disconnecting {} failed: {cause}. Its grant may already have been revoked at the provider. {done}. The account was left in place.",
                    failed.describe()
                )
            },
        )
    }
}

/// The outcome of a removal attempt that reached the database.
#[derive(Debug, Clone)]
pub enum UserRemoval {
    /// Nothing was touched: these rows reference the user without cascading
    /// and an operator must reassign them first.
    Blocked(Vec<UserReference>),
    /// The user is gone, every supported grant withdrawn first.
    Removed(RemovalReport),
    /// A disconnect or the account delete failed; what was done first stands.
    Interrupted(Interruption),
}

/// The outcome of disconnecting one provider in every tenant that holds it.
#[derive(Debug, Clone)]
pub enum ProviderDisconnection {
    /// Every tenant's grant was disconnected; empty when the user held none.
    Disconnected(Vec<DisconnectedProvider>),
    /// This build cannot disconnect the provider (its registry does not carry
    /// it), so nothing was called and nothing changed; these are the rows it
    /// holds. Only the account delete clears them.
    NotRevocable(Vec<HeldProvider>),
    /// A tenant's disconnect failed, after whichever were disconnected first.
    Interrupted(Interruption),
}

/// Disconnect each target in turn through the chokepoint. The first failure
/// stops the walk and comes back as an [`Interruption`] naming what was
/// disconnected before it, since the failed one may have revoked upstream.
async fn disconnect_each(
    disconnector: &dyn ProviderDisconnector,
    user_id: Uuid,
    targets: Vec<HeldProvider>,
) -> Result<Vec<DisconnectedProvider>, Box<Interruption>> {
    let mut disconnected = Vec::with_capacity(targets.len());
    for target in targets {
        match disconnector
            .disconnect(
                user_id,
                &target.provider,
                target.tenant_id,
                DisconnectReason::Operator,
            )
            .await
        {
            Ok(revocation) => {
                info!(
                    user_id = %user_id,
                    tenant_id = %target.tenant_id,
                    provider = %target.provider,
                    revocation = ?revocation,
                    "Operator disconnected a user's provider"
                );
                disconnected.push(DisconnectedProvider {
                    tenant_id: target.tenant_id,
                    provider: target.provider,
                    revocation,
                });
            }
            Err(error) => {
                return Err(Box::new(Interruption {
                    disconnected,
                    failed: Some(target),
                    error,
                }))
            }
        }
    }
    Ok(disconnected)
}

/// Every provider a user holds, across every tenant, from both sources of
/// truth: the token rows (read without decrypting) and the connection rows.
///
/// Either can exist without the other — an orphaned connection, or a token
/// registered before connections were — and a disconnect clears both, so both
/// are enumerated. Named by user-facing provider and deduplicated, sorted by
/// tenant then provider.
///
/// # Errors
///
/// Returns a database error when either listing fails, or an internal error
/// when a stored `tenant_id` is not a tenant id.
pub async fn held_providers(
    repos: &RepositoryRegistry,
    user_id: Uuid,
) -> AppResult<Vec<HeldProvider>> {
    let mut held: BTreeSet<(String, String)> = BTreeSet::new();
    for (tenant_id, backend) in repos.oauth_tokens.list_token_providers(user_id).await? {
        held.insert((
            tenant_id,
            backend_resolver::user_facing_name(&backend).to_owned(),
        ));
    }
    for connection in repos
        .provider_connections
        .get_for_user(user_id, None)
        .await?
    {
        held.insert((
            connection.tenant_id,
            backend_resolver::user_facing_name(&connection.provider).to_owned(),
        ));
    }

    held.into_iter()
        .map(|(tenant_id, provider)| {
            let tenant_id = TenantId::parse_str(&tenant_id).map_err(|e| {
                AppError::internal(format!(
                    "Stored provider row carries an invalid tenant_id {tenant_id}: {e}"
                ))
            })?;
            Ok(HeldProvider {
                tenant_id,
                provider,
            })
        })
        .collect()
}

/// Disconnect one provider for a user in every tenant `targets` names,
/// through the chokepoint, and report each grant's revocation.
///
/// `targets` comes from [`held_providers`], narrowed by the caller to the
/// provider (and the tenants it may act in). An empty `Disconnected` means
/// there was nothing to disconnect and nothing was called; the caller turns
/// that into its own refusal. A provider this build cannot disconnect is
/// `NotRevocable`, before the chokepoint is called. A failed disconnect — a
/// failed delete, or a row that survived — is `Interrupted`, naming the
/// tenants disconnected before it.
pub async fn disconnect_user_provider(
    disconnector: &dyn ProviderDisconnector,
    user_id: Uuid,
    targets: Vec<HeldProvider>,
) -> ProviderDisconnection {
    if targets
        .iter()
        .any(|target| !disconnector.supports(&target.provider))
    {
        return ProviderDisconnection::NotRevocable(targets);
    }
    match disconnect_each(disconnector, user_id, targets).await {
        Ok(disconnected) => ProviderDisconnection::Disconnected(disconnected),
        Err(interruption) => ProviderDisconnection::Interrupted(*interruption),
    }
}

/// The held providers that are `provider` by its user-facing name.
#[must_use]
pub fn providers_named(held: Vec<HeldProvider>, provider: &str) -> Vec<HeldProvider> {
    let wanted = backend_resolver::user_facing_name(provider);
    held.into_iter().filter(|h| h.provider == wanted).collect()
}

/// Remove a user completely.
///
/// In order: refuse while any row references the user without cascading
/// (nothing touched), disconnect every held provider through the chokepoint
/// so each grant is revoked at the provider, then delete the account in one
/// transaction that clears the user's memberships and every other row no
/// foreign key cascades to, and proves none survived before committing. A
/// row written between the blocker read and the delete fails the delete with
/// `ErrorCode::ResourceLocked`, after the grants were already withdrawn,
/// which the [`Interruption`] reports.
///
/// # Errors
///
/// Returns a database error from the reads, and a `ResourceUnavailable`
/// error when the user holds providers and no disconnector is wired
/// (deleting would strand the grants upstream); nothing was touched then. A
/// failed disconnect or account delete is `UserRemoval::Interrupted`, not an
/// error, since a disconnect may have revoked its grant before failing.
pub async fn remove_user(
    repos: &RepositoryRegistry,
    disconnector: Option<&dyn ProviderDisconnector>,
    user_id: Uuid,
) -> AppResult<UserRemoval> {
    let blockers = repos.users.deletion_blockers(user_id).await?;
    if !blockers.is_empty() {
        return Ok(UserRemoval::Blocked(blockers));
    }

    let held = held_providers(repos, user_id).await?;
    let mut report = RemovalReport::default();
    if !held.is_empty() {
        let Some(disconnector) = disconnector else {
            return Err(AppError::resource_unavailable(
                "Provider disconnect is not wired on this server; deleting would leave the user's grants authorized at the provider",
            ));
        };
        let (revocable, not_revocable): (Vec<HeldProvider>, Vec<HeldProvider>) = held
            .into_iter()
            .partition(|target| disconnector.supports(&target.provider));
        for target in &not_revocable {
            warn!(
                user_id = %user_id,
                tenant_id = %target.tenant_id,
                provider = %target.provider,
                "Provider not registered in this build; the account delete clears its rows, nothing revoked upstream"
            );
        }
        report.not_revocable = not_revocable;
        report.disconnected = match disconnect_each(disconnector, user_id, revocable).await {
            Ok(disconnected) => disconnected,
            Err(interruption) => return Ok(UserRemoval::Interrupted(*interruption)),
        };
    }

    let deletion = match repos.users.delete(user_id).await {
        Ok(deletion) => deletion,
        Err(error) => {
            return Ok(UserRemoval::Interrupted(Interruption {
                disconnected: report.disconnected,
                failed: None,
                error,
            }))
        }
    };
    report.memberships_removed = deletion.removed_from("coaching_group_members");
    report.rows_removed = deletion.rows_removed;

    info!(
        user_id = %user_id,
        disconnected = report.disconnected.len(),
        not_revocable = report.not_revocable.len(),
        memberships_removed = report.memberships_removed,
        tables_cleared = report.rows_removed.len(),
        "User removed completely"
    );
    Ok(UserRemoval::Removed(report))
}

/// The operator-facing refusal naming each blocking reference, capped at
/// [`BLOCKERS_NAMED`] with a count of the rest.
#[must_use]
pub fn blockers_message(blockers: &[UserReference]) -> String {
    let mut named: Vec<String> = blockers
        .iter()
        .take(BLOCKERS_NAMED)
        .map(UserReference::describe)
        .collect();
    if blockers.len() > BLOCKERS_NAMED {
        named.push(format!("and {} more", blockers.len() - BLOCKERS_NAMED));
    }
    format!(
        "User cannot be deleted until these are reassigned or removed: {}",
        named.join("; ")
    )
}
