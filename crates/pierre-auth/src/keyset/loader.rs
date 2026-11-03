// ABOUTME: Keeps one process's keyring current: reads the keyset document and rebuilds the ring when its revision moves
// ABOUTME: Unwraps each new key version once with the KEK; falls back to the legacy layout until the keyset is adopted

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_database::backends::shared::key_material::KEYSET_RELOAD_FAILED;
use pierre_database::backends::shared::keyring::{KeyFamily, Keyring, KeyringCell};
use pierre_database::backends::SecurityRepository;
use tracing::{error, info};

use super::document::{KeysetDocument, KEYSET_SECRET};
use super::events;
use super::secrets::{load_key, read_legacy_active_version};
use crate::key_management::KekProvider;

/// Reads the keyset and keeps a [`KeyringCell`] current.
pub struct KeysetLoader {
    kek: Arc<dyn KekProvider>,
    cell: Arc<KeyringCell>,
    /// Keys already unwrapped, by family and version: KMS is called once per
    /// new version, never per refresh. Holds only what the last document
    /// read loads.
    unwrapped: Mutex<HashMap<(KeyFamily, u32), Vec<u8>>>,
}

impl fmt::Debug for KeysetLoader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeysetLoader")
            .field("cell", &self.cell)
            .finish_non_exhaustive()
    }
}

/// The stored keyset document and its exact stored text, which a
/// compare-and-set write must name.
pub type StoredKeyset = (String, KeysetDocument);

/// Read the keyset document; `None` before the keyset is adopted.
///
/// # Errors
///
/// When the read fails or the stored document is invalid.
pub async fn read_keyset(security: &dyn SecurityRepository) -> AppResult<Option<StoredKeyset>> {
    match security.get_system_secret(KEYSET_SECRET).await {
        Ok(text) => {
            let doc = KeysetDocument::parse(&text)?;
            Ok(Some((text, doc)))
        }
        Err(e) if e.code == ErrorCode::ResourceNotFound => Ok(None),
        Err(e) => Err(e),
    }
}

impl KeysetLoader {
    /// A loader that unwraps with `kek` and installs into `cell`.
    #[must_use]
    pub fn new(kek: Arc<dyn KekProvider>, cell: Arc<KeyringCell>) -> Self {
        Self {
            kek,
            cell,
            unwrapped: Mutex::new(HashMap::new()),
        }
    }

    /// The cell this loader keeps current.
    #[must_use]
    pub const fn cell(&self) -> &Arc<KeyringCell> {
        &self.cell
    }

    /// Build the keyring from what is stored and install it, whatever the
    /// cell holds: the boot load.
    ///
    /// # Errors
    ///
    /// When the keyset or a key cannot be read or unwrapped.
    pub async fn load(&self, security: &dyn SecurityRepository) -> AppResult<()> {
        let issued = Utc::now();
        let ring = match read_keyset(security).await? {
            Some((_, doc)) => self.ring_from_document(security, &doc).await?,
            None => self.legacy_ring(security).await?,
        };
        let (revision, data, index) = (ring.revision(), ring.data_primary(), ring.index_primary());
        self.cell.install(ring, issued);
        info!(
            event = events::KEYRING_RELOADED,
            revision,
            data_primary = data,
            index_primary = index,
            "keyring loaded"
        );
        Ok(())
    }

