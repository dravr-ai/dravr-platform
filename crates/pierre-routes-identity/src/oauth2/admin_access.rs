// ABOUTME: The consent screen's Admin access box: offered unticked to a live admin, naming in bold where the access goes
// ABOUTME: Read back from the form only for a live admin on a request bound to the platform's own resource (carnet#1511)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Admin access on the consent screen (carnet#1511, step 12)
//!
//! An admin is offered one unticked box, Admin access, on every app's consent
//! screen (JF, Q20 and Q26: no allowlist). Ticking it is the only way an
//! OAuth grant holds `admin`, and with it the admin tools, the tools kept on
//! Dravr's own surfaces, and support sessions.
//!
//! With the box offered to every app, the host the code and the admin access
//! go to is the one thing that tells the real client from a look-alike that
//! registered itself under the same name, so it is shown in bold beside the
//! box, never only the name the client gives itself:
//!
//! - the redirect host, where the code goes;
//! - also the host of the client's metadata document (`client_id`) when it is
//!   not the redirect host;
//! - "a program on this device" for a loopback redirect, which has no domain.
//!
//! The admin's role line ([`KEY_CONSENT_ROLE_ADMIN`]) says what ticking the box
//! gives, so it is shown inside the box and only with it.
//!
//! A box whose lines read empty in this language and in English is neither
//! shown nor honoured ([`offered_in`]): an admin is never handed an unlabelled
//! choice that grants admin.

use std::collections::HashMap;
use std::iter;

use pierre_auth::oauth2_server::cimd::is_metadata_document_id;
use pierre_auth::oauth2_server::client_registration::redirects_to_loopback;
use pierre_auth::oauth2_server::endpoints::{AdminAccess, CheckedAuthorization};
use pierre_contremaitre::consent_strings::{
    KEY_CONSENT_ADMIN_ACCESS_GOES_TO as KEY_GOES_TO,
    KEY_CONSENT_ADMIN_ACCESS_GOES_TO_TWO_HOSTS as KEY_GOES_TO_TWO_HOSTS,
    KEY_CONSENT_ADMIN_ACCESS_HINT as KEY_HINT, KEY_CONSENT_ADMIN_ACCESS_LABEL as KEY_LABEL,
    KEY_CONSENT_ADMIN_ACCESS_THIS_COMPUTER as KEY_THIS_COMPUTER, KEY_CONSENT_ROLE_ADMIN,
};
use pierre_core::html::escape_html_attribute;
use pierre_core::models::User;
use tracing::{info, warn};
use url::Url;

use super::consent_text::ConsentText;
use super::pages::redirect_host;

/// Every line the box shows of its own.
pub(super) const ADMIN_ACCESS_KEYS: [&str; 5] = [
    KEY_LABEL,
    KEY_HINT,
    KEY_GOES_TO,
    KEY_GOES_TO_TWO_HOSTS,
    KEY_THIS_COMPUTER,
];

/// The form field the box submits, and the value a ticked box sends.
pub(super) const FIELD: &str = "admin_access";
const TICKED: &str = "granted";

/// Whether `text` can show the box: every line of it reads in this language
/// or in English. A box it cannot label is not offered, and its field is not
/// read.
pub(super) fn offered_in(text: &ConsentText<'_>) -> bool {
    ADMIN_ACCESS_KEYS
        .iter()
        .chain(iter::once(&KEY_CONSENT_ROLE_ADMIN))
        .all(|key| !text.line(key).is_empty())
}

/// What `user` decided about Admin access in the consent `form` for `checked`.
///
/// [`AdminAccess::Ticked`] only for a user who is an admin now (read by the
/// consent route at the submission), on a request bound to the platform's own
/// resource, whose screen could show the box, with the box ticked. A form
/// that ticks it for anyone else, however it was built, grants nothing.
pub(super) fn decided(
    form: &HashMap<String, String>,
    user: &User,
    checked: &CheckedAuthorization,
    text: &ConsentText<'_>,
) -> AdminAccess {
    let ticked = form.get(FIELD).is_some_and(|value| value == TICKED);
    if !ticked {
        return AdminAccess::Withheld;
    }
    if !(user.role.is_admin() && checked.admin_grantable && offered_in(text)) {
        warn!(user_id = %user.id, "consent form ticked Admin access for a user or request it is not offered to; not granted");
        return AdminAccess::Withheld;
    }
    info!(user_id = %user.id, "admin granted Admin access at consent");
    AdminAccess::Ticked
}

/// The Admin access box for `client` (already markup), unticked, with the
/// admin's role line and where the access goes in bold; empty when `text`
/// cannot show it.
pub(super) fn admin_access_box(
    client: &str,
    client_id: &str,
    redirect_uri: &str,
    text: &ConsentText<'_>,
) -> String {
    if !offered_in(text) {
        return String::new();
    }
    format!(
        concat!(
            r#"<div class="callout">"#,
            r#"<label class="check"><input type="checkbox" name="{field}" value="{ticked}"> "#,
            r#"<span class="callout-title">{label}</span></label>"#,
            "<p>{role}</p><p>{goes_to}</p><p>{hint}</p></div>"
        ),
        field = FIELD,
        ticked = TICKED,
        label = text.html(KEY_LABEL, &[]),
        role = text.html(KEY_CONSENT_ROLE_ADMIN, &[client]),
        goes_to = goes_to_line(client_id, redirect_uri, text),
        hint = text.html(KEY_HINT, &[]),
    )
}

/// Where the admin access goes, as the box says it: the redirect host in
/// bold, or "a program on this device" for a loopback redirect, and the
/// metadata document's host in bold beside it when it is another host.
fn goes_to_line(client_id: &str, redirect_uri: &str, text: &ConsentText<'_>) -> String {
    let loopback = redirects_to_loopback(redirect_uri);
    let destination = if loopback {
        format!("<strong>{}</strong>", text.html(KEY_THIS_COMPUTER, &[]))
    } else {
        bold(&redirect_host(redirect_uri))
    };
    document_host(client_id)
        .filter(|host| loopback || !same_host(host, redirect_uri))
        .map_or_else(
            || text.html(KEY_GOES_TO, &[&destination]),
            |host| text.html(KEY_GOES_TO_TWO_HOSTS, &[&destination, &bold(&host)]),
        )
}

/// `host`, escaped, in bold.
fn bold(host: &str) -> String {
    format!("<strong>{}</strong>", escape_html_attribute(host))
}

/// The host of the client's metadata document, when its `client_id` is one.
fn document_host(client_id: &str) -> Option<String> {
    if !is_metadata_document_id(client_id) {
        return None;
    }
    Url::parse(client_id)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
}

/// Whether `host` is the host `redirect_uri` returns to, port aside.
fn same_host(host: &str, redirect_uri: &str) -> bool {
    Url::parse(redirect_uri)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|redirect| redirect.eq_ignore_ascii_case(host))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_host_is_read_only_from_a_metadata_url() {
        assert_eq!(
            document_host("https://claude.ai/oauth/mcp-oauth-client-metadata").as_deref(),
            Some("claude.ai"),
            "bug: a metadata document URL's host was not read"
        );
        assert_eq!(
            document_host("3f1c6b2e-registered-client"),
            None,
            "bug: a registered client id was read as a metadata document"
        );
    }

    #[test]
    fn the_same_host_is_compared_without_its_port_or_case() {
        assert!(
            same_host("claude.ai", "https://Claude.ai:443/api/mcp/auth_callback"),
            "bug: a port or a capital told the same host apart"
        );
        assert!(
            !same_host("claude.ai", "https://evil.example/callback"),
            "bug: another host read as the redirect host"
        );
    }
}
