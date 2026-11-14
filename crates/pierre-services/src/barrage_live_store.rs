// ABOUTME: The live activity store over dravr-barrage: every provider read kept as a record, projected into the cache
// ABOUTME: Also opens barrage's record and job stores on the deployment's database, for the server and the CLI alike

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! [`LiveActivityStore`](crate::live_activities::LiveActivityStore) over
//! barrage, and the stores it runs on.
//!
//! Barrage owns the record and its retention; the cache readers read the
//! projection ([`LiveCacheSink`](crate::activity_cache_sink::LiveCacheSink)).
//! The record store is chosen from the deployment's database
//! ([`live_record_store`](crate::barrage_live_store::live_record_store)):
//! Postgres schema `barrage` on the platform's own pool when deployed, a
//! separate SQLite file locally
//! ([`ENV_SQLITE_URL`](crate::barrage_live_store::ENV_SQLITE_URL)), memory in
//! tests. The server builds its store here at boot, and `pierre-cli` builds the
//! same one for the seeders that warm the cache.

use std::collections::{BTreeSet, HashSet};
use std::env;
use std::fmt;
#[cfg(not(feature = "postgresql"))]
use std::future::{self, Ready};
use std::slice;
use std::sync::Arc;

use crate::live_activities::{LiveActivityStore, LiveIngest};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use dravr_barrage::blob::memory::MemoryBlobStore;
use dravr_barrage::blob::BlobStore;
use dravr_barrage::record::{StreamsState, SummaryWindow, TotalsFrom};
use dravr_barrage::store::memory::{MemoryActivityRecordStore, MemoryImportJobStore};
use dravr_barrage::store::sql::sqlite::{self, SqliteJobStore, SqliteRecordStore};
use dravr_barrage::store::{ActivityRecordStore, PageReq, RecordQuery, MAX_PAGE_LIMIT};
use dravr_barrage::{
    ActivityId, ActivityIngest, ActivityRecord, BarrageError, BarrageStores, LiveSource,
    ProjectionSink, Provenance, RetractReason, Scope, SourceKey, UserId,
};
use pierre_config::database::DatabaseUrl;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId};
use pierre_database::backends::factory::Database;
use pierre_database::repositories::amended_summary;
use pierre_database::RepositoryRegistry;
use tracing::warn;
use uuid::Uuid;

use crate::activity_cache_sink::LiveCacheSink;

/// Records a termination walk reads per page of a source's athletes.
const SOURCE_WALK_PAGE: u32 = 500;

/// Pages a termination walk reads before it gives up on a source whose
/// records keep arriving: each page's athletes are deleted whole, so only
/// syncs landing during the walk, or more athletes than the passes cover,
/// leave records for a retry.
const SOURCE_WALK_PASSES: u32 = 10_000;

/// Cached rows a window's discard reads per page.
const WINDOW_PAGE: i64 = 500;

/// How far past now the termination walk reads as "after every read": a
/// century, so the moment stays a four-digit year every store compares as
/// text and as a timestamp alike.
const EVERY_READ_HORIZON_DAYS: i64 = 36_500;

/// Every live provider read, kept in barrage.
pub struct BarrageLiveStore {
    records: Arc<dyn ActivityRecordStore>,
    sink: Arc<dyn ProjectionSink>,
    ingest: ActivityIngest,
    repos: Arc<RepositoryRegistry>,
}

impl fmt::Debug for BarrageLiveStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarrageLiveStore").finish_non_exhaustive()
    }
}

impl BarrageLiveStore {
    /// A store over `records` and the originals they keep in `blobs`
    /// (`None` on a deployment with no blob store, which keeps none),
    /// projecting through `sink`; `repos` is the activity cache an erasure
    /// reads a copy barrage never held from.
    #[must_use]
    pub fn new(
        records: Arc<dyn ActivityRecordStore>,
        blobs: Option<Arc<dyn BlobStore>>,
        sink: Arc<dyn ProjectionSink>,
        repos: Arc<RepositoryRegistry>,
    ) -> Self {
        Self {
            ingest: ActivityIngest::new(Arc::clone(&records), blobs, Arc::clone(&sink)),
            records,
            sink,
            repos,
        }
    }

