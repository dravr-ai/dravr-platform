// ABOUTME: DPoP (RFC 9449) for the authorization and resource servers: proof verification, replay burn, token binding
// ABOUTME: The verifier is tronc's; replay memory is the shared cache; the binding rules are one function both servers call

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sender-constrained tokens (RFC 9449).
//!
//! A client proves possession of a key by signing a `DPoP` proof for each
//! request. The authorization server binds a grant to the key's thumbprint
//! (`dpop_jkt` on the code and refresh token, `cnf.jkt` in the access token);
//! the resource server then admits that access token only under the `DPoP`
//! scheme with a proof signed by the same key.
//!
//! - [`DpopProofs`] verifies one presented proof through tronc's verifier and
//!   burns its `jti` in the shared cache, so a proof is good once across every
//!   replica.
//! - [`DpopProofs::check_presentation`] is the resource server's one rule set:
//!   a bound token presented as Bearer, and the `DPoP` scheme on an unbound
//!   token, are refused; an unbound Bearer token passes untouched.
//!
//! Server nonces (RFC 9449 §8) are not issued, so no request carries one.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use dravr_tronc::dates::Timestamp;
use dravr_tronc::dpop::{
    DpopError, DpopRequest, DpopVerifier, DEFAULT_LEEWAY_SECS, DEFAULT_MAX_AGE_SECS,
    DEFAULT_SIGNING_ALGS,
};
use pierre_cache::{Cache, CacheKey, CacheResource};
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use sha2::{Digest, Sha256};
use tracing::{debug, error};
use uuid::Uuid;

use crate::auth::Claims;

pub use dravr_tronc::dpop::{VerifiedProof, DPOP_HEADER};
pub use dravr_tronc::server::auth::dpop_credential;

/// The `Authorization` scheme a bound token is presented under.
pub const DPOP_SCHEME: &str = "DPoP";

/// The `Authorization` scheme an unbound token is presented under.
pub const BEARER_SCHEME: &str = "Bearer";

/// The RFC 9449 error code for a proof that is missing, malformed or wrong.
pub const INVALID_DPOP_PROOF: &str = "invalid_dpop_proof";

/// The cache's provider segment for burned proof ids.
const REPLAY_PROVIDER_KEY: &str = "dpop";

/// The algorithms a proof may be signed with.
///
/// For the authorization and resource server metadata and the `algs` of a
/// `DPoP` challenge. Read from the verifier itself, so what is published is
/// what is accepted.
#[must_use]
pub fn signing_algorithms() -> Vec<String> {
    DpopVerifier::new(&DEFAULT_SIGNING_ALGS).algorithm_names()
}

/// The `WWW-Authenticate` challenge a refused `DPoP` request is answered with
/// (RFC 9449 §7.1), beside the Bearer one: the accepted algorithms, and the
/// error code when the refusal names one.
#[must_use]
pub fn dpop_challenge(error: Option<&str>) -> String {
    let algs = signing_algorithms().join(" ");
    error.map_or_else(
        || format!("{DPOP_SCHEME} algs=\"{algs}\""),
        |error| format!("{DPOP_SCHEME} error=\"{error}\", algs=\"{algs}\""),
    )
}

/// The scheme an access token was presented under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentedScheme {
    /// `Authorization: Bearer <token>`, or a credential with no scheme
    Bearer,
    /// `Authorization: DPoP <token>`
    Dpop,
}

/// The `DPoP` headers one request carries, as the transport counted them.
/// RFC 9449 §4.3 refuses a request with more than one.
#[derive(Debug, Clone, Copy, Default)]
pub struct PresentedProofs<'a> {
    /// The proof, when exactly one `DPoP` header was sent; the first when
    /// several were (refused by its count)
    pub proof: Option<&'a str>,
    /// How many `DPoP` headers the request carried
    pub count: usize,
}

impl<'a> PresentedProofs<'a> {
    /// The one proof, or why there is not exactly one.
    fn single(&self) -> Result<&'a str, DpopRefusal> {
        match (self.count, self.proof) {
            (1, Some(proof)) => Ok(proof),
            (0, _) | (_, None) => Err(DpopRefusal::Missing),
            _ => Err(DpopRefusal::Multiple),
        }
    }
}

/// What a resource server request presents for the binding check.
///
/// The proofs it carries, its method, and the URI the client dialed (`None`
/// when the dialed origin is not one of this server's, which no proof can then
/// match).
#[derive(Debug, Clone, Copy)]
pub struct DpopContext<'a> {
    /// The request's `DPoP` headers
    pub proofs: PresentedProofs<'a>,
    /// The HTTP method, compared with the proof's `htm`
    pub htm: &'a str,
    /// The dialed URI without query, compared with the proof's `htu`
    pub htu: Option<&'a str>,
}

