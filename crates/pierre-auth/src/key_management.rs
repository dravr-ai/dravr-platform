// ABOUTME: Two-tier key management — a KEK (via KekProvider) wraps the Database Encryption Key (DEK)
// ABOUTME: LocalKekProvider wraps the DEK with the env Master Encryption Key; GcpKmsKekProvider wraps it with Cloud KMS (ADR-017)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::env;

use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as Base64Standard;
use base64::Engine as Base64Engine;
use rand::RngCore;
use tracing::info;

use pierre_core::errors::{AppError, AppResult};
use pierre_database::backends::factory::Database;
use pierre_database::backends::SecurityRepository;
use pierre_database::database::generate_encryption_key;

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

/// Database Encryption Key (DEK) — Tier 2, used for actual data encryption.
///
/// Stored wrapped (KEK-encrypted) in the database; held in memory in plaintext
/// only while the process runs.
pub struct DatabaseEncryptionKey {
    key: [u8; 32],
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

/// Two-tier key management system: a [`KekProvider`] wrapping the [`DatabaseEncryptionKey`].
pub struct KeyManager {
    kek: Box<dyn KekProvider>,
    dek: DatabaseEncryptionKey,
}

impl KeyManager {
    /// Bootstrap initialization: load the KEK and generate a temporary DEK for database initialization
    ///
    /// # Errors
    ///
    /// Returns an error if KEK loading fails
    pub fn bootstrap() -> AppResult<(Self, [u8; 32])> {
        info!("Bootstrapping two-tier key management system");

        // Select the KEK provider (GCP Cloud KMS when configured, else local).
        let kek = Self::configured_kek_provider()?;

        // Generate temporary DEK for database initialization
        let temp_dek = DatabaseEncryptionKey::generate();
        let database_key = *temp_dek.as_bytes();

        let manager = Self { kek, dek: temp_dek };

        info!("Bootstrap key management system initialized");

        Ok((manager, database_key))
    }

    /// Construct a manager from an explicit KEK provider and DEK.
    ///
    /// Used by KEK-migration tooling (to supply the old/source KEK) and tests.
    #[must_use]
    pub fn with_provider(kek: Box<dyn KekProvider>, dek: DatabaseEncryptionKey) -> Self {
        Self { kek, dek }
    }

    /// Select the configured KEK provider.
    ///
    /// With the `gcp-kms` feature enabled and `PIERRE_KMS_KEY_RESOURCE` set, the
    /// DEK is wrapped by GCP Cloud KMS. Otherwise the env-backed local provider is
    /// used (the default for development, CI, and tests). Exposed so migration
    /// tooling (`rewrap_deks`) can obtain the target provider.
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

    /// Decode base64-encoded wrapped DEK
    fn decode_encrypted_dek(encrypted_dek_base64: &str) -> AppResult<Vec<u8>> {
        Base64Standard
            .decode(encrypted_dek_base64)
            .map_err(|e| AppError::internal(format!("Invalid base64 encoding for stored DEK: {e}")))
    }

    /// System-secret holding the active DEK version number.
    const ACTIVE_VERSION_SECRET: &'static str = "database_encryption_key_active_version";
    /// System-secret holding the version-1 (legacy) wrapped DEK.
    const V1_SECRET: &'static str = "database_encryption_key";

    /// Name of the system-secret that stores the wrapped DEK for `version`.
    ///
    /// Version 1 keeps its historical name so existing deployments need no migration.
    fn version_secret_name(version: u32) -> String {
        if version == 1 {
            Self::V1_SECRET.to_owned()
        } else {
            format!("database_encryption_key_v{version}")
        }
    }

    /// Read the active DEK version, defaulting to 1 for pre-versioning deployments.
    async fn read_active_version(security: &dyn SecurityRepository) -> AppResult<u32> {
        security
            .get_system_secret(Self::ACTIVE_VERSION_SECRET)
            .await
            .map_or(Ok(1), |value| {
                value.trim().parse::<u32>().map_err(|e| {
                    AppError::internal(format!("Invalid stored active DEK version '{value}': {e}"))
                })
            })
    }

