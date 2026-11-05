// ABOUTME: The consent screen's statements about who is asking and where the data goes
// ABOUTME: Verified or unverified client, loopback warning, health-data line, outside-Québec and express-consent lines
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What the consent screen tells the athlete beyond the scopes.
//!
//! Every value is HTML-escaped here, so the page builder substitutes the
//! results as markup. The wording is plain and factual; the Law 25 lines
//! (s.8 and s.12 of the Act respecting the protection of personal information
//! in the private sector) are drafted for counsel's review and are not legal
//! advice.

use pierre_auth::oauth2_server::cimd::is_metadata_document_id;
use pierre_auth::oauth2_server::client_registration::redirects_to_loopback;
use pierre_core::html::escape_html_attribute;
use url::Url;

/// A client Dravr knows by its metadata-document URL: the one place a
/// registered name can be trusted, because the URL is the client's identity
/// and only its owner can serve a document there. A name a client registers
/// for itself (dynamic registration) proves nothing and is never badged.
struct KnownClient {
    /// The `client_id`: the https URL of the client's metadata document
    client_id: &'static str,
    /// The product, as the badge names it
    product: &'static str,
    /// The company operating it
    operator: &'static str,
    /// The country where it processes what it receives
    country: &'static str,
}

/// The clients that get a verified badge.
const KNOWN_CLIENTS: [KnownClient; 2] = [
    KnownClient {
        client_id: "https://claude.ai/oauth/claude-code-client-metadata",
        product: "Claude Code",
        operator: "Anthropic",
        country: "the United States",
    },
    KnownClient {
        client_id: "https://chatgpt.com/oauth/client.json",
        product: "ChatGPT",
        operator: "OpenAI",
        country: "the United States",
    },
];

/// What Dravr can say about the client asking.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ClientTrust {
    /// A client Dravr knows by its metadata URL
    Verified {
        /// The product's name
        product: &'static str,
        /// The company operating it
        operator: &'static str,
        /// Where it processes what it receives
        country: &'static str,
    },
    /// Any other client
    Unverified {
        /// Whether it returns the athlete only to their own device, which is
        /// what a client running on the athlete's machine does
        loopback_only: bool,
        /// The host of its metadata document, when it has one. A document can
        /// call itself anything, but only the host's owner can serve it there.
        document_host: Option<String>,
    },
}

impl ClientTrust {
    /// The trust Dravr places in `client_id`, returning to `redirect_uri`.
    pub(super) fn of(client_id: &str, redirect_uri: &str) -> Self {
        KNOWN_CLIENTS
            .iter()
            .find(|known| known.client_id == client_id)
            .map_or_else(
                || Self::Unverified {
                    loopback_only: redirects_to_loopback(redirect_uri),
                    document_host: Url::parse(client_id)
                        .ok()
                        .filter(|_| is_metadata_document_id(client_id))
                        .and_then(|url| url.host_str().map(str::to_owned)),
                },
                |known| Self::Verified {
                    product: known.product,
                    operator: known.operator,
                    country: known.country,
                },
            )
    }
}

/// The line under the heading: a badge for a known client, a warning for any
/// other, and for one that returns to the athlete's own machine, what that
/// means.
pub(super) fn trust_line(trust: &ClientTrust) -> String {
    match trust {
        ClientTrust::Verified {
            product, operator, ..
        } => format!(
            r#"<p class="fineprint"><strong>Verified: {} by {}</strong></p>"#,
            escape_html_attribute(product),
            escape_html_attribute(operator),
        ),
        ClientTrust::Unverified {
            loopback_only,
            document_host,
        } => {
            let mut line = String::from(
                r#"<p class="alert alert-error"><strong>Unverified app.</strong> Dravr has not verified who runs this app. Allow access only if you started this connection yourself."#,
            );
            if let Some(host) = document_host {
                line.push_str(" It is published by <strong>");
                line.push_str(&escape_html_attribute(host));
                line.push_str("</strong>; the name it gives itself is not checked.");
            }
            if *loopback_only {
                line.push_str(
                    " It returns you to a program on this device, so only continue if you are running it.",
                );
            }
            line.push_str("</p>");
            line
        }
    }
}

/// What the data is, and that it is wellness information.
pub(super) fn health_line(client_label: &str) -> String {
    format!(
        r#"<p class="fineprint">Training, sleep and recovery data you connected to Dravr will be shared with <span class="client">{}</span>. Dravr gives training and wellness information, not medical advice.</p>"#,
        escape_html_attribute(client_label),
    )
}

