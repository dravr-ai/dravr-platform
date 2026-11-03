// ABOUTME: The loaded keyset: data keys that seal stored secrets and index keys that hash refresh tokens
// ABOUTME: One cell per process, shared by every database handle, swapped whole when the keyset document changes

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The keyring
//!
//! Two key families, each a set of versions with one primary (ADR-017,
//! carnet#1347):
//!
//! - **data** keys seal stored secrets with AES-256-GCM. Ciphertext reads
//!   `v{N}:{base64}`; an untagged value is version 1.
//! - **index** keys compute the HMAC-SHA256 blind index of a refresh token.
//!   A hash reads `k{N}:{base64}`; an untagged hash is version 1, which is
//!   the version-1 data key's bytes, as every hash written before the keyset
//!   existed was.
//!
//! New ciphertext and new hashes use the family's primary. Opening a value
//! uses whatever version its tag names, and a lookup hashes under every
//! loaded index key, at most two ([`Keyring::lookup_hashes`]).
//!
//! A process holds one [`KeyringCell`]; every clone of a database handle
//! shares it, so one [`KeyringCell::install`] reaches every repository. Once
//! the keyset is adopted, a cell whose keyring was last confirmed more than
//! [`FLEET_WINDOW`] ago refuses to seal or write a hash: an instance that
//! cannot reload the keyset cannot write under a primary that was demoted
//! while it was blind. Opening and lookups never refuse.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use tracing::error;

use super::encryption::{split_dek_version, tag_dek_version};
use super::key_material::{StoredSecretKind, KEYRING_STALE};

/// How stale a process's keyring may be while it seals.
///
/// Promotion waits this long after staging, so every instance that can seal
/// already holds the staged key, and disabling waits this long after
/// demotion, so no instance still seals under the demoted key.
pub const FLEET_WINDOW: Duration = Duration::from_mins(15);

/// The length of every key in either family.
pub const KEY_LEN: usize = 32;

/// The two key families.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyFamily {
    /// AES-256-GCM keys that seal stored secrets.
    Data,
    /// HMAC-SHA256 keys that compute the refresh-token blind index.
    Index,
}

impl KeyFamily {
    /// Both families.
    pub const ALL: [Self; 2] = [Self::Data, Self::Index];

    /// The name the keyset document and the CLI use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Index => "index",
        }
    }

    /// The family `text` names.
    ///
    /// # Errors
    ///
    /// An invalid-input error for any other text.
    pub fn parse(text: &str) -> AppResult<Self> {
        Self::ALL
            .into_iter()
            .find(|family| family.as_str() == text)
            .ok_or_else(|| {
                AppError::invalid_input(format!("unknown key family `{text}` (data or index)"))
            })
    }
}

impl fmt::Display for KeyFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a seal protects. Every seal and open names one, so the closed list
/// of stored columns the rotation sweep re-seals ([`StoredSecretKind`]) is
/// also the list of every stored seal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SealDomain {
    /// A platform column the sweep re-seals and the rotation counts.
    Stored(StoredSecretKind),
    /// The Google sign-in transaction cookie: client-held, bounded by its
    /// TTL, never stored by the platform.
    GoogleSignInTransaction,
    /// An archive import's export link: stored by barrage, counted through
    /// its external holder.
    ArchiveImportLink,
}

impl SealDomain {
    /// The name error messages carry.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stored(kind) => kind.as_str(),
            Self::GoogleSignInTransaction => "google_sign_in_transaction",
            Self::ArchiveImportLink => "archive_import_link",
        }
    }
}

/// Tag a base64 blind-index hash with the index version that produced it.
/// Version 1 stays untagged, as every hash written before the keyset was.
#[must_use]
pub fn tag_index_version(version: u32, mac: &str) -> String {
    if version == 1 {
        mac.to_owned()
    } else {
        format!("k{version}:{mac}")
    }
}

/// The index version a stored hash was computed under. Standard base64
/// never holds `:`, so an untagged hash is never misread.
#[must_use]
pub fn index_version_of(stored: &str) -> u32 {
    stored
        .strip_prefix('k')
        .and_then(|rest| rest.split_once(':'))
        .and_then(|(digits, _)| digits.parse::<u32>().ok())
        .unwrap_or(1)
}

/// A key's bytes, never printed.
#[derive(Clone)]
struct KeyBytes(Vec<u8>);