    /// Load and unwrap every DEK version in `1..=active_version` into a map.
    async fn load_all_versions(
        security: &dyn SecurityRepository,
        kek: &dyn KekProvider,
        active_version: u32,
    ) -> AppResult<HashMap<u32, Vec<u8>>> {
        let mut versions: HashMap<u32, Vec<u8>> = HashMap::new();
        for version in 1..=active_version {
            versions.insert(
                version,
                Self::load_version_key(security, kek, version).await?,
            );
        }
        Ok(versions)
    }

    /// Persist the active DEK version number.
    async fn store_active_version(
        security: &dyn SecurityRepository,
        version: u32,
    ) -> AppResult<()> {
        security
            .update_system_secret(Self::ACTIVE_VERSION_SECRET, &version.to_string())
            .await
    }

    /// Load and unwrap the DEK key bytes for a specific version.
    async fn load_version_key(
        security: &dyn SecurityRepository,
        kek: &dyn KekProvider,
        version: u32,
    ) -> AppResult<Vec<u8>> {
        let wrapped_base64 = security
            .get_system_secret(&Self::version_secret_name(version))
            .await?;
        let wrapped = Self::decode_encrypted_dek(&wrapped_base64)?;
        let unwrapped = kek.unwrap(&wrapped).await.map_err(|e| {
            if e.message.contains("Decryption failed") {
                let database_url =
                    env::var("DATABASE_URL").unwrap_or_else(|_| "unknown".to_owned());
                AppError::encryption_key_mismatch(&database_url)
            } else {
                e
            }
        })?;
        if unwrapped.len() != 32 {
            return Err(AppError::internal(format!(
                "Decrypted DEK v{version} has invalid length: expected 32 bytes, got {}",
                unwrapped.len()
            )));
        }
        Ok(unwrapped)
    }

    /// Persist a wrapped DEK for a specific version.
    async fn store_version_key(
        security: &dyn SecurityRepository,
        version: u32,
        wrapped_dek: &[u8],
    ) -> AppResult<()> {
        security
            .update_system_secret(
                &Self::version_secret_name(version),
                &Base64Standard.encode(wrapped_dek),
            )
            .await
    }

    /// Complete initialization after the database is available.
    ///
    /// For a fresh database, persists the bootstrap DEK as version 1. For an
    /// existing database, loads every DEK version (1..=active) and installs them
    /// so the active version encrypts new data while prior versions stay available
    /// for decrypting older ciphertext.
    ///
    /// # Errors
    ///
    /// Returns an error if database operations or DEK wrap/unwrap fail.
    pub async fn complete_initialization(&mut self, database: &mut Database) -> AppResult<()> {
        info!("Completing two-tier key management initialization");

        let is_fresh = {
            let security = database.as_security_repository();
            security.get_system_secret(Self::V1_SECRET).await.is_err()
        };

        if is_fresh {
            // Fresh database: persist the bootstrap DEK as version 1.
            let security = database.as_security_repository();
            self.store_new_dek(security).await?;
            info!("Two-tier key management initialized with a new DEK (version 1)");
            return Ok(());
        }

        self.load_and_install_existing(database).await
    }

    /// Load every stored DEK version and install them, with the active version
    /// encrypting new data and prior versions retained for decryption.
    async fn load_and_install_existing(&mut self, database: &mut Database) -> AppResult<()> {
        let (active_version, mut versions) = {
            let security = database.as_security_repository();
            let active_version = Self::read_active_version(security).await?;
            let versions =
                Self::load_all_versions(security, self.kek.as_ref(), active_version).await?;
            (active_version, versions)
        };

        let active_key = versions.remove(&active_version).ok_or_else(|| {
            AppError::internal(format!(
                "Active DEK version {active_version} not found among loaded versions"
            ))
        })?;
        self.dek = DatabaseEncryptionKey::from_unwrapped(&active_key)?;
        database.install_dek_versions(active_version, active_key, versions);
        info!("Two-tier key management initialized at active DEK version {active_version}");
        Ok(())
    }

