// ABOUTME: Removal of a user, and of one provider grant: every grant revoked upstream, then one transaction
// ABOUTME: The account, every provider's data, the tombstones and a chained receipt commit together or not at all

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
//! every row the provider contributed and raises `provider.disconnected`
//! exactly as the athlete's own does. What the provider answered is reported
//! per grant: the local rows go either way, so "disconnected" never stands in
//! for "revoked".
//!
//! A whole-account removal ([`remove_user`]) withdraws every grant at the
//! provider first ([`ProviderDisconnector::withdraw_upstream`]: network calls,
//! best-effort, each outcome recorded), then deletes everything in ONE
//! database transaction (`AccountDeletionRepository::delete_account`): every
//! provider's data, every row the user owns in a table no foreign key
//! cascades to, the account row, a tombstone per provider-side id and for
//! the email, and a hash-chained deletion receipt. It refuses to commit while
//! a purged table still holds a row. A delete that fails rolls all of that
//! back and comes back as an [`Interruption`], since the grants were already
//! withdrawn upstream. After the commit, the user's cache entries and their
//! cached email are evicted, and each provider raises `provider.disconnected`.
//!
//! A single-provider disconnect ([`disconnect_user_provider`]) still goes
//! through the chokepoint, and a failed one comes back as an [`Interruption`]
//! naming what was already done, its own grant included, since it may have
//! been revoked before the failure.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use chrono::Utc;
use pierre_auth::firebase_identity::{FirebaseIdentityDeleter, FirebaseIdentityRemoval};
use pierre_cache::Cache;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, UserReference};
use pierre_database::repositories::{
    AccountDeletion, AccountDeletionPlan, DeletionActor, PlannedBackend,
};
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver;
use serde::Serialize;
use serde_json::json;
use tracing::{info, warn};
use uuid::Uuid;

use crate::analytics::evict_user_email;
use crate::firebase_identity_outbox;
use crate::oauth_flow::OAuthService;
use crate::provider_revocation::{self, DisconnectReason, RevocationOutcome};

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
    /// deleted, every row the provider contributed is purged, and success is
    /// refused while any row survives. `reason` names who asked on the
    /// `provider.disconnected` event. Returns what the provider said about
    /// the grant, since the local deletion never waits on it.
    async fn disconnect(
        &self,
        user_id: Uuid,
        provider: &str,
        tenant_id: TenantId,
        reason: DisconnectReason,
    ) -> AppResult<RevocationOutcome>;

    /// Withdraw every grant `provider` holds for `user_id` in `tenant_id` at
    /// the provider (the revocation, and the scrape session's drop), deleting
    /// nothing here: the account deletion deletes the rows afterwards, in one
    /// transaction. Best-effort, so never an error: returns what the provider
    /// said.
    async fn withdraw_upstream(
        &self,
        user_id: Uuid,
        provider: &str,
        tenant_id: TenantId,
    ) -> RevocationOutcome;

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

    async fn withdraw_upstream(
        &self,
        user_id: Uuid,
        provider: &str,
        tenant_id: TenantId,
    ) -> RevocationOutcome {
        provider_revocation::withdraw_upstream(self, user_id, tenant_id, provider).await
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
    /// What became of the account's Firebase identity at Google, deleted
    /// after the account itself.
    pub firebase_identity: FirebaseIdentityRemoval,
    /// The receipt and tombstones the deletion committed, and what the
    /// post-commit evictions did.
    pub ledger: DeletionLedger,
}

