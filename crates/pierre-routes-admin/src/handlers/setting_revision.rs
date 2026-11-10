// ABOUTME: The compare-and-set contract of an edited system setting: which version a save names, and its refusals
// ABOUTME: Shared by the Guardian and harness documents, which both clients read, edit whole and write back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The compare-and-set a document setting's save answers to.
//!
//! A save names the `updated_at` its editor read as `expected_updated_at`, or `null`
//! when it read no row. A body without it, or with a stale one, gets a 409 and writes
//! nothing, so two super admins cannot lose each other's edits.
//!
//! A refusal is JSON with a stable `code` and the sentence that says what to do:
//! `AppError` sanitizes a conflict down to one generic sentence and axum answers a
//! bad body in plain text.

use axum::{
    extract::rejection::JsonRejection,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

/// Which version of the row a save edits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExpectedRevision {
    /// `expected_updated_at` is missing: the save names no base and is refused.
    #[default]
    Unnamed,
    /// `expected_updated_at` is `null`: the editor read no row, so the save
    /// creates it, and only while there still is none.
    NoRow,
    /// `expected_updated_at` is the `updated_at` the editor read.
    At(DateTime<Utc>),
}

/// A JSON `null` and a missing field mean different things here. serde reaches
/// this only when the field is present; `#[serde(default)]` covers the missing case.
impl<'de> Deserialize<'de> for ExpectedRevision {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Option::<DateTime<Utc>>::deserialize(deserializer)?.map_or(Self::NoRow, Self::At))
    }
}

/// Body of a refused save.
#[derive(Debug, Serialize)]
struct SettingRefusal<'text> {
    /// Stable identifier for a client to branch on.
    #[serde(rename = "code")]
    refusal_code: &'text str,
    /// What was refused and the next action.
    message: &'text str,
}

/// The save's base is missing or no longer the row's.
pub(crate) fn stale_refusal(code: &str, message: &str) -> Response {
    (
        StatusCode::CONFLICT,
        Json(SettingRefusal {
            refusal_code: code,
            message,
        }),
    )
        .into_response()
}

/// The save's body did not parse: broken JSON, a wrong content type, or a value out
/// of a field's range.
pub(crate) fn malformed_refusal(code: &str, setting: &str, rejection: &JsonRejection) -> Response {
    (
        rejection.status(),
        Json(SettingRefusal {
            refusal_code: code,
            message: &format!("{setting} was not saved: {}", rejection.body_text()),
        }),
    )
        .into_response()
}
