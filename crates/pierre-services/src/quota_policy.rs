// ABOUTME: The usage-cap policy — one ladder, one tier resolution, one bypass rule, for every surface
// ABOUTME: Chat turns and direct /mcp tool calls differ only in which extra counters they add on top

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Pre-request usage-cap enforcement.
//!
//! [`check_quotas`] is the single implementation of the platform's quota
//! policy. Two surfaces enter it:
//!
//! - a chat turn, through `pierre_chat_pipeline::quota_policy::check_pre_chat_quotas_scoped`,
//!   which is itself reached from exactly one place (the turn service), so no
//!   chat surface can acquire a turn without passing it;
//! - a direct `POST /mcp` tool call, through
//!   [`QuotaSurface::McpToolCall`].
//!
//! Both read the same account ladder — daily messages, weekly tokens, daily
//! tokens — resolve the tier from the same `users` row, degrade to the same
//! compiled-in [`pierre_core::models::TierQuotaConfig`] defaults when the admin
//! config service is unavailable, and honour the same bypass allow-list. The
//! surface decides only which *extra* counters it adds: per-conversation and
//! per-agent caps for a chat turn, the tool-call ladder for an `/mcp` call.
//!
//! That is the whole point of the module. Messaging spent four months
//! bypassing every message and token cap because enforcement lived in the web
//! handler and the webhook path never invoked it (registre#9). The `/mcp`
//! handler then grew a *second* ladder that resolved the tier and the
//! thresholds itself and described itself in a comment as mirroring the chat
//! route; a mirror is a copy that drifts. There is now one ladder and two
//! entry points into it.
//!
//! Placement: the policy lives here rather than in `pierre-chat-pipeline`
//! because `pierre-server`'s `/mcp` handler is compiled unconditionally while
//! `pierre-chat-pipeline` is an optional dependency behind `client-chat`. A
//! policy only half the surfaces can link to is not shared.

use std::env;
use std::fmt;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, UserTier};
use pierre_database::RepositoryRegistry;
use pierre_runtime_context::{default_admin_config, AdminConfigLookup};
use tracing::{debug, info};
use uuid::Uuid;

use crate::analytics;
use crate::usage_counter::{LimitCheckResult, UsageCounterService};

/// Env var holding a comma-separated list of user UUIDs that bypass
/// quota enforcement. Reserved for emergency on-call overrides; the
/// canonical bypass path is to elevate the user to `Enterprise` tier
/// via `POST /api/admin/users/{id}/tier` (Enterprise caps are
/// `i64::MAX` so quotas effectively never trip).
///
/// Deliberately the *only* bypass. The admin role does not bypass quotas on
/// any surface: `/mcp` used to exempt admins and chat never did, which meant
/// the same account refused at two different points depending on which door
/// it knocked on.
const QUOTA_BYPASS_USER_IDS_ENV: &str = "QUOTA_BYPASS_USER_IDS";

/// Optional scope hint passed through so conversation- and agent-keyed caps
/// fire in the same call as the global daily/weekly caps.
#[derive(Debug, Default, Clone)]
pub struct PreChatScope<'a> {
    /// Conversation row id (`chat_conversations.id`). When present, the
    /// `conversation_messages` per-conversation cap from
    /// [`pierre_core::models::TierQuotaConfig`] is enforced.
    pub conversation_id: Option<&'a str>,
    /// Agent id (`agents.id`). When present, the `daily_coach_messages` cap
    /// from [`pierre_core::models::TierQuotaConfig`] is enforced.
    pub agent_id: Option<&'a str>,
}

/// Which surface is asking, and therefore which counters the shared account
/// ladder is extended with.
///
/// The account ladder itself is not a variant of this enum — it is what both
/// surfaces have in common, and making it selectable is how the two
/// implementations drifted in the first place.
#[derive(Debug, Clone)]
pub enum QuotaSurface<'a> {
    /// A chat turn on any chat surface (web, mobile, messaging). Adds the
    /// per-conversation and per-agent daily message caps when the relevant
    /// ids are present.
    ChatTurn(PreChatScope<'a>),
    /// A direct `POST /mcp` tool call. Adds the daily and weekly tool-call
    /// ladder, the counters that path increments after a tool executes.
    ///
    /// Not a chat-turn ingress: it has its own entry point, its own
    /// `call_type = "mcp_tool"` usage row, and no conversation or agent to
    /// scope by. Only the policy is shared.
    McpToolCall,
}

