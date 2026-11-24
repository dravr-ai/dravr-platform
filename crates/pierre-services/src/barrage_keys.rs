// ABOUTME: The platform's keys for barrage: Keyed over the Index key (identity under its first version), and the checkpoint Signer
// ABOUTME: select_signer picks Cloud KMS, a local key seeded by an identity, or none on a KMS deployment without a signing key
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
//!   tombstones key with, so the domain-separation rule exists once. Its
//!   `identity` (keyed original names, keyed Apple ids) goes through
//!   [`pierre_database::keyed::index_identity`]: the raw MAC under the Index
//!   family's **first** version, never the primary, so an identity does not
//!   move when the primary rotates (version 1, never retired: carnet#1347).
//! - [`checkpoint_signer`]: the Cloud KMS `EC_SIGN_ED25519` key named by
//!   `PIERRE_CHECKPOINT_SIGNING_KEY` ([`KmsSigner`]); on a deployment that
//!   runs no KMS (local, tests), a ring key whose seed is an identity under
//!   the Index key ([`LocalSigner`]), so it survives a rotation of the
//!   primary. A deployment that runs KMS (`PIERRE_KMS_KEY_RESOURCE` set) and
//!   names no signing key gets **none**: it cuts no checkpoint rather than
//!   sign the deployed log with a key that would later change. The choice is
//!   `select_signer`'s, over the two variables' values.

use std::env;
use std::fmt;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use dravr_barrage::keyed::{Keyed, KeyedDigest, KeyedDomain, Signer};
use dravr_barrage::store::memory::MemoryActivityRecordStore;
use dravr_barrage::BarrageError;
use pierre_auth::gcp_kms_signer::{GcpKmsEd25519Signer, CHECKPOINT_SIGNING_KEY_ENV};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::backends::shared::encryption::HasEncryption;
use pierre_database::keyed::{index_identity, index_lookup, index_mac, KeyedDomain as IndexDomain};
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
/// The backend handle shares its process keyring with every clone, so a
/// handle taken at boot keys under whatever Index primary is current.
/// [`Keyed::identity`] stays under the family's first version whatever the
/// primary: keyed names and ids must never move.
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
    /// Keyed digests under `database`'s Index key. The handle shares the
    /// process keyring, so the keys the key manager installs later reach it.
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
            IndexKeys::Ephemeral(key) => Ok(hex::encode(ephemeral_tag(key, domain, message))),
        }
    }

    /// The raw MAC under the Index family's first version. An ephemeral key
    /// is one version for its whole life, so it is its own first one.
    fn identity_of(&self, domain: IndexDomain, message: &[u8]) -> AppResult<[u8; 32]> {
        match &self.keys {
            IndexKeys::Backend(keys) => index_identity(keys.as_ref(), domain, message),
            IndexKeys::Ephemeral(key) => Ok(ephemeral_tag(key, domain, message)),
        }
    }

    fn lookup_digests(&self, domain: IndexDomain, message: &[u8]) -> AppResult<[String; 2]> {
        match &self.keys {
            IndexKeys::Backend(keys) => index_lookup(keys.as_ref(), domain, message),
            IndexKeys::Ephemeral(key) => {
                let mac = hex::encode(ephemeral_tag(key, domain, message));
                Ok([mac.clone(), mac])
            }
        }
    }

    /// The 32-byte seed of the local checkpoint signer.
    ///
    /// Derived from an identity under the Index key, so it is stable across
    /// restarts of one deployment and across a rotation of the Index primary.
    ///
    /// The HMAC is keyed by the identity's base64, which is the digest the
    /// seed was derived from before identities existed (the Index key is the
    /// blind-index key pinned to DEK v1 on main): a local deployment keeps its
    /// checkpoint key through this change.
    fn local_seed(&self) -> AppResult<[u8; 32]> {
        let identity =
            self.identity_of(IndexDomain::CheckpointSeed, CHECKPOINT_KEY_NAME.as_bytes())?;
        Ok(seed_of(&identity))
    }
}

