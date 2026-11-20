// ABOUTME: One structured log line for every usage limit an athlete is refused by or warned about
// ABOUTME: Same field names as the quota.limit_reached notify event, so one GCP query finds both
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Usage-limit log lines.
//!
//! The account ladder's refusals already reach operators as the
//! `quota.limit_reached` notify event (`pierre_services::quota_policy`). Every
//! other limit an athlete can hit — the conversation cap, the agent cap, a
//! group's athlete allowance or member cap, a request budget, the activity-mode
//! caps of `get_activities` — and every warning that a cap is near was refused or
//! shown without a line saying who, which limit and where the counter stood. On
//! 2026-11-17 a coach reported running out of tokens and nothing in the logs
//! could say which limit he had met (carnet#1630).
//!
//! These lines go to the [`TARGET`] target, never to `notify`: they are for
//! answering "which limit stopped user X", not for paging anyone. They carry the
//! notify event's field names (`user_id`, `limit_type`, `current`, `limit`,
//! `surface`), and the Cloud Logging formatter writes an event's fields at the
//! top of `jsonPayload`, so this one query covers both:
//!
//! ```text
//! jsonPayload.user_id="<uuid>" AND jsonPayload.limit_type:*
//! ```

use tracing::{info, warn};

/// The `tracing` target every usage-limit line is emitted on.
pub const TARGET: &str = "usage";

/// `event` of a line for a request a usage limit refused.
pub const LIMIT_REACHED: &str = "usage.limit_reached";

/// `event` of a line for an athlete told a usage limit is near.
pub const WARNING: &str = "usage.warning";

/// `event` of a line for a reply a provider or tool-loop budget cut short.
pub const REPLY_CUT_SHORT: &str = "usage.reply_cut_short";

/// One usage limit an athlete met, and where its counter stood.
#[derive(Debug, Clone, Copy)]
pub struct LimitHit<'hit> {
    /// The athlete the limit applies to.
    pub user_id: &'hit str,
    /// Their tenant, when the refusing code knows it. `None` leaves the field
    /// off the line, so the request span's own `tenant_id` shows through.
    pub tenant_id: Option<&'hit str>,
    /// The limit's name, e.g. `max_active_conversations`.
    pub limit_type: &'hit str,
    /// The counter's value when the limit was checked.
    pub current: i64,
    /// The limit the counter is measured against. `0` for a plan that
    /// disables the feature outright (`group_coaching_plan`), and for a
    /// provider output cap the platform does not set (`max_tokens`).
    pub limit: i64,
    /// Where it was met: `chat`, `mcp`, `a2a`, `groups`, a messaging channel, …
    pub surface: &'hit str,
}

/// Log a request a usage limit refused.
pub fn limit_reached(hit: &LimitHit<'_>) {
    warn!(
        target: TARGET,
        event = LIMIT_REACHED,
        user_id = hit.user_id,
        tenant_id = hit.tenant_id,
        limit_type = hit.limit_type,
        current = hit.current,
        limit = hit.limit,
        surface = hit.surface,
        "usage limit reached"
    );
}

/// Log an athlete being told a usage limit is near.
///
/// `level` is `approaching` (past the warning threshold) or `burst` (past the
/// limit, inside the burst allowance).
pub fn warning(hit: &LimitHit<'_>, level: &str) {
    info!(
        target: TARGET,
        event = WARNING,
        user_id = hit.user_id,
        tenant_id = hit.tenant_id,
        limit_type = hit.limit_type,
        current = hit.current,
        limit = hit.limit,
        surface = hit.surface,
        level,
        "usage limit near"
    );
}

