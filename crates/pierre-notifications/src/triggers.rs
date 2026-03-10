// ABOUTME: Notification triggers for intelligence events, social interactions, and coach communications
// ABOUTME: All fire-and-forget via tokio::spawn — failures logged at WARN, never block the caller
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Notification Triggers
//!
//! Each trigger function builds a `DispatchRequest` for a specific event
//! and spawns an async task to dispatch it. Failures are logged at WARN level but
//! never block the caller — notification delivery is always fire-and-forget.

use pierre_core::models::notifications::{
    NotificationAction, NotificationActionType, NotificationCategory,
};
use pierre_core::models::TenantId;
use serde_json::json;
use std::sync::Arc;
use tracing::warn;
use uuid::Uuid;

use crate::dispatch::DispatchRequest;
use crate::service::NotificationService;

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
///
/// Example body: "Run — 10.2 km in 52:14"
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
///
/// Threshold is caller-defined (e.g., ATL > 1.5 * CTL).
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
///
/// Threshold is caller-defined (e.g., score < 40).
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
///
/// Example body: "New 5K PR: 22:14"
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
///
/// Example body: "You've logged 1,000 km this year"
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
///
/// Example body: "Your FTP increased to 265W"
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
// Social Triggers
// ============================================================================

/// Trigger notification when a user receives a friend request.
///
/// Notifies the receiver that someone wants to connect.
pub fn trigger_friend_request_received(
    service: &Arc<NotificationService>,
    receiver_id: Uuid,
    tenant_id: TenantId,
    request_id: &str,
    sender_name: &str,
) {
    let request = DispatchRequest {
        user_id: receiver_id,
        tenant_id,
        category: NotificationCategory::Social,
        notification_type: "friend_request_received".to_owned(),
        title: "Friend request".to_owned(),
        body: format!("{sender_name} wants to connect"),
        data: Some(json!({ "screen": "social", "action": "friend_request", "id": request_id })),
        image_url: None,
        actions: Some(vec![
            NotificationAction {
                id: "accept".to_owned(),
                title: "Accept".to_owned(),
                action_type: NotificationActionType::AcceptDecline,
            },
            NotificationAction {
                id: "decline".to_owned(),
                title: "Decline".to_owned(),
                action_type: NotificationActionType::AcceptDecline,
            },
        ]),
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), request);
}

/// Trigger notification when a friend request is accepted.
///
/// Notifies the original requester that their request was accepted.
pub fn trigger_friend_request_accepted(
    service: &Arc<NotificationService>,
    initiator_id: Uuid,
    tenant_id: TenantId,
    accepter_name: &str,
) {
    let request = DispatchRequest {
        user_id: initiator_id,
        tenant_id,
        category: NotificationCategory::Social,
        notification_type: "friend_request_accepted".to_owned(),
        title: "Connection accepted".to_owned(),
        body: format!("{accepter_name} accepted your connection"),
        data: Some(json!({ "screen": "social" })),
        image_url: None,
        actions: None,
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), request);
}

/// Trigger notification when someone reacts to or gives kudos on an insight.
///
/// Notifies the insight owner about the reaction.
pub fn trigger_activity_kudos(
    service: &Arc<NotificationService>,
    insight_owner_id: Uuid,
    tenant_id: TenantId,
    insight_id: &str,
    reactor_name: &str,
    insight_type: &str,
) {
    let request = DispatchRequest {
        user_id: insight_owner_id,
        tenant_id,
        category: NotificationCategory::Social,
        notification_type: "activity_kudos".to_owned(),
        title: "New kudos".to_owned(),
        body: format!("{reactor_name} gave kudos on your {insight_type}"),
        data: Some(json!({ "screen": "activity", "id": insight_id })),
        image_url: None,
        actions: None,
        bypass_frequency_cap: false,
    };
    spawn_dispatch(Arc::clone(service), request);
}

/// Trigger notification when someone shares an insight visible to a user.
///
/// Notifies friends about a shared coaching insight.
pub fn trigger_insight_shared(
    service: &Arc<NotificationService>,
    recipient_id: Uuid,
    tenant_id: TenantId,
    insight_id: &str,
    sharer_name: &str,
) {
    let request = DispatchRequest {
        user_id: recipient_id,
        tenant_id,
        category: NotificationCategory::Social,
        notification_type: "insight_shared".to_owned(),
        title: "Insight shared with you".to_owned(),
        body: format!("{sharer_name} shared a coaching insight"),
        data: Some(json!({ "screen": "social", "id": insight_id })),
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
///
/// Coach notifications bypass the daily frequency cap so athletes always
/// receive their coach's communications.
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
///
/// Coach notifications bypass the daily frequency cap.
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
///
/// Coach notifications bypass the daily frequency cap.
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

/// Trigger notification when a provider sync fails (OAuth expired, API error, etc.).
///
/// Includes a "Reconnect" action button to guide users to re-authorize.
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