    /// Store and serve `rows` of `live`. Barrage refuses a row the athlete
    /// deleted, and serves only what it kept.
    async fn store(
        &self,
        scope: Scope,
        live: LiveSource,
        rows: Vec<ActivityRecord>,
    ) -> AppResult<LiveIngest> {
        let source = SourceKey::Live(live);
        let report = self
            .ingest
            .ingest(&scope, live, &rows, &[])
            .await
            .map_err(|e| store_error(source, &e))?;
        let counts = report.counts;
        Ok(LiveIngest {
            stored: counts.inserted + counts.updated + counts.unchanged + counts.rewritten,
            inserted: counts.inserted,
            refused: counts.erased,
            newer_session: report.newer_session,
        })
    }
}

/// `provider` as the live source its records are kept under.
fn live_source(provider: &str) -> AppResult<LiveSource> {
    match SourceKey::parse(provider)
        .map_err(|e| AppError::invalid_input(format!("activity source {provider}: {e}")))?
    {
        SourceKey::Live(live) => Ok(live),
        _ => Err(AppError::invalid_input(format!(
            "{provider} is an archive: its records arrive through an import"
        ))),
    }
}

/// `activity_id` as an id barrage holds.
fn activity_id(provider: &str, activity_id: &str) -> AppResult<ActivityId> {
    ActivityId::parse(activity_id)
        .map_err(|e| AppError::invalid_input(format!("{provider} activity id {activity_id}: {e}")))
}

/// A barrage failure on `source` as the platform's error.
fn store_error(source: SourceKey, error: &BarrageError) -> AppError {
    AppError::database(format!("activity store, {source}: {error}"))
}

/// `activity`, read from `source` at `synced_at`, as a barrage record of
/// `scope`; `None` when its id is not one barrage can store.
fn record_of(
    scope: Scope,
    source: SourceKey,
    activity: &Activity,
    synced_at: DateTime<Utc>,
) -> Option<ActivityRecord> {
    let id = match ActivityId::try_from(activity.id().to_owned()) {
        Ok(id) => id,
        Err(e) => {
            warn!(activity_id = activity.id(), source = %source, error = %e, "activity id barrage cannot store; not kept");
            return None;
        }
    };
    let provenance = Provenance {
        source,
        import_id: None,
        source_activity_id: activity.id().to_owned(),
        source_athlete_id: None,
        ingested_at: synced_at,
        original: None,
        original_sha256: None,
        session_index: None,
        streams_state: StreamsState::None,
        source_gear: activity.gear().map(|gear| gear.name.clone()),
        importer_version: None,
        summary_window: SummaryWindow::default(),
        totals_from: TotalsFrom::default(),
    };
    ActivityRecord::try_new(scope, id, provenance, activity.clone())
        .map_err(|e| warn!(activity_id = activity.id(), error = %e, "activity not kept"))
        .ok()
}

/// A record of `source` naming only `id`: what an erasure remembers of an
/// activity neither the store nor the cache ever held. Barrage keys what the
/// athlete deleted by scope, source and id; the start it keeps beside them is
/// read only by archive imports, which never read a live source's.
fn bare_record(
    scope: Scope,
    source: SourceKey,
    id: ActivityId,
    provider: &str,
) -> AppResult<ActivityRecord> {
    let now = Utc::now();
    let activity = ActivityBuilder::new(
        id.as_str(),
        "",
        SportType::Other(String::new()),
        now,
        0,
        provider,
    )
    .build();
    let provenance = Provenance {
        source,
        import_id: None,
        source_activity_id: id.as_str().to_owned(),
        source_athlete_id: None,
        ingested_at: now,
        original: None,
        original_sha256: None,
        session_index: None,
        streams_state: StreamsState::None,
        source_gear: None,
        importer_version: None,
        summary_window: SummaryWindow::default(),
        totals_from: TotalsFrom::default(),
    };
    ActivityRecord::try_new(scope, id, provenance, activity).map_err(|e| store_error(source, &e))
}

