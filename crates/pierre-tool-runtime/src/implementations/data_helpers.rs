// ABOUTME: Shared vocabulary for the data tools — coverage notes, cache eligibility
// ABOUTME: Split out of data.rs, which was frozen over the size ceiling and could not grow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Helpers shared by the data-access tools.
//!
//! These were defined inside `data.rs` next to `GetActivitiesTool`, but most of
//! them are read from other files, so they were already a shared vocabulary
//! living in one tool's implementation.
//! Splitting them out is what the file-size gate asks for when a frozen file
//! needs to change, and it puts each item where its callers can see it.

use std::env;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};

use serde_json::{json, Value};

use pierre_core::models::coverage::{
    Completeness, Coverage, CoverageWindow, IncompleteReason, Pending, WindowScope, WindowTotals,
};
use pierre_core::models::TenantId;
use pierre_formatters::OutputFormat;
use pierre_providers::backend_resolver::user_facing_name;
#[cfg(feature = "tools-data")]
use pierre_providers::core::ActivityQueryParams;
#[cfg(feature = "tools-data")]
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use pierre_services::provider_refresh::UNREAD_PROVIDER_DIRECTIVE;
use uuid::Uuid;

#[cfg(feature = "tools-data")]
use crate::activity_fetch::{grant_window, serve_without_primary};
use crate::activity_fetch::{FallbackServe, StaleReason};
#[cfg(feature = "tools-data")]
use crate::context::ToolExecutionContext;
use crate::runtime::ToolRuntime;

/// How far forward the coverage probe looks from `after`.
///
/// When a historical `get_activities` query leaves `before` open, the backfill
/// gate checks cache coverage over `[after, after + 1 year]` so recent rows in
/// `[after, now]` don't mask a missing historical season.
pub const HISTORICAL_COVERAGE_BOUND_SECS: i64 = 365 * 24 * 60 * 60;

pub use crate::activity_fetch::HISTORICAL_WINDOW_READ_LIMIT;

/// Read limit (as `usize`) for the historical backfill window read.
#[must_use]
pub const fn historical_window_read_limit() -> usize {
    HISTORICAL_WINDOW_READ_LIMIT
}

/// Default fetch limit a background backfill passes to the provider so the scrape
/// pages the WHOLE requested window instead of stopping at the user's display
/// limit. The sciotte date-bounded scrape stops on `in_window_count >= limit`
/// OR `oldest <= after`; with a small (display) limit it caps at the recent tail
/// and never reaches the window start. A generous limit makes `oldest <= after`
/// the binding condition, so the scrape pages the full season.
const DEFAULT_HISTORICAL_BACKFILL_FETCH_LIMIT: usize = 2_000;

