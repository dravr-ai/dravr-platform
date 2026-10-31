// ABOUTME: Adapter bridging dravr-enforme's store traits to Pierre's RepositoryRegistry
// ABOUTME: Its wellness writes go through the one wellness store as whole days; cursors and credentials stay here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use secrecy::ExposeSecret;
use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use chrono::Utc;
use dravr_enforme::error::{EnformeError, EnformeResult};
use dravr_enforme::models::connection::{ConnectedUser, ProviderCredentials};
use dravr_enforme::models::cursor::SyncCursor;
use dravr_enforme::models::deletion::DeletionPolicy;
use dravr_enforme::providers::build_provider_registry_with_reader;
use dravr_enforme::providers::sciotte_reader::DailySummaryReader;
use dravr_enforme::traits::connection_store::UserConnectionStore;
use dravr_enforme::traits::credential_store::CredentialStore;
use dravr_enforme::traits::cursor_store::SyncCursorStore;
use dravr_enforme::traits::daily_activity_store::DailyActivityStore;
use dravr_enforme::traits::data_source_store::DataSourceStore;
use dravr_enforme::traits::health_store::HealthStore;
use dravr_enforme::traits::recovery_store::RecoveryStore;
use dravr_enforme::traits::sleep_store::SleepStore;
use dravr_enforme::traits::timeseries_store::TimeSeriesPointStore;
use dravr_enforme::{RequestGate, RequestGateHandle};
use dravr_equilibre_sync::SyncStatus;
use dravr_riviere::DataPoint;
use dravr_sciotte::client::SciotteClient;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{DelegatedConnection, DelegationStatus, TenantId, UserOAuthToken};
use pierre_database::repositories::{SyncCursorRow, WorkerRunRepository};
use pierre_database::{AuthRepos, FitnessRepos, RepositoryRegistry};
use pierre_providers::backend_resolver::{pushed_backend_for, sync_backend};
use pierre_providers::credential_kind;
use pierre_providers::request_budget::RequestBudget;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::coach_consents::link_consents;
use crate::delegated_wellness::{
    excluded_from_wellness, health_the_link_files, recovery_the_link_files, sleep_the_link_files,
    DelegatedWellness, PrivateWellness,
};
use crate::garmin_push;
use crate::sciotte_health_reader::{SciotteServiceReader, SessionRows};
use crate::wellness_store::{IntradaySeries, WellnessLanded, WellnessStore, WellnessWriteMode};

/// Adapter bridging dravr-enforme's store traits to Pierre's repository layer.
///
/// Wraps narrow `FitnessRepos` + `AuthRepos` views and implements each
/// of enforme's 8 granular store traits by delegating to the
/// corresponding repository method. The constructor still takes the
/// full `RepositoryRegistry` because the adapter cross-cuts both views
/// — internal narrowing keeps the trait impls tight without forcing
/// callers to construct two separate view-structs.
///
/// `dravr-equilibre` resolves to a single workspace-wide version (the patch
/// in the workspace `Cargo.toml` redirects the crates.io alias used by
/// enforme to the same git tag pierre-core consumes), so the model types
/// flow through this adapter without any cross-version translation.
///
/// The sync knows a provider by its own name (`garmin`, `coros`), while a
/// scrape-backed connection's session lives on its mirror backend's token row
/// (`sciotte_garmin`, `sciotte_coros`). Every lookup here — the roster, the
/// credentials, the tenant a row is written under — goes through
/// [`sync_backend`], the one mapping between the two.
///
/// A provider that also pushes its data ([`pushed_backend_for`]: Garmin's
/// Developer Program API) leaves the roster every athlete who holds the
/// pushed backend in the tenant their usr/pwd session lives in: their reads
/// resolve to the API, whose pushes already write their days, so pulling them
/// through the session too would write each day twice.
///
/// Every sleep, recovery, body, data-source and series record it is handed
/// is written through the one wellness writer, [`WellnessStore`], a day as
/// [`WellnessWriteMode::FullDay`]: a sync reads the day whole, so a reading
/// the provider no longer holds is cleared. The store applies WHOOP's terms:
/// WHOOP's own scores are never kept, and no WHOOP record is kept for an
/// account that owes WHOOP's owner authorization; each store call logs how
/// many it withheld. A disconnect deletes the rest through the chokepoint's
/// provider data purge. WHOOP exposes no continuous series, so the
/// time-series store receives none of its data.
///
/// Every row it writes carries its sync time, refreshed on each re-sync, so
/// a provider whose terms cap how long a copy is held has its copies evicted
/// past that cap by the cache TTL sweep (`provider_cache_sweeper`). Athletes
/// connected to WHOOP are left out of the cross-athlete archetype priors
/// (`archetype_aggregation`).
///
/// LIMITATION(registre#539): `PierreSyncStorage` persists WHOOP-classified physiology (HRV rMSSD, sleep
/// stage durations) instead of holding it in memory within WHOOP's cache header: what that header allows
/// awaits WHOOP's answer (W1) and the lawyer's read of HRV (W8), so WHOOP declares no cache TTL. Derived
/// rows with no provider column (`training_history`, `user_facts`, playbooks) cannot be attributed to
/// WHOOP, by a purge or by the cross-athlete exclusion, which goes by current connection (carnet#769).
pub struct PierreSyncStorage {
    /// Fitness-domain stores backing enforme's sleep / recovery / health /
    /// data-source / sync-cursor / time-series trait impls.
    fitness: FitnessRepos,
    /// Auth-domain handle for the `oauth_tokens` and `delegated_connections`
    /// lookups behind `resolve_tenant_id` (enforme's contract gives us only
    /// `user_id` + `provider`, so we recover the owning tenant from the token
    /// row, or from the link a coach-linked member's wellness syncs through).
    auth: AuthRepos,
    /// Bridge to the platform OAuth refresh flow (`AuthService`), injected by
    /// pierre-server after runtime construction — the refresh path lives above
    /// this crate in the dependency graph, so it cannot be built here. Until
    /// injection, credential reads fall back to the stored token row (startup
    /// window before the first scheduled sync tick, and storage-only tests).
    refresher: OnceLock<Arc<dyn SyncCredentialRefresher>>,
    /// The scrape-session rows handed out to syncs, which a session rotated
    /// on a read is written back over ([`SessionRows`]).
    session_rows: SessionRows,
    /// The full registry, for the WHOOP owner-authorization read (the notice
    /// flag and the account's acceptance live outside both views) and for the
    /// delegated wellness pass's links and worker ledger. Shared with the
    /// server's own handle to the same pools.
    registry: Arc<RepositoryRegistry>,
}

