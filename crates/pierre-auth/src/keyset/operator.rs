// ABOUTME: The operator's keyset steps behind `pierre-cli key`: adopt, rotate, enable, destroy, status
// ABOUTME: Adopt and destroy are the one-way steps; each is guarded, and every change is a compare-and-set on the document

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::backends::factory::Database;
use pierre_database::backends::shared::key_material::StoredSecretKind;
use pierre_database::backends::shared::keyring::KeyFamily;
use pierre_database::repositories::security::KeyLifecycleEvent;
use tracing::info;

use super::controller::{count_remaining, record_transitions, stage_key, write_keyset, SweepState};
use super::document::{KeyState, KeyVersion, KeysetDocument, KEYSET_SECRET};
use super::events;
use super::loader::read_keyset;
use super::secrets::{load_key, read_legacy_active_version, secret_name, ACTIVE_VERSION_SECRET};
use crate::key_management::KeyManager;

/// One key version as `key status` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionStatus {
    /// Its family.
    pub family: KeyFamily,
    /// Its version.
    pub version: u32,
    /// Its lifecycle record.
    pub key: KeyVersion,
    /// For a loaded non-primary version, the rows still under it; `None`
    /// for a primary, or when the count failed.
    pub remaining: Option<u64>,
}

/// What `key status` shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeysetStatus {
    /// Whether a keyset document exists.
    pub adopted: bool,
    /// The document revision.
    pub revision: u64,
    /// Every version of both families.
    pub versions: Vec<VersionStatus>,
    /// Rows under the data primary's predecessors, by kind (non-primary only).
    pub rows_not_under_primary: Vec<(StoredSecretKind, u64)>,
    /// The controller's sweep state as of its last tick.
    pub sweep: SweepState,
    /// The legacy layout's active version, before adoption.
    pub legacy_active_version: Option<u32>,
}

impl KeyManager {
    /// Adopt the legacy key layout: write the keyset document (the legacy
    /// active version primary, every older data version `decrypt_only`,
    /// index v1 primary) and delete the legacy active-version secret, in one
    /// transaction. The document written, or `None` when the keyset was
    /// already adopted.
    ///
    /// One-way: from here on nothing reads the active-version secret, and a
    /// binary that predates the keyset reads the layout as version 1 only.
    ///
    /// # Errors
    ///
    /// When a legacy version cannot be read and unwrapped (each must, before
    /// anything is written), or the write fails.
    pub async fn adopt(&self, database: &Database) -> AppResult<Option<KeysetDocument>> {
        let security = database.as_security_repository();
        if read_keyset(security).await?.is_some() {
            return Ok(None);
        }
        let active = read_legacy_active_version(security).await?;
        for version in 1..=active {
            load_key(security, self.kek().as_ref(), KeyFamily::Data, version).await?;
        }
        let now = Utc::now();
        let (doc, transitions) = KeysetDocument::adopted(active, now);
        if !security
            .adopt_keyset(KEYSET_SECRET, &doc.to_text()?, ACTIVE_VERSION_SECRET)
            .await?
        {
            return Ok(None);
        }
        info!(
            event = events::ADOPTED,
            data_primary = active,
            "the legacy key layout was adopted into a keyset"
        );
        record_transitions(security, &transitions, now).await;
        self.refresh_keyring(database).await?;
        Ok(Some(doc))
    }

    /// Stage a new key of `family`; the controller promotes it after the
    /// fleet window. The version staged.
    ///
    /// # Errors
    ///
    /// When the keyset is not adopted, the family is mid-rotation, or the
    /// keyset changed concurrently.
    pub async fn rotate(&self, database: &Database, family: KeyFamily) -> AppResult<u32> {
        let security = database.as_security_repository();
        let version = stage_key(security, self.kek().as_ref(), family, Utc::now()).await?;
        self.refresh_keyring(database).await?;
        Ok(version)
    }

    /// Move a disabled version back to `decrypt_only`, so its ciphertext
    /// opens again.
    ///
    /// # Errors
    ///
    /// When the keyset is not adopted, the version is not disabled, its
    /// stored key does not unwrap, or the keyset changed concurrently.
    pub async fn enable(
        &self,
        database: &Database,
        family: KeyFamily,
        version: u32,
    ) -> AppResult<()> {
        let security = database.as_security_repository();
        let (text, mut doc) = Self::adopted_keyset(database).await?;
        let now = Utc::now();
        let transition = doc.enable(family, version, now)?;
        // The key must unwrap before any process is told to load it: an
        // enabled key that does not would fail every instance's reload, and
        // after the fleet window every instance would refuse to seal.
        load_key(security, self.kek().as_ref(), family, version).await?;
        Self::write_or_conflict(database, &text, &doc, &[transition], now).await?;
        self.refresh_keyring(database).await
    }

