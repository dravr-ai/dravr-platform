// ABOUTME: The key rotation controller: promotes staged keys, re-seals old rows, disables drained keys, stages scheduled rotations
// ABOUTME: One bounded, idempotent tick; every keyset write is a compare-and-set, so a lost lease never loses an update

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The rotation controller
//!
//! Each tick, once the keyset is adopted:
//!
//! 1. **Promote** a key staged for the fleet window; the old primary becomes
//!    `decrypt_only`.
//! 2. **Sweep** up to [`SWEEP_ROWS_PER_TICK`] stored secrets not sealed under
//!    the data primary: open each, re-seal it under the primary, and write it
//!    back only while the row still holds what was read, so a concurrent
//!    write wins. A legacy plaintext private key is sealed. A row that does
//!    not open is kept in the unreadable set and blocks its key's disable.
//! 3. **Disable** a `decrypt_only` key whose wait has passed and whose count
//!    was zero on two consecutive ticks. A count that fails is not zero.
//! 4. **Stage** a scheduled rotation for a family whose primary is older than
//!    the configured age.
//!
//! Everything a tick does can be undone: a disabled key is enabled back with
//! `pierre-cli key enable`. Only the operator destroys a key.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_database::backends::shared::encryption::{
    is_plaintext_private_key_pem, split_dek_version,
};
use pierre_database::backends::shared::key_material::{
    StoredSecretKind, DRAIN_COUNT_FAILED, SWEEP_ROW_UNREADABLE,
};
use pierre_database::backends::shared::keyring::{
    KeyFamily, Keyring, KeyringCell, SealDomain, FLEET_WINDOW,
};
use pierre_database::backends::SecurityRepository;
use pierre_database::repositories::security::SealedRow;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use super::document::{KeysetDocument, Transition, KEYSET_SECRET};
use super::events;
use super::loader::{read_keyset, KeysetLoader};
use super::secrets::{insert_key_if_absent, secret_name};
use crate::key_management::{DatabaseEncryptionKey, KekProvider};

/// The most rows one tick re-seals.
pub const SWEEP_ROWS_PER_TICK: u64 = 500;

/// Rows read per sweep page.
const SWEEP_PAGE: i64 = 200;

/// The system secret the controller keeps its sweep state in: the rows that
/// do not open and the external holders' last counts. Kept apart from the
/// keyset document so it never bumps the revision.
pub const SWEEP_STATE_SECRET: &str = "keyset_sweep_state";

/// Ciphertext the platform seals but another crate stores: the controller
/// waits for a holder's count before disabling a data key.
#[async_trait]
pub trait CiphertextHolder: Send + Sync {
    /// The name `key status` prints.
    fn name(&self) -> &'static str;
    /// How many values it holds that were sealed before `before`.
    async fn count_sealed_before(&self, before: DateTime<Utc>) -> AppResult<u64>;
}

/// A stored secret that does not open under any loaded key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UnreadableRow {
    /// Its kind, as [`StoredSecretKind::as_str`] names it.
    pub kind: String,
    /// Its row key.
    pub row_key: String,
    /// The data version its tag names.
    pub version: u32,
}

/// What the controller keeps between ticks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SweepState {
    /// Rows that did not open, which block their version's disable.
    pub unreadable: BTreeSet<UnreadableRow>,
    /// Each external holder's count at the last disable check, by name; `None`
    /// when its count failed.
    pub holders: BTreeMap<String, Option<u64>>,
    /// When the last tick ran.
    pub last_tick_at: Option<DateTime<Utc>>,
    /// Consecutive ticks on which a drain count failed.
    #[serde(default)]
    pub failed_count_ticks: u32,
}

