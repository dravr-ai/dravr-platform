// ABOUTME: The hourly record_log worker: keys erasures and tombstones, baselines records, renames originals, sweeps orphans, cuts checkpoints
// ABOUTME: Then drains consent grant receipts (baseline, log, JWS) and re-keys one page of legacy apple- records
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The `record_log` worker (carnet#1542)
//!
//! One hourly tick on [`spawn_periodic`], leased to one instance by the
//! worker ledger, does every step, each bounded:
//!
//! 1. keys barrage erasures and import tombstones written without their keyed
//!    digest (before the columns existed, or by an older revision during a
//!    rolling deploy);
//! 2. baselines records no log entry names yet;
//! 3. with a blob store, renames one page of originals still named by their
//!    content to their keyed names (`backfill_keyed_names`), resuming where
//!    the last tick stopped; when a sweep completes it logs what remains and
//!    how much of it could not be read, the backfill's done signal being the
//!    two equal (carnet#1550 then drops the old names); then it copies one
//!    page of uploaded files still named by their content (activity files,
//!    feedback screenshots, lab reports) to their keyed names
//!    (`OriginalsStore::rename_uploads_page`, carnet#1701), one table after
//!    the other, the same done signal logged when the last table's sweep
//!    ends;
//! 4. deletes the log of an account the platform no longer holds (an older
//!    binary's account deletion leaves it behind; see the plan's D2b);
//! 5. with a signer, cuts a checkpoint anchoring the deletion receipt chain's
//!    head, records the anchor, and logs the signed note to Cloud Logging
//!    (target `dravr::checkpoint`): the place the database cannot reach.
//!    An unchanged tree is not cut again, so a tick that finds the newest
//!    checkpoint without its anchor row records it then;
//! 6. gives each un-revoked consent grant (live, or past its expiry) with
//!    no `issue` receipt a baseline one (carnet#1544): grants given before
//!    receipts were, including one the expire pass already ended;
//! 7. appends each receipt not yet logged to its athlete's log
//!    (`log_grant`, idempotent, so a retry after a crash appends nothing)
//!    and marks it logged;
//! 8. with a signer, signs each receipt not yet signed as a compact JWS
//!    ([`crate::grant_receipt_jws`]); without one they wait, unsigned.
//!
//! 9. with an importer, moves one page of Apple records still stored under
//!    their legacy `apple-<hex16>` id to their keyed id, the legacy id kept
//!    as the alias erasures match (`Importer::rekey_legacy_apple`,
//!    carnet#1550, JF 2026-11-25: "re-key them once"), resuming where the
//!    last tick stopped, and logs how many remain, every tick. The pass is
//!    done when what remains is what the sweep refused (records the athlete
//!    deleted under one of their ids, which the deletion removes).
//!
//! Steps 6 to 9 run after the checkpoint, so a failing grant step or re-key
//! never stops checkpointing; each takes one page a tick, and a row a step
//! cannot finish is left for the next tick. Neither barrage's store nor the
//! signer is ever inside a grant's transaction: the event row is the outbox.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use dravr_barrage::blob::backfill_keyed_names;
use dravr_barrage::digest::Sha256;
use dravr_barrage::keyed::{KeyedDigest, Signer};
use dravr_barrage::record_log::checkpoint::{Anchor, Checkpoint};
use dravr_barrage::record_log::entry::EntryKind;
use dravr_barrage::record_log::{cut_checkpoint, standing_checkpoint, LOG_PAGE};
use dravr_barrage::store::{ActivityRecordStore, BarrageStores, RecordPlace};
use dravr_barrage::{BarrageError, Importer, Scope, UserId};
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{GrantEvent, OAuthGrantEvent};
use pierre_database::repositories::{
    subject_hash, AccountDeletionRepository, ContentNamedFile, ImportTombstoneRepository,
    OAuthGrantEventRepository, ReceiptAnchor, UploadHolder, UserRepository, WorkerRunRepository,
    MAX_GRANT_RECEIPT_ATTEMPTS,
};
use pierre_providers::originals::OriginalsStore;
use tokio::task::AbortHandle;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::barrage_keys::CHECKPOINT_ORIGIN;
use crate::grant_receipt_jws;
use crate::periodic::spawn_periodic;

/// The worker's ledger name.
const WORKER_NAME: &str = "record_log";

/// One tick an hour: a checkpoint is cut at most once per hour.
const INTERVAL: Duration = Duration::from_hours(1);

/// How many rows each backfill step keys per tick.
const BACKFILL_PER_TICK: u32 = 500;

/// What one tick did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecordLogTick {
    /// Barrage erasures keyed.
    pub erasures_keyed: u64,
    /// Import tombstones keyed.
    pub tombstones_keyed: u64,
    /// Baseline entries appended.
    pub baselined: u64,
    /// Originals renamed to their keyed name; `None` without a blob store.
    pub keyed_names: Option<KeyedNamesPage>,
    /// Uploaded files copied to their keyed name; `None` without a blob
    /// store.
    pub upload_names: Option<UploadNamesPage>,
    /// Deleted accounts whose log was swept.
    pub orphans_swept: u64,
    /// The checkpoint cut, with the receipt it anchors; or the newest
    /// checkpoint whose missing anchor row this tick recorded.
    pub anchored: Option<ReceiptAnchor>,
    /// Baseline grant receipts written for grants given before receipts.
    pub grant_receipts_baselined: u64,
    /// Grant receipts whose entry this tick appended (or found appended) to
    /// the athlete's log and marked logged.
    pub grant_receipts_logged: u64,
    /// Grant receipts signed; 0 without a signer.
    pub grant_receipts_signed: u64,
    /// Legacy Apple records re-keyed; `None` without an importer.
    pub legacy_apple: Option<LegacyApplePage>,
}

