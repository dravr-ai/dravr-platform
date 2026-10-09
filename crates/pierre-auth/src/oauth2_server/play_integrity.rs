// ABOUTME: Google Play Integrity verification: the verdict Google decoded from the Android app's integrity token
// ABOUTME: Pure checks over the decoded payload — package, request hash, freshness, app and device verdicts; decoding is the caller's
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Google Play Integrity, server side (carnet#810).
//!
//! The Android app asks Google Play for an integrity token over a request
//! hash it computes from the authorization code it is about to redeem
//! ([`request_hash`]). It uses the standard API, whose tokens only Google can
//! decrypt: the server hands the token to `decodeIntegrityToken` and receives
//! the verdict as JSON ([`TokenPayload`]). This module checks that verdict,
//! following "Integrity verdicts" in the Play Integrity documentation:
//!
//! - `requestDetails` names this app's package, carries the hash of the code
//!   being redeemed, and was issued within [`MAX_TOKEN_AGE`] (allowing
//!   [`MAX_CLOCK_SKEW`] of clock drift ahead).
//! - `appIntegrity` says Google Play recognises the binary
//!   (`PLAY_RECOGNIZED`) and names the same package.
//! - `deviceIntegrity` says the device is a genuine, certified Android device
//!   (`MEETS_DEVICE_INTEGRITY`). `MEETS_STRONG_INTEGRITY` (a hardware-backed
//!   keystore and recent security patches) is recorded, not required.

use std::error::Error;
use std::fmt;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// How old a token may be when it is redeemed. The app requests it right
/// before the code exchange; the code itself lives for minutes.
pub const MAX_TOKEN_AGE: Duration = Duration::minutes(10);

/// How far ahead of this server's clock a token's timestamp may be.
pub const MAX_CLOCK_SKEW: Duration = Duration::minutes(1);

/// The app verdict of a binary Google Play distributed.
const PLAY_RECOGNIZED: &str = "PLAY_RECOGNIZED";

/// The device verdict of a genuine, certified Android device.
const MEETS_DEVICE_INTEGRITY: &str = "MEETS_DEVICE_INTEGRITY";

/// The device verdict of a device with a hardware-backed keystore and recent
/// security updates.
const MEETS_STRONG_INTEGRITY: &str = "MEETS_STRONG_INTEGRITY";

/// The verdict `decodeIntegrityToken` returns as `tokenPayloadExternal`.
///
/// Only the fields the checks read are named; Google adds others
/// (`accountDetails`, `environmentDetails`, the signing certificates, the
/// version code, …) that are ignored. Fields a
/// verdict may leave out are optional here so that their absence is a
/// refusal with a reason, not a parse failure.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenPayload {
    /// What the app asked for
    pub request_details: RequestDetails,
    /// What Google Play knows of the binary
    #[serde(default)]
    pub app_integrity: AppIntegrity,
    /// What Google Play knows of the device
    #[serde(default)]
    pub device_integrity: DeviceIntegrity,
}

/// `requestDetails`: the request the token was minted for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestDetails {
    /// The package that requested the token
    #[serde(default)]
    pub request_package_name: String,
    /// The request hash the app passed (standard requests only)
    #[serde(default)]
    pub request_hash: Option<String>,
    /// When the token was minted, in milliseconds since the epoch; an
    /// `int64` Google serialises as a JSON string
    #[serde(default)]
    pub timestamp_millis: String,
}

/// `appIntegrity`: whether Google Play recognises the binary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppIntegrity {
    /// `PLAY_RECOGNIZED`, `UNRECOGNIZED_VERSION` or `UNEVALUATED`
    #[serde(default)]
    pub app_recognition_verdict: String,
    /// The package Google Play evaluated (absent when `UNEVALUATED`)
    #[serde(default)]
    pub package_name: Option<String>,
}

/// `deviceIntegrity`: the labels the device earned.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceIntegrity {
    /// Empty when the device shows signs of attack or compromise
    #[serde(default)]
    pub device_recognition_verdict: Vec<String>,
}

/// A verdict that passed every check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayIntegrityVerdict {
    /// Whether the device also earned `MEETS_STRONG_INTEGRITY`
    pub strong_integrity: bool,
}

/// Why a Play Integrity verdict was refused. Each names the check that
/// failed; none carries the token or the verdict itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayIntegrityError {
    /// A field the checks read is not in the shape Google documents
    Malformed(&'static str),
    /// The token was requested by another package
    RequestPackageMismatch,
    /// The token was requested for another authorization code
    RequestHashMismatch,
    /// The token was minted longer ago than [`MAX_TOKEN_AGE`]
    Stale,
    /// The token's timestamp is further ahead than [`MAX_CLOCK_SKEW`]
    FromTheFuture,
    /// Google Play does not recognise the binary
    AppNotRecognized,
    /// Google Play evaluated another package
    AppPackageMismatch,
    /// The device is not a genuine, certified Android device
    DeviceIntegrityMissing,
}

