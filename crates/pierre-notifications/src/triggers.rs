// ABOUTME: Notification triggers for intelligence events, agent traffic, sync failures and coach TrainingPeaks links
// ABOUTME: Fire-and-forget on the service's dispatch tracker — failures logged at WARN, never block the caller
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Notification Triggers
//!
//! One function per product event, each fire-and-forget.
//!
//! A trigger declares *what happened* — a [`NotificationEvent`] plus the
//! parameters that describe it — and spawns an async task to dispatch it.
//! Failures are logged at WARN level but never block the caller; notification
//! delivery is always fire-and-forget.
//!
//! No trigger writes a sentence. The facade renders the title, body and action
//! labels through its localizer in the recipient's own language, so the push
//! and the linked chat channels read correctly the first time, and the stored
//! row keeps the event and its parameters so the notification centre can
//! render it again after the athlete changes language.
//!
//! These live here rather than in `dravr-commere` because they dispatch through
//! [`crate::NotificationService`], the platform facade that runs the messaging
//! sink after the upstream pipeline accepts a notification. Firing them at
//! `dravr-commere`'s own service would deliver every one of these categories to
//! persist and Expo push only — which is exactly the gap the sink closes, and
//! most notifications the product raises go through a trigger.

use pierre_core::transport::TransportPolicy;
use std::sync::Arc;

use pierre_core::models::NotificationScreen;
use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;

use crate::events::{
    NotificationActionSpec, SubjectAthlete, ACTION_RECONNECT, ACTION_REPLY,
    SUBJECT_ATHLETE_DATA_KEY,
};
use crate::models::{NotificationActionType, NotificationCategory, TenantId};
use crate::{EventDispatch, NotificationEvent, NotificationService, PushTier};

/// Spawns a fire-and-forget notification dispatch task at the event's tier.
///
/// Each trigger declares its own [`PushTier`] because the trigger knows its
/// event's product semantics — the facade only compares the tier against the
/// recipient's persona floor. Failures are logged at WARN level but never
/// propagated to the caller.
///
/// The task goes onto the service's dispatch tracker rather than a bare
/// `tokio::spawn`: the caller does not wait for it, but the server's shutdown
/// drain does, so a notification fired just before SIGTERM still reaches the
/// athlete's devices and linked chats.
fn spawn_dispatch(service: Arc<NotificationService>, mut dispatch: EventDispatch, tier: PushTier) {
    // Read here, on the caller's task: the spawned task inherits none of the
    // derivation the trigger fired from (carnet#769).
    dispatch.transport_policy = dispatch
        .transport_policy
        .strictest(service.derived_policy());
    let dispatches = service.dispatches.clone();
    dispatches.spawn(async move {
        if let Err(e) = service.dispatch_event(&dispatch, tier).await {
            warn!(
                user_id = %dispatch.user_id,
                notification_type = %dispatch.event.wire(),
                error = %e,
                "Notification dispatch failed"
            );
        }
    });
}

/// The conversation `conversation_id` names, which the clients open as that
/// thread.
fn conversation_route(conversation_id: &str) -> Value {
    json!({
        "screen": NotificationScreen::Coach.as_str(),
        "action": "chat",
        "id": conversation_id,
    })
}

/// Where an insight the agent computed opens: the conversation it was computed
/// in, where the agent's own answer explains the number.
///
/// An insight computed outside any conversation — an MCP or A2A client calling
/// the tool directly — has no thread in the app to open, so it carries no
/// destination and the clients show it as information rather than as a link.
fn insight_route(conversation_id: Option<&str>) -> Value {
    conversation_id.map_or_else(|| json!({}), conversation_route)
}

// ============================================================================
// Intelligence / Training Triggers
// ============================================================================
//
// Each of these fires from inside the tool that computed the number, so
// `conversation_id` is the conversation the tool ran in, when it ran in one.

/// Trigger notification when acute training load exceeds threshold.
pub fn trigger_training_load_alert(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    atl_value: f64,
    conversation_id: Option<&str>,
) {
    let dispatch = EventDispatch {
        user_id,
        tenant_id,
        category: NotificationCategory::Training,
        event: NotificationEvent::TrainingLoadAlert,
        params: json!({ "atl_value": format!("{atl_value:.0}") }),
        route: insight_route(conversation_id),
        actions: None,
        bypass_frequency_cap: false,
        transport_policy: TransportPolicy::AnyTransport,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P2);
}

/// Trigger notification when recovery score drops below threshold.
pub fn trigger_low_recovery_score(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    score: f64,
    conversation_id: Option<&str>,
) {
    let dispatch = EventDispatch {
        user_id,
        tenant_id,
        category: NotificationCategory::Recovery,
        event: NotificationEvent::LowRecoveryScore,
        params: json!({ "score": format!("{score:.0}") }),
        route: insight_route(conversation_id),
        actions: None,
        bypass_frequency_cap: false,
        transport_policy: TransportPolicy::AnyTransport,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P2);
}