/// Fetch limit a background backfill requests.
///
/// Read from `PIERRE_HISTORICAL_BACKFILL_FETCH_LIMIT`, falling back to
/// [`DEFAULT_HISTORICAL_BACKFILL_FETCH_LIMIT`]. Operators raise this for
/// athletes with very deep histories; zero or unparseable values fall back to
/// the default.
#[must_use]
pub fn historical_backfill_fetch_limit() -> usize {
    env::var("PIERRE_HISTORICAL_BACKFILL_FETCH_LIMIT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_HISTORICAL_BACKFILL_FETCH_LIMIT)
}

/// How many activities to ask the provider for.
///
/// A window bounded on both ends ("my 2011 rides") is read whole: `limit` only
/// bounds what is shown, and the provider is asked for up to
/// [`historical_backfill_fetch_limit`]. Asking for `limit` instead returned the
/// newest `limit` activities of every sport, so a sport filter left a 63-ride
/// year as its last three December rides (carnet#874). A paged read (an
/// `offset`) and an open-ended window keep `limit`.
#[must_use]
pub fn window_fetch_limit(
    limit: usize,
    after: Option<i64>,
    before: Option<i64>,
    offset: Option<usize>,
) -> usize {
    if after.is_some() && before.is_some() && offset.unwrap_or(0) == 0 {
        historical_backfill_fetch_limit().max(limit)
    } else {
        limit
    }
}

/// Whether more activities exist than the response shows.
///
/// True when the window held more than is shown, or when the provider filled
/// the page it was asked for, so the window may extend past it. Judged on the
/// shown count alone, a sport filter that left three of twenty rows read as
/// "nothing more" (carnet#874).
#[must_use]
pub const fn pagination_has_more(
    returned_count: usize,
    window_total: usize,
    provider_filled_page: bool,
) -> bool {
    window_total > returned_count || provider_filled_page
}

/// Whether `provider`'s connection is in an unusable state (`NeedsReauth` /
/// `Revoked`) and so needs an interactive reconnect.
///
/// The historical gate consults this BEFORE spawning a background backfill: a
/// dead session would just fail the scrape and leave the user looping on
/// "fetching, ask again shortly". When true, `get_activities` returns
/// `provider_auth_required` instead, so the chat pipeline hands back the
/// reconnect link this turn.
///
/// Re-exported from [`pierre_core::models::connection_needs_reauth`] — the
/// single home for the scan, shared with `AuthService`. Kept re-exported here
/// so the reconnect-gate integration test can exercise the decision via this
/// module.
pub use pierre_core::models::connection_needs_reauth;

/// Whether a `get_activities` query may use the `CacheKey` response cache.
///
/// One decision governs BOTH the read short-circuit and the write-through, so
/// the two never diverge. The cache holds summaries only; a hit details the
/// recent window from the stored details, and reads no provider. Three query
/// shapes must bypass it:
///
/// * `explicit_detail` — an explicit `mode=detailed` reads the detail of the
///   older rows from the provider, which a hit never does, and the cache key
///   omits `mode`, so a summary call's entry would answer it without them.
/// * `is_historical` — a deep historical window must route through the
///   coverage-aware gate, never a TTL'd response that could keep serving a stale
///   slice after the coverage was purged. Excluding it from the WRITE too keeps
///   the cache from accumulating historical entries that are never read back.
/// * `is_custom_sort` — the cache key omits `sort_by`, and the cached-serve path
///   re-orders by date, so a cached entry cannot satisfy a "longest/oldest/…"
///   ask. A non-default sort therefore bypasses the cache on both read and write.
///
/// `pub` so the read/write contract is exercisable by the integration test suite.
#[must_use]
pub const fn response_cache_eligible(
    explicit_detail: bool,
    is_historical: bool,
    is_custom_sort: bool,
) -> bool {
    !explicit_detail && !is_historical && !is_custom_sort
}

/// The window a `get_activities` call asked for, as epoch seconds.
#[derive(Debug, Clone, Copy, Default)]
pub struct AskedWindow {
    /// The call's `after`, when it set one.
    pub after: Option<i64>,
    /// The call's `before`, when it set one.
    pub before: Option<i64>,
}

/// Build the LLM-facing `coverage` sidecar for a served activity window.
///
/// Present whenever the window's total is known or the window is incomplete.
/// It says what was asked (`window.asked_from`/`asked_to`), what was served
/// (`window.oldest`/`newest`, `window.total`, `returned`, and the window's
/// `totals` when they were summed), and whether that is the whole truth for
/// the ask (`complete`, with `incomplete_reason` when it is not). A count
/// stated from an incomplete window is not a fact: the note says so in the
/// words a model acts on (carnet#1481: 43 trail runs since 2020, counted from
/// a 151-row cached window, were stated as fact). A window that filled the
/// durable read cap holds at least its total, never exactly it.
///
/// Lives in the tool RESULT, not the tool's `input_schema`/description, so it
/// adds no contremaitre drift; the model renders it in the user's language,
/// so it needs no localized `messaging_strings` key.
#[must_use]
pub fn activity_coverage(
    window_total: Option<usize>,
    returned: usize,
    window_span: Option<(NaiveDate, NaiveDate)>,
    asked: AskedWindow,
    completeness: Completeness,
    totals: Option<WindowTotals>,
) -> Option<Coverage> {
    if window_total.is_none() && completeness.is_complete() {
        return None;
    }
    let total = window_total.unwrap_or(returned);
    if total <= returned && completeness.is_complete() && asked.after.is_none() {
        return None;
    }
    let count = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
    let day = |epoch: Option<i64>| {
        epoch
            .and_then(|e| DateTime::<Utc>::from_timestamp(e, 0))
            .map(|at| at.date_naive())
    };
    Some(Coverage {
        completeness,
        window: CoverageWindow {
            scope: WindowScope::Asked,
            asked_from: day(asked.after),
            asked_to: day(asked.before),
            oldest: window_span.map(|(oldest, _)| oldest),
            newest: window_span.map(|(_, newest)| newest),
            total: Some(count(total)),
            total_is_lower_bound: total >= HISTORICAL_WINDOW_READ_LIMIT,
            returned: count(returned),
            totals,
        },
        trustworthy_from: None,
        pending: None,
    })
}

/// Build the `coverage` a `get_segment_history` answer carries
/// ([`WindowScope::SegmentsRead`]).
///
/// The window asked, `[asked_from, asked_to]`, with `window.total` the
/// activities in it and `window.returned` those whose segment efforts were
/// read; `None` for a total no read counted (Strava's live listing names
/// only the activities it holds efforts of). Always present, unlike
/// [`activity_coverage`]: a segment's efforts are never more than the
/// activities read, so a model must always see how many those were.
#[must_use]
pub(crate) fn segments_read_coverage(
    activities: Option<usize>,
    read: usize,
    asked: (NaiveDate, NaiveDate),
    completeness: Completeness,
    pending: Option<Pending>,
) -> Coverage {
    let count = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
    Coverage {
        completeness,
        window: CoverageWindow {
            scope: WindowScope::SegmentsRead,
            asked_from: Some(asked.0),
            asked_to: Some(asked.1),
            total: activities.map(count),
            returned: count(read),
            ..CoverageWindow::default()
        },
        trustworthy_from: None,
        pending,
    }
}

/// Build the `reconnect_required` sidecar for a window that was served WITHOUT
/// a connection the athlete has to re-authorize.
///
/// A multi-source aggregator serves what it holds and prompts for the dead
/// source alongside: an athlete with years of Strava behind a healthy connection
/// is owed that answer even while their watch token is expired. This sidecar is
/// how the reconnect signal survives the merge instead of being the only thing
/// the turn manages to say.
///
/// Lives in the tool RESULT, not the tool's `input_schema`/description, so it
/// adds no SDK or contremaitre drift. `pub` so the caveat's shape is
/// exercisable by the integration test suite.
///
/// Two readers, and they need different halves. `provider` and `note` address
/// the model, in the vocabulary an athlete uses: `reconnect_required` is on
/// `tool_results::ACTIVITIES_ENVELOPE_KEPT`, so the whole sidecar survives the
/// projection the tool loop runs the payload through, and the prefetch, which
/// keeps the prose alone, appends the note after it
/// (`tool_results::served_without_notes`). Either way the agent reads that the
/// window it is about to answer from is missing a source.
/// `provider_slug` is the backend key `tool_results::reconnect_offer_in_responses`
/// lifts out for the chat pipeline to mint a reconnect URL from —
/// `sciotte_garmin` takes the Dravr-hosted login page and `garmin` takes an
/// OAuth authorization round-trip, so the display name cannot stand in for it.
/// The projection drops it before the model reads the sidecar
/// (`reconnect::RECONNECT_BACKEND_KEY`): the backend is never the athlete's word.
#[must_use]
pub fn provider_reconnect_note(display_name: &str, backend: &str) -> Value {
    json!({
        "provider": display_name,
        "provider_slug": backend,
        "note": format!(
            "The activities above were served WITHOUT {display_name}: that connection expired and the athlete must re-authorize it. Answer the question from the activities shown, then add one short sentence that {display_name} is disconnected and that reconnecting it restores the sessions only it records."
        ),
    })
}

/// Build the `provider_unavailable` sidecar for a window served WITHOUT a
/// connection that could not answer just now.
///
/// Its grant stands: a rate-limited or failing token refresh over a connection
/// no refusal flagged (a flagged one is reported dead instead), a provider
/// outage. The window is as partial as one served without a dead connection,
/// so the agent is told so, but nothing here is the athlete's to fix: the note
/// says the connection is intact, and carries no `provider_slug`, since no
/// reconnect link is owed. It reaches the model by both routes
/// `reconnect_required` does: kept by the tool-loop projection on
/// `tool_results::ACTIVITIES_ENVELOPE_KEPT`, and appended to the prose by the
/// prefetch.
#[must_use]
pub fn provider_unavailable_note(display_name: &str) -> Value {
    json!({
        "provider": display_name,
        "note": format!(
            "The activities above were served WITHOUT {display_name}: its data could not be fetched just now, so sessions only {display_name} records may be missing, and its connection is intact, so do not ask the athlete to reconnect it. Answer the question from the activities shown. {UNREAD_PROVIDER_DIRECTIVE}"
        ),
    })
}

/// Build the `provider_unavailable` sidecar for a window served from the
/// elected provider's own cache after its live read failed.
///
/// The rows are the provider's, as the platform last synced them, so nothing
/// is missing but what it recorded since: the note says that, keeps the
/// connection intact (no reconnect is owed), and tells the agent not to read
/// the window's end as the athlete's last session. Same key and shape as
/// [`provider_unavailable_note`], so it reaches the model by the same routes.
#[must_use]
pub fn provider_stale_note(display_name: &str) -> Value {
    json!({
        "provider": display_name,
        "note": format!(
            "{display_name} could not be read just now, so the activities above are what was last synced from it: sessions recorded since may be missing, and its connection is intact, so do not ask the athlete to reconnect it. Answer the question from the activities shown. {UNREAD_PROVIDER_DIRECTIVE}"
        ),
    })
}

/// Build the `provider_unavailable` sidecar for a window the provider's own
/// OAuth grant served in place of the athlete's usr/pwd session
/// (carnet#1494).
///
/// Nothing is missing: the grant is the same provider, read through its API,
/// and it vouched for the window. The note says so, so the agent neither
/// treats the window as partial nor asks for a reconnect; the session's own
/// sign-in, when it died, is asked for by the reconnect notice, not here.
#[must_use]
fn provider_served_by_grant_note(display_name: &str) -> Value {
    json!({
        "provider": display_name,
        "note": format!(
            "{display_name} could not be read through the athlete's sign-in just now, so the activities above came from its API connection instead: they are current and complete. Do not ask the athlete to reconnect it. Answer the question from the activities shown. {UNREAD_PROVIDER_DIRECTIVE}"
        ),
    })
}

/// Why the elected provider's window was not its own live read, keyed by
/// that provider's backend.
///
/// Its grant served it in its session's place, the athlete's other
/// connections served it, or its own cached window did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrimaryStandIn {
    /// Its session could not answer and the provider's own OAuth grant
    /// served the window in its place (carnet#1494).
    ServedByGrant(String),
    /// Its grant or session is dead: the athlete must reconnect it.
    Dead(String),
    /// It could not answer just now, and its connection stands.
    Unreachable(String),
    /// Its live read failed, timed out or is held back, or answered nothing
    /// over a window the cache holds its activities in, and its own cached
    /// window answered instead.
    Stale(String, StaleReason),
}