/// Bridge to the platform's OAuth token-refresh infrastructure.
///
/// Implemented in pierre-server on top of `AuthService` (which refreshes
/// near-expiry tokens, persists the result, and maintains provider-connection
/// status). `pierre-services` sits below `pierre-tool-runtime` in the crate
/// graph, so the implementation is injected via
/// [`PierreSyncStorage::set_credential_refresher`] rather than called directly.
#[async_trait]
pub trait SyncCredentialRefresher: Send + Sync {
    /// Return valid credentials for the user+provider, refreshing through the
    /// platform OAuth flow when the stored token is expired or near expiry.
    /// `None` means no usable token exists and reconnecting is the remedy: none
    /// is stored, none can be refreshed, or the provider refused the refresh,
    /// now or earlier with the connection still flagged. An error means the
    /// lookup failed, or a refresh failed without the provider refusing it (a
    /// rate limit, a 5xx, a transport failure) over an unflagged connection:
    /// the grant stands, and a later sync can refresh it.
    async fn valid_credentials(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
    ) -> EnformeResult<Option<ProviderCredentials>>;

    /// Refresh the stored token regardless of its recorded expiry — the
    /// reactive path after the provider rejected the current token (e.g. a
    /// 401 despite a DB-valid `expires_at`). `None` and an error mean what
    /// they mean for [`Self::valid_credentials`].
    async fn force_refresh(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
    ) -> EnformeResult<Option<ProviderCredentials>>;

    /// What `member`'s `provider` wellness syncs with through their human
    /// coach's confirmed link
    /// ([`crate::delegated_wellness::delegated_wellness_credentials`]), for a
    /// member who holds no credential of their own: the coach's credentials,
    /// and what the athlete keeps private from the coach. `None` when it does
    /// not sync that way. The decision reads the coach's roster through the
    /// provider registry, the cache and the notifications only the runtime
    /// holds.
    async fn delegated_credentials(
        &self,
        member: Uuid,
        provider: &str,
    ) -> EnformeResult<Option<DelegatedWellness>>;
}

/// One sync of a member by the scheduler's delegated wellness pass.
struct DelegatedSync {
    /// The member whose wellness the pass syncs.
    member: Uuid,
    /// What the athlete keeps private from the coach, recorded when the
    /// sync is handed its credentials ([`record_private_wellness`]).
    private: Mutex<Option<PrivateWellness>>,
}

tokio::task_local! {
    /// The delegated pass's sync running on this task.
    static DELEGATED_SYNC: DelegatedSync;
}

/// Run `work`, one sync of `member` by the scheduler's delegated wellness
/// pass: the only place credentials read through a coach's link are handed
/// out ([`PierreSyncStorage::get_credentials`]). Everywhere else a member
/// with no credential of their own has none.
pub(crate) async fn delegated_sync_scope<F: Future>(member: Uuid, work: F) -> F::Output {
    let sync = DelegatedSync {
        member,
        private: Mutex::new(None),
    };
    DELEGATED_SYNC.scope(sync, work).await
}

/// Whether the running task is the delegated pass's sync of `member`.
fn in_delegated_sync_of(member: Uuid) -> bool {
    DELEGATED_SYNC
        .try_with(|sync| sync.member == member)
        .unwrap_or(false)
}

/// Record that the delegated pass's sync of `member` keeps `private` from
/// what it files. A second record within one sync keeps what either names.
fn record_private_wellness(member: Uuid, private: PrivateWellness) {
    let _ = DELEGATED_SYNC.try_with(|sync| {
        if sync.member != member {
            return;
        }
        // A poisoned record stays unrecorded, which files nothing.
        if let Ok(mut held) = sync.private.lock() {
            *held = Some(held.map_or(private, |earlier| earlier.union(private)));
        }
    });
}