/// Trigger notification when TSS trend suggests overtraining risk.
pub fn trigger_overtraining_warning(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    conversation_id: Option<&str>,
) {
    let dispatch = EventDispatch {
        user_id,
        tenant_id,
        category: NotificationCategory::Recovery,
        event: NotificationEvent::OvertrainingWarning,
        params: json!({}),
        route: insight_route(conversation_id),
        actions: None,
        bypass_frequency_cap: false,
        transport_policy: TransportPolicy::AnyTransport,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P2);
}

/// Trigger notification when a synced run sets an all-time best effort.
///
/// `distance` is the standard distance's catalogue code (`5k`, `10k`,
/// `half_marathon`, `marathon`), never a name: the renderer names it in the
/// reader's own language, so the row reads right again after a language
/// change. `time_display` is the effort's elapsed time, `h:mm:ss` or `m:ss`.
/// `transport_policy` is the run's own (carnet#769): the sync that finds the
/// record runs in no derivation the facade could read it from.
pub fn trigger_personal_record(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    activity_id: &str,
    distance: &str,
    time_display: &str,
    transport_policy: TransportPolicy,
) {
    let dispatch = EventDispatch {
        user_id,
        tenant_id,
        category: NotificationCategory::Achievement,
        event: NotificationEvent::PersonalRecord,
        params: json!({ "distance": distance, "time_display": time_display }),
        route: json!({ "screen": NotificationScreen::Activity.as_str(), "id": activity_id }),
        actions: None,
        bypass_frequency_cap: false,
        transport_policy,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P3);
}

/// Trigger notification when a fitness metric improves (FTP, `VO2max`, etc.).
pub fn trigger_fitness_improvement(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    metric_name: &str,
    value_display: &str,
    conversation_id: Option<&str>,
) {
    let dispatch = EventDispatch {
        user_id,
        tenant_id,
        category: NotificationCategory::Achievement,
        event: NotificationEvent::FitnessImprovement,
        params: json!({ "metric_name": metric_name, "value_display": value_display }),
        route: insight_route(conversation_id),
        actions: None,
        bypass_frequency_cap: false,
        transport_policy: TransportPolicy::AnyTransport,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P3);
}

// ============================================================================
// Agent Triggers (bypass frequency cap)
// ============================================================================

/// Trigger notification when an agent sends a message to an athlete.
pub fn trigger_agent_message(
    service: &Arc<NotificationService>,
    athlete_id: Uuid,
    tenant_id: TenantId,
    conversation_id: &str,
    agent_name: &str,
) {
    let dispatch = EventDispatch {
        user_id: athlete_id,
        tenant_id,
        category: NotificationCategory::Coach,
        event: NotificationEvent::AgentMessage,
        params: json!({ "agent_name": agent_name }),
        route: conversation_route(conversation_id),
        actions: Some(vec![NotificationActionSpec {
            id: ACTION_REPLY,
            action_type: NotificationActionType::QuickReply,
        }]),
        bypass_frequency_cap: true,
        transport_policy: TransportPolicy::AnyTransport,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P1);
}

// ============================================================================
// Agent Administration Triggers
// ============================================================================
//
// An administrator editing a system agent or assigning one reaches every
// athlete it concerns. Neither is a message: they keep the daily cap, and they
// carry no destination — no screen shows an agent's definition, and no thread
// with a newly assigned agent exists yet.

/// Trigger notification when an administrator edits a system agent the
/// athlete is assigned.
pub fn trigger_agent_updated(
    service: &Arc<NotificationService>,
    athlete_id: Uuid,
    tenant_id: TenantId,
    agent_name: &str,
) {
    let dispatch = EventDispatch {
        user_id: athlete_id,
        tenant_id,
        category: NotificationCategory::Coach,
        event: NotificationEvent::AgentUpdated,
        params: json!({ "agent_name": agent_name }),
        route: json!({}),
        actions: None,
        bypass_frequency_cap: false,
        transport_policy: TransportPolicy::AnyTransport,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P3);
}

/// Trigger notification when an administrator assigns the athlete a system
/// agent.
pub fn trigger_agent_assigned(
    service: &Arc<NotificationService>,
    athlete_id: Uuid,
    tenant_id: TenantId,
    agent_name: &str,
) {
    let dispatch = EventDispatch {
        user_id: athlete_id,
        tenant_id,
        category: NotificationCategory::Coach,
        event: NotificationEvent::AgentAssigned,
        params: json!({ "agent_name": agent_name }),
        route: json!({}),
        actions: None,
        bypass_frequency_cap: false,
        transport_policy: TransportPolicy::AnyTransport,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P2);
}

