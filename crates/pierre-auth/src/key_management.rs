// ABOUTME: Two-tier key management — a KEK (via KekProvider) wraps the Database Encryption Key (DEK)
// ABOUTME: LocalKekProvider wraps the DEK with the env Master Encryption Key; GcpKmsKekProvider wraps it with Cloud KMS (ADR-017)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeSet;
use std::env;
use std::fmt;
use std::sync::Arc;

use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as Base64Standard;
use base64::Engine as Base64Engine;
use chrono::Utc;
use rand::RngCore;
use tracing::{error, info};

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_database::backends::factory::Database;
use pierre_database::backends::shared::key_material::{
    key_read_event, DEK_MINTED_OVER_CIPHERTEXT, DEK_READ_FAILED,
};
use pierre_database::backends::shared::keyring::KeyFamily;
use pierre_database::backends::SecurityRepository;
use pierre_database::database::generate_encryption_key;

use crate::keyset::secrets::{
    insert_key_if_absent, load_key, read_legacy_active_version, rewrite_key, secret_name, V1_SECRET,
};
use crate::keyset::{read_keyset, KeyState, KeysetDocument, KeysetLoader, KEYSET_SECRET};

/// Key Encryption Key provider — the Tier-1 boundary that wraps/unwraps the DEK.
///
/// The KEK never encrypts application data directly; it only wraps the 32-byte
/// Database Encryption Key (DEK) for storage. This is the seam where the backing
/// key store is swapped: [`LocalKekProvider`] uses an env-supplied master key,
/// while `GcpKmsKekProvider` delegates wrap/unwrap to Cloud KMS without
/// the key material ever leaving KMS (see ADR-017).
#[async_trait]
pub trait KekProvider: Send + Sync {
    /// Wrap (encrypt) DEK key material for at-rest storage.
    ///
    /// Async because a managed-KMS provider performs a network round-trip; the
    /// local provider resolves immediately. Called only at startup and rotation.
    ///
    /// # Errors
    ///
    /// Returns an error if the wrap operation fails.
    async fn wrap(&self, plaintext: &[u8]) -> AppResult<Vec<u8>>;

    /// Unwrap (decrypt) previously wrapped DEK key material.
    ///
    /// # Errors
    ///
    /// Returns an error if the ciphertext is malformed or the unwrap fails.
    async fn unwrap(&self, ciphertext: &[u8]) -> AppResult<Vec<u8>>;
}

/// Master Encryption Key (MEK) — Tier 1 key material loaded from the environment.
///
/// Used by [`LocalKekProvider`] to wrap the DEK. AES-256-GCM with a random nonce
/// prepended to each ciphertext.
pub struct MasterEncryptionKey {
    key: [u8; 32],
}

impl fmt::Debug for MasterEncryptionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MasterEncryptionKey")
            .field("key", &"[REDACTED]")
            .finish()
    }
}

/// Database Encryption Key (DEK) — Tier 2, used for actual data encryption.
///
/// Stored wrapped (KEK-encrypted) in the database; held in memory in plaintext
/// only while the process runs.
pub struct DatabaseEncryptionKey {
    key: [u8; 32],
}

impl fmt::Debug for DatabaseEncryptionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseEncryptionKey")
            .field("key", &"[REDACTED]")
            .finish()
    }
}

