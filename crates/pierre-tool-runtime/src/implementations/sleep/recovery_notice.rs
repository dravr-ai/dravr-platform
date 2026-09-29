// ABOUTME: The low-recovery notification the recovery tool fires when a score falls below threshold
// ABOUTME: Names the conversation the tool ran in, so a tap opens the thread where the agent read the score
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_notifications::triggers as notification_triggers;
use pierre_notifications::TenantId;
use uuid::Uuid;

use crate::context::scoped_conversation_id;
use crate::runtime::ToolRuntime;

/// Recovery score below which the athlete is told to take an easy day.
const LOW_RECOVERY_THRESHOLD: f64 = 40.0;

/// Fire the low-recovery notification when `score` falls below the threshold.
///
/// The notification names the conversation the tool is answering in, when it
/// runs in one, so a tap opens the thread where the agent explained the score.
pub(super) fn fire_low_recovery_notification(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id_str: Option<&str>,
    score: f64,
) {
    if score >= LOW_RECOVERY_THRESHOLD {
        return;
    }
    let Some(service) = resources.notification_service() else {
        return;
    };
    let Some(tenant_str) = tenant_id_str else {
        return;
    };
    let Ok(tenant_uuid) = tenant_str.parse::<Uuid>() else {
        return;
    };
    notification_triggers::trigger_low_recovery_score(
        service,
        user_id,
        TenantId(tenant_uuid),
        score,
        scoped_conversation_id().as_deref(),
    );
}