#[async_trait]
impl LiveActivityStore for BarrageLiveStore {
    async fn ingest(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activities: &[Activity],
        synced_at: DateTime<Utc>,
    ) -> AppResult<LiveIngest> {
        let live = live_source(provider)?;
        let scope = Scope::new(tenant_id.as_uuid(), user_id);
        let source = SourceKey::Live(live);
        let rows: Vec<ActivityRecord> = activities
            .iter()
            .filter_map(|activity| record_of(scope, source, activity, synced_at))
            .collect();
        self.store(scope, live, rows).await
    }

    async fn ingest_received(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity: &Activity,
        received_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let live = live_source(provider)?;
        let scope = Scope::new(tenant_id.as_uuid(), user_id);
        let source = SourceKey::Live(live);
        let Some(row) = record_of(scope, source, activity, received_at) else {
            return Ok(false);
        };
        // The record's receipt is its last change's: a copy received later
        // than this one stands.
        let held = self
            .records
            .get(&scope, source, &row.activity_id)
            .await
            .map_err(|e| store_error(source, &e))?;
        if held.is_some_and(|held| held.provenance.ingested_at > received_at) {
            return Ok(false);
        }
        Ok(self.store(scope, live, vec![row]).await?.stored > 0)
    }

    async fn amend(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity: &Activity,
    ) -> AppResult<bool> {
        let live = live_source(provider)?;
        let source = SourceKey::Live(live);
        let id = activity_id(provider, activity.id())?;
        let scope = Scope::new(tenant_id.as_uuid(), user_id);
        let held = self
            .records
            .get(&scope, source, &id)
            .await
            .map_err(|e| store_error(source, &e))?;
        // A row the cache served before the store held the provider's reads
        // (one `adopt-cache` has not reached yet) is adopted here, at its own
        // receipt, so the amendment lands in both. Rare once the adoption has
        // run, so reading the provider's rows is fine.
        let received_at = if let Some(held) = held {
            held.provenance.ingested_at
        } else {
            let cached = self
                .repos
                .activity_cache
                .get_stored_activity_rows(user_id, &tenant_id, provider)
                .await?
                .into_iter()
                .find(|row| row.activity.id() == activity.id());
            let Some(cached) = cached else {
                return Ok(false);
            };
            cached.synced_at
        };
        // Rewritten at the record's own receipt: the cache row's stamp is
        // that receipt, so its projection rewrites the row in place, and a
        // copy received later still stands over this one. The summary alone,
        // as the cache's own amend writes it: laps a cache read folded in
        // stand beside the row, not in it nor in the record.
        let summary = amended_summary(activity)?;
        let Some(row) = record_of(scope, source, &summary, received_at) else {
            return Ok(false);
        };
        Ok(self.store(scope, live, vec![row]).await?.stored > 0)
    }