impl MasterEncryptionKey {
    /// Create MEK from raw key bytes - primarily for testing
    #[must_use]
    pub const fn from_bytes(key: [u8; 32]) -> Self {
        Self { key }
    }
    /// Load MEK from environment variable (required)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The `PIERRE_MASTER_ENCRYPTION_KEY` environment variable is not set
    /// - The environment variable contains invalid base64 encoding
    /// - The decoded key is not exactly 32 bytes
    pub fn load_or_generate() -> AppResult<Self> {
        env::var("PIERRE_MASTER_ENCRYPTION_KEY").map_or_else(
            |_| {
                Err(AppError::config(
                    "PIERRE_MASTER_ENCRYPTION_KEY environment variable is required.\n\n\
                     CRITICAL: This key MUST be consistent across all server instances.\n\
                     In multi-instance deployments (K8s, GCP), different keys cause data corruption:\n\
                     - Tokens encrypted by one instance become unreadable by others\n\
                     - Users get locked out with cryptic decryption errors\n\n\
                     FOR DEVELOPMENT:\n\
                     \x20\x20export PIERRE_MASTER_ENCRYPTION_KEY=\"$(openssl rand -base64 32)\"\n\n\
                     FOR KUBERNETES/PRODUCTION:\n\
                     \x20\x201. Generate key: openssl rand -base64 32\n\
                     \x20\x202. Create K8s Secret:\n\
                     \x20\x20   kubectl create secret generic pierre-secrets \\\n\
                     \x20\x20     --from-literal=PIERRE_MASTER_ENCRYPTION_KEY=<your-key>\n\
                     \x20\x203. Mount in deployment:\n\
                     \x20\x20   envFrom:\n\
                     \x20\x20     - secretRef:\n\
                     \x20\x20         name: pierre-secrets\n\n\
                     This key encrypts OAuth tokens, admin secrets, and other sensitive data.",
                ))
            },
            |encoded_key| Self::load_from_environment(&encoded_key),
        )
    }

    /// Load MEK from base64-encoded environment variable
    ///
    /// # Errors
    /// Returns error if decoding fails or key is wrong length
    fn load_from_environment(encoded_key: &str) -> AppResult<Self> {
        info!("Loading Master Encryption Key from environment variable");
        let key_bytes = Base64Standard.decode(encoded_key).map_err(|e| {
            AppError::config(format!(
                "Invalid base64 encoding in PIERRE_MASTER_ENCRYPTION_KEY: {e}"
            ))
        })?;

        if key_bytes.len() != 32 {
            return Err(AppError::config(format!(
                "Master encryption key must be exactly 32 bytes, got {} bytes",
                key_bytes.len()
            )));
        }

        let mut key = [0u8; 32];
        key.copy_from_slice(&key_bytes);
        Ok(Self { key })
    }

    /// Get the raw key bytes for encryption operations
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.key
    }

    /// Encrypt data with the MEK (used to wrap the DEK)
    ///
    /// # Errors
    ///
    /// Returns an error if encryption fails
    pub fn encrypt(&self, plaintext: &[u8]) -> AppResult<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| AppError::internal(format!("Invalid key length: {e}")))?;

        // Generate random nonce
        let mut nonce_bytes = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Encrypt the data
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| AppError::internal(format!("Encryption failed: {e}")))?;

        // Prepend nonce to ciphertext
        let mut result = Vec::with_capacity(12 + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);

        Ok(result)
    }

    /// Decrypt data with the MEK (used to unwrap the DEK)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The encrypted data is too short to contain a nonce
    /// - Decryption fails due to invalid data or wrong key
    pub fn decrypt(&self, encrypted_data: &[u8]) -> AppResult<Vec<u8>> {
        if encrypted_data.len() < 12 {
            return Err(AppError::invalid_input("Encrypted data too short"));
        }

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| AppError::internal(format!("Invalid key length: {e}")))?;

        // Extract nonce and ciphertext
        let nonce = Nonce::from_slice(&encrypted_data[..12]);
        let ciphertext = &encrypted_data[12..];

        // Decrypt the data
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| AppError::internal(format!("Decryption failed: {e}")))?;

        Ok(plaintext)
    }
}

/// KEK provider backed by the env-supplied [`MasterEncryptionKey`].
///
/// This is the default provider for development, CI, tests, and any deployment
/// not configured with a managed KMS. Behavior is identical to the historical
/// MEK-wraps-DEK scheme.
#[derive(Debug)]
pub struct LocalKekProvider {
    mek: MasterEncryptionKey,
}

