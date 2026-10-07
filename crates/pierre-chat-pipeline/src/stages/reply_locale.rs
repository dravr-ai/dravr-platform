// ABOUTME: Picks the language of a platform note appended to an LLM reply, from the reply itself
// ABOUTME: Shared by the claim-verification banner and the provider-stop caveat so both match the reply
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::language::detect_locale;

/// Localizes a platform note written into the LLM's reply: the claim-
/// verification warn / block-fallback strings and the provider-stop caveat.
///
/// Such a note is appended verbatim to the reply, so its language must match
/// the reply's language — otherwise an English session ends with a French
/// postscript (or vice versa).
///
/// Resolution order (returns first match):
/// 1. **Reply text** — the reply's own language, from the same local
///    detector as the turn's locale ([`crate::language::detect_locale`]), so
///    a model that answered in another language than the question still gets
///    a matching note. A build without the `language-detection` feature has
///    no local detector and goes straight to step 2; it does not spend an
///    LLM call here, because the turn's locale is already the question's
///    language and the reply was directed to answer in it.
/// 2. **The turn's resolved `locale`** — [`crate::SurfaceProfile::locale`],
///    settled once at the ingress boundary from the user's input, the
///    channel link, and `users.locale`. Honored verbatim whenever the
///    reply's own language is inconclusive.
pub fn resolve_banner_locale(reply: &str, locale: &str) -> String {
    detect_locale(reply).map_or_else(|| locale.to_owned(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::resolve_banner_locale;

    const ENGLISH_REPLY: &str = "ChefFamille, same situation — the readiness ladder stops at \
        Level 0: No Data. None of the data tools are responding. The honest answer: I cannot \
        responsibly prescribe intensity, duration, or workout structure without your activity \
        history, your physiological profile, and your recent session quality.";

    const FRENCH_REPLY: &str = "ChefFamille, même situation — l'échelle de prêt s'arrête au \
        niveau zéro : aucune donnée. Aucun des outils de données ne répond. La réponse \
        honnête : je ne peux pas prescrire d'intensité, de durée, ni de structure de séance \
        sans ton historique d'activités.";

    /// The 2026-05-01 sweep: an English session ended with a French
    /// postscript. The note follows the reply, not the stored locale.
    #[cfg(feature = "language-detection")]
    #[test]
    fn an_english_reply_gets_an_english_note_on_a_french_account() {
        assert_eq!(resolve_banner_locale(ENGLISH_REPLY, "fr"), "en");
    }

    #[cfg(feature = "language-detection")]
    #[test]
    fn a_french_reply_gets_a_french_note_on_an_english_account() {
        assert_eq!(resolve_banner_locale(FRENCH_REPLY, "en"), "fr");
    }

    /// A short reply is read too (carnet#825), so a terse English answer gets
    /// an English note on a French account.
    #[cfg(feature = "language-detection")]
    #[test]
    fn a_one_sentence_reply_is_read() {
        assert_eq!(
            resolve_banner_locale("What should I do tomorrow morning?", "fr"),
            "en"
        );
    }

    #[cfg(not(feature = "language-detection"))]
    #[test]
    fn without_a_local_detector_the_note_follows_the_turn_locale() {
        assert_eq!(resolve_banner_locale(ENGLISH_REPLY, "fr"), "fr");
        assert_eq!(resolve_banner_locale(FRENCH_REPLY, "en"), "en");
    }

    #[test]
    fn a_reply_too_ambiguous_to_call_keeps_the_turn_locale() {
        assert_eq!(
            resolve_banner_locale("Muéstrame mis últimas cinco actividades", "fr"),
            "fr"
        );
    }
}