impl QuotaSurface<'_> {
    /// The `surface` value the `quota.limit_reached` notice carries: `chat`
    /// for a chat turn, `mcp` for a direct tool call.
    const fn label(&self) -> &'static str {
        match self {
            Self::ChatTurn(_) => "chat",
            Self::McpToolCall => "mcp",
        }
    }
}

/// Everything the policy reads, independent of which surface asked.
pub struct QuotaPolicyInputs<'a> {
    /// Repository registry — supplies the `users` row the tier comes from and
    /// the usage counters the ladder reads.
    pub repos: &'a RepositoryRegistry,
    /// Admin config lookup for per-tenant cap overrides. `None` degrades to
    /// the compiled-in tier defaults rather than skipping enforcement.
    pub admin_config: Option<&'a dyn AdminConfigLookup>,
}

impl fmt::Debug for QuotaPolicyInputs<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuotaPolicyInputs")
            .field("repos", &self.repos)
            .finish_non_exhaustive()
    }
}

/// Who the caps are measured for: the tier that selects them, and the email
/// the `quota.limit_reached` notice names the athlete by.
struct QuotaSubject {
    tier: UserTier,
    /// `None` when the `users` row could not be loaded.
    email: Option<String>,
}

/// Resolve the user's tier and email from the `users` row, falling back to
/// `Starter` and no email when the row cannot be loaded (multi-tenant
/// `get_global` here because quotas key on the user, not on a single tenant
/// membership).
async fn resolve_quota_subject(repos: &RepositoryRegistry, user_id: Uuid) -> QuotaSubject {
    match repos.users.get_global(user_id).await {
        Ok(Some(user)) => QuotaSubject {
            tier: user.tier,
            email: Some(user.email),
        },
        _ => QuotaSubject {
            tier: UserTier::Starter,
            email: None,
        },
    }
}

/// Returns `true` when the user's UUID is listed in `QUOTA_BYPASS_USER_IDS`.
fn is_quota_bypass_user(user_id: Uuid) -> bool {
    let raw = env::var(QUOTA_BYPASS_USER_IDS_ENV).unwrap_or_default();
    if raw.is_empty() {
        return false;
    }
    let needle = user_id.to_string();
    raw.split(',')
        .map(str::trim)
        .any(|candidate| candidate.eq_ignore_ascii_case(&needle))
}

/// The counter a request was refused on, and where it stood.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BreachedCap {
    /// The counter, e.g. `daily_messages` — the notice's `limit_type`.
    counter_type: &'static str,
    current: i64,
    limit: i64,
    /// ISO 8601 instant the counter resets at.
    resets_at: String,
}

impl BreachedCap {
    fn from_check(counter_type: &'static str, check: &LimitCheckResult) -> Self {
        Self {
            counter_type,
            current: check.current,
            limit: check.limit,
            resets_at: check.resets_at.clone(),
        }
    }

    /// The refusal the caller receives.
    fn into_error(self) -> AppError {
        AppError::quota_exceeded(self.counter_type, self.current, self.limit, &self.resets_at)
    }
}

/// Why the ladder stopped: a cap refused the request, or a counter could not
/// be read. Only the first is reported as `quota.limit_reached`.
#[derive(Debug)]
enum LadderStop {
    Cap(BreachedCap),
    Failed(AppError),
}

impl From<AppError> for LadderStop {
    fn from(error: AppError) -> Self {
        Self::Failed(error)
    }
}

/// Refuse when `check` reports the hard limit breached (burst multiplier
/// applied), naming `counter_type` in the refusal.
fn refuse_if_over_burst(
    counter_type: &'static str,
    check: &LimitCheckResult,
) -> Result<(), LadderStop> {
    if check.allowed {
        return Ok(());
    }
    Err(LadderStop::Cap(BreachedCap::from_check(
        counter_type,
        check,
    )))
}

/// Refuse when `check` has reached the soft limit, for counters that allow no
/// burst at all (weekly tokens, per-conversation messages).
fn refuse_if_at_limit(
    counter_type: &'static str,
    check: &LimitCheckResult,
) -> Result<(), LadderStop> {
    if check.current < check.limit {
        return Ok(());
    }
    Err(LadderStop::Cap(BreachedCap::from_check(
        counter_type,
        check,
    )))
}

