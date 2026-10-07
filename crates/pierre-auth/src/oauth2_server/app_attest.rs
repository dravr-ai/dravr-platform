// ABOUTME: Apple App Attest verification: a key's attestation against Apple's root, and each assertion it signs
// ABOUTME: Pure checks over the bytes the iOS app sends; storing the key and its counter is the caller's
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Apple App Attest, server side (carnet#810).
//!
//! The iOS app generates a key in the Secure Enclave and asks Apple to attest
//! it; Apple answers with a certificate for the key, chained to the App
//! Attestation root, that names the app (Team ID and bundle) it was issued
//! to. From then on the app signs assertions with that key. A script cannot
//! produce either: the attestation needs Apple to vouch for a genuine device
//! running Dravr's signed binary, and an assertion needs the key that never
//! leaves that device.
//!
//! Both checks follow Apple's "Validating apps that connect to your server":
//!
//! - [`verify_attestation`] checks the certificate chain against the pinned
//!   root, the nonce the leaf certificate carries, that the key id is the
//!   SHA-256 of the certified public key, the app id hash, a zero counter, and
//!   the environment (AAGUID).
//! - [`verify_assertion`] checks the signature with the stored public key over
//!   the same nonce construction, the app id hash, and that the counter moved
//!   forward.
//!
//! `client_data` is whatever the app and the server agreed the evidence
//! binds to. The app hashes it with SHA-256 before handing it to the Secure
//! Enclave (`@expo/app-integrity` passes the UTF-8 bytes of its challenge
//! string), so the server hashes the same bytes.

use std::error::Error;
use std::fmt;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ciborium::Value;
use der::asn1::ObjectIdentifier;
use der::Decode as _;
use pierre_core::models::AppAttestEnvironment;
use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1};
use rustls_pki_types::{CertificateDer, SignatureVerificationAlgorithm, UnixTime};
use sha2::{Digest, Sha256};
use webpki::ring::{ECDSA_P256_SHA256, ECDSA_P256_SHA384, ECDSA_P384_SHA256, ECDSA_P384_SHA384};
use webpki::{EndEntityCert, ExtendedKeyUsageValidator, KeyPurposeIdIter};
use x509_cert::Certificate;

/// Apple App Attestation Root CA, from
/// <https://www.apple.com/certificateauthority/Apple_App_Attestation_Root_CA.pem>.
/// SHA-256 fingerprint `1C:B9:82:3B:A2:8B:A6:AD:2D:33:A0:06:94:1D:E2:AE:4F:51:3E:F1:D4:E8:31:B9:F7:E0:FA:7B:62:42:C9:32`,
/// valid until 2045-03-15.
const APPLE_ROOT_CA_PEM: &str = include_str!("apple_app_attestation_root_ca.pem");

/// The attestation statement format App Attest uses.
const ATTESTATION_FORMAT: &str = "apple-appattest";

/// The certificate extension carrying the attestation nonce.
const NONCE_EXTENSION_OID: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113635.100.8.2");

/// The DER the nonce extension holds ahead of its 32 bytes:
/// `SEQUENCE { [1] EXPLICIT { OCTET STRING (32) } }`. DER has one encoding
/// for that shape, so a byte comparison is the whole parse.
const NONCE_EXTENSION_PREFIX: [u8; 6] = [0x30, 0x24, 0xa1, 0x22, 0x04, 0x20];

/// AAGUID of a key attested by Apple's production App Attest service.
const AAGUID_PRODUCTION: &[u8; 16] = b"appattest\0\0\0\0\0\0\0";

/// AAGUID of a key attested by the development service, which a build signed
/// with a development profile uses.
const AAGUID_DEVELOPMENT: &[u8; 16] = b"appattestdevelop";

/// Offsets into authenticator data: `rpIdHash(32) flags(1) signCount(4)`,
/// then, in an attestation only, `aaguid(16) credentialIdLength(2)
/// credentialId(n) credentialPublicKey(COSE)`.
const RP_ID_HASH_END: usize = 32;
const COUNTER_START: usize = 33;
const COUNTER_END: usize = 37;
const AAGUID_END: usize = 53;
const CREDENTIAL_ID_LENGTH_END: usize = 55;

/// Signature algorithms on Apple's chain: the root and intermediate are
/// P-384 keys, the credential certificate a P-256 key.
static CHAIN_ALGORITHMS: &[&dyn SignatureVerificationAlgorithm] = &[
    ECDSA_P256_SHA256,
    ECDSA_P256_SHA384,
    ECDSA_P384_SHA256,
    ECDSA_P384_SHA384,
];

