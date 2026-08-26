// ABOUTME: Notification triggers for intelligence events, coach communications, and sync failures
// ABOUTME: All fire-and-forget via tokio::spawn — failures logged at WARN, never block the caller
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Notification Triggers
//!
//! One function per product event, each fire-and-forget.
//!
//! A trigger builds a `DispatchRequest` for its event and spawns an async task
//! to dispatch it. Failures are logged at WARN level but never block the
//! caller — notification delivery is always fire-and-forget.
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

use crate::models::{NotificationAction, NotificationActionType, NotificationCategory, TenantId};
use crate::{DispatchRequest, NotificationService};

/// Spawns a fire-and-forget notification dispatch task.
/// Failures are logged at WARN level but never propagated to the caller.
fn spawn_dispatch(service: Arc<NotificationService>, request: DispatchRequest) {
    tokio::spawn(async move {
        if let Err(e) = service.dispatch(&request).await {
            warn!(
                user_id = %request.user_id,
                notification_type = %request.notification_type,
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
    let request = DispatchRequest {
        user_id,
        tenant_id,
        category: NotificationCategory::Training,
        notification_type: "activity_synced".to_owned(),
        title: "New activity synced".to_owned(),
        body: format!("{activity_type} — {distance_display} in {duration_display}"),
        data: Some(json!({ "screen": "activity", "id": activity_id })),
        image_url: None,
        actions: None,
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), request);
}

/// Trigger notification when acute training load exceeds threshold.
pub fn trigger_training_load_alert(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    atl_value: f64,
) {
    let request = DispatchRequest {
        user_id,
        tenant_id,
        category: NotificationCategory::Training,
        notification_type: "training_load_alert".to_owned(),
        title: "Training load elevated".to_owned(),
        body: format!("Your acute training load is {atl_value:.0} — consider a recovery day"),
        data: Some(json!({ "screen": "recovery" })),
        image_url: None,
        actions: None,
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), request);
}

/// Trigger notification when recovery score drops below threshold.
pub fn trigger_low_recovery_score(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    score: f64,
) {
    let request = DispatchRequest {
        user_id,
        tenant_id,
        category: NotificationCategory::Recovery,
        notification_type: "low_recovery_score".to_owned(),
        title: "Recovery score is low".to_owned(),
        body: format!("Your recovery score is {score:.0}/100 — easy day recommended"),
        data: Some(json!({ "screen": "recovery" })),
        image_url: None,
        actions: None,
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), request);
}

/// Trigger notification when TSS trend suggests overtraining risk.
pub fn trigger_overtraining_warning(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
) {
    let request = DispatchRequest {
        user_id,
        tenant_id,
        category: NotificationCategory::Recovery,
        notification_type: "overtraining_warning".to_owned(),
        title: "Overtraining risk detected".to_owned(),
        body: "Your training stress trend suggests fatigue accumulation".to_owned(),
        data: Some(json!({ "screen": "recovery" })),
        image_url: None,
        actions: None,
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), request);
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
    let request = DispatchRequest {
        user_id,
        tenant_id,
        category: NotificationCategory::Achievement,
        notification_type: "personal_record".to_owned(),
        title: "New personal record!".to_owned(),
        body: format!("New {distance_label} PR: {time_display}"),
        data: Some(json!({ "screen": "activity", "id": activity_id })),
        image_url: None,
        actions: None,
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), request);
}

/// Trigger notification when a cumulative milestone is reached.
pub fn trigger_milestone_reached(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    value_display: &str,
    unit: &str,
) {
    let request = DispatchRequest {
        user_id,
        tenant_id,
        category: NotificationCategory::Achievement,
        notification_type: "milestone_reached".to_owned(),
        title: "Milestone reached!".to_owned(),
        body: format!("You've logged {value_display} {unit} this year"),
        data: Some(json!({ "screen": "activities" })),
        image_url: None,
        actions: None,
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), request);
}

/// Trigger notification when a fitness metric improves (FTP, `VO2max`, etc.).
pub fn trigger_fitness_improvement(
    service: &Arc<NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    metric_name: &str,
    value_display: &str,
) {
    let request = DispatchRequest {
        user_id,
        tenant_id,
        category: NotificationCategory::Achievement,
        notification_type: "fitness_improvement".to_owned(),
        title: "Fitness improvement detected".to_owned(),
        body: format!("Your {metric_name} increased to {value_display}"),
        data: Some(json!({ "screen": "stats" })),
        image_url: None,
        actions: None,
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), request);
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
    let request = DispatchRequest {
        user_id: athlete_id,
        tenant_id,
        category: NotificationCategory::Coach,
        notification_type: "coach_message".to_owned(),
        title: "Message from your coach".to_owned(),
        body: format!("{coach_name} sent you a message"),
        data: Some(json!({ "screen": "coach", "action": "chat", "id": conversation_id })),
        image_url: None,
        actions: Some(vec![NotificationAction {
            id: "reply".to_owned(),
            title: "Reply".to_owned(),
            action_type: NotificationActionType::QuickReply,
        }]),
        bypass_frequency_cap: true,
    };
    spawn_dispatch(Arc::clone(service), request);
}

/// Trigger notification when a coach updates an athlete's training plan.
pub fn trigger_plan_updated(
    service: &Arc<NotificationService>,
    athlete_id: Uuid,
    tenant_id: TenantId,
    coach_name: &str,
) {
    let request = DispatchRequest {
        user_id: athlete_id,
        tenant_id,
        category: NotificationCategory::Coach,
        notification_type: "plan_updated".to_owned(),
        title: "Training plan updated".to_owned(),
        body: format!("{coach_name} updated your training plan"),
        data: Some(json!({ "screen": "coach", "action": "plan" })),
        image_url: None,
        actions: None,
        bypass_frequency_cap: true,
    };
    spawn_dispatch(Arc::clone(service), request);
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
    let request = DispatchRequest {
        user_id: athlete_id,
        tenant_id,
        category: NotificationCategory::Coach,
        notification_type: "coach_feedback".to_owned(),
        title: "Coach feedback".to_owned(),
        body: format!("{coach_name} left a note on your {activity_type}"),
        data: Some(json!({ "screen": "activity", "id": activity_id })),
        image_url: None,
        actions: None,
        bypass_frequency_cap: true,
    };
    spawn_dispatch(Arc::clone(service), request);
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
    let request = DispatchRequest {
        user_id,
        tenant_id,
        category: NotificationCategory::System,
        notification_type: "sync_failure".to_owned(),
        title: format!("{provider_name} sync failed"),
        body: error_summary.to_owned(),
        data: Some(
            json!({ "screen": "settings", "action": "reconnect", "provider": provider_name }),
        ),
        image_url: None,
        actions: Some(vec![NotificationAction {
            id: "reconnect".to_owned(),
            title: "Reconnect".to_owned(),
            action_type: NotificationActionType::OpenScreen,
        }]),
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), request);
}