impl LocalKekProvider {
    /// Build a provider from the `PIERRE_MASTER_ENCRYPTION_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns an error if the MEK cannot be loaded from the environment.
    pub fn from_env() -> AppResult<Self> {
        Ok(Self {
            mek: MasterEncryptionKey::load_or_generate()?,
        })
    }

    /// Build a provider from an already-loaded MEK (used for tests and explicit wiring).
    #[must_use]
    pub const fn from_mek(mek: MasterEncryptionKey) -> Self {
        Self { mek }
    }
}

#[async_trait]
impl KekProvider for LocalKekProvider {
    async fn wrap(&self, plaintext: &[u8]) -> AppResult<Vec<u8>> {
        self.mek.encrypt(plaintext)
    }

    async fn unwrap(&self, ciphertext: &[u8]) -> AppResult<Vec<u8>> {
        self.mek.decrypt(ciphertext)
    }
}

impl DatabaseEncryptionKey {
    /// Create a new random DEK
    #[must_use]
    pub fn generate() -> Self {
        let key = generate_encryption_key();
        Self { key }
    }

    /// Create DEK from existing key bytes
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { key: bytes }
    }

    /// Reconstruct a DEK from unwrapped key material, validating its length.
    ///
    /// # Errors
    ///
    /// Returns an error if the material is not exactly 32 bytes.
    pub fn from_unwrapped(bytes: &[u8]) -> AppResult<Self> {
        if bytes.len() != 32 {
            return Err(AppError::internal(format!(
                "Decrypted DEK has invalid length: expected 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(bytes);
        Ok(Self { key })
    }

    /// Get the raw key bytes for database encryption
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.key
    }
}

/// Two-tier key management: a [`KekProvider`] wrapping every key version of
/// the keyset (ADR-017, carnet#1347).
pub struct KeyManager {
    kek: Arc<dyn KekProvider>,
    /// The key a fresh database stores as v1; unused on any other database.
    bootstrap: DatabaseEncryptionKey,
    /// The loader that keeps this process's keyring current, once
    /// [`Self::complete_initialization`] ran.
    loader: Option<Arc<KeysetLoader>>,
}

impl fmt::Debug for KeyManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyManager")
            .field("bootstrap", &self.bootstrap)
            .finish_non_exhaustive()
    }
}

impl KeyManager {
    /// Bootstrap initialization: load the KEK and generate the key the
    /// database handle opens with, which a fresh database stores as v1.
    ///
    /// # Errors
    ///
    /// Returns an error if KEK loading fails
    pub fn bootstrap() -> AppResult<(Self, [u8; 32])> {
        Ok(Self::bootstrap_with(Self::configured_kek_provider()?))
    }

    /// Bootstrap initialization over an explicit KEK provider: the half of
    /// [`Self::bootstrap`] that runs after the provider is chosen.
    #[must_use]
    pub fn bootstrap_with(kek: Box<dyn KekProvider>) -> (Self, [u8; 32]) {
        info!("Bootstrapping two-tier key management system");
        let bootstrap = DatabaseEncryptionKey::generate();
        let database_key = *bootstrap.as_bytes();
        (Self::with_provider(kek, bootstrap), database_key)
    }

    /// Construct a manager from an explicit KEK provider and bootstrap key.
    ///
    /// Used by KEK-migration tooling (to supply the old/source KEK) and tests.
    #[must_use]
    pub fn with_provider(kek: Box<dyn KekProvider>, bootstrap: DatabaseEncryptionKey) -> Self {
        Self {
            kek: Arc::from(kek),
            bootstrap,
            loader: None,
        }
    }

