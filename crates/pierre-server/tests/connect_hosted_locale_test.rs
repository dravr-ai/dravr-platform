// ABOUTME: Integration tests for the language of the hosted connect pages a chat link opens
// ABOUTME: Pins that each page is written in the token user's stored locale, from the catalogue, in all five
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `/connect` in a chat answers in the athlete's language and hands them a
//! link. The pages behind it — the provider picker, the Intervals.icu API-key
//! form, the hosted provider login, and the success and link-error pages —
//! are written in that same language:
//!
//! - the locale is the stored one of the user the link-token names, never a
//!   query parameter or a header the browser sends;
//! - every string is the dravr-contremaitre catalogue's, so each assertion
//!   here compares the page against the catalogue lookup, with one literal
//!   per locale so a catalogue that returned the wrong language would fail;
//! - the stored locale is the one the chat itself answers in: a `/language`
//!   override on the channel the link was sent to comes before the profile;
//! - a failed credential sign-in is worded by the page from the catalogue,
//!   never by the server's or the provider's own text;
//! - a page with no verifiable token follows `Accept-Language`.

mod common;
mod helpers;

use std::sync::Arc;

use helpers::axum_test::AxumTestRequest;
use pierre_contremaitre::hosted_strings::{
    KEY_HOSTED_COMMON_ACCOUNT_TITLE, KEY_HOSTED_COMMON_BACK, KEY_HOSTED_COMMON_CREDENTIALS_NOTE,
    KEY_HOSTED_COMMON_EMAIL_LABEL, KEY_HOSTED_COMMON_LINKED_FROM, KEY_HOSTED_COMMON_LOG_IN,
    KEY_HOSTED_COMMON_OTP_LABEL_NAMED, KEY_HOSTED_COMMON_PASSWORD_LABEL,
    KEY_HOSTED_COMMON_SIGN_IN_EXPIRED, KEY_HOSTED_COMMON_SIGN_IN_REJECTED,
    KEY_HOSTED_COMMON_SIGN_IN_UNAVAILABLE, KEY_HOSTED_COMMON_SUBTITLE,
    KEY_HOSTED_COMMON_SUBTITLE_NAMED, KEY_HOSTED_COMMON_YOUR_CHAT_APP, KEY_HOSTED_ERROR_HEADING,
    KEY_HOSTED_ERROR_INVALID_LINK, KEY_HOSTED_ERROR_MISSING_TOKEN,
    KEY_HOSTED_ERROR_OAUTH_START_FAILED, KEY_HOSTED_ERROR_PAGE_TITLE,
    KEY_HOSTED_INTERVALS_API_KEY_REQUIRED, KEY_HOSTED_INTERVALS_ATHLETE_ID_REQUIRED,
    KEY_HOSTED_INTERVALS_PAGE_TITLE, KEY_HOSTED_INTERVALS_STORAGE_NOTE,
    KEY_HOSTED_PICKER_DATA_AVAILABLE, KEY_HOSTED_PICKER_HEADING, KEY_HOSTED_PICKER_PAGE_TITLE,
    KEY_HOSTED_PICKER_SECURITY_NOTE, KEY_HOSTED_PICKER_TAG_API_KEY,
    KEY_HOSTED_PICKER_TAG_CONNECTED, KEY_HOSTED_SUCCESS_AUTO_CLOSE,
    KEY_HOSTED_SUCCESS_DATA_AVAILABLE, KEY_HOSTED_SUCCESS_HEADING,
    KEY_HOSTED_SUCCESS_HEADING_GENERIC, KEY_HOSTED_SUCCESS_RETURN_TO_CHAT,
    KEY_INTERVALS_API_KEY_LABEL, KEY_INTERVALS_ATHLETE_ID, KEY_INTERVALS_CONNECT_ACTION,
    KEY_INTERVALS_CREDENTIALS_HELP, KEY_SCIOTTE_CODE_REJECTED, TEMPLATE_KEYS,
};
use pierre_contremaitre::messaging_strings::{
    DEFAULT_LOCALE, KEY_PROVIDER_DESCRIPTION_TRAININGPEAKS,
};
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::html::escape_html_attribute;
use pierre_core::models::{TenantId, SUPPORTED_LOCALES};
use pierre_database::backends::CreateChannelLinkParams;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::{
    mint_connect_link_token, mint_link_token, MintProviderLinkTokenArgs,
};
use pierre_routes_auth::{AuthRoutes, CODE_REJECTED_REASON, LOGIN_FLOW_EXPIRED_REASON};
use pierre_services::locale::resolve_channel_locale;
use serde_json::{json, Value};
use uuid::Uuid;

/// One sentence per locale that only that locale's catalogue carries: the
/// picker's heading. A registry that answered every locale in one language
/// would satisfy the catalogue comparisons and fail here.
const PICKER_HEADINGS: [(&str, &str); 5] = [
    ("fr", "Choisis un service"),
    ("en", "Choose a provider"),
    ("es", "Elige un servicio"),
    ("de", "Wähle einen Dienst"),
    ("pt", "Escolhe um serviço"),
];

