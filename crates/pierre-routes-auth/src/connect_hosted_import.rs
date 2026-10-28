// ABOUTME: Hosted import page the connect picker opens, authed by the picker's connect link-token
// ABOUTME: Narrows that token into an import link-token for the page's start, follow and cancel calls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The hosted **import page** (carnet#1221).
//!
//! A chat user reaches the hosted connect picker
//! ([`crate::connect_hosted`]) from a connect link-token. Below its provider
//! cards, the picker offers "Import your own data", which opens
//! `/providers/connect/import?token=<connect token>`: this page.
//!
//! The page asks Strava for the athlete's export, takes the link Strava
//! emails, and follows the import. It does that with an **import
//! link-token** it mints from the connect token
//! ([`mint_import_link_token`]): one athlete, one source, at most an hour
//! and never longer than the connect token. That token starts, follows and
//! cancels the import through the routes `pierre-server` serves under
//! `/providers/connect/import/{source}`; it reads no import report, deletes
//! no history, and opens no provider login. The report and the delete stay
//! in Dravr's own apps, behind the athlete's session.
//!
//! The page is written in the stored locale of the athlete the token names,
//! like every hosted page ([`crate::hosted_page`]), from strings it shares
//! with the web and mobile import screens.

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Response};
use pierre_contremaitre::hosted_strings::{
    IMPORT_STRAVA_FAILURES, IMPORT_STRAVA_REFUSALS, KEY_HOSTED_COMMON_LINKED_FROM,
    KEY_HOSTED_ERROR_GENERIC, KEY_HOSTED_ERROR_INVALID_LINK, KEY_HOSTED_ERROR_MALFORMED_LINK,
    KEY_HOSTED_ERROR_MISSING_TOKEN, KEY_HOSTED_IMPORT_DONE_TITLE, KEY_HOSTED_IMPORT_NOT_IMPORTED,
    KEY_HOSTED_IMPORT_PROGRESS, KEY_HOSTED_IMPORT_STOPPED, KEY_IMPORT_STATE_DOWNLOADING,
    KEY_IMPORT_STATE_QUEUED, KEY_IMPORT_STRAVA_GENERIC_ERROR, KEY_IMPORT_STRAVA_IN_PROGRESS,
    KEY_IMPORT_STRAVA_PROGRESS_STARTING, KEY_IMPORT_STRAVA_STARTING,
};
use pierre_core::constants::oauth_providers::STRAVA_ARCHIVE;
use pierre_core::html::{escape_html_attribute, with_hosted_page_css};
use pierre_middleware::provider_link_token::mint_import_link_token;
use secrecy::ExposeSecret;
use serde_json::{json, Map, Value};
use tracing::{error, info, warn};
use urlencoding::encode;
use uuid::Uuid;

use crate::connect_hosted::{validate_connect_token, ConnectPageQuery};
use crate::connect_hosted_templates::render_connect_error_page;
use crate::hosted_page::{locale_for_reader, script_json, PageStrings};
use crate::short_link::preferred_locale;
use crate::AuthRoutesContext;

const IMPORT_TEMPLATE: &str = include_str!("../templates/connect_import.html");

/// Strava's page where an athlete requests their account archive.
const STRAVA_EXPORT_PAGE: &str = "https://www.strava.com/athlete/delete_your_account";

/// The link-error page in `locale`, worded by the catalogue's `message_key`.
fn error_page(resources: &AuthRoutesContext, locale: &str, message_key: &str) -> Response {
    let strings = PageStrings::new(&resources.messaging_strings, locale);
    Html(render_connect_error_page(&strings, message_key)).into_response()
}