/// A key whose attestation verified: what the server keeps to check the
/// assertions it signs later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestedKey {
    /// The key id the app names the key by (standard base64 of the SHA-256
    /// of the public key)
    pub key_id: String,
    /// The certified public key, an uncompressed P-256 point (65 bytes)
    pub public_key: Vec<u8>,
    /// The service that attested it
    pub environment: AppAttestEnvironment,
}

/// Why App Attest evidence was refused. Each names the check that failed;
/// none carries the evidence itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAttestError {
    /// Not base64, not CBOR, or missing a field the format requires
    Malformed(&'static str),
    /// The certificate chain does not lead to Apple's App Attestation root
    UntrustedChain,
    /// The leaf certificate's nonce is not the one this request computes
    NonceMismatch,
    /// The key id is not the SHA-256 of the certified public key
    KeyIdMismatch,
    /// The evidence was issued to another app (Team ID and bundle)
    AppIdMismatch,
    /// An attestation whose counter is not zero
    CounterNotZero,
    /// An assertion whose counter did not move past the stored one
    CounterReplayed,
    /// An AAGUID that is neither Apple's production nor development service
    UnknownEnvironment,
    /// The assertion's signature does not verify with the stored key
    BadSignature,
}

impl fmt::Display for AppAttestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(what) => write!(f, "malformed App Attest evidence: {what}"),
            Self::UntrustedChain => f.write_str("certificate chain is not Apple's App Attest"),
            Self::NonceMismatch => f.write_str("attestation nonce does not match the request"),
            Self::KeyIdMismatch => f.write_str("key id does not match the certified key"),
            Self::AppIdMismatch => f.write_str("evidence was issued to another app"),
            Self::CounterNotZero => f.write_str("attestation counter is not zero"),
            Self::CounterReplayed => f.write_str("assertion counter did not advance"),
            Self::UnknownEnvironment => f.write_str("unknown App Attest environment"),
            Self::BadSignature => f.write_str("assertion signature does not verify"),
        }
    }
}

impl Error for AppAttestError {}