    /// Destroy a disabled version: refused unless it has been disabled for
    /// 14 days, a fresh count of its rows is zero, and (for v1) both
    /// families' v1 are out of use. The document moves to `destroyed`
    /// first; the wrapped key is deleted second.
    ///
    /// The count covers the stored rows. External holders were counted when
    /// the key was disabled, and a disabled key seals nothing new, so they
    /// cannot have gained a value under it since.
    ///
    /// One-way: a destroyed key is gone.
    ///
    /// # Errors
    ///
    /// Why it was refused, or a failed read or write.
    pub async fn destroy(
        &self,
        database: &Database,
        family: KeyFamily,
        version: u32,
    ) -> AppResult<()> {
        let security = database.as_security_repository();
        let (text, mut doc) = Self::adopted_keyset(database).await?;
        let now = Utc::now();
        doc.check_destroyable(family, version, now)?;
        let demoted_at = doc
            .versions(family)
            .get(&version)
            .and_then(|key| key.demoted_at)
            .unwrap_or(now);
        let mut state = SweepState::read(security).await?;
        let remaining =
            count_remaining(security, &[], &mut state, family, version, demoted_at, now).await?;
        if remaining > 0 {
            return Err(AppError::invalid_input(format!(
                "{family} v{version} still has {remaining} rows under it; enable it, let the sweep finish, and retry"
            )));
        }
        let transition = doc.destroy(family, version, now)?;
        Self::write_or_conflict(database, &text, &doc, &[transition], now).await?;
        // v1 is one row for both families: it goes once neither needs it.
        let other_v1_gone = version != 1 || {
            let other = match family {
                KeyFamily::Data => KeyFamily::Index,
                KeyFamily::Index => KeyFamily::Data,
            };
            doc.versions(other)
                .get(&1)
                .is_none_or(|key| key.state == KeyState::Destroyed)
        };
        if other_v1_gone {
            security
                .delete_system_secret(&secret_name(family, version))
                .await?;
        }
        Ok(())
    }

    /// The keyset's states, ages, rows left under each non-primary version,
    /// and the controller's last sweep state.
    ///
    /// # Errors
    ///
    /// When a read fails.
    pub async fn status(&self, database: &Database) -> AppResult<KeysetStatus> {
        let security = database.as_security_repository();
        let sweep = SweepState::read(security).await?;
        let Some((_, doc)) = read_keyset(security).await? else {
            return Ok(KeysetStatus {
                adopted: false,
                revision: 0,
                versions: Vec::new(),
                rows_not_under_primary: Vec::new(),
                sweep,
                legacy_active_version: Some(read_legacy_active_version(security).await?),
            });
        };
        let now = Utc::now();
        let mut versions = Vec::new();
        let mut scratch = sweep.clone();
        for family in KeyFamily::ALL {
            for (version, key) in doc.versions(family) {
                let remaining = if matches!(key.state, KeyState::DecryptOnly | KeyState::Disabled) {
                    count_remaining(
                        security,
                        &[],
                        &mut scratch,
                        family,
                        version,
                        key.demoted_at.unwrap_or(now),
                        now,
                    )
                    .await
                    .ok()
                } else {
                    None
                };
                versions.push(VersionStatus {
                    family,
                    version,
                    key,
                    remaining,
                });
            }
        }
        let primary = doc.primary(KeyFamily::Data)?;
        let mut rows_not_under_primary = Vec::new();
        for kind in StoredSecretKind::ALL {
            let mut left = 0;
            for version in doc
                .versions(KeyFamily::Data)
                .keys()
                .filter(|v| **v != primary)
            {
                left += security.count_sealed_under(kind, *version).await?;
            }
            rows_not_under_primary.push((kind, left));
        }
        Ok(KeysetStatus {
            adopted: true,
            revision: doc.revision,
            versions,
            rows_not_under_primary,
            sweep,
            legacy_active_version: None,
        })
    }

    /// The latest key lifecycle transitions, newest first.
    ///
    /// # Errors
    ///
    /// When the read fails.
    pub async fn history(
        &self,
        database: &Database,
        limit: i64,
    ) -> AppResult<Vec<KeyLifecycleEvent>> {
        database
            .as_security_repository()
            .list_key_lifecycle(limit)
            .await
    }

    /// Re-read the keyset into this process's keyring, when it was loaded.
    async fn refresh_keyring(&self, database: &Database) -> AppResult<()> {
        if let Some(loader) = self.keyset_loader() {
            loader.refresh(database.as_security_repository()).await?;
        }
        Ok(())
    }

    async fn adopted_keyset(database: &Database) -> AppResult<(String, KeysetDocument)> {
        read_keyset(database.as_security_repository())
            .await?
            .ok_or_else(|| {
                AppError::invalid_input(
                    "the keyset is not adopted yet; run `pierre-cli key adopt` first",
                )
            })
    }

    async fn write_or_conflict(
        database: &Database,
        text: &str,
        doc: &KeysetDocument,
        transitions: &[super::document::Transition],
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        if write_keyset(
            database.as_security_repository(),
            text,
            doc,
            transitions,
            now,
        )
        .await?
        {
            Ok(())
        } else {
            Err(AppError::internal(
                "the keyset changed concurrently; nothing was written, retry",
            ))
        }
    }
}
