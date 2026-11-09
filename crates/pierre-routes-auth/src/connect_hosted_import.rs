// ABOUTME: Hosted import page the connect picker opens for a provider's export, authed by the picker's connect link-token
// ABOUTME: Narrows that token into an import link-token for one source; only an offered source whose link is pasted is taken
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The hosted **import page** (carnet#1221).
//!
//! A chat user reaches the hosted connect picker
//! ([`crate::connect_hosted`]) from a connect link-token. Under a card whose
//! provider's export this page imports, the picker offers "Have an export?
//! Import it", which opens
//! `/providers/connect/import?token=<connect token>&source=<source>`: this
//! page. A link with no `source` is a Strava export's, as the picker's links
//! before it named none.
//!
//! The page asks the service for the athlete's export, takes the link the
//! service emails, and follows the import. It takes only a source this
//! server offers ([`AuthRoutesContext::offered_archive_sources`]) whose export
//! arrives as an emailed link ([`HOSTED_IMPORT_SOURCES`]): an export the
//! athlete uploads is imported from Dravr's own apps. It does that with an
//! **import link-token** it mints from the connect token
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
    IMPORT_GARMIN_FAILURES, IMPORT_GARMIN_REFUSALS, IMPORT_STRAVA_FAILURES, IMPORT_STRAVA_REFUSALS,
    KEY_HOSTED_COMMON_LINKED_FROM, KEY_HOSTED_ERROR_GENERIC, KEY_HOSTED_ERROR_INVALID_LINK,
    KEY_HOSTED_ERROR_MALFORMED_LINK, KEY_HOSTED_ERROR_MISSING_TOKEN, KEY_HOSTED_IMPORT_DONE_TITLE,
    KEY_HOSTED_IMPORT_NOT_IMPORTED, KEY_HOSTED_IMPORT_PROGRESS, KEY_HOSTED_IMPORT_STOPPED,
    KEY_IMPORT_GARMIN_INTRO, KEY_IMPORT_GARMIN_IN_PROGRESS, KEY_IMPORT_GARMIN_LINK_HINT,
    KEY_IMPORT_GARMIN_LINK_LABEL, KEY_IMPORT_GARMIN_PROGRESS_STARTING,
    KEY_IMPORT_GARMIN_STEP1_BODY, KEY_IMPORT_GARMIN_STEP1_OPEN, KEY_IMPORT_GARMIN_STEP1_TITLE,
    KEY_IMPORT_GARMIN_STEP2_TITLE, KEY_IMPORT_STATE_AWAITING_LINK, KEY_IMPORT_STATE_DOWNLOADING,
    KEY_IMPORT_STATE_QUEUED, KEY_IMPORT_STRAVA_GENERIC_ERROR, KEY_IMPORT_STRAVA_INTRO,
    KEY_IMPORT_STRAVA_IN_PROGRESS, KEY_IMPORT_STRAVA_LINK_LABEL,
    KEY_IMPORT_STRAVA_PROGRESS_STARTING, KEY_IMPORT_STRAVA_STARTING, KEY_IMPORT_STRAVA_STEP1_BODY,
    KEY_IMPORT_STRAVA_STEP1_OPEN, KEY_IMPORT_STRAVA_STEP1_TITLE, KEY_IMPORT_STRAVA_STEP2_TITLE,
    KEY_IMPORT_UNAVAILABLE,
};
use pierre_core::constants::oauth_providers::{GARMIN_ARCHIVE, STRAVA_ARCHIVE};
use pierre_core::html::{escape_html_attribute, with_hosted_page_css};
use pierre_middleware::provider_link_token::{mint_import_link_token, ProviderLinkTokenClaims};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tracing::{error, info, warn};
use urlencoding::encode;
use uuid::Uuid;

use crate::connect_hosted::validate_connect_token;
use crate::connect_hosted_templates::render_connect_error_page;
use crate::hosted_page::{locale_for_reader, script_json, PageStrings};
use crate::short_link::preferred_locale;
use crate::AuthRoutesContext;

const IMPORT_TEMPLATE: &str = include_str!("../templates/connect_import.html");

