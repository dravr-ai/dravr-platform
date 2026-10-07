// ABOUTME: The iOS app's App Attest evidence on its code exchange: read it off the request, verify it, record the key
// ABOUTME: Verified before the code is redeemed, recorded after, so a refusal leaves the code for a retry with fresh evidence
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! App Attest on the mobile sign-in (carnet#810).
//!
//! When Dravr's iOS app redeems the code the hosted login page issued, it
//! sends evidence that the request comes from a genuine install of the app:
//! on its first sign-in, the attestation of a fresh Secure Enclave key; on
//! every later one, an assertion signed with that key. Both bind to the
//! authorization code itself — its bytes are the client data the device
//! signs — so the evidence is good for that one exchange and no other. The
//! code is server-issued, single-use and short-lived, which is the challenge
//! Apple asks the server to supply.
//!
//! Attestation adds a signal to the first-party sign-in; it does not
//! authenticate anyone. The athlete authenticated on the hosted page, and
//! PKCE ties the code to the app that started the flow. Evidence that is
//! present must verify. Evidence that is absent is accepted until the Android
//! app attests too (Play Integrity, carnet#810), because the server cannot
//! tell an Android install from a script claiming to be one.

use std::time::Duration;

use chrono::{DateTime, Utc};
use pierre_core::models::AppAttestKey;
use pierre_database::repositories::AppAttestKeyRepository;
use rustls_pki_types::UnixTime;
use tracing::warn;

use crate::oauth2_server::app_attest::{
    verify_assertion, verify_attestation, AppAttestError, AttestedKey,
};
use crate::oauth2_server::models::OAuth2Error;

/// The App Attest evidence a code exchange carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppAttestEvidence {
    /// The key the evidence is about, as the app names it
    pub key_id: String,
    /// The attestation or assertion
    pub proof: AppAttestProof,
}

/// What the app proves with its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAttestProof {
    /// The install's first sign-in: Apple's attestation of a new key
    Attestation(String),
    /// A later sign-in: an assertion signed by the registered key
    Assertion(String),
}

impl AppAttestEvidence {
    /// Read the evidence off the token request's three fields: none of them
    /// (no evidence), or the key id with exactly one of attestation and
    /// assertion.
    ///
    /// # Errors
    /// `invalid_request` for any other combination.
    pub fn from_request(
        key_id: Option<&str>,
        attestation: Option<&str>,
        assertion: Option<&str>,
    ) -> Result<Option<Self>, OAuth2Error> {
        let (key_id, proof) = match (key_id, attestation, assertion) {
            (None, None, None) => return Ok(None),
            (Some(key_id), Some(attestation), None) => {
                (key_id, AppAttestProof::Attestation(attestation.to_owned()))
            }
            (Some(key_id), None, Some(assertion)) => {
                (key_id, AppAttestProof::Assertion(assertion.to_owned()))
            }
            _ => {
                return Err(OAuth2Error::invalid_request(
                    "App Attest evidence is app_attest_key_id with exactly one of \
                     app_attest_attestation and app_attest_assertion",
                ))
            }
        };
        Ok(Some(Self {
            key_id: key_id.to_owned(),
            proof,
        }))
    }
}

/// Evidence that verified, waiting for the code exchange to succeed before
/// it is recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifiedEvidence {
    /// A newly attested key, to register
    NewKey(AttestedKey),
    /// An assertion by a registered key, whose counter to store
    Assertion {
        /// The registered key
        key_id: String,
        /// The assertion's counter
        sign_count: u32,
    },
}

impl VerifiedEvidence {
    /// The name the sign-in's span records the evidence under.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NewKey(_) => "attestation",
            Self::Assertion { .. } => "assertion",
        }
    }
}

/// Verify the evidence a code exchange carried, for the authorization code
/// it carried it for. Reads the registered key for an assertion; writes
/// nothing.
///
/// # Errors
/// `invalid_client` when the evidence does not verify or names a key this
/// server never registered, `server_error` when the key cannot be read.
pub async fn verify_evidence(
    keys: &dyn AppAttestKeyRepository,
    evidence: &AppAttestEvidence,
    code: &str,
    app_id: &str,
    now: DateTime<Utc>,
) -> Result<VerifiedEvidence, OAuth2Error> {
    match &evidence.proof {
        AppAttestProof::Attestation(attestation) => verify_attestation(
            attestation,
            &evidence.key_id,
            code.as_bytes(),
            app_id,
            unix_time(now),
        )
        .map(VerifiedEvidence::NewKey)
        .map_err(refused),
        AppAttestProof::Assertion(assertion) => {
            let key = keys.find_key(&evidence.key_id).await.map_err(|e| {
                warn!(error = %e, "App Attest key lookup failed");
                OAuth2Error::server_error("App Attest key could not be read")
            })?;
            let Some(key) = key else {
                warn!("App Attest assertion refused: key not registered");
                return Err(invalid_client("App Attest key is not registered"));
            };
            let sign_count = verify_assertion(
                assertion,
                code.as_bytes(),
                app_id,
                &key.public_key,
                key.sign_count,
            )
            .map_err(refused)?;
            Ok(VerifiedEvidence::Assertion {
                key_id: key.key_id,
                sign_count,
            })
        }
    }
}

/// Record evidence that verified, once the code it was for has been
/// redeemed: register the new key, or store the assertion's counter.
///
/// # Errors
/// `invalid_client` when the key was registered already or the counter was
/// spent by a concurrent sign-in, `server_error` when the write fails.
pub async fn record_evidence(
    keys: &dyn AppAttestKeyRepository,
    verified: VerifiedEvidence,
    now: DateTime<Utc>,
) -> Result<(), OAuth2Error> {
    let stored = match verified {
        VerifiedEvidence::NewKey(attested) => {
            keys.register_key(&AppAttestKey {
                key_id: attested.key_id,
                public_key: attested.public_key,
                environment: attested.environment,
                sign_count: 0,
                created_at: now,
                last_used_at: now,
            })
            .await
        }
        VerifiedEvidence::Assertion { key_id, sign_count } => {
            keys.advance_counter(&key_id, sign_count, now).await
        }
    }
    .map_err(|e| {
        warn!(error = %e, "App Attest key write failed");
        OAuth2Error::server_error("App Attest key could not be stored")
    })?;
    if stored {
        Ok(())
    } else {
        warn!("App Attest evidence refused: key already registered or counter already spent");
        Err(invalid_client(
            "App Attest key was already registered or its counter already spent",
        ))
    }
}

/// `invalid_client` for evidence that did not verify: attestation is how
/// the app authenticates as itself, as in the OAuth attestation-based client
/// authentication draft.
fn refused(error: AppAttestError) -> OAuth2Error {
    warn!(reason = %error, "App Attest evidence refused");
    invalid_client(&error.to_string())
}

fn invalid_client(description: &str) -> OAuth2Error {
    OAuth2Error {
        error_description: Some(description.to_owned()),
        ..OAuth2Error::invalid_client()
    }
}

/// `now` as the certificate-validity clock webpki reads. A time before the
/// epoch reads as the epoch, which no certificate in Apple's chain is valid at.
fn unix_time(now: DateTime<Utc>) -> UnixTime {
    UnixTime::since_unix_epoch(Duration::from_secs(
        u64::try_from(now.timestamp()).unwrap_or_default(),
    ))
}