    async fn erase(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity_id_text: &str,
    ) -> AppResult<bool> {
        let source = SourceKey::Live(live_source(provider)?);
        let id = activity_id(provider, activity_id_text)?;
        let scope = Scope::new(tenant_id.as_uuid(), user_id);
        let held = self
            .records
            .get(&scope, source, &id)
            .await
            .map_err(|e| store_error(source, &e))?;
        // A copy the cache served before barrage held the provider's reads
        // is remembered all the same, so no later read brings it back.
        let remembered = match &held {
            Some(record) => Some(record.clone()),
            None => self
                .repos
                .activity_cache
                .get_cached_activity(user_id, &tenant_id, provider, activity_id_text)
                .await?
                .and_then(|activity| record_of(scope, source, &activity, Utc::now())),
        };
        let found = remembered.is_some();
        // A deletion that arrives before any read of the activity — a
        // webhook outrunning the sync — is remembered from the bare id, so
        // the read that follows is refused too.
        let record = match remembered {
            Some(record) => record,
            None => bare_record(scope, source, id.clone(), provider)?,
        };
        // Remembered first, as barrage's own erasure does: from here no read
        // stores it again, so what the retraction stops serving stays stopped.
        self.records
            .remember_erased(&scope, &record, &[])
            .await
            .map_err(|e| store_error(source, &e))?;
        self.sink
            .retract(
                &scope,
                source,
                Some(slice::from_ref(&id)),
                RetractReason::Erased,
            )
            .await
            .map_err(|e| store_error(source, &e))?;
        if held.is_some() {
            self.ingest
                .delete(&scope, source, slice::from_ref(&id))
                .await
                .map_err(|e| store_error(source, &e))?;
        }
        Ok(found)
    }

    async fn discard(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity_id_text: &str,
    ) -> AppResult<bool> {
        let source = SourceKey::Live(live_source(provider)?);
        let id = activity_id(provider, activity_id_text)?;
        let scope = Scope::new(tenant_id.as_uuid(), user_id);
        self.discard_record(scope, source, &id).await
    }

