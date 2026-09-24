// ABOUTME: HTML escaping and the shared Boreal stylesheet for server-rendered hosted pages
// ABOUTME: Attribute-safe escaping for injected values, and the one stylesheet every hosted template embeds
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// The Boreal stylesheet, light and dark, generated from the shared design
/// tokens by `packages/shared-constants/scripts/generate-hosted-css.ts`.
///
/// It lives in this crate because every crate that renders a hosted page
/// depends on it; `scripts/ci/check-hosted-css.sh` regenerates the file and
/// fails a push that left it behind the tokens. A page built from a template
/// file takes it through [`with_hosted_page_css`]; a page assembled with
/// `format!` interpolates it into its own `<style>` element.
pub const HOSTED_PAGE_CSS: &str = include_str!("hosted_page.css");

/// Where a hosted-page template asks for the stylesheet, inside its `<style>`.
const HOSTED_PAGE_CSS_PLACEHOLDER: &str = "{{HOSTED_PAGE_CSS}}";

/// A hosted-page template with the Boreal stylesheet in place of its
/// `{{HOSTED_PAGE_CSS}}` placeholder.
///
/// Renderers call this before substituting any request value, so a value that
/// happens to spell the placeholder is never expanded into the stylesheet.
#[must_use]
pub fn with_hosted_page_css(template: &str) -> String {
    template.replace(HOSTED_PAGE_CSS_PLACEHOLDER, HOSTED_PAGE_CSS)
}

/// Escape a string for safe insertion into HTML attribute values.
///
/// Replaces the five HTML-special characters (`&`, `<`, `>`, `"`, `'`) with their
/// corresponding HTML entities. This prevents attribute breakout and script injection
/// when inserting user-controlled values into HTML attributes like `value="..."`.
#[must_use]
pub fn escape_html_attribute(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#x27;"),
            _ => output.push(ch),
        }
    }
    output
}