impl SweepState {
    /// Read the stored state; empty when none is stored.
    ///
    /// # Errors
    ///
    /// When the read fails or the stored state is invalid.
    pub async fn read(security: &dyn SecurityRepository) -> AppResult<Self> {
        match security.get_system_secret(SWEEP_STATE_SECRET).await {
            Ok(text) => serde_json::from_str(&text).map_err(|e| {
                AppError::internal(format!("The stored sweep state is not valid: {e}"))
            }),
            Err(e) if e.code == ErrorCode::ResourceNotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    async fn write(&self, security: &dyn SecurityRepository) -> AppResult<()> {
        let text = serde_json::to_string(self)
            .map_err(|e| AppError::internal(format!("Failed to serialize the sweep state: {e}")))?;
        security
            .update_system_secret(SWEEP_STATE_SECRET, &text)
            .await
    }

    /// The unreadable rows under data `version`.
    #[must_use]
    pub fn unreadable_under(&self, version: u32) -> u64 {
        self.unreadable
            .iter()
            .filter(|row| row.version == version)
            .count() as u64
    }
}

/// What the sweep did with one row.
enum RowOutcome {
    /// Re-sealed under the data primary.
    Resealed,
    /// A concurrent write changed the row first; it won.
    Raced,
    /// It does not open under any loaded key.
    Unreadable(UnreadableRow, AppError),
}

/// What one tick did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    /// The keyset changes it made.
    pub transitions: Vec<Transition>,
    /// Rows re-sealed under the data primary.
    pub resealed: u64,
    /// Rows that did not open.
    pub unreadable: u64,
    /// Whether the sweep reached the end of every kind.
    pub sweep_complete: bool,
}

/// Write `doc` over the stored `expected` text and record `transitions`;
/// whether the write won.
///
/// # Errors
///
/// When the write fails.
pub async fn write_keyset(
    security: &dyn SecurityRepository,
    expected: &str,
    doc: &KeysetDocument,
    transitions: &[Transition],
    now: DateTime<Utc>,
) -> AppResult<bool> {
    if !security
        .compare_and_swap_system_secret(KEYSET_SECRET, expected, &doc.to_text()?)
        .await?
    {
        return Ok(false);
    }
    record_transitions(security, transitions, now).await;
    Ok(true)
}

/// Log each transition under its `key_lifecycle.*` event and record it in
/// `audit_events`. A failed record is logged; the keyset already moved.
pub async fn record_transitions(
    security: &dyn SecurityRepository,
    transitions: &[Transition],
    now: DateTime<Utc>,
) {
    for transition in transitions {
        info!(
            event = transition.event(),
            key_family = %transition.family,
            key_version = transition.version,
            from = transition.from.map(super::document::KeyState::as_str),
            to = transition.to.as_str(),
            "{}",
            transition.message()
        );
        if let Err(e) = security
            .record_key_lifecycle(&transition.message(), transition.metadata(), now)
            .await
        {
            warn!(error = %e, "{} was not recorded in audit_events", transition.message());
        }
    }
}

/// Stage a new key of `family`: generate it, store it wrapped, and record it
/// as staged. The version staged.
///
/// # Errors
///
/// When the keyset is not adopted, the family is mid-rotation, or another
/// writer staged or changed the keyset concurrently (nothing is left stored).
pub async fn stage_key(
    security: &dyn SecurityRepository,
    kek: &dyn KekProvider,
    family: KeyFamily,
    now: DateTime<Utc>,
) -> AppResult<u32> {
    let (text, mut doc) = read_keyset(security).await?.ok_or_else(|| {
        AppError::invalid_input("the keyset is not adopted yet; run `pierre-cli key adopt` first")
    })?;
    let version = doc.next_version(family)?;
    let key = DatabaseEncryptionKey::generate();
    let wrapped = kek.wrap(key.as_bytes()).await?;
    if !insert_key_if_absent(security, family, version, &wrapped).await? {
        return Err(AppError::internal(format!(
            "{family} v{version} was stored by a concurrent rotation; retry"
        )));
    }
    let transition = doc.stage(family, version, now)?;
    if !write_keyset(security, &text, &doc, &[transition], now).await? {
        security
            .delete_system_secret(&secret_name(family, version))
            .await?;
        return Err(AppError::internal(
            "the keyset changed while staging; nothing was staged, retry",
        ));
    }
    Ok(version)
}

/// Count what is still sealed or hashed under `family` `version`.
///
/// The stored rows (and, for data, the unreadable ones), plus for data each
/// external holder's values sealed before `demoted_at` + the fleet window.
/// Holder counts are written into `state`.
///
/// # Errors
///
/// When any count fails: a failed count is never zero.
pub async fn count_remaining(
    security: &dyn SecurityRepository,
    holders: &[Arc<dyn CiphertextHolder>],
    state: &mut SweepState,
    family: KeyFamily,
    version: u32,
    demoted_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> AppResult<u64> {
    match family {
        KeyFamily::Index => security.count_live_index_hashes(version, now).await,
        KeyFamily::Data => {
            let mut total = state.unreadable_under(version);
            for kind in StoredSecretKind::ALL {
                total += security.count_sealed_under(kind, version).await?;
            }
            let before = demoted_at
                + chrono::Duration::from_std(FLEET_WINDOW).unwrap_or(chrono::Duration::MAX);
            let mut failed = None;
            for holder in holders {
                match holder.count_sealed_before(before).await {
                    Ok(n) => {
                        state.holders.insert(holder.name().to_owned(), Some(n));
                        total += n;
                    }
                    Err(e) => {
                        state.holders.insert(holder.name().to_owned(), None);
                        failed = Some(e);
                    }
                }
            }
            failed.map_or(Ok(total), Err)
        }
    }
}

/// The key rotation controller.
pub struct KeysetController {
    loader: Arc<KeysetLoader>,
    kek: Arc<dyn KekProvider>,
    holders: Vec<Arc<dyn CiphertextHolder>>,
    rotation_days: u32,
}

impl fmt::Debug for KeysetController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeysetController")
            .field(
                "holders",
                &self
                    .holders
                    .iter()
                    .map(|h| h.name().to_owned())
                    .collect::<Vec<_>>(),
            )
            .field("rotation_days", &self.rotation_days)
            .finish_non_exhaustive()
    }
}