/// The catalogue as a page in one locale reads it, escaped as the page is.
struct Catalogue {
    registry: MessagingStringsRegistry,
    locale: &'static str,
}

impl Catalogue {
    fn new(locale: &'static str) -> Self {
        Self {
            registry: MessagingStringsRegistry::new(),
            locale,
        }
    }

    /// The raw text of `key`, which must exist in this locale.
    fn text(&self, key: &str, args: &[&str]) -> String {
        let text = self.registry.render(key, self.locale, args);
        assert!(!text.is_empty(), "{}: the catalogue has {key}", self.locale);
        text
    }

    /// The text of `key` as it lands in the page's markup.
    fn html(&self, key: &str, args: &[&str]) -> String {
        escape_html_attribute(&self.text(key, args))
    }
}

struct Fixture {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
}

/// A server and one athlete whose stored locale is `locale`.
async fn athlete(locale: &str) -> Fixture {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    resources
        .common
        .repos
        .users
        .update_locale(user_id, locale)
        .await
        .expect("store the athlete's locale");
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id;
    Fixture {
        resources,
        user_id,
        tenant_id,
    }
}

impl Fixture {
    fn connect_token(&self) -> String {
        mint_connect_link_token(
            self.user_id,
            self.tenant_id.as_uuid(),
            "telegram",
            None,
            &self.resources.auth.admin_jwt_secret,
        )
        .expect("mint connect token")
    }

    fn login_token(&self, target: &str) -> String {
        mint_link_token(
            &MintProviderLinkTokenArgs {
                user_id: self.user_id,
                tenant_id: self.tenant_id.as_uuid(),
                provider: "sciotte",
                target,
                channel: "telegram",
                channel_thread: None,
            },
            &self.resources.auth.admin_jwt_secret,
        )
        .expect("mint hosted-login token")
    }

    async fn get(&self, url: &str, accept_language: Option<&str>) -> String {
        let mut request = AxumTestRequest::get(url);
        if let Some(value) = accept_language {
            request = request.header("accept-language", value);
        }
        request
            .send(AuthRoutes::routes(self.resources.auth_routes_context()))
            .await
            .text()
    }
}

/// The page's markup, without the script that carries slot templates for the
/// browser to fill.
fn markup(body: &str) -> &str {
    body.split("<script>").next().unwrap_or(body)
}

/// Nothing the renderer should have replaced reached the athlete: no template
/// placeholder, no unfilled positional slot in the markup, no catalogue key
/// printed where its text belongs.
fn assert_fully_rendered(body: &str, what: &str) {
    assert!(!body.contains("{{"), "{what}: a placeholder survived");
    let markup = markup(body);
    for slot in ["{0}", "{1}"] {
        assert!(!markup.contains(slot), "{what}: an unfilled {slot} slot");
    }
    for key in TEMPLATE_KEYS {
        assert!(!body.contains(key), "{what}: the raw key {key} is printed");
    }
    for prefix in ["hosted.", "shell.intervals", "Notice.title"] {
        assert!(!body.contains(prefix), "{what}: a raw {prefix}* key leaked");
    }
}

