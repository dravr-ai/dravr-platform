// ABOUTME: Decodes a JWT signed by one of Google's published keys, against the claim checks its caller sets
// ABOUTME: Firebase and Google sign-in ID tokens share it: tronc resolves the kid's key, the caller's Validation checks the claims
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Google signs Firebase ID tokens and its own `OpenID` Connect ID tokens the
//! same way: RS256 under a key it publishes as a JWK, named by the token's
//! `kid`. [`GoogleKeySet::decoding_key_for`] resolves that key; what differs
//! is only the key set and the claims each caller expects, so each caller
//! supplies its [`GoogleKeySet`] and [`Validation`], and this module turns the
//! outcome into the platform's errors once.

use dravr_tronc::iam::{GoogleKeySet, IamError};
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{decode, Validation};
use pierre_core::errors::{AppError, AppResult};
use serde::de::DeserializeOwned;
use tracing::{debug, warn};

/// Decode `token` with the key its `kid` names in `keys`, checked against
/// `validation`. `label` names the token kind in log lines and in the one
/// server-side error.
///
/// # Errors
///
/// A token that is malformed, names no published key, fails its signature,
/// is expired or fails a claim check is refused with an authentication error
/// (`auth_expired` for an expired one). A key set that cannot be fetched is
/// an internal error: the token was never judged.
pub async fn decode_google_signed<C: DeserializeOwned>(
    keys: &GoogleKeySet,
    token: &str,
    validation: &Validation,
    label: &str,
) -> AppResult<C> {
    let decoding_key = keys.decoding_key_for(token).await.map_err(|e| match e {
        IamError::Rejected(why) => {
            debug!(reason = %why, "{label} token names no usable published signing key");
            AppError::auth_invalid("Invalid token")
        }
        other => {
            warn!(error = %other, "{label} signing keys are unavailable");
            AppError::internal(format!("{label} signing keys are unavailable"))
        }
    })?;

    decode::<C>(token, &decoding_key, validation)
        .map(|data| data.claims)
        .map_err(|e| {
            debug!(error = %e, "{label} token validation failed");
            match e.kind() {
                ErrorKind::ExpiredSignature => AppError::auth_expired(),
                ErrorKind::InvalidAudience => AppError::auth_invalid("Invalid token audience"),
                ErrorKind::InvalidIssuer => AppError::auth_invalid("Invalid token issuer"),
                _ => AppError::auth_invalid("Invalid token"),
            }
        })
}
