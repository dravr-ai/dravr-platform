// ABOUTME: UserApprovalNotifier — the injected seam for notifying a just-approved
// ABOUTME: user by account email plus a localized message on each linked channel.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! User-approval notification seam.
//!
//! Every approval path (admin REST, web-admin, Slack ops button, and
//! registration auto-approve) notifies the approved user identically: an
//! account-approved email plus a message on each of their linked messaging
//! channels (Telegram/WhatsApp/Slack/…). The concrete implementation lives in
//! the binary (`pierre-server`), where the email service and messaging
//! adapters are wired; it is injected into each approval path's context behind
//! this trait so the route crates need no messaging dependency.

use async_trait::async_trait;
use uuid::Uuid;

/// Notifies a just-approved user across every surface.
///
/// Best-effort by contract: implementations log and swallow per-surface
/// failures so a notification never fails the approval itself. The user's
/// tenant(s) and linked channels are resolved by the implementation, so
/// callers pass only the user's identity.
#[async_trait]
pub trait UserApprovalNotifier: Send + Sync {
    /// Send the account-approved email and a localized "approved" message to
    /// each of the user's linked messaging channels.
    async fn notify_user_approved(&self, user_id: Uuid, email: &str, display_name: Option<&str>);
}
