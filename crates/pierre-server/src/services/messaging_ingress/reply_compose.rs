// ABOUTME: Compose a messaging reply by prepending the get_activities list (adaptively
// ABOUTME: capped for small screens) to the coach's analysis, mirroring the web/mobile card.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Messaging reply composition.
//!
//! `get_activities` returns a pre-formatted `activity_list` prose block, and the
//! web/mobile UI renders it as a "Your Activities" card ABOVE the coach's
//! analysis. Text channels (Telegram/WhatsApp/…) have no such card, so the
//! coach prompt's "the surface already shows the list" assumption breaks and the
//! list is never seen. This module re-establishes that contract for messaging:
//! it prepends the list to the coach's analysis, adaptively capped so a long
//! history doesn't overflow a small screen.

use pierre_contremaitre::messaging_strings::{MessagingStringsRegistry, KEY_BACKFILL_LIST_MORE};

/// At or below this many activities the full list is shown; above it, the list
/// collapses to the top entries + a localized "…and N more" footer so a long
/// history (e.g. 186 rows) doesn't overflow the chat.
const MESSAGING_FULL_LIST_THRESHOLD: usize = 20;

/// Number of entries kept when a long list is collapsed.
const MESSAGING_LIST_TOP_N: usize = 12;

/// Prepend the rendered activity list to the coach's analysis for a messaging
/// channel, so the user actually SEES their activities.
///
/// The web/mobile UI auto-renders this list as a card; text channels must carry
/// it inline. Returns `content` unchanged when there is no list. When the coach
/// produced only the list (empty analysis), the list stands alone. The list is
/// adaptively capped for small screens via [`cap_activity_list_for_channel`].
pub fn compose_messaging_reply(
    activity_list: Option<&str>,
    content: String,
    strings: &MessagingStringsRegistry,
    locale: &str,
) -> String {
    match activity_list {
        Some(list) if !list.trim().is_empty() => {
            // Drop the prose's English "Your Activities:" header. Web/mobile show
            // it as a card header, but on a (often non-English) text channel it's
            // redundant noise above the entries — the coach analysis that follows
            // gives the context.
            let list = list
                .strip_prefix("Your Activities:")
                .map_or(list, str::trim_start);
            let capped = cap_activity_list_for_channel(list, strings, locale);
            if content.trim().is_empty() {
                capped
            } else {
                format!("{capped}\n\n{content}")
            }
        }
        _ => content,
    }
}

/// Adaptively cap a `get_activities` `activity_list` prose block for a small
/// screen.
///
/// The prose is a header/notes block followed by numbered entries
/// (`"12. [Run] … - 2026-05-15 - 10.32 km - …"`). When there are more than
/// [`MESSAGING_FULL_LIST_THRESHOLD`] entries, keep the header + the first
/// [`MESSAGING_LIST_TOP_N`] entries (already ordered by the request's `sort_by`)
/// and replace the tail with a localized "…and N more" footer. Otherwise return
/// the prose unchanged.
fn cap_activity_list_for_channel(
    prose: &str,
    strings: &MessagingStringsRegistry,
    locale: &str,
) -> String {
    let total_entries = prose.lines().filter(|l| is_numbered_entry(l)).count();
    if total_entries <= MESSAGING_FULL_LIST_THRESHOLD {
        return prose.to_owned();
    }

    let mut kept = 0usize;
    let mut out: Vec<&str> = Vec::with_capacity(MESSAGING_LIST_TOP_N + 4);
    for line in prose.lines() {
        if is_numbered_entry(line) {
            // Keep the first N entries (in the caller's sort order); the rest are
            // summarized by the footer below.
            if kept < MESSAGING_LIST_TOP_N {
                out.push(line);
                kept += 1;
            }
        } else {
            out.push(line);
        }
    }

    let remaining = (total_entries - MESSAGING_LIST_TOP_N).to_string();
    let footer = strings.render(KEY_BACKFILL_LIST_MORE, locale, &[&remaining]);
    let mut result = out.join("\n");
    result.push('\n');
    result.push_str(&footer);
    result
}

/// Whether a prose line is a numbered activity entry (`"<digits>. [<sport>] …"`),
/// as emitted by `format_activities_as_list`. Header and `[Note]` lines are not.
fn is_numbered_entry(line: &str) -> bool {
    line.split_once(". [")
        .is_some_and(|(prefix, _)| !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_digit()))
}
