// ABOUTME: Tests for key management — MEK raw encryption and DEK wrap/unwrap via the KekProvider
// ABOUTME: Validates master key operations and that a DEK survives a wrap/unwrap roundtrip
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_auth::key_management::{
    DatabaseEncryptionKey, KekProvider, LocalKekProvider, MasterEncryptionKey,
};

#[test]
fn test_mek_encrypt_decrypt() {
    let mek = MasterEncryptionKey::from_bytes([1u8; 32]);

    let data = b"test data to encrypt";
    let encrypted = mek.encrypt(data).unwrap();
    let decrypted = mek.decrypt(&encrypted).unwrap();

    assert_eq!(data.as_slice(), decrypted.as_slice());
}

#[tokio::test]
async fn test_dek_wrap_unwrap_via_kek_provider() {
    let kek = LocalKekProvider::from_mek(MasterEncryptionKey::from_bytes([1u8; 32]));

    let dek = DatabaseEncryptionKey::generate();
    let original_key = *dek.as_bytes();

    // Wrap the DEK with the KEK, then unwrap and reconstruct it.
    let wrapped = kek.wrap(dek.as_bytes()).await.unwrap();
    let unwrapped = kek.unwrap(&wrapped).await.unwrap();
    let restored = DatabaseEncryptionKey::from_unwrapped(&unwrapped).unwrap();

    assert_eq!(original_key, *restored.as_bytes());
}