// ============================================================================
// Provider / System Triggers
// ============================================================================

/// Trigger notification when a provider sync fails.
///
/// `provider_name` is the provider as the athlete knows it. The provider's own
/// error never reaches the athlete: it is internal detail, and it is in no
/// language the athlete reads.
pub fn trigger_sync_failure(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider_name: &str,
) {
    let dispatch = EventDispatch {
        user_id,
        tenant_id,
        category: NotificationCategory::System,
        event: NotificationEvent::SyncFailure,
        params: json!({ "provider_name": provider_name }),
        route: json!({
            "screen": NotificationScreen::Connections.as_str(),
            "provider": provider_name,
        }),
        actions: Some(vec![NotificationActionSpec {
            id: ACTION_RECONNECT,
            action_type: NotificationActionType::OpenScreen,
        }]),
        bypass_frequency_cap: false,
        transport_policy: TransportPolicy::AnyTransport,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P1);
}

// ============================================================================
// Delegated Connection Triggers (bypass frequency cap)
// ============================================================================
//
// A group's coach reads a member's TrainingPeaks workouts through the coach's
// own TrainingPeaks account once the member confirms. Each step reaches the
// other side: the member is asked, the coach hears the answer, and both hear
// when TrainingPeaks drops the athlete from the coach's roster. They bypass
// the daily cap like agent traffic: each is a one-off answer between two
// people, and a coach linking a whole squad must not lose the last replies.
// The member's notices open their connections, where the TrainingPeaks card
// names the link; the coach's name the group and carry no destination, like
// the group's weekly digest. Each of the coach's names the member it concerns
// under `SUBJECT_ATHLETE_DATA_KEY`, which is what a `per_athlete` persona
// digest rolls the coach's withheld notices up on.

/// A delegated-connection notice for `user_id`, in category `coach`.
fn delegation_dispatch(
    user_id: Uuid,
    tenant_id: TenantId,
    event: NotificationEvent,
    params: Value,
    route: Value,
) -> EventDispatch {
    EventDispatch {
        user_id,
        tenant_id,
        category: NotificationCategory::Coach,
        event,
        params,
        route,
        actions: None,
        bypass_frequency_cap: true,
        transport_policy: TransportPolicy::AnyTransport,
    }
}

/// The data of a coach's notice about one member: no destination, and the
/// member it concerns.
fn about_member(member_id: Uuid, member_name: &str) -> Value {
    let subject = SubjectAthlete {
        id: member_id,
        name: member_name.to_owned(),
    };
    json!({ SUBJECT_ATHLETE_DATA_KEY: subject.to_value() })
}

/// The coaching platform a link notice is about.
#[derive(Debug, Clone, Copy)]
pub struct LinkPlatform<'a> {
    /// The platform as clients key it (`trainingpeaks`, `intervals_icu`):
    /// the connections card a notice opens.
    pub provider: &'a str,
    /// The platform's brand, as the notice's words name it.
    pub name: &'a str,
}

/// The member's connections, where the card of `provider` (the coaching
/// platform as the user knows it) names the link.
fn delegation_connections_route(provider: &str) -> Value {
    json!({
        "screen": NotificationScreen::Connections.as_str(),
        "provider": provider,
    })
}

/// Trigger notification asking a member to confirm the link on `platform`
/// their group's coach proposed. Their confirmation is the consent to the read.
pub fn trigger_delegation_proposed(
    service: &Arc<NotificationService>,
    member_id: Uuid,
    tenant_id: TenantId,
    platform: LinkPlatform<'_>,
    coach_name: &str,
    group_name: &str,
) {
    let dispatch = delegation_dispatch(
        member_id,
        tenant_id,
        NotificationEvent::DelegationProposed,
        json!({
            "coach_name": coach_name,
            "group_name": group_name,
            "platform_name": platform.name,
        }),
        delegation_connections_route(platform.provider),
    );
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P1);
}

/// Trigger notification telling the coach a member confirmed their link.
pub fn trigger_delegation_confirmed(
    service: &Arc<NotificationService>,
    coach_id: Uuid,
    tenant_id: TenantId,
    platform_name: &str,
    member_id: Uuid,
    member_name: &str,
    group_name: &str,
) {
    let dispatch = delegation_dispatch(
        coach_id,
        tenant_id,
        NotificationEvent::DelegationConfirmed,
        json!({
            "member_name": member_name,
            "group_name": group_name,
            "platform_name": platform_name,
        }),
        about_member(member_id, member_name),
    );
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P2);
}

