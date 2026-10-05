// ABOUTME: The clients the password grant serves — Dravr's own web and mobile apps, and nothing else
// ABOUTME: A password grant naming any other client_id is refused with invalid_client before the password is read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The password grant's client binding (carnet#768).
//!
//! The password grant mints a first-party session — the self grant, accepted
//! by chat and every REST route as Dravr's own app signed in. A third party
//! holding the athlete's password must not walk away with that, so the grant
//! names the client it is for, and only Dravr's own apps are served. An
//! integration authorizes through the OAuth authorization-code flow, which
//! mints a delegated grant as narrow as the athlete consents to, or is handed
//! the athlete's API key.
//!
//! Both identifiers are public, as every native and browser client's is (RFC
//! 8252 §8.4): this binds the grant to a declared first-party client, it does
//! not authenticate one.

use axum::response::Response;
use tracing::warn;

use crate::token_errors::oauth2_error;

/// The `client_id` Dravr's web app sends on the password grant.
const WEB_CLIENT_ID: &str = "dravr-web";

/// The `client_id` Dravr's mobile app sends on the password grant.
const MOBILE_CLIENT_ID: &str = "dravr-mobile";

/// The clients the password grant serves.
const FIRST_PARTY_CLIENT_IDS: [&str; 2] = [WEB_CLIENT_ID, MOBILE_CLIENT_ID];

/// The RFC 6749 §5.2 `invalid_client` answer for a password grant that names
/// no first-party client, or `None` when it names one.
///
/// Called before the password is read, so an unbound caller learns nothing
/// about the account and leaves no login behind.
pub fn refuse_unbound_client(client_id: Option<&str>) -> Option<Response> {
    if client_id.is_some_and(|id| FIRST_PARTY_CLIENT_IDS.contains(&id)) {
        return None;
    }
    warn!(
        client_id = client_id.unwrap_or("<none>"),
        "Password grant refused: not a first-party client"
    );
    Some(oauth2_error(
        "invalid_client",
        "The password grant is reserved for Dravr's own apps. Authorize your \
         application with OAuth (authorization code + PKCE) or use an API key.",
    ))
}
