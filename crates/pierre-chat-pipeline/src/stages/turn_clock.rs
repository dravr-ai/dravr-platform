// ABOUTME: The turn clock — today's local date, time and epoch table, sent in its own message each turn
// ABOUTME: Kept out of the cached system prompt, which carries only a stable pointer to it (carnet#895)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The turn clock.
//!
//! The clock changes every five minutes, so it stays out of the system block
//! the Anthropic tier caches: inside it, a follow-up turn that crosses a
//! quantum boundary re-writes ~20K prompt tokens (measured 2026-10-13,
//! carnet#895). The platform contract's `{{CURRENT_DATE}}` resolves to
//! [`CURRENT_DATE_POINTER`], a fixed sentence, and the clock rides as a
//! platform-authored `User` message inserted just before the athlete's latest
//! message after Tier-1 compaction, like the activity refresh, so it is never
//! summarized, never persisted and never replayed into a later turn.

use dravr_tronc::dates::civil_clock::{format_clock_stamp, monday_of, resolve_zone};
use pierre_llm::ChatMessage;

/// Opening sentence of the turn-clock message.
///
/// A platform marker like the grounding leads: `push_history_row` neutralizes
/// it in athlete-typed text, so an athlete cannot hand the model a clock of
/// their own.
pub const TURN_CLOCK_LEAD: &str = "Current date and time for this turn:";

/// What `{{CURRENT_DATE}}` resolves to in the platform contract.
///
/// Byte-identical on every turn, so the system block stays cacheable. It names
/// [`TURN_CLOCK_LEAD`] so the model can find the clock, and keeps the
/// contract's "the date above" wording meaningful.
pub const CURRENT_DATE_POINTER: &str = "The athlete's current local date and time, with the Unix \
     epochs to copy, are in the platform message that opens with \"Current date and time for \
     this turn:\", sent this turn ahead of their latest message. That is the date every \
     \"today\", \"yesterday\" or \"this week\" refers to.";

/// Insert the turn clock just before the athlete's latest message.
///
/// Must run AFTER Tier-1 compaction, as the activity refresh does: the clock is
/// then never summarized, and inserting it cannot desync the parallel
/// `source_ids` vector compaction consumes. Clamped to index >= 1 for the
/// refresh's reason: the system prompt must stay first, because
/// `non_system_count` and `sliding_window_to_fit` infer it from
/// `messages.first()`. Never past the end, so an empty list gets the clock
/// rather than a panic: unlike the refresh, this runs on every turn.
pub fn inject_turn_clock(llm_messages: &mut Vec<ChatMessage>, clock: &str) {
    let insert_pos = llm_messages
        .len()
        .saturating_sub(1)
        .max(1)
        .min(llm_messages.len());
    llm_messages.insert(
        insert_pos,
        ChatMessage::user(format!("{TURN_CLOCK_LEAD} {clock}")),
    );
}

/// Mark the end of the replayed conversation as a prompt-cache breakpoint.
///
/// Must run AFTER Tier-1 compaction and BEFORE the clock, the activity refresh
/// and every other per-turn message is inserted: at that point the list is the
/// system prompt, the replayed history and the athlete's latest message, so the
/// message before the latest one is the last thing the next turn replays
/// unchanged. Without the mark, the only cache entry inside the conversation
/// ends after this turn's clock and refresh, which the next turn never sends,
/// so every turn re-wrote its whole history to the cache: 40-66k tokens a turn
/// for a coach measured on 2026-11-14, charged to the athlete's quota
/// (carnet#1632). A conversation with no history yet has nothing to mark, and
/// the caller skips the first turn outright: there the message before the
/// latest one can be the startup grounding Stage 10 inserted, which no later
/// turn replays.
pub fn mark_history_cache_breakpoint(llm_messages: &mut [ChatMessage]) {
    let Some(end) = llm_messages.len().checked_sub(2).filter(|&end| end >= 1) else {
        return;
    };
    if let Some(message) = llm_messages.get_mut(end) {
        message.cache_breakpoint = true;
    }
}

/// The turn clock with the provider freshness hint after it, when there is one.
///
/// The hint names each stale provider's age, which moves as time passes, so it
/// rides this message for the clock's own reason: after the cache breakpoints,
/// never summarized, never replayed into a later turn.
#[must_use]
pub fn with_freshness(clock: String, freshness_hint: Option<String>) -> String {
    match freshness_hint {
        Some(hint) => format!("{clock}\n\n{hint}"),
        None => clock,
    }
}

/// Granularity the prompt's "now" values are floored to, in seconds.
///
/// Five minutes: short enough that the clock the agent reads is never
/// meaningfully wrong. The clock rides after the cache breakpoints, so the
/// quantum has no bearing on prompt caching. The date boundaries below are NOT
/// quantized — they are derived from the true local time, so "today" never
/// shifts.
const NOW_QUANTUM_SECS: i64 = 300;

// Divides `now`; a zero quantum would panic. Const-asserted rather than guarded
// at runtime, because the value is compile-time.
const _: () = assert!(
    NOW_QUANTUM_SECS > 0,
    "NOW_QUANTUM_SECS divides the clock and must be positive: bug in this constant"
);

