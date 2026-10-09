// ABOUTME: The mobile app's attestation on its code exchange: App Attest (iOS) or Play Integrity (Android), read and verified
// ABOUTME: Verified before the code is redeemed, an App Attest key recorded after, so a refusal leaves the code for a retry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Device attestation on the mobile sign-in (carnet#810).
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
//! That is App Attest, from the iOS app. The Android app sends a Play
//! Integrity token instead, requested over [`request_hash`] of the same code,
//! which Google decodes into a verdict ([`verify_play_integrity`]). Play
//! keeps no key on this server, so there is nothing to record after the
//! exchange.
//!
//! Attestation adds a signal to the first-party sign-in; it does not
//! authenticate anyone. The athlete authenticated on the hosted page, and
//! PKCE ties the code to the app that started the flow. Evidence that is
//! present must verify. Evidence that is absent — or a Play Integrity token
//! Google could not be asked about — is still accepted: enforcement waits
//! until both apps attest in production.

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
use crate::oauth2_server::play_integrity::{
    request_hash, verify_verdict, PlayIntegrityError, PlayIntegrityVerdict,
};
use crate::oauth2_server::play_integrity_decoder::{DecodeError, PlayIntegrityDecoder};

/// The attestation evidence a mobile code exchange carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobileEvidence {
    /// The iOS app's App Attest key and its attestation or assertion
    AppAttest(AppAttestEvidence),
    /// The Android app's Play Integrity token
    PlayIntegrity(String),
}

impl MobileEvidence {
    /// Read the evidence off the token request: App Attest's three fields
    /// ([`AppAttestEvidence::from_request`]), or the Play Integrity token
    /// alone, or nothing.
    ///
    /// # Errors
    /// `invalid_request` for a malformed App Attest combination, or for a
    /// Play Integrity token sent beside any App Attest field.
    pub fn from_request(
        key_id: Option<&str>,
        attestation: Option<&str>,
        assertion: Option<&str>,
        play_integrity_token: Option<&str>,
    ) -> Result<Option<Self>, OAuth2Error> {
        let app_attest = AppAttestEvidence::from_request(key_id, attestation, assertion)?;
        match (app_attest, play_integrity_token) {
            (None, None) => Ok(None),
            (Some(evidence), None) => Ok(Some(Self::AppAttest(evidence))),
            (None, Some(token)) => Ok(Some(Self::PlayIntegrity(token.to_owned()))),
            (Some(_), Some(_)) => Err(OAuth2Error::invalid_request(
                "Send either App Attest evidence or play_integrity_token, not both",
            )),
        }
    }
}

/// What became of a Play Integrity token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayIntegrityOutcome {
    /// Google decoded it and the verdict passed every check
    Verified(PlayIntegrityVerdict),
    /// Google could not be asked; the exchange proceeds as if no evidence
    /// had been sent
    Unavailable,
}

impl PlayIntegrityOutcome {
    /// The name the sign-in's span records the outcome under.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Verified(_) => "play_integrity",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Have Google decode the Play Integrity token a code exchange carried, and
/// check the verdict against the authorization code it carried it for.
///
/// # Errors
/// `invalid_client` when Google refuses the token or the verdict fails a
/// check.
pub async fn verify_play_integrity(
    decoder: &dyn PlayIntegrityDecoder,
    token: &str,
    code: &str,
    package_name: &str,
    now: DateTime<Utc>,
) -> Result<PlayIntegrityOutcome, OAuth2Error> {
    let payload = match decoder.decode(token).await {
        Ok(payload) => payload,
        Err(DecodeError::Unavailable) => {
            warn!("Play Integrity token not checked: Google could not be asked");
            return Ok(PlayIntegrityOutcome::Unavailable);
        }
        Err(DecodeError::Rejected) => {
            warn!("Play Integrity token refused by Google");
            return Err(invalid_client("Play Integrity token was not accepted"));
        }
    };
    verify_verdict(&payload, &request_hash(code), package_name, now)
        .map(PlayIntegrityOutcome::Verified)
        .map_err(refused_verdict)
}

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

/// `invalid_client` for a Play Integrity verdict that failed a check.
fn refused_verdict(error: PlayIntegrityError) -> OAuth2Error {
    warn!(reason = %error, "Play Integrity verdict refused");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn read(
        key_id: Option<&str>,
        attestation: Option<&str>,
        assertion: Option<&str>,
        play: Option<&str>,
    ) -> Result<Option<MobileEvidence>, String> {
        MobileEvidence::from_request(key_id, attestation, assertion, play).map_err(|e| e.error)
    }

    #[test]
    fn no_field_is_no_evidence() {
        assert_eq!(read(None, None, None, None), Ok(None));
    }

    #[test]
    fn a_play_integrity_token_alone_is_play_evidence() {
        assert_eq!(
            read(None, None, None, Some("token")),
            Ok(Some(MobileEvidence::PlayIntegrity("token".to_owned())))
        );
    }

    #[test]
    fn app_attest_evidence_alone_is_app_attest_evidence() {
        assert_eq!(
            read(Some("key"), None, Some("assertion"), None),
            Ok(Some(MobileEvidence::AppAttest(AppAttestEvidence {
                key_id: "key".to_owned(),
                proof: AppAttestProof::Assertion("assertion".to_owned()),
            })))
        );
    }

    #[test]
    fn a_play_integrity_token_beside_any_app_attest_field_is_malformed() {
        for (key_id, attestation, assertion) in [
            (Some("key"), None, Some("assertion")),
            (Some("key"), Some("attestation"), None),
            (Some("key"), None, None),
            (None, None, Some("assertion")),
        ] {
            assert_eq!(
                read(key_id, attestation, assertion, Some("token")),
                Err("invalid_request".to_owned())
            );
        }
    }

    /// The span values are a wire contract, not an implementation detail:
    /// the `app_attest` field's `play_integrity` and `unavailable` shares are
    /// what the `mobile-attestation-enforcement` arming criterion in
    /// `feature-phases.yaml` is read from in Cloud Logging.
    #[test]
    fn outcomes_name_their_span_value() {
        assert_eq!(
            PlayIntegrityOutcome::Verified(PlayIntegrityVerdict {
                strong_integrity: true
            })
            .kind(),
            "play_integrity"
        );
        assert_eq!(PlayIntegrityOutcome::Unavailable.kind(), "unavailable");
    }
}