/// Check every cap that applies to this request and report where the athlete
/// stands.
///
/// Enforces the account ladder — daily messages (with burst), weekly tokens
/// (hard cap), daily tokens (with burst) — for every surface, then the extra
/// counters [`QuotaSurface`] names. The user's [`UserTier`] is resolved from
/// the `users` row so per-tier defaults from
/// [`pierre_core::models::TierQuotaConfig`] apply before any admin-config
/// override.
///
/// `tenant_id` is the athlete's own tenant. On a shared messaging bot that is
/// not the tenant that owns the webhook, and reading the bot's counters is how
/// registre#9 stayed invisible: the caps were measured against a budget
/// nothing was ever recorded into.
///
/// On success returns the most restrictive soft warning observed on the
/// account ladder, or `None` when every counter is clear. Callers that render
/// a warning to the athlete read `burst_zone` / `warning` off the returned
/// check to pick a severity.
///
/// A refusal is also reported to operators as the `quota.limit_reached`
/// notify event, whose Slack card carries the button that raises the athlete
/// to Enterprise. A bypassed user and a counter that cannot be read report
/// nothing.
///
/// # Errors
///
/// Returns [`AppError::quota_exceeded`] when any hard cap is breached, and the
/// underlying repository error when a counter cannot be read.
pub async fn check_quotas(
    inputs: &QuotaPolicyInputs<'_>,
    tenant_id: TenantId,
    user_id: Uuid,
    surface: &QuotaSurface<'_>,
) -> AppResult<Option<LimitCheckResult>> {
    if is_quota_bypass_user(user_id) {
        debug!("Skipping quota check for user via QUOTA_BYPASS_USER_IDS allow-list");
        return Ok(None);
    }

    let subject = resolve_quota_subject(inputs.repos, user_id).await;
    match run_ladder(inputs, tenant_id, user_id, &subject.tier, surface).await {
        Ok(warning) => Ok(warning),
        Err(LadderStop::Failed(error)) => Err(error),
        Err(LadderStop::Cap(breach)) => {
            report_limit_reached(tenant_id, user_id, &subject, surface, &breach);
            Err(breach.into_error())
        }
    }
}

/// Tell operators an athlete hit a hard cap, as the `quota.limit_reached`
/// notify event.
///
/// Repeats are not filtered here: every refused request reports. Collapsing
/// an athlete's retries against one cap into a single ping belongs to the
/// event's routing rule in contremaitre's `notify-routing.yaml`, which must
/// declare `dedup_per: [user_id, limit_type]` with a `dedup_window`; without
/// that rule, each refusal posts its own card.
fn report_limit_reached(
    tenant_id: TenantId,
    user_id: Uuid,
    subject: &QuotaSubject,
    surface: &QuotaSurface<'_>,
    breach: &BreachedCap,
) {
    let user_id = user_id.to_string();
    // Warm the identity cache first: the notify enricher resolves user_email
    // from user_id, and without it the Slack ping loses the athlete.
    if let Some(email) = &subject.email {
        analytics::cache_user_email(&user_id, email);
    }
    info!(
        target: "notify",
        event = "quota.limit_reached",
        user_id = %user_id,
        tenant_id = %tenant_id,
        limit_type = breach.counter_type,
        current = breach.current,
        limit = breach.limit,
        tier = subject.tier.as_str(),
        surface = surface.label(),
        "athlete reached a usage cap"
    );
}

/// The account ladder plus the counters `surface` adds, stopping at the first
/// cap that refuses.
async fn run_ladder(
    inputs: &QuotaPolicyInputs<'_>,
    tenant_id: TenantId,
    user_id: Uuid,
    tier: &UserTier,
    surface: &QuotaSurface<'_>,
) -> Result<Option<LimitCheckResult>, LadderStop> {
    // Degrade to compile-time tier defaults (rather than skipping
    // enforcement) when the admin config service is unavailable.
    let compiled_defaults: &dyn AdminConfigLookup = default_admin_config();
    let admin_config: &dyn AdminConfigLookup = inputs.admin_config.unwrap_or(compiled_defaults);

    let usage_svc = UsageCounterService::new(inputs.repos.usage_counters.as_ref(), admin_config);
    let tenant_str = tenant_id.to_string();
    let user_str = user_id.to_string();

    // Daily message cap (allows 1.5x burst).
    let daily_msg_check = usage_svc
        .check_limit_for_tier(&tenant_str, &user_str, "daily_messages", tier)
        .await?;
    refuse_if_over_burst("daily_messages", &daily_msg_check)?;

    // Weekly token budget (hard cap, no burst allowed).
    let weekly_token_check = usage_svc
        .check_limit_for_tier(&tenant_str, &user_str, "weekly_tokens", tier)
        .await?;
    refuse_if_at_limit("weekly_tokens", &weekly_token_check)?;

    // Daily token budget (allows 1.5x burst).
    let daily_token_check = usage_svc
        .check_limit_for_tier(&tenant_str, &user_str, "daily_tokens", tier)
        .await?;
    refuse_if_over_burst("daily_tokens", &daily_token_check)?;

    match surface {
        QuotaSurface::ChatTurn(scope) => {
            check_chat_turn_scope(&usage_svc, &tenant_str, &user_str, tier, scope).await?;
        }
        QuotaSurface::McpToolCall => {
            check_tool_call_ladder(&usage_svc, &tenant_str, &user_str, tier).await?;
        }
    }

    Ok(select_usage_warning(
        &daily_msg_check,
        &daily_token_check,
        &weekly_token_check,
    ))
}