    /// Select the configured KEK provider.
    ///
    /// With the `gcp-kms` feature enabled and `PIERRE_KMS_KEY_RESOURCE` set, keys
    /// are wrapped by GCP Cloud KMS. Otherwise the env-backed local provider is
    /// used (the default for development, CI, and tests).
    ///
    /// # Errors
    ///
    /// Returns an error if the selected provider cannot be constructed.
    pub fn configured_kek_provider() -> AppResult<Box<dyn KekProvider>> {
        #[cfg(feature = "gcp-kms")]
        {
            use crate::gcp_kms::GcpKmsKekProvider;
            if env::var("PIERRE_KMS_KEY_RESOURCE").is_ok() {
                info!("Using GCP Cloud KMS KEK provider");
                return Ok(Box::new(GcpKmsKekProvider::from_env()?));
            }
        }
        info!("Using local env-backed KEK provider");
        Ok(Box::new(LocalKekProvider::from_env()?))
    }

    /// The KEK provider, shared with the loader and the controller.
    #[must_use]
    pub fn kek(&self) -> Arc<dyn KekProvider> {
        Arc::clone(&self.kek)
    }

    /// The loader keeping this process's keyring current, once
    /// [`Self::complete_initialization`] ran: the server and the CLI run its
    /// refresh every [`crate::keyset::RELOAD_PERIOD`].
    #[must_use]
    pub fn keyset_loader(&self) -> Option<Arc<KeysetLoader>> {
        self.loader.clone()
    }

    /// Complete initialization once the database is open: load the keyset
    /// into the database handle's keyring.
    ///
    /// - A keyset document: its loaded versions.
    /// - No document and a v1 key row: the legacy layout (data
    ///   `1..=active`, index v1), until `pierre-cli key adopt`.
    /// - No document and no v1 key row on a database that holds no
    ///   ciphertext: a fresh database. The bootstrap key is stored as v1,
    ///   insert-if-absent, and the keyset document is written.
    ///
    /// A missing v1 row beside ciphertext is a lost key, and the instance
    /// does not boot (carnet#703). Any other read failure is returned, so the
    /// instance never mints a key over the real one (carnet#696).
    ///
    /// # Errors
    ///
    /// Returns an error if database operations or key unwrap fail, or if no
    /// key is stored on a database that holds ciphertext.
    pub async fn complete_initialization(&mut self, database: &Database) -> AppResult<()> {
        info!("Completing two-tier key management initialization");
        let security = database.as_security_repository();
        let keyset = read_keyset(security).await.inspect_err(|e| {
            if let Some(event) = key_read_event(e, DEK_READ_FAILED) {
                error!(
                    event,
                    error_code = ?e.code,
                    error = %e,
                    "Failed to read the stored keyset; refusing to initialize key management"
                );
            }
        })?;
        if keyset.is_none() {
            match security.get_system_secret(V1_SECRET).await {
                Ok(_) => {}
                Err(e) if e.code == ErrorCode::ResourceNotFound => {
                    self.store_first_dek(security).await?;
                    Self::write_fresh_keyset(security).await?;
                }
                Err(e) => {
                    Self::report_stored_dek_read_failure(&e);
                    return Err(e);
                }
            }
        }
        let loader = Arc::new(KeysetLoader::new(
            self.kek(),
            Arc::clone(database.keyring()),
        ));
        loader.load(security).await?;
        self.loader = Some(loader);
        let ring = database.keyring().current();
        info!(
            data_primary = ring.data_primary(),
            index_primary = ring.index_primary(),
            adopted = ring.adopted(),
            "Two-tier key management initialized"
        );
        Ok(())
    }

    /// Write a fresh database's keyset document, unless a racing instance
    /// wrote it first. A legacy active-version secret means the database is
    /// not fresh after all, and it is left to `key adopt`.
    async fn write_fresh_keyset(security: &dyn SecurityRepository) -> AppResult<()> {
        if read_legacy_active_version(security).await? != 1 {
            return Ok(());
        }
        let doc = KeysetDocument::fresh(Utc::now());
        security
            .insert_system_secret_if_absent(KEYSET_SECRET, &doc.to_text()?)
            .await?;
        Ok(())
    }