/// What one tick's page of the legacy Apple re-key did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LegacyApplePage {
    /// Records moved to their keyed id, or whose legacy record was retired
    /// beside a keyed one already held.
    pub rekeyed: u64,
    /// Records of the page left under their legacy id because the athlete
    /// deleted the activity under one of its ids.
    pub refused: u64,
    /// Records still under a legacy Apple id, in every scope.
    pub remaining: u64,
    /// Whether this page ended a sweep over every record.
    pub sweep_done: bool,
}

/// Where the legacy Apple re-key resumes, and what its sweep refused.
#[derive(Debug, Default)]
struct LegacyAppleSweep {
    /// The next page's start; `None` starts a sweep.
    cursor: Option<RecordPlace>,
    /// Records the sweep refused so far.
    refused: u64,
}

/// What one tick's page of the keyed-name backfill did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KeyedNamesPage {
    /// Records re-pointed to their original's keyed name.
    pub repointed: u64,
    /// Records of the page whose old blob could not be read, left as they are.
    pub unreadable: u64,
    /// Records still keeping a content-named original, in every scope.
    pub remaining: u64,
    /// Whether this page ended a sweep over every record.
    pub sweep_done: bool,
}

/// Where the keyed-name backfill resumes, and what its sweep could not read.
#[derive(Debug, Default)]
struct KeyedNamesSweep {
    /// The next page's start; `None` starts a sweep.
    cursor: Option<RecordPlace>,
    /// Records the sweep skipped as unreadable so far.
    unreadable: u64,
}

/// What one tick's page of the uploaded files' keyed-name copy did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadNamesPage {
    /// The table whose files the page copied.
    pub holder: UploadHolder,
    /// Files copied to their keyed name.
    pub copied: u64,
    /// Files of the page neither name holds, or whose copy failed.
    pub unreadable: u64,
    /// Files still named only by their content, over every table, counted
    /// when this page ended a sweep over every table; `None` otherwise.
    pub remaining: Option<u64>,
}

/// Where the uploaded files' keyed-name copy resumes, and what its sweep
/// could not read.
#[derive(Debug, Default)]
struct UploadNamesSweep {
    /// The index in [`UploadHolder::ALL`] of the table being swept.
    phase: usize,
    /// The next page's start in that table; `None` starts it.
    cursor: Option<ContentNamedFile>,
    /// Files the sweep skipped as unreadable so far, over every table.
    unreadable: u64,
}

/// The stores the keyed-name backfills rename in, where a blob store is
/// placed.
pub struct KeyedNameStores {
    /// The stores whose content-named originals are renamed.
    pub originals: BarrageStores,
    /// The uploaded files, copied to their keyed names.
    pub uploads: Arc<OriginalsStore>,
    /// The importer over `originals` that re-keys legacy Apple records and
    /// projects them as an import would; `None` re-keys nothing.
    pub apple_rekey: Option<Importer>,
}

