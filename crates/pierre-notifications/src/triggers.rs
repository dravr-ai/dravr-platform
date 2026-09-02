// ABOUTME: Notification triggers for intelligence events, coach communications, and sync failures
// ABOUTME: All fire-and-forget via tokio::spawn — failures logged at WARN, never block the caller
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

use std::sync::Arc;

use serde_json::json;
use tracing::warn;
use uuid::Uuid;

use crate::events::{NotificationActionSpec, ACTION_RECONNECT, ACTION_REPLY};
use crate::models::{NotificationActionType, NotificationCategory, TenantId};
use crate::{EventDispatch, NotificationEvent, NotificationService, PushTier};

/// Spawns a fire-and-forget notification dispatch task at the event's tier.
///
/// Each trigger declares its own [`PushTier`] because the trigger knows its
/// event's product semantics — the facade only compares the tier against the
/// recipient's persona floor. Failures are logged at WARN level but never
/// propagated to the caller.
fn spawn_dispatch(service: Arc<NotificationService>, dispatch: EventDispatch, tier: PushTier) {
    tokio::spawn(async move {
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

// ============================================================================
// Intelligence / Training Triggers
// ============================================================================

/// Trigger notification when a new activity is synced from a provider.
pub fn trigger_activity_synced(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    activity_id: &str,
    activity_type: &str,
    distance_display: &str,
    duration_display: &str,
) {
    let dispatch = EventDispatch {
        user_id,
        tenant_id,
        category: NotificationCategory::Training,
        event: NotificationEvent::ActivitySynced,
        params: json!({
            "activity_type": activity_type,
            "distance_display": distance_display,
            "duration_display": duration_display,
        }),
        route: json!({ "screen": "activity", "id": activity_id }),
        actions: None,
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P3);
}

/// Trigger notification when acute training load exceeds threshold.
pub fn trigger_training_load_alert(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    atl_value: f64,
) {
    let dispatch = EventDispatch {
        user_id,
        tenant_id,
        category: NotificationCategory::Training,
        event: NotificationEvent::TrainingLoadAlert,
        params: json!({ "atl_value": format!("{atl_value:.0}") }),
        route: json!({ "screen": "recovery" }),
        actions: None,
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P2);
}

/// Trigger notification when recovery score drops below threshold.
pub fn trigger_low_recovery_score(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    score: f64,
) {
    let dispatch = EventDispatch {
        user_id,
        tenant_id,
        category: NotificationCategory::Recovery,
        event: NotificationEvent::LowRecoveryScore,
        params: json!({ "score": format!("{score:.0}") }),
        route: json!({ "screen": "recovery" }),
        actions: None,
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P2);
}

/// Trigger notification when TSS trend suggests overtraining risk.
pub fn trigger_overtraining_warning(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
) {
    let dispatch = EventDispatch {
        user_id,
        tenant_id,
        category: NotificationCategory::Recovery,
        event: NotificationEvent::OvertrainingWarning,
        params: json!({}),
        route: json!({ "screen": "recovery" }),
        actions: None,
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P2);
}

/// Trigger notification when a personal record is detected.
pub fn trigger_personal_record(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    activity_id: &str,
    distance_label: &str,
    time_display: &str,
) {
    let dispatch = EventDispatch {
        user_id,
        tenant_id,
        category: NotificationCategory::Achievement,
        event: NotificationEvent::PersonalRecord,
        params: json!({ "distance_label": distance_label, "time_display": time_display }),
        route: json!({ "screen": "activity", "id": activity_id }),
        actions: None,
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P3);
}

/// Trigger notification when a cumulative milestone is reached.
pub fn trigger_milestone_reached(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    value_display: &str,
    unit: &str,
) {
    let dispatch = EventDispatch {
        user_id,
        tenant_id,
        category: NotificationCategory::Achievement,
        event: NotificationEvent::MilestoneReached,
        params: json!({ "value_display": value_display, "unit": unit }),
        route: json!({ "screen": "activities" }),
        actions: None,
        bypass_frequency_cap: false,
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
) {
    let dispatch = EventDispatch {
        user_id,
        tenant_id,
        category: NotificationCategory::Achievement,
        event: NotificationEvent::FitnessImprovement,
        params: json!({ "metric_name": metric_name, "value_display": value_display }),
        route: json!({ "screen": "stats" }),
        actions: None,
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P3);
}

// ============================================================================
// Coach Triggers (bypass frequency cap)
// ============================================================================

/// Trigger notification when a coach sends a message to an athlete.
pub fn trigger_coach_message(
    service: &Arc<NotificationService>,
    athlete_id: Uuid,
    tenant_id: TenantId,
    conversation_id: &str,
    coach_name: &str,
) {
    let dispatch = EventDispatch {
        user_id: athlete_id,
        tenant_id,
        category: NotificationCategory::Coach,
        event: NotificationEvent::CoachMessage,
        params: json!({ "coach_name": coach_name }),
        route: json!({ "screen": "coach", "action": "chat", "id": conversation_id }),
        actions: Some(vec![NotificationActionSpec {
            id: ACTION_REPLY,
            action_type: NotificationActionType::QuickReply,
        }]),
        bypass_frequency_cap: true,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P1);
}

/// Trigger notification when a coach updates an athlete's training plan.
pub fn trigger_plan_updated(
    service: &Arc<NotificationService>,
    athlete_id: Uuid,
    tenant_id: TenantId,
    coach_name: &str,
) {
    let dispatch = EventDispatch {
        user_id: athlete_id,
        tenant_id,
        category: NotificationCategory::Coach,
        event: NotificationEvent::PlanUpdated,
        params: json!({ "coach_name": coach_name }),
        route: json!({ "screen": "coach", "action": "plan" }),
        actions: None,
        bypass_frequency_cap: true,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P1);
}

/// Trigger notification when a coach leaves feedback on an athlete's activity.
pub fn trigger_coach_feedback(
    service: &Arc<NotificationService>,
    athlete_id: Uuid,
    tenant_id: TenantId,
    activity_id: &str,
    coach_name: &str,
    activity_type: &str,
) {
    let dispatch = EventDispatch {
        user_id: athlete_id,
        tenant_id,
        category: NotificationCategory::Coach,
        event: NotificationEvent::CoachFeedback,
        params: json!({ "coach_name": coach_name, "activity_type": activity_type }),
        route: json!({ "screen": "activity", "id": activity_id }),
        actions: None,
        bypass_frequency_cap: true,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P1);
}

// ============================================================================
// Provider / System Triggers
// ============================================================================

/// Trigger notification when a provider sync fails.
pub fn trigger_sync_failure(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider_name: &str,
    error_summary: &str,
) {
    let dispatch = EventDispatch {
        user_id,
        tenant_id,
        category: NotificationCategory::System,
        event: NotificationEvent::SyncFailure,
        params: json!({ "provider_name": provider_name, "error_summary": error_summary }),
        route: json!({ "screen": "settings", "action": "reconnect", "provider": provider_name }),
        actions: Some(vec![NotificationActionSpec {
            id: ACTION_RECONNECT,
            action_type: NotificationActionType::OpenScreen,
        }]),
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), dispatch, PushTier::P1);
}