/// Verify the attestation of a freshly generated key.
///
/// `attestation` and `key_id` are standard base64, as the app receives them
/// from `DCAppAttestService`. `app_id` is `<Team ID>.<bundle id>`. `now` is
/// when the certificate chain must be valid.
///
/// # Errors
/// The [`AppAttestError`] naming the first check that failed.
pub fn verify_attestation(
    attestation: &str,
    key_id: &str,
    client_data: &[u8],
    app_id: &str,
    now: UnixTime,
) -> Result<AttestedKey, AppAttestError> {
    let object = decode_cbor_map(attestation)?;
    if text_field(&object, "fmt")? != ATTESTATION_FORMAT {
        return Err(AppAttestError::Malformed("fmt is not apple-appattest"));
    }
    let auth_data = bytes_field(&object, "authData")?;
    let statement = map_field(&object, "attStmt")?;
    let chain = match field(statement, "x5c")? {
        Value::Array(certs) => certs
            .iter()
            .map(|cert| match cert {
                Value::Bytes(der) => Ok(der.as_slice()),
                _ => Err(AppAttestError::Malformed("x5c holds a non-bytes entry")),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(AppAttestError::Malformed("x5c is not an array")),
    };
    let (leaf_der, intermediates) = chain
        .split_first()
        .ok_or(AppAttestError::Malformed("x5c is empty"))?;

    // 1. The chain leads to Apple's root.
    verify_chain(leaf_der, intermediates, now)?;

    // 2–4. The leaf carries SHA-256(authData ‖ SHA-256(clientData)).
    let leaf = Certificate::from_der(leaf_der)
        .map_err(|_| AppAttestError::Malformed("credential certificate is not DER"))?;
    if certificate_nonce(&leaf)? != nonce(auth_data, client_data) {
        return Err(AppAttestError::NonceMismatch);
    }

    // 5. The key id is the SHA-256 of the certified key.
    let public_key = leaf
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .ok_or(AppAttestError::Malformed("public key has unused bits"))?
        .to_vec();
    let key_id_bytes = STANDARD
        .decode(key_id)
        .map_err(|_| AppAttestError::Malformed("key id is not base64"))?;
    if Sha256::digest(&public_key).as_slice() != key_id_bytes.as_slice() {
        return Err(AppAttestError::KeyIdMismatch);
    }

    // 6–9. Authenticator data: our app, a fresh counter, a known service,
    // and the credential is the key id.
    check_app_id(auth_data, app_id)?;
    if counter(auth_data)? != 0 {
        return Err(AppAttestError::CounterNotZero);
    }
    let aaguid = auth_data
        .get(COUNTER_END..AAGUID_END)
        .ok_or(AppAttestError::Malformed("authData too short for aaguid"))?;
    let environment = if aaguid == AAGUID_PRODUCTION {
        AppAttestEnvironment::Production
    } else if aaguid == AAGUID_DEVELOPMENT {
        AppAttestEnvironment::Development
    } else {
        return Err(AppAttestError::UnknownEnvironment);
    };
    let credential_id = credential_id(auth_data)?;
    if credential_id != key_id_bytes.as_slice() {
        return Err(AppAttestError::KeyIdMismatch);
    }

    Ok(AttestedKey {
        key_id: key_id.to_owned(),
        public_key,
        environment,
    })
}

/// Verify an assertion signed by a key whose attestation was verified
/// earlier, and return its counter for the caller to store.
///
/// `public_key` is [`AttestedKey::public_key`]; `stored_counter` is the
/// counter of the last assertion accepted from this key (zero after the
/// attestation).
///
/// # Errors
/// The [`AppAttestError`] naming the first check that failed.
pub fn verify_assertion(
    assertion: &str,
    client_data: &[u8],
    app_id: &str,
    public_key: &[u8],
    stored_counter: u32,
) -> Result<u32, AppAttestError> {
    let object = decode_cbor_map(assertion)?;
    let signature = bytes_field(&object, "signature")?;
    let authenticator_data = bytes_field(&object, "authenticatorData")?;

    let nonce = nonce(authenticator_data, client_data);
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, public_key)
        .verify(&nonce, signature)
        .map_err(|_| AppAttestError::BadSignature)?;

    check_app_id(authenticator_data, app_id)?;
    let counter = counter(authenticator_data)?;
    if counter <= stored_counter {
        return Err(AppAttestError::CounterReplayed);
    }
    Ok(counter)
}

/// Verify `leaf` chains through `intermediates` to Apple's App Attestation
/// root, valid at `now`.
fn verify_chain(leaf: &[u8], intermediates: &[&[u8]], now: UnixTime) -> Result<(), AppAttestError> {
    let root_der = pem_body(APPLE_ROOT_CA_PEM)?;
    let root = CertificateDer::from(root_der.as_slice());
    let anchor =
        webpki::anchor_from_trusted_cert(&root).map_err(|_| AppAttestError::UntrustedChain)?;
    let leaf = CertificateDer::from(leaf);
    let end_entity = EndEntityCert::try_from(&leaf).map_err(|_| AppAttestError::UntrustedChain)?;
    let intermediates: Vec<CertificateDer<'_>> = intermediates
        .iter()
        .map(|der| CertificateDer::from(*der))
        .collect();
    end_entity
        .verify_for_usage(
            CHAIN_ALGORITHMS,
            &[anchor],
            &intermediates,
            now,
            AnyExtendedKeyUsage,
            None,
            None,
        )
        .map(|_| ())
        .map_err(|_| AppAttestError::UntrustedChain)
}

/// Apple's credential certificates carry no extended key usage, and its
/// validation steps check none: the chain to the App Attestation root is
/// what scopes the certificate.
struct AnyExtendedKeyUsage;

impl ExtendedKeyUsageValidator for AnyExtendedKeyUsage {
    fn validate(&self, _iter: KeyPurposeIdIter<'_, '_>) -> Result<(), webpki::Error> {
        Ok(())
    }
}

/// `SHA-256(data ‖ SHA-256(client_data))`, the value an attestation's leaf
/// certificate carries and an assertion signs.
fn nonce(data: &[u8], client_data: &[u8]) -> [u8; 32] {
    let client_data_hash = Sha256::digest(client_data);
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.update(client_data_hash);
    hasher.finalize().into()
}

/// The 32-byte nonce in the credential certificate's App Attest extension.
fn certificate_nonce(leaf: &Certificate) -> Result<[u8; 32], AppAttestError> {
    let extension = leaf
        .tbs_certificate
        .extensions
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|extension| extension.extn_id == NONCE_EXTENSION_OID)
        .ok_or(AppAttestError::Malformed("no nonce extension"))?;
    extension
        .extn_value
        .as_bytes()
        .strip_prefix(NONCE_EXTENSION_PREFIX.as_slice())
        .and_then(|nonce| <[u8; 32]>::try_from(nonce).ok())
        .ok_or(AppAttestError::Malformed(
            "nonce extension is not one 32-byte octet string",
        ))
}