    /// Store the bootstrap key as version 1 unless another instance already
    /// stored one; the caller then loads whichever key the store holds.
    ///
    /// A database with no key row should hold no ciphertext either. When it
    /// does, either a racing instance stored its key and wrote ciphertext
    /// after this one looked, and that key is adopted, or the row was lost
    /// and a key minted here could open none of it: nothing is stored, the
    /// refusal logs at `ERROR` under [`DEK_MINTED_OVER_CIPHERTEXT`] and the
    /// instance does not boot (carnet#703).
    async fn store_first_dek(&self, security: &dyn SecurityRepository) -> AppResult<()> {
        let encrypted_rows = Self::count_ciphertext_before_mint(security).await?;
        if encrypted_rows > 0 {
            return Self::adopt_racing_dek_or_refuse(security, encrypted_rows).await;
        }
        self.mint_first_dek(security).await
    }

    /// Store this instance's bootstrap key as version 1 on a database that
    /// holds no ciphertext, insert-if-absent.
    async fn mint_first_dek(&self, security: &dyn SecurityRepository) -> AppResult<()> {
        info!("No existing DEK found, storing the bootstrap Database Encryption Key as version 1");
        let wrapped = self.kek.wrap(self.bootstrap.as_bytes()).await?;
        if insert_key_if_absent(security, KeyFamily::Data, 1, &wrapped).await? {
            info!("Database Encryption Key version 1 stored");
        } else {
            info!("Another instance stored DEK version 1 first; adopting the stored key");
        }
        Ok(())
    }

    /// Read the version-1 key row again on a database that holds
    /// `encrypted_rows` ciphertext rows: a racing instance's row is adopted,
    /// a row still missing refuses the boot.
    async fn adopt_racing_dek_or_refuse(
        security: &dyn SecurityRepository,
        encrypted_rows: i64,
    ) -> AppResult<()> {
        match security.get_system_secret(V1_SECRET).await {
            Ok(_) => {
                info!("Another instance stored DEK version 1 first; adopting the stored key");
                Ok(())
            }
            Err(e) if e.code == ErrorCode::ResourceNotFound => {
                error!(
                    event = DEK_MINTED_OVER_CIPHERTEXT,
                    encrypted_rows,
                    "No DEK stored on a database that holds ciphertext; refusing to mint one"
                );
                Err(AppError::config(format!(
                    "No Database Encryption Key is stored, but the database holds \
                     {encrypted_rows} encrypted rows that a new key could not open. \
                     Restore the original wrapped DEK row in system_secrets (the \
                     carnet#696 recovery procedure) before booting."
                )))
            }
            Err(e) => {
                Self::report_stored_dek_read_failure(&e);
                Err(e)
            }
        }
    }

    /// Log a failed read of the stored version-1 key row, unless the database
    /// could not serve it (the boot's key-store queue reports that).
    fn report_stored_dek_read_failure(e: &AppError) {
        if let Some(event) = key_read_event(e, DEK_READ_FAILED) {
            error!(
                event,
                error_code = ?e.code,
                error = %e,
                "Failed to read the stored DEK; refusing to initialize key management"
            );
        }
    }

    /// Count the rows holding ciphertext before a key is minted; a failed
    /// count refuses the mint, since the answer decides whether it is safe.
    async fn count_ciphertext_before_mint(security: &dyn SecurityRepository) -> AppResult<i64> {
        security.count_encrypted_rows().await.inspect_err(|e| {
            if let Some(event) = key_read_event(e, DEK_READ_FAILED) {
                error!(
                    event,
                    error_code = ?e.code,
                    error = %e,
                    "Failed to count encrypted rows before storing a DEK; refusing to initialize key management"
                );
            }
        })
    }

