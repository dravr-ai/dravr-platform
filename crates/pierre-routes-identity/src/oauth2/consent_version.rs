// ABOUTME: The version of the consent screen's text in one language: a digest of every line the screen can show
// ABOUTME: Recorded with each approval; a grant approved under another version is shown the screen again
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Consent text versions
//!
//! A user who approved a client is not asked again for the same client and
//! permissions, so a consent stands only for the text it was given to: when
//! the screen's wording changes (the 2026-11-06 lines naming injuries and
//! where activities took place), a consent given before says less than what
//! Dravr shares (EFVP 2026-11-17, section 5.3 and decision 9.4a).
//!
//! Each approval records the version of the text it was given under and its
//! language. The version is computed, never maintained by hand: a SHA-256
//! digest of what the consent page shows a reader, rendered as it is served
//! ([`digested_page`]) and read as its reader reads it ([`visible_text`]:
//! the words as they render, and every attribute that hides or shows one), and
//! of every catalogue line the screen can show ([`CONSENT_SCREEN_KEYS`]) in
//! that language, as the page reads them (the language's own text, else
//! English). Every line the page shows is in it, whether a catalogue holds
//! it, the template places it or the code builds it, and nothing that only
//! says how the page looks is: a class or layout change asks no one again.
//! The stylesheet is not read, so a restyle asks no one again either; a
//! sheet rule that would hide something the page shows fails the test
//! `nothing_the_page_shows_is_hidden_by_its_stylesheet` rather than reaching
//! a reader. A change to what the reader reads, in the catalogue Dravr ships, in a
//! contremaitre bundle synced at runtime, in the template or in the code, is
//! a new version with no step anyone could forget, and the next
//! authorization of a grant approved under the old one shows the screen
//! again. A grant with no recorded version predates versions and is asked
//! again too.
//!
//! The digest is per language: a French correction asks again only those who
//! read the screen in French.
//!
//! The page also says things no catalogue line holds: whether the client is
//! verified, which company runs it and where it processes data (the
//! known-client table), whether it returns the athlete to their own machine,
//! and which services' data never reaches it (the first-party-only
//! providers). The page is rendered with them as they are for that client,
//! so a provider leaving the first-party-only list, or a client gaining a
//! badge or changing where it processes data, asks again too. What changes
//! on every visit (the account's email, the redirect host, the form's
//! tokens) and the name a client registers are rendered as fixed stand-ins:
//! they say nothing about what is shared. Every scope a client may ask for
//! is listed, so each scope's line is in the version.

use pierre_auth::oauth2_server::authorization_details::{review_digest, ResolvedDetails};
use pierre_auth::oauth2_server::client_registration::redirects_to_loopback;
use pierre_contremaitre::consent_strings::CONSENT_SCREEN_KEYS;
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::models::OAuthClientGrant;
use pierre_core::permissions::scopes::OAuthScope;
use sha2::{Digest, Sha256};

use super::consent_text::ConsentText;
use super::pages::{ConsentHtmlParams, ConsentRole};
use super::visible_text::visible_text;
use super::OAuth2Routes;

/// The client name the digested page is rendered with: the name a client
/// registers is its own, not part of what Dravr discloses about it.
const DIGESTED_CLIENT_LABEL: &str = "client";

/// The account the digested page is rendered for: the reader's own email
/// says nothing about what is shared.
const DIGESTED_ACCOUNT: &str = "athlete@example.invalid";

/// Where the digested page says a client returns the athlete, for a client
/// that returns them to their own machine and for any other: only which of
/// the two it is changes what the screen discloses.
const DIGESTED_LOOPBACK_REDIRECT: &str = "http://127.0.0.1/callback";
const DIGESTED_REDIRECT: &str = "https://client.invalid/callback";

/// Who the consent screen is shown for, as far as it changes what the screen
/// discloses.
#[derive(Debug, Clone, Copy)]
pub(super) struct ConsentSubject<'screen> {
    /// The client asking
    pub client_id: &'screen str,
    /// Where it returns the athlete, which says whether it runs on their own
    /// machine
    pub redirect_uri: &'screen str,
    /// The services whose data never reaches any client
    pub first_party_only_sources: &'screen [&'screen str],
}