/// Why a `DPoP` check refused a request.
#[derive(Debug)]
#[non_exhaustive]
pub enum DpopRefusal {
    /// A proof was required and the request carried none
    Missing,
    /// The request carried more than one `DPoP` header
    Multiple,
    /// The proof failed one of RFC 9449 §4.3's checks
    Invalid(DpopError),
    /// The proof's `jti` was already used within its lifetime
    Replayed,
    /// The proof was signed by another key than the one the grant is bound to
    KeyMismatch,
    /// The dialed origin is none of this server's, so no `htu` can match
    UnknownUri,
    /// A token bound to a key was presented as Bearer (RFC 9449 §7.2)
    BoundTokenAsBearer,
    /// The `DPoP` scheme was used with a token bound to no key
    UnboundTokenAsDpop,
    /// The replay memory could not be read or written; refused, never admitted
    Unavailable(AppError),
}

impl DpopRefusal {
    /// The RFC 9449 / RFC 6750 error code the response names.
    #[must_use]
    pub const fn error_code(&self) -> &'static str {
        match self {
            Self::BoundTokenAsBearer | Self::UnboundTokenAsDpop => "invalid_token",
            Self::Unavailable(_) => "server_error",
            Self::Missing
            | Self::Multiple
            | Self::Invalid(_)
            | Self::Replayed
            | Self::KeyMismatch
            | Self::UnknownUri => INVALID_DPOP_PROOF,
        }
    }

    /// Whether the refusal is the server failing rather than the request.
    #[must_use]
    pub const fn is_server_fault(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

impl fmt::Display for DpopRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("a DPoP proof is required"),
            Self::Multiple => f.write_str("more than one DPoP header was sent"),
            Self::Invalid(error) => write!(f, "{error}"),
            Self::Replayed => f.write_str("the DPoP proof was already used"),
            Self::KeyMismatch => {
                f.write_str("the DPoP proof is not signed by the key the grant is bound to")
            }
            Self::UnknownUri => f.write_str("the request URI is not one of this server's"),
            Self::BoundTokenAsBearer => {
                f.write_str("a DPoP-bound access token must be presented with the DPoP scheme")
            }
            Self::UnboundTokenAsDpop => {
                f.write_str("the DPoP scheme was used with an access token bound to no key")
            }
            Self::Unavailable(_) => f.write_str("DPoP replay protection is unavailable"),
        }
    }
}

/// Verifies `DPoP` proofs and remembers every accepted `jti` until the proof
/// could no longer pass the `iat` window.
#[derive(Clone)]
pub struct DpopProofs {
    verifier: DpopVerifier,
    replay: Arc<Cache>,
}

impl fmt::Debug for DpopProofs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DpopProofs")
            .field("verifier", &self.verifier)
            .finish_non_exhaustive()
    }
}

impl DpopProofs {
    /// Proofs verified with tronc's defaults (300 s max age, 60 s leeway,
    /// [`signing_algorithms`]), burned in `replay`. A Redis-backed cache makes
    /// the burn hold across replicas.
    #[must_use]
    pub fn new(replay: Arc<Cache>) -> Self {
        Self {
            verifier: DpopVerifier::new(&DEFAULT_SIGNING_ALGS)
                .with_max_age(DEFAULT_MAX_AGE_SECS)
                .with_leeway(DEFAULT_LEEWAY_SECS),
            replay,
        }
    }

    /// Verify the one proof `proofs` carries for a request to `htu` with
    /// `htm`, bound to `access_token` when one is presented, and burn its
    /// `jti`.
    ///
    /// # Errors
    ///
    /// A [`DpopRefusal`] naming the first check the request fails; a replay
    /// memory that cannot be reached refuses as
    /// [`DpopRefusal::Unavailable`].
    pub async fn verify(
        &self,
        proofs: PresentedProofs<'_>,
        htm: &str,
        htu: Option<&str>,
        access_token: Option<&str>,
    ) -> Result<VerifiedProof, DpopRefusal> {
        let proof = proofs.single()?;
        let htu = htu.ok_or(DpopRefusal::UnknownUri)?;
        let verified = self
            .verifier
            .verify(
                proof,
                &DpopRequest {
                    htm,
                    htu,
                    access_token,
                    nonce: None,
                },
                Timestamp::now(),
            )
            .map_err(DpopRefusal::Invalid)?;
        self.burn(&verified).await?;
        Ok(verified)
    }

