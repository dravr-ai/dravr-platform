// ABOUTME: Recognises the turn's language-classification call and answers it as an agreeing model would
// ABOUTME: For scripted mock LLMs whose replies must go to the coaching turn, not to the locale check
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The locale check a short message triggers.
//!
//! `resolve_turn_locale` asks the turn's own provider which language a short
//! message is in when lingua's verdict disagrees with the stored locale. A
//! mock with a sequential script would spend its first scripted reply on that
//! call; [`language_classification_reply`] lets it answer the call first and
//! keep its script for the turn.

use pierre_chat_pipeline::language::detect_locale;
use pierre_core::llm::ChatRequest;
use pierre_llm::prompts::LANGUAGE_CLASSIFICATION_PROMPT;

/// Where the rendered prompt puts the message it asks about.
const MESSAGE_MARKER: &str = "MESSAGE:";

/// The opening line of the compiled-in prompt: present in every rendered
/// classification request and in nothing a coaching turn sends.
fn classification_fragment() -> &'static str {
    LANGUAGE_CLASSIFICATION_PROMPT
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .expect("the language_classification prompt has a first line")
}

/// The answer to `request` when it is a language-classification call, `None`
/// for every other call.
///
/// The answer is what a model agreeing with lingua would say: the locale
/// lingua reads in the message, or `other` when it reads none.
pub(crate) fn language_classification_reply(request: &ChatRequest) -> Option<String> {
    let fragment = classification_fragment();
    let prompt = request
        .messages
        .iter()
        .map(|m| m.content.as_str())
        .find(|content| content.contains(fragment))?;
    let message = prompt
        .rsplit_once(MESSAGE_MARKER)
        .map_or(prompt, |(_, after)| after)
        .trim();
    Some(detect_locale(message).unwrap_or("other").to_owned())
}