impl PrimaryStandIn {
    /// Whether the elected provider itself answered the window — its own
    /// cache, or its grant in its session's place — which counts as that
    /// connection serving the athlete.
    ///
    /// `CaptureFreshness::last_used_at` promises "live fetch or cache
    /// alike", and the active-athlete sweep selects on it. Touching only a
    /// live answer let a scrape that outlives every turn's 15 s budget freeze
    /// the mark on 2026-10-19, and the sweep passed the athlete over for ten
    /// days while chat served him daily (carnet#1278). A sibling's answer
    /// (`Dead`, `Unreachable`) is still not the primary's serve.
    #[must_use]
    pub const fn served_by_the_provider(&self) -> bool {
        matches!(self, Self::Stale(..) | Self::ServedByGrant(_))
    }

    /// The sidecar the served window carries for it, and the key it rides
    /// under: `reconnect_required` for a dead provider, `provider_unavailable`
    /// for one that could not answer or was served from its cache.
    #[must_use]
    pub fn caveat(&self) -> (&'static str, Value) {
        let (key, mut note, reason) = match self {
            Self::ServedByGrant(backend) => (
                "provider_unavailable",
                provider_served_by_grant_note(user_facing_name(backend)),
                "grant",
            ),
            Self::Dead(backend) => (
                "reconnect_required",
                provider_reconnect_note(user_facing_name(backend), backend),
                "auth",
            ),
            Self::Unreachable(backend) => (
                "provider_unavailable",
                provider_unavailable_note(user_facing_name(backend)),
                "error",
            ),
            Self::Stale(backend, reason) => (
                "provider_unavailable",
                provider_stale_note(user_facing_name(backend)),
                reason.as_str(),
            ),
        };
        if let Some(object) = note.as_object_mut() {
            object.insert("reason".to_owned(), Value::String(reason.to_owned()));
        }
        (key, note)
    }

