// ABOUTME: Locale resolution and catalogue lookups for the hosted pages a chat link opens
// ABOUTME: Resolves the page language from the link-token's user and fills a template's `{{t:<key>}}` strings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What language a hosted page is written in, and its strings in it.
//!
//! The hosted connect pages are opened from a chat by an athlete whose
//! language the platform already knows. The page language is **resolved,
//! never stated**: it is read from the stored preference of the user the
//! signed link-token names, on the channel it names, not from a query
//! parameter or a claim the browser supplies.
//!
//! The chat that carries the link words its reply through the channel chain
//! — the athlete's `/language` override for that channel, then their profile
//! — so the page walks the same chain ([`resolve_linked_channel_locale`]) and
//! reads in the language of the chat that opened it.
//!
//! A page with no verifiable token — a missing, expired or forged link — has
//! no user to ask. It follows the browser's `Accept-Language`, as the
//! short-link "expired" page does.
//!
//! Every string comes from the dravr-contremaitre catalogue through
//! [`MessagingStringsRegistry`], under the keys
//! [`pierre_contremaitre::hosted_strings`] declares, and is HTML-escaped on
//! its way into the page.

use axum::http::HeaderMap;
use pierre_contremaitre::hosted_strings::{KEY_HOSTED_COMMON_YOUR_CHAT_APP, TEMPLATE_KEYS};
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::html::escape_html_attribute;
use pierre_core::models::messaging::channel_display_name;
use pierre_core::models::TenantId;
use pierre_middleware::provider_link_token::{verify_link_token, ProviderLinkTokenClaims};
use pierre_services::locale::{resolve_linked_channel_locale, resolve_user_locale};
use serde_json::Value;
use uuid::Uuid;

use crate::short_link::preferred_locale;
use crate::AuthRoutesContext;

/// Where a hosted template asks for its `<html lang>`.
const LANG_PLACEHOLDER: &str = "{{LANG}}";

/// The catalogue, read in one page's locale.
pub struct PageStrings<'a> {
    registry: &'a MessagingStringsRegistry,
    locale: String,
}

impl<'a> PageStrings<'a> {
    /// The catalogue as a page in `locale` reads it.
    #[must_use]
    pub fn new(registry: &'a MessagingStringsRegistry, locale: impl Into<String>) -> Self {
        Self {
            registry,
            locale: locale.into(),
        }
    }

    /// The page's locale.
    #[must_use]
    pub fn locale(&self) -> &str {
        &self.locale
    }

    /// The text of `key`, unescaped.
    #[must_use]
    pub fn get(&self, key: &str) -> String {
        self.registry.get(key, &self.locale)
    }

    /// The text of `key` with its positional slots filled, unescaped.
    #[must_use]
    pub fn render(&self, key: &str, args: &[&str]) -> String {
        self.registry.render(key, &self.locale, args)
    }

    /// The text of `key` with its slots filled, escaped for the page.
    #[must_use]
    pub fn html(&self, key: &str, args: &[&str]) -> String {
        escape_html_attribute(&self.render(key, args))
    }

    /// What the page calls the chat the athlete came from: the channel's own
    /// name, or the catalogue's "your chat app" for a slug that names none.
    #[must_use]
    pub fn channel(&self, slug: &str) -> String {
        channel_display_name(slug).map_or_else(
            || self.get(KEY_HOSTED_COMMON_YOUR_CHAT_APP),
            ToOwned::to_owned,
        )
    }

    /// `template` with its `<html lang>` and every `{{t:<key>}}` it names
    /// filled from the catalogue, each escaped.
    ///
    /// Renderers call this before substituting any request value, so a typed
    /// value that spells a placeholder is never expanded.
    #[must_use]
    pub fn fill(&self, template: &str) -> String {
        let mut page = template.replace(LANG_PLACEHOLDER, &escape_html_attribute(&self.locale));
        for key in TEMPLATE_KEYS {
            let placeholder = format!("{{{{t:{key}}}}}");
            if page.contains(&placeholder) {
                page = page.replace(&placeholder, &self.html(key, &[]));
            }
        }
        page
    }
}

/// The locale of the athlete a verified link-token names, on the channel
/// the link was sent to.
///
/// `tenant_id` and `channel` are the token's own: together with the user they
/// name the channel link whose `/language` override the chat reply was worded
/// by. A token whose tenant does not parse names no link, and the athlete's
/// profile-wide locale answers.
pub async fn locale_for_reader(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    channel: &str,
) -> String {
    let users = resources.repos.users.as_ref();
    match tenant_id {
        Some(tenant_id) => {
            resolve_linked_channel_locale(
                resources.repos.messaging.as_ref(),
                users,
                TenantId::from_uuid(tenant_id),
                channel,
                user_id,
            )
            .await
        }
        None => resolve_user_locale(users, user_id).await,
    }
}

/// The locale of the athlete `claims` names, or `None` when their subject is
/// not a user id and there is nobody's preference to read.
pub async fn locale_for_claims(
    resources: &AuthRoutesContext,
    claims: &ProviderLinkTokenClaims,
) -> Option<String> {
    let user_id = Uuid::parse_str(&claims.sub).ok()?;
    let tenant_id = Uuid::parse_str(&claims.tid).ok();
    Some(locale_for_reader(resources, user_id, tenant_id, &claims.channel).await)
}

/// The locale of a page that may or may not hold a link-token.
///
/// A token that verifies for `provider` names its athlete and their channel,
/// whose stored locale is the page's. Anything else — no token, an expired or
/// forged one, a subject that is not a user id — leaves the browser's
/// `Accept-Language`.
pub async fn locale_for_token(
    resources: &AuthRoutesContext,
    token: Option<&str>,
    provider: &str,
    headers: &HeaderMap,
) -> String {
    let claims = token
        .filter(|token| !token.is_empty())
        .and_then(|token| verify_link_token(token, &resources.admin_jwt_secret, provider).ok());
    let resolved = match claims {
        Some(claims) => locale_for_claims(resources, &claims).await,
        None => None,
    };
    resolved.unwrap_or_else(|| preferred_locale(headers).to_owned())
}

/// `value` as JSON that is safe inside a `<script>` element.
///
/// JSON leaves `<` as it is, and the HTML parser ends a script at `</script`
/// wherever it appears — inside a string literal too. Its JSON unicode
/// escape (backslash, `u003c`) is the same character to the script and no tag
/// to the parser.
#[must_use]
pub fn script_json(value: &Value) -> String {
    value.to_string().replace('<', "\\u003c")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn script_json_cannot_close_the_script_element() {
        let value = json!({
            "display_name": "</script><script>alert(1)</script>",
            "note": "a < b",
        });
        let embedded = script_json(&value);

        assert!(!embedded.contains('<'), "{embedded}");
        assert!(
            embedded.contains(r"\u003c/script>"),
            "the tag opener is a JSON unicode escape: {embedded}"
        );
        // The script reads back exactly what the server serialized.
        assert_eq!(serde_json::from_str::<Value>(&embedded).ok(), Some(value));
    }
}