/// The turn's clock: local date and time plus the epoch table the model copies.
///
/// Output shape: `YYYY-MM-DD <weekday> HH:MM (Continent/City)` when the user has
/// a valid IANA timezone, e.g. `2026-05-21 jeu 14:30 (America/Toronto)`. Falls
/// back to `(UTC)` when the timezone is `None` (no client has reported yet) or
/// fails to parse (e.g. a malformed string somehow landed in the column). Both
/// date and time are *local* to the user's tz, not the server's UTC clock —
/// that's the whole point of the anchor: when the user says "today" at 23:30
/// EDT, the prompt must say 2026-05-21 23:30, not the 2026-05-22 the server
/// clock has already rolled over to. The wall-clock time lets the agent reason
/// about time of day (morning/evening) without asking.
///
/// The weekday is named in `locale` for the same reason the epoch table below
/// precomputes every window boundary: the model gets calendar arithmetic wrong,
/// and a wrong weekday is the failure an athlete actually notices.
#[must_use]
pub fn format_current_date(user_timezone: Option<&str>, locale: &str) -> String {
    use dravr_tronc::dates::{CivilDate, Elapsed, Timestamp};
    let now_utc = Timestamp::now();
    let tz = resolve_zone(user_timezone);
    let local = tz.civil_at(now_utc);
    let label = tz.name();

    // Floored, never rounded up: rounding up crosses midnight and would move
    // "today" a day early at 23:58. Floor costs an upper bound up to one
    // quantum stale, which is immaterial here because the guidance below tells
    // the model to pass NO bounds for a freshness fetch — `now` is only ever a
    // `before` for a window question (today / this week / this month).
    let quantized_epoch = (now_utc.epoch_seconds() / NOW_QUANTUM_SECS) * NOW_QUANTUM_SECS;
    // The weekday is named here, never left to the model. Deriving "mercredi"
    // from 2026-09-02 is calendar arithmetic, which the epoch table below
    // already refuses to ask of the model for epochs; on 2026-09-02 an athlete
    // left the conversation after correcting weekday claims three times.
    let datetime_str = Timestamp::from_epoch_seconds(quantized_epoch)
        .ok()
        .map_or_else(
            || format_clock_stamp(now_utc, &tz, locale),
            |q| format_clock_stamp(q, &tz, locale),
        );
    let now_epoch = quantized_epoch;

    // Local-midnight epoch of a local date, DST-safe (a spring gap/fold at
    // midnight is vanishingly rare; take the earliest valid instant). Falls
    // back to `now` if construction ever fails so a boundary is never silently
    // wrong by years.
    let midnight_epoch = |date: CivilDate| -> i64 {
        tz.resolve(date.at_midnight())
            .earliest()
            .map_or(now_epoch, Timestamp::epoch_seconds)
    };
    let today0 = midnight_epoch(local.date());
    let yesterday0 = midnight_epoch(tz.date_at(now_utc - Elapsed::days(1)));
    // Monday-anchored week (the convention pierre_system.md already states).
    let week0 = midnight_epoch(monday_of(local.date()));
    // First of the current month, 00:00 local — the "ce mois" / "this month"
    // window, which is otherwise a from-scratch date→epoch conversion.
    let month0 = local
        .date()
        .with_day(1)
        .map(CivilDate::at_midnight)
        .and_then(|naive| tz.resolve(naive).earliest())
        .map_or(now_epoch, Timestamp::epoch_seconds);

    // A full epoch reference table so the model COPIES every common boundary
    // instead of converting an absolute date to an epoch — the arithmetic LLMs
    // get wrong. 2026-07-24: an agent reading the human date still passed
    // `before=1753362000` (2025-07-24, a year early) and served year-old data;
    // the same failure lurks on any window bound (`after=<Monday 00:00>` etc.),
    // so every boundary is precomputed here. Local to the user's tz, not the
    // server's UTC clock. The guidance is inline so it holds regardless of the
    // surrounding prompt template.
    format!(
        "{datetime_str} ({label}).\n\
         Unix epochs (seconds) — COPY these; never convert a date to an epoch yourself (it is error-prone):\n\
         - now = {now_epoch}\n\
         - start of today (00:00 local) = {today0}\n\
         - start of yesterday (00:00 local) = {yesterday0}\n\
         - start of this week (Monday 00:00 local) = {week0}\n\
         - start of this month (1st, 00:00 local) = {month0}\n\
         For a freshness/recent fetch pass no `after`/`before` (the server returns your newest \
         activities first). For a window question (today / hier / cette semaine / ce mois) pass \
         `after` = the matching start above and `before` = now (count only the rows whose start \
         falls inside the window)."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(messages: &[ChatMessage]) -> Vec<usize> {
        messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.cache_breakpoint)
            .map(|(i, _)| i)
            .collect()
    }

    /// The mark lands on the last replayed message, and the clock inserted
    /// after it stays outside the cached history: the next turn replays the
    /// history unchanged and reads it back (carnet#1632).
    #[test]
    fn the_last_replayed_message_is_marked_before_the_clock_lands_after_it() {
        let mut messages = vec![
            ChatMessage::system("persona"),
            ChatMessage::user("how was my week?"),
            ChatMessage::assistant("Solid: four rides."),
            ChatMessage::user("and today?"),
        ];
        mark_history_cache_breakpoint(&mut messages);
        inject_turn_clock(&mut messages, "Tuesday 14:54");
        assert_eq!(marked(&messages), [2]);
        assert!(messages[3].content.starts_with(TURN_CLOCK_LEAD));
        assert_eq!(messages[4].content, "and today?");
    }

    /// A first turn has no history: the system prompt keeps its own
    /// breakpoints and nothing else is marked.
    #[test]
    fn a_turn_without_history_marks_nothing() {
        let mut first = vec![ChatMessage::system("persona"), ChatMessage::user("hi")];
        mark_history_cache_breakpoint(&mut first);
        assert_eq!(marked(&first), Vec::<usize>::new());

        let mut lone = vec![ChatMessage::user("hi")];
        mark_history_cache_breakpoint(&mut lone);
        assert_eq!(marked(&lone), Vec::<usize>::new());
    }
}