/// Law 25 s.8: that the information may be communicated outside Québec, and
/// to whom. Names the operator and country for a known client; says only
/// that the app may process it elsewhere for any other, since Dravr does not
/// know where it runs.
pub(super) fn outside_quebec_line(trust: &ClientTrust, client_label: &str) -> String {
    let client = escape_html_attribute(client_label);
    match trust {
        ClientTrust::Verified {
            operator, country, ..
        } => format!(
            r#"<p class="fineprint">This information may be communicated outside Québec: <span class="client">{client}</span> is run by {} and processes what it receives in {}.</p>"#,
            escape_html_attribute(operator),
            escape_html_attribute(country),
        ),
        ClientTrust::Unverified { .. } => format!(
            r#"<p class="fineprint">This information may be communicated outside Québec: Dravr does not know where <span class="client">{client}</span> processes what it receives.</p>"#,
        ),
    }
}

/// Law 25 s.12: health data is sensitive, so consent is expressly asked for,
/// in words apart from the scope list.
pub(super) fn express_consent_line(client_label: &str) -> String {
    format!(
        r#"<p class="fineprint">Your training, sleep and recovery data is sensitive personal information. By choosing Allow access you expressly consent to Dravr sharing it with <span class="client">{}</span>.</p>"#,
        escape_html_attribute(client_label),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHATGPT: &str = "https://chatgpt.com/oauth/client.json";

    #[test]
    fn a_known_metadata_url_is_verified_and_a_registered_name_never_is() {
        assert_eq!(
            ClientTrust::of(CHATGPT, "https://chatgpt.com/cb"),
            ClientTrust::Verified {
                product: "ChatGPT",
                operator: "OpenAI",
                country: "the United States",
            }
        );
        // A dynamically registered client is a random id whatever name it gave.
        assert_eq!(
            ClientTrust::of("dyn-8c1f", "https://claude.ai/api/mcp/auth_callback"),
            ClientTrust::Unverified {
                loopback_only: false,
                document_host: None,
            }
        );
    }

    #[test]
    fn a_loopback_return_is_flagged_and_a_lookalike_host_is_not() {
        for uri in [
            "http://localhost:53124/callback",
            "http://127.0.0.1:8000/cb",
            "http://[::1]:9/cb",
        ] {
            assert_eq!(
                ClientTrust::of("x", uri),
                ClientTrust::Unverified {
                    loopback_only: true,
                    document_host: None,
                },
                "{uri}"
            );
        }
        assert_eq!(
            ClientTrust::of("x", "https://localhost.evil.example/cb"),
            ClientTrust::Unverified {
                loopback_only: false,
                document_host: None,
            }
        );
    }

    #[test]
    fn a_metadata_document_client_is_shown_with_the_host_it_cannot_fake() {
        let trust = ClientTrust::of(
            "https://chatgpt.com.evil.example/client.json",
            "https://chatgpt.com.evil.example/cb",
        );
        assert_eq!(
            trust,
            ClientTrust::Unverified {
                loopback_only: false,
                document_host: Some("chatgpt.com.evil.example".to_owned()),
            }
        );
        assert!(trust_line(&trust).contains("<strong>chatgpt.com.evil.example</strong>"));
    }

    #[test]
    fn the_lines_escape_the_client_label_and_say_what_the_law_asks() {
        let hostile = "<script>alert(1)</script>";
        let trust = ClientTrust::Unverified {
            loopback_only: true,
            document_host: None,
        };
        for line in [
            trust_line(&trust),
            health_line(hostile),
            outside_quebec_line(&trust, hostile),
            express_consent_line(hostile),
        ] {
            assert!(!line.contains("<script>"), "{line}");
        }
        assert!(trust_line(&trust).contains("Unverified app"));
        assert!(trust_line(&trust).contains("program on this device"));
        assert!(health_line("X").contains("not medical advice"));
        assert!(outside_quebec_line(&trust, "X").contains("outside Québec"));
        assert!(express_consent_line("X").contains("expressly consent"));
    }

    #[test]
    fn a_verified_client_is_named_with_its_operator_and_country() {
        let trust = ClientTrust::of(CHATGPT, "https://chatgpt.com/cb");
        assert!(trust_line(&trust).contains("Verified: ChatGPT by OpenAI"));
        let line = outside_quebec_line(&trust, "ChatGPT");
        assert!(
            line.contains("OpenAI") && line.contains("the United States"),
            "{line}"
        );
    }
}