    /// How complete a window this stand-in served is, or `None` when the
    /// stand-in says nothing about it: a grant that served in its session's
    /// place vouched for the window. A read past the turn's budget is still
    /// running; any other stand-in is the provider unavailable.
    #[must_use]
    pub const fn completeness(&self) -> Option<Completeness> {
        match self {
            Self::ServedByGrant(_) => None,
            Self::Stale(_, StaleReason::Timeout) => {
                Some(Completeness::Incomplete(IncompleteReason::LiveReadRunning))
            }
            Self::Dead(_) | Self::Unreachable(_) | Self::Stale(..) => Some(
                Completeness::Incomplete(IncompleteReason::ProviderUnavailable),
            ),
        }
    }

    /// The stand-in for a window [`serve_in_place`] served: the grant's, when
    /// it answered first, else `otherwise`.
    #[must_use]
    pub fn of_fallback(fallback: &FallbackServe, primary: &str, otherwise: Self) -> Self {
        if fallback.served_by_grant.is_some() {
            Self::ServedByGrant(primary.to_owned())
        } else {
            otherwise
        }
    }

    /// The backend this stand-in is about.
    #[must_use]
    pub fn backend(&self) -> &str {
        match self {
            Self::ServedByGrant(backend)
            | Self::Dead(backend)
            | Self::Unreachable(backend)
            | Self::Stale(backend, _) => backend,
        }
    }
}