/// Log a reply cut short by a budget rather than finished.
///
/// The budget is the provider's output cap (`limit_type` `max_tokens`,
/// `current` the completion tokens it spent) or the tool loop's iteration cap
/// (`max_tool_iterations`).
pub fn reply_cut_short(hit: &LimitHit<'_>) {
    warn!(
        target: TARGET,
        event = REPLY_CUT_SHORT,
        user_id = hit.user_id,
        tenant_id = hit.tenant_id,
        limit_type = hit.limit_type,
        current = hit.current,
        limit = hit.limit,
        surface = hit.surface,
        "reply cut short by a budget"
    );
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fmt::Debug;
    use std::sync::{Arc, Mutex, PoisonError};

    use tracing::field::{Field, Visit};
    use tracing::subscriber::with_default;
    use tracing::{Event, Subscriber};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::registry::Registry;

    use super::*;

    type Lines = Arc<Mutex<Vec<BTreeMap<String, String>>>>;

    struct Capture(Lines);

    struct Fields<'a>(&'a mut BTreeMap<String, String>);

    impl Visit for Fields<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
            self.0.insert(
                field.name().to_owned(),
                format!("{value:?}").trim_matches('"').to_owned(),
            );
        }
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }
        fn record_i64(&mut self, field: &Field, value: i64) {
            self.0.insert(field.name().to_owned(), value.to_string());
        }
    }

    impl<S: Subscriber> Layer<S> for Capture {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            if event.metadata().target() != TARGET {
                return;
            }
            let mut fields = BTreeMap::new();
            event.record(&mut Fields(&mut fields));
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(fields);
        }
    }

    fn lines_while(f: impl FnOnce()) -> Vec<BTreeMap<String, String>> {
        let lines: Lines = Arc::default();
        with_default(Registry::default().with(Capture(Arc::clone(&lines))), f);
        let captured = lines.lock().unwrap_or_else(PoisonError::into_inner).clone();
        captured
    }

    const HIT: LimitHit<'static> = LimitHit {
        user_id: "520aac5e-baf0-4c8d-a343-286952ed553f",
        tenant_id: Some("8f8730fd-37dc-41ad-83b7-de19f346102e"),
        limit_type: "max_active_conversations",
        current: 50,
        limit: 50,
        surface: "chat",
    };

    /// A refusal names who, which limit and where its counter stood, under
    /// the notify event's field names.
    #[test]
    fn a_refusal_names_the_athlete_the_limit_and_the_counter() {
        let lines = lines_while(|| limit_reached(&HIT));
        assert_eq!(lines.len(), 1, "one line per refusal: {lines:?}");
        let line = &lines[0];
        assert_eq!(line.get("event").map(String::as_str), Some(LIMIT_REACHED));
        assert_eq!(line.get("user_id").map(String::as_str), Some(HIT.user_id));
        assert_eq!(
            line.get("limit_type").map(String::as_str),
            Some("max_active_conversations")
        );
        assert_eq!(line.get("current").map(String::as_str), Some("50"));
        assert_eq!(line.get("limit").map(String::as_str), Some("50"));
        assert_eq!(line.get("surface").map(String::as_str), Some("chat"));
        assert_eq!(line.get("tenant_id").map(String::as_str), HIT.tenant_id);
    }

    /// An unknown tenant is left off the line rather than logged empty, so
    /// it cannot mask the tenant the request span carries.
    #[test]
    fn an_unknown_tenant_is_left_off_the_line() {
        let hit = LimitHit {
            tenant_id: None,
            ..HIT
        };
        let lines = lines_while(|| limit_reached(&hit));
        assert_eq!(lines.len(), 1, "one line per refusal: {lines:?}");
        assert!(
            !lines[0].contains_key("tenant_id"),
            "no tenant_id field when the tenant is unknown: {lines:?}"
        );
    }

    /// A warning carries its level beside the same fields.
    #[test]
    fn a_warning_carries_its_level() {
        let lines = lines_while(|| warning(&HIT, "approaching"));
        assert_eq!(lines.len(), 1, "one line per warning: {lines:?}");
        assert_eq!(lines[0].get("event").map(String::as_str), Some(WARNING));
        assert_eq!(
            lines[0].get("level").map(String::as_str),
            Some("approaching")
        );
        assert_eq!(
            lines[0].get("limit_type").map(String::as_str),
            Some(HIT.limit_type)
        );
    }

    /// A reply cut short names the budget that cut it.
    #[test]
    fn a_cut_reply_names_the_budget() {
        let hit = LimitHit {
            limit_type: "max_tool_iterations",
            current: 10,
            limit: 10,
            ..HIT
        };
        let lines = lines_while(|| reply_cut_short(&hit));
        assert_eq!(lines.len(), 1, "one line per cut reply: {lines:?}");
        assert_eq!(
            lines[0].get("event").map(String::as_str),
            Some(REPLY_CUT_SHORT)
        );
        assert_eq!(
            lines[0].get("limit_type").map(String::as_str),
            Some("max_tool_iterations")
        );
        assert_eq!(
            lines[0].get("user_id").map(String::as_str),
            Some(HIT.user_id)
        );
    }
}
