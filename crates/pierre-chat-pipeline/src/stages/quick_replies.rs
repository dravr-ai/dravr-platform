// ABOUTME: Lifts the model's dravr-actions fence out of a reply: up to three replies the athlete might send next
// ABOUTME: Feeds next_steps ahead of the catalogue's transitions; an invalid item is dropped, never truncated

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Quick replies (carnet#1486).
//!
//! A reply that ends on a choice may close on a fenced block the model
//! writes:
//!
//! ````text
//! ```dravr-actions
//! ["Easy spin tomorrow", "Rest day"]
//! ```
//! ````
//!
//! Post-processing lifts every such fence out of the reply, on every surface
//! and for every agent, before the claim check: a quick reply is what the
//! athlete might say next, not a statement by the coach. The valid items
//! become next steps ([`crate::next_steps`]) whose postback is
//! `qr:<slot>:<text>` ([`crate::suggestions`]); a tap sends the words as the
//! athlete's message.

use std::collections::HashSet;

use pierre_contremaitre::use_case_catalogue::MAX_POSTBACK_BYTES;
use tracing::debug;

use super::viz_blocks::next_fence;
use crate::next_steps::MAX_NEXT_STEPS;
use crate::suggestions::{Postback, QR_PREFIX};

/// Info string that marks a fenced block as the model's quick replies.
pub(super) const FENCE_INFO: &str = "dravr-actions";

/// The longest quick reply, in bytes: what fits in Telegram's 64-byte
/// `callback_data` after `qr:<slot>:` with a one-digit slot. Derived, so the
/// limit has one definition.
pub(crate) const MAX_QUICK_REPLY_BYTES: usize = MAX_POSTBACK_BYTES - QR_PREFIX.len() - "0:".len();

/// A reply with its quick-reply fences lifted out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QuickReplies {
    /// The reply without any `dravr-actions` fence, trimmed when one was cut.
    pub text: String,
    /// The valid items, in order, deduplicated, at most [`MAX_NEXT_STEPS`].
    pub candidates: Vec<String>,
}

/// Lift every `dravr-actions` fence out of `reply`: the text without them,
/// and the items they offer that may be sent as the athlete's words.
pub(crate) fn lift_quick_replies(reply: &str) -> QuickReplies {
    if !reply.contains(FENCE_INFO) {
        return QuickReplies {
            text: reply.to_owned(),
            candidates: Vec::new(),
        };
    }
    let mut text = String::with_capacity(reply.len());
    let mut candidates: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut rest = reply;
    let mut found = false;
    while let Some(fence) = next_fence(rest, FENCE_INFO) {
        found = true;
        text.push_str(&rest[..fence.start]);
        for item in items(fence.body) {
            if candidates.len() < MAX_NEXT_STEPS && seen.insert(item.to_lowercase()) {
                candidates.push(item);
            }
        }
        rest = &rest[fence.end..];
    }
    if !found {
        return QuickReplies {
            text: reply.to_owned(),
            candidates: Vec::new(),
        };
    }
    text.push_str(rest);
    QuickReplies {
        text: text.trim().to_owned(),
        candidates,
    }
}

/// The items of one fence's body that may be offered; a body that is not a
/// JSON array of strings offers none (logged).
fn items(body: &str) -> Vec<String> {
    match serde_json::from_str::<Vec<String>>(body.trim()) {
        Ok(items) => items
            .into_iter()
            .map(|item| item.trim().to_owned())
            .filter(|item| offerable(item))
            .collect(),
        Err(e) => {
            debug!(error = %e, "a dravr-actions fence is not a JSON array of strings; nothing offered");
            Vec::new()
        }
    }
}

/// Whether `item` may be offered as the athlete's next words: one line, 1 to
/// [`MAX_QUICK_REPLY_BYTES`] bytes, no control character, no slash command,
/// and nothing that already reads as a postback. Too long is dropped, never
/// truncated: truncating would change what the athlete sends.
fn offerable(item: &str) -> bool {
    !item.is_empty()
        && item.len() <= MAX_QUICK_REPLY_BYTES
        && !item.chars().any(char::is_control)
        && !item.starts_with('/')
        && Postback::parse(item).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fenced(body: &str) -> String {
        format!("Easy week ahead.\n\n```dravr-actions\n{body}\n```")
    }

    #[test]
    fn a_json_array_is_lifted_into_candidates() {
        let lifted = lift_quick_replies(&fenced(r#"["Easy spin tomorrow", "Rest day"]"#));
        assert_eq!(
            lifted.text, "Easy week ahead.",
            "bug: the fence must leave the text"
        );
        assert_eq!(
            lifted.candidates,
            vec!["Easy spin tomorrow".to_owned(), "Rest day".to_owned()],
            "bug: the items must be offered in order"
        );
    }

    #[test]
    fn a_body_that_is_not_json_is_stripped_and_offers_nothing() {
        let lifted = lift_quick_replies(&fenced("Easy spin, Rest day"));
        assert_eq!(
            lifted.text, "Easy week ahead.",
            "bug: a bad fence must still be stripped"
        );
        assert!(
            lifted.candidates.is_empty(),
            "bug: a bad body must offer nothing"
        );
    }

    #[test]
    fn commands_postbacks_and_long_items_are_dropped_never_truncated() {
        let longest = "a".repeat(MAX_QUICK_REPLY_BYTES);
        let too_long = "b".repeat(MAX_QUICK_REPLY_BYTES + 1);
        let lifted = lift_quick_replies(&fenced(&format!(
            r#"["/plan", "nx:0:season", "qr:1:Rest day", "{too_long}", "{longest}"]"#
        )));
        assert_eq!(
            lifted.candidates,
            vec![longest],
            "bug: only the item that fits and is plain words may be offered"
        );
        assert_eq!(
            MAX_QUICK_REPLY_BYTES, 59,
            "bug: Telegram's 64 bytes less qr:<slot>:"
        );
    }

    #[test]
    fn a_fourth_item_and_a_duplicate_are_dropped() {
        let lifted = lift_quick_replies(&fenced(r#"["One", "one", "Two", "Three", "Four"]"#));
        assert_eq!(
            lifted.candidates,
            vec!["One".to_owned(), "Two".to_owned(), "Three".to_owned()],
            "bug: at most three, case-folded duplicates removed"
        );
    }

    #[test]
    fn a_fence_inside_another_fence_is_not_opened() {
        let reply = "Here:\n```json\n```dravr-actions\n[\"No\"]\n```\n```";
        let lifted = lift_quick_replies(reply);
        assert!(
            lifted.candidates.is_empty(),
            "bug: a dravr-actions mention inside a json fence must not open one"
        );
    }

    #[test]
    fn two_fences_are_both_stripped() {
        let reply = "A\n```dravr-actions\n[\"Yes\"]\n```\nB\n```dravr-actions\n[\"No\"]\n```";
        let lifted = lift_quick_replies(reply);
        assert_eq!(lifted.text, "A\n\nB", "bug: every fence must be stripped");
        assert_eq!(
            lifted.candidates,
            vec!["Yes".to_owned(), "No".to_owned()],
            "bug: both fences' items must be offered"
        );
    }
}
