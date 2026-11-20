// ABOUTME: GCP Cloud KMS Ed25519 signer — signs barrage's record_log checkpoints with a key that never leaves KMS
// ABOUTME: asymmetricSign on the raw message (PureEdDSA), the public key read once, every signature checked before use
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! GCP Cloud KMS-backed Ed25519 signer (design Q11, carnet#1542).
//!
//! Selected at boot when
//! `PIERRE_CHECKPOINT_SIGNING_KEY` names a crypto-key **version** of an
//! `EC_SIGN_ED25519` key (`projects/{p}/locations/{l}/keyRings/{r}/cryptoKeys/{k}/cryptoKeyVersions/{v}`,
//! terraform's `checkpoint_signing_key_version` output). The private key never
//! leaves KMS, and KMS's Data Access logs record every signature.
//!
//! Ed25519 in KMS signs the message itself, not a digest, so each request
//! carries `data`. The public key is read once ([`GcpKmsEd25519Signer::connect`])
//! and every signature KMS returns is verified against it with ring before it
//! is handed back: a corrupted or substituted signature never reaches a
//! checkpoint.

use std::env;
use std::fmt;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as Base64Standard;
use base64::Engine as _;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::gcp_token::{MetadataTokenProvider, TokenProvider};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::Deserialize;

/// Cloud KMS REST base URL.
pub const KMS_BASE_URL: &str = "https://cloudkms.googleapis.com/v1";

/// The environment variable naming the signing key version.
pub const CHECKPOINT_SIGNING_KEY_ENV: &str = "PIERRE_CHECKPOINT_SIGNING_KEY";

/// Ceiling on one KMS call.
const KMS_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Ceiling on opening the connection to KMS.
const KMS_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The DER prefix of an Ed25519 `SubjectPublicKeyInfo` (RFC 8410): the 32-byte
/// key follows it.
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// An Ed25519 signer whose key lives in Cloud KMS.
pub struct GcpKmsEd25519Signer {
    key_version: String,
    base_url: String,
    http: reqwest::Client,
    token_provider: Box<dyn TokenProvider>,
    public_key: [u8; 32],
}

impl fmt::Debug for GcpKmsEd25519Signer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcpKmsEd25519Signer")
            .field("key_version", &self.key_version)
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct PublicKeyResponse {
    pem: String,
    algorithm: String,
}

#[derive(Deserialize)]
struct SignResponse {
    signature: String,
}

impl GcpKmsEd25519Signer {
    /// The signer of `key_version` from the environment's
    /// [`CHECKPOINT_SIGNING_KEY_ENV`], on the production KMS endpoint with the
    /// metadata server's tokens; `None` when the variable is unset.
    ///
    /// # Errors
    ///
    /// As [`Self::connect`].
    pub async fn from_env() -> AppResult<Option<Self>> {
        let Ok(key_version) = env::var(CHECKPOINT_SIGNING_KEY_ENV) else {
            return Ok(None);
        };
        let key_version = key_version.trim().to_owned();
        if key_version.is_empty() {
            return Ok(None);
        }
        Self::connect(
            key_version,
            KMS_BASE_URL.to_owned(),
            Box::new(MetadataTokenProvider::default()),
        )
        .await
        .map(Some)
    }

    /// The signer of `key_version` at `base_url`, its public key read now.
    ///
    /// # Errors
    ///
    /// When the HTTP client cannot be built, the key cannot be read, or it is
    /// not an Ed25519 key.
    pub async fn connect(
        key_version: String,
        base_url: String,
        token_provider: Box<dyn TokenProvider>,
    ) -> AppResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(KMS_REQUEST_TIMEOUT)
            .connect_timeout(KMS_CONNECT_TIMEOUT)
            .build()
            .map_err(|e| AppError::internal(format!("Failed to build the KMS HTTP client: {e}")))?;
        let token = token_provider.access_token().await?;
        let response = http
            .get(format!("{base_url}/{key_version}/publicKey"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| AppError::internal(format!("KMS getPublicKey request failed: {e}")))?;
        if !response.status().is_success() {
            return Err(AppError::internal(format!(
                "KMS getPublicKey returned status {}",
                response.status()
            )));
        }
        let parsed: PublicKeyResponse = response
            .json()
            .await
            .map_err(|e| AppError::internal(format!("Invalid KMS getPublicKey response: {e}")))?;
        if parsed.algorithm != "EC_SIGN_ED25519" {
            return Err(AppError::config(format!(
                "{CHECKPOINT_SIGNING_KEY_ENV} names a {} key; checkpoints need EC_SIGN_ED25519",
                parsed.algorithm
            )));
        }
        let public_key = ed25519_from_pem(&parsed.pem)?;
        Ok(Self {
            key_version,
            base_url,
            http,
            token_provider,
            public_key,
        })
    }

    /// The Ed25519 public key.
    #[must_use]
    pub const fn public_key(&self) -> [u8; 32] {
        self.public_key
    }

    /// The Ed25519 signature of `message`, checked against the public key.
    ///
    /// # Errors
    ///
    /// When KMS refuses or cannot be reached, or returns a signature that
    /// does not verify.
    pub async fn sign(&self, message: &[u8]) -> AppResult<[u8; 64]> {
        let token = self.token_provider.access_token().await?;
        let body = serde_json::json!({ "data": Base64Standard.encode(message) });
        let response = self
            .http
            .post(format!(
                "{}/{}:asymmetricSign",
                self.base_url, self.key_version
            ))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::internal(format!("KMS asymmetricSign request failed: {e}")))?;
        if !response.status().is_success() {
            return Err(AppError::internal(format!(
                "KMS asymmetricSign returned status {}",
                response.status()
            )));
        }
        let parsed: SignResponse = response
            .json()
            .await
            .map_err(|e| AppError::internal(format!("Invalid KMS asymmetricSign response: {e}")))?;
        let signature: [u8; 64] = Base64Standard
            .decode(parsed.signature)
            .map_err(|e| AppError::internal(format!("Invalid KMS signature base64: {e}")))?
            .try_into()
            .map_err(|_| AppError::internal("A KMS Ed25519 signature is not 64 bytes"))?;
        UnparsedPublicKey::new(&ED25519, self.public_key)
            .verify(message, &signature)
            .map_err(|_| AppError::internal("A KMS signature does not verify under its key"))?;
        Ok(signature)
    }
}