impl fmt::Debug for KeyBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// The keys one process has loaded: both families' usable versions and
/// their primaries.
#[derive(Clone, Debug)]
pub struct Keyring {
    data: BTreeMap<u32, KeyBytes>,
    data_primary: u32,
    index: BTreeMap<u32, KeyBytes>,
    index_primary: u32,
    revision: u64,
    adopted: bool,
}

impl Keyring {
    /// The one-key ring: `key` as version 1 of both families, not adopted.
    /// What a process holds before the keyset is loaded, and what a legacy
    /// database at version 1 loads.
    #[must_use]
    pub fn single(key: Vec<u8>) -> Self {
        Self {
            data: BTreeMap::from([(1, KeyBytes(key.clone()))]),
            data_primary: 1,
            index: BTreeMap::from([(1, KeyBytes(key))]),
            index_primary: 1,
            revision: 0,
            adopted: false,
        }
    }

    /// A ring of loaded keys.
    ///
    /// `adopted` is whether a keyset document exists; until then the stale
    /// refusal is off and the ring is the legacy layout's.
    ///
    /// # Errors
    ///
    /// When a primary is not among its family's keys, a family holds more
    /// than two index keys, or a key is not [`KEY_LEN`] bytes.
    pub fn new(
        data: BTreeMap<u32, Vec<u8>>,
        data_primary: u32,
        index: BTreeMap<u32, Vec<u8>>,
        index_primary: u32,
        revision: u64,
        adopted: bool,
    ) -> AppResult<Self> {
        if !data.contains_key(&data_primary) {
            return Err(AppError::internal(format!(
                "the data primary v{data_primary} is not among the loaded data keys"
            )));
        }
        if !index.contains_key(&index_primary) {
            return Err(AppError::internal(format!(
                "the index primary k{index_primary} is not among the loaded index keys"
            )));
        }
        if index.len() > 2 {
            return Err(AppError::internal(format!(
                "{} index keys are loaded; a lookup binds at most two",
                index.len()
            )));
        }
        if let Some((family, version)) = data
            .iter()
            .map(|(v, k)| (KeyFamily::Data, *v, k.len()))
            .chain(index.iter().map(|(v, k)| (KeyFamily::Index, *v, k.len())))
            .find(|(_, _, len)| *len != KEY_LEN)
            .map(|(family, version, _)| (family, version))
        {
            return Err(AppError::internal(format!(
                "{family} key v{version} is not {KEY_LEN} bytes"
            )));
        }
        let wrap = |keys: BTreeMap<u32, Vec<u8>>| {
            keys.into_iter()
                .map(|(version, key)| (version, KeyBytes(key)))
                .collect()
        };
        Ok(Self {
            data: wrap(data),
            data_primary,
            index: wrap(index),
            index_primary,
            revision,
            adopted,
        })
    }

    /// The keyset document revision this ring was built from; 0 before one
    /// exists.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether a keyset document exists.
    #[must_use]
    pub const fn adopted(&self) -> bool {
        self.adopted
    }

    /// The version new ciphertext is sealed under.
    #[must_use]
    pub const fn data_primary(&self) -> u32 {
        self.data_primary
    }

    /// The version new hashes are computed under.
    #[must_use]
    pub const fn index_primary(&self) -> u32 {
        self.index_primary
    }

    /// The loaded versions of `family`, ascending.
    #[must_use]
    pub fn versions(&self, family: KeyFamily) -> Vec<u32> {
        match family {
            KeyFamily::Data => self.data.keys().copied().collect(),
            KeyFamily::Index => self.index.keys().copied().collect(),
        }
    }

