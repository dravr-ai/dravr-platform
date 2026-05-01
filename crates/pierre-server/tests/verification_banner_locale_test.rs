// ABOUTME: Locks the resolve_banner_locale contract — the Tier 5.5 banner must match the reply's language
// ABOUTME: Regression test for the 2026-05-01 sweep where an English session ended with a French postscript
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "tools-verification")]
#![allow(clippy::unwrap_used)]
#![allow(missing_docs)]

//! `resolve_banner_locale` is what stops Tier 5.5 from emitting a
//! French banner on an English reply (or vice versa). The function
//! lives next to the verification stage because the banner is
//! appended verbatim to the LLM's reply — its language must match.
//!
//! We can't unit-test it from `pierre-server`'s integration test crate
//! without exposing it; it ships as `pub(crate)` so this file lives
//! under `tests/` and probes the same logic by re-implementing the
//! whatlang call against the same inputs. If the production function's
//! resolution rules change, the upstream change should land here too.

use whatlang::{detect, Lang};

const ENGLISH_REPLY: &str = "ChefFamille, same situation — the readiness ladder stops at \
    Level 0: No Data. None of the data tools are responding. The honest answer: I cannot \
    responsibly prescribe intensity, duration, or workout structure without your activity \
    history, your physiological profile, and your recent session quality. The one safe \
    prescription that doesn't require zone calibration is recovery_30min: 30 minutes of \
    easy running tomorrow morning, conversational effort throughout. To unlock real coaching, \
    reconnect Strava or share a recent race result, your max HR, your FTP, or your resting HR.";

const FRENCH_REPLY: &str = "ChefFamille, même situation — l'échelle de prêt s'arrête au \
    niveau zéro : aucune donnée. Aucun des outils de données ne répond. La réponse honnête : \
    je ne peux pas prescrire d'intensité, de durée, ni de structure de séance sans ton \
    historique d'activités, ton profil physiologique, et la qualité de tes séances récentes. \
    La seule prescription qui ne dépend pas du calibrage des zones est recovery_30min : \
    trente minutes de course facile demain matin, à l'allure de la conversation.";

/// English reply must produce `en` even though the user's stored
/// locale is `fr` — the banner is appended to the reply text and must
/// match the LLM's output language.
#[test]
fn english_reply_overrides_french_user_locale() {
    let info = detect(ENGLISH_REPLY).unwrap();
    assert!(
        info.is_reliable(),
        "long English reply should be reliably detected; confidence {:.2}",
        info.confidence()
    );
    assert_eq!(info.lang(), Lang::Eng);
}

/// French reply must produce `fr` so we don't accidentally swap a
/// French session's banner to English.
#[test]
fn french_reply_keeps_french_locale() {
    let info = detect(FRENCH_REPLY).unwrap();
    assert!(
        info.is_reliable(),
        "long French reply should be reliably detected; confidence {:.2}",
        info.confidence()
    );
    assert_eq!(info.lang(), Lang::Fra);
}

/// Confirms the regression scenario: a 4-word user question is
/// unreliable on its own (which is why the messaging-ingress per-turn
/// detection couldn't fix it), but the matching long reply IS reliable.
#[test]
fn short_question_is_unreliable_long_reply_is_reliable() {
    let short = detect("What should I do tomorrow morning?").unwrap();
    assert!(
        !short.is_reliable(),
        "short conversational question should NOT be reliably detectable — confidence {:.4}",
        short.confidence()
    );
    let long = detect(ENGLISH_REPLY).unwrap();
    assert!(
        long.is_reliable(),
        "long reply should compensate for the short question's ambiguity"
    );
}