/// What an account deletion left on the record, and the evictions after it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DeletionLedger {
    /// The deletion receipt's id.
    pub receipt_id: String,
    /// Its position in the receipt chain.
    pub receipt_seq: i64,
    /// Its hash, the chain's new head.
    pub receipt_hash: String,
    /// The tombstones left live: one per provider-side id and the email.
    pub tombstone_ids: Vec<String>,
    /// The copies the deletion could not delete, as the receipt records them.
    pub not_deleted: Vec<String>,
    /// Cache entries removed after the commit; `None` when no cache was
    /// given or it could not be reached (logged).
    pub cache_entries_removed: Option<u64>,
    /// Whether this process held the account's email for notify events and
    /// evicted it.
    pub identity_cache_evicted: bool,
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
        let done = self.describe_disconnected();
        self.failed.as_ref().map_or_else(
            || {
                format!(
                    "The account delete failed: {cause}. {done}. The account, its provider data, its memberships and its other rows were left in place."
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

    /// The providers disconnected before the failure, as an operator reads
    /// them; the sentence every interrupted operator action opens its account
    /// of what was already done with.
    #[must_use]
    pub fn describe_disconnected(&self) -> String {
        if self.disconnected.is_empty() {
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
        }
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

/// Disconnect each target in turn through the chokepoint, `reason` naming who
/// asked on each `provider.disconnected` event. The first failure stops the
/// walk and comes back as an [`Interruption`] naming what was disconnected
/// before it, since the failed one may have revoked upstream.
pub(crate) async fn disconnect_each(
    disconnector: &dyn ProviderDisconnector,
    user_id: Uuid,
    targets: Vec<HeldProvider>,
    reason: DisconnectReason,
) -> Result<Vec<DisconnectedProvider>, Box<Interruption>> {
    let mut disconnected = Vec::with_capacity(targets.len());
    for target in targets {
        match disconnector
            .disconnect(user_id, &target.provider, target.tenant_id, reason)
            .await
        {
            Ok(revocation) => {
                info!(
                    user_id = %user_id,
                    tenant_id = %target.tenant_id,
                    provider = %target.provider,
                    revocation = ?revocation,
                    reason = reason.as_str(),
                    "Disconnected a provider before removing the user"
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
    match disconnect_each(disconnector, user_id, targets, DisconnectReason::Operator).await {
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

/// Remove a user completely, on an operator's request or the user's own
/// (`actor` names who asked, on the receipt and as the reason of every
/// `provider.disconnected` event).
///
/// In order: refuse while any row references the user without cascading
/// (nothing touched); withdraw every held grant at the provider
/// ([`ProviderDisconnector::withdraw_upstream`], best-effort, each outcome
/// kept); then delete in ONE transaction every provider's data, every row
/// the user owns, the account row, with a tombstone per provider-side id and
/// for the email and a chained deletion receipt, proving no purged table
/// still holds a row before it commits. A row written between the blocker
/// read and the delete fails it with `ErrorCode::ResourceLocked`, and the
/// whole transaction rolls back: the account and its provider data stay,
/// while the grants were already withdrawn upstream, which the
/// [`Interruption`] reports.
///
/// After the commit, the user's entries in `cache` (every tenant, every
/// provider) and their cached email are evicted, each disconnected provider
/// raises `provider.disconnected`, and the account's Firebase identity
/// (queued in `firebase_identity_deletions` by the transaction) is deleted
/// through `firebase`, so Firebase being unreachable can never keep or
/// resurrect an account the user asked to delete, and never loses the
/// identity either: a failure stays queued for the retry sweep
/// ([`crate::firebase_identity_outbox`]) and is reported in
/// [`RemovalReport::firebase_identity`]. `firebase` is `None` when this server
/// has no Firebase project configured; nothing is queued then.
///
/// # Errors
///
/// Returns a database error from the reads, and a `ResourceUnavailable`
/// error when the user holds providers and no disconnector is wired
/// (deleting would strand the grants upstream); nothing was touched then. A
/// failed account delete is `UserRemoval::Interrupted`, not an error, since
/// the grants were withdrawn before it.
pub async fn remove_user(
    repos: &RepositoryRegistry,
    disconnector: Option<&dyn ProviderDisconnector>,
    firebase: Option<&FirebaseIdentityDeleter>,
    cache: Option<&Cache>,
    user_id: Uuid,
    actor: DeletionActor,
) -> AppResult<UserRemoval> {
    let blockers = repos.users.deletion_blockers(user_id).await?;
    if !blockers.is_empty() {
        return Ok(UserRemoval::Blocked(blockers));
    }
    let firebase_uid = repos
        .users
        .get_global(user_id)
        .await?
        .and_then(|user| user.firebase_uid);

    let held = held_providers(repos, user_id).await?;
    let mut report = RemovalReport::default();
    if !held.is_empty() {
        let Some(disconnector) = disconnector else {
            return Err(AppError::resource_unavailable(
                "Provider disconnect is not wired on this server; deleting would leave the user's grants authorized at the provider",
            ));
        };
        let (disconnected, not_revocable) =
            withdraw_held(disconnector, user_id, &held, actor).await;
        report.disconnected = disconnected;
        report.not_revocable = not_revocable;
    }

    let firebase_project = firebase.map(|deleter| deleter.project_id().to_owned());
    let not_deleted = copies_not_deleted(&report, firebase_uid.is_some(), firebase.is_some());
    let plan = AccountDeletionPlan {
        user_id,
        actor,
        firebase_project,
        backends: planned_backends(&held),
        revocations: revocations_json(&report)?,
        not_deleted: not_deleted.clone(),
    };
    let deletion = match repos.account_deletion.delete_account(&plan).await {
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
    report.ledger = ledger_of(&deletion, not_deleted);
    report.rows_removed = deletion.rows_removed;

    evict_after_commit(cache, user_id, &mut report.ledger).await;
    announce_disconnects(&report, user_id, actor);
    if let Some(uid) = firebase_uid {
        report.firebase_identity = delete_firebase_identity(repos, firebase, user_id, &uid).await;
    }

    info!(
        user_id = %user_id,
        disconnected = report.disconnected.len(),
        not_revocable = report.not_revocable.len(),
        memberships_removed = report.memberships_removed,
        tables_cleared = report.rows_removed.len(),
        receipt_seq = report.ledger.receipt_seq,
        tombstones = report.ledger.tombstone_ids.len(),
        firebase_identity = ?report.firebase_identity,
        actor = actor.as_str(),
        "User removed completely"
    );
    Ok(UserRemoval::Removed(report))
}

/// Withdraw every grant in `held` at its provider, through `disconnector`.
///
/// Returns the providers withdrawn, each with what its provider said, and
/// those this build cannot revoke (warned, left to the account delete).
async fn withdraw_held(
    disconnector: &dyn ProviderDisconnector,
    user_id: Uuid,
    held: &[HeldProvider],
    actor: DeletionActor,
) -> (Vec<DisconnectedProvider>, Vec<HeldProvider>) {
    let (revocable, not_revocable): (Vec<HeldProvider>, Vec<HeldProvider>) = held
        .iter()
        .cloned()
        .partition(|target| disconnector.supports(&target.provider));
    for target in &not_revocable {
        warn!(
            user_id = %user_id,
            tenant_id = %target.tenant_id,
            provider = %target.provider,
            "Provider not registered in this build; the account delete clears its rows, nothing revoked upstream"
        );
    }
    let mut disconnected = Vec::with_capacity(revocable.len());
    for target in revocable {
        let revocation = disconnector
            .withdraw_upstream(user_id, &target.provider, target.tenant_id)
            .await;
        info!(
            user_id = %user_id,
            tenant_id = %target.tenant_id,
            provider = %target.provider,
            revocation = ?revocation,
            actor = actor.as_str(),
            "Withdrew a provider's grant upstream before deleting the user"
        );
        disconnected.push(DisconnectedProvider {
            tenant_id: target.tenant_id,
            provider: target.provider,
            revocation,
        });
    }
    (disconnected, not_revocable)
}

/// The reason each `provider.disconnected` event of an account deletion
/// carries: an athlete deleting from the app or a channel is the athlete's.
const fn disconnect_reason(actor: DeletionActor) -> DisconnectReason {
    match actor {
        DeletionActor::Operator => DisconnectReason::Operator,
        DeletionActor::Athlete | DeletionActor::Messaging => DisconnectReason::Athlete,
    }
}

/// Every backend whose data the deletion purges: both halves of each held
/// provider's coalesced pair, in the tenant holding it.
fn planned_backends(held: &[HeldProvider]) -> Vec<PlannedBackend> {
    held.iter()
        .flat_map(|target| {
            backend_resolver::backend_pair_for(&target.provider)
                .into_iter()
                .map(|backend| PlannedBackend {
                    tenant_id: target.tenant_id,
                    backend,
                })
        })
        .collect()
}

/// What each provider said about its grant, as the receipt stores it.
fn revocations_json(report: &RemovalReport) -> AppResult<String> {
    let mut entries: Vec<serde_json::Value> = report
        .disconnected
        .iter()
        .map(|d| json!({ "provider": d.provider, "tenant_id": d.tenant_id, "revocation": d.revocation }))
        .collect();
    entries.extend(report.not_revocable.iter().map(|h| {
        json!({ "provider": h.provider, "tenant_id": h.tenant_id, "revocation": { "status": "not_revocable" } })
    }));
    serde_json::to_string(&entries)
        .map_err(|e| AppError::internal(format!("Failed to encode the revocations: {e}")))
}

/// The copies an account deletion cannot delete, each named with why, as its
/// receipt records them. Decided before the transaction, so it names what is
/// known then: grants the provider did not confirm withdrawn, and the copies
/// no deletion here reaches.
fn copies_not_deleted(
    report: &RemovalReport,
    has_firebase_identity: bool,
    firebase_configured: bool,
) -> Vec<String> {
    let mut copies = Vec::new();
    for d in &report.disconnected {
        if let RevocationOutcome::Unconfirmed(reason) = &d.revocation {
            copies.push(format!(
                "provider_grant:{}: the provider did not confirm the revocation ({reason}); the grant may still be authorized there",
                d.provider
            ));
        }
    }
    for h in &report.not_revocable {
        copies.push(format!(
            "provider_grant:{}: this build cannot revoke it; the grant may still be authorized at the provider",
            h.provider
        ));
    }
    if has_firebase_identity {
        copies.push(if firebase_configured {
            "firebase_identity: queued in this transaction and deleted at Google after the commit; the receipt cannot attest Google's answer".to_owned()
        } else {
            "firebase_identity: no Firebase project is configured here, so the sign-in identity at Google was left there".to_owned()
        });
    }
    copies.push(
        "posthog_person: no PostHog person-deletion call is wired; events already captured under the account's hashed id stay in PostHog".to_owned(),
    );
    // The coach's thread about the account, their flags and corrections, the
    // account's room transcript, and the rows of a coach's own group chat that
    // name the account go with it (`SQLITE_USER_PURGE`); the rows naming no
    // single member do not.
    // LIMITATION(registre#1255): `coach_group_chats` — a reply from a turn that read several members, a reply restating the group prompt's member summaries, the agent's replies copied into the room transcript, and rows written before the chat recorded which member a row is about name no member, so the deletion cannot find them.
    copies.push(
        "coach_group_chats: in a human coach's own copy of a group's chat, the rows drawn from the account alone go with it; the coach's own questions stay, and four kinds of copy name no single member and stay until removed on request: a reply from a turn that read several members, a reply restating the member summaries the coach's group prompt carries, the agent's replies to the coach as copied into the group's room transcript, and rows written before the chat recorded which member a row is about".to_owned(),
    );
    copies.push(
        "process_caches: the response cache and the notify-event email cache are evicted after the commit, the email cache on this instance only; other instances keep their copy until they restart".to_owned(),
    );
    copies.push(
        "server_logs: log lines naming the account id stay in the log store until its retention expires".to_owned(),
    );
    copies.push(
        "database_backups: backups taken before the deletion keep the account until they rotate out".to_owned(),
    );
    copies
}

/// The ledger the committed deletion left.
fn ledger_of(deletion: &AccountDeletion, not_deleted: Vec<String>) -> DeletionLedger {
    DeletionLedger {
        receipt_id: deletion.receipt.id.clone(),
        receipt_seq: deletion.receipt.seq,
        receipt_hash: deletion.receipt.hash.clone(),
        tombstone_ids: deletion.tombstone_ids.clone(),
        not_deleted,
        cache_entries_removed: None,
        identity_cache_evicted: false,
    }
}

/// Evict what this process caches about the deleted user, recording it on
/// `ledger`: every response cached under their id, and their email.
async fn evict_after_commit(cache: Option<&Cache>, user_id: Uuid, ledger: &mut DeletionLedger) {
    if let Some(cache) = cache {
        match cache.invalidate_user(user_id).await {
            Ok(removed) => ledger.cache_entries_removed = Some(removed),
            Err(e) => warn!(
                user_id = %user_id,
                error = %e,
                "Could not evict the deleted user's cache entries; they expire with their TTL"
            ),
        }
    }
    ledger.identity_cache_evicted = evict_user_email(&user_id.to_string());
}

/// Raise `provider.disconnected` for each provider the deletion withdrew,
/// as the chokepoint raises it for a single disconnect, so a deleted
/// account's providers count in the same funnel.
fn announce_disconnects(report: &RemovalReport, user_id: Uuid, actor: DeletionActor) {
    let reason = disconnect_reason(actor);
    for d in &report.disconnected {
        info!(
            target: "notify",
            event = "provider.disconnected",
            provider = %d.provider,
            user_id = %user_id,
            tenant_id = %d.tenant_id,
            reason = reason.as_str(),
            "user disconnected fitness provider"
        );
    }
}

/// Delete the Firebase identity of the just-deleted account `user_id`, the
/// first attempt on the row its delete queued, and settle that row. Never an
/// error: the account is already gone, and a row that cannot be settled
/// stays due for the sweep.
async fn delete_firebase_identity(
    repos: &RepositoryRegistry,
    firebase: Option<&FirebaseIdentityDeleter>,
    user_id: Uuid,
    firebase_uid: &str,
) -> FirebaseIdentityRemoval {
    let Some(deleter) = firebase else {
        warn!(
            user_id = %user_id,
            firebase_uid = %firebase_uid,
            "Firebase is not configured; the deleted account's Firebase identity was left at Google"
        );
        return FirebaseIdentityRemoval::NotConfigured;
    };
    let project = deleter.project_id();
    let removal = deleter.delete_user(project, firebase_uid).await;
    if let Err(error) = firebase_identity_outbox::settle(
        repos.firebase_identity_deletions.as_ref(),
        project,
        firebase_uid,
        &removal,
        0,
        Utc::now(),
    )
    .await
    {
        warn!(
            user_id = %user_id,
            firebase_uid = %firebase_uid,
            error = %error,
            "Could not settle the queued Firebase identity deletion; the sweep retries it"
        );
    }
    removal
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