    /// Seal `plaintext` under the data primary, bound to `aad`.
    ///
    /// # Errors
    ///
    /// When the nonce cannot be drawn or the key is unusable.
    pub fn seal(&self, domain: SealDomain, plaintext: &str, aad: &str) -> AppResult<String> {
        let key = self.data_key(domain, self.data_primary)?;
        let mut nonce_bytes = [0u8; 12];
        SystemRandom::new()
            .fill(&mut nonce_bytes)
            .map_err(|e| AppError::internal(format!("Failed to generate nonce: {e}")))?;
        let mut sealed = plaintext.as_bytes().to_vec();
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce_bytes),
            Aad::from(aad.as_bytes()),
            &mut sealed,
        )
        .map_err(|e| AppError::internal(format!("Failed to encrypt {}: {e}", domain.as_str())))?;
        let mut combined = nonce_bytes.to_vec();
        combined.extend(sealed);
        Ok(tag_dek_version(
            self.data_primary,
            &STANDARD.encode(combined),
        ))
    }

    /// Open a value sealed under any loaded data version, bound to `aad`.
    ///
    /// # Errors
    ///
    /// When the version its tag names is not loaded, the value is malformed,
    /// or it fails to authenticate (a wrong key or a wrong `aad`).
    pub fn open(&self, domain: SealDomain, stored: &str, aad: &str) -> AppResult<String> {
        let (version, payload) = split_dek_version(stored);
        let key = self.data_key(domain, version)?;
        let combined = STANDARD
            .decode(payload)
            .map_err(|e| AppError::internal(format!("Failed to decode base64: {e}")))?;
        if combined.len() < 12 {
            return Err(AppError::internal("Invalid encrypted data: too short"));
        }
        let (nonce_bytes, sealed) = combined.split_at(12);
        let nonce = Nonce::assume_unique_for_key(
            nonce_bytes
                .try_into()
                .map_err(|e| AppError::internal(format!("Invalid nonce size: {e}")))?,
        );
        let mut buffer = sealed.to_vec();
        let opened = key
            .open_in_place(nonce, Aad::from(aad.as_bytes()), &mut buffer)
            .map_err(|e| {
                AppError::internal(format!(
                    "Decryption failed (possible AAD mismatch or tampered data): {e:?}"
                ))
            })?;
        String::from_utf8(opened.to_vec()).map_err(|e| {
            AppError::internal(format!("Failed to convert decrypted data to string: {e}"))
        })
    }

    /// The blind index of `token` under the index primary, tagged.
    #[must_use]
    pub fn hash_for_storage(&self, token: &str) -> String {
        tag_index_version(
            self.index_primary,
            &self.mac(self.index_primary, token).unwrap_or_default(),
        )
    }

    /// The hashes a lookup of `token` binds: under the primary, then under
    /// the other loaded index key (a staged or a draining one), or the
    /// primary's again when it is the only one.
    #[must_use]
    pub fn lookup_hashes(&self, token: &str) -> [String; 2] {
        let primary = self.hash_for_storage(token);
        let other = self
            .index
            .keys()
            .copied()
            .find(|version| *version != self.index_primary)
            .and_then(|version| {
                self.mac(version, token)
                    .map(|mac| tag_index_version(version, &mac))
            })
            .unwrap_or_else(|| primary.clone());
        [primary, other]
    }

    fn mac(&self, version: u32, token: &str) -> Option<String> {
        let key = self.index.get(&version)?;
        let tag = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &key.0), token.as_bytes());
        Some(STANDARD.encode(tag.as_ref()))
    }

    fn data_key(&self, domain: SealDomain, version: u32) -> AppResult<LessSafeKey> {
        let bytes = self.data.get(&version).ok_or_else(|| {
            AppError::internal(format!(
                "No DEK available for ciphertext version {version} of {} (primary is v{})",
                domain.as_str(),
                self.data_primary
            ))
        })?;
        let unbound = UnboundKey::new(&AES_256_GCM, &bytes.0)
            .map_err(|e| AppError::internal(format!("Failed to create encryption key: {e}")))?;
        Ok(LessSafeKey::new(unbound))
    }
}

/// The keyring one process uses, shared by every database handle it opens.
pub struct KeyringCell {
    ring: ArcSwap<Keyring>,
    /// When the keyring was last confirmed current, in Unix milliseconds:
    /// the moment the keyset read that confirmed it was issued.
    confirmed_at_ms: AtomicI64,
}

impl fmt::Debug for KeyringCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyringCell")
            .field("ring", &self.ring.load())
            .field("confirmed_at_ms", &self.confirmed_at_ms)
            .finish()
    }
}

impl KeyringCell {
    /// A cell holding `ring`, confirmed now.
    #[must_use]
    pub fn new(ring: Keyring) -> Arc<Self> {
        Arc::new(Self {
            ring: ArcSwap::from_pointee(ring),
            confirmed_at_ms: AtomicI64::new(Utc::now().timestamp_millis()),
        })
    }

    /// The keyring in use.
    #[must_use]
    pub fn current(&self) -> Arc<Keyring> {
        self.ring.load_full()
    }