/// The 32-byte key of an Ed25519 public key in PEM `SubjectPublicKeyInfo`.
///
/// # Errors
///
/// When the PEM is not one Ed25519 `SubjectPublicKeyInfo`.
pub fn ed25519_from_pem(pem: &str) -> AppResult<[u8; 32]> {
    let body: String = pem
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("-----") && !line.is_empty())
        .collect();
    let der = Base64Standard
        .decode(body)
        .map_err(|e| AppError::internal(format!("Invalid public key PEM: {e}")))?;
    der.strip_prefix(&ED25519_SPKI_PREFIX).map_or_else(
        || {
            Err(AppError::internal(
                "The public key is not an Ed25519 SubjectPublicKeyInfo",
            ))
        },
        |key| {
            key.try_into()
                .map_err(|_| AppError::internal("An Ed25519 public key is not 32 bytes"))
        },
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::Uri;
    use axum::{Json, Router};
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use serde_json::{json, Value};
    use tokio::net::TcpListener;

    use super::*;

    struct FixedToken;

    #[async_trait]
    impl TokenProvider for FixedToken {
        async fn access_token(&self) -> AppResult<String> {
            Ok("test-token".to_owned())
        }
    }

    struct Fake {
        pair: Ed25519KeyPair,
        /// Signs with this key instead, when set: a substituted signature.
        impostor: Option<Ed25519KeyPair>,
        algorithm: &'static str,
    }

    fn pair() -> Ed25519KeyPair {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    fn pem_of(key: &[u8]) -> String {
        let mut der = ED25519_SPKI_PREFIX.to_vec();
        der.extend_from_slice(key);
        format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            Base64Standard.encode(der)
        )
    }

    /// The fake's one handler: KMS's two methods, told apart by path. A
    /// fallback rather than routes, so the scan of the backend's route
    /// surface never reads this test fake as a backend route.
    async fn kms(State(fake): State<Arc<Fake>>, uri: Uri, body: Bytes) -> Json<Value> {
        let path = uri.path();
        if path == format!("/{VERSION}/publicKey") {
            return Json(json!({
                "pem": pem_of(fake.pair.public_key().as_ref()),
                "algorithm": fake.algorithm,
            }));
        }
        assert_eq!(
            path,
            format!("/{VERSION}:asymmetricSign"),
            "bug: the signer called another method"
        );
        let body: Value = serde_json::from_slice(&body).unwrap_or_default();
        let data = Base64Standard
            .decode(body["data"].as_str().unwrap_or_default())
            .unwrap_or_default();
        let signer = fake.impostor.as_ref().unwrap_or(&fake.pair);
        Json(json!({ "signature": Base64Standard.encode(signer.sign(&data).as_ref()) }))
    }

    /// A fake of the two KMS methods the signer calls, on a local port.
    async fn serve(fake: Fake) -> String {
        let app = Router::new().fallback(kms).with_state(Arc::new(fake));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        format!("http://{address}")
    }

    const VERSION: &str = "projects/p/locations/l/keyRings/r/cryptoKeys/k/cryptoKeyVersions/1";

    #[tokio::test]
    async fn it_signs_through_kms_and_verifies_what_comes_back() {
        let kms = pair();
        let expected: [u8; 32] = kms.public_key().as_ref().try_into().unwrap();
        let base = serve(Fake {
            pair: kms,
            impostor: None,
            algorithm: "EC_SIGN_ED25519",
        })
        .await;
        let signer = GcpKmsEd25519Signer::connect(VERSION.to_owned(), base, Box::new(FixedToken))
            .await
            .unwrap();
        assert_eq!(
            signer.public_key(),
            expected,
            "bug: the public key read is not KMS's"
        );
        let signature = signer
            .sign(b"dravr.ai/barrage-log\n3\nroot\n")
            .await
            .unwrap();
        assert!(
            UnparsedPublicKey::new(&ED25519, expected)
                .verify(b"dravr.ai/barrage-log\n3\nroot\n", &signature)
                .is_ok(),
            "bug: KMS's signature does not verify under its key"
        );
    }

    #[tokio::test]
    async fn a_signature_by_another_key_is_refused() {
        let base = serve(Fake {
            pair: pair(),
            impostor: Some(pair()),
            algorithm: "EC_SIGN_ED25519",
        })
        .await;
        let signer = GcpKmsEd25519Signer::connect(VERSION.to_owned(), base, Box::new(FixedToken))
            .await
            .unwrap();
        assert!(
            signer.sign(b"note").await.is_err(),
            "bug: a signature that does not verify was handed back"
        );
    }

    #[tokio::test]
    async fn a_key_of_another_algorithm_is_refused() {
        let base = serve(Fake {
            pair: pair(),
            impostor: None,
            algorithm: "EC_SIGN_P256_SHA256",
        })
        .await;
        assert!(
            GcpKmsEd25519Signer::connect(VERSION.to_owned(), base, Box::new(FixedToken))
                .await
                .is_err(),
            "bug: a non-Ed25519 key was accepted"
        );
    }

    #[test]
    fn a_pem_that_is_not_ed25519_is_refused() {
        assert_eq!(
            ed25519_from_pem(&pem_of(&[7; 32])).ok(),
            Some([7; 32]),
            "bug: an Ed25519 SubjectPublicKeyInfo is not read back"
        );
        assert!(
            ed25519_from_pem("-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----").is_err(),
            "bug: a PEM that is not Ed25519 was accepted"
        );
    }
}
