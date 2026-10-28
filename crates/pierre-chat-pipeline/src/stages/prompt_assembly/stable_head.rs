// ABOUTME: Marks the unchanging head of the assembled system prompt so the provider can cache it apart
// ABOUTME: A prompt that no longer opens with its head is sent as one block, never split mid-content
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_llm::{ChatMessage, MessageRole};
use tracing::warn;

/// Name the system message's unchanging head, when the assembled prompt still
/// opens with it.
///
/// Every stage after the boundary appends, so the head is always a prefix
/// today; the check is what keeps a future stage that writes into the middle
/// from shipping a split that no longer marks the unchanging part. Without it
/// the message is sent as one block, with no boundary marked.
pub(super) fn mark_stable_head(messages: &mut [ChatMessage], stable_head: &str) {
    if let Some(system) = messages
        .first_mut()
        .filter(|message| message.role == MessageRole::System)
    {
        if system.content.starts_with(stable_head) {
            system.stable_prefix_len = Some(stable_head.len());
        } else {
            warn!(
                head_len = stable_head.len(),
                "assembled prompt no longer opens with its stable head; sent as one cache block"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use pierre_llm::{ChatMessage, MessageRole};

    use super::mark_stable_head;

    #[test]
    fn the_unchanging_head_is_named_without_moving_a_byte() {
        let head = "contract\n\npersona\n\ntools";
        let prompt = format!("{head}\n\nfollowups\n\nplan<!-- session-integrity:x -->");
        let mut messages = vec![ChatMessage::system(prompt.clone()), ChatMessage::user("hi")];
        mark_stable_head(&mut messages, head);
        assert_eq!(
            messages[0].content, prompt,
            "bug: marking the head changed the prompt bytes"
        );
        assert_eq!(
            messages[0].stable_prefix_len,
            Some(head.len()),
            "bug: a prompt opening with its head must name the head's length"
        );
        assert_eq!(
            messages[0].stable_split(),
            Some((head, &prompt[head.len()..])),
            "bug: the split must cut exactly at the head"
        );
        assert_eq!(
            messages[1].stable_prefix_len, None,
            "bug: only the system message carries a stable head"
        );
    }

    #[test]
    fn a_prompt_that_no_longer_opens_with_its_head_stays_one_block() {
        let mut messages = vec![ChatMessage::system("planner\n\ncontract\n\nrest")];
        mark_stable_head(&mut messages, "contract");
        assert_eq!(
            messages[0].stable_prefix_len, None,
            "bug: a prompt not opening with its head must stay one block"
        );

        let mut no_system = vec![ChatMessage::user("contract and more")];
        mark_stable_head(&mut no_system, "contract");
        assert_eq!(
            no_system[0].role,
            MessageRole::User,
            "bug: marking must never add a system message"
        );
        assert_eq!(
            no_system[0].stable_prefix_len, None,
            "bug: a user message never carries a stable head"
        );
    }
}