/// Query parameters for the hosted import page.
#[derive(Debug, Deserialize)]
pub(crate) struct ConnectImportQuery {
    /// The connect link-token the picker was opened with.
    pub token: Option<SecretString>,
    /// The import source (`strava_archive`, `garmin_archive`). Absent, the
    /// page is the Strava export's.
    pub source: Option<String>,
}

/// The copy of one source the hosted import page takes: where the service's
/// export is requested, and the keys of each line that names the service.
/// The steps that name no service are the Strava page's generic wording.
#[derive(Debug)]
pub(crate) struct HostedImportSource {
    /// The import source this page imports.
    source: &'static str,
    /// The service's page where an athlete requests their export.
    export_page: &'static str,
    intro: &'static str,
    step1_title: &'static str,
    step1_body: &'static str,
    step1_open: &'static str,
    step2_title: &'static str,
    link_label: &'static str,
    /// A hint under the link field, for a service whose email needs one.
    link_hint: Option<&'static str>,
    /// The line while an import waits for its link, for a service whose
    /// wording has one.
    awaiting_link: Option<&'static str>,
    progress_starting: &'static str,
    in_progress: &'static str,
    refusals: &'static [(&'static str, &'static str)],
    failures: &'static [(&'static str, &'static str)],
}

/// The sources whose export arrives as an emailed link, which this page
/// takes: Strava's and Garmin's. An export the athlete uploads (Apple Health,
/// COROS, WHOOP) has no link to paste here.
pub(crate) const HOSTED_IMPORT_SOURCES: [HostedImportSource; 2] = [
    HostedImportSource {
        source: STRAVA_ARCHIVE,
        export_page: "https://www.strava.com/athlete/delete_your_account",
        intro: KEY_IMPORT_STRAVA_INTRO,
        step1_title: KEY_IMPORT_STRAVA_STEP1_TITLE,
        step1_body: KEY_IMPORT_STRAVA_STEP1_BODY,
        step1_open: KEY_IMPORT_STRAVA_STEP1_OPEN,
        step2_title: KEY_IMPORT_STRAVA_STEP2_TITLE,
        link_label: KEY_IMPORT_STRAVA_LINK_LABEL,
        link_hint: None,
        awaiting_link: Some(KEY_IMPORT_STATE_AWAITING_LINK),
        progress_starting: KEY_IMPORT_STRAVA_PROGRESS_STARTING,
        in_progress: KEY_IMPORT_STRAVA_IN_PROGRESS,
        refusals: &IMPORT_STRAVA_REFUSALS,
        failures: &IMPORT_STRAVA_FAILURES,
    },
    HostedImportSource {
        source: GARMIN_ARCHIVE,
        export_page: "https://www.garmin.com/account/datamanagement/exportdata/",
        intro: KEY_IMPORT_GARMIN_INTRO,
        step1_title: KEY_IMPORT_GARMIN_STEP1_TITLE,
        step1_body: KEY_IMPORT_GARMIN_STEP1_BODY,
        step1_open: KEY_IMPORT_GARMIN_STEP1_OPEN,
        step2_title: KEY_IMPORT_GARMIN_STEP2_TITLE,
        link_label: KEY_IMPORT_GARMIN_LINK_LABEL,
        link_hint: Some(KEY_IMPORT_GARMIN_LINK_HINT),
        awaiting_link: None,
        progress_starting: KEY_IMPORT_GARMIN_PROGRESS_STARTING,
        in_progress: KEY_IMPORT_GARMIN_IN_PROGRESS,
        refusals: &IMPORT_GARMIN_REFUSALS,
        failures: &IMPORT_GARMIN_FAILURES,
    },
];

/// The hosted import copy of `source` among `offered`, the sources this
/// server imports; `None` for a source it does not offer, or whose export
/// this page cannot take.
fn source_among(offered: &[&str], source: &str) -> Option<&'static HostedImportSource> {
    if !offered.contains(&source) {
        return None;
    }
    HOSTED_IMPORT_SOURCES
        .iter()
        .find(|hosted| hosted.source == source)
}