/// What a row enforme hands over for `user_id` keeps private, when the
/// running task is the delegated pass's sync of that member, so the row is
/// filed through their human coach's link; `None` for every other row. The
/// pass syncs only members who hold no credential of their own for the
/// platform, so within it every row is the link's. A sync whose credentials
/// recorded nothing keeps every field, and files nothing.
fn delegated_private_of_row(user_id: &str) -> Option<PrivateWellness> {
    let member = user_id.parse::<Uuid>().ok()?;
    DELEGATED_SYNC
        .try_with(|sync| {
            (sync.member == member).then(|| {
                sync.private
                    .lock()
                    .ok()
                    .and_then(|held| *held)
                    .unwrap_or(PrivateWellness::ALL)
            })
        })
        .ok()
        .flatten()
}

impl PierreSyncStorage {
    /// Create a new adapter wrapping the given repository registry.
    ///
    /// Cross-cuts `FitnessRepos` (sleep/recovery/health/etc.) and
    /// `AuthRepos` (`oauth_tokens` for tenant recovery); narrowing
    /// happens inside the struct.
    #[must_use]
    pub fn new(repos: &Arc<RepositoryRegistry>) -> Self {
        Self {
            fitness: repos.fitness_repos(),
            auth: repos.auth_repos(),
            refresher: OnceLock::new(),
            session_rows: SessionRows::new(Arc::clone(&repos.oauth_tokens)),
            registry: Arc::clone(repos),
        }
    }

    /// Inject the OAuth refresh bridge once the server runtime exists.
    ///
    /// Idempotent: only the first injection wins (subsequent calls are
    /// ignored, mirroring `OnceLock` semantics).
    pub fn set_credential_refresher(&self, refresher: Arc<dyn SyncCredentialRefresher>) {
        if self.refresher.set(refresher).is_err() {
            debug!("credential refresher already injected; keeping the first");
        }
    }

    /// Build a `SyncOrchestrator` backed by this adapter with default configuration.
    ///
    /// Takes `&Arc<Self>` (not `self`) so the caller keeps a handle to the
    /// storage for post-construction injection of the credential refresher.
    /// The orchestrator is ready to run the scheduler or handle webhook events.
    /// The scrape-backed providers read their daily summaries on the dedicated
    /// sciotte service (ADR-021) through `sciotte`, the server's one client
    /// for it: no browser runs in this pod.
    ///
    /// # Errors
    ///
    /// When a provider's HTTP client cannot be built: enforme bounds every
    /// vendor call with timeouts and never builds an untimed client instead.
    pub fn build_orchestrator(
        self: &Arc<Self>,
        sciotte: Option<SciotteClient>,
    ) -> EnformeResult<Arc<dravr_enforme::SyncOrchestrator>> {
        let reader: Arc<dyn DailySummaryReader> = Arc::new(SciotteServiceReader::new(
            sciotte,
            self.session_rows.clone(),
        ));
        self.build_orchestrator_with_reader(&reader)
    }

    /// Build a `SyncOrchestrator` backed by this adapter whose scrape-backed
    /// providers read their daily summaries through `reader`.
    ///
    /// # Errors
    ///
    /// When a provider's HTTP client cannot be built.
    pub fn build_orchestrator_with_reader(
        self: &Arc<Self>,
        reader: &Arc<dyn DailySummaryReader>,
    ) -> EnformeResult<Arc<dravr_enforme::SyncOrchestrator>> {
        let storage = Arc::clone(self);
        let deps = Arc::new(dravr_enforme::SyncDeps {
            sleep: storage.clone(),
            recovery: storage.clone(),
            health: storage.clone(),
            daily_activity: storage.clone(),
            time_series: storage.clone(),
            data_sources: storage.clone(),
            cursors: storage.clone(),
            credentials: storage.clone(),
            connections: storage,
        });

        let config = dravr_enforme::SyncConfig::from_env();
        let providers = build_provider_registry_with_reader(reader)?;

        Ok(Arc::new(dravr_enforme::SyncOrchestrator::new(
            deps, providers, config,
        )))
    }