    /// Replace the keyring, confirmed as of `read_issued_at`.
    pub fn install(&self, ring: Keyring, read_issued_at: DateTime<Utc>) {
        self.ring.store(Arc::new(ring));
        self.confirm(read_issued_at);
    }

    /// Record that a keyset read issued at `read_issued_at` found the
    /// keyring current. A confirmation never moves the time back.
    pub fn confirm(&self, read_issued_at: DateTime<Utc>) {
        self.confirmed_at_ms
            .fetch_max(read_issued_at.timestamp_millis(), Ordering::AcqRel);
    }

    /// When the keyring was last confirmed current.
    #[must_use]
    pub fn confirmed_at(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(self.confirmed_at_ms.load(Ordering::Acquire))
            .unwrap_or_default()
    }

    /// Seal under the data primary.
    ///
    /// # Errors
    ///
    /// When the keyset is adopted and the keyring is stale, or sealing fails.
    pub fn seal(&self, domain: SealDomain, plaintext: &str, aad: &str) -> AppResult<String> {
        let ring = self.current();
        self.refuse_if_stale(&ring, "seal")?;
        ring.seal(domain, plaintext, aad)
    }

    /// Open a value sealed under any loaded data version.
    ///
    /// # Errors
    ///
    /// See [`Keyring::open`].
    pub fn open(&self, domain: SealDomain, stored: &str, aad: &str) -> AppResult<String> {
        self.current().open(domain, stored, aad)
    }

    /// The blind index a new row stores.
    ///
    /// # Errors
    ///
    /// When the keyset is adopted and the keyring is stale.
    pub fn hash_for_storage(&self, token: &str) -> AppResult<String> {
        let ring = self.current();
        self.refuse_if_stale(&ring, "hash")?;
        Ok(ring.hash_for_storage(token))
    }

    /// The hashes a lookup binds; see [`Keyring::lookup_hashes`].
    #[must_use]
    pub fn lookup_hashes(&self, token: &str) -> [String; 2] {
        self.current().lookup_hashes(token)
    }