/// The source this server imports on the hosted page as `source`, if any:
/// one it offers whose export arrives as an emailed link.
pub(crate) fn hosted_import_source(
    resources: &AuthRoutesContext,
    source: &str,
) -> Option<&'static str> {
    source_among(&resources.offered_archive_sources, source).map(|hosted| hosted.source)
}

/// The link-error page in `locale`, worded by the catalogue's `message_key`.
fn error_page(resources: &AuthRoutesContext, locale: &str, message_key: &str) -> Response {
    let strings = PageStrings::new(&resources.messaging_strings, locale);
    Html(render_connect_error_page(&strings, message_key)).into_response()
}

/// GET `/providers/connect/import?token=...` — the hosted import page.
pub(crate) async fn handle_connect_import_page(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Query(query): Query<ConnectImportQuery>,
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
    let requested = query.source.as_deref().unwrap_or(STRAVA_ARCHIVE);
    match import_page_for(&resources, &claims, requested) {
        Ok((source, import_token)) => {
            info!(
                user_id = %user_id,
                channel = %claims.channel,
                source = source.source,
                locale = strings.locale(),
                "Rendered hosted import page"
            );
            Html(render_import_page(
                &strings,
                &ImportPage {
                    connect_token: token,
                    import_token: &import_token,
                    channel: &claims.channel,
                    source,
                },
            ))
            .into_response()
        }
        Err(message_key) => error_page(&resources, strings.locale(), message_key),
    }
}