    async fn discard_between(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<u64> {
        let source = SourceKey::Live(live_source(provider)?);
        let scope = Scope::new(tenant_id.as_uuid(), user_id);
        let mut dropped: BTreeSet<ActivityId> = BTreeSet::new();
        // The cache's rows first: one the store never held (served before
        // the store kept the provider's reads) goes all the same. Each page
        // is read after the last one's rows went, so the walk ends on a page
        // holding nothing new.
        loop {
            let page = self
                .repos
                .activity_cache
                .get_cached_activities(user_id, &tenant_id, Some(provider), start, end, WINDOW_PAGE)
                .await?;
            let mut fresh = Vec::new();
            for activity in &page {
                match ActivityId::try_from(activity.id().to_owned()) {
                    Ok(id) if !dropped.contains(&id) => fresh.push(id),
                    Ok(_) => {}
                    Err(e) => {
                        warn!(%user_id, provider, activity_id = activity.id(), error = %e, "cached activity id the store cannot name; left in the cache");
                    }
                }
            }
            if fresh.is_empty() {
                break;
            }
            for id in fresh {
                self.discard_record(scope, source, &id).await?;
                dropped.insert(id);
            }
        }
        // Then the store's records in the window its projection no longer
        // serves. The store's window ends before `until`; `end` is included,
        // so the records are read from `start` on and bounded here.
        let mut held = Vec::new();
        let mut offset = Some(0_u64);
        while let Some(at) = offset {
            let query = RecordQuery {
                source: Some(source),
                since: Some(start),
                until: None,
                page: PageReq {
                    limit: MAX_PAGE_LIMIT,
                    offset: at,
                },
            };
            let page = self
                .records
                .list(&scope, &query)
                .await
                .map_err(|e| store_error(source, &e))?;
            held.extend(
                page.items
                    .into_iter()
                    .filter(|record| record.activity.start_date() <= end)
                    .map(|record| record.activity_id)
                    .filter(|id| !dropped.contains(id)),
            );
            offset = page.next_offset;
        }
        for id in held {
            self.discard_record(scope, source, &id).await?;
            dropped.insert(id);
        }
        u64::try_from(dropped.len()).map_err(|_| AppError::internal("activity count overflows u64"))
    }

    async fn held(&self, user_id: Uuid, tenant_id: TenantId, provider: &str) -> AppResult<u64> {
        let source = SourceKey::Live(live_source(provider)?);
        let scope = Scope::new(tenant_id.as_uuid(), user_id);
        let mut held = 0_u64;
        let mut offset = Some(0_u64);
        while let Some(at) = offset {
            let query = RecordQuery {
                source: Some(source),
                since: None,
                until: None,
                page: PageReq {
                    limit: MAX_PAGE_LIMIT,
                    offset: at,
                },
            };
            let page = self
                .records
                .list(&scope, &query)
                .await
                .map_err(|e| store_error(source, &e))?;
            held += u64::try_from(page.items.len()).unwrap_or(u64::MAX);
            offset = page.next_offset;
        }
        Ok(held)
    }

    async fn held_among(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activities: &[Activity],
    ) -> AppResult<HashSet<String>> {
        let starts = activities.iter().map(Activity::start_date);
        let (Some(first), Some(last)) = (starts.clone().min(), starts.max()) else {
            return Ok(HashSet::new());
        };
        let wanted: HashSet<&str> = activities.iter().map(Activity::id).collect();
        // The cache serves every record the store holds, and the rows it
        // served before the store held the provider's reads.
        Ok(self
            .repos
            .activity_cache
            .get_cached_activities(user_id, &tenant_id, Some(provider), first, last, i64::MAX)
            .await?
            .iter()
            .map(Activity::id)
            .filter(|id| wanted.contains(id))
            .map(str::to_owned)
            .collect())
    }

    async fn recorded_among(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activities: &[Activity],
    ) -> AppResult<HashSet<String>> {
        let source = SourceKey::Live(live_source(provider)?);
        let starts = activities.iter().map(Activity::start_date);
        let (Some(first), Some(last)) = (starts.clone().min(), starts.max()) else {
            return Ok(HashSet::new());
        };
        let wanted: HashSet<&str> = activities.iter().map(Activity::id).collect();
        let scope = Scope::new(tenant_id.as_uuid(), user_id);
        let mut recorded = HashSet::new();
        let mut offset = Some(0_u64);
        while let Some(at) = offset {
            let query = RecordQuery {
                source: Some(source),
                since: Some(first),
                until: None,
                page: PageReq {
                    limit: MAX_PAGE_LIMIT,
                    offset: at,
                },
            };
            let page = self
                .records
                .list(&scope, &query)
                .await
                .map_err(|e| store_error(source, &e))?;
            recorded.extend(
                page.items
                    .iter()
                    .filter(|record| record.activity.start_date() <= last)
                    .map(|record| record.activity_id.as_str())
                    .filter(|id| wanted.contains(id))
                    .map(str::to_owned),
            );
            offset = page.next_offset;
        }
        Ok(recorded)
    }

    async fn delete_source(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()> {
        let source = SourceKey::Live(live_source(provider)?);
        self.delete_scope(Scope::new(tenant_id.as_uuid(), user_id), source)
            .await
    }

    fn keeps(&self, provider: &str) -> bool {
        matches!(SourceKey::parse(provider), Ok(SourceKey::Live(_)))
    }

    async fn delete_source_everywhere(&self, provider: &str) -> AppResult<u64> {
        let source = SourceKey::Live(live_source(provider)?);
        // Every record of the source shows as expired before a moment no
        // record was read at, scope by scope: each page names athletes whose
        // records then go whole. An athlete still connected keeps syncing,
        // so a scope already deleted can show again; it is deleted again,
        // and the walk ends on an empty page or fails after its last pass.
        let after_every_read = Utc::now() + Duration::days(EVERY_READ_HORIZON_DAYS);
        let mut deleted: BTreeSet<Scope> = BTreeSet::new();
        for _ in 0..SOURCE_WALK_PASSES {
            let page = self
                .records
                .expired(source, after_every_read, SOURCE_WALK_PAGE)
                .await
                .map_err(|e| store_error(source, &e))?;
            if page.is_empty() {
                return u64::try_from(deleted.len())
                    .map_err(|_| AppError::internal("athlete count overflows u64"));
            }
            let scopes: BTreeSet<Scope> = page.into_iter().map(|(scope, _)| scope).collect();
            for scope in scopes {
                self.delete_scope(scope, source).await?;
                deleted.insert(scope);
            }
        }
        Err(AppError::database(format!(
            "activity store, {source}: records still held after {SOURCE_WALK_PASSES} deletion passes"
        )))
    }

    async fn delete_user(&self, user_id: Uuid) -> AppResult<u64> {
        self.records
            .delete_user(UserId(user_id))
            .await
            .map_err(|e| AppError::database(format!("activity store, deleting an account: {e}")))
    }
}

impl BarrageLiveStore {
    /// Stop serving `scope`'s activity `id` of the live `source` — its gear
    /// attribution released, then its cached row and samples deleted — and
    /// delete the store's record of it, without remembering it as erased.
    /// Returns whether the store held it.
    async fn discard_record(
        &self,
        scope: Scope,
        source: SourceKey,
        id: &ActivityId,
    ) -> AppResult<bool> {
        self.sink
            .retract(
                &scope,
                source,
                Some(slice::from_ref(id)),
                RetractReason::Erased,
            )
            .await
            .map_err(|e| store_error(source, &e))?;
        let held = self
            .records
            .get(&scope, source, id)
            .await
            .map_err(|e| store_error(source, &e))?
            .is_some();
        if held {
            self.ingest
                .delete(&scope, source, slice::from_ref(id))
                .await
                .map_err(|e| store_error(source, &e))?;
        }
        Ok(held)
    }

    /// Stop serving `scope`'s records of the live `source`, then delete them,
    /// what the athlete had deleted of it and the originals no other record
    /// keeps.
    async fn delete_scope(&self, scope: Scope, source: SourceKey) -> AppResult<()> {
        self.sink
            .retract(&scope, source, None, RetractReason::SourceDeleted)
            .await
            .map_err(|e| store_error(source, &e))?;
        self.ingest
            .delete_source(&scope, source)
            .await
            .map_err(|e| store_error(source, &e))?;
        Ok(())
    }
}

/// The live store a server runs on: over `records` when the binary selected
/// them, in memory otherwise, with the originals they keep in `blobs`,
/// projecting into `repos`' activity cache.
#[must_use]
pub fn live_store(
    records: Option<Arc<dyn ActivityRecordStore>>,
    blobs: Option<Arc<dyn BlobStore>>,
    repos: &Arc<RepositoryRegistry>,
) -> Arc<dyn LiveActivityStore> {
    let records = records.unwrap_or_else(|| Arc::new(MemoryActivityRecordStore::new()));
    let sink = Arc::new(LiveCacheSink::new(Arc::clone(repos)));
    Arc::new(BarrageLiveStore::new(
        records,
        blobs,
        sink,
        Arc::clone(repos),
    ))
}

/// The SQLite database a local run keeps barrage's records and jobs in: a
/// file of its own, never the platform's (the platform's migrator refuses a
/// history holding versions it does not know).
pub const ENV_SQLITE_URL: &str = "BARRAGE_SQLITE_URL";

/// Where a local run keeps records and jobs when [`ENV_SQLITE_URL`] is unset.
const DEFAULT_SQLITE_URL: &str = "sqlite:./data/barrage.db";

/// The platform's Postgres pool, when the platform runs on Postgres: the
/// pool barrage's record and job stores run their statements on.
#[derive(Clone, Debug)]
pub struct PlatformPool(#[cfg(feature = "postgresql")] Option<sqlx::PgPool>);

impl PlatformPool {
    /// `database`'s pool, when it is a Postgres one.
    #[cfg(feature = "postgresql")]
    #[must_use]
    pub fn of(database: &Database) -> Self {
        Self(database.postgres_pool().cloned())
    }

    /// No pool: a build without Postgres support runs on no Postgres.
    #[cfg(not(feature = "postgresql"))]
    #[must_use]
    pub const fn of(_database: &Database) -> Self {
        Self()
    }
}

/// The activity record store live provider reads are kept in, on the
/// deployment's database (carnet#1482).
///
/// It is the selection [`stores_for`] makes, without the blob store only
/// imports need. Unlike imports, it is not
/// optional: every live read is written through it.
///
/// # Errors
///
/// Why the store cannot be placed: the boot, or the seeder, stops on it.
pub async fn live_record_store(
    database: &Database,
    url: &DatabaseUrl,
) -> Result<Arc<dyn ActivityRecordStore>, String> {
    let blobs: Arc<dyn BlobStore> = Arc::new(MemoryBlobStore::new());
    Ok(stores_for(PlatformPool::of(database), url, blobs)
        .await?
        .records)
}

/// The stores the deployment's database selects, over its blob store
/// `blobs`; `pool` is the platform's, which a Postgres deployment's stores
/// run their statements on.
///
/// Awaited before anything that must be `Send` is built: sqlx's migrator
/// holds a connection across awaits, which the compiler cannot prove `Send`
/// for every lifetime.
///
/// # Errors
///
/// Why the stores cannot be placed: the SQLite file does not open, the
/// `barrage` schema cannot be migrated, or this build has no Postgres.
pub async fn stores_for(
    pool: PlatformPool,
    url: &DatabaseUrl,
    blobs: Arc<dyn BlobStore>,
) -> Result<BarrageStores, String> {
    if url.is_memory() {
        return Ok(BarrageStores {
            blobs,
            records: Arc::new(MemoryActivityRecordStore::new()),
            jobs: Arc::new(MemoryImportJobStore::new()),
        });
    }
    if url.is_postgresql() {
        return postgres_stores(pool, url, blobs).await;
    }
    let sqlite_url = env::var(ENV_SQLITE_URL).unwrap_or_else(|_| DEFAULT_SQLITE_URL.to_owned());
    let pool = sqlite::open(&sqlite_url)
        .await
        .map_err(|e| format!("barrage SQLite database: {e}"))?;
    Ok(BarrageStores {
        blobs,
        records: Arc::new(SqliteRecordStore::new(pool.clone())),
        jobs: Arc::new(SqliteJobStore::new(pool)),
    })
}

/// The `barrage` schema migrated on a connection of its own, then the record
/// and job stores on the platform's pool: no connection slot is added.
#[cfg(feature = "postgresql")]
async fn postgres_stores(
    pool: PlatformPool,
    url: &DatabaseUrl,
    blobs: Arc<dyn BlobStore>,
) -> Result<BarrageStores, String> {
    use std::str::FromStr;

    use dravr_barrage::store::sql::postgres::{migrate, PostgresJobStore, PostgresRecordStore};
    use secrecy::ExposeSecret;
    use sqlx::postgres::PgConnectOptions;

    let pool = pool
        .0
        .ok_or_else(|| "the platform database is not Postgres".to_owned())?;
    let options = PgConnectOptions::from_str(url.connection_string().expose_secret())
        .map_err(|e| format!("the database URL does not parse: {e}"))?;
    migrate(&options)
        .await
        .map_err(|e| format!("the barrage schema could not be migrated: {e}"))?;
    Ok(BarrageStores {
        blobs,
        records: Arc::new(PostgresRecordStore::new(pool.clone())),
        jobs: Arc::new(PostgresJobStore::new(pool)),
    })
}

/// A build without Postgres support has no Postgres deployment to serve:
/// the refusal is ready at once, awaited as the Postgres build's migration is.
#[cfg(not(feature = "postgresql"))]
fn postgres_stores(
    _pool: PlatformPool,
    _url: &DatabaseUrl,
    _blobs: Arc<dyn BlobStore>,
) -> Ready<Result<BarrageStores, String>> {
    future::ready(Err("this build has no Postgres support".to_owned()))
}
