// ABOUTME: Picks the language of a platform note appended to an LLM reply, from the reply itself
// ABOUTME: Shared by the claim-verification banner and the provider-stop caveat so both match the reply
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Localizes a platform note written into the LLM's reply: the claim-
/// verification warn / block-fallback strings and the provider-stop caveat.
///
/// Such a note is appended verbatim to the reply, so its language must match
/// the reply's language — otherwise an English session ends with a French
/// postscript (or vice versa).
///
/// Resolution order (returns first match):
/// 1. **Reply text via whatlang** — long replies (≥ a few sentences)
///    detect reliably even for casual conversational tone, which is the
///    case the per-turn locale heuristic in `messaging_ingress` misses
///    (a 4-word user question can't be detected, but the 200-word reply
///    can).
/// 2. **The turn's resolved `locale`** — [`crate::SurfaceProfile::locale`],
///    settled once at the ingress boundary from the user's input, the
///    channel link, and `users.locale`. Honored verbatim whenever the
///    reply's own language is inconclusive.
pub fn resolve_banner_locale(reply: &str, locale: &str) -> String {
    if let Some(info) = whatlang::detect(reply) {
        if info.is_reliable() {
            let detected = match info.lang() {
                whatlang::Lang::Fra => Some("fr"),
                whatlang::Lang::Eng => Some("en"),
                whatlang::Lang::Spa => Some("es"),
                whatlang::Lang::Deu => Some("de"),
                whatlang::Lang::Por => Some("pt"),
                _ => None,
            };
            if let Some(code) = detected {
                return code.to_owned();
            }
        }
    }
    locale.to_owned()
}
