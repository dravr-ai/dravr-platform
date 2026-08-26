// ABOUTME: Pins sentence-boundary reply chunking — every message fits, nothing is lost, no split word
// ABOUTME: The regression guard for registre#2, where an over-limit reply was dropped instead of split

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Chunking a reply the transport will not carry whole.
//!
//! Four properties, asserted on concrete values rather than on "it returned
//! something": every message is inside the ceiling, the messages concatenate
//! back to the reply, no message begins or ends in the middle of a word, and
//! the split lands after a sentence wherever one was available.

use std::fmt::Write as _;

use pierre_core::chunking::chunk_reply;

/// Discord's real ceiling. Every messaging ceiling in the platform comes from
/// canot's channel descriptors; 2000 is the smallest of them and therefore
/// the one that splits most often.
const DISCORD: usize = 2000;

/// A coach paragraph of `sentences` sentences, each exactly 80 characters
/// including the space that follows it.
fn coach_reply(sentences: usize) -> String {
    let mut out = String::new();
    for n in 1..=sentences {
        let _ = write!(
            out,
            "Sentence number {n:04} of this reply about your training load and recovery trend. "
        );
    }
    out.trim_end().to_owned()
}

/// No message may exceed the ceiling.
fn assert_all_fit(messages: &[String], ceiling: usize) {
    for (index, message) in messages.iter().enumerate() {
        assert!(
            message.chars().count() <= ceiling,
            "message {index} is {} characters, over the {ceiling} ceiling: {message}",
            message.chars().count()
        );
    }
}

/// Every non-whitespace character of the original must survive, in order.
/// Whitespace that sat exactly on a split point is consumed by the split.
fn assert_nothing_lost(messages: &[String], original: &str) {
    let rejoined: String = messages
        .concat()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let expected: String = original.chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(
        rejoined, expected,
        "the reply must survive the split intact"
    );
}

/// No message may start or end mid-word: the original text must contain each
/// message's first and last word as whole words.
fn assert_no_word_split(messages: &[String], original: &str) {
    let words: Vec<&str> = original.split_whitespace().collect();
    for (index, message) in messages.iter().enumerate() {
        let first = message.split_whitespace().next().unwrap();
        let last = message.split_whitespace().next_back().unwrap();
        assert!(
            words.contains(&first),
            "message {index} opens on a fragment: {first}"
        );
        assert!(
            words.contains(&last),
            "message {index} ends on a fragment: {last}"
        );
    }
}

#[test]
fn a_reply_inside_the_ceiling_is_one_message() {
    let reply = "Nice ride. Your load is trending up, and your sleep held.";

    let messages = chunk_reply(reply, DISCORD);

    assert_eq!(messages, vec![reply.to_owned()]);
}

#[test]
fn a_blank_reply_produces_no_messages() {
    assert_eq!(chunk_reply("   \n\n  ", DISCORD), Vec::<String>::new());
    assert_eq!(chunk_reply("", DISCORD), Vec::<String>::new());
}

#[test]
fn the_in_app_ceiling_never_splits() {
    let reply = coach_reply(400);

    let messages = chunk_reply(&reply, usize::MAX);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].chars().count(), reply.chars().count());
}

/// The headline case: a long reply against Discord's real ceiling.
///
/// 75 sentences of 80 characters is 5999 characters — exactly three Discord
/// messages once the split is confined to sentence boundaries.
#[test]
fn an_over_limit_reply_becomes_ordered_messages_that_all_fit() {
    let reply = coach_reply(75);
    assert_eq!(
        reply.chars().count(),
        5999,
        "fixture length is load-bearing"
    );

    let messages = chunk_reply(&reply, DISCORD);

    assert_eq!(
        messages.len(),
        3,
        "5999 characters at 2000 per message, split only between sentences"
    );
    assert_all_fit(&messages, DISCORD);
    assert_nothing_lost(&messages, &reply);
    assert_no_word_split(&messages, &reply);
    assert!(
        messages[0].starts_with("Sentence number 0001"),
        "the first message must open the reply: {}",
        &messages[0][..40]
    );
    assert!(
        messages[2].ends_with("recovery trend."),
        "the last message must close the reply"
    );
}