impl fmt::Debug for KeyedNameStores {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyedNameStores")
            .field("uploads", &self.uploads)
            .field("apple_rekey", &self.apple_rekey.is_some())
            .finish_non_exhaustive()
    }
}

/// The consent grant receipts the worker drains, and the issuer they name.
pub struct GrantReceiptOutbox {
    /// The receipts: written by the grant writes, drained here.
    pub events: Arc<dyn OAuthGrantEventRepository>,
    /// The OAuth issuer URL: a receipt's `iss`, and the host its `kid`
    /// names the checkpoint key under, as `/.well-known/did.json` does.
    pub issuer_url: String,
}

impl fmt::Debug for GrantReceiptOutbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GrantReceiptOutbox")
            .field("issuer_url", &self.issuer_url)
            .finish_non_exhaustive()
    }
}

/// Why one grant receipt could not go through the outbox.
enum ReceiptFailure {
    /// The row itself is at fault, so no retry repairs it: counted, and
    /// poisoned at [`MAX_GRANT_RECEIPT_ATTEMPTS`].
    Row(AppError),
    /// The store, the database or the signer failed: retried uncounted, so
    /// an outage never poisons the receipts waiting behind it.
    Transient(AppError),
}

/// Report a transient failure to `step` `receipt`: it is retried next tick,
/// and nothing is counted against it.
fn transient_receipt_failure(receipt: &OAuthGrantEvent, step: &str, error: &AppError) {
    warn!(
        receipt_id = %receipt.id,
        error = %error,
        "A grant receipt could not be {step}; the failure is not its own, so it is retried next tick uncounted"
    );
}

/// The `record_log` worker's dependencies.
pub struct RecordLogWorker {
    records: Arc<dyn ActivityRecordStore>,
    tombstones: Arc<dyn ImportTombstoneRepository>,
    receipts: Arc<dyn AccountDeletionRepository>,
    users: Arc<dyn UserRepository>,
    /// Consent grant receipts: the outbox the grant steps drain.
    grant_receipts: GrantReceiptOutbox,
    signer: Option<Arc<dyn Signer>>,
    /// The stores the keyed-name backfills rename originals and uploaded
    /// files in; `None` without a blob store, where neither is kept.
    renames: Option<KeyedNameStores>,
    /// Where the orphan sweep resumes: one page per tick, wrapping around.
    orphan_cursor: Mutex<Option<KeyedDigest>>,
    /// Where the keyed-name backfill resumes: one page per tick, wrapping around.
    keyed_names: Mutex<KeyedNamesSweep>,
    /// Where the uploaded files' copy resumes: one page per tick, one table
    /// after the other, wrapping around.
    upload_names: Mutex<UploadNamesSweep>,
    /// Where the legacy Apple re-key resumes: one page per tick, wrapping
    /// around.
    legacy_apple: Mutex<LegacyAppleSweep>,
}

impl fmt::Debug for RecordLogWorker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordLogWorker")
            .field("signs", &self.signer.is_some())
            .field("renames", &self.renames.is_some())
            .finish_non_exhaustive()
    }
}

impl RecordLogWorker {
    /// The worker over `records`, the platform's repositories, the consent
    /// grant receipts to drain (`grant_receipts`), `signer` (`None`: no
    /// checkpoint is cut and no grant receipt signed) and `renames`, the
    /// stores whose content-named originals and uploaded files it renames
    /// (`None`: no blob store, no rename).
    #[must_use]
    pub fn new(
        records: Arc<dyn ActivityRecordStore>,
        tombstones: Arc<dyn ImportTombstoneRepository>,
        receipts: Arc<dyn AccountDeletionRepository>,
        users: Arc<dyn UserRepository>,
        grant_receipts: GrantReceiptOutbox,
        signer: Option<Arc<dyn Signer>>,
        renames: Option<KeyedNameStores>,
    ) -> Self {
        Self {
            records,
            tombstones,
            receipts,
            users,
            grant_receipts,
            signer,
            renames,
            orphan_cursor: Mutex::new(None),
            keyed_names: Mutex::new(KeyedNamesSweep::default()),
            upload_names: Mutex::new(UploadNamesSweep::default()),
            legacy_apple: Mutex::new(LegacyAppleSweep::default()),
        }
    }