impl fmt::Display for PlayIntegrityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(what) => write!(f, "malformed Play Integrity verdict: {what}"),
            Self::RequestPackageMismatch => {
                f.write_str("integrity token was requested by another app")
            }
            Self::RequestHashMismatch => {
                f.write_str("integrity token was requested for another sign-in")
            }
            Self::Stale => f.write_str("integrity token is too old"),
            Self::FromTheFuture => f.write_str("integrity token is dated in the future"),
            Self::AppNotRecognized => f.write_str("Google Play does not recognize the app"),
            Self::AppPackageMismatch => f.write_str("integrity verdict is for another app"),
            Self::DeviceIntegrityMissing => {
                f.write_str("device does not meet Play device integrity")
            }
        }
    }
}

impl Error for PlayIntegrityError {}

/// The request hash the app binds its integrity token to: the SHA-256 of the
/// authorization code's UTF-8 bytes, base64url without padding (43
/// characters).
#[must_use]
pub fn request_hash(code: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(code.as_bytes()))
}

/// Check a decoded verdict against the request it must have been minted for.
///
/// `expected_request_hash` is [`request_hash`] of the code being redeemed;
/// `package_name` is the app's; `now` is this server's clock.
///
/// # Errors
/// The [`PlayIntegrityError`] naming the first check that failed.
pub fn verify_verdict(
    payload: &TokenPayload,
    expected_request_hash: &str,
    package_name: &str,
    now: DateTime<Utc>,
) -> Result<PlayIntegrityVerdict, PlayIntegrityError> {
    let request = &payload.request_details;
    if request.request_package_name != package_name {
        return Err(PlayIntegrityError::RequestPackageMismatch);
    }
    if request.request_hash.as_deref() != Some(expected_request_hash) {
        return Err(PlayIntegrityError::RequestHashMismatch);
    }
    let issued_at = request
        .timestamp_millis
        .parse::<i64>()
        .ok()
        .and_then(DateTime::from_timestamp_millis)
        .ok_or(PlayIntegrityError::Malformed("timestampMillis"))?;
    if issued_at > now + MAX_CLOCK_SKEW {
        return Err(PlayIntegrityError::FromTheFuture);
    }
    if issued_at < now - MAX_TOKEN_AGE {
        return Err(PlayIntegrityError::Stale);
    }

    let app = &payload.app_integrity;
    if app.app_recognition_verdict != PLAY_RECOGNIZED {
        return Err(PlayIntegrityError::AppNotRecognized);
    }
    if app.package_name.as_deref() != Some(package_name) {
        return Err(PlayIntegrityError::AppPackageMismatch);
    }

    let labels = &payload.device_integrity.device_recognition_verdict;
    if !labels.iter().any(|label| label == MEETS_DEVICE_INTEGRITY) {
        return Err(PlayIntegrityError::DeviceIntegrityMissing);
    }
    Ok(PlayIntegrityVerdict {
        strong_integrity: labels.iter().any(|label| label == MEETS_STRONG_INTEGRITY),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const PACKAGE: &str = "ai.dravr.app";
    const CODE: &str = "an-authorization-code";

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp_millis(1_675_655_009_345).unwrap()
    }

    /// Google's documented example verdict, for this app and [`CODE`],
    /// minted at [`now`].
    fn documented() -> Value {
        json!({
            "requestDetails": {
                "requestPackageName": PACKAGE,
                "requestHash": request_hash(CODE),
                "timestampMillis": "1675655009345"
            },
            "accountDetails": {
                "appLicensingVerdict": "LICENSED"
            },
            "appIntegrity": {
                "appRecognitionVerdict": "PLAY_RECOGNIZED",
                "packageName": PACKAGE,
                "certificateSha256Digest": ["6a6a1474b5cbbb2b1aa57e0bc3"],
                "versionCode": "42"
            },
            "deviceIntegrity": {
                "deviceRecognitionVerdict": ["MEETS_DEVICE_INTEGRITY"]
            },
            "environmentDetails": {
                "appAccessRiskVerdict": {
                    "appsDetected": ["KNOWN_INSTALLED", "UNKNOWN_INSTALLED", "UNKNOWN_CAPTURING"]
                },
                "playProtectVerdict": "NO_ISSUES"
            }
        })
    }

    fn verify(payload: &Value) -> Result<PlayIntegrityVerdict, PlayIntegrityError> {
        verify_at(payload, now())
    }

    fn verify_at(
        payload: &Value,
        now: DateTime<Utc>,
    ) -> Result<PlayIntegrityVerdict, PlayIntegrityError> {
        let payload: TokenPayload = serde_json::from_value(payload.clone()).unwrap();
        verify_verdict(&payload, &request_hash(CODE), PACKAGE, now)
    }

    #[test]
    fn request_hash_is_unpadded_base64url_sha256_of_the_code() {
        // SHA-256("abc") = ba7816bf…15ad, as base64url without padding
        assert_eq!(
            request_hash("abc"),
            "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0"
        );
        assert_eq!(request_hash(CODE).len(), 43);
        assert_ne!(request_hash(CODE), request_hash("another-code"));
    }

    /// Google's whole documented verdict, fields the checks ignore included,
    /// parses and verifies.
    #[test]
    fn the_documented_verdict_parses_and_verifies() {
        assert_eq!(
            verify(&documented()),
            Ok(PlayIntegrityVerdict {
                strong_integrity: false
            })
        );
    }

    #[test]
    fn strong_integrity_is_recorded() {
        let mut payload = documented();
        payload["deviceIntegrity"]["deviceRecognitionVerdict"] = json!([
            "MEETS_BASIC_INTEGRITY",
            "MEETS_DEVICE_INTEGRITY",
            "MEETS_STRONG_INTEGRITY"
        ]);
        assert_eq!(
            verify(&payload),
            Ok(PlayIntegrityVerdict {
                strong_integrity: true
            })
        );
    }

    #[test]
    fn a_token_another_package_requested_is_refused() {
        let mut payload = documented();
        payload["requestDetails"]["requestPackageName"] = json!("com.example.other");
        assert_eq!(
            verify(&payload),
            Err(PlayIntegrityError::RequestPackageMismatch)
        );
    }

    #[test]
    fn a_token_over_another_code_is_refused() {
        let mut payload = documented();
        payload["requestDetails"]["requestHash"] = json!(request_hash("another-code"));
        assert_eq!(
            verify(&payload),
            Err(PlayIntegrityError::RequestHashMismatch)
        );
    }

    #[test]
    fn a_classic_token_without_a_request_hash_is_refused() {
        let mut payload = documented();
        payload["requestDetails"]
            .as_object_mut()
            .unwrap()
            .remove("requestHash");
        payload["requestDetails"]["nonce"] = json!("R2Rra24fVm5xa2Mg");
        assert_eq!(
            verify(&payload),
            Err(PlayIntegrityError::RequestHashMismatch)
        );
    }

    #[test]
    fn a_token_older_than_ten_minutes_is_refused() {
        let just_in_time = now() + MAX_TOKEN_AGE;
        assert!(verify_at(&documented(), just_in_time).is_ok());
        assert_eq!(
            verify_at(&documented(), just_in_time + Duration::milliseconds(1)),
            Err(PlayIntegrityError::Stale)
        );
    }

    #[test]
    fn a_token_from_more_than_a_minute_ahead_is_refused() {
        let skewed = now() - MAX_CLOCK_SKEW;
        assert!(verify_at(&documented(), skewed).is_ok());
        assert_eq!(
            verify_at(&documented(), skewed - Duration::milliseconds(1)),
            Err(PlayIntegrityError::FromTheFuture)
        );
    }

    #[test]
    fn an_unparseable_timestamp_is_malformed() {
        for timestamp in [json!("yesterday"), json!("")] {
            let mut payload = documented();
            payload["requestDetails"]["timestampMillis"] = timestamp;
            assert_eq!(
                verify(&payload),
                Err(PlayIntegrityError::Malformed("timestampMillis"))
            );
        }
    }

    #[test]
    fn an_app_play_does_not_recognize_is_refused() {
        for verdict in ["UNRECOGNIZED_VERSION", "UNEVALUATED"] {
            let mut payload = documented();
            payload["appIntegrity"]["appRecognitionVerdict"] = json!(verdict);
            assert_eq!(verify(&payload), Err(PlayIntegrityError::AppNotRecognized));
        }
        let mut unevaluated = documented();
        unevaluated["appIntegrity"] = json!({ "appRecognitionVerdict": "UNEVALUATED" });
        assert_eq!(
            verify(&unevaluated),
            Err(PlayIntegrityError::AppNotRecognized)
        );
    }

    #[test]
    fn a_verdict_for_another_package_is_refused() {
        let mut payload = documented();
        payload["appIntegrity"]["packageName"] = json!("com.example.other");
        assert_eq!(
            verify(&payload),
            Err(PlayIntegrityError::AppPackageMismatch)
        );

        let mut absent = documented();
        absent["appIntegrity"]
            .as_object_mut()
            .unwrap()
            .remove("packageName");
        assert_eq!(verify(&absent), Err(PlayIntegrityError::AppPackageMismatch));
    }

    #[test]
    fn a_device_without_device_integrity_is_refused() {
        for labels in [
            json!([]),
            json!(["MEETS_BASIC_INTEGRITY"]),
            json!(["MEETS_VIRTUAL_INTEGRITY"]),
        ] {
            let mut payload = documented();
            payload["deviceIntegrity"]["deviceRecognitionVerdict"] = labels;
            assert_eq!(
                verify(&payload),
                Err(PlayIntegrityError::DeviceIntegrityMissing)
            );
        }
        let mut absent = documented();
        absent["deviceIntegrity"] = json!({});
        assert_eq!(
            verify(&absent),
            Err(PlayIntegrityError::DeviceIntegrityMissing)
        );
    }

    #[test]
    fn a_refusal_never_names_the_evidence() {
        let reason = PlayIntegrityError::RequestHashMismatch.to_string();
        assert!(!reason.contains(&request_hash(CODE)));
    }
}