impl KeysetController {
    /// A controller over this process's loader, staging scheduled rotations
    /// after `rotation_days` (0: never), waiting on `holders`.
    #[must_use]
    pub fn new(
        loader: Arc<KeysetLoader>,
        kek: Arc<dyn KekProvider>,
        holders: Vec<Arc<dyn CiphertextHolder>>,
        rotation_days: u32,
    ) -> Self {
        Self {
            loader,
            kek,
            holders,
            rotation_days,
        }
    }

    /// Run one tick at `now`. Nothing happens before the keyset is adopted.
    ///
    /// # Errors
    ///
    /// When a read or write the tick needs fails; whatever it wrote before
    /// stays written, and the next tick picks up from the stored state.
    pub async fn tick(
        &self,
        security: &dyn SecurityRepository,
        now: DateTime<Utc>,
    ) -> AppResult<TickReport> {
        let mut report = TickReport::default();
        let Some((text, mut doc)) = read_keyset(security).await? else {
            return Ok(report);
        };

        let promoted = doc.promote_due(now);
        if !promoted.is_empty() && write_keyset(security, &text, &doc, &promoted, now).await? {
            report.transitions.extend(promoted);
        }
        self.loader.refresh(security).await?;

        let mut state = SweepState::read(security).await?;
        self.sweep(security, &mut state, &mut report).await?;

        if let Some((text, mut doc)) = read_keyset(security).await? {
            let disabled = self
                .check_disables(security, &mut doc, &mut state, now)
                .await;
            let before = KeysetDocument::parse(&text)?;
            if doc != before && write_keyset(security, &text, &doc, &disabled, now).await? {
                report.transitions.extend(disabled);
                self.loader.refresh(security).await?;
            }
        }
        state.last_tick_at = Some(now);
        state.write(security).await?;

        if let Some((_, doc)) = read_keyset(security).await? {
            for family in KeyFamily::ALL {
                if doc.rotation_due(family, self.rotation_days, now) {
                    let version = stage_key(security, self.kek.as_ref(), family, now).await?;
                    info!(event = events::SCHEDULED, key_family = %family, key_version = version, "scheduled rotation staged");
                }
            }
        }
        Ok(report)
    }

    async fn check_disables(
        &self,
        security: &dyn SecurityRepository,
        doc: &mut KeysetDocument,
        state: &mut SweepState,
        now: DateTime<Utc>,
    ) -> Vec<Transition> {
        let mut disabled = Vec::new();
        let mut count_failed = false;
        for (family, version, demoted_at) in doc.disable_candidates(now) {
            let drained = match count_remaining(
                security,
                &self.holders,
                state,
                family,
                version,
                demoted_at,
                now,
            )
            .await
            {
                Ok(remaining) => {
                    info!(event = events::DRAIN_COUNTED, key_family = %family, key_version = version, remaining, "drain counted");
                    remaining == 0
                }
                Err(e) => {
                    count_failed = true;
                    Self::report_count_failure(state, family, version, &e);
                    false
                }
            };
            if let Some(transition) = doc.record_drain(family, version, drained, now) {
                disabled.push(transition);
            }
        }
        state.failed_count_ticks = if count_failed {
            state.failed_count_ticks.saturating_add(1)
        } else {
            0
        };
        disabled
    }