    /// Re-wrap every stored key version under a new KEK provider (KEK
    /// rotation, or a provider migration such as local MEK → Cloud KMS).
    ///
    /// Every version the keyset has not destroyed is re-wrapped, disabled
    /// ones included, in both families (index v1 is the data v1 row); before
    /// adoption, the legacy layout's `1..=active`. The keys inside are
    /// unchanged, so no ciphertext or hash changes. On success the manager
    /// adopts `new_kek`.
    ///
    /// # Errors
    ///
    /// Returns an error if database access or wrap/unwrap fails.
    pub async fn rewrap_deks(
        &mut self,
        new_kek: Box<dyn KekProvider>,
        database: &Database,
    ) -> AppResult<()> {
        info!("Re-wrapping every key version under a new KEK");
        let security = database.as_security_repository();
        let slots = Self::stored_key_slots(security).await?;
        let mut rewrapped = Vec::with_capacity(slots.len());
        for (family, version) in slots {
            let key = load_key(security, self.kek.as_ref(), family, version).await?;
            rewrapped.push((family, version, new_kek.wrap(&key).await?));
        }
        for (family, version, wrapped) in &rewrapped {
            rewrite_key(security, *family, *version, wrapped).await?;
        }
        self.kek = Arc::from(new_kek);
        info!(
            "Re-wrapped {} key version(s) under the new KEK",
            rewrapped.len()
        );
        Ok(())
    }

    /// Every stored key row: each version the keyset has not destroyed, in
    /// both families ([`document_key_slots`]), or before adoption the legacy
    /// layout's `1..=active`.
    async fn stored_key_slots(
        security: &dyn SecurityRepository,
    ) -> AppResult<Vec<(KeyFamily, u32)>> {
        Ok(match read_keyset(security).await? {
            Some((_, doc)) => document_key_slots(&doc),
            None => (1..=read_legacy_active_version(security).await?)
                .map(|version| (KeyFamily::Data, version))
                .collect(),
        })
    }
}

/// The stored key rows of an adopted keyset: each version not destroyed, in
/// both families. Index v1 is the data v1 row, listed once, and listed while
/// either family's v1 is not destroyed, since the row is kept until both are.
fn document_key_slots(doc: &KeysetDocument) -> Vec<(KeyFamily, u32)> {
    let mut rows = BTreeSet::new();
    let mut slots = Vec::new();
    for family in KeyFamily::ALL {
        for (version, key) in doc.versions(family) {
            if key.state != KeyState::Destroyed && rows.insert(secret_name(family, version)) {
                slots.push((family, version));
            }
        }
    }
    slots
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration};

    use super::*;

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap() + Duration::minutes(minutes)
    }

    #[test]
    fn the_shared_v1_row_is_rewrapped_once_and_while_either_family_needs_it() {
        let mut doc = KeysetDocument::fresh(at(0));
        assert_eq!(
            document_key_slots(&doc),
            vec![(KeyFamily::Data, 1)],
            "data v1 and index v1 are one row, re-wrapped once"
        );

        doc.stage(KeyFamily::Data, 2, at(0)).unwrap();
        doc.stage(KeyFamily::Index, 2, at(0)).unwrap();
        doc.promote_due(at(15));
        for family in KeyFamily::ALL {
            doc.record_drain(family, 1, true, at(40));
            doc.record_drain(family, 1, true, at(45));
        }
        let later = at(45 + 60 * 24 * 15);
        doc.destroy(KeyFamily::Data, 1, later).unwrap();
        assert_eq!(
            document_key_slots(&doc),
            vec![
                (KeyFamily::Data, 2),
                (KeyFamily::Index, 1),
                (KeyFamily::Index, 2)
            ],
            "index v1 is disabled, not destroyed, so the shared row is still re-wrapped"
        );

        doc.destroy(KeyFamily::Index, 1, later).unwrap();
        assert_eq!(
            document_key_slots(&doc),
            vec![(KeyFamily::Data, 2), (KeyFamily::Index, 2)],
            "once both families destroyed v1 its row is gone"
        );
    }
}