/// Refuse authenticator data whose `rpIdHash` is not `SHA-256(app_id)`.
fn check_app_id(auth_data: &[u8], app_id: &str) -> Result<(), AppAttestError> {
    let rp_id_hash = auth_data
        .get(..RP_ID_HASH_END)
        .ok_or(AppAttestError::Malformed("authData too short for rpIdHash"))?;
    if rp_id_hash == Sha256::digest(app_id.as_bytes()).as_slice() {
        Ok(())
    } else {
        Err(AppAttestError::AppIdMismatch)
    }
}

/// The big-endian sign counter in authenticator data.
fn counter(auth_data: &[u8]) -> Result<u32, AppAttestError> {
    auth_data
        .get(COUNTER_START..COUNTER_END)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
        .map(u32::from_be_bytes)
        .ok_or(AppAttestError::Malformed("authData too short for counter"))
}

/// The credential id an attestation's authenticator data names.
fn credential_id(auth_data: &[u8]) -> Result<&[u8], AppAttestError> {
    let length = auth_data
        .get(AAGUID_END..CREDENTIAL_ID_LENGTH_END)
        .and_then(|bytes| <[u8; 2]>::try_from(bytes).ok())
        .map(u16::from_be_bytes)
        .ok_or(AppAttestError::Malformed(
            "authData too short for credential id",
        ))?;
    auth_data
        .get(CREDENTIAL_ID_LENGTH_END..CREDENTIAL_ID_LENGTH_END + usize::from(length))
        .ok_or(AppAttestError::Malformed(
            "authData too short for credential id",
        ))
}

/// The DER inside a single-certificate PEM.
fn pem_body(pem: &str) -> Result<Vec<u8>, AppAttestError> {
    let body: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    STANDARD
        .decode(body)
        .map_err(|_| AppAttestError::UntrustedChain)
}

/// Decode standard base64, then a CBOR map.
fn decode_cbor_map(encoded: &str) -> Result<Vec<(Value, Value)>, AppAttestError> {
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| AppAttestError::Malformed("not base64"))?;
    match ciborium::from_reader::<Value, _>(bytes.as_slice()) {
        Ok(Value::Map(entries)) => Ok(entries),
        Ok(_) => Err(AppAttestError::Malformed("not a CBOR map")),
        Err(_) => Err(AppAttestError::Malformed("not CBOR")),
    }
}

/// The value under a text key of a CBOR map.
fn field<'a>(map: &'a [(Value, Value)], name: &'static str) -> Result<&'a Value, AppAttestError> {
    map.iter()
        .find(|(key, _)| key.as_text() == Some(name))
        .map(|(_, value)| value)
        .ok_or(AppAttestError::Malformed(name))
}

fn text_field<'a>(
    map: &'a [(Value, Value)],
    name: &'static str,
) -> Result<&'a str, AppAttestError> {
    field(map, name)?
        .as_text()
        .ok_or(AppAttestError::Malformed(name))
}

fn bytes_field<'a>(
    map: &'a [(Value, Value)],
    name: &'static str,
) -> Result<&'a [u8], AppAttestError> {
    field(map, name)?
        .as_bytes()
        .map(Vec::as_slice)
        .ok_or(AppAttestError::Malformed(name))
}