/// GET `/providers/connect/import?token=...` — the hosted import page.
pub(crate) async fn handle_connect_import_page(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Query(query): Query<ConnectPageQuery>,
) -> Response {
    let browser_locale = preferred_locale(&headers);
    let Some(token) = query.token.as_ref().map(ExposeSecret::expose_secret) else {
        return error_page(&resources, browser_locale, KEY_HOSTED_ERROR_MISSING_TOKEN);
    };
    let claims = match validate_connect_token(&resources, token) {
        Ok(claims) => claims,
        Err(e) => {
            warn!(error = %e, "Rejected hosted import page: invalid link-token");
            return error_page(&resources, browser_locale, KEY_HOSTED_ERROR_INVALID_LINK);
        }
    };
    let (Ok(user_id), Ok(tenant_id)) = (Uuid::parse_str(&claims.sub), Uuid::parse_str(&claims.tid))
    else {
        return error_page(&resources, browser_locale, KEY_HOSTED_ERROR_MALFORMED_LINK);
    };

    let locale = locale_for_reader(&resources, user_id, Some(tenant_id), &claims.channel).await;
    let strings = PageStrings::new(&resources.messaging_strings, locale);
    let import_token = match mint_import_link_token(
        &claims,
        STRAVA_ARCHIVE,
        &resources.admin_jwt_secret,
    ) {
        Ok(token) => token,
        Err(e) => {
            error!(user_id = %user_id, error = %e, "Could not mint the hosted import page's link-token");
            return error_page(&resources, strings.locale(), KEY_HOSTED_ERROR_GENERIC);
        }
    };

    info!(
        user_id = %user_id,
        channel = %claims.channel,
        enabled = resources.archive_imports_offered,
        locale = strings.locale(),
        "Rendered hosted import page"
    );
    Html(render_import_page(
        &strings,
        &ImportPage {
            connect_token: token,
            import_token: &import_token,
            channel: &claims.channel,
            enabled: resources.archive_imports_offered,
        },
    ))
    .into_response()
}

/// What one rendering of the import page carries.
struct ImportPage<'a> {
    /// The connect token the picker was opened with: the Back link returns
    /// to the picker with it.
    connect_token: &'a str,
    /// The import token the page's script calls the import routes with.
    import_token: &'a str,
    /// The channel the link was sent to.
    channel: &'a str,
    /// Whether imports run here.
    enabled: bool,
}

/// `pairs` (code, catalogue key) as a JSON object of each code's text.
fn copy_by_code(strings: &PageStrings<'_>, pairs: &[(&str, &str)]) -> Value {
    Value::Object(
        pairs
            .iter()
            .map(|(code, key)| ((*code).to_owned(), Value::String(strings.get(key))))
            .collect::<Map<_, _>>(),
    )
}

/// Render the hosted import page in the athlete's locale.
///
/// The tokens and channel are escaped into the page; the strings and the
/// refusal and failure copy go into the `<script>` as JSON, so none of them
/// is ever markup.
fn render_import_page(strings: &PageStrings<'_>, page: &ImportPage<'_>) -> String {
    let script_strings = json!({
        "queued": strings.get(KEY_IMPORT_STATE_QUEUED),
        "downloading": strings.get(KEY_IMPORT_STATE_DOWNLOADING),
        "progress": strings.get(KEY_HOSTED_IMPORT_PROGRESS),
        "progressStarting": strings.get(KEY_IMPORT_STRAVA_PROGRESS_STARTING),
        "starting": strings.get(KEY_IMPORT_STRAVA_STARTING),
        "doneTitle": strings.get(KEY_HOSTED_IMPORT_DONE_TITLE),
        "notImported": strings.get(KEY_HOSTED_IMPORT_NOT_IMPORTED),
        "stopped": strings.get(KEY_HOSTED_IMPORT_STOPPED),
        "inProgress": strings.get(KEY_IMPORT_STRAVA_IN_PROGRESS),
        "genericError": strings.get(KEY_IMPORT_STRAVA_GENERIC_ERROR),
        "linkExpired": strings.get(KEY_HOSTED_ERROR_INVALID_LINK),
    });
    let back_url = format!("/providers/connect?token={}", encode(page.connect_token));
    strings
        .fill(&with_hosted_page_css(IMPORT_TEMPLATE))
        .replace(
            "{{LINKED_FROM}}",
            &strings.html(
                KEY_HOSTED_COMMON_LINKED_FROM,
                &[&strings.channel(page.channel)],
            ),
        )
        .replace("{{BACK_URL}}", &escape_html_attribute(&back_url))
        .replace("{{STRAVA_EXPORT_PAGE}}", STRAVA_EXPORT_PAGE)
        .replace(
            "{{IMPORT_TOKEN}}",
            &escape_html_attribute(page.import_token),
        )
        .replace("{{SOURCE}}", STRAVA_ARCHIVE)
        .replace("{{ENABLED}}", if page.enabled { "true" } else { "false" })
        // The JSON goes in last, so an escaped value can never close the
        // script context before it.
        .replace("{{STRINGS_JSON}}", &script_json(&script_strings))
        .replace(
            "{{REFUSALS_JSON}}",
            &script_json(&copy_by_code(strings, &IMPORT_STRAVA_REFUSALS)),
        )
        .replace(
            "{{FAILURES_JSON}}",
            &script_json(&copy_by_code(strings, &IMPORT_STRAVA_FAILURES)),
        )
}
