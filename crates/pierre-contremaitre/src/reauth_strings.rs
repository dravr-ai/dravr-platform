// ABOUTME: Catalogue keys of the reconnect copy for a sign-in session its grant still serves, and the reconnect push
// ABOUTME: The chat sentence a dead session beside a live grant gets, and the app push every reconnect notice sends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Keys of the reconnect copy (carnet#1494).
//!
//! A provider's sign-in session serves its reads first, and its live OAuth
//! grant answers when the session cannot. When the session dies the grant
//! keeps the provider syncing, so the athlete is told it still syncs for now
//! and asked to sign in again to keep it that way. The app's reconnect push
//! is worded from the catalogue too, in the athlete's locale. `{0}` is the
//! provider as the athlete knows it; `{1}`, where a key has it, the link.

/// Key: "One note: `{0}` is still syncing for now, but your `{0}` sign-in has
/// expired. Sign in to `{0}` again here to keep it that way: `{1}`"
pub const KEY_PROVIDER_REAUTH_FALLBACK_SERVED: &str = "messaging.provider.reauth_fallback_served";
/// The same standing as [`KEY_PROVIDER_REAUTH_FALLBACK_SERVED`], said
/// without a link: sign in again from the settings. `{0}` only.
pub const KEY_PROVIDER_REAUTH_FALLBACK_SERVED_NO_LINK: &str =
    "messaging.provider.reauth_fallback_served_no_link";

/// Key: "Reconnect needed" No format placeholders.
pub const KEY_NOTIFICATION_PROVIDER_REAUTH_TITLE: &str = "notifications.provider_reauth.title";
/// Key: "Reconnect `{0}` to keep your data coming."
pub const KEY_NOTIFICATION_PROVIDER_REAUTH_BODY: &str = "notifications.provider_reauth.body";
