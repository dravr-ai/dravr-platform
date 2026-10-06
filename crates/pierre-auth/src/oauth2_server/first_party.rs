// ABOUTME: Dravr's own web and mobile apps as OAuth clients of this authorization server
// ABOUTME: Their identifiers and the redirect URIs this deployment lets them receive a code at
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The first-party clients (carnet#787).
//!
//! Dravr's web and mobile apps sign in the way RFC 9700 §2.4 and RFC 8252
//! require of any client: the athlete types the password on this server's
//! hosted login page, never into the app, and the app receives an
//! authorization code it redeems with its PKCE verifier. Two things set them
//! apart from an integration:
//!
//! - **No consent screen.** The athlete is signing in to Dravr itself, so
//!   there is nothing to consent to, and the code is redeemed for a
//!   first-party session rather than a delegated grant.
//! - **Redirect URIs fixed by the deployment, not registered.** The web
//!   callback is `/auth/callback` on this deployment's own origins; the
//!   mobile callback is the app's `dravr://auth/callback`. Because consent is
//!   skipped, a code must never reach any other place: anyone who started the
//!   flow holds the PKCE verifier, so a redirect they control would hand them
//!   the athlete's session.
//!
//! Both identifiers are public, as every browser and native client's is (RFC
//! 8252 §8.4). They are public clients: no secret, PKCE mandatory. Their
//! registration rows are written by a migration
//! (`20261006150000_first_party_oauth2_clients`), because the codes they are
//! issued reference one.

use serde::{Deserialize, Serialize};

/// The `client_id` Dravr's web app signs in as.
pub const WEB_CLIENT_ID: &str = "dravr-web";

/// The `client_id` Dravr's mobile app signs in as.
pub const MOBILE_CLIENT_ID: &str = "dravr-mobile";

/// The path both apps receive their authorization code on.
pub const CALLBACK_PATH: &str = "/auth/callback";

/// The mobile app's callback: its registered `dravr` scheme.
pub const MOBILE_CALLBACK_URI: &str = "dravr://auth/callback";

/// The path Expo Go appends its deep links under: a development build run
/// in Expo Go returns to `exp://<host>:<port>/--/auth/callback`.
const EXPO_GO_CALLBACK_SUFFIX: &str = "/--/auth/callback";

/// Whether `client_id` names one of Dravr's own apps.
#[must_use]
pub fn is_first_party(client_id: &str) -> bool {
    client_id == WEB_CLIENT_ID || client_id == MOBILE_CLIENT_ID
}

/// Where this deployment lets the first-party clients receive a code.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirstPartyRedirects {
    /// The origins the web app is served from (`scheme://host[:port]`); its
    /// callback is [`CALLBACK_PATH`] on each.
    pub web_origins: Vec<String>,
    /// Accept the mobile app's Expo Go callback on any `exp://` host.
    ///
    /// Only for a server on a developer's machine or a CI runner: an
    /// `exp://` URL loads whatever bundle its host serves, so on a deployed
    /// server it would let anyone who sends an athlete a crafted link receive
    /// their code.
    pub allow_expo_go: bool,
}

impl FirstPartyRedirects {
    /// Whether `client_id`, a first-party client, may receive its code at
    /// `redirect_uri`. Exact matches only, but for the opt-in Expo Go host.
    #[must_use]
    pub fn accepts(&self, client_id: &str, redirect_uri: &str) -> bool {
        match client_id {
            WEB_CLIENT_ID => self.web_origins.iter().any(|origin| {
                redirect_uri
                    .strip_prefix(origin.trim_end_matches('/'))
                    .is_some_and(|path| path == CALLBACK_PATH)
            }),
            MOBILE_CLIENT_ID => {
                redirect_uri == MOBILE_CALLBACK_URI
                    || (self.allow_expo_go && is_expo_go_callback(redirect_uri))
            }
            _ => false,
        }
    }
}

/// `exp://<authority>/--/auth/callback`, with nothing in the authority that
/// could smuggle a path, query, fragment or user-info.
fn is_expo_go_callback(redirect_uri: &str) -> bool {
    redirect_uri
        .strip_prefix("exp://")
        .and_then(|rest| rest.strip_suffix(EXPO_GO_CALLBACK_SUFFIX))
        .is_some_and(|authority| {
            !authority.is_empty()
                && authority
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redirects(allow_expo_go: bool) -> FirstPartyRedirects {
        FirstPartyRedirects {
            web_origins: vec![
                "https://app.dravr.ai".to_owned(),
                "http://localhost:5173/".to_owned(),
            ],
            allow_expo_go,
        }
    }

    #[test]
    fn web_callback_is_the_callback_path_on_a_configured_origin() {
        let policy = redirects(false);
        assert!(policy.accepts(WEB_CLIENT_ID, "https://app.dravr.ai/auth/callback"));
        assert!(policy.accepts(WEB_CLIENT_ID, "http://localhost:5173/auth/callback"));
        assert!(!policy.accepts(WEB_CLIENT_ID, "https://app.dravr.ai/auth/callback?x=1"));
        assert!(!policy.accepts(WEB_CLIENT_ID, "https://app.dravr.ai.evil.io/auth/callback"));
        assert!(!policy.accepts(WEB_CLIENT_ID, "https://evil.io/auth/callback"));
        assert!(!policy.accepts(WEB_CLIENT_ID, MOBILE_CALLBACK_URI));
    }

    #[test]
    fn mobile_callback_is_the_app_scheme() {
        let policy = redirects(false);
        assert!(policy.accepts(MOBILE_CLIENT_ID, MOBILE_CALLBACK_URI));
        assert!(!policy.accepts(MOBILE_CLIENT_ID, "https://app.dravr.ai/auth/callback"));
        assert!(!policy.accepts(MOBILE_CLIENT_ID, "dravr://auth/callback/x"));
    }

    #[test]
    fn expo_go_callback_needs_the_opt_in() {
        let uri = "exp://192.168.1.20:8082/--/auth/callback";
        assert!(!redirects(false).accepts(MOBILE_CLIENT_ID, uri));
        assert!(redirects(true).accepts(MOBILE_CLIENT_ID, uri));
        assert!(redirects(true).accepts(MOBILE_CLIENT_ID, "exp://127.0.0.1:8082/--/auth/callback"));
    }

    #[test]
    fn expo_go_authority_cannot_smuggle_a_path_or_user_info() {
        let policy = redirects(true);
        for uri in [
            "exp:///--/auth/callback",
            "exp://evil.io/x/--/auth/callback",
            "exp://user@evil.io/--/auth/callback",
            "exp://evil.io?/--/auth/callback",
            "exp://evil.io#/--/auth/callback",
        ] {
            assert!(!policy.accepts(MOBILE_CLIENT_ID, uri), "{uri}");
        }
    }

    #[test]
    fn other_clients_are_never_first_party() {
        assert!(!is_first_party("mcp_client_123"));
        assert!(!redirects(true).accepts("mcp_client_123", MOBILE_CALLBACK_URI));
    }
}