/// The version of the consent screen's text as `text` reads it for `subject`.
pub(super) fn consent_version(text: &ConsentText<'_>, subject: ConsentSubject<'_>) -> String {
    version_of(
        &digested_page(OAuth2Routes::OAUTH_CONSENT_TEMPLATE, text, subject),
        text,
    )
}

/// The consent page `template` lays out for `subject`, as the version reads
/// it: every scope a client may ask for listed, and fixed stand-ins for what
/// changes on every visit (the account, the redirect host, the form's
/// tokens) and for the client's own name.
fn digested_page(template: &str, text: &ConsentText<'_>, subject: ConsentSubject<'_>) -> String {
    // The list's order is the registry's; only its membership is disclosed.
    let mut sources = subject.first_party_only_sources.to_vec();
    sources.sort_unstable();
    sources.dedup();
    let redirect_uri = if redirects_to_loopback(subject.redirect_uri) {
        DIGESTED_LOOPBACK_REDIRECT
    } else {
        DIGESTED_REDIRECT
    };
    let scope = OAuthScope::advertised_as_str().join(" ");
    OAuth2Routes::consent_html_from(
        template,
        ConsentHtmlParams {
            client_id: subject.client_id,
            redirect_uri,
            response_type: "code",
            state: "",
            scope: &scope,
            code_challenge: "",
            code_challenge_method: "",
            resource: "",
            request_uri: "",
            csrf_token: "",
            client_label: DIGESTED_CLIENT_LABEL,
            account_email: DIGESTED_ACCOUNT,
            switch_account_url: "",
            first_party_only_sources: &sources,
            text,
            // The version digests the screen an athlete reads; the role
            // lines join it through their text (CONSENT_SCREEN_KEYS).
            role: ConsentRole::default(),
            // Each choice's lines join it through their text; the details
            // each choice grants join its review digest.
            grant_choice: None,
        },
    )
}

/// The version of the consent text: what `page` shows a reader, and every
/// catalogue line the screen can show as `text` reads it.
fn version_of(page: &str, text: &ConsentText<'_>) -> String {
    version_over(page, text, &CONSENT_SCREEN_KEYS)
}

/// [`version_of`] over the lines `keys` name.
fn version_over(page: &str, text: &ConsentText<'_>, keys: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(visible_text(page).as_bytes());
    digest.update([0]);
    for key in keys {
        // A key the catalogue has no line for yet says nothing to a reader:
        // listing it here ahead of its strings asks no one again.
        let line = text.line(key);
        if line.is_empty() {
            continue;
        }
        // Each key and its line end in a NUL, so no two texts run together
        // into the same bytes.
        digest.update(key.as_bytes());
        digest.update([0]);
        digest.update(line.as_bytes());
        digest.update([0]);
    }
    hex::encode(digest.finalize())
}

/// Whether `grant`, which stored `details`, was approved under the consent
/// text its language reads now for `subject`: its recorded review digest is
/// the one those details have under that text (the text version itself for
/// a plain grant). A grant recording no version or no language was approved
/// before either was recorded, and is not current.
pub(super) fn approved_under_current_text(
    strings: &MessagingStringsRegistry,
    grant: &OAuthClientGrant,
    details: Option<&ResolvedDetails>,
    subject: ConsentSubject<'_>,
) -> bool {
    let (Some(version), Some(locale)) = (&grant.consent_version, &grant.consent_locale) else {
        return false;
    };
    let current = consent_version(&ConsentText::new(strings, locale), subject);
    review_digest(&current, details).is_ok_and(|digest| *version == digest)
}

#[cfg(test)]
mod tests {
    use super::super::consent_copy::{express_consent_line, health_line};
    use super::*;
    use dravr_tronc::dates::Timestamp;
    use pierre_contremaitre::consent_strings::{KEY_CONSENT_EXPRESS, KEY_CONSENT_HEALTH_PRESET};