    /// The token row that serves `provider` for this user: the row of its
    /// [`sync_backend`], or `None` when the user holds none — or holds, in
    /// that row's tenant, the backend the provider pushes through
    /// ([`pushed_backend_for`]) with the permission that lets it push
    /// wellness ([`Self::wellness_is_pushed`]). Such a user's days arrive by
    /// push, so every pull of them (scheduled, on request, after a connect)
    /// finds nothing of their own to read, as the roster already leaves them
    /// out. A user who withheld that permission is pulled as before: nothing
    /// else serves their wellness.
    async fn serving_token(
        &self,
        user_uuid: Uuid,
        provider: &str,
    ) -> EnformeResult<Option<UserOAuthToken>> {
        let backend = sync_backend(provider);
        let tokens = self
            .auth
            .oauth_tokens
            .get_tokens(user_uuid, None)
            .await
            .map_err(|e| EnformeError::store(format!("Failed to look up user tokens: {e}")))?;
        let Some(serving) = tokens.iter().find(|t| t.provider == backend) else {
            return Ok(None);
        };
        let holds_pushed = pushed_backend_for(provider).filter(|pushed| {
            tokens
                .iter()
                .any(|t| t.provider == *pushed && t.tenant_id == serving.tenant_id)
        });
        let fed_by_push = match holds_pushed {
            Some(pushed) => {
                let tenant = TenantId::parse_str(&serving.tenant_id).map_err(|e| {
                    EnformeError::store(format!("Stored token has an invalid tenant id: {e}"))
                })?;
                self.wellness_is_pushed(user_uuid, tenant, pushed).await?
            }
            None => false,
        };
        Ok((!fed_by_push).then(|| serving.clone()))
    }

    /// Whether the `pushed` backend's connection of this user in `tenant`
    /// pushes their wellness: its last reported permissions (none reported
    /// reads as all granted) include the health export.
    async fn wellness_is_pushed(
        &self,
        user_uuid: Uuid,
        tenant: TenantId,
        pushed: &str,
    ) -> EnformeResult<bool> {
        let grants = self
            .registry
            .provider_connections
            .provider_grants(user_uuid, tenant, pushed)
            .await
            .map_err(|e| {
                EnformeError::store(format!("Failed to read the {pushed} permissions: {e}"))
            })?;
        Ok(garmin_push::shares_wellness(grants.as_deref()))
    }

    /// Resolve the tenant a user's `provider` rows are written under: the
    /// tenant of the token row that serves that provider, else — only within
    /// the delegated pass's sync of that member ([`delegated_sync_scope`]) —
    /// the tenant of the confirmed link whose member granted it their
    /// wellness.
    ///
    /// There is no fallback to another provider's token. A record is written
    /// only under the tenant of the athlete's own connection to its provider,
    /// or of the link that reads it: once a disconnect has deleted that
    /// connection and purged its rows, or the link has ended or lost the
    /// member's wellness consent, a sync still in flight must fail rather
    /// than write them again.
    async fn resolve_tenant_id(&self, user_id: &str, provider: &str) -> EnformeResult<TenantId> {
        let user_uuid = user_id
            .parse::<Uuid>()
            .map_err(|e| EnformeError::store(format!("Invalid user_id UUID: {e}")))?;

        if let Some(token) = self.serving_token(user_uuid, provider).await? {
            return token_tenant_id(&token);
        }
        self.wellness_link_tenant(user_uuid, provider)
            .await?
            .ok_or_else(|| {
                EnformeError::store(format!(
                    "No token serves provider '{provider}' for user '{user_id}'"
                ))
            })
    }

    /// The tenant `member`'s confirmed `provider` link files their wellness
    /// under, while the group relation backs it and the member's wellness
    /// consent holds; `None` outside the delegated pass's sync of `member`.
    async fn wellness_link_tenant(
        &self,
        member: Uuid,
        provider: &str,
    ) -> EnformeResult<Option<TenantId>> {
        if !in_delegated_sync_of(member) {
            return Ok(None);
        }
        let links = self
            .auth
            .delegated_connections
            .list_backed_for_member(member, &sync_backend(provider))
            .await
            .map_err(|e| EnformeError::store(format!("Failed to look up delegated links: {e}")))?;
        for link in links
            .iter()
            .filter(|link| link.status == DelegationStatus::Confirmed)
        {
            let consents = link_consents(self.registry.groups.as_ref(), link)
                .await
                .map_err(|e| {
                    EnformeError::store(format!(
                        "Failed to look up the member's coach consents: {e}"
                    ))
                })?;
            if consents.wellness_read {
                return Ok(link.member_tenant_id);
            }
        }
        Ok(None)
    }

    /// The credentials `member`'s `provider` wellness syncs with through
    /// their human coach's link, for a member who holds no token of their
    /// own: handed out only within the delegated pass's sync of `member`, and
    /// only once the runtime's refresher is installed, since deciding it
    /// reads the coach's roster through what only the runtime holds.
    async fn delegated_credentials(
        &self,
        member: Uuid,
        provider: &str,
    ) -> EnformeResult<Option<ProviderCredentials>> {
        if !in_delegated_sync_of(member) {
            return Ok(None);
        }
        let Some(refresher) = self.refresher.get() else {
            return Ok(None);
        };
        Ok(refresher
            .delegated_credentials(member, provider)
            .await?
            .map(|wellness| {
                record_private_wellness(member, wellness.private);
                wellness.credentials
            }))
    }

