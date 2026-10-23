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

use pierre_core::civil_time::{format_clock_stamp, resolve_zone};
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
    use chrono::{Datelike, Duration, TimeZone, Utc};
    let now_utc = Utc::now();
    let tz = resolve_zone(user_timezone);
    let local = now_utc.with_timezone(&tz);
    let label = tz.name();

    // Floored, never rounded up: rounding up crosses midnight and would move
    // "today" a day early at 23:58. Floor costs an upper bound up to one
    // quantum stale, which is immaterial here because the guidance below tells
    // the model to pass NO bounds for a freshness fetch — `now` is only ever a
    // `before` for a window question (today / this week / this month).
    let quantized_epoch = (now_utc.timestamp() / NOW_QUANTUM_SECS) * NOW_QUANTUM_SECS;
    // The weekday is named here, never left to the model. Deriving "mercredi"
    // from 2026-09-02 is calendar arithmetic, which the epoch table below
    // already refuses to ask of the model for epochs; on 2026-09-02 an athlete
    // left the conversation after correcting weekday claims three times.
    let datetime_str = Utc.timestamp_opt(quantized_epoch, 0).single().map_or_else(
        || format_clock_stamp(local.with_timezone(&Utc), tz, locale),
        |q| format_clock_stamp(q, tz, locale),
    );
    let now_epoch = quantized_epoch;

    // Local-midnight epoch `days_back` days before today, DST-safe (a spring
    // gap/fold at midnight is vanishingly rare; take the earliest valid
    // instant). Falls back to `now` if construction ever fails so a boundary
    // is never silently wrong by years.
    let midnight_epoch = |days_back: i64| -> i64 {
        (local - Duration::days(days_back))
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|naive| tz.from_local_datetime(&naive).earliest())
            .map_or(now_epoch, |dt| dt.timestamp())
    };
    let today0 = midnight_epoch(0);
    let yesterday0 = midnight_epoch(1);
    // Monday-anchored week (the convention pierre_system.md already states).
    let days_since_monday = i64::from(local.weekday().num_days_from_monday());
    let week0 = midnight_epoch(days_since_monday);
    // First of the current month, 00:00 local — the "ce mois" / "this month"
    // window, which is otherwise a from-scratch date→epoch conversion.
    let month0 = local
        .date_naive()
        .with_day(1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|naive| tz.from_local_datetime(&naive).earliest())
        .map_or(now_epoch, |dt| dt.timestamp());

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
