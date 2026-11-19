// ABOUTME: Hands the workout a live activity write landed to the server's landing sink, off the write's path
// ABOUTME: The store decided the landing; this only announces it, so a fetch never waits on a post or a digest
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_core::models::TenantId;
use pierre_services::live_activities::LandedSession;
use uuid::Uuid;

use crate::runtime::ToolRuntime;

/// Announce `landed`, the workout a live write landed for the athlete
/// ([`pierre_services::live_activities::LiveIngest::landed`]), to the
/// runtime's [`SessionLandedSink`](crate::runtime::SessionLandedSink), off
/// the write's path: the sink may post the coach's reaction or send the
/// per-session digest, and a turn's fetch must not wait on either. Nothing
/// happens when nothing landed or no sink is wired.
pub(super) fn announce(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    landed: Option<LandedSession>,
) {
    let (Some(session), Some(sink)) = (landed, runtime.session_landed_sink()) else {
        return;
    };
    tokio::spawn(async move {
        sink.session_landed(user_id, tenant_id, session).await;
    });
}
