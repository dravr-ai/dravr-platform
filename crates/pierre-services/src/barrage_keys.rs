// ABOUTME: The platform's keys for barrage: Keyed over the Index key, and the checkpoint Signer (Cloud KMS, or a local key)
// ABOUTME: A KMS deployment without PIERRE_CHECKPOINT_SIGNING_KEY gets no signer, so it never signs with a local key
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Barrage's keys, held by the platform
//!
//! Barrage links no cryptography (carnet#1542, design Q3/Q11): it asks a
//! [`Keyed`] for its keyed digests and a [`Signer`] for checkpoint
//! signatures. The platform implements both over its one key system:
//!
//! - [`PlatformKeyed`]: an HMAC under the Index key, through
//!   [`pierre_database::keyed::index_mac`], the same helper the global import
//!   tombstones key with, so the domain-separation rule exists once.
//! - [`checkpoint_signer`]: the Cloud KMS `EC_SIGN_ED25519` key named by
//!   `PIERRE_CHECKPOINT_SIGNING_KEY` ([`KmsSigner`]); on a deployment that
//!   runs no KMS (local, tests), a ring key derived from the Index key
//!   ([`LocalSigner`]). A deployment that runs KMS (`PIERRE_KMS_KEY_RESOURCE`
//!   set) and names no signing key gets **none**: it cuts no checkpoint
//!   rather than sign the deployed log with a key that would later change.

use std::env;
use std::fmt;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use dravr_barrage::keyed::{Keyed, KeyedDigest, KeyedDomain, Signer};
use dravr_barrage::store::memory::MemoryActivityRecordStore;
use dravr_barrage::BarrageError;
use pierre_auth::gcp_kms_signer::GcpKmsEd25519Signer;
use pierre_core::errors::{AppError, AppResult};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::backends::shared::encryption::HasEncryption;
use pierre_database::keyed::{index_lookup, index_mac, KeyedDomain as IndexDomain};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{Ed25519KeyPair, KeyPair};
use tracing::{info, warn};
use uuid::Uuid;

/// The key name a checkpoint signed by Dravr's KMS key carries (C2SP note).
pub const CHECKPOINT_KEY_NAME: &str = "dravr.ai/barrage-log";

/// The origin line of every checkpoint note.
pub const CHECKPOINT_ORIGIN: &str = "dravr.ai/barrage-log";

/// The key name of a checkpoint signed by a local key: never mistaken for
/// the KMS key's.
pub const LOCAL_KEY_NAME: &str = "dravr.ai/barrage-log-local";

/// The environment variable a deployment that runs KMS sets.
const KMS_KEY_RESOURCE_ENV: &str = "PIERRE_KMS_KEY_RESOURCE";

/// What barrage's [`Keyed`] calls go through: one backend's encryption.
#[derive(Clone)]
enum IndexKeys {
    /// The platform database's backend, holding the Index key.
    Backend(Arc<dyn HasEncryption + Send + Sync>),
    /// A random per-process HMAC key, for a context with no database.
    Ephemeral(Arc<hmac::Key>),
}

/// Barrage's [`Keyed`] over the platform's Index key.
///
/// The Index key is pinned to DEK v1 on main, so a handle taken at boot keys
/// the same as one taken later; carnet#1347 replaces it with a keyring every
/// clone shares.
#[derive(Clone)]
pub struct PlatformKeyed {
    keys: IndexKeys,
}

impl fmt::Debug for PlatformKeyed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlatformKeyed").finish_non_exhaustive()
    }
}

impl PlatformKeyed {
    /// Keyed digests under `database`'s Index key. Build it once the real
    /// DEK is installed (after the key manager's initialization).
    #[must_use]
    pub fn of(database: &Database) -> Self {
        let keys: Arc<dyn HasEncryption + Send + Sync> = match database.backend() {
            DatabaseBackend::SQLite(db) => db.clone(),
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => db.clone(),
        };
        Self {
            keys: IndexKeys::Backend(keys),
        }
    }

    /// Keyed digests under a random key that lives as long as the process:
    /// for stores in memory, which outlive nothing either. The key is two
    /// version-4 UUIDs (244 random bits from the OS source).
    #[must_use]
    pub fn ephemeral() -> Self {
        let mut key = Uuid::new_v4().as_bytes().to_vec();
        key.extend_from_slice(Uuid::new_v4().as_bytes());
        Self {
            keys: IndexKeys::Ephemeral(Arc::new(hmac::Key::new(hmac::HMAC_SHA256, &key))),
        }
    }

