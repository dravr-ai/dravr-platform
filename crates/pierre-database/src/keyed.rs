// ABOUTME: Keyed digests under the platform's Index key: one domain-separated HMAC for every permanent keyed id
// ABOUTME: Barrage's erasures and log entries, and the global import tombstones, all go through index_mac / index_lookup
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Keyed digests under the Index key
//!
//! A keyed digest names something without naming it in clear: someone who
//! holds only the database cannot confirm a known identity or id, because
//! the digest needs the Index key, the keyset's rotating `index` family.
//!
//! Every such digest is made here, through the backend's own
//! [`HasEncryption::hash_token_for_storage`], on the message
//! `dravr.keyed.v1 ‖ NUL ‖ domain ‖ NUL ‖ hex(message)`. A refresh token never
//! contains a NUL, so no keyed digest is ever a refresh token's blind index,
//! and the domain keeps any two uses apart.
//!
//! The digest is returned **verbatim**, never decoded: under a rotating
//! Index family it carries the version that made it (`k{N}:` before the
//! base64), and a stored digest is never recomputed. A read binds both of
//! [`index_lookup`]'s digests, so one written under the key being retired
//! still matches. The one decoded form is [`index_identity`]: the raw MAC of
//! a name or an id that never moves, under the family's first version, which
//! carries no version tag.
//!
//! **A used key is never retired** (JF, carnet#1347): erasures and
//! tombstones never expire, so the rotation controller never disables an
//! Index version that has been primary, and the family rotates only once.

use pierre_core::errors::AppResult;

use crate::backends::shared::encryption::HasEncryption;

/// The version tag of the keyed-digest message.
const KEYED_FORMAT: &str = "dravr.keyed.v1";

/// What a keyed digest is of. A domain's text is part of its message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyedDomain {
    /// A deleted account's identity, already SHA-256 hashed (`import_tombstones`).
    ImportTombstone,
    /// A barrage domain, by its own text.
    Barrage(&'static str),
    /// The local checkpoint signer's seed (a deployment without KMS).
    CheckpointSeed,
}

impl KeyedDomain {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ImportTombstone => "import_tombstone",
            Self::Barrage(domain) => domain,
            Self::CheckpointSeed => "checkpoint_seed",
        }
    }
}

fn keyed_message(domain: KeyedDomain, message: &[u8]) -> String {
    format!(
        "{KEYED_FORMAT}\0{}\0{}",
        domain.as_str(),
        hex::encode(message)
    )
}

/// The keyed digest of `message` in `domain` to write: under the Index
/// key's primary version, verbatim.
///
/// # Errors
/// The backend's HMAC failure.
pub fn index_mac(
    keys: &dyn HasEncryption,
    domain: KeyedDomain,
    message: &[u8],
) -> AppResult<String> {
    keys.hash_token_for_storage(&keyed_message(domain, message))
}

/// The two digests a read of `message` in `domain` binds: the primary's,
/// then the other live version's, or the primary's twice with one live key.
///
/// # Errors
/// None today; kept fallible so a caller's key failure has a path.
pub fn index_lookup(
    keys: &dyn HasEncryption,
    domain: KeyedDomain,
    message: &[u8],
) -> AppResult<[String; 2]> {
    Ok(keys.blind_index_lookups(&keyed_message(domain, message)))
}

/// The raw 32-byte MAC of `message` in `domain` under the Index family's
/// **first** version.
///
/// For a name or an id that never moves (barrage's `Keyed::identity`: keyed
/// original names, keyed Apple ids, the local checkpoint seed).
///
/// Unlike [`index_mac`], an identity must not follow the primary: a name made
/// once is found again after any rotation, so no stored name is recomputed.
/// It is keyed under Index version 1 whatever the primary, and version 1 is
/// never retired (carnet#1347). A rotation therefore does not re-protect
/// identities: JF accepted that cost on carnet#1542 (2026-11-20).
///
/// # Errors
/// When Index version 1 is not loaded.
pub fn index_identity(
    keys: &dyn HasEncryption,
    domain: KeyedDomain,
    message: &[u8],
) -> AppResult<[u8; 32]> {
    keys.index_identity_mac(&keyed_message(domain, message))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;

    use super::*;
    use crate::backends::shared::keyring::{index_version_of, Keyring, KeyringCell};

    struct Keys(Arc<KeyringCell>);

    impl HasEncryption for Keys {
        fn keyring(&self) -> &Arc<KeyringCell> {
            &self.0
        }
    }

    fn keys(index: &[u32], index_primary: u32) -> Keys {
        let key = |v: u32| vec![u8::try_from(v).unwrap(); 32];
        Keys(KeyringCell::new(
            Keyring::new(
                BTreeMap::from([(1, key(1))]),
                1,
                index.iter().map(|v| (*v, key(*v))).collect(),
                index_primary,
                1,
                true,
            )
            .unwrap(),
        ))
    }

    #[test]
    fn a_keyed_message_carries_its_domain_and_never_reads_as_a_token() {
        let message = keyed_message(KeyedDomain::ImportTombstone, b"ab");
        assert_eq!(
            message,
            concat!("dravr.keyed.v1\0import_tombstone\0", "6162"),
            "bug: the keyed message is not format, domain and hex message"
        );
        assert!(message.contains('\0'), "a refresh token never holds a NUL");
        assert_ne!(
            keyed_message(KeyedDomain::Barrage("barrage.log_record"), b"ab"),
            message,
            "bug: two domains share a keyed message"
        );
    }

    #[test]
    fn a_write_keys_under_the_primary_and_a_read_binds_both_live_versions() {
        let old = keys(&[1], 1);
        let rotated = keys(&[1, 2], 2);
        let domain = KeyedDomain::ImportTombstone;

        let before_rotation = index_mac(&old, domain, b"ab").unwrap();
        let after_rotation = index_mac(&rotated, domain, b"ab").unwrap();
        assert_eq!(index_version_of(&before_rotation), 1);
        assert_eq!(
            index_version_of(&after_rotation),
            2,
            "bug: a write is not under the primary"
        );

        let [primary, other] = index_lookup(&rotated, domain, b"ab").unwrap();
        assert_eq!(
            primary, after_rotation,
            "bug: a lookup does not bind the digest a write stores"
        );
        assert_eq!(
            other, before_rotation,
            "bug: a digest written before the rotation no longer matches"
        );
        assert_eq!(
            index_lookup(&old, domain, b"ab").unwrap(),
            [before_rotation.clone(), before_rotation],
            "with one live key a lookup binds the primary twice"
        );
    }

    #[test]
    fn an_identity_stays_under_index_v1_whatever_the_primary() {
        let domain = KeyedDomain::Barrage("barrage.blob_name");
        let identity = index_identity(&keys(&[1], 1), domain, b"ab").unwrap();
        assert_eq!(
            STANDARD.encode(identity),
            index_mac(&keys(&[1], 1), domain, b"ab").unwrap(),
            "bug: an identity is not Index v1's MAC of the keyed message"
        );
        assert_eq!(
            index_identity(&keys(&[1, 2], 2), domain, b"ab").unwrap(),
            identity,
            "bug: an identity moved with the Index primary"
        );
        assert_ne!(
            index_identity(&keys(&[1], 1), KeyedDomain::CheckpointSeed, b"ab").unwrap(),
            identity,
            "bug: two domains share an identity"
        );
        assert!(
            index_identity(&keys(&[2], 2), domain, b"ab").is_err(),
            "bug: an identity was keyed without Index v1"
        );
    }
}