    fn refuse_if_stale(&self, ring: &Keyring, what: &str) -> AppResult<()> {
        if !ring.adopted() {
            return Ok(());
        }
        let age = Utc::now() - self.confirmed_at();
        let window = chrono::Duration::from_std(FLEET_WINDOW).unwrap_or(chrono::Duration::MAX);
        if age <= window {
            return Ok(());
        }
        error!(
            event = KEYRING_STALE,
            revision = ring.revision(),
            stale_secs = age.num_seconds(),
            "the keyring has not been confirmed within the fleet window; refusing to {what}"
        );
        Err(AppError::internal(format!(
            "the keyring was last confirmed {}s ago, past the {}s fleet window; refusing to {what} until the keyset reloads",
            age.num_seconds(),
            FLEET_WINDOW.as_secs()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AAD: &str = "tenant|user|strava|user_oauth_tokens";
    const DOMAIN: SealDomain = SealDomain::Stored(StoredSecretKind::OAuthAccessToken);

    fn key(byte: u8) -> Vec<u8> {
        vec![byte; KEY_LEN]
    }

    fn ring(data: &[u32], data_primary: u32, index: &[u32], index_primary: u32) -> Keyring {
        Keyring::new(
            data.iter()
                .map(|v| (*v, key(u8::try_from(*v).unwrap())))
                .collect(),
            data_primary,
            index
                .iter()
                .map(|v| (*v, key(u8::try_from(*v).unwrap())))
                .collect(),
            index_primary,
            1,
            true,
        )
        .unwrap()
    }

    #[test]
    fn a_value_sealed_under_one_primary_opens_after_the_next() {
        let before = ring(&[1], 1, &[1], 1);
        let sealed = before.seal(DOMAIN, "secret", AAD).unwrap();
        assert!(sealed.starts_with("v1:"));
        let after = ring(&[1, 2], 2, &[1], 1);
        assert_eq!(after.open(DOMAIN, &sealed, AAD).unwrap(), "secret");
        let resealed = after.seal(DOMAIN, "secret", AAD).unwrap();
        assert!(resealed.starts_with("v2:"));
        assert!(after.open(DOMAIN, &resealed, "another|context").is_err());
        let retired = ring(&[2], 2, &[1], 1);
        assert!(retired.open(DOMAIN, &sealed, AAD).is_err());
    }

    #[test]
    fn an_untagged_value_reads_as_version_one() {
        let sealed = ring(&[1], 1, &[1], 1).seal(DOMAIN, "secret", AAD).unwrap();
        let untagged = sealed.strip_prefix("v1:").unwrap();
        assert_eq!(
            ring(&[1, 2], 2, &[1], 1)
                .open(DOMAIN, untagged, AAD)
                .unwrap(),
            "secret"
        );
    }

    #[test]
    fn version_one_hashes_stay_untagged_and_match_the_single_key_ring() {
        let single = Keyring::single(key(1));
        let hash = single.hash_for_storage("refresh");
        assert!(!hash.contains(':'));
        assert_eq!(index_version_of(&hash), 1);
        assert_eq!(ring(&[1], 1, &[1], 1).hash_for_storage("refresh"), hash);
    }

    #[test]
    fn a_lookup_binds_the_primary_and_the_other_live_index_key() {
        let old = ring(&[1], 1, &[1], 1).hash_for_storage("refresh");
        // k2 staged: writes stay under k1, lookups also try k2.
        let staged = ring(&[1], 1, &[1, 2], 1);
        let [first, second] = staged.lookup_hashes("refresh");
        assert_eq!(first, old);
        assert!(second.starts_with("k2:"));
        assert_eq!(index_version_of(&second), 2);
        // k2 promoted: writes move to k2, lookups still find k1 rows.
        let promoted = ring(&[1], 1, &[1, 2], 2);
        let new = promoted.hash_for_storage("refresh");
        assert_eq!(new, second);
        assert_eq!(promoted.lookup_hashes("refresh"), [second, old]);
        // One key: both slots carry it.
        let alone = ring(&[1], 1, &[2], 2);
        let [a, b] = alone.lookup_hashes("refresh");
        assert_eq!(a, b);
    }

    #[test]
    fn a_ring_refuses_a_missing_primary_a_short_key_and_a_third_index_key() {
        let one = |v: u32| BTreeMap::from([(v, key(1))]);
        assert!(Keyring::new(one(1), 2, one(1), 1, 1, true).is_err());
        assert!(Keyring::new(one(1), 1, one(1), 2, 1, true).is_err());
        assert!(Keyring::new(BTreeMap::from([(1, vec![0; 16])]), 1, one(1), 1, 1, true).is_err());
        let three = BTreeMap::from([(1, key(1)), (2, key(2)), (3, key(3))]);
        assert!(Keyring::new(one(1), 1, three, 1, 1, true).is_err());
    }

    #[test]
    fn a_stale_cell_refuses_to_seal_and_hash_only_once_adopted() {
        let long_ago = Utc::now() - chrono::Duration::hours(1);

        let legacy = KeyringCell::new(Keyring::single(key(1)));
        legacy
            .confirmed_at_ms
            .store(long_ago.timestamp_millis(), Ordering::Release);
        assert!(legacy.seal(DOMAIN, "secret", AAD).is_ok());
        assert!(legacy.hash_for_storage("refresh").is_ok());

        let adopted = KeyringCell::new(ring(&[1], 1, &[1], 1));
        let sealed = adopted.seal(DOMAIN, "secret", AAD).unwrap();
        adopted
            .confirmed_at_ms
            .store(long_ago.timestamp_millis(), Ordering::Release);
        assert!(adopted.seal(DOMAIN, "secret", AAD).is_err());
        assert!(adopted.hash_for_storage("refresh").is_err());
        assert_eq!(adopted.open(DOMAIN, &sealed, AAD).unwrap(), "secret");
        let [primary, _] = adopted.lookup_hashes("refresh");
        assert_eq!(
            primary,
            adopted.current().hash_for_storage("refresh"),
            "lookups never refuse"
        );

        adopted.confirm(Utc::now());
        assert!(adopted.seal(DOMAIN, "secret", AAD).is_ok());
        adopted.confirm(long_ago);
        assert!(
            adopted.seal(DOMAIN, "secret", AAD).is_ok(),
            "a confirmation never moves back"
        );
    }

    #[test]
    fn install_reaches_every_holder_of_the_cell() {
        let cell = KeyringCell::new(Keyring::single(key(1)));
        let other = Arc::clone(&cell);
        cell.install(ring(&[1, 2], 2, &[1], 1), Utc::now());
        assert_eq!(other.current().data_primary(), 2);
        assert!(other.seal(DOMAIN, "x", AAD).unwrap().starts_with("v2:"));
    }
}