    /// One tick.
    ///
    /// # Errors
    ///
    /// The first step that fails; the steps before it stay done.
    pub async fn run(&self) -> AppResult<RecordLogTick> {
        let barrage = |e: BarrageError| AppError::internal(format!("record_log: {e}"));
        let mut tick = RecordLogTick {
            erasures_keyed: self
                .records
                .backfill_erased_keys(BACKFILL_PER_TICK)
                .await
                .map_err(barrage)?,
            tombstones_keyed: self
                .tombstones
                .backfill_identity_keys(i64::from(BACKFILL_PER_TICK))
                .await?,
            baselined: self
                .records
                .baseline(BACKFILL_PER_TICK)
                .await
                .map_err(barrage)?,
            ..RecordLogTick::default()
        };
        if let Some(renames) = &self.renames {
            tick.keyed_names = Some(self.rename_originals(&renames.originals).await?);
            tick.upload_names = Some(self.rename_uploads(&renames.uploads).await?);
        }
        tick.orphans_swept = self.sweep_orphans().await?;
        if let Some(signer) = &self.signer {
            tick.anchored = self.anchor(signer.as_ref()).await?;
        }
        tick.grant_receipts_baselined = self
            .grant_receipts
            .events
            .baseline_live_grants(BACKFILL_PER_TICK)
            .await?;
        tick.grant_receipts_logged = self.log_grant_receipts().await?;
        if let Some(signer) = &self.signer {
            tick.grant_receipts_signed = self.sign_grant_receipts(signer.as_ref()).await?;
        }
        if let Some(importer) = self
            .renames
            .as_ref()
            .and_then(|renames| renames.apple_rekey.as_ref())
        {
            tick.legacy_apple = Some(self.rekey_legacy_apple(importer).await?);
        }
        Ok(tick)
    }

    /// Re-key one page of Apple records still under their legacy id, after
    /// where the last tick stopped, and log what remains: once it equals
    /// what the sweep refused, every record that can move has.
    async fn rekey_legacy_apple(&self, importer: &Importer) -> AppResult<LegacyApplePage> {
        let cursor = self
            .legacy_apple
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cursor
            .clone();
        let report = importer
            .rekey_legacy_apple(cursor.as_ref(), BACKFILL_PER_TICK)
            .await
            .map_err(|e| AppError::internal(format!("record_log legacy apple ids: {e}")))?;
        let sweep_done = report.next.is_none();
        let swept_refused = {
            let mut sweep = self
                .legacy_apple
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            sweep.refused += report.refused;
            let refused = sweep.refused;
            if sweep_done {
                *sweep = LegacyAppleSweep::default();
            } else {
                sweep.cursor.clone_from(&report.next);
            }
            refused
        };
        info!(
            rekeyed = report.rekeyed,
            refused = report.refused,
            remaining = report.remaining,
            sweep_refused = swept_refused,
            sweep_done,
            done = sweep_done && report.remaining == swept_refused,
            "Legacy Apple id re-key page"
        );
        Ok(LegacyApplePage {
            rekeyed: report.rekeyed,
            refused: report.refused,
            remaining: report.remaining,
            sweep_done,
        })
    }

    /// Append one page of unlogged grant receipts to their athletes' logs
    /// and mark each logged. A receipt whose own row is at fault has its
    /// failure counted ([`Self::note_receipt_failure`]); one the store or the
    /// database failed is retried next tick uncounted; `log_grant` appends nothing for an
    /// entry the log already holds, so a receipt appended before a crash is
    /// only marked.
    async fn log_grant_receipts(&self) -> AppResult<u64> {
        let events = &self.grant_receipts.events;
        let mut logged = 0;
        for receipt in events.list_unlogged(BACKFILL_PER_TICK).await? {
            match self.log_grant_receipt(&receipt).await {
                Ok(()) => logged += 1,
                Err(ReceiptFailure::Row(error)) => {
                    self.note_receipt_failure(&receipt, "logged", &error).await;
                }
                Err(ReceiptFailure::Transient(error)) => {
                    transient_receipt_failure(&receipt, "logged", &error);
                }
            }
        }
        Ok(logged)
    }