    async fn store_new_dek(&self, security: &dyn SecurityRepository) -> AppResult<()> {
        info!("No existing DEK found, storing current Database Encryption Key");
        let wrapped_dek = self.kek.wrap(self.dek.as_bytes()).await?;
        Self::store_version_key(security, 1, &wrapped_dek).await?;
        info!("Database Encryption Key stored successfully");
        Ok(())
    }

    /// Rotate the Database Encryption Key to a fresh active version.
    ///
    /// Generates a new DEK, wraps it with the KEK, persists it as the new active
    /// version, and retains all prior versions for decrypting existing data. New
    /// writes use the new version immediately; older ciphertext stays readable via
    /// the retained versions (lazy rotation — no bulk re-encryption). The blind
    /// index (refresh-token HMAC) stays pinned to version 1.
    ///
    /// # Errors
    ///
    /// Returns an error if database operations or DEK wrap/unwrap fail.
    pub async fn rotate_dek(&mut self, database: &mut Database) -> AppResult<u32> {
        info!("Rotating Database Encryption Key");

        // Load and retain every existing version (scoped immutable borrow).
        let (new_version, prior) = {
            let security = database.as_security_repository();
            let current_active = Self::read_active_version(security).await?;
            let prior =
                Self::load_all_versions(security, self.kek.as_ref(), current_active).await?;
            (current_active + 1, prior)
        };

        // Generate, wrap, and persist the new active DEK.
        let new_dek = DatabaseEncryptionKey::generate();
        let wrapped = self.kek.wrap(new_dek.as_bytes()).await?;
        {
            let security = database.as_security_repository();
            Self::store_version_key(security, new_version, &wrapped).await?;
            Self::store_active_version(security, new_version).await?;
        }

        let active_key = new_dek.as_bytes().to_vec();
        self.dek = new_dek;
        database.install_dek_versions(new_version, active_key, prior);
        info!("Rotated Database Encryption Key to version {new_version}");
        Ok(new_version)
    }

    /// Re-wrap every stored DEK version under a new KEK provider (KEK rotation /
    /// provider migration, e.g. local MEK → GCP Cloud KMS).
    ///
    /// Each version is unwrapped with the current KEK, wrapped with `new_kek`, and
    /// persisted. The DEK bytes are unchanged, so existing ciphertext stays valid —
    /// only the at-rest wrapping changes. On success the manager adopts `new_kek`.
    ///
    /// # Errors
    ///
    /// Returns an error if database access or wrap/unwrap fails.
    pub async fn rewrap_deks(
        &mut self,
        new_kek: Box<dyn KekProvider>,
        database: &mut Database,
    ) -> AppResult<()> {
        info!("Re-wrapping all DEK versions under a new KEK");

        let rewrapped: Vec<(u32, Vec<u8>)> = {
            let security = database.as_security_repository();
            let active_version = Self::read_active_version(security).await?;
            let versions =
                Self::load_all_versions(security, self.kek.as_ref(), active_version).await?;
            let mut out = Vec::with_capacity(versions.len());
            for (version, key) in versions {
                out.push((version, new_kek.wrap(&key).await?));
            }
            out
        };

        {
            let security = database.as_security_repository();
            for (version, wrapped) in &rewrapped {
                Self::store_version_key(security, *version, wrapped).await?;
            }
        }

        self.kek = new_kek;
        info!(
            "Re-wrapped {} DEK version(s) under the new KEK",
            rewrapped.len()
        );
        Ok(())
    }

    /// Get the DEK for database operations (what we previously called "encryption key")
    #[must_use]
    pub const fn database_key(&self) -> &[u8; 32] {
        self.dek.as_bytes()
    }
}