/// Trigger notification telling the coach a member declined their link.
pub fn trigger_delegation_declined(
    service: &Arc<NotificationService>,
    coach_id: Uuid,
    tenant_id: TenantId,
    platform_name: &str,
    member_id: Uuid,
    member_name: &str,
    group_name: &str,
) {
    let dispatch = delegation_dispatch(
        coach_id,
        tenant_id,
        NotificationEvent::DelegationDeclined,
        json!({
            "member_name": member_name,
            "group_name": group_name,
            "platform_name": platform_name,
        }),
        about_member(member_id, member_name),
    );
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P2);
}

/// Trigger notification telling the coach a linked member left their
/// coaching-platform roster, which ended the link.
pub fn trigger_delegation_off_roster(
    service: &Arc<NotificationService>,
    coach_id: Uuid,
    tenant_id: TenantId,
    platform_name: &str,
    member_id: Uuid,
    member_name: &str,
    group_name: &str,
) {
    let dispatch = delegation_dispatch(
        coach_id,
        tenant_id,
        NotificationEvent::DelegationOffRoster,
        json!({
            "member_name": member_name,
            "group_name": group_name,
            "platform_name": platform_name,
        }),
        about_member(member_id, member_name),
    );
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P2);
}

/// Trigger notification telling the member their coach's roster dropped them.
///
/// Their workouts are no longer read through the coach's account. P1: their
/// data on `platform` stopped flowing.
pub fn trigger_delegation_off_coach_roster(
    service: &Arc<NotificationService>,
    member_id: Uuid,
    tenant_id: TenantId,
    platform: LinkPlatform<'_>,
    coach_name: &str,
    group_name: &str,
) {
    let dispatch = delegation_dispatch(
        member_id,
        tenant_id,
        NotificationEvent::DelegationOffCoachRoster,
        json!({
            "coach_name": coach_name,
            "group_name": group_name,
            "platform_name": platform.name,
        }),
        delegation_connections_route(platform.provider),
    );
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P1);
}

#[cfg(test)]
mod tests {
    /// The service under test is the `SQLite` one.
    #[cfg(feature = "sqlite")]
    mod sqlite_dispatch {
        use std::sync::Arc;

        use pierre_test_support::db::create_sqlite_test_db;
        use tokio_util::task::TaskTracker;
        use uuid::Uuid;

        use pierre_core::transport::TransportPolicy;

        use super::super::trigger_agent_message;
        use crate::events::data_transport_policy;
        use crate::models::TenantId;
        use crate::NotificationService;

        /// A trigger's dispatch is counted by the tracker the service was built
        /// with the moment the trigger returns, so a shutdown drain awaiting that
        /// tracker cannot miss it; and the tracker empties once it has run.
        #[tokio::test]
        async fn a_trigger_dispatches_on_the_service_tracker() {
            let db = create_sqlite_test_db().await.unwrap();
            let pool = db.sqlite_pool().unwrap().clone();
            let dispatches = TaskTracker::new();
            let service = Arc::new(NotificationService::from_sqlite(pool, dispatches.clone()));

            trigger_agent_message(
                &service,
                Uuid::new_v4(),
                TenantId(Uuid::new_v4()),
                "conversation-1",
                "Coach",
            );
            assert_eq!(
                dispatches.len(),
                1,
                "the dispatch is on the service's tracker, not a bare spawn"
            );

            dispatches.close();
            dispatches.wait().await;
            assert!(dispatches.is_empty(), "the dispatch ran to its end");
        }

        /// A trigger fired inside a derivation that served first-party-only
        /// data is stamped with it, though its dispatch runs on another task;
        /// one fired outside any is left open (carnet#769).
        #[tokio::test]
        async fn a_trigger_carries_the_stamp_of_the_derivation_it_fired_from() {
            let db = create_sqlite_test_db().await.unwrap();
            let pool = db.sqlite_pool().unwrap().clone();
            for (probe, expected) in [
                (
                    TransportPolicy::FirstPartyOnly,
                    TransportPolicy::FirstPartyOnly,
                ),
                (TransportPolicy::AnyTransport, TransportPolicy::AnyTransport),
            ] {
                let dispatches = TaskTracker::new();
                let service = Arc::new(
                    NotificationService::from_sqlite(pool.clone(), dispatches.clone())
                        .with_provenance(Arc::new(move || probe)),
                );
                let (user_id, tenant_id) = (Uuid::new_v4(), TenantId(Uuid::new_v4()));
                trigger_agent_message(&service, user_id, tenant_id, "conversation-1", "Coach");
                dispatches.close();
                dispatches.wait().await;

                let (rows, total, _) = service
                    .list_notifications(user_id, tenant_id, 10, 0, None, false)
                    .await
                    .unwrap();
                assert_eq!(total, 1, "{probe:?}");
                assert_eq!(
                    data_transport_policy(rows[0].data.as_ref()),
                    expected,
                    "{probe:?}"
                );
            }
        }
    }
}