    /// Count `receipt`'s own failure to be `step`: a fault of the row itself,
    /// which no retry repairs (an id that is no UUID, a digest that does not
    /// read). It is
    /// retried behind every receipt not yet tried, and once it has failed
    /// [`MAX_GRANT_RECEIPT_ATTEMPTS`] times it is poisoned: both scans skip it
    /// from then on, and it is reported at error level with its id so it can
    /// be repaired by hand.
    async fn note_receipt_failure(&self, receipt: &OAuthGrantEvent, step: &str, error: &AppError) {
        let counted = self
            .grant_receipts
            .events
            .record_failure(
                &receipt.tenant_id,
                &receipt.user_id,
                &receipt.id,
                Timestamp::now(),
            )
            .await;
        match counted {
            Ok(Some(attempts)) if attempts >= MAX_GRANT_RECEIPT_ATTEMPTS => error!(
                receipt_id = %receipt.id,
                attempts,
                error = %error,
                "A grant receipt could not be {step} and is poisoned: the outbox skips it from now on"
            ),
            Ok(attempts) => warn!(
                receipt_id = %receipt.id,
                attempts = attempts.unwrap_or_default(),
                error = %error,
                "A grant receipt could not be {step}; it is retried behind the receipts not yet tried"
            ),
            Err(counting) => warn!(
                receipt_id = %receipt.id,
                error = %error,
                counting_error = %counting,
                "A grant receipt could not be {step}, nor its failure counted; it is retried next tick"
            ),
        }
    }

    /// Append `receipt`'s entry to its athlete's log, then mark it logged.
    /// A row that cannot name its scope or digest is the row's own failure;
    /// a store or database that fails is transient.
    async fn log_grant_receipt(&self, receipt: &OAuthGrantEvent) -> Result<(), ReceiptFailure> {
        let uuid = |field: &str, value: &str| {
            Uuid::parse_str(value).map_err(|e| {
                ReceiptFailure::Row(AppError::internal(format!(
                    "grant receipt {}: {field} `{value}`: {e}",
                    receipt.id
                )))
            })
        };
        let scope = Scope::new(
            uuid("tenant_id", &receipt.tenant_id)?,
            uuid("user_id", &receipt.user_id)?,
        );
        let kind = match receipt.event {
            GrantEvent::Issue => EntryKind::GrantIssue,
            GrantEvent::Revoke => EntryKind::GrantRevoke,
            GrantEvent::Expire => EntryKind::GrantExpire,
        };
        let content = Sha256::parse_hex(&receipt.payload_sha256).map_err(|e| {
            ReceiptFailure::Row(AppError::internal(format!(
                "grant receipt {}: {e}",
                receipt.id
            )))
        })?;
        self.records
            .log_grant(&scope, kind, &receipt.client_grant_id, content)
            .await
            .map_err(|e| {
                ReceiptFailure::Transient(AppError::internal(format!(
                    "record_log grant entry: {e}"
                )))
            })?;
        self.grant_receipts
            .events
            .mark_logged(
                &receipt.tenant_id,
                &receipt.user_id,
                &receipt.id,
                Timestamp::now(),
            )
            .await
            .map_err(ReceiptFailure::Transient)?;
        Ok(())
    }

    /// Sign one page of unsigned grant receipts with `signer`. A receipt
    /// that cannot be signed or stored is retried next tick, uncounted: the
    /// signer or the database failed, not the receipt.
    async fn sign_grant_receipts(&self, signer: &dyn Signer) -> AppResult<u64> {
        let GrantReceiptOutbox { events, issuer_url } = &self.grant_receipts;
        let mut stamped = 0;
        for receipt in events.list_unsigned(BACKFILL_PER_TICK).await? {
            let stored =
                match grant_receipt_jws::sign_receipt(signer, issuer_url, &receipt.payload).await {
                    Ok(jws) => {
                        events
                            .set_jws(&receipt.tenant_id, &receipt.user_id, &receipt.id, &jws)
                            .await
                    }
                    Err(error) => Err(error),
                };
            // Signing fails only on the signer or the issuer configuration,
            // never on one receipt's own content: no failure here is counted.
            match stored {
                Ok(_) => stamped += 1,
                Err(error) => transient_receipt_failure(&receipt, "signed", &error),
            }
        }
        Ok(stamped)
    }