    /// Re-read the keyset: rebuild and install the keyring when its revision
    /// moved, otherwise confirm the current one. Whether it rebuilt.
    ///
    /// # Errors
    ///
    /// When the keyset or a new key cannot be read or unwrapped; the cell
    /// keeps its current keyring.
    pub async fn refresh(&self, security: &dyn SecurityRepository) -> AppResult<bool> {
        let issued = Utc::now();
        let current = self.cell.current();
        match read_keyset(security).await? {
            Some((_, doc)) if current.adopted() && doc.revision == current.revision() => {
                self.cell.confirm(issued);
                Ok(false)
            }
            Some((_, doc)) => {
                let ring = self.ring_from_document(security, &doc).await?;
                let (data, index) = (ring.data_primary(), ring.index_primary());
                self.cell.install(ring, issued);
                info!(
                    event = events::KEYRING_RELOADED,
                    revision = doc.revision,
                    data_primary = data,
                    index_primary = index,
                    "keyring reloaded"
                );
                Ok(true)
            }
            // The legacy layout changes only at adoption, so a ring built from
            // it stays current until a document appears.
            None if !current.adopted() => {
                self.cell.confirm(issued);
                Ok(false)
            }
            None => {
                let ring = self.legacy_ring(security).await?;
                self.cell.install(ring, issued);
                info!(
                    event = events::KEYRING_RELOADED,
                    "the keyset document is gone; keyring reloaded from the legacy layout"
                );
                Ok(true)
            }
        }
    }

    /// [`Self::refresh`], logging a failure under
    /// [`KEYSET_RELOAD_FAILED`]: the body of the per-process reload loop.
    pub async fn refresh_or_report(&self, security: &dyn SecurityRepository) {
        if let Err(e) = self.refresh(security).await {
            error!(
                event = KEYSET_RELOAD_FAILED,
                error_code = ?e.code,
                error = %e,
                "keyset reload failed; keeping the current keyring"
            );
        }
    }

    async fn ring_from_document(
        &self,
        security: &dyn SecurityRepository,
        doc: &KeysetDocument,
    ) -> AppResult<Keyring> {
        let mut data = BTreeMap::new();
        for version in doc.loaded(KeyFamily::Data) {
            data.insert(version, self.key(security, KeyFamily::Data, version).await?);
        }
        let mut index = BTreeMap::new();
        for version in doc.loaded(KeyFamily::Index) {
            index.insert(
                version,
                self.key(security, KeyFamily::Index, version).await?,
            );
        }
        let ring = Keyring::new(
            data,
            doc.primary(KeyFamily::Data)?,
            index,
            doc.primary(KeyFamily::Index)?,
            doc.revision,
            true,
        )?;
        // A key the document no longer loads (disabled or destroyed) leaves
        // this process's memory with the ring that dropped it; `key enable`
        // unwraps it again.
        let live: Vec<(KeyFamily, u32)> = KeyFamily::ALL
            .into_iter()
            .flat_map(|family| {
                doc.loaded(family)
                    .into_iter()
                    .map(move |version| Self::slot(family, version))
            })
            .collect();
        self.unwrapped
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|slot, _| live.contains(slot));
        Ok(ring)
    }

    /// Where `family` `version` is cached: index v1 is the v1 data key, one
    /// row and one unwrap.
    const fn slot(family: KeyFamily, version: u32) -> (KeyFamily, u32) {
        if version == 1 {
            (KeyFamily::Data, 1)
        } else {
            (family, version)
        }
    }

    /// The legacy layout: data `1..=active`, `active` primary, and index v1
    /// (the v1 data key) primary. Every version must be stored.
    async fn legacy_ring(&self, security: &dyn SecurityRepository) -> AppResult<Keyring> {
        let active = read_legacy_active_version(security).await?;
        let mut data = BTreeMap::new();
        for version in 1..=active {
            data.insert(version, self.key(security, KeyFamily::Data, version).await?);
        }
        let v1 = data
            .get(&1)
            .cloned()
            .ok_or_else(|| AppError::internal("the legacy layout has no v1 data key"))?;
        Keyring::new(data, active, BTreeMap::from([(1, v1)]), 1, 0, false)
    }

    async fn key(
        &self,
        security: &dyn SecurityRepository,
        family: KeyFamily,
        version: u32,
    ) -> AppResult<Vec<u8>> {
        let slot = Self::slot(family, version);
        if let Some(key) = self.cached(slot) {
            return Ok(key);
        }
        let key = load_key(security, self.kek.as_ref(), slot.0, slot.1).await?;
        self.unwrapped
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(slot, key.clone());
        Ok(key)
    }

    fn cached(&self, slot: (KeyFamily, u32)) -> Option<Vec<u8>> {
        self.unwrapped
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&slot)
            .cloned()
    }
}
