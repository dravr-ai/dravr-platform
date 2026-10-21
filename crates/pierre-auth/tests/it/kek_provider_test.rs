// ABOUTME: Tests for the KekProvider abstraction and the env-backed LocalKekProvider
// ABOUTME: Verifies DEK wrap/unwrap roundtrip and that a mismatched KEK fails to unwrap
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_auth::key_management::{KekProvider, LocalKekProvider, MasterEncryptionKey};

#[tokio::test]
async fn local_kek_provider_wrap_unwrap_roundtrip() {
    let provider = LocalKekProvider::from_mek(MasterEncryptionKey::from_bytes([7u8; 32]));
    let dek = [42u8; 32];

    let wrapped = provider.wrap(&dek).await.expect("wrap should succeed");
    // Wrapped output must not equal the plaintext DEK and carries a nonce + tag.
    assert_ne!(wrapped.as_slice(), dek.as_slice());
    assert!(wrapped.len() > dek.len());

    let unwrapped = provider
        .unwrap(&wrapped)
        .await
        .expect("unwrap should succeed");
    assert_eq!(unwrapped.as_slice(), dek.as_slice());
}

#[tokio::test]
async fn wrap_uses_random_nonce_so_ciphertexts_differ() {
    let provider = LocalKekProvider::from_mek(MasterEncryptionKey::from_bytes([3u8; 32]));
    let dek = [9u8; 32];

    let a = provider.wrap(&dek).await.expect("wrap a");
    let b = provider.wrap(&dek).await.expect("wrap b");
    assert_ne!(a, b, "AES-GCM must use a fresh random nonce per wrap");

    assert_eq!(provider.unwrap(&a).await.expect("unwrap a"), dek);
    assert_eq!(provider.unwrap(&b).await.expect("unwrap b"), dek);
}

#[tokio::test]
async fn unwrap_with_wrong_kek_fails() {
    let writer = LocalKekProvider::from_mek(MasterEncryptionKey::from_bytes([1u8; 32]));
    let reader = LocalKekProvider::from_mek(MasterEncryptionKey::from_bytes([2u8; 32]));
    let dek = [55u8; 32];

    let wrapped = writer.wrap(&dek).await.expect("wrap with writer KEK");
    let result = reader.unwrap(&wrapped).await;
    assert!(
        result.is_err(),
        "unwrapping with a different KEK must fail (AEAD authentication)"
    );
}

#[tokio::test]
async fn unwrap_rejects_truncated_ciphertext() {
    let provider = LocalKekProvider::from_mek(MasterEncryptionKey::from_bytes([8u8; 32]));
    let result = provider.unwrap(&[0u8; 4]).await;
    assert!(
        result.is_err(),
        "ciphertext shorter than the nonce must fail"
    );
}