/// Each ceiling is the channel's own, so the same reply splits differently per
/// surface. Telegram's 4096 and Slack's 40000 are the other two real numbers.
#[test]
fn the_ceiling_that_splits_is_the_channels_own_not_a_constant() {
    let reply = coach_reply(75);

    let discord = chunk_reply(&reply, 2000);
    let telegram = chunk_reply(&reply, 4096);
    let slack = chunk_reply(&reply, 40_000);

    assert_eq!(discord.len(), 3);
    assert_eq!(telegram.len(), 2);
    assert_eq!(
        slack.len(),
        1,
        "Slack carries the whole reply in one message"
    );
    assert_all_fit(&discord, 2000);
    assert_all_fit(&telegram, 4096);
}

/// A split lands *after* a sentence when one is available, so a message never
/// hands the athlete half a thought.
#[test]
fn the_split_lands_after_a_sentence() {
    let reply = coach_reply(75);

    let messages = chunk_reply(&reply, DISCORD);

    for (index, message) in messages.iter().enumerate() {
        assert!(
            message.ends_with('.'),
            "message {index} must end on a completed sentence: …{}",
            &message[message.len().saturating_sub(40)..]
        );
    }
}

/// A period inside a number or a domain is not a sentence end. A decimal must
/// never become a split point.
#[test]
fn a_decimal_point_is_not_a_sentence_boundary() {
    // Two "sentences" whose only periods are decimals, plus one real one.
    let reply = format!(
        "{} Ton allure moyenne est 4.35 min/km sur dravr.ai cette semaine.",
        "x".repeat(1990)
    );

    let messages = chunk_reply(&reply, DISCORD);

    assert_eq!(messages.len(), 2);
    assert!(
        messages[1].contains("4.35 min/km"),
        "the decimal must stay whole: {}",
        messages[1]
    );
    assert!(
        messages[1].contains("dravr.ai"),
        "the domain must stay whole: {}",
        messages[1]
    );
}

/// A line-broken list has no sentence punctuation to split on, so the newline
/// is the boundary.
#[test]
fn a_bulleted_list_splits_between_lines() {
    let mut reply = String::new();
    for n in 1..=40 {
        let _ = writeln!(
            reply,
            "• Activity {n:03} · 2026-08-{:02} · Run · 10.0 km",
            n % 28 + 1
        );
    }

    let messages = chunk_reply(&reply, 200);

    assert_all_fit(&messages, 200);
    assert_nothing_lost(&messages, &reply);
    for message in &messages {
        assert!(
            message.starts_with('•'),
            "a message must open on a whole bullet: {message}"
        );
        assert!(
            message.ends_with("km"),
            "a message must close on a whole bullet: {message}"
        );
    }
}

/// A single sentence longer than the whole ceiling still has to go out. It is
/// split between words — never inside one.
#[test]
fn one_enormous_sentence_splits_between_words() {
    let reply = format!("{} done", "word ".repeat(900));
    assert!(reply.chars().count() > DISCORD);

    let messages = chunk_reply(&reply, DISCORD);

    assert_eq!(messages.len(), 3);
    assert_all_fit(&messages, DISCORD);
    assert_nothing_lost(&messages, &reply);
    for message in &messages {
        assert!(
            !message.starts_with(' ') && !message.ends_with(' '),
            "a message carries no boundary whitespace"
        );
        for word in message.split_whitespace() {
            assert!(
                word == "word" || word == "done",
                "no word may be cut in half, got {word}"
            );
        }
    }
}

/// A pathological token longer than the ceiling has no word boundary to use.
/// It is cut on character boundaries rather than becoming a message no
/// transport will accept.
#[test]
fn a_token_longer_than_the_ceiling_is_cut_on_character_boundaries() {
    let token = "é".repeat(300);

    let messages = chunk_reply(&token, 100);

    assert_eq!(messages.len(), 3);
    assert_all_fit(&messages, 100);
    assert_eq!(messages.concat(), token, "multi-byte characters stay whole");
}