    fn digest(&self, domain: IndexDomain, message: &[u8]) -> AppResult<String> {
        match &self.keys {
            IndexKeys::Backend(keys) => index_mac(keys.as_ref(), domain, message),
            IndexKeys::Ephemeral(key) => Ok(ephemeral_mac(key, domain, message)),
        }
    }

    fn lookup_digests(&self, domain: IndexDomain, message: &[u8]) -> AppResult<[String; 2]> {
        match &self.keys {
            IndexKeys::Backend(keys) => index_lookup(keys.as_ref(), domain, message),
            IndexKeys::Ephemeral(key) => {
                let mac = ephemeral_mac(key, domain, message);
                Ok([mac.clone(), mac])
            }
        }
    }

    /// The 32-byte seed of the local checkpoint signer: keyed under the
    /// Index key, so it is stable across restarts of one deployment.
    fn local_seed(&self) -> AppResult<[u8; 32]> {
        let digest = self.digest(IndexDomain::CheckpointSeed, CHECKPOINT_KEY_NAME.as_bytes())?;
        let tag = hmac::sign(
            &hmac::Key::new(hmac::HMAC_SHA256, digest.as_bytes()),
            b"ed25519 seed",
        );
        tag.as_ref()
            .try_into()
            .map_err(|_| AppError::internal("An HMAC-SHA256 tag is not 32 bytes"))
    }
}

fn ephemeral_mac(key: &hmac::Key, domain: IndexDomain, message: &[u8]) -> String {
    let mut context = hmac::Context::with_key(key);
    context.update(format!("{domain:?}").as_bytes());
    context.update(&[0]);
    context.update(message);
    hex::encode(context.sign().as_ref())
}

fn barrage_error(error: &AppError) -> BarrageError {
    BarrageError::invalid("index key", error.to_string())
}

impl Keyed for PlatformKeyed {
    fn mac(&self, domain: KeyedDomain, message: &[u8]) -> Result<KeyedDigest, BarrageError> {
        self.digest(IndexDomain::Barrage(domain.as_str()), message)
            .map(KeyedDigest)
            .map_err(|e| barrage_error(&e))
    }

    fn lookup(
        &self,
        domain: KeyedDomain,
        message: &[u8],
    ) -> Result<[KeyedDigest; 2], BarrageError> {
        let [a, b] = self
            .lookup_digests(IndexDomain::Barrage(domain.as_str()), message)
            .map_err(|e| barrage_error(&e))?;
        Ok([KeyedDigest(a), KeyedDigest(b)])
    }
}

/// A record store in memory, keyed under a process-lifetime key: what a
/// context with no database keeps its activity records in.
#[must_use]
pub fn memory_records() -> MemoryActivityRecordStore {
    MemoryActivityRecordStore::new(Arc::new(PlatformKeyed::ephemeral()))
}

/// Barrage's [`Signer`] over Dravr's Cloud KMS checkpoint key.
#[derive(Debug)]
pub struct KmsSigner(GcpKmsEd25519Signer);

#[async_trait]
impl Signer for KmsSigner {
    fn key_name(&self) -> &str {
        CHECKPOINT_KEY_NAME
    }

    fn public_key(&self) -> [u8; 32] {
        self.0.public_key()
    }

    async fn sign(&self, message: &[u8]) -> Result<[u8; 64], BarrageError> {
        self.0
            .sign(message)
            .await
            .map_err(|e| BarrageError::invalid("checkpoint signature", e.to_string()))
    }
}

/// Barrage's [`Signer`] over a ring Ed25519 key, for a deployment with no KMS.
pub struct LocalSigner {
    pair: Ed25519KeyPair,
}

impl fmt::Debug for LocalSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalSigner").finish_non_exhaustive()
    }
}

impl LocalSigner {
    /// The local signer whose seed `keyed` derives from the Index key.
    ///
    /// # Errors
    ///
    /// When the seed cannot be derived or is refused as a key.
    pub fn derived(keyed: &PlatformKeyed) -> AppResult<Self> {
        Self::from_seed(&keyed.local_seed()?)
    }

    /// The local signer of a fresh random seed.
    ///
    /// # Errors
    ///
    /// When the system's random source fails.
    pub fn random() -> AppResult<Self> {
        let mut seed = [0_u8; 32];
        SystemRandom::new()
            .fill(&mut seed)
            .map_err(|_| AppError::internal("Failed to generate a signing seed"))?;
        Self::from_seed(&seed)
    }

    fn from_seed(seed: &[u8; 32]) -> AppResult<Self> {
        let pair = Ed25519KeyPair::from_seed_unchecked(seed)
            .map_err(|e| AppError::internal(format!("Refused checkpoint signing seed: {e}")))?;
        Ok(Self { pair })
    }
}