fn map_field<'a>(
    map: &'a [(Value, Value)],
    name: &'static str,
) -> Result<&'a [(Value, Value)], AppAttestError> {
    field(map, name)?
        .as_map()
        .map(Vec::as_slice)
        .ok_or(AppAttestError::Malformed(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair as _, ECDSA_P256_SHA256_ASN1_SIGNING};

    /// A real attestation from an iPhone, published with the MIT-licensed
    /// `appattest-rs` crate's examples: app `762U5G7236.network.gandalf.connect`,
    /// production service, credential certificate valid 2024-06-29 to 2025-01-24.
    const ATTESTATION: &str = include_str!("testdata/app_attest_attestation.b64");
    const ATTESTATION_APP_ID: &str = "762U5G7236.network.gandalf.connect";
    const ATTESTATION_KEY_ID: &str = "G3ef9pHt9N4DxUjo/hli9tV5gGDKaD3Ue7K8cqeN/r8=";
    const ATTESTATION_CHALLENGE: &str = "2f04f0ba-aa3a-42e4-8de1-7625c929faae";

    /// The app id the synthetic assertions below are made for.
    const DRAVR_APP_ID: &str = "RGDD7MLAK7.ai.dravr.app";

    /// A Secure Enclave stand-in: a P-256 key that signs assertions the way
    /// Apple specifies them, `ECDSA-SHA256(SHA-256(authenticatorData ‖
    /// SHA-256(clientData)))`, with authenticator data
    /// `SHA-256(app id) ‖ flags ‖ counter`.
    struct Device {
        key: EcdsaKeyPair,
        rng: SystemRandom,
    }

    impl Device {
        fn new() -> Self {
            let rng = SystemRandom::new();
            let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng);
            let Ok(pkcs8) = pkcs8 else {
                panic!("P-256 key generation failed");
            };
            let key =
                EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng);
            let Ok(key) = key else {
                panic!("P-256 key import failed");
            };
            Self { key, rng }
        }

        fn public_key(&self) -> Vec<u8> {
            self.key.public_key().as_ref().to_vec()
        }

        fn assert(&self, app_id: &str, counter: u32, client_data: &[u8]) -> String {
            let mut authenticator_data = Sha256::digest(app_id.as_bytes()).to_vec();
            authenticator_data.push(0x40);
            authenticator_data.extend_from_slice(&counter.to_be_bytes());
            let signature = self
                .key
                .sign(&self.rng, &nonce(&authenticator_data, client_data));
            let Ok(signature) = signature else {
                panic!("signing failed");
            };
            let object = Value::Map(vec![
                (
                    Value::Text("signature".to_owned()),
                    Value::Bytes(signature.as_ref().to_vec()),
                ),
                (
                    Value::Text("authenticatorData".to_owned()),
                    Value::Bytes(authenticator_data),
                ),
            ]);
            let mut encoded = Vec::new();
            assert!(ciborium::into_writer(&object, &mut encoded).is_ok());
            STANDARD.encode(encoded)
        }
    }

    /// 2024-07-01, inside the attestation's certificate validity.
    fn while_valid() -> UnixTime {
        UnixTime::since_unix_epoch(Duration::from_hours(19_905 * 24))
    }

    fn attest(
        challenge: &str,
        app_id: &str,
        key_id: &str,
        now: UnixTime,
    ) -> Result<AttestedKey, AppAttestError> {
        verify_attestation(
            ATTESTATION.trim(),
            key_id,
            challenge.as_bytes(),
            app_id,
            now,
        )
    }

    #[test]
    fn a_genuine_attestation_verifies() {
        let key = attest(
            ATTESTATION_CHALLENGE,
            ATTESTATION_APP_ID,
            ATTESTATION_KEY_ID,
            while_valid(),
        );
        let Ok(key) = key else {
            panic!("genuine attestation refused: {key:?}");
        };
        assert_eq!(key.key_id, ATTESTATION_KEY_ID);
        assert_eq!(key.environment, AppAttestEnvironment::Production);
        assert_eq!(key.public_key.len(), 65);
        assert_eq!(key.public_key.first(), Some(&0x04));
    }

    #[test]
    fn an_attestation_for_another_challenge_is_refused() {
        assert_eq!(
            attest(
                "another-challenge",
                ATTESTATION_APP_ID,
                ATTESTATION_KEY_ID,
                while_valid()
            ),
            Err(AppAttestError::NonceMismatch)
        );
    }

    #[test]
    fn an_attestation_for_another_app_is_refused() {
        assert_eq!(
            attest(
                ATTESTATION_CHALLENGE,
                DRAVR_APP_ID,
                ATTESTATION_KEY_ID,
                while_valid()
            ),
            Err(AppAttestError::AppIdMismatch)
        );
    }

    #[test]
    fn an_attestation_naming_another_key_is_refused() {
        let other_key = STANDARD.encode([7_u8; 32]);
        assert_eq!(
            attest(
                ATTESTATION_CHALLENGE,
                ATTESTATION_APP_ID,
                &other_key,
                while_valid()
            ),
            Err(AppAttestError::KeyIdMismatch)
        );
    }

    #[test]
    fn an_attestation_whose_certificate_expired_is_refused() {
        // 2025-06-01, after the credential certificate's notAfter.
        let later = UnixTime::since_unix_epoch(Duration::from_hours(20_240 * 24));
        assert_eq!(
            attest(
                ATTESTATION_CHALLENGE,
                ATTESTATION_APP_ID,
                ATTESTATION_KEY_ID,
                later
            ),
            Err(AppAttestError::UntrustedChain)
        );
    }

    #[test]
    fn an_attestation_with_its_chain_cut_short_is_refused() {
        // Drop the intermediate: the leaf alone does not reach Apple's root.
        let object = decode_cbor_map(ATTESTATION.trim()).unwrap_or_default();
        let truncated: Vec<(Value, Value)> = object
            .into_iter()
            .map(|(key, value)| {
                if key.as_text() == Some("attStmt") {
                    let statement = value
                        .into_map()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(name, entry)| match (name.as_text(), entry) {
                            (Some("x5c"), Value::Array(mut certs)) => {
                                certs.truncate(1);
                                (name, Value::Array(certs))
                            }
                            (_, entry) => (name, entry),
                        })
                        .collect();
                    (key, Value::Map(statement))
                } else {
                    (key, value)
                }
            })
            .collect();
        let mut encoded = Vec::new();
        assert!(ciborium::into_writer(&Value::Map(truncated), &mut encoded).is_ok());
        assert_eq!(
            verify_attestation(
                &STANDARD.encode(encoded),
                ATTESTATION_KEY_ID,
                ATTESTATION_CHALLENGE.as_bytes(),
                ATTESTATION_APP_ID,
                while_valid(),
            ),
            Err(AppAttestError::UntrustedChain)
        );
    }

    #[test]
    fn garbage_is_malformed_not_a_panic() {
        for evidence in ["", "not base64!", "AAAA", "oA=="] {
            assert!(matches!(
                verify_attestation(
                    evidence,
                    ATTESTATION_KEY_ID,
                    b"x",
                    ATTESTATION_APP_ID,
                    while_valid()
                ),
                Err(AppAttestError::Malformed(_))
            ));
            assert!(matches!(
                verify_assertion(evidence, b"x", DRAVR_APP_ID, &Device::new().public_key(), 0),
                Err(AppAttestError::Malformed(_))
            ));
        }
    }

    #[test]
    fn a_genuine_assertion_verifies_and_returns_its_counter() {
        let device = Device::new();
        let assertion = device.assert(DRAVR_APP_ID, 5, b"code-1");
        assert_eq!(
            verify_assertion(&assertion, b"code-1", DRAVR_APP_ID, &device.public_key(), 4),
            Ok(5)
        );
    }

    #[test]
    fn a_replayed_assertion_is_refused() {
        let device = Device::new();
        let assertion = device.assert(DRAVR_APP_ID, 5, b"code-1");
        for stored in [5, 6] {
            assert_eq!(
                verify_assertion(
                    &assertion,
                    b"code-1",
                    DRAVR_APP_ID,
                    &device.public_key(),
                    stored
                ),
                Err(AppAttestError::CounterReplayed)
            );
        }
    }

    #[test]
    fn an_assertion_over_other_client_data_is_refused() {
        let device = Device::new();
        let assertion = device.assert(DRAVR_APP_ID, 1, b"code-1");
        assert_eq!(
            verify_assertion(&assertion, b"code-2", DRAVR_APP_ID, &device.public_key(), 0),
            Err(AppAttestError::BadSignature)
        );
    }

    #[test]
    fn an_assertion_signed_by_another_key_is_refused() {
        let device = Device::new();
        let assertion = Device::new().assert(DRAVR_APP_ID, 1, b"code-1");
        assert_eq!(
            verify_assertion(&assertion, b"code-1", DRAVR_APP_ID, &device.public_key(), 0),
            Err(AppAttestError::BadSignature)
        );
    }

    #[test]
    fn an_assertion_for_another_app_is_refused() {
        let device = Device::new();
        let assertion = device.assert(ATTESTATION_APP_ID, 1, b"code-1");
        assert_eq!(
            verify_assertion(&assertion, b"code-1", DRAVR_APP_ID, &device.public_key(), 0),
            Err(AppAttestError::AppIdMismatch)
        );
    }
}
