// ABOUTME: An iOS App Attest key whose attestation verified: its public key, the service that vouched for it, its counter
// ABOUTME: Keyed by the key id the app names it by; checks every later assertion the same install signs

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};

/// Which App Attest service vouched for a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAttestEnvironment {
    /// Apple's production service: App Store, `TestFlight` and ad hoc builds
    Production,
    /// Apple's development service: a build signed with a development profile
    Development,
}

impl AppAttestEnvironment {
    /// The name stored beside the key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Development => "development",
        }
    }

    /// The environment a stored name denotes; `None` for any other name.
    #[must_use]
    pub fn from_stored(name: &str) -> Option<Self> {
        match name {
            "production" => Some(Self::Production),
            "development" => Some(Self::Development),
            _ => None,
        }
    }
}

/// A Secure Enclave key in an install of Dravr's iOS app that Apple attested
/// (carnet#810).
///
/// The key belongs to the install, not to an account: an athlete who signs
/// out and another who signs in on the same phone are vouched for by the same
/// key. So the record names no user or tenant, and holds nothing personal —
/// a public key and a counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppAttestKey {
    /// The key id the app names the key by: standard base64 of the SHA-256
    /// of the public key.
    pub key_id: String,
    /// The attested public key, an uncompressed P-256 point.
    pub public_key: Vec<u8>,
    /// The App Attest service that vouched for it.
    pub environment: AppAttestEnvironment,
    /// The counter of the last assertion accepted from this key; zero until
    /// its first assertion.
    pub sign_count: u32,
    /// When the attestation was verified.
    pub created_at: DateTime<Utc>,
    /// When the key last vouched for a sign-in.
    pub last_used_at: DateTime<Utc>,
}