    /// The resource server's binding rules for an access token whose
    /// signature and claims already verified.
    ///
    /// - a token with `cnf` presented as Bearer is refused;
    /// - the `DPoP` scheme on a token without `cnf` is refused;
    /// - `DPoP` with `cnf` needs exactly one proof for this request and this
    ///   token (`ath`), signed by the bound key, never seen before;
    /// - an unbound Bearer token passes, exactly as before `DPoP` existed.
    ///
    /// `dpop` is `None` on a path that takes no proof (REST), where a bound
    /// token is therefore always refused.
    ///
    /// # Errors
    ///
    /// The [`DpopRefusal`] the request is answered with.
    pub async fn check_presentation(
        &self,
        claims: &Claims,
        token: &str,
        scheme: PresentedScheme,
        dpop: Option<&DpopContext<'_>>,
    ) -> Result<(), DpopRefusal> {
        check_binding(claims, scheme)?;
        let Some(confirmation) = claims.cnf.as_ref() else {
            return Ok(());
        };
        let context = dpop.ok_or(DpopRefusal::Missing)?;
        let verified = self
            .verify(context.proofs, context.htm, context.htu, Some(token))
            .await?;
        if verified.jkt != confirmation.jkt {
            return Err(DpopRefusal::KeyMismatch);
        }
        Ok(())
    }

    /// Remember the proof's `jti` under its key for as long as the proof
    /// could pass the `iat` window, refusing one already remembered.
    async fn burn(&self, proof: &VerifiedProof) -> Result<(), DpopRefusal> {
        // The jti is the client's string: hashed, so its length cannot grow
        // the key.
        let jti = URL_SAFE_NO_PAD.encode(Sha256::digest(proof.jti.as_bytes()));
        let key = CacheKey::new(
            TenantId::nil(),
            Uuid::nil(),
            REPLAY_PROVIDER_KEY.to_owned(),
            CacheResource::Custom(format!("{}:{jti}", proof.jkt)),
        );
        let lifetime = self
            .verifier
            .max_age_secs()
            .saturating_add(DEFAULT_LEEWAY_SECS);
        let ttl = Duration::from_secs(u64::try_from(lifetime).unwrap_or(0));
        match self.replay.set_if_absent(&key, &true, ttl).await {
            Ok(true) => Ok(()),
            Ok(false) => {
                debug!("DPoP proof refused: its jti was already used");
                Err(DpopRefusal::Replayed)
            }
            Err(e) => {
                error!(error = %e, "DPoP replay memory unavailable; the proof is refused");
                Err(DpopRefusal::Unavailable(e))
            }
        }
    }
}

/// The scheme half of the binding rules, which needs no proof: a bound token
/// presented as Bearer, and the `DPoP` scheme on an unbound token, are refused.
///
/// # Errors
///
/// [`DpopRefusal::BoundTokenAsBearer`] or [`DpopRefusal::UnboundTokenAsDpop`].
pub const fn check_binding(claims: &Claims, scheme: PresentedScheme) -> Result<(), DpopRefusal> {
    match (scheme, claims.cnf.is_some()) {
        (PresentedScheme::Bearer, true) => Err(DpopRefusal::BoundTokenAsBearer),
        (PresentedScheme::Dpop, false) => Err(DpopRefusal::UnboundTokenAsDpop),
        (PresentedScheme::Bearer, false) | (PresentedScheme::Dpop, true) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{dpop_challenge, signing_algorithms, PresentedProofs};

    #[test]
    fn the_challenge_names_every_accepted_algorithm() {
        let algs = signing_algorithms();
        assert!(
            algs.iter().any(|alg| alg == "ES256"),
            "bug: ES256 is the algorithm every DPoP client signs with: {algs:?}"
        );
        let challenge = dpop_challenge(Some("invalid_dpop_proof"));
        assert!(
            challenge.starts_with("DPoP error=\"invalid_dpop_proof\", algs=\""),
            "bug: the challenge names its error, then the algorithms: {challenge}"
        );
        let bare = dpop_challenge(None);
        assert!(
            bare.starts_with("DPoP algs=\"") && !bare.contains("error="),
            "bug: a challenge with no error names only the algorithms: {bare}"
        );
        for alg in &algs {
            assert!(
                challenge.contains(alg.as_str()),
                "{alg} missing from {challenge}"
            );
        }
    }

    #[test]
    fn exactly_one_proof_is_taken() {
        let proofs = |proof, count| PresentedProofs { proof, count };
        assert!(
            proofs(None, 0).single().is_err(),
            "bug: no DPoP header is no proof"
        );
        assert_eq!(
            proofs(Some("a"), 1).single().ok(),
            Some("a"),
            "bug: one DPoP header is the proof"
        );
        let counted_two = PresentedProofs {
            proof: Some("a"),
            count: 2,
        };
        assert!(
            counted_two.single().is_err(),
            "bug: RFC 9449 §4.3 refuses more than one DPoP header"
        );
    }
}