/// The window served in place of a primary that cannot answer: for a read
/// pinned to the provider, its grant alone ([`grant_window`]: the same
/// provider, so the pin still holds), else the grant first and the athlete's
/// other connections after it ([`serve_without_primary`]). `None` when
/// nothing served a row.
#[cfg(feature = "tools-data")]
pub(super) async fn serve_in_place(
    context: &ToolExecutionContext,
    primary: &str,
    pinned: bool,
    is_historical: bool,
    params: &ActivityQueryParams,
) -> Option<FallbackServe> {
    if !pinned {
        return serve_without_primary(context, primary, is_historical, params).await;
    }
    let tenant = TenantId::from_uuid(context.tenant_id?);
    let (grant, rows) = Box::pin(grant_window(
        &context.resources,
        primary,
        context.user_id,
        tenant,
        params,
    ))
    .await?;
    if rows.is_empty() {
        return None;
    }
    let (activities, merge_report) = merge_duplicates(rows, &DedupConfig::from_env());
    Some(FallbackServe {
        activities,
        merge_report,
        served_by: vec![user_facing_name(&grant).to_owned()],
        served_by_grant: Some(grant),
    })
}

/// Put a stand-in's caveat on a served window's payload.
///
/// The caveat says when that provider was last synced (`last_synced_at`,
/// RFC 3339, `null` when never), so a reader knows how old "what was last
/// synced" is instead of guessing.
pub async fn attach_stand_in_caveat(
    result: Option<&mut Value>,
    stand_in: &PrimaryStandIn,
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: &TenantId,
) {
    let Some(object) = result.and_then(Value::as_object_mut) else {
        return;
    };
    let (key, mut caveat) = stand_in.caveat();
    let last_synced = runtime
        .repos()
        .activity_cache
        .latest_activity_sync(user_id, tenant_id, stand_in.backend())
        .await
        .ok()
        .flatten();
    if let Some(note) = caveat.as_object_mut() {
        note.insert(
            "last_synced_at".to_owned(),
            last_synced.map_or(Value::Null, |at| Value::String(at.to_rfc3339())),
        );
    }
    object.insert(key.to_owned(), caveat);
}

