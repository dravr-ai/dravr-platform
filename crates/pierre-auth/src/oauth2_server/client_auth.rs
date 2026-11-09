// ABOUTME: Client authentication on the token endpoint: HTTP Basic (client_secret_basic) next to the form body
// ABOUTME: Decides which credentials a token request carries, refusing a request that sends them two ways

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Client authentication
//!
//! RFC 6749 §2.3.1 requires an authorization server to accept the client's
//! credentials in an `Authorization: Basic` header, the user name and password
//! being the form-urlencoded `client_id` and `client_secret`. A client may
//! instead put them in the request body (`client_secret_post`), and a public
//! client sends a `client_id` and no secret at all (`none`). A request must use
//! one method: RFC 6749 §2.3 forbids more than one.

use super::models::OAuth2Error;
use base64::{engine::general_purpose, Engine as _};
use secrecy::SecretString;
use std::borrow::Cow;

/// The credentials a token request authenticates its client with.
#[derive(Debug)]
pub struct ClientCredentials {
    /// The client identifier
    pub client_id: String,
    /// The secret; empty for a public client, which presents none
    pub client_secret: SecretString,
}

/// Whether an `Authorization` header value names the `Basic` scheme: a client
/// that authenticated this way is answered 401 when it fails (RFC 6749 §5.2).
#[must_use]
pub fn is_basic_authorization(authorization: Option<&str>) -> bool {
    authorization
        .and_then(|header| header.split_once(' '))
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("basic"))
}

/// Resolve the client credentials of a token request from its
/// `Authorization` header and its form fields.
///
/// # Errors
/// Returns `invalid_request` when the request has no `client_id`, sends
/// credentials by more than one method, or carries an `Authorization` header
/// that is not well-formed Basic; and `invalid_client` when the `client_id`
/// in a Basic header and in the body differ.
pub fn resolve_client_credentials(
    authorization: Option<&str>,
    form_client_id: Option<&str>,
    form_client_secret: Option<&str>,
) -> Result<ClientCredentials, OAuth2Error> {
    if let Some((client_id, client_secret)) = basic_credentials(authorization)? {
        if form_client_secret.is_some() {
            return Err(OAuth2Error::invalid_request(
                "Client credentials must be sent by one method only",
            ));
        }
        if form_client_id.is_some_and(|form| form != client_id) {
            return Err(OAuth2Error::invalid_client());
        }
        return Ok(ClientCredentials {
            client_id,
            client_secret: SecretString::from(client_secret),
        });
    }
    let client_id = form_client_id
        .ok_or_else(|| OAuth2Error::invalid_request("Missing client_id parameter"))?;
    Ok(ClientCredentials {
        client_id: client_id.to_owned(),
        // Secrets are base64, and a form decoder turns their `+` into a space.
        client_secret: SecretString::from(
            form_client_secret
                .map(|secret| secret.replace(' ', "+"))
                .unwrap_or_default(),
        ),
    })
}

/// The `(client_id, client_secret)` of a `Basic` `Authorization` header, or
/// `None` when the header is absent or names another scheme.
fn basic_credentials(header: Option<&str>) -> Result<Option<(String, String)>, OAuth2Error> {
    let Some(header) = header else {
        return Ok(None);
    };
    let Some((scheme, encoded)) = header.split_once(' ') else {
        return Ok(None);
    };
    if !scheme.eq_ignore_ascii_case("basic") {
        return Ok(None);
    }
    let malformed = || OAuth2Error::invalid_request("Malformed Basic authorization header");
    let decoded = general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|_| malformed())?;
    let decoded = String::from_utf8(decoded).map_err(|_| malformed())?;
    let (id, secret) = decoded.split_once(':').ok_or_else(malformed)?;
    let unescape = |part: &str| {
        urlencoding::decode(part)
            .map(Cow::into_owned)
            .map_err(|_| malformed())
    };
    let client_id = unescape(id)?;
    if client_id.is_empty() {
        return Err(malformed());
    }
    Ok(Some((client_id, unescape(secret)?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    fn basic(id: &str, secret: &str) -> String {
        format!(
            "Basic {}",
            general_purpose::STANDARD.encode(format!("{id}:{secret}"))
        )
    }

    #[test]
    fn basic_credentials_are_read_and_unescaped() {
        let header = basic("mcp_client_1", "a%2Bb%2Fc%3D");
        let got = resolve_client_credentials(Some(&header), None, None).unwrap();
        assert_eq!(got.client_id, "mcp_client_1");
        assert_eq!(got.client_secret.expose_secret(), "a+b/c=");
    }

    #[test]
    fn an_unescaped_plus_in_a_basic_secret_stays_a_plus() {
        let header = basic("c", "a+b/c=");
        let got = resolve_client_credentials(Some(&header), None, None).unwrap();
        assert_eq!(got.client_secret.expose_secret(), "a+b/c=");
    }

    #[test]
    fn the_scheme_is_case_insensitive() {
        let header = basic("c", "s").replacen("Basic", "bAsIc", 1);
        assert!(resolve_client_credentials(Some(&header), None, None).is_ok());
    }

    #[test]
    fn a_basic_header_and_a_matching_form_client_id_agree() {
        let header = basic("c", "s");
        let got = resolve_client_credentials(Some(&header), Some("c"), None).unwrap();
        assert_eq!(got.client_secret.expose_secret(), "s");
    }

    #[test]
    fn a_basic_header_with_another_form_client_id_is_refused() {
        let header = basic("c", "s");
        let error = resolve_client_credentials(Some(&header), Some("other"), None).unwrap_err();
        assert_eq!(error.error, "invalid_client");
    }

    #[test]
    fn credentials_sent_two_ways_are_refused() {
        let header = basic("c", "s");
        let error = resolve_client_credentials(Some(&header), Some("c"), Some("s")).unwrap_err();
        assert_eq!(error.error, "invalid_request");
    }

    #[test]
    fn a_malformed_basic_header_is_refused() {
        for header in ["Basic !!!", "Basic Zm9v", "Basic ", "Basic OnNlY3JldA=="] {
            let error = resolve_client_credentials(Some(header), None, None).unwrap_err();
            assert_eq!(error.error, "invalid_request", "{header}");
        }
    }

    #[test]
    fn a_form_post_carries_the_secret_and_restores_plus_signs() {
        let got = resolve_client_credentials(None, Some("c"), Some("a b")).unwrap();
        assert_eq!(got.client_secret.expose_secret(), "a+b");
    }

    #[test]
    fn a_public_client_sends_a_client_id_and_no_secret() {
        let got = resolve_client_credentials(None, Some("https://x.example/c.json"), None).unwrap();
        assert_eq!(got.client_secret.expose_secret(), "");
    }

    #[test]
    fn no_client_id_anywhere_is_refused() {
        assert!(resolve_client_credentials(None, None, None).is_err());
        assert!(resolve_client_credentials(None, None, Some("s")).is_err());
    }

    #[test]
    fn only_the_basic_scheme_reads_as_basic() {
        assert!(is_basic_authorization(Some("bAsIc abc")));
        assert!(!is_basic_authorization(Some("Bearer abc")));
        assert!(!is_basic_authorization(Some("Basic")));
        assert!(!is_basic_authorization(None));
    }

    #[test]
    fn another_scheme_falls_through_to_the_form() {
        let got = resolve_client_credentials(Some("Bearer abc"), Some("c"), Some("s")).unwrap();
        assert_eq!(got.client_id, "c");
    }
}