/// The Ed25519 seed an identity derives.
fn seed_of(identity: &[u8; 32]) -> [u8; 32] {
    let key = hmac::Key::new(hmac::HMAC_SHA256, BASE64.encode(identity).as_bytes());
    let mut seed = [0_u8; 32];
    seed.copy_from_slice(hmac::sign(&key, b"ed25519 seed").as_ref());
    seed
}

fn ephemeral_tag(key: &hmac::Key, domain: IndexDomain, message: &[u8]) -> [u8; 32] {
    let mut context = hmac::Context::with_key(key);
    context.update(format!("{domain:?}").as_bytes());
    context.update(&[0]);
    context.update(message);
    let mut tag = [0_u8; 32];
    tag.copy_from_slice(context.sign().as_ref());
    tag
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

    fn identity(&self, domain: KeyedDomain, message: &[u8]) -> Result<[u8; 32], BarrageError> {
        self.identity_of(IndexDomain::Barrage(domain.as_str()), message)
            .map_err(|e| barrage_error(&e))
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

/// Publish `signer`'s key into `slot`, once: the process-wide slot is
/// [`PUBLISHED_KEY`].
///
/// The same key offered again is a no-op. Another key is refused: a process
/// signs with one key, and serving the first while signing with the second
/// would publish a key no new checkpoint verifies against.
///
/// # Errors
///
/// When `slot` already holds a different key.
fn publish_into(slot: &OnceLock<CheckpointKey>, signer: &dyn Signer) -> AppResult<()> {
    let key = CheckpointKey {
        name: signer.key_name().to_owned(),
        public_key: signer.public_key(),
    };
    let published = slot.get_or_init(|| key.clone());
    if *published == key {
        return Ok(());
    }
    Err(AppError::internal(format!(
        "a second checkpoint key `{}` was offered while `{}` is published; a process signs \
         record_log checkpoints with one key",
        key.name, published.name
    )))
}

/// Which checkpoint signer a deployment runs, from its configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SignerChoice {
    /// The Cloud KMS key version named by `PIERRE_CHECKPOINT_SIGNING_KEY`.
    Kms(String),
    /// None: the deployment runs KMS and names no signing key.
    Off,
    /// A local key derived from the Index key: the deployment runs no KMS.
    Local,
}

/// The signer a deployment with these values of `PIERRE_KMS_KEY_RESOURCE`
/// and `PIERRE_CHECKPOINT_SIGNING_KEY` runs.
///
/// A blank signing key names none. The KMS resource counts when it is set at
/// all, as the KEK selection reads it (`configured_kek_provider`): a blank
/// value still marks a KMS deployment, never a local-key one.
#[must_use]
fn select_signer(kms_resource: Option<&str>, signing_key: Option<&str>) -> SignerChoice {
    match signing_key.map(str::trim) {
        Some(version) if !version.is_empty() => SignerChoice::Kms(version.to_owned()),
        _ if kms_resource.is_some() => SignerChoice::Off,
        _ => SignerChoice::Local,
    }
}

/// The checkpoint signer this deployment runs, or `None` when it must not
/// sign: KMS is configured and no signing key is named yet. The signer's key
/// is published ([`published_checkpoint_key`]).
///
/// # Errors
///
/// When the named KMS key cannot be read or is not Ed25519, the local key
/// cannot be derived, or another key is already published in this process.
pub async fn checkpoint_signer(keyed: &PlatformKeyed) -> AppResult<Option<Arc<dyn Signer>>> {
    let kms_resource = env::var_os(KMS_KEY_RESOURCE_ENV).map(|v| v.to_string_lossy().into_owned());
    let signing_key = env::var(CHECKPOINT_SIGNING_KEY_ENV).ok();
    let signer: Arc<dyn Signer> =
        match select_signer(kms_resource.as_deref(), signing_key.as_deref()) {
            SignerChoice::Kms(version) => {
                info!("record_log checkpoints are signed by the Cloud KMS key");
                Arc::new(KmsSigner(GcpKmsEd25519Signer::production(version).await?))
            }
            SignerChoice::Off => {
                warn!(
                    "record_log checkpoints are off: this deployment runs KMS and names no \
                     PIERRE_CHECKPOINT_SIGNING_KEY (apply infra/modules/kms checkpoint_signer)"
                );
                return Ok(None);
            }
            SignerChoice::Local => {
                info!("record_log checkpoints are signed by a local key (no KMS)");
                Arc::new(LocalSigner::derived(keyed)?)
            }
        };
    publish_into(&PUBLISHED_KEY, signer.as_ref())?;
    Ok(Some(signer))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use dravr_tronc::dates::Timestamp;
    use pierre_database::backends::shared::keyring::Keyring;
    use pierre_test_support::db::{create_test_db, create_test_db_with_key};
    use ring::signature::{UnparsedPublicKey, ED25519};

    use super::*;

    fn seed_identity(keyed: &PlatformKeyed) -> [u8; 32] {
        keyed
            .identity_of(IndexDomain::CheckpointSeed, CHECKPOINT_KEY_NAME.as_bytes())
            .unwrap()
    }

    #[tokio::test]
    async fn a_backend_identity_is_32_raw_bytes_stable_per_key() {
        let database = create_test_db().await.unwrap();
        let keyed = PlatformKeyed::of(&database);
        let name = keyed.identity(KeyedDomain::BlobName, b"m").unwrap();
        assert_eq!(
            name,
            keyed.identity(KeyedDomain::BlobName, b"m").unwrap(),
            "bug: one message names differently twice"
        );
        assert_ne!(
            name,
            keyed.identity(KeyedDomain::AppleRecordId, b"m").unwrap(),
            "bug: two domains share an identity"
        );
        assert_eq!(
            BASE64.encode(name),
            keyed.mac(KeyedDomain::BlobName, b"m").unwrap().0,
            "bug: on main the primary is the first version, so an identity is the digest's bytes"
        );
        let same_key = PlatformKeyed::of(&create_test_db().await.unwrap());
        assert_eq!(
            same_key.identity(KeyedDomain::BlobName, b"m").unwrap(),
            name,
            "bug: one Index key names differently in two handles"
        );
        let other_key = PlatformKeyed::of(&create_test_db_with_key(vec![7_u8; 32]).await.unwrap());
        assert_ne!(
            other_key.identity(KeyedDomain::BlobName, b"m").unwrap(),
            name,
            "bug: an identity does not depend on the Index key"
        );
    }

    #[tokio::test]
    async fn a_backend_identity_stays_put_when_the_index_primary_rotates() {
        let v1 = vec![7_u8; 32];
        let database = create_test_db_with_key(v1.clone()).await.unwrap();
        let keyed = PlatformKeyed::of(&database);
        let name = keyed.identity(KeyedDomain::BlobName, b"m").unwrap();
        let digest = keyed.mac(KeyedDomain::BlobName, b"m").unwrap().0;

        let rotated = Keyring::new(
            BTreeMap::from([(1, v1.clone())]),
            1,
            BTreeMap::from([(1, v1), (2, vec![8_u8; 32])]),
            2,
            1,
            true,
        )
        .unwrap();
        database.keyring().install(rotated, Timestamp::now());

        assert_ne!(
            keyed.mac(KeyedDomain::BlobName, b"m").unwrap().0,
            digest,
            "bug: a new digest is not written under the rotated Index primary"
        );
        assert_eq!(
            keyed.identity(KeyedDomain::BlobName, b"m").unwrap(),
            name,
            "bug: an identity moved with the Index primary"
        );
    }

    #[test]
    fn an_ephemeral_identity_is_the_raw_tag_its_digest_encodes() {
        let keyed = PlatformKeyed::ephemeral();
        let name = keyed.identity(KeyedDomain::BlobName, b"m").unwrap();
        assert_eq!(
            name,
            keyed.identity(KeyedDomain::BlobName, b"m").unwrap(),
            "bug: one message names differently twice"
        );
        assert_eq!(
            hex::encode(name),
            keyed.mac(KeyedDomain::BlobName, b"m").unwrap().0,
            "bug: an ephemeral identity is not the tag its digest encodes"
        );
        assert_ne!(
            PlatformKeyed::ephemeral()
                .identity(KeyedDomain::BlobName, b"m")
                .unwrap(),
            name,
            "bug: two processes share an ephemeral key"
        );
    }

    #[tokio::test]
    async fn the_local_seed_is_derived_from_the_identity_not_the_primary() {
        let database = create_test_db().await.unwrap();
        let keyed = PlatformKeyed::of(&database);
        let seed = keyed.local_seed().unwrap();
        assert_eq!(
            seed,
            seed_of(&seed_identity(&keyed)),
            "bug: the local seed is not derived from the checkpoint identity"
        );
        assert_eq!(
            seed,
            PlatformKeyed::of(&database).local_seed().unwrap(),
            "bug: one Index key derives two local seeds"
        );
        assert_eq!(
            LocalSigner::derived(&keyed).unwrap().public_key(),
            LocalSigner::derived(&PlatformKeyed::of(&database))
                .unwrap()
                .public_key(),
            "bug: one Index key derives two checkpoint keys"
        );
        let ephemeral = PlatformKeyed::ephemeral();
        assert_eq!(
            ephemeral.local_seed().unwrap(),
            seed_of(&seed_identity(&ephemeral)),
            "bug: an ephemeral local seed is not derived from its identity"
        );
    }

    #[test]
    fn a_kms_deployment_without_a_signing_key_selects_no_signer() {
        assert_eq!(
            select_signer(Some("projects/p/keyRings/r/cryptoKeys/k"), None),
            SignerChoice::Off,
            "bug: a KMS deployment signs with a local key"
        );
        assert_eq!(
            select_signer(Some(""), Some("  ")),
            SignerChoice::Off,
            "bug: a blank KMS resource or signing key reads as a local deployment"
        );
        assert_eq!(
            select_signer(Some("projects/p"), Some(" v1 ")),
            SignerChoice::Kms("v1".to_owned()),
            "bug: a named signing key is not the one KMS signs with"
        );
    }

    #[test]
    fn no_kms_selects_the_local_signer() {
        assert_eq!(
            select_signer(None, None),
            SignerChoice::Local,
            "bug: a deployment without KMS signs nothing"
        );
        assert_eq!(
            select_signer(None, Some("")),
            SignerChoice::Local,
            "bug: a blank signing key without KMS signs nothing"
        );
        assert_eq!(
            select_signer(None, Some("v1")),
            SignerChoice::Kms("v1".to_owned()),
            "bug: a named signing key is ignored without a KMS resource"
        );
    }

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

    /// The key a process signs with, offered again (a second boot path, a
    /// retry), stays published and is no error.
    #[test]
    fn the_same_checkpoint_key_offered_again_is_a_no_op() {
        let slot = OnceLock::new();
        let signer = LocalSigner::random().unwrap();
        publish_into(&slot, &signer).unwrap();
        assert!(
            publish_into(&slot, &signer).is_ok(),
            "bug: the published key offered again is refused"
        );
        assert_eq!(
            slot.get().map(|key| key.public_key),
            Some(signer.public_key()),
            "bug: the published key changed"
        );
    }

    /// A second, different key is refused loudly and the first stays
    /// published: did.json never names a key the process does not sign with
    /// unnoticed.
    #[test]
    fn a_second_different_checkpoint_key_is_refused() {
        let slot = OnceLock::new();
        let first = LocalSigner::random().unwrap();
        let second = LocalSigner::random().unwrap();
        publish_into(&slot, &first).unwrap();
        let refused = publish_into(&slot, &second);
        assert!(
            refused
                .as_ref()
                .is_err_and(|e| e.to_string().contains("a second checkpoint key")),
            "bug: a second checkpoint key is published silently: {refused:?}"
        );
        assert_eq!(
            slot.get().map(|key| key.public_key),
            Some(first.public_key()),
            "bug: the second key replaced the first"
        );
    }
}