/// Pull the `format` arg, defaulting to JSON.
pub fn parse_output_format(args: &Value) -> OutputFormat {
    args.get("format")
        .and_then(Value::as_str)
        .map_or(OutputFormat::Json, OutputFormat::from_str_param)
}

// ============================================================================
// GetActivitiesTool - Retrieve user activities
// ============================================================================

/// The `message` a still-fetching historical window carries back to the model.
///
/// Two axes, because two different things are true on different paths.
///
/// `started` distinguishes the first ask from a re-ask while the same window is
/// already in flight. Repeating "I'm pulling it now" on the second one reads as
/// no progress, so that branch says the fetch is already running and never
/// repeats the ask-again line.
///
/// `followed_up` is whether anything will carry the finished window back on its
/// own: a chat turn gets it delivered into the same conversation, through the
/// channel's adapter or as a persisted turn, while a direct MCP or A2A caller
/// has no such path. Telling an athlete to ask again when the answer is already
/// on its way is the defect this exists to prevent — and telling a caller that
/// nothing will notify to sit and wait would be the mirror of it.
#[must_use]
pub fn backfill_placeholder_message(
    display_provider: &str,
    started: bool,
    followed_up: bool,
) -> String {
    if started {
        let delivery = if followed_up {
            "The finished window comes back into this conversation on its own — tell \
             the athlete it is on its way here, and do not ask them to repeat the \
             question."
        } else {
            "Ask me again shortly and it'll be ready."
        };
        format!(
            "I'm pulling your older {display_provider} history now — this can take a \
             minute. {delivery}"
        )
    } else {
        let delivery = if followed_up {
            " It comes back into this conversation on its own when it lands."
        } else {
            ""
        };
        format!(
            "Your older {display_provider} history is still being fetched from an \
             earlier request — it is not ready yet. Say so plainly, do not promise it \
             for a specific moment, and answer with the data already available.\
             {delivery}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        provider_reconnect_note, provider_stale_note, provider_unavailable_note,
        UNREAD_PROVIDER_DIRECTIVE,
    };

    /// A provider that could not answer this turn is the agent's to know and
    /// not the athlete's to hear about: on 2026-10-23 and 2026-10-27 the
    /// "add one short sentence" these notes asked for read as a broken sync.
    #[test]
    fn an_unread_provider_is_not_narrated_to_the_athlete() {
        for note in [
            provider_stale_note("Garmin"),
            provider_unavailable_note("Garmin"),
        ] {
            let text = note["note"].as_str().unwrap_or_default();
            assert!(text.contains(UNREAD_PROVIDER_DIRECTIVE), "{text}");
            assert!(text.contains("may be missing"), "{text}");
            assert!(!text.contains("add one short sentence"), "{text}");
        }
    }

    /// A dead connection is the athlete's to fix, so that one is still told.
    #[test]
    fn a_dead_connection_is_still_told_to_the_athlete() {
        let note = provider_reconnect_note("Garmin", "sciotte_garmin");
        let text = note["note"].as_str().unwrap_or_default();
        assert!(text.contains("is disconnected"), "{text}");
        assert!(!text.contains(UNREAD_PROVIDER_DIRECTIVE), "{text}");
    }
}
