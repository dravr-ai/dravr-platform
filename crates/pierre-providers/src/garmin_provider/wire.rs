// ABOUTME: The one Garmin Health API response the native Garmin provider deserializes: the user id behind a token
// ABOUTME: Every data read is served from what Garmin pushed, so no data response is read here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::Deserialize;

/// Garmin's `GET user/id` answer: the user id the access token belongs to,
/// which is all the Health API says about the user.
#[derive(Debug, Deserialize)]
pub(super) struct GarminUserIdResponse {
    #[serde(rename = "userId")]
    pub(super) user_id: String,
}
