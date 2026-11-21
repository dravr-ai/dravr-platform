// ABOUTME: Keyed digests under the platform's Index key: one domain-separated HMAC for every permanent keyed id
// ABOUTME: Barrage's erasures and log entries, and the global import tombstones, all go through index_mac / index_lookup
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Keyed digests under the Index key
//!
//! A keyed digest names something without naming it in clear: someone who
//! holds only the database cannot confirm a known identity or id, because
//! the digest needs the Index key (today the blind-index key pinned to DEK
//! v1, carnet#1347 makes it a rotating family).
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
//! **Retirement is carnet#1347's to settle**: erasures and tombstones never
//! expire, so a version these digests use must not be disabled while they
//! hold it, or they must be re-keyed from plaintext before carnet#1550 drops
//! it (open question 5 of the owl phase 3 plan). On main the key never
//! retires.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use pierre_core::errors::{AppError, AppResult};

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
/// then the other live version's. With one live key (main today) the
/// primary's twice; carnet#1347 maps this onto its `blind_index_lookups`.
///
/// # Errors
/// The backend's HMAC failure.
pub fn index_lookup(
    keys: &dyn HasEncryption,
    domain: KeyedDomain,
    message: &[u8],
) -> AppResult<[String; 2]> {
    let primary = index_mac(keys, domain, message)?;
    Ok([primary.clone(), primary])
}

/// The raw 32-byte MAC of `message` in `domain` under the Index family's
/// **first** version.
///
/// For a name or an id that never moves (barrage's `Keyed::identity`: keyed
/// original names, keyed Apple ids, the local checkpoint seed).
///
/// Unlike [`index_mac`], an identity must not follow the primary: a name made
/// once is found again after any rotation, so no stored name is recomputed.
/// On main the Index key is the blind-index key pinned to DEK v1, which is
/// that first version; carnet#1347 maps this onto its version 1, whatever its
/// primary, and never retires it. A rotation therefore does not re-protect
/// identities: JF accepted that cost on carnet#1542 (2026-11-20). The
/// backend's base64 digest is decoded, so a versioned (`k{N}:`) digest is
/// refused rather than read as an identity.
///
/// # Errors
/// The backend's HMAC failure, or a digest that is not 32 base64 bytes.
pub fn index_identity(
    keys: &dyn HasEncryption,
    domain: KeyedDomain,
    message: &[u8],
) -> AppResult<[u8; 32]> {
    let digest = keys.hash_token_for_storage(&keyed_message(domain, message))?;
    BASE64
        .decode(&digest)
        .ok()
        .and_then(|raw| <[u8; 32]>::try_from(raw).ok())
        .ok_or_else(|| AppError::internal("The Index key's digest is not 32 base64 bytes"))
}

#[cfg(test)]
mod tests {
    use ring::hmac;

    use super::*;

    struct Echo;

    impl HasEncryption for Echo {
        fn encrypt_data_with_aad(&self, data: &str, _aad: &str) -> AppResult<String> {
            Ok(data.to_owned())
        }

        fn decrypt_data_with_aad(&self, encrypted: &str, _aad: &str) -> AppResult<String> {
            Ok(encrypted.to_owned())
        }

        fn hash_token_for_storage(&self, token: &str) -> AppResult<String> {
            Ok(token.to_owned())
        }
    }

    #[test]
    fn a_keyed_message_carries_its_domain_and_never_reads_as_a_token() {
        let mac = index_mac(&Echo, KeyedDomain::ImportTombstone, b"ab").unwrap();
        assert_eq!(
            mac,
            concat!("dravr.keyed.v1\0import_tombstone\0", "6162"),
            "bug: the keyed message is not format, domain and hex message"
        );
        assert!(mac.contains('\0'), "a refresh token never holds a NUL");
        assert_ne!(
            index_mac(&Echo, KeyedDomain::Barrage("barrage.log_record"), b"ab").unwrap(),
            mac,
            "bug: two domains share a keyed message"
        );
        let [a, b] = index_lookup(&Echo, KeyedDomain::ImportTombstone, b"ab").unwrap();
        assert_eq!(
            (a.as_str(), b.as_str()),
            (mac.as_str(), mac.as_str()),
            "bug: a lookup does not bind the digest a write stores"
        );
    }

    /// The backend's shape: a base64 HMAC-SHA256 under one key.
    struct Hmac;

    impl HasEncryption for Hmac {
        fn encrypt_data_with_aad(&self, data: &str, _aad: &str) -> AppResult<String> {
            Ok(data.to_owned())
        }

        fn decrypt_data_with_aad(&self, encrypted: &str, _aad: &str) -> AppResult<String> {
            Ok(encrypted.to_owned())
        }

        fn hash_token_for_storage(&self, token: &str) -> AppResult<String> {
            let key = hmac::Key::new(hmac::HMAC_SHA256, b"index key");
            Ok(BASE64.encode(hmac::sign(&key, token.as_bytes()).as_ref()))
        }
    }

    #[test]
    fn an_identity_is_the_raw_mac_a_keyed_digest_encodes() {
        let domain = KeyedDomain::Barrage("barrage.blob_name");
        let identity = index_identity(&Hmac, domain, b"ab").unwrap();
        assert_eq!(
            BASE64.encode(identity),
            index_mac(&Hmac, domain, b"ab").unwrap(),
            "bug: an identity is not the Index key's MAC of the keyed message"
        );
        assert_ne!(
            index_identity(&Hmac, KeyedDomain::CheckpointSeed, b"ab").unwrap(),
            identity,
            "bug: two domains share an identity"
        );
        assert!(
            index_identity(&Echo, domain, b"ab").is_err(),
            "bug: a digest that is not 32 base64 bytes reads as an identity"
        );
    }
}