    fn grant(version: Option<String>, locale: Option<&str>) -> OAuthClientGrant {
        OAuthClientGrant {
            id: "grant".to_owned(),
            user_id: "user".to_owned(),
            tenant_id: "tenant".to_owned(),
            client_id: "client".to_owned(),
            scope: "fitness:read".to_owned(),
            granted_at: Timestamp::now(),
            revoked_at: None,
            consent_version: version,
            consent_locale: locale.map(str::to_owned),
            authorization_details: None,
            expires_at: None,
        }
    }

    /// The list the version digests is the screen's whole catalogue: a key
    /// added under `oauth.consent.` and left out of it would change the screen
    /// without changing the version.
    #[test]
    fn every_consent_screen_key_in_the_catalogue_is_versioned() {
        let registry = MessagingStringsRegistry::new();
        let mut missing: Vec<String> = registry
            .list()
            .into_iter()
            .map(|(key, _, _)| key)
            .filter(|key| key.starts_with("oauth.consent."))
            .filter(|key| !CONSENT_SCREEN_KEYS.contains(&key.as_str()))
            .collect();
        missing.sort_unstable();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "bug: add these keys to CONSENT_SCREEN_KEYS: {missing:?}"
        );
    }

    const CLAUDE: &str = "https://claude.ai/oauth/mcp-oauth-client-metadata";
    const CALLBACK: &str = "https://claude.ai/api/mcp/auth_callback";
    const SOURCES: [&str; 2] = ["Garmin", "WHOOP"];

    fn claude(sources: &'static [&'static str]) -> ConsentSubject<'static> {
        ConsentSubject {
            client_id: CLAUDE,
            redirect_uri: CALLBACK,
            first_party_only_sources: sources,
        }
    }

    fn version(registry: &MessagingStringsRegistry, locale: &str) -> String {
        consent_version(&ConsentText::new(registry, locale), claude(&SOURCES))
    }

    #[test]
    fn a_wording_change_is_a_new_version_in_its_language_only() {
        let registry = MessagingStringsRegistry::new();
        let french = version(&registry, "fr");
        let english = version(&registry, "en");
        assert_ne!(french, english, "each language reads its own text");
        assert_eq!(
            french,
            version(&registry, "fr"),
            "the same text is the same version"
        );

        registry.update(
            KEY_CONSENT_EXPRESS,
            "fr",
            "Une autre formulation.".to_owned(),
            "reworded".to_owned(),
        );
        assert_ne!(
            version(&registry, "fr"),
            french,
            "bug: a reworded line is a new version"
        );
        assert_eq!(
            version(&registry, "en"),
            english,
            "a French change leaves the English version alone"
        );
    }

    /// The platform lists a key before the catalogue ships its line: a
    /// listed key with no line in the reader's language or in English adds
    /// nothing to the version, so listing it asks no one again, and its
    /// line's arrival is a new version.
    #[test]
    fn a_listed_key_without_a_line_adds_nothing_to_the_version() {
        let registry = MessagingStringsRegistry::new();
        let key = KEY_CONSENT_HEALTH_PRESET;
        for locale in ["fr", "en"] {
            registry.update(key, locale, String::new(), "cleared".to_owned());
        }
        let text = ConsentText::new(&registry, "fr");
        assert!(
            text.line(key).is_empty(),
            "fixture: the cleared key reads no line"
        );
        let page = digested_page(
            OAuth2Routes::OAUTH_CONSENT_TEMPLATE,
            &text,
            claude(&SOURCES),
        );
        let others: Vec<&str> = CONSENT_SCREEN_KEYS
            .into_iter()
            .filter(|listed| *listed != key)
            .collect();
        let without_line = version_of(&page, &text);
        assert_eq!(
            without_line,
            version_over(&page, &text, &others),
            "bug: a listed key with no line changed the version"
        );

        registry.update(
            key,
            "fr",
            "Les données énumérées ci-dessous seront communiquées à {0}.".to_owned(),
            "arrived".to_owned(),
        );
        assert_ne!(
            version_of(&page, &ConsentText::new(&registry, "fr")),
            without_line,
            "bug: a line's arrival is a new version"
        );
    }

    /// A service leaving the first-party-only list sends its data to the
    /// client from then on, which the screen says: a grant approved while it
    /// was on the list is asked again. The list's order discloses nothing.
    #[test]
    fn a_change_to_the_first_party_only_services_is_a_new_version() {
        let registry = MessagingStringsRegistry::new();
        let text = ConsentText::new(&registry, "en");
        let both = consent_version(&text, claude(&SOURCES));
        assert_ne!(
            consent_version(&text, claude(&["Garmin"])),
            both,
            "bug: a service whose data now reaches the client is a new version"
        );
        assert_ne!(
            consent_version(&text, claude(&[])),
            both,
            "bug: no first-party-only service at all is a new version"
        );
        assert_eq!(
            consent_version(&text, claude(&["WHOOP", "Garmin"])),
            both,
            "the same services in another order are the same version"
        );
    }

    /// What the screen says about the client — verified or not, who runs it,
    /// whether it runs on the athlete's own machine — is part of what they
    /// approved.
    #[test]
    fn what_the_screen_says_about_the_client_is_part_of_the_version() {
        let registry = MessagingStringsRegistry::new();
        let text = ConsentText::new(&registry, "en");
        let verified = consent_version(&text, claude(&SOURCES));
        let unverified = consent_version(
            &text,
            ConsentSubject {
                client_id: "dyn-8c1f",
                ..claude(&SOURCES)
            },
        );
        assert_ne!(
            verified, unverified,
            "bug: a client losing its verified badge is a new version"
        );
        assert_ne!(
            consent_version(
                &text,
                ConsentSubject {
                    client_id: "dyn-8c1f",
                    redirect_uri: "http://127.0.0.1:7777/cb",
                    first_party_only_sources: &SOURCES,
                },
            ),
            unverified,
            "bug: an unverified client returning to the athlete's machine reads another warning"
        );
        assert_eq!(
            consent_version(
                &text,
                ConsentSubject {
                    client_id: "dyn-0000",
                    ..claude(&SOURCES)
                },
            ),
            unverified,
            "two unverified clients the screen describes alike are the same version"
        );
    }

    /// The page the version is read from, for `template` and Claude.
    fn page(template: &str, text: &ConsentText<'_>) -> String {
        digested_page(template, text, claude(&SOURCES))
    }

    /// The version is what the page a reader gets says: every scope's line,
    /// the client's disclosures and every line the code builds are on it.
    #[test]
    fn the_version_is_what_the_served_page_says() {
        let registry = MessagingStringsRegistry::new();
        let text = ConsentText::new(&registry, "en");
        let page = page(OAuth2Routes::OAUTH_CONSENT_TEMPLATE, &text);
        assert_eq!(
            consent_version(&text, claude(&SOURCES)),
            version_of(&page, &text),
            "bug: the version is read from the page the reader gets"
        );
        let shown = visible_text(&page);
        for scope in OAuthScope::advertised_as_str() {
            let line = visible_text(&text.scope_item(scope, false));
            assert!(shown.contains(&line), "bug: {scope}'s line is read: {line}");
        }
        for line in [
            health_line(DIGESTED_CLIENT_LABEL, false, &text),
            express_consent_line(DIGESTED_CLIENT_LABEL, false, &text),
        ] {
            let line = visible_text(&line);
            assert!(shown.contains(&line), "bug: the line is read: {line}");
        }
    }

    /// A class, an attribute or a layout change says the same thing to the
    /// reader, whether the template or the code that builds a line makes it:
    /// no one is asked again.
    #[test]
    fn a_markup_change_asks_no_one_again() {
        let registry = MessagingStringsRegistry::new();
        let text = ConsentText::new(&registry, "en");
        let template = OAuth2Routes::OAUTH_CONSENT_TEMPLATE;
        let current = version_of(&page(template, &text), &text);

        let restyled = template
            .replace(r#"class="card card-center""#, r#"class="card wide""#)
            .replace(
                r#"<div class="disclosures">"#,
                "<section class=\"disclosures\">\n\n    ",
            )
            .replace("<!-- Copyright (c) 2026 dravr.ai -->", "");
        assert_ne!(restyled, template, "fixture: the template's markup changed");
        assert_eq!(
            version_of(&page(&restyled, &text), &text),
            current,
            "bug: a markup-only change to the template asks no one again"
        );

        // As the code that builds the disclosures would restyle them.
        let page = page(template, &text);
        let restyled = page
            .replace(r#"class="disclosure""#, r#"class="disclosure wide""#)
            .replace(r#"class="fineprint""#, r#"class="fineprint muted""#);
        assert_ne!(restyled, page, "fixture: a built line's markup changed");
        assert_eq!(
            version_of(&restyled, &text),
            current,
            "bug: a markup-only change to a line the code builds asks no one again"
        );

        // A word wrapped in an inline tag reads as it did, the full stop after
        // it included; a space written as a reference reads as a space; a
        // `<template>` never renders.
        let health = health_line(DIGESTED_CLIENT_LABEL, false, &text);
        let dot = health
            .rfind(".</p>")
            .expect("fixture: the health line ends a sentence");
        let space = health[..dot].rfind(' ').expect("fixture: a word ends it");
        let word = &health[space + 1..dot];
        for (case, line) in [
            (
                "a word before a full stop in <strong>",
                format!(
                    "{}<strong>{word}</strong>{}",
                    &health[..=space],
                    &health[dot..]
                ),
            ),
            (
                "a space as &nbsp;",
                format!("{}&nbsp;{}", &health[..space], &health[space + 1..]),
            ),
            (
                "a space as &#32;",
                format!("{}&#32;{}", &health[..space], &health[space + 1..]),
            ),
            (
                "a <template> holding words",
                format!("{health}<template><p>Never rendered.</p></template>"),
            ),
        ] {
            let marked = page.replace(&health, &line);
            assert_ne!(marked, page, "fixture: {case} changed the page");
            assert_eq!(
                version_of(&marked, &text),
                current,
                "bug: {case} asks no one again"
            );
        }
    }

    /// An attribute that hides a line, or shows one, changes what the reader
    /// sees: everyone is asked again.
    #[test]
    fn a_line_hidden_or_shown_is_a_new_version() {
        let registry = MessagingStringsRegistry::new();
        let text = ConsentText::new(&registry, "en");
        let template = OAuth2Routes::OAUTH_CONSENT_TEMPLATE;
        let current = version_of(&page(template, &text), &text);
        for hidden in [
            r#"<div class="disclosures" hidden>"#,
            r#"<div class="disclosures" aria-hidden="true">"#,
            r#"<div class="disclosures" style="display:none">"#,
        ] {
            let changed = template.replace(r#"<div class="disclosures">"#, hidden);
            assert_ne!(changed, template, "fixture: {hidden}");
            assert_ne!(
                version_of(&page(&changed, &text), &text),
                current,
                "bug: {hidden} hides every disclosure and asks no one again"
            );
        }
    }

    /// The selectors of `stylesheet`'s rules that hide what they match, each
    /// as its subject: the last compound selector, which names the elements
    /// hidden.
    fn hiding_subjects(stylesheet: &str) -> Vec<String> {
        const HIDING: [&str; 6] = [
            "display:none",
            "visibility:hidden",
            "visibility:collapse",
            "opacity:0",
            "font-size:0",
            "content-visibility:hidden",
        ];
        let mut sheet = stylesheet.to_owned();
        while let Some(open) = sheet.find("/*") {
            let close = sheet[open..]
                .find("*/")
                .map_or(sheet.len(), |c| open + c + 2);
            sheet.replace_range(open..close, "");
        }
        let mut subjects = Vec::new();
        for rule in sheet.split('}') {
            let Some((head, declarations)) = rule.rsplit_once('{') else {
                continue;
            };
            let selectors = head.rsplit('{').next().unwrap_or(head);
            let hides = declarations.split(';').any(|declaration| {
                let declaration: String = declaration
                    .replace("!important", "")
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    .to_ascii_lowercase();
                HIDING.iter().any(|hiding| {
                    declaration == *hiding
                        || (declaration.starts_with(hiding)
                            && declaration[hiding.len()..]
                                .chars()
                                .all(|c| c == '.' || c == '0' || c.is_ascii_alphabetic()))
                })
            });
            if hides {
                for selector in selectors.split(',') {
                    if let Some(subject) = selector
                        .split([' ', '>', '+', '~'])
                        .rfind(|s| !s.is_empty())
                    {
                        subjects.push(subject.trim().to_owned());
                    }
                }
            }
        }
        subjects
    }

    /// The class and element names of a page's elements.
    fn names_on(page: &str) -> (Vec<String>, Vec<String>) {
        let (mut classes, mut elements) = (Vec::new(), Vec::new());
        for tag in page.split('<').skip(1) {
            let name: String = tag
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect::<String>()
                .to_ascii_lowercase();
            if name.is_empty() {
                continue;
            }
            let tag = tag.split('>').next().unwrap_or(tag);
            if let Some((_, rest)) = tag.split_once("class=\"") {
                let list = rest.split('"').next().unwrap_or(rest);
                classes.extend(list.split_whitespace().map(str::to_owned));
            }
            elements.push(name);
        }
        (classes, elements)
    }

    /// The consent version does not read the stylesheet, so a rule hiding
    /// anything the page shows would change what the reader sees and ask no
    /// one again. None may: a sheet rule hiding a class the page uses, or an
    /// element the page shows, fails here, where it is decided whether the
    /// change is one the reader must be asked again about.
    #[test]
    fn nothing_the_page_shows_is_hidden_by_its_stylesheet() {
        // Not rendered, or holding nothing a reader reads.
        const UNSHOWN: [&str; 7] = ["head", "meta", "title", "style", "script", "input", "link"];
        let registry = MessagingStringsRegistry::new();
        let text = ConsentText::new(&registry, "en");
        let page = page(OAuth2Routes::OAUTH_CONSENT_TEMPLATE, &text);
        let open = page
            .find("<style>")
            .expect("fixture: the page holds its sheet");
        let close = page.find("</style>").expect("fixture: the sheet ends");
        let stylesheet = &page[open + "<style>".len()..close];
        let subjects = hiding_subjects(stylesheet);
        assert!(
            subjects.iter().any(|s| s == "[hidden]"),
            "fixture: the sheet's `[hidden]` rule is read as one that hides: {subjects:?}"
        );

        let (classes, elements) = names_on(&page);
        let mut hidden: Vec<String> = Vec::new();
        for subject in &subjects {
            let element: String = subject
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect::<String>()
                .to_ascii_lowercase();
            if !element.is_empty()
                && !UNSHOWN.contains(&element.as_str())
                && elements.contains(&element)
            {
                hidden.push(subject.clone());
            }
            for class in subject.split('.').skip(1) {
                let class: String = class
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                    .collect();
                if classes.contains(&class) {
                    hidden.push(subject.clone());
                }
            }
        }

        // As a rule hiding the disclosures would read.
        assert!(
            hiding_subjects(".disclosures { display: none }")
                .iter()
                .any(|s| classes.contains(&s.trim_start_matches('.').to_owned())),
            "fixture: a rule hiding a class the page uses is caught"
        );
        assert!(
            hidden.is_empty(),
            "bug: the stylesheet hides what the consent page shows, which the version does not read: {hidden:?}"
        );
    }

    /// The words the reader reads, and which lines the page shows in what
    /// order, are the version, whoever writes them.
    #[test]
    fn a_change_to_what_the_reader_reads_is_a_new_version() {
        let registry = MessagingStringsRegistry::new();
        let text = ConsentText::new(&registry, "en");
        let template = OAuth2Routes::OAUTH_CONSENT_TEMPLATE;
        let current = version_of(&page(template, &text), &text);

        let reworded = template.replace("{{T_TITLE}} - Dravr", "{{T_TITLE}} | Dravr");
        assert_ne!(
            version_of(&page(&reworded, &text), &text),
            current,
            "bug: the template's own words are part of what is read"
        );
        let dropped = template.replace("{{OUTSIDE_QUEBEC_LINE}}", "");
        assert_ne!(
            version_of(&page(&dropped, &text), &text),
            current,
            "bug: a line the page stops showing is a new version"
        );

        // As the code that builds the health line would reword it.
        let served = page(template, &text);
        let health = health_line(DIGESTED_CLIENT_LABEL, false, &text);
        assert!(served.contains(&health), "fixture: the page shows the line");
        let reworded = served.replace(&health, &format!("Not sensitive. {health}"));
        assert_ne!(
            version_of(&reworded, &text),
            current,
            "bug: a reworded line the code builds is a new version"
        );
    }

    /// What changes on every visit says nothing about what is shared.
    #[test]
    fn the_redirect_host_is_not_part_of_the_version() {
        let registry = MessagingStringsRegistry::new();
        let text = ConsentText::new(&registry, "en");
        assert_eq!(
            consent_version(
                &text,
                ConsentSubject {
                    redirect_uri: "https://other.claude.ai/callback",
                    ..claude(&SOURCES)
                }
            ),
            consent_version(&text, claude(&SOURCES)),
            "bug: the redirect host says nothing about what is shared"
        );
    }

    #[test]
    fn only_a_grant_approved_under_the_current_text_is_current() {
        let registry = MessagingStringsRegistry::new();
        let current = version(&registry, "fr");
        let subject = claude(&SOURCES);
        assert!(
            approved_under_current_text(
                &registry,
                &grant(Some(current.clone()), Some("fr")),
                None,
                subject
            ),
            "bug: a grant approved under the current text in its language is current"
        );
        assert!(
            !approved_under_current_text(
                &registry,
                &grant(Some(current.clone()), Some("en")),
                None,
                subject
            ),
            "the version is the text of the language it was approved in"
        );
        assert!(
            !approved_under_current_text(
                &registry,
                &grant(Some(current), Some("fr")),
                None,
                claude(&["Garmin"])
            ),
            "bug: a grant approved before a service left the first-party-only list is asked again"
        );
        assert!(
            !approved_under_current_text(
                &registry,
                &grant(Some("old".to_owned()), Some("fr")),
                None,
                subject
            ),
            "bug: a grant approved under older text is asked again"
        );
        assert!(
            !approved_under_current_text(&registry, &grant(None, None), None, subject),
            "bug: a grant recorded before versions is asked again"
        );
    }

    #[test]
    fn a_detailed_grant_is_current_only_under_its_own_review_digest() {
        use pierre_auth::oauth2_server::authorization_details::{preset_defaults, Preset};
        let registry = MessagingStringsRegistry::new();
        let current = version(&registry, "fr");
        let subject = claude(&SOURCES);
        let training = preset_defaults(Preset::TrainingCoach, "https://mcp.example.test/mcp");
        let sleep = preset_defaults(Preset::SleepCoach, "https://mcp.example.test/mcp");
        let digest = review_digest(&current, Some(&training)).unwrap_or_default();
        let approved = grant(Some(digest), Some("fr"));
        assert!(
            approved_under_current_text(&registry, &approved, Some(&training), subject),
            "bug: a preset grant approved under the current text is current"
        );
        assert!(
            !approved_under_current_text(&registry, &approved, Some(&sleep), subject),
            "bug: a digest binds the details it was approved for"
        );
        assert!(
            !approved_under_current_text(&registry, &approved, None, subject),
            "bug: a detailed grant's digest read as a plain grant's"
        );
        assert!(
            approved_under_current_text(
                &registry,
                &grant(Some(current), Some("fr")),
                None,
                subject
            ),
            "bug: a plain grant's digest is its text version"
        );
    }
}
