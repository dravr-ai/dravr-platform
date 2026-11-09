// ABOUTME: OAuth template rendering for multi-tenant MCP server
// ABOUTME: Provides HTML template rendering for OAuth success and error pages
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use html_escape::encode_text;
use pierre_core::html::{oauth_error_page, with_hosted_page_css};

use crate::OAuthCallbackResponse;

/// Template renderer for OAuth success and error pages
#[derive(Debug)]
pub struct OAuthTemplateRenderer;

impl OAuthTemplateRenderer {
    /// Render OAuth success template
    #[must_use]
    pub fn render_success_template(
        provider: &str,
        callback_response: &OAuthCallbackResponse,
    ) -> String {
        const TEMPLATE: &str = include_str!("../templates/oauth_success.html");

        // Capitalize provider name for display (e.g., "strava" -> "Strava")
        let capitalized_provider = Self::capitalize_provider(provider);

        // Escape all interpolated values to prevent XSS
        with_hosted_page_css(TEMPLATE)
            .replace("{{PROVIDER}}", &encode_text(&capitalized_provider))
            .replace("{{PROVIDER_LOWER}}", &encode_text(&provider.to_lowercase()))
            .replace("{{USER_ID}}", &encode_text(&callback_response.user_id))
    }

    /// Capitalize provider name for display
    fn capitalize_provider(provider: &str) -> String {
        let mut chars = provider.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().collect::<String>() + chars.as_str()
        })
    }

    /// Render the OAuth failure page in `lang`, escaping what it says.
    #[must_use]
    pub fn render_error_template(lang: &str, title: &str, message: &str) -> String {
        oauth_error_page(lang, &encode_text(title), &encode_text(message))
    }
}