#[async_trait]
impl Signer for LocalSigner {
    fn key_name(&self) -> &str {
        LOCAL_KEY_NAME
    }

    fn public_key(&self) -> [u8; 32] {
        let mut key = [0_u8; 32];
        key.copy_from_slice(self.pair.public_key().as_ref());
        key
    }

    async fn sign(&self, message: &[u8]) -> Result<[u8; 64], BarrageError> {
        let mut signature = [0_u8; 64];
        signature.copy_from_slice(self.pair.sign(message).as_ref());
        Ok(signature)
    }
}

/// The key this process signs checkpoints with, as a verifier needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointKey {
    /// The C2SP key name its notes carry.
    pub name: String,
    /// The Ed25519 public key.
    pub public_key: [u8; 32],
}

/// The checkpoint key, published once at boot by [`checkpoint_signer`].
static PUBLISHED_KEY: OnceLock<CheckpointKey> = OnceLock::new();

/// The key this process signs checkpoints with: what an export's proof and
/// `/.well-known/did.json` name. `None` while it signs none.
#[must_use]
pub fn published_checkpoint_key() -> Option<&'static CheckpointKey> {
    PUBLISHED_KEY.get()
}

fn publish(signer: &dyn Signer) {
    let key = CheckpointKey {
        name: signer.key_name().to_owned(),
        public_key: signer.public_key(),
    };
    if PUBLISHED_KEY.set(key.clone()).is_err() && PUBLISHED_KEY.get() != Some(&key) {
        warn!("a second checkpoint key was offered; the first one stays published");
    }
}

/// The checkpoint signer this deployment runs, or `None` when it must not
/// sign: KMS is configured and no signing key is named yet. The signer's key
/// is published ([`published_checkpoint_key`]).
///
/// # Errors
///
/// When the named KMS key cannot be read or is not Ed25519, or the local
/// key cannot be derived.
pub async fn checkpoint_signer(keyed: &PlatformKeyed) -> AppResult<Option<Arc<dyn Signer>>> {
    if let Some(kms) = GcpKmsEd25519Signer::from_env().await? {
        info!("record_log checkpoints are signed by the Cloud KMS key");
        let signer = KmsSigner(kms);
        publish(&signer);
        return Ok(Some(Arc::new(signer)));
    }
    // Set at all, as the KEK selection reads it (`configured_kek_provider`):
    // a blank value still marks a KMS deployment, never a local-key one.
    if env::var_os(KMS_KEY_RESOURCE_ENV).is_some() {
        warn!(
            "record_log checkpoints are off: this deployment runs KMS and names no \
             PIERRE_CHECKPOINT_SIGNING_KEY (apply infra/modules/kms checkpoint_signer)"
        );
        return Ok(None);
    }
    info!("record_log checkpoints are signed by a local key (no KMS)");
    let signer = LocalSigner::derived(keyed)?;
    publish(&signer);
    Ok(Some(Arc::new(signer)))
}

#[cfg(test)]
mod tests {
    use ring::signature::{UnparsedPublicKey, ED25519};

    use super::*;

    #[tokio::test]
    async fn a_local_signature_verifies_under_its_public_key() {
        let signer = LocalSigner::random().unwrap();
        let signature = signer.sign(b"note").await.unwrap();
        assert!(
            UnparsedPublicKey::new(&ED25519, signer.public_key())
                .verify(b"note", &signature)
                .is_ok(),
            "bug: a local signature does not verify under its own key"
        );
        assert_eq!(
            signer.key_name(),
            LOCAL_KEY_NAME,
            "bug: a local key is named like the KMS key"
        );
    }

    #[test]
    fn ephemeral_digests_are_stable_per_domain_and_message() {
        let keyed = PlatformKeyed::ephemeral();
        let a = keyed.mac(KeyedDomain::LogRecord, b"m").unwrap();
        assert_eq!(
            a,
            keyed.mac(KeyedDomain::LogRecord, b"m").unwrap(),
            "bug: one message keys differently twice"
        );
        assert_ne!(
            a,
            keyed.mac(KeyedDomain::ErasedActivity, b"m").unwrap(),
            "bug: two domains share a digest"
        );
        let [x, y] = keyed.lookup(KeyedDomain::LogRecord, b"m").unwrap();
        assert_eq!(
            [x, y],
            [a.clone(), a],
            "bug: the write digest is not looked up"
        );
        let other = PlatformKeyed::ephemeral();
        assert_ne!(
            other.mac(KeyedDomain::LogRecord, b"m").unwrap(),
            keyed.mac(KeyedDomain::LogRecord, b"m").unwrap(),
            "bug: two processes share an ephemeral key"
        );
    }
}