    /// The confirmed `provider` links whose member's wellness the delegated
    /// pass may sync, as far as Dravr's own rows tell
    /// ([`excluded_from_wellness`]), oldest confirmation first. What only the
    /// platform can tell — whether its roster still lists the athlete, and
    /// what the athlete keeps private — is decided when each sync asks for
    /// its credentials.
    ///
    /// A link whose exclusions cannot be read is logged and left out.
    ///
    /// # Errors
    ///
    /// Returns the repository error when the links cannot be listed.
    pub(crate) async fn delegated_wellness_links(
        &self,
        provider: &str,
    ) -> AppResult<Vec<DelegatedConnection>> {
        let links = self
            .registry
            .delegated_connections
            .list_confirmed_for_provider(&sync_backend(provider))
            .await?;
        let mut eligible = Vec::with_capacity(links.len());
        for link in links {
            match excluded_from_wellness(&self.registry, &link).await {
                Ok(None) => eligible.push(link),
                Ok(Some(_)) => {}
                Err(e) => warn!(
                    link_id = %link.id,
                    error = %e,
                    "Delegated wellness pass skipped a link whose exclusions could not be read"
                ),
            }
        }
        Ok(eligible)
    }

    /// The worker ledger the delegated pass claims each link's sync in.
    pub(crate) fn worker_runs(&self) -> &dyn WorkerRunRepository {
        self.registry.worker_runs.as_ref()
    }
}

/// The tenant `token` is stored under.
fn token_tenant_id(token: &UserOAuthToken) -> EnformeResult<TenantId> {
    TenantId::parse_str(&token.tenant_id).map_err(|e| {
        EnformeError::store(format!("Invalid tenant_id UUID '{}': {e}", token.tenant_id))
    })
}

impl fmt::Debug for PierreSyncStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PierreSyncStorage").finish_non_exhaustive()
    }
}

// ============================================================================
// SleepStore, RecoveryStore, HealthStore, DataSourceStore: the wellness store
// ============================================================================

/// A wellness store failure as enforme's store error, naming what failed.
fn wellness_error(what: &'static str) -> impl Fn(AppError) -> EnformeError {
    move |e| EnformeError::store(format!("Failed to {what}: {e}"))
}

#[async_trait]
impl SleepStore for PierreSyncStorage {
    async fn store_sleep_sessions(
        &self,
        sessions: &[dravr_equilibre_sync::StoredSleepSession],
    ) -> EnformeResult<u64> {
        let synced_at = Utc::now();
        let mut writes = WellnessStore::new(&self.registry).writes("sleep");
        let mut count = 0u64;
        for session in sessions {
            let tenant_id = self
                .resolve_tenant_id(&session.user_id, &session.source_name)
                .await?;
            // A link files only what the member's consent names, less what
            // the athlete keeps private.
            let session = match delegated_private_of_row(&session.user_id) {
                Some(private) => match sleep_the_link_files(session, private) {
                    Some(kept) => Cow::Owned(kept),
                    None => continue,
                },
                None => Cow::Borrowed(session),
            };
            let landed = writes
                .sleep(tenant_id, &session, synced_at)
                .await
                .map_err(wellness_error("upsert sleep session"))?;
            count += u64::from(landed == WellnessLanded::Written);
        }
        writes.report();
        Ok(count)
    }

