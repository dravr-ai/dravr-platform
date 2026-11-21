// ABOUTME: The hourly record_log worker: keys erasures and tombstones, baselines records, renames originals, sweeps orphans, cuts checkpoints
// ABOUTME: Always started; only the checkpoint cut waits for a signer, which a KMS deployment gets once its key is applied
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
//!    two equal (carnet#1550 then drops the old names);
//! 4. deletes the log of an account the platform no longer holds (an older
//!    binary's account deletion leaves it behind; see the plan's D2b);
//! 5. with a signer, cuts a checkpoint anchoring the deletion receipt chain's
//!    head, records the anchor, and logs the signed note to Cloud Logging
//!    (target `dravr::checkpoint`): the place the database cannot reach.
//!    An unchanged tree is not cut again, so a tick that finds the newest
//!    checkpoint without its anchor row records it then.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::Utc;
use dravr_barrage::blob::backfill_keyed_names;
use dravr_barrage::digest::Sha256;
use dravr_barrage::keyed::{KeyedDigest, Signer};
use dravr_barrage::record_log::checkpoint::Anchor;
use dravr_barrage::record_log::{cut_checkpoint, merkle, RecordLog, LOG_PAGE};
use dravr_barrage::store::{ActivityRecordStore, BarrageStores, RecordPlace};
use dravr_barrage::{BarrageError, UserId};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::{
    subject_hash, AccountDeletionRepository, ImportTombstoneRepository, ReceiptAnchor,
    UserRepository, WorkerRunRepository,
};
use tokio::task::AbortHandle;
use tracing::{info, warn};
use uuid::Uuid;

use crate::barrage_keys::CHECKPOINT_ORIGIN;
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
    /// Deleted accounts whose log was swept.
    pub orphans_swept: u64,
    /// The checkpoint cut, with the receipt it anchors; or the newest
    /// checkpoint whose missing anchor row this tick recorded.
    pub anchored: Option<ReceiptAnchor>,
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

/// The `record_log` worker's dependencies.
pub struct RecordLogWorker {
    records: Arc<dyn ActivityRecordStore>,
    tombstones: Arc<dyn ImportTombstoneRepository>,
    receipts: Arc<dyn AccountDeletionRepository>,
    users: Arc<dyn UserRepository>,
    signer: Option<Arc<dyn Signer>>,
    /// The stores the keyed-name backfill renames originals in; `None`
    /// without a blob store, where no original is kept.
    originals: Option<BarrageStores>,
    /// Where the orphan sweep resumes: one page per tick, wrapping around.
    orphan_cursor: Mutex<Option<KeyedDigest>>,
    /// Where the keyed-name backfill resumes: one page per tick, wrapping around.
    keyed_names: Mutex<KeyedNamesSweep>,
}

impl fmt::Debug for RecordLogWorker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordLogWorker")
            .field("signs", &self.signer.is_some())
            .field("renames_originals", &self.originals.is_some())
            .finish_non_exhaustive()
    }
}

impl RecordLogWorker {
    /// The worker over `records`, the platform's repositories, `signer`
    /// (`None`: no checkpoint is cut) and `originals`, the stores whose
    /// content-named originals it renames (`None`: no blob store, no rename).
    #[must_use]
    pub fn new(
        records: Arc<dyn ActivityRecordStore>,
        tombstones: Arc<dyn ImportTombstoneRepository>,
        receipts: Arc<dyn AccountDeletionRepository>,
        users: Arc<dyn UserRepository>,
        signer: Option<Arc<dyn Signer>>,
        originals: Option<BarrageStores>,
    ) -> Self {
        Self {
            records,
            tombstones,
            receipts,
            users,
            signer,
            originals,
            orphan_cursor: Mutex::new(None),
            keyed_names: Mutex::new(KeyedNamesSweep::default()),
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
        if let Some(originals) = &self.originals {
            tick.keyed_names = Some(self.rename_originals(originals).await?);
        }
        tick.orphans_swept = self.sweep_orphans().await?;
        if let Some(signer) = &self.signer {
            tick.anchored = self.anchor(signer.as_ref()).await?;
        }
        Ok(tick)
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
            Utc::now(),
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
        let recorded = ReceiptAnchor {
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
            note: checkpoint.note.clone(),
            anchored_at: Utc::now(),
        };
        self.receipts.record_anchor(&recorded).await?;
        Ok(Some(recorded))
    }

    /// Record the anchor of the newest checkpoint when its row is missing:
    /// a cut whose anchor write failed is repaired on the next tick, not at
    /// the next change to the tree, since an unchanged tree is never cut
    /// again.
    ///
    /// The row is written only when the newest checkpoint commits to the
    /// chain head the tick read (`anchor`, at `seq` / `hash`): its size and
    /// root are those of that anchor over every athlete's head now. A
    /// checkpoint another instance cut over another head is left to it.
    async fn repair_anchor(
        &self,
        seq: i64,
        hash: String,
        anchor: &Anchor,
    ) -> AppResult<Option<ReceiptAnchor>> {
        let barrage =
            |e: BarrageError| AppError::internal(format!("record_log anchor repair: {e}"));
        let log: &dyn RecordLog = self.records.as_ref();
        let Some(latest) = log.latest_checkpoint().await.map_err(barrage)? else {
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
        let mut leaves = vec![anchor.leaf()];
        let mut after = None;
        loop {
            let page = log
                .log_heads(after.as_ref(), LOG_PAGE)
                .await
                .map_err(barrage)?;
            let full = page.len() == LOG_PAGE as usize;
            after = page.last().map(|(_, head)| head.keyed_scope.clone());
            leaves.extend(page.iter().map(|(_, head)| head.leaf()));
            if !full {
                break;
            }
        }
        if latest.size != leaves.len() as u64 || latest.root != merkle::root(&leaves) {
            return Ok(None);
        }
        let recorded = ReceiptAnchor {
            receipt_seq: seq,
            receipt_hash: hash,
            checkpoint_id: latest.id,
            tree_size: i64::try_from(latest.size).map_err(|_| {
                AppError::internal(format!(
                    "checkpoint {} has a size {} past i64",
                    latest.id, latest.size
                ))
            })?,
            tree_root: latest.root.to_hex(),
            note: latest.note,
            anchored_at: Utc::now(),
        };
        self.receipts.record_anchor(&recorded).await?;
        warn!(
            checkpoint_id = recorded.checkpoint_id,
            receipt_seq = seq,
            "Recorded the missing anchor of the newest record_log checkpoint"
        );
        Ok(Some(recorded))
    }
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