    /// One failed count is retried next tick, and its cause is usually a
    /// database that briefly had no free connection, so it logs `WARN`. A count
    /// that also failed last tick is broken, and it pages: the key stays loaded
    /// for as long as it fails, and otherwise nothing says so.
    fn report_count_failure(state: &SweepState, family: KeyFamily, version: u32, e: &AppError) {
        if state.failed_count_ticks == 0 {
            warn!(
                event = events::DRAIN_COUNTED,
                key_family = %family,
                key_version = version,
                error = %e,
                "drain count failed; the key stays loaded"
            );
        } else {
            error!(
                event = DRAIN_COUNT_FAILED,
                key_family = %family,
                key_version = version,
                failed_ticks = state.failed_count_ticks + 1,
                error = %e,
                "drain count keeps failing; the key stays loaded until it works"
            );
        }
    }

    /// Open one row (or take a legacy plaintext key as it is), seal it under
    /// the data primary, and write it back while it still holds what was read.
    async fn reseal_one(
        security: &dyn SecurityRepository,
        cell: &KeyringCell,
        ring: &Keyring,
        kind: StoredSecretKind,
        row: &SealedRow,
    ) -> AppResult<RowOutcome> {
        let domain = SealDomain::Stored(kind);
        let parts = [
            row.parts[0].as_deref(),
            row.parts[1].as_deref(),
            row.parts[2].as_deref(),
        ];
        let opened = kind.row_aad(&row.row_key, parts).and_then(|aad| {
            let plaintext = if kind == StoredSecretKind::RsaPrivateKey
                && is_plaintext_private_key_pem(&row.sealed)
            {
                row.sealed.clone()
            } else {
                ring.open(domain, &row.sealed, &aad)?
            };
            Ok((plaintext, aad))
        });
        let (plaintext, aad) = match opened {
            Ok(opened) => opened,
            Err(e) => {
                let unreadable = UnreadableRow {
                    kind: kind.as_str().to_owned(),
                    row_key: row.row_key.clone(),
                    version: split_dek_version(&row.sealed).0,
                };
                return Ok(RowOutcome::Unreadable(unreadable, e));
            }
        };
        let resealed = cell.seal(domain, &plaintext, &aad)?;
        if security
            .reseal_row(kind, &row.row_key, &row.sealed, &resealed)
            .await?
        {
            Ok(RowOutcome::Resealed)
        } else {
            Ok(RowOutcome::Raced)
        }
    }

    async fn sweep(
        &self,
        security: &dyn SecurityRepository,
        state: &mut SweepState,
        report: &mut TickReport,
    ) -> AppResult<()> {
        let cell: &KeyringCell = self.loader.cell();
        let ring = cell.current();
        let primary = ring.data_primary();
        let mut seen = BTreeSet::new();
        let mut complete = true;
        'kinds: for kind in StoredSecretKind::ALL {
            let mut after = String::new();
            loop {
                let page = security
                    .sealed_rows_not_under(kind, primary, &after, SWEEP_PAGE)
                    .await?;
                let full = page.len() == usize::try_from(SWEEP_PAGE).unwrap_or(usize::MAX);
                for row in page {
                    if report.resealed >= SWEEP_ROWS_PER_TICK {
                        complete = false;
                        break 'kinds;
                    }
                    after.clone_from(&row.row_key);
                    match Self::reseal_one(security, cell, &ring, kind, &row).await? {
                        RowOutcome::Resealed => report.resealed += 1,
                        RowOutcome::Raced => {}
                        RowOutcome::Unreadable(unreadable, e) => {
                            if !state.unreadable.contains(&unreadable) {
                                error!(
                                    event = SWEEP_ROW_UNREADABLE,
                                    secret_kind = kind.as_str(),
                                    table = kind.table(),
                                    column = kind.column(),
                                    row_key = %row.row_key,
                                    dek_version = unreadable.version,
                                    error = %e,
                                    "A stored secret does not open under any loaded key; the sweep skips it and its key stays loaded"
                                );
                            }
                            seen.insert(unreadable);
                        }
                    }
                }
                if !full {
                    break;
                }
            }
        }
        report.unreadable = seen.len() as u64;
        report.sweep_complete = complete;
        // A complete pass saw every unreadable row; a partial one adds to
        // what earlier passes saw.
        if complete {
            state.unreadable = seen;
        } else {
            state.unreadable.extend(seen);
        }
        if report.resealed > 0 {
            info!(
                event = events::SWEPT,
                resealed = report.resealed,
                primary,
                complete,
                "stored secrets re-sealed under the data primary"
            );
        }
        Ok(())
    }
}