    async fn delete_sleep_session(&self, id: &str, policy: &DeletionPolicy) -> EnformeResult<()> {
        // enforme names the row by id alone; the store resolves its tenant
        // and deletes under it, so the delete stays tenant-scoped.
        let deleted = WellnessStore::new(&self.registry)
            .delete_sleep_session(id, policy.is_soft_delete())
            .await
            .map_err(wellness_error("delete sleep session"))?;
        if !deleted {
            return Err(EnformeError::store(format!(
                "Sleep session {id} not found for delete"
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl RecoveryStore for PierreSyncStorage {
    async fn store_recovery_metrics(
        &self,
        metrics: &[dravr_equilibre_sync::StoredRecoveryMetrics],
    ) -> EnformeResult<u64> {
        let synced_at = Utc::now();
        let mut writes = WellnessStore::new(&self.registry).writes("recovery");
        let mut count = 0u64;
        for metric in metrics {
            let tenant_id = self
                .resolve_tenant_id(&metric.user_id, &metric.source_name)
                .await?;
            // A link files only what the member's consent names, less what
            // the athlete keeps private.
            let metric = match delegated_private_of_row(&metric.user_id) {
                Some(private) => match recovery_the_link_files(metric, private) {
                    Some(kept) => Cow::Owned(kept),
                    None => continue,
                },
                None => Cow::Borrowed(metric),
            };
            // A sync reads the whole day as the provider holds it now.
            let landed = writes
                .recovery(tenant_id, &metric, WellnessWriteMode::FullDay, synced_at)
                .await
                .map_err(wellness_error("upsert recovery metrics"))?;
            count += u64::from(landed == WellnessLanded::Written);
        }
        writes.report();
        Ok(count)
    }

    async fn delete_recovery_metric(&self, id: &str, policy: &DeletionPolicy) -> EnformeResult<()> {
        let deleted = WellnessStore::new(&self.registry)
            .delete_recovery_metric(id, policy.is_soft_delete())
            .await
            .map_err(wellness_error("delete recovery metric"))?;
        if !deleted {
            return Err(EnformeError::store(format!(
                "Recovery metric {id} not found for delete"
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl HealthStore for PierreSyncStorage {
    async fn store_health_snapshots(
        &self,
        snapshots: &[dravr_equilibre_sync::StoredHealthMetrics],
    ) -> EnformeResult<u64> {
        let synced_at = Utc::now();
        let mut writes = WellnessStore::new(&self.registry).writes("health");
        let mut count = 0u64;
        for snapshot in snapshots {
            let tenant_id = self
                .resolve_tenant_id(&snapshot.user_id, &snapshot.source_name)
                .await?;
            // A link files only what the member's consent names, less what
            // the athlete keeps private.
            let snapshot = match delegated_private_of_row(&snapshot.user_id) {
                Some(private) => match health_the_link_files(snapshot, private) {
                    Some(kept) => Cow::Owned(kept),
                    None => continue,
                },
                None => Cow::Borrowed(snapshot),
            };
            let landed = writes
                .body(tenant_id, &snapshot, WellnessWriteMode::FullDay, synced_at)
                .await
                .map_err(wellness_error("upsert health snapshot"))?;
            count += u64::from(landed == WellnessLanded::Written);
        }
        writes.report();
        Ok(count)
    }

    async fn delete_health_snapshot(&self, id: &str, policy: &DeletionPolicy) -> EnformeResult<()> {
        let deleted = WellnessStore::new(&self.registry)
            .delete_health_snapshot(id, policy.is_soft_delete())
            .await
            .map_err(wellness_error("delete health snapshot"))?;
        if !deleted {
            return Err(EnformeError::store(format!(
                "Health snapshot {id} not found for delete"
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl DailyActivityStore for PierreSyncStorage {
    async fn store_daily_activity(
        &self,
        days: &[dravr_equilibre_sync::StoredDailyActivity],
    ) -> EnformeResult<u64> {
        let synced_at = Utc::now();
        let mut writes = WellnessStore::new(&self.registry).writes("daily activity");
        let mut count = 0u64;
        for day in days {
            // A link files only what the member's consent names: sleep,
            // recovery and weight. A day's activity totals are none of them.
            if delegated_private_of_row(&day.user_id).is_some() {
                continue;
            }
            let tenant_id = self
                .resolve_tenant_id(&day.user_id, &day.source_name)
                .await?;
            let landed = writes
                .daily_activity(tenant_id, day, WellnessWriteMode::FullDay, synced_at)
                .await
                .map_err(wellness_error("upsert daily activity"))?;
            count += u64::from(landed == WellnessLanded::Written);
        }
        writes.report();
        Ok(count)
    }
}

#[async_trait]
impl DataSourceStore for PierreSyncStorage {
    async fn upsert_data_source(
        &self,
        source: &dravr_equilibre_sync::DataSource,
    ) -> EnformeResult<String> {
        let tenant_id = self
            .resolve_tenant_id(&source.user_id, &source.provider)
            .await?;
        WellnessStore::new(&self.registry)
            .record_data_source(&tenant_id, source)
            .await
            .map_err(wellness_error("upsert data source"))
    }
}

// ============================================================================
// SyncCursorStore
// ============================================================================

#[async_trait]
impl SyncCursorStore for PierreSyncStorage {
    async fn get_cursor(
        &self,
        user_id: &str,
        provider: &str,
        data_type: &str,
    ) -> EnformeResult<Option<SyncCursor>> {
        let tenant_id = self.resolve_tenant_id(user_id, provider).await?;
        let row = self
            .fitness
            .sync_cursors
            .get_sync_cursor(user_id, &tenant_id, provider, data_type)
            .await
            .map_err(|e| EnformeError::store(format!("Failed to get sync cursor: {e}")))?;

        Ok(row.map(|r| sync_cursor_row_to_enforme(&r)))
    }

    async fn update_cursor(&self, cursor: &SyncCursor) -> EnformeResult<()> {
        let tenant_id = self
            .resolve_tenant_id(&cursor.user_id, &cursor.provider)
            .await?;
        let row = sync_cursor_to_row(cursor, &tenant_id);
        self.fitness
            .sync_cursors
            .upsert_sync_cursor(&row)
            .await
            .map_err(|e| EnformeError::store(format!("Failed to update sync cursor: {e}")))
    }
}

// ============================================================================
// CredentialStore
// ============================================================================

/// The wait an enforme rate-limit refusal reports when the budget's refusal
/// carries none; [`RequestBudget::admit`] always states one.
const UNSTATED_RETRY_AFTER_SECS: u64 = 60;

/// A token's request budget as enforme's [`RequestGate`], so the health
/// sync's provider calls are admitted against the same windows as every
/// other call the token makes.
struct BudgetGate(RequestBudget);

#[async_trait]
impl RequestGate for BudgetGate {
    async fn admit(&self, provider: &str) -> EnformeResult<()> {
        self.0.admit(provider).await.map_err(|e| {
            if e.code == ErrorCode::ExternalRateLimited {
                EnformeError::RateLimited {
                    provider: provider.to_owned(),
                    retry_after_secs: e.retry_after_secs().unwrap_or(UNSTATED_RETRY_AFTER_SECS),
                }
            } else {
                EnformeError::store(format!("provider request budget: {e}"))
            }
        })
    }
}

/// The gate an enforme credential carries for `budget`, the signing app's.
#[must_use]
pub fn request_gate(budget: RequestBudget) -> RequestGateHandle {
    RequestGateHandle(Arc::new(BudgetGate(budget)))
}

#[async_trait]
impl CredentialStore for PierreSyncStorage {
    /// The athlete's own token row serving `provider`, refreshed through the
    /// runtime once its refresher is installed. An athlete who holds none is
    /// served through their human coach's link only within the delegated
    /// pass's sync of them ([`delegated_sync_scope`]), and has no credentials
    /// anywhere else: an on-demand refresh or a webhook never spends the
    /// coach's key.
    async fn get_credentials(
        &self,
        user_id: &str,
        provider: &str,
    ) -> EnformeResult<Option<ProviderCredentials>> {
        let user_uuid = user_id
            .parse::<Uuid>()
            .map_err(|e| EnformeError::store(format!("Invalid user_id UUID: {e}")))?;

        let Some(token) = self.serving_token(user_uuid, provider).await? else {
            return self.delegated_credentials(user_uuid, provider).await;
        };
        let tenant_id = token_tenant_id(&token)?;
        let backend = sync_backend(provider);
        self.session_rows.remember(&token);

        // The injected refresher routes through AuthService::get_valid_token,
        // which transparently refreshes near-expiry tokens and persists the
        // result — the same path live tool calls use.
        if let Some(refresher) = self.refresher.get() {
            return refresher
                .valid_credentials(user_uuid, &tenant_id.to_string(), &backend)
                .await;
        }

        // No refresher yet (startup window before injection, storage-only
        // tests): return the stored token row as-is.
        let scopes = token
            .scope
            .map(|s| s.split(' ').map(String::from).collect::<Vec<_>>())
            .unwrap_or_default();
        Ok(Some(ProviderCredentials {
            access_token: token.access_token.expose_secret().to_owned(),
            refresh_token: token
                .refresh_token
                .as_ref()
                .map(|s| s.expose_secret().to_owned()),
            expires_at: token.expires_at,
            scopes,
            user_id: token.user_id.to_string(),
            provider: token.provider,
            provider_user_id: token.provider_user_id,
            kind: credential_kind(&token.token_type),
            // Only the refresher resolves the signing app; this stored-row
            // read serves the startup window before it is installed.
            request_gate: None,
        }))
    }

    async fn refresh_credentials(
        &self,
        user_id: &str,
        provider: &str,
    ) -> EnformeResult<ProviderCredentials> {
        let expired = || EnformeError::CredentialsExpired {
            user_id: user_id.to_owned(),
            provider: provider.to_owned(),
        };
        let user_uuid = user_id
            .parse::<Uuid>()
            .map_err(|e| EnformeError::store(format!("Invalid user_id UUID: {e}")))?;

        // An athlete served through their coach's link holds nothing of their
        // own to refresh, and the coach's key is the coach's to renew: the
        // sync ends as expired, and no connection is touched.
        let Some(token) = self.serving_token(user_uuid, provider).await? else {
            return Err(expired());
        };
        if let Some(refresher) = self.refresher.get() {
            let tenant_id = token_tenant_id(&token)?;
            return refresher
                .force_refresh(user_uuid, &tenant_id.to_string(), &sync_backend(provider))
                .await?
                .ok_or_else(expired);
        }

        // No refresher yet: return the stored credentials and let the caller
        // retry — without the OAuth bridge there is nothing to refresh with.
        self.get_credentials(user_id, provider)
            .await?
            .ok_or_else(expired)
    }
}

// ============================================================================
// UserConnectionStore
// ============================================================================

#[async_trait]
impl UserConnectionStore for PierreSyncStorage {
    async fn list_connected_users(&self, provider: &str) -> EnformeResult<Vec<ConnectedUser>> {
        let rows = self
            .fitness
            .sync_cursors
            .list_connected_provider_users(&sync_backend(provider))
            .await
            .map_err(|e| EnformeError::store(format!("Failed to list connected users: {e}")))?;
        let pushed = self.pushed_holders(provider).await?;

        // dravr-enforme's ConnectedUser.user_id is `String` (leaf-dep API).
        // ConnectedUserRow.user_id is the UserId newtype, so we render to the
        // canonical hyphenated form via Display at the boundary. The roster
        // names the provider as the sync knows it, whichever backend row
        // served the user.
        let roster: Vec<ConnectedUser> = rows
            .into_iter()
            .filter(|r| !pushed.contains(&(r.user_id.to_string(), r.tenant_id)))
            .map(|r| ConnectedUser {
                user_id: r.user_id.to_string(),
                provider: provider.to_owned(),
                connected_at: Utc::now(),
                is_active: true,
            })
            .collect();
        if !pushed.is_empty() {
            debug!(
                provider,
                fed_by_push = pushed.len(),
                "Health sync roster leaves out the athletes the provider pushes to"
            );
        }
        Ok(roster)
    }
}

impl PierreSyncStorage {
    /// Every (user, tenant) holding the backend that pushes `provider`'s data
    /// ([`pushed_backend_for`]) with the permission that lets it push their
    /// wellness ([`Self::wellness_is_pushed`]); empty for a provider that
    /// pushes nothing. A holder who withheld that permission stays on the
    /// roster: nothing else serves their wellness.
    async fn pushed_holders(&self, provider: &str) -> EnformeResult<HashSet<(String, TenantId)>> {
        let Some(pushed) = pushed_backend_for(provider) else {
            return Ok(HashSet::new());
        };
        let rows = self
            .fitness
            .sync_cursors
            .list_connected_provider_users(pushed)
            .await
            .map_err(|e| {
                EnformeError::store(format!("Failed to list the users {pushed} pushes to: {e}"))
            })?;
        let mut holders = HashSet::with_capacity(rows.len());
        for row in rows {
            if self
                .wellness_is_pushed(row.user_id.as_uuid(), row.tenant_id, pushed)
                .await?
            {
                holders.insert((row.user_id.to_string(), row.tenant_id));
            }
        }
        Ok(holders)
    }
}

// ============================================================================
// TimeSeriesPointStore
// ============================================================================

#[async_trait]
impl TimeSeriesPointStore for PierreSyncStorage {
    async fn store_continuous_metrics(
        &self,
        source_id: &str,
        batches: &[dravr_equilibre_sync::ContinuousMetricBatch],
    ) -> EnformeResult<u64> {
        let store = WellnessStore::new(&self.registry);
        let mut total: u64 = 0;
        for batch in batches {
            // riviere's TimeSeriesStore::insert_batch reports success, not a row
            // count, so the written total is the number of points submitted.
            let count = batch.points.len() as u64;
            let points: Vec<DataPoint> = batch
                .points
                .iter()
                .map(|&(timestamp, value)| DataPoint::new(timestamp, value))
                .collect();
            let series = IntradaySeries::try_from_id(batch.series_type_id, points)
                .map_err(wellness_error("insert continuous metrics"))?;
            store
                .store_series(source_id, series)
                .await
                .map_err(wellness_error("insert continuous metrics"))?;
            total += count;
        }
        Ok(total)
    }
}

// ============================================================================
// Conversion helpers
// ============================================================================

/// Convert a database `SyncCursorRow` to an enforme `SyncCursor`.
fn sync_cursor_row_to_enforme(row: &SyncCursorRow) -> SyncCursor {
    let status = match row.last_sync_status.as_str() {
        "completed" => SyncStatus::Completed,
        "failed" => SyncStatus::Failed,
        "in_progress" => SyncStatus::InProgress,
        "cancelled" => SyncStatus::Cancelled,
        _ => SyncStatus::Pending,
    };

    let last_sync_at = row.last_sync_at.unwrap_or_else(Utc::now);
    let next_retry_at = row.next_retry_at;

    SyncCursor {
        user_id: row.user_id.clone(),
        provider: row.provider.clone(),
        data_type: row.data_type.clone(),
        value: row.cursor_value.clone().unwrap_or_default(),
        last_sync_at,
        status,
        records_synced: row.records_synced as u64,
        error_message: row.error_message.clone(),
        retry_count: row.retry_count.cast_unsigned(),
        next_retry_at,
    }
}

/// Convert an enforme `SyncCursor` to a database `SyncCursorRow`.
fn sync_cursor_to_row(cursor: &SyncCursor, tenant_id: &TenantId) -> SyncCursorRow {
    let status_str = match cursor.status {
        SyncStatus::Pending => "pending",
        SyncStatus::InProgress => "in_progress",
        SyncStatus::Completed => "completed",
        SyncStatus::Failed => "failed",
        SyncStatus::Cancelled => "cancelled",
    };

    // Deterministic ID from user+tenant+provider+data_type
    let id = format!(
        "{}:{}:{}:{}",
        cursor.user_id, tenant_id, cursor.provider, cursor.data_type
    );

    SyncCursorRow {
        id,
        user_id: cursor.user_id.clone(),
        tenant_id: tenant_id.to_string(),
        provider: cursor.provider.clone(),
        data_type: cursor.data_type.clone(),
        cursor_value: if cursor.value.is_empty() {
            None
        } else {
            Some(cursor.value.clone())
        },
        last_sync_at: Some(cursor.last_sync_at),
        last_sync_status: status_str.to_owned(),
        records_synced: cursor.records_synced.cast_signed(),
        error_message: cursor.error_message.clone(),
        retry_count: cursor.retry_count.cast_signed(),
        next_retry_at: cursor.next_retry_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::models::API_KEY_TOKEN_TYPE;
    use pierre_providers::CredentialKind;

    #[test]
    fn a_pasted_api_key_reaches_enforme_as_an_api_key() {
        // intervals.icu sends an API key as HTTP Basic and an OAuth token as a
        // bearer; the stored token_type is all that tells them apart.
        assert_eq!(credential_kind(API_KEY_TOKEN_TYPE), CredentialKind::ApiKey);
        assert_eq!(credential_kind("Bearer"), CredentialKind::OAuthBearer);
    }
}