    /// Rename one page of originals still named by their content, after
    /// where the last tick stopped. The page that ends a sweep logs what
    /// remains and what the sweep could not read: once they are equal, every
    /// readable original has its keyed name.
    async fn rename_originals(&self, originals: &BarrageStores) -> AppResult<KeyedNamesPage> {
        let cursor = self
            .keyed_names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cursor
            .clone();
        let report = backfill_keyed_names(originals, cursor.as_ref(), BACKFILL_PER_TICK)
            .await
            .map_err(|e| AppError::internal(format!("record_log keyed names: {e}")))?;
        let sweep_done = report.next.is_none();
        let swept_unreadable = {
            let mut sweep = self
                .keyed_names
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            sweep.unreadable += report.unreadable;
            let unreadable = sweep.unreadable;
            if sweep_done {
                *sweep = KeyedNamesSweep::default();
            } else {
                sweep.cursor.clone_from(&report.next);
            }
            unreadable
        };
        if sweep_done {
            info!(
                remaining = report.remaining,
                unreadable = swept_unreadable,
                done = report.remaining == swept_unreadable,
                "Keyed-name backfill sweep complete"
            );
        }
        Ok(KeyedNamesPage {
            repointed: u64::try_from(report.repointed.len()).unwrap_or(u64::MAX),
            unreadable: report.unreadable,
            remaining: report.remaining,
            sweep_done,
        })
    }

    /// Copy one page of uploaded files still named by their content to their
    /// keyed names, in the table the sweep is on, after where the last tick
    /// stopped. The page that ends the last table's sweep counts what
    /// remains over every table and logs it with what the sweep could not
    /// read: once they are equal, every readable upload has its keyed name.
    async fn rename_uploads(&self, uploads: &OriginalsStore) -> AppResult<UploadNamesPage> {
        let (phase, cursor) = {
            let sweep = self
                .upload_names
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            (sweep.phase, sweep.cursor.clone())
        };
        let holder = UploadHolder::ALL
            .get(phase)
            .copied()
            .unwrap_or(UploadHolder::ActivityFiles);
        let page = uploads
            .rename_uploads_page(holder, cursor.as_ref(), BACKFILL_PER_TICK)
            .await?;
        let (sweep_done, swept_unreadable) = {
            let mut sweep = self
                .upload_names
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            sweep.unreadable += page.unreadable;
            let unreadable = sweep.unreadable;
            let sweep_done = match page.next {
                Some(next) => {
                    sweep.cursor = Some(next);
                    false
                }
                None if phase + 1 < UploadHolder::ALL.len() => {
                    sweep.phase = phase + 1;
                    sweep.cursor = None;
                    false
                }
                None => {
                    *sweep = UploadNamesSweep::default();
                    true
                }
            };
            (sweep_done, unreadable)
        };
        let remaining = if sweep_done {
            let mut remaining = 0;
            for holder in UploadHolder::ALL {
                remaining += uploads.count_content_named(holder).await?;
            }
            info!(
                remaining,
                unreadable = swept_unreadable,
                done = remaining == swept_unreadable,
                "Uploaded files keyed-name backfill sweep complete"
            );
            Some(remaining)
        } else {
            None
        };
        Ok(UploadNamesPage {
            holder,
            copied: page.copied,
            unreadable: page.unreadable,
            remaining,
        })
    }

    /// Delete the log of every account on one page of heads that the
    /// platform no longer holds.
    async fn sweep_orphans(&self) -> AppResult<u64> {
        let after = self
            .orphan_cursor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let heads = self
            .records
            .log_heads(after.as_ref(), LOG_PAGE)
            .await
            .map_err(|e| AppError::internal(format!("record_log heads: {e}")))?;
        let next = if heads.len() == LOG_PAGE as usize {
            heads.last().map(|(_, head)| head.keyed_scope.clone())
        } else {
            None
        };
        *self
            .orphan_cursor
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = next;
        let users: Vec<Uuid> = heads.iter().map(|(scope, _)| scope.user_id.0).collect();
        let held = self.users.get_global_many(&users).await?;
        let mut swept = 0;
        for user in users.into_iter().filter(|user| !held.contains_key(user)) {
            self.records
                .delete_user(UserId(user))
                .await
                .map_err(|e| AppError::internal(format!("record_log orphan sweep: {e}")))?;
            warn!(
                subject_hash = %subject_hash(user),
                "Deleted the record_log of an account the platform no longer holds"
            );
            swept += 1;
        }
        Ok(swept)
    }