/// The copy of `requested` and the import token minted for it from the
/// connect token's `claims`, or the catalogue key of the page that refuses
/// it: a source this page does not import, or a token that cannot be minted.
fn import_page_for(
    resources: &AuthRoutesContext,
    claims: &ProviderLinkTokenClaims,
    requested: &str,
) -> Result<(&'static HostedImportSource, String), &'static str> {
    let Some(source) = source_among(&resources.offered_archive_sources, requested) else {
        info!(user = %claims.sub, source = requested, "Hosted import page: the source is not imported here");
        return Err(KEY_IMPORT_UNAVAILABLE);
    };
    mint_import_link_token(claims, source.source, &resources.admin_jwt_secret)
        .map(|token| (source, token))
        .map_err(|e| {
            error!(user = %claims.sub, error = %e, "Could not mint the hosted import page's link-token");
            KEY_HOSTED_ERROR_GENERIC
        })
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
    /// The source imported, with its copy. The page is rendered only for a
    /// source this server offers, so imports run here.
    source: &'static HostedImportSource,
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

/// Render the hosted import page in the athlete's locale, in its source's
/// copy.
///
/// The tokens and channel are escaped into the page, as is every line of
/// copy; the strings the script sets and the refusal and failure copy go into
/// the `<script>` as JSON, so none of them is ever markup.
fn render_import_page(strings: &PageStrings<'_>, page: &ImportPage<'_>) -> String {
    let source = page.source;
    let script_strings = json!({
        "queued": strings.get(KEY_IMPORT_STATE_QUEUED),
        "downloading": strings.get(KEY_IMPORT_STATE_DOWNLOADING),
        "progress": strings.get(KEY_HOSTED_IMPORT_PROGRESS),
        "progressStarting": strings.get(source.progress_starting),
        "starting": strings.get(KEY_IMPORT_STRAVA_STARTING),
        "doneTitle": strings.get(KEY_HOSTED_IMPORT_DONE_TITLE),
        "notImported": strings.get(KEY_HOSTED_IMPORT_NOT_IMPORTED),
        "stopped": strings.get(KEY_HOSTED_IMPORT_STOPPED),
        "inProgress": strings.get(source.in_progress),
        "genericError": strings.get(KEY_IMPORT_STRAVA_GENERIC_ERROR),
        "linkExpired": strings.get(KEY_HOSTED_ERROR_INVALID_LINK),
    });
    let back_url = format!("/providers/connect?token={}", encode(page.connect_token));
    let line = |key: Option<&str>| key.map_or_else(String::new, |key| strings.html(key, &[]));
    strings
        .fill(&with_hosted_page_css(IMPORT_TEMPLATE))
        .replace("{{INTRO}}", &strings.html(source.intro, &[]))
        .replace("{{STEP1_TITLE}}", &strings.html(source.step1_title, &[]))
        .replace("{{STEP1_BODY}}", &strings.html(source.step1_body, &[]))
        .replace("{{STEP1_OPEN}}", &strings.html(source.step1_open, &[]))
        .replace("{{STEP2_TITLE}}", &strings.html(source.step2_title, &[]))
        .replace("{{LINK_LABEL}}", &strings.html(source.link_label, &[]))
        .replace("{{LINK_HINT}}", &line(source.link_hint))
        .replace("{{AWAITING_LINE}}", &line(source.awaiting_link))
        .replace(
            "{{LINKED_FROM}}",
            &strings.html(
                KEY_HOSTED_COMMON_LINKED_FROM,
                &[&strings.channel(page.channel)],
            ),
        )
        .replace("{{BACK_URL}}", &escape_html_attribute(&back_url))
        .replace("{{EXPORT_PAGE}}", source.export_page)
        .replace(
            "{{IMPORT_TOKEN}}",
            &escape_html_attribute(page.import_token),
        )
        .replace("{{SOURCE}}", source.source)
        // The JSON goes in last, so an escaped value can never close the
        // script context before it.
        .replace("{{STRINGS_JSON}}", &script_json(&script_strings))
        .replace(
            "{{REFUSALS_JSON}}",
            &script_json(&copy_by_code(strings, source.refusals)),
        )
        .replace(
            "{{FAILURES_JSON}}",
            &script_json(&copy_by_code(strings, source.failures)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_contremaitre::hosted_strings::{
        KEY_IMPORT_GARMIN_FAILURE_NOT_AN_EXPORT, KEY_IMPORT_GARMIN_REFUSAL_LINK_HOST_NOT_ALLOWED,
    };

    const OFFERED: [&str; 3] = [STRAVA_ARCHIVE, GARMIN_ARCHIVE, "coros_archive"];

    #[test]
    fn an_offered_link_source_is_taken_and_any_other_is_not() {
        assert_eq!(
            source_among(&OFFERED, STRAVA_ARCHIVE).map(|s| s.source),
            Some(STRAVA_ARCHIVE),
            "bug: Strava's export is imported here"
        );
        let garmin =
            source_among(&OFFERED, GARMIN_ARCHIVE).expect("Garmin's export is imported here");
        assert_eq!(
            garmin.source, GARMIN_ARCHIVE,
            "bug: the Garmin copy imports the Garmin source"
        );
        assert_eq!(
            garmin.export_page, "https://www.garmin.com/account/datamanagement/exportdata/",
            "bug: the Garmin page sends the athlete to Garmin's export request"
        );
        assert_eq!(
            garmin.intro, KEY_IMPORT_GARMIN_INTRO,
            "bug: the Garmin page speaks of Garmin"
        );
        assert!(
            source_among(&OFFERED, "coros_archive").is_none(),
            "bug: an uploaded export has no link to paste here"
        );
        assert!(
            source_among(&[STRAVA_ARCHIVE], GARMIN_ARCHIVE).is_none(),
            "bug: a source this server does not offer is never taken"
        );
        assert!(
            source_among(&OFFERED, "nonsense").is_none(),
            "bug: an unknown source is never taken"
        );
    }

    #[test]
    fn garmin_refusals_name_garmin_where_they_name_the_service() {
        let garmin = source_among(&OFFERED, GARMIN_ARCHIVE).unwrap();
        let refusal = |code: &str| {
            garmin
                .refusals
                .iter()
                .find(|(c, _)| *c == code)
                .map(|(_, key)| *key)
        };
        assert_eq!(
            refusal("link_host_not_allowed"),
            Some(KEY_IMPORT_GARMIN_REFUSAL_LINK_HOST_NOT_ALLOWED),
            "bug: a link off Garmin's hosts is refused in Garmin's words"
        );
        assert!(
            garmin
                .failures
                .iter()
                .any(|(code, key)| *code == "not_an_export"
                    && *key == KEY_IMPORT_GARMIN_FAILURE_NOT_AN_EXPORT),
            "bug: a file that is no Garmin export is named so"
        );
    }
}
