// ABOUTME: A withheld reply is persisted for the athlete but never replayed into a later prompt
// ABOUTME: Regression for the withhold-poison loop (P7, 2026-08-02)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! When the response boundary withholds a reply it persists a localized apology
//! — « Ma réponse n'est pas passée — elle mélangeait des détails techniques… » —
//! in place of the model's output. The athlete saw that text, so the row must
//! stay in the database and in the UI.
//!
//! It must not stay in the *prompt*. Replayed as an assistant turn it is
//! first-person narration about the platform failing, which is the exact
//! self-referential-failure class `scrub_replayed_narration` exists to delete
//! (2026-07-23 learned-helplessness incident, `b57e0dee9`). It survived that
//! scrub because the string is authored in `dravr-contremaitre` in five
//! locales and matches none of the pattern tables — and prose matching would
//! drift from the remote source on the next reword.
//!
//! So the row is stamped at write time and dropped by stamp on replay. This is
//! not hypothetical: conversation `043fdd88` took two withholds 26 minutes
//! apart on 2026-07-24, carried both rows into every later prompt, and leaked
//! again on 2026-07-28 — after the identity anchor shipped.

use pierre_chat_pipeline::stages::prompt_builder::build_llm_messages;
use pierre_core::models::{MessageRecord, WITHHELD_REPLY_FINISH_REASON};

fn row(id: &str, role: &str, content: &str, finish_reason: Option<&str>) -> MessageRecord {
    MessageRecord {
        id: id.to_owned(),
        conversation_id: "conv-1".to_owned(),
        role: role.to_owned(),
        content: content.to_owned(),
        token_count: None,
        prompt_tokens: None,
        model: None,
        finish_reason: finish_reason.map(str::to_owned),
        structured_content: None,
        created_at: "2026-08-02T12:00:00Z".to_owned(),
    }
}

/// The verbatim FR default (`messaging_strings.rs` `FR_REPLY_WITHHELD`).
const FR_WITHHELD: &str = "Ma réponse n'est pas passée — elle mélangeait des détails techniques \
                           qui n'ont pas leur place ici. Renvoie ton dernier message et on \
                           reprend là où on en était.";

#[test]
fn a_withheld_row_is_dropped_from_the_replayed_prompt() {
    let history = vec![
        row("m1", "user", "c'est quoi mon plan?", None),
        row(
            "m2",
            "assistant",
            FR_WITHHELD,
            Some(WITHHELD_REPLY_FINISH_REASON),
        ),
        row("m3", "user", "et pour samedi?", None),
    ];

    let (messages, source_ids) = build_llm_messages(Some("sys"), &history);

    assert!(
        messages
            .iter()
            .all(|m| !m.content.contains("n'est pas passée")),
        "the withheld apology must not re-enter the prompt"
    );
    assert!(
        !source_ids.iter().any(|s| s.as_deref() == Some("m2")),
        "the withheld row must not appear in source_ids either"
    );
    // The surrounding athlete turns are untouched.
    assert!(messages.iter().any(|m| m.content.contains("mon plan")));
    assert!(messages.iter().any(|m| m.content.contains("samedi")));
}

#[test]
fn an_ordinary_assistant_row_with_the_same_text_shape_is_kept() {
    // The drop is keyed on the stamp, not on prose — a coach legitimately
    // saying something similar is still replayed. This is why the fix does not
    // add a sixth pattern table.
    let history = vec![
        row("m1", "user", "ça a marché?", None),
        row(
            "m2",
            "assistant",
            "Ton dernier message est bien passé, on reprend.",
            Some("stop"),
        ),
    ];

    let (messages, _) = build_llm_messages(Some("sys"), &history);

    assert!(
        messages.iter().any(|m| m.content.contains("bien passé")),
        "an unstamped assistant row must still replay"
    );
}

#[test]
fn every_locale_default_is_dropped_by_the_stamp_not_by_its_wording() {
    // Five locales, one mechanism. If this ever regresses to prose matching,
    // a reword in dravr-contremaitre silently reopens the poison loop.
    let locales = [
        FR_WITHHELD,
        "My reply didn't go through — it mixed in technical details that don't belong here.",
        "Mi respuesta no salió — mezclaba detalles técnicos que no tienen lugar aquí.",
        "Meine Antwort ging nicht raus — sie enthielt technische Details.",
        "A minha resposta não seguiu — misturava detalhes técnicos que não têm lugar aqui.",
        // Deliberately not a real locale string: the stamp is what matters.
        "totally different wording that no pattern table would ever match",
    ];
    for text in locales {
        let history = vec![
            row("m1", "user", "question", None),
            row("m2", "assistant", text, Some(WITHHELD_REPLY_FINISH_REASON)),
        ];
        let (messages, _) = build_llm_messages(Some("sys"), &history);
        assert!(
            messages.iter().all(|m| m.content != text),
            "stamped row must be dropped regardless of wording: {text:?}"
        );
    }
}