    /// Cut a checkpoint anchoring the receipt chain's head, and record it.
    async fn anchor(&self, signer: &dyn Signer) -> AppResult<Option<ReceiptAnchor>> {
        let (seq, hash) = self
            .receipts
            .receipt_chain_head()
            .await?
            .unwrap_or_else(|| (0, hex::encode([0_u8; 32])));
        let anchor = Anchor {
            seq: u64::try_from(seq).map_err(|_| {
                AppError::internal(format!("receipt chain head has a negative seq {seq}"))
            })?,
            hash: Sha256::parse_hex(&hash)
                .map_err(|e| AppError::internal(format!("receipt chain head: {e}")))?,
        };
        let Some(checkpoint) = cut_checkpoint(
            self.records.as_ref(),
            signer,
            anchor,
            CHECKPOINT_ORIGIN,
            Timestamp::now(),
        )
        .await
        .map_err(|e| AppError::internal(format!("record_log checkpoint: {e}")))?
        else {
            return self.repair_anchor(seq, hash, &anchor).await;
        };
        // Logged before the anchor row is written: the copy outside the
        // database must not depend on that write, since a cut checkpoint is
        // never cut again for an unchanged tree.
        info!(
            target: "dravr::checkpoint",
            checkpoint_id = checkpoint.id,
            tree_size = checkpoint.size,
            receipt_seq = seq,
            note = %checkpoint.note,
            "Signed record_log checkpoint"
        );
        let recorded = anchor_row(seq, hash, checkpoint)?;
        self.receipts.record_anchor(&recorded).await?;
        Ok(Some(recorded))
    }

    /// Record the anchor of the newest checkpoint when its row is missing:
    /// a cut whose anchor write failed is repaired on the next tick, not at
    /// the next change to the tree, since an unchanged tree is never cut
    /// again.
    ///
    /// The row is written only when the newest checkpoint commits to the
    /// chain head the tick read (`anchor`, at `seq` / `hash`), which barrage
    /// answers ([`standing_checkpoint`]): a checkpoint another instance cut
    /// over another head is left to it, and so is one cut between the two
    /// reads (the standing checkpoint must be the newest read first). The
    /// tree is read only when a row is missing, not on every tick.
    async fn repair_anchor(
        &self,
        seq: i64,
        hash: String,
        anchor: &Anchor,
    ) -> AppResult<Option<ReceiptAnchor>> {
        let barrage =
            |e: BarrageError| AppError::internal(format!("record_log anchor repair: {e}"));
        let Some(latest) = self.records.latest_checkpoint().await.map_err(barrage)? else {
            return Ok(None);
        };
        if self
            .receipts
            .latest_anchor()
            .await?
            .is_some_and(|recorded| recorded.checkpoint_id == latest.id)
        {
            return Ok(None);
        }
        let Some(standing) = standing_checkpoint(self.records.as_ref(), *anchor)
            .await
            .map_err(barrage)?
            .filter(|standing| standing.id == latest.id)
        else {
            return Ok(None);
        };
        let recorded = anchor_row(seq, hash, standing)?;
        self.receipts.record_anchor(&recorded).await?;
        warn!(
            checkpoint_id = recorded.checkpoint_id,
            receipt_seq = seq,
            "Recorded the missing anchor of the newest record_log checkpoint"
        );
        Ok(Some(recorded))
    }
}

/// The anchor row naming `checkpoint` and the receipt chain head at `seq` /
/// `hash` it commits to.
fn anchor_row(seq: i64, hash: String, checkpoint: Checkpoint) -> AppResult<ReceiptAnchor> {
    Ok(ReceiptAnchor {
        receipt_seq: seq,
        receipt_hash: hash,
        checkpoint_id: checkpoint.id,
        tree_size: i64::try_from(checkpoint.size).map_err(|_| {
            AppError::internal(format!(
                "checkpoint {} has a size {} past i64",
                checkpoint.id, checkpoint.size
            ))
        })?,
        tree_root: checkpoint.root.to_hex(),
        note: checkpoint.note,
        anchored_at: Timestamp::now(),
    })
}

/// Start the worker on [`spawn_periodic`], hourly.
pub fn start_record_log_worker(
    worker: Arc<RecordLogWorker>,
    ledger: Arc<dyn WorkerRunRepository>,
) -> AbortHandle {
    let signs = worker.signer.is_some();
    let handle = spawn_periodic(WORKER_NAME, INTERVAL, ledger, move || {
        let worker = Arc::clone(&worker);
        async move { worker.run().await.map(|_| ()) }
    });
    info!(signs, "record_log worker started (hourly)");
    handle
}
