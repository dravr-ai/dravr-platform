// ABOUTME: serde helpers for SecretString fields whose wire format carries the secret on purpose
// ABOUTME: A response that hands a client its own token, or a request a client sends, exposes it only here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Serializers for `SecretString` fields whose wire format carries the secret.
//!
//! A [`SecretString`] has no `Serialize` impl, so a struct holding one
//! cannot put it on the wire by accident. Where the wire format is the
//! point — the token a login returns, the password a client posts, a row
//! the cache stores — the field names one of these functions with
//! `#[serde(serialize_with = "...")]`, which keeps the JSON exactly what
//! a plain `String` produced while `Debug` still prints a redaction.

use secrecy::{ExposeSecret, SecretString};
use serde::Serializer;

/// Serialize the secret as the plain string it holds.
///
/// # Errors
///
/// Returns the serializer's error.
pub fn serialize_exposed<S: Serializer>(
    secret: &SecretString,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose_secret())
}

/// Serialize an optional secret as the plain string it holds, or `null`.
///
/// # Errors
///
/// Returns the serializer's error.
pub fn serialize_exposed_option<S: Serializer>(
    secret: &Option<SecretString>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match secret {
        Some(secret) => serializer.serialize_some(secret.expose_secret()),
        None => serializer.serialize_none(),
    }
}