/// The per-conversation and per-agent daily message caps a chat turn adds on
/// top of the account ladder.
async fn check_chat_turn_scope(
    usage_svc: &UsageCounterService<'_>,
    tenant_str: &str,
    user_str: &str,
    tier: &UserTier,
    scope: &PreChatScope<'_>,
) -> Result<(), LadderStop> {
    // Per-conversation daily message cap (hard cap, no burst).
    if let Some(conv_id) = scope.conversation_id {
        let conv_check = usage_svc
            .check_limit_with_dimension_for_tier(
                tenant_str,
                user_str,
                "conversation_messages",
                conv_id,
                tier,
            )
            .await?;
        refuse_if_at_limit("conversation_messages", &conv_check)?;
    }

    // Per-agent daily message cap (allows 1.5x burst — agents are already
    // individually rate-limited at the model layer).
    if let Some(agent_id) = scope.agent_id {
        let agent_check = usage_svc
            .check_limit_with_dimension_for_tier(
                tenant_str,
                user_str,
                "daily_coach_messages",
                agent_id,
                tier,
            )
            .await?;
        refuse_if_over_burst("daily_coach_messages", &agent_check)?;
    }

    Ok(())
}

/// The daily and weekly tool-call caps a direct `/mcp` tool call adds on top
/// of the account ladder — the same counters that path increments once the
/// tool has executed.
async fn check_tool_call_ladder(
    usage_svc: &UsageCounterService<'_>,
    tenant_str: &str,
    user_str: &str,
    tier: &UserTier,
) -> Result<(), LadderStop> {
    for counter_type in ["daily_tool_calls", "weekly_tool_calls"] {
        let check = usage_svc
            .check_limit_for_tier(tenant_str, user_str, counter_type, tier)
            .await?;
        refuse_if_over_burst(counter_type, &check)?;
    }
    Ok(())
}