fn assert_lang(body: &str, locale: &str, what: &str) {
    assert!(
        body.contains(&format!(r#"<html lang="{locale}">"#)),
        "{what} declares lang={locale}"
    );
}

/// The object the picker's script reads its strings from.
fn script_strings(body: &str) -> Value {
    let start = body.find("strings: ").expect("the page embeds its strings") + "strings: ".len();
    let json = &body[start..];
    let end = json.find(",\n").expect("the strings end their line");
    serde_json::from_str(&json[..end]).expect("the embedded strings are JSON")
}

/// The cards the picker embeds.
fn picker_cards(body: &str) -> Vec<Value> {
    let start = body.find("providers: ").expect("the page embeds its cards") + "providers: ".len();
    let json = &body[start..];
    let end = json.find(",\n").expect("the card list ends its line");
    serde_json::from_str(&json[..end]).expect("the embedded cards are JSON")
}

// ============================================================================
// The picker
// ============================================================================

#[tokio::test]
async fn the_picker_is_written_in_the_athletes_locale_in_all_five() {
    assert_eq!(
        PICKER_HEADINGS.map(|(locale, _)| locale),
        SUPPORTED_LOCALES,
        "one literal per supported locale"
    );
    for (locale, heading) in PICKER_HEADINGS {
        let fixture = athlete(locale).await;
        let catalogue = Catalogue::new(locale);
        let body = fixture
            .get(
                &format!("/providers/connect?token={}", fixture.connect_token()),
                None,
            )
            .await;

        assert_lang(&body, locale, "the picker");
        assert_fully_rendered(&body, &format!("{locale} picker"));
        // The literal: this locale's own sentence, as the heading.
        assert_eq!(catalogue.text(KEY_HOSTED_PICKER_HEADING, &[]), heading);
        assert!(
            body.contains(&format!("<h1>{}</h1>", escape_html_attribute(heading))),
            "{locale}: the picker's heading reads {heading:?}"
        );
        for (key, args) in [
            (KEY_HOSTED_PICKER_PAGE_TITLE, &[][..]),
            (KEY_HOSTED_COMMON_SUBTITLE, &[]),
            (KEY_HOSTED_COMMON_LINKED_FROM, &["Telegram"]),
            (KEY_HOSTED_PICKER_SECURITY_NOTE, &[]),
            (KEY_HOSTED_COMMON_BACK, &[]),
            (KEY_HOSTED_COMMON_PASSWORD_LABEL, &[]),
            (KEY_HOSTED_COMMON_LOG_IN, &[]),
            (KEY_HOSTED_PICKER_DATA_AVAILABLE, &["Telegram"]),
        ] {
            assert!(
                markup(&body).contains(&catalogue.html(key, args)),
                "{locale}: the picker shows {key}"
            );
        }

        // What the script writes arrives as data, in the same locale; a slot
        // it fills itself is still a slot.
        let strings = script_strings(&body);
        assert_eq!(
            strings["tagConnected"],
            catalogue.text(KEY_HOSTED_PICKER_TAG_CONNECTED, &[])
        );
        assert_eq!(
            strings["tagApiKey"],
            catalogue.text(KEY_HOSTED_PICKER_TAG_API_KEY, &[])
        );
        assert_eq!(
            strings["email"],
            catalogue.text(KEY_HOSTED_COMMON_EMAIL_LABEL, &[])
        );
        assert_eq!(
            strings["accountTitle"],
            catalogue.text(KEY_HOSTED_COMMON_ACCOUNT_TITLE, &[])
        );
        assert_eq!(
            strings["credentialsNote"],
            catalogue.text(KEY_HOSTED_COMMON_CREDENTIALS_NOTE, &[])
        );
        assert!(
            strings["accountTitle"].as_str().unwrap().contains("{0}"),
            "{locale}: the account title keeps the slot the script fills"
        );
        for (name, text) in strings.as_object().expect("an object") {
            assert!(
                text.as_str().is_some_and(|text| !text.is_empty()),
                "{locale}: script string {name} is empty"
            );
        }

        // A card's notice is the catalogue's, in the athlete's language.
        let cards = picker_cards(&body);
        let trainingpeaks = cards
            .iter()
            .find(|card| card["target"] == "trainingpeaks")
            .unwrap_or_else(|| panic!("{locale}: the picker offers TrainingPeaks"));
        assert_eq!(
            trainingpeaks["notice"]["title"],
            catalogue.text("providers.trainingpeaksNotice.title", &[]),
            "{locale}: the TrainingPeaks notice is translated"
        );
        assert_eq!(
            trainingpeaks["notice"]["consent"],
            catalogue.text("providers.trainingpeaksNotice.consent", &[])
        );
        // So is the line under the card's name.
        assert_eq!(
            trainingpeaks["description"],
            catalogue.text(KEY_PROVIDER_DESCRIPTION_TRAININGPEAKS, &[]),
            "{locale}: the TrainingPeaks description is translated"
        );
    }
}

/// The line under a card's name on the picker `body`, for `target`.
fn card_description(body: &str, target: &str) -> String {
    picker_cards(body)
        .iter()
        .find(|card| card["target"] == target)
        .and_then(|card| card["description"].as_str())
        .unwrap_or_else(|| panic!("the picker describes {target}"))
        .to_owned()
}

/// The locale is the stored one: a browser asking for another language, or a
/// link carrying one, changes nothing on a page that knows its athlete.
#[tokio::test]
async fn a_stated_language_never_overrides_the_athletes_stored_locale() {
    let fixture = athlete("de").await;
    let german = Catalogue::new("de");
    let token = fixture.connect_token();

    for url in [
        format!("/providers/connect?token={token}"),
        format!("/providers/connect?token={token}&lang=en&locale=en"),
        format!("/providers/connect/intervals_icu?token={token}&lang=en"),
        format!("/providers/connect/success?channel=telegram&target=strava&token={token}&lang=en"),
    ] {
        let body = fixture.get(&url, Some("en-US,en;q=0.9")).await;
        assert_lang(&body, "de", &url);
        assert!(
            body.contains(&german.html(KEY_HOSTED_COMMON_SUBTITLE, &[]))
                || body.contains(&german.html(KEY_HOSTED_SUCCESS_AUTO_CLOSE, &[])),
            "{url} stays German: {body}"
        );
    }
}

// ============================================================================
// The Intervals.icu form
// ============================================================================

#[tokio::test]
async fn the_intervals_form_is_written_in_the_athletes_locale_in_all_five() {
    for locale in SUPPORTED_LOCALES {
        let fixture = athlete(locale).await;
        let catalogue = Catalogue::new(locale);
        let token = fixture.connect_token();
        let body = fixture
            .get(
                &format!("/providers/connect/intervals_icu?token={token}"),
                None,
            )
            .await;

        assert_lang(&body, locale, "the form");
        assert_fully_rendered(&body, &format!("{locale} form"));
        for (key, args) in [
            (KEY_HOSTED_INTERVALS_PAGE_TITLE, &[][..]),
            (KEY_HOSTED_COMMON_SUBTITLE, &[]),
            (KEY_HOSTED_COMMON_LINKED_FROM, &["Telegram"]),
            (KEY_HOSTED_COMMON_BACK, &[]),
            (KEY_HOSTED_COMMON_ACCOUNT_TITLE, &["Intervals.icu"]),
            (KEY_INTERVALS_CREDENTIALS_HELP, &[]),
            (KEY_HOSTED_INTERVALS_STORAGE_NOTE, &[]),
        ] {
            assert!(
                body.contains(&catalogue.html(key, args)),
                "{locale}: the form shows {key}"
            );
        }
        // The fields and the button carry the labels the app dialog shows.
        assert!(body.contains(&format!(
            r#"<label for="athlete_id">{}</label>"#,
            catalogue.html(KEY_INTERVALS_ATHLETE_ID, &[])
        )));
        assert!(body.contains(&format!(
            r#"<label for="api_key">{}</label>"#,
            catalogue.html(KEY_INTERVALS_API_KEY_LABEL, &[])
        )));
        assert!(body.contains(&format!(
            r#"id="submit-api-key">{}</button>"#,
            catalogue.html(KEY_INTERVALS_CONNECT_ACTION, &[])
        )));
        // Where to find them names Intervals.icu's own section, verbatim, in
        // every language.
        assert!(
            catalogue
                .text(KEY_INTERVALS_CREDENTIALS_HELP, &[])
                .contains("Developer Settings"),
            "{locale}: the instruction keeps Intervals.icu's own label"
        );
        assert!(body.contains("Developer Settings"), "{locale}");

        // A validation failure is worded in the same language, in the banner.
        for (athlete_id, api_key, reason) in [
            ("i123456", "  ", KEY_HOSTED_INTERVALS_API_KEY_REQUIRED),
            ("", "a-key", KEY_HOSTED_INTERVALS_ATHLETE_ID_REQUIRED),
        ] {
            let resp = AxumTestRequest::post("/providers/connect/intervals_icu")
                .form(&[
                    ("token", token.as_str()),
                    ("athlete_id", athlete_id),
                    ("api_key", api_key),
                ])
                .send(AuthRoutes::routes(fixture.resources.auth_routes_context()))
                .await;
            assert_eq!(resp.status(), 400, "{locale}: {reason} is a bad request");
            let refused = resp.text();
            assert_lang(&refused, locale, "the refused form");
            assert_fully_rendered(&refused, &format!("{locale} refused form"));
            assert!(
                refused.contains(&format!(
                    r#"class="alert alert-error" role="alert">{}</div>"#,
                    catalogue.html(reason, &[])
                )),
                "{locale}: the banner words {reason}: {refused}"
            );
            assert!(
                !refused.contains("is required"),
                "{locale}: the English API error text is not shown"
            );
        }
    }
}

#[tokio::test]
async fn the_form_literals_read_in_french_and_german() {
    for (locale, athlete_id_missing, back) in [
        ("fr", "Saisis ton identifiant athlète.", "&larr; Retour"),
        ("de", "Gib deine Athleten-ID ein.", "&larr; Zurück"),
    ] {
        let fixture = athlete(locale).await;
        let token = fixture.connect_token();
        let refused = AxumTestRequest::post("/providers/connect/intervals_icu")
            .form(&[
                ("token", token.as_str()),
                ("athlete_id", ""),
                ("api_key", "a-key"),
            ])
            .send(AuthRoutes::routes(fixture.resources.auth_routes_context()))
            .await
            .text();
        assert!(refused.contains(athlete_id_missing), "{locale}: {refused}");
        assert!(refused.contains(back), "{locale}: {refused}");
    }
}

// ============================================================================
// Success, with and without the token
// ============================================================================

#[tokio::test]
async fn the_success_page_follows_the_token_then_the_browser() {
    for locale in SUPPORTED_LOCALES {
        let fixture = athlete(locale).await;
        let catalogue = Catalogue::new(locale);
        let token = fixture.connect_token();

        // With the connect token the page is the athlete's, whatever the
        // browser asks for.
        let body = fixture
            .get(
                &format!(
                    "/providers/connect/success?channel=telegram&target=intervals_icu&token={token}"
                ),
                Some("pt-BR"),
            )
            .await;
        assert_lang(&body, locale, "the success page");
        assert_fully_rendered(&body, &format!("{locale} success"));
        for (key, args) in [
            (KEY_HOSTED_SUCCESS_HEADING, &["Intervals.icu"][..]),
            (KEY_HOSTED_SUCCESS_DATA_AVAILABLE, &["Intervals.icu"]),
            (KEY_HOSTED_SUCCESS_RETURN_TO_CHAT, &["Telegram"]),
            (KEY_HOSTED_SUCCESS_AUTO_CLOSE, &[]),
        ] {
            assert!(
                body.contains(&catalogue.html(key, args)),
                "{locale}: the success page shows {key}"
            );
        }

        // The hosted login's own success page reads its narrower token.
        let login = fixture.login_token("strava");
        let body = fixture
            .get(
                &format!("/providers/sciotte/success?channel=telegram&target=strava&token={login}"),
                Some("pt-BR"),
            )
            .await;
        assert_lang(&body, locale, "the hosted-login success page");
        assert!(body.contains(&catalogue.html(KEY_HOSTED_SUCCESS_HEADING, &["Strava"])));
    }

    // Without a token there is no athlete to ask: the browser's language,
    // French when it names none the platform speaks.
    let fixture = athlete("es").await;
    for (accept_language, locale) in [
        (Some("de-CH,de;q=0.9,en;q=0.8"), "de"),
        (Some("en-GB"), "en"),
        (Some("ja"), "fr"),
        (None, "fr"),
    ] {
        let catalogue = Catalogue::new(locale);
        // A target the registry does not name, and no channel: the unnamed
        // wording, not a made-up label.
        let body = fixture
            .get("/providers/connect/success?target=nope", accept_language)
            .await;
        assert_lang(&body, locale, "the token-less success page");
        assert_fully_rendered(&body, "token-less success");
        assert!(body.contains(&catalogue.html(KEY_HOSTED_SUCCESS_HEADING_GENERIC, &[])));
        let chat_app = catalogue.text(KEY_HOSTED_COMMON_YOUR_CHAT_APP, &[]);
        assert!(body.contains(&catalogue.html(KEY_HOSTED_SUCCESS_RETURN_TO_CHAT, &[&chat_app])));
    }
}

// ============================================================================
// Link errors
// ============================================================================

#[tokio::test]
async fn a_refused_link_is_explained_in_the_browsers_language() {
    let fixture = athlete("en").await;
    for (accept_language, locale) in [
        ("fr-CA,fr;q=0.9", "fr"),
        ("en-US", "en"),
        ("es-ES", "es"),
        ("de-DE", "de"),
        ("pt-PT", "pt"),
    ] {
        let catalogue = Catalogue::new(locale);
        for (url, reason) in [
            ("/providers/connect", KEY_HOSTED_ERROR_MISSING_TOKEN),
            (
                "/providers/connect?token=not-a-jwt",
                KEY_HOSTED_ERROR_INVALID_LINK,
            ),
            (
                "/providers/connect/intervals_icu",
                KEY_HOSTED_ERROR_MISSING_TOKEN,
            ),
            (
                "/providers/connect/intervals_icu?token=not-a-jwt",
                KEY_HOSTED_ERROR_INVALID_LINK,
            ),
            ("/providers/sciotte/login", KEY_HOSTED_ERROR_MISSING_TOKEN),
            (
                "/providers/sciotte/login?token=not-a-jwt",
                KEY_HOSTED_ERROR_INVALID_LINK,
            ),
        ] {
            let body = fixture.get(url, Some(accept_language)).await;
            assert_lang(&body, locale, url);
            assert_fully_rendered(&body, url);
            assert!(
                body.contains(&format!(
                    "<title>{}</title>",
                    catalogue.html(KEY_HOSTED_ERROR_PAGE_TITLE, &[])
                )),
                "{locale} {url}: the title"
            );
            assert!(
                body.contains(&format!(
                    "<h1>{}</h1>",
                    catalogue.html(KEY_HOSTED_ERROR_HEADING, &[])
                )),
                "{locale} {url}: the heading"
            );
            assert!(
                body.contains(&format!("<p>{}</p>", catalogue.html(reason, &[]))),
                "{locale} {url}: the reason {reason}: {body}"
            );
        }
    }

    // One literal, so the German page is German.
    let body = fixture
        .get("/providers/connect?token=not-a-jwt", Some("de"))
        .await;
    assert!(
        body.contains("Dieser Link ist ungültig oder abgelaufen."),
        "{body}"
    );
}

/// A failure the page reports once the token has named its athlete is in the
/// athlete's language, not the browser's.
#[tokio::test]
async fn a_failed_oauth_start_is_explained_in_the_athletes_locale() {
    let fixture = athlete("es").await;
    let spanish = Catalogue::new("es");
    let body = fixture
        .get(
            &format!(
                "/api/providers/connect/oauth-init/no-such-provider?token={}",
                fixture.connect_token()
            ),
            Some("en-US"),
        )
        .await;
    assert_lang(&body, "es", "the refused OAuth start");
    assert!(
        body.contains(&spanish.html(KEY_HOSTED_ERROR_OAUTH_START_FAILED, &[])),
        "{body}"
    );
}

// ============================================================================
// The hosted provider login
// ============================================================================

#[tokio::test]
async fn the_hosted_login_is_written_in_the_athletes_locale_in_all_five() {
    for locale in SUPPORTED_LOCALES {
        let fixture = athlete(locale).await;
        let catalogue = Catalogue::new(locale);
        let body = fixture
            .get(
                &format!(
                    "/providers/sciotte/login?token={}",
                    fixture.login_token("garmin")
                ),
                Some("ja"),
            )
            .await;

        assert_lang(&body, locale, "the hosted login");
        assert_fully_rendered(&body, &format!("{locale} hosted login"));
        for (key, args) in [
            (KEY_HOSTED_COMMON_SUBTITLE_NAMED, &["Garmin"][..]),
            (KEY_HOSTED_COMMON_LINKED_FROM, &["Telegram"]),
            (KEY_HOSTED_COMMON_ACCOUNT_TITLE, &["Garmin"]),
            (KEY_HOSTED_COMMON_CREDENTIALS_NOTE, &["Garmin"]),
            (KEY_HOSTED_COMMON_LOG_IN, &[]),
        ] {
            assert!(
                markup(&body).contains(&catalogue.html(key, args)),
                "{locale}: the hosted login shows {key}"
            );
        }
        assert!(body.contains(&format!(
            r#"<label for="email">{}</label>"#,
            catalogue.html(KEY_HOSTED_COMMON_EMAIL_LABEL, &[])
        )));
        let strings = script_strings(&body);
        for (name, text) in strings.as_object().expect("an object") {
            assert!(
                text.as_str().is_some_and(|text| !text.is_empty()),
                "{locale}: script string {name} is empty"
            );
        }
    }
}

// ============================================================================
// An athlete with no stored preference
// ============================================================================

/// A token naming a user the platform holds no locale for is answered by the
/// resolver's own default — not by the browser, and not by English.
#[tokio::test]
async fn an_athlete_without_a_stored_locale_reads_the_resolvers_default() {
    let fixture = athlete("en").await;
    let nobody = mint_connect_link_token(
        Uuid::new_v4(),
        fixture.tenant_id.as_uuid(),
        "telegram",
        None,
        &fixture.resources.auth.admin_jwt_secret,
    )
    .expect("mint connect token");

    let body = fixture
        .get(
            &format!("/providers/connect/intervals_icu?token={nobody}"),
            Some("en-US"),
        )
        .await;
    assert_lang(&body, DEFAULT_LOCALE, "the form of an unknown athlete");
    assert!(body.contains(&Catalogue::new("fr").html(KEY_HOSTED_COMMON_SUBTITLE, &[])));
}

// ============================================================================
// The channel's own language
// ============================================================================

/// The athlete's id on Telegram, as the chat's own resolver is keyed.
const TELEGRAM_SENDER: &str = "tg-694";

impl Fixture {
    /// Link the athlete's Telegram identity to their account.
    async fn link_telegram(&self) {
        self.resources
            .common
            .repos
            .messaging
            .create_channel_link(&CreateChannelLinkParams {
                id: &Uuid::new_v4().to_string(),
                tenant_id: self.tenant_id,
                user_id: &self.user_id.to_string(),
                channel_type: "telegram",
                channel_user_id: TELEGRAM_SENDER,
                display_name: None,
            })
            .await
            .expect("link the Telegram identity");
    }

    /// What `/language <locale>` typed on Telegram stores; `None` clears it.
    async fn set_telegram_language(&self, locale: Option<&str>) {
        self.resources
            .common
            .repos
            .messaging
            .set_channel_link_locale(
                self.tenant_id,
                &self.user_id.to_string(),
                "telegram",
                locale,
            )
            .await
            .expect("store the channel's language");
    }

    /// The locale the Telegram chat words its `/connect` reply in.
    async fn telegram_chat_locale(&self) -> String {
        let repos = &self.resources.common.repos;
        resolve_channel_locale(
            repos.messaging.as_ref(),
            repos.users.as_ref(),
            self.tenant_id,
            "telegram",
            TELEGRAM_SENDER,
            Some(self.user_id),
        )
        .await
    }

    /// A connect token for a link sent to `channel`.
    fn connect_token_from(&self, channel: &str) -> String {
        mint_connect_link_token(
            self.user_id,
            self.tenant_id.as_uuid(),
            channel,
            None,
            &self.resources.auth.admin_jwt_secret,
        )
        .expect("mint connect token")
    }

    /// Every page a Telegram link opens, each from a fresh token.
    fn telegram_pages(&self) -> [(&'static str, String); 4] {
        let token = self.connect_token();
        [
            ("the picker", format!("/providers/connect?token={token}")),
            (
                "the Intervals.icu form",
                format!("/providers/connect/intervals_icu?token={token}"),
            ),
            (
                "the success page",
                format!("/providers/connect/success?channel=telegram&target=strava&token={token}"),
            ),
            (
                "the hosted login",
                format!(
                    "/providers/sciotte/login?token={}",
                    self.login_token("garmin")
                ),
            ),
        ]
    }
}

/// `/language` on a channel overrides the profile for that channel alone, and
/// the chat's `/connect` reply is worded by it. Every page the link opens
/// reads in the language of the chat that sent it — the override first, the
/// profile once it is cleared — and a link sent to another channel keeps the
/// profile's.
#[tokio::test]
async fn a_channel_language_override_writes_the_pages_the_chat_links_to() {
    let fixture = athlete("fr").await;
    fixture.link_telegram().await;
    fixture.set_telegram_language(Some("de")).await;

    let chat_locale = fixture.telegram_chat_locale().await;
    assert_eq!(
        chat_locale, "de",
        "the chat answers in the channel's language"
    );

    for (what, url) in fixture.telegram_pages() {
        let body = fixture.get(&url, Some("en-US")).await;
        assert_lang(&body, &chat_locale, what);
        assert_fully_rendered(&body, what);
    }

    // The cards' descriptions follow the chat too, not the profile beneath it.
    let german = Catalogue::new("de");
    let french = Catalogue::new("fr");
    let german_line = german.text(KEY_PROVIDER_DESCRIPTION_TRAININGPEAKS, &[]);
    assert_ne!(
        german_line,
        french.text(KEY_PROVIDER_DESCRIPTION_TRAININGPEAKS, &[]),
        "the two languages word the description differently"
    );
    let picker = fixture
        .get(
            &format!("/providers/connect?token={}", fixture.connect_token()),
            None,
        )
        .await;
    assert_eq!(
        card_description(&picker, "trainingpeaks"),
        german_line,
        "the picker's cards are described in the chat's language"
    );

    // The form again, after a refused attempt, stays in the chat's language.
    let token = fixture.connect_token();
    let refused = AxumTestRequest::post("/providers/connect/intervals_icu")
        .form(&[
            ("token", token.as_str()),
            ("athlete_id", ""),
            ("api_key", "a-key"),
        ])
        .send(AuthRoutes::routes(fixture.resources.auth_routes_context()))
        .await
        .text();
    assert_lang(&refused, "de", "the refused form");
    assert!(
        refused.contains(&german.html(KEY_HOSTED_INTERVALS_ATHLETE_ID_REQUIRED, &[])),
        "the refusal is worded in German: {refused}"
    );

    // The override is Telegram's: a link sent to Slack reads the profile.
    let slack = fixture
        .get(
            &format!(
                "/providers/connect?token={}",
                fixture.connect_token_from("slack")
            ),
            None,
        )
        .await;
    assert_lang(&slack, "fr", "a picker linked from another channel");
    assert_eq!(
        card_description(&slack, "trainingpeaks"),
        french.text(KEY_PROVIDER_DESCRIPTION_TRAININGPEAKS, &[]),
        "a picker linked from another channel describes its cards in the profile's language"
    );

    // Cleared, the channel inherits the profile again — chat and pages alike.
    fixture.set_telegram_language(None).await;
    let chat_locale = fixture.telegram_chat_locale().await;
    assert_eq!(chat_locale, "fr");
    for (what, url) in fixture.telegram_pages() {
        assert_lang(&fixture.get(&url, Some("en-US")).await, &chat_locale, what);
    }
}

// ============================================================================
// A failed credential sign-in
// ============================================================================

/// One failure sentence per locale that only that locale's catalogue carries.
const SIGN_IN_EXPIRED: [(&str, &str); 5] = [
    (
        "fr",
        "Cette connexion n’est plus active. Recommence la connexion.",
    ),
    (
        "en",
        "This sign-in is no longer active. Please start the sign-in again.",
    ),
    (
        "es",
        "Este inicio de sesión ya no está activo. Empieza de nuevo.",
    ),
    (
        "de",
        "Diese Anmeldung ist nicht mehr aktiv. Starte die Anmeldung bitte erneut.",
    ),
    (
        "pt",
        "Este início de sessão já não está ativo. Começa de novo.",
    ),
];

/// The page's script, where a failure is worded.
fn script(body: &str) -> &str {
    body.split_once("<script>")
        .expect("the page carries a script")
        .1
}

/// The script words a failure itself: it never prints the `error` or
/// `message` a response carries, and it knows the reason a lapsed sign-in is
/// refused with.
fn assert_failures_are_worded_by_the_page(body: &str, what: &str) {
    let script = script(body);
    for server_text in ["data.error", ".message"] {
        assert!(
            !script.contains(server_text),
            "{what}: the script reads the server's own text ({server_text})"
        );
    }
    assert!(
        script.contains(&format!(
            r#"flowExpiredReason: "{LOGIN_FLOW_EXPIRED_REASON}","#
        )),
        "{what}: the script knows the lapsed-sign-in reason"
    );
    assert!(
        script.contains(&format!(r#"codeRejectedReason: "{CODE_REJECTED_REASON}","#)),
        "{what}: the script knows the refused-code reason"
    );
    for wording in [
        "S.codeRejected",
        "S.signInRejected",
        "S.signInUnavailable",
        "S.signInExpired",
        "S.linkExpired",
    ] {
        assert!(
            script.contains(wording),
            "{what}: the script uses {wording}"
        );
    }
}

/// A code the provider refused keeps the athlete on the code step, so both
/// pages carry the catalogue's sentence for it, and name the provider in the
/// code step's label, in all five locales.
#[tokio::test]
async fn the_code_step_is_worded_in_the_athletes_locale_in_all_five() {
    for locale in SUPPORTED_LOCALES {
        let fixture = athlete(locale).await;
        let catalogue = Catalogue::new(locale);
        let code_rejected = catalogue.text(KEY_SCIOTTE_CODE_REJECTED, &[]);

        let picker = fixture
            .get(
                &format!("/providers/connect?token={}", fixture.connect_token()),
                None,
            )
            .await;
        let strings = script_strings(&picker);
        assert_eq!(strings["codeRejected"], code_rejected, "{locale} picker");
        assert_eq!(
            strings["otpLabel"],
            catalogue.text(KEY_HOSTED_COMMON_OTP_LABEL_NAMED, &[]),
            "{locale} picker"
        );
        assert!(
            strings["otpLabel"]
                .as_str()
                .is_some_and(|text| text.contains("{0}")),
            "{locale}: the picker's code label keeps the slot the script fills"
        );

        let login = fixture
            .get(
                &format!(
                    "/providers/sciotte/login?token={}",
                    fixture.login_token("garmin")
                ),
                None,
            )
            .await;
        assert_eq!(
            script_strings(&login)["codeRejected"],
            code_rejected,
            "{locale} hosted login"
        );
        assert!(
            markup(&login)
                .contains(&catalogue.html(KEY_HOSTED_COMMON_OTP_LABEL_NAMED, &["Garmin"])),
            "{locale}: the hosted login's code label names the provider"
        );
    }
}

/// A provider's refusal and the server's messages are not in the athlete's
/// language, so neither page prints them: each carries the catalogue's own
/// sentence for every way a credential sign-in fails, in all five locales.
#[tokio::test]
async fn a_failed_sign_in_is_worded_in_the_athletes_locale_in_all_five() {
    assert_eq!(
        SIGN_IN_EXPIRED.map(|(locale, _)| locale),
        SUPPORTED_LOCALES,
        "one literal per supported locale"
    );
    for (locale, expired) in SIGN_IN_EXPIRED {
        let fixture = athlete(locale).await;
        let catalogue = Catalogue::new(locale);
        assert_eq!(
            catalogue.text(KEY_HOSTED_COMMON_SIGN_IN_EXPIRED, &[]),
            expired
        );

        // The picker fills the provider into the slot once one is picked.
        let picker = fixture
            .get(
                &format!("/providers/connect?token={}", fixture.connect_token()),
                None,
            )
            .await;
        assert_failures_are_worded_by_the_page(&picker, &format!("{locale} picker"));
        let strings = script_strings(&picker);
        assert_eq!(
            strings["signInRejected"],
            catalogue.text(KEY_HOSTED_COMMON_SIGN_IN_REJECTED, &[])
        );
        assert_eq!(
            strings["signInUnavailable"],
            catalogue.text(KEY_HOSTED_COMMON_SIGN_IN_UNAVAILABLE, &[])
        );
        assert_eq!(strings["signInExpired"], expired);
        assert_eq!(
            strings["linkExpired"],
            catalogue.text(KEY_HOSTED_ERROR_INVALID_LINK, &[])
        );
        for slotted in ["signInRejected", "signInUnavailable"] {
            assert!(
                strings[slotted]
                    .as_str()
                    .is_some_and(|text| text.contains("{0}")),
                "{locale}: {slotted} keeps the slot the script fills with the provider"
            );
        }

        // The hosted login knows its provider, so the sentences name it.
        let login = fixture
            .get(
                &format!(
                    "/providers/sciotte/login?token={}",
                    fixture.login_token("garmin")
                ),
                None,
            )
            .await;
        assert_failures_are_worded_by_the_page(&login, &format!("{locale} hosted login"));
        let strings = script_strings(&login);
        assert_eq!(
            strings["signInRejected"],
            catalogue.text(KEY_HOSTED_COMMON_SIGN_IN_REJECTED, &["Garmin"])
        );
        assert_eq!(
            strings["signInUnavailable"],
            catalogue.text(KEY_HOSTED_COMMON_SIGN_IN_UNAVAILABLE, &["Garmin"])
        );
        assert_eq!(strings["signInExpired"], expired);
        assert_eq!(
            strings["linkExpired"],
            catalogue.text(KEY_HOSTED_ERROR_INVALID_LINK, &[])
        );
        for (name, text) in strings.as_object().expect("an object") {
            assert!(
                text.as_str().is_some_and(|text| !text.contains("{0}")),
                "{locale}: the hosted login's {name} has its provider filled in"
            );
        }
    }
}

/// The page tells a lapsed sign-in from a failed one by the reason the
/// refusal carries, not by its English message: a continuation with no live
/// flow answers with exactly the reason the page's script compares against.
#[tokio::test]
async fn a_lapsed_sign_in_is_refused_with_the_reason_the_page_reads() {
    let fixture = athlete("de").await;
    let token = fixture.login_token("garmin");

    for (path, body) in [
        (
            "/api/providers/sciotte/submit-otp",
            json!({ "code": "123456" }),
        ),
        (
            "/api/providers/sciotte/select-2fa",
            json!({ "option_id": "sms" }),
        ),
    ] {
        let response = AxumTestRequest::post(path)
            .header("authorization", &format!("Bearer {token}"))
            .json(&body)
            .send(AuthRoutes::routes(fixture.resources.auth_routes_context()))
            .await;
        assert_eq!(response.status(), 400, "{path}");
        let refusal: Value = response.json();
        assert_eq!(
            refusal["details"]["reason"], LOGIN_FLOW_EXPIRED_REASON,
            "{path}: {refusal}"
        );
    }
}
