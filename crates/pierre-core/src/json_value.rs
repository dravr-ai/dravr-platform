// ABOUTME: Converts a payload to a serde_json Value holding exactly the numbers its JSON text holds
// ABOUTME: serde_json::to_value and json! widen f32 to f64; this is the one exact conversion for readers

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The one conversion from a typed payload to a [`Value`] for anything a reader
//! sees — a tool result, a reply block, an activity row.
//!
//! `serde_json::to_value` and `json!` widen every `f32` to `f64` on the way into
//! a `Value`, and 12.8 has no exact `f32`: it reached athletes as
//! 12.800000190734863 — in activity lists (weather temperature, RPE), in
//! activity detail (temperature, humidity, wind, altitude, blood oxygen — every `f32`
//! on the cageux `Activity`), in sleep payloads and on plan cards. Writing the
//! payload to JSON text formats each `f32` at its own shortest precision, and
//! parsing that text back keeps exactly those digits. The parse is exact for
//! `f64` too because this crate enables `serde_json`'s `float_roundtrip`, which
//! Cargo unifies into every crate of the build.

use serde::Serialize;
use serde_json::Value;

/// Convert `payload` to a [`Value`] whose numbers are the ones its JSON text
/// carries.
///
/// # Errors
///
/// Returns the serde error when `payload` does not serialize.
pub fn to_value_as_written<T: Serialize + ?Sized>(payload: &T) -> serde_json::Result<Value> {
    serde_json::from_str(&serde_json::to_string(payload)?)
}