/// Select the most restrictive usage warning from the daily and weekly checks.
///
/// Priority: burst zone > approaching warning. Within each tier, weekly caps
/// take precedence over daily since they represent a harder boundary.
fn select_usage_warning(
    daily_msg_check: &LimitCheckResult,
    daily_token_check: &LimitCheckResult,
    weekly_token_check: &LimitCheckResult,
) -> Option<LimitCheckResult> {
    let checks: [&LimitCheckResult; 3] = [weekly_token_check, daily_token_check, daily_msg_check];

    checks
        .iter()
        .find(|c| c.burst_zone)
        .or_else(|| checks.iter().find(|c| c.warning))
        .map(|c| (*c).clone())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fmt::Debug as FmtDebug;
    use std::mem;
    use std::sync::{Arc, Mutex};

    use pierre_core::errors::ErrorCode;
    use pierre_core::models::UserTier;
    use pierre_runtime_context::default_admin_config;
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::server::create_test_user;
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::subscriber::{self, DefaultGuard};
    use tracing::{Event, Metadata, Subscriber};
    use uuid::Uuid;

    use super::{
        check_quotas, refuse_if_at_limit, refuse_if_over_burst, BreachedCap, LadderStop,
        LimitCheckResult, PreChatScope, QuotaPolicyInputs, QuotaSurface, TenantId,
        UsageCounterService,
    };

    const RESETS_AT: &str = "2099-01-01T00:00:00Z";

    /// The fields of every `target: "notify"` event emitted on this thread
    /// while the capture is installed, each rendered as text.
    type NotifyEvents = Arc<Mutex<Vec<HashMap<String, String>>>>;

    #[derive(Default)]
    struct FieldText(HashMap<String, String>);

    impl Visit for FieldText {
        fn record_debug(&mut self, field: &Field, value: &dyn FmtDebug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            self.0.insert(field.name().to_owned(), value.to_string());
        }
    }

    /// A whole subscriber that keeps `notify` events and owns no spans, so no
    /// span id it hands out is ever resolved elsewhere.
    #[derive(Default)]
    struct NotifyCapture(NotifyEvents);

    impl Subscriber for NotifyCapture {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            if event.metadata().target() != "notify" {
                return;
            }
            let mut fields = FieldText::default();
            event.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    /// Capture this thread's `notify` events until the guard drops.
    fn capture_notify() -> (NotifyEvents, DefaultGuard) {
        let capture = NotifyCapture::default();
        let events = Arc::clone(&capture.0);
        (events, subscriber::set_default(capture))
    }

    /// The captured events, leaving none behind.
    fn drain(events: &NotifyEvents) -> Vec<HashMap<String, String>> {
        mem::take(&mut *events.lock().unwrap())
    }

    fn check(allowed: bool, current: i64, limit: i64) -> LimitCheckResult {
        LimitCheckResult {
            allowed,
            current,
            limit,
            warning: false,
            burst_zone: false,
            resets_at: RESETS_AT.to_owned(),
        }
    }

    /// The cap a refusal names, or `None` when the check let the request through.
    fn breach(outcome: Result<(), LadderStop>) -> Option<BreachedCap> {
        match outcome {
            Ok(()) => None,
            Err(LadderStop::Cap(cap)) => Some(cap),
            Err(LadderStop::Failed(e)) => panic!("a pure check cannot fail to read: {e}"),
        }
    }

    #[test]
    fn a_counter_without_burst_refuses_at_its_limit_and_not_one_below() {
        assert_eq!(
            breach(refuse_if_at_limit("weekly_tokens", &check(true, 99, 100))),
            None,
            "one below the limit is still allowed"
        );
        assert_eq!(
            breach(refuse_if_at_limit("weekly_tokens", &check(true, 100, 100))),
            Some(BreachedCap {
                counter_type: "weekly_tokens",
                current: 100,
                limit: 100,
                resets_at: RESETS_AT.to_owned(),
            }),
            "reaching the limit is a breach of that counter at those values"
        );
    }

    #[test]
    fn a_burst_counter_refuses_only_once_the_check_disallows_it() {
        assert_eq!(
            breach(refuse_if_over_burst("daily_messages", &check(true, 60, 50))),
            None,
            "the burst zone above the soft limit is still allowed"
        );
        let cap = breach(refuse_if_over_burst(
            "daily_messages",
            &check(false, 75, 50),
        ))
        .expect("a disallowed check is a breach");
        assert_eq!(
            cap.counter_type, "daily_messages",
            "the breached counter is named"
        );
        assert_eq!((cap.current, cap.limit), (75, 50), "its values ride along");
    }

    #[test]
    fn a_breach_is_refused_with_the_quota_exceeded_error_naming_the_counter() {
        let error = BreachedCap {
            counter_type: "daily_tool_calls",
            current: 30,
            limit: 20,
            resets_at: RESETS_AT.to_owned(),
        }
        .into_error();
        assert_eq!(
            error.code,
            ErrorCode::QuotaExceeded,
            "a cap is a quota refusal"
        );
        let details = error
            .details
            .as_deref()
            .expect("quota errors carry details");
        assert_eq!(details["limit_type"], "daily_tool_calls", "{details}");
        assert_eq!(details["current"], 30, "{details}");
        assert_eq!(details["limit"], 20, "{details}");
        assert_eq!(details["resets_at"], RESETS_AT, "{details}");
    }

    #[test]
    fn each_surface_is_named_as_the_notice_catalogues_it() {
        assert_eq!(
            QuotaSurface::ChatTurn(PreChatScope::default()).label(),
            "chat",
            "a chat turn reports the chat surface"
        );
        assert_eq!(
            QuotaSurface::McpToolCall.label(),
            "mcp",
            "a direct tool call reports the mcp surface"
        );
    }

    #[tokio::test]
    async fn a_starter_over_its_daily_message_burst_is_refused_with_that_counter() {
        let db = create_test_db().await.expect("test database");
        let repos = db.repositories();
        let user = create_test_user(&format!("{}@quota.test", Uuid::new_v4()), None);
        let user_id = repos.users.create(&user).await.expect("user row");
        let tenant_id = TenantId::from_uuid(Uuid::new_v4());
        let inputs = QuotaPolicyInputs {
            repos,
            admin_config: None,
        };
        let usage = UsageCounterService::new(repos.usage_counters.as_ref(), default_admin_config());
        let (tenant, user) = (tenant_id.to_string(), user_id.to_string());

        let (events, _guard) = capture_notify();

        // Starter allows 50 daily messages with a 1.5x burst: 74 still passes.
        usage
            .increment(&tenant, &user, "daily_messages", 74)
            .await
            .expect("counter write");
        let allowed = check_quotas(&inputs, tenant_id, user_id, &QuotaSurface::McpToolCall).await;
        assert!(allowed.is_ok(), "74 of a 75 burst is allowed: {allowed:?}");
        assert_eq!(
            drain(&events),
            Vec::<HashMap<String, String>>::new(),
            "a request under the cap reports nothing"
        );

        usage
            .increment(&tenant, &user, "daily_messages", 1)
            .await
            .expect("counter write");
        let error = check_quotas(&inputs, tenant_id, user_id, &QuotaSurface::McpToolCall)
            .await
            .expect_err("the 75th message breaches the burst");
        assert_eq!(error.code, ErrorCode::QuotaExceeded, "{error:?}");
        let details = error
            .details
            .as_deref()
            .expect("quota errors carry details");
        assert_eq!(details["limit_type"], "daily_messages", "{details}");
        assert_eq!(details["current"], 75, "{details}");
        assert_eq!(details["limit"], 50, "{details}");

        let reported = drain(&events);
        assert_eq!(reported.len(), 1, "one refusal, one notice: {reported:?}");
        let notice = &reported[0];
        for (field, expected) in [
            ("event", "quota.limit_reached".to_owned()),
            ("user_id", user_id.to_string()),
            ("tenant_id", tenant_id.to_string()),
            ("limit_type", "daily_messages".to_owned()),
            ("current", "75".to_owned()),
            ("limit", "50".to_owned()),
            ("tier", "starter".to_owned()),
            ("surface", "mcp".to_owned()),
        ] {
            assert_eq!(
                notice.get(field),
                Some(&expected),
                "the notice carries {field}: {notice:?}"
            );
        }
    }

    #[tokio::test]
    async fn an_enterprise_athlete_passes_the_tool_call_counts_that_refuse_every_other_tier() {
        let db = create_test_db().await.expect("test database");
        let repos = db.repositories();
        let tenant_id = TenantId::from_uuid(Uuid::new_v4());
        let tenant = tenant_id.to_string();
        let inputs = QuotaPolicyInputs {
            repos,
            admin_config: None,
        };
        let usage = UsageCounterService::new(repos.usage_counters.as_ref(), default_admin_config());

        let (events, _guard) = capture_notify();
        for (tier, refused_on) in [
            (UserTier::Starter, Some("daily_tool_calls")),
            (UserTier::Professional, Some("weekly_tool_calls")),
            (UserTier::Enterprise, None),
        ] {
            let mut athlete = create_test_user(&format!("{}@quota.test", Uuid::new_v4()), None);
            athlete.tier = tier.clone();
            let user_id = repos.users.create(&athlete).await.expect("user row");
            let user = user_id.to_string();
            // Past the Starter daily burst (300) and the Starter and Professional
            // weekly burst (750), within Professional's daily one (3000).
            for counter in ["daily_tool_calls", "weekly_tool_calls"] {
                usage
                    .increment(&tenant, &user, counter, 800)
                    .await
                    .expect("counter write");
            }

            let outcome =
                check_quotas(&inputs, tenant_id, user_id, &QuotaSurface::McpToolCall).await;
            let reported = drain(&events);
            if let Some(counter) = refused_on {
                let error = outcome.expect_err("a capped tier is refused");
                let details = error.details.as_deref().expect("quota details");
                assert_eq!(details["limit_type"], counter, "{tier:?}: {details}");
                assert_eq!(reported.len(), 1, "{tier:?}: {reported:?}");
                assert_eq!(
                    reported[0].get("limit_type").map(String::as_str),
                    Some(counter),
                    "{tier:?}: the notice names the counter that refused"
                );
            } else {
                assert!(
                    outcome.is_ok(),
                    "Enterprise has no tool-call cap, daily or weekly: {outcome:?}"
                );
                assert!(reported.is_empty(), "{tier:?}: {reported:?}");
            }
        }
    }
}
