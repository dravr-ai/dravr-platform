// ABOUTME: Catalogue keys of the reconnect copy for a sign-in session its grant still serves, and the reconnect push
// ABOUTME: The chat sentence a dead session beside a live grant gets, a dead writing grant beside a session, and the pushes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Keys of the reconnect copy (carnet#1494).
//!
//! A provider's sign-in session serves its reads first, and its live OAuth
//! grant answers when the session cannot. When the session dies the grant
//! keeps the provider syncing, so the athlete is told it still syncs for now
//! and asked to sign in again to keep it that way. The app's reconnect push
//! is worded from the catalogue too, in the athlete's locale. A grant that
//! dies beside a session serving its provider, when only that grant writes
//! (Garmin's workouts and calendar), is told the reads go on and the writes
//! need the grant again. `{0}` is the provider as the athlete knows it;
//! `{1}`, where a key has it, the link.

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

/// Key: "One note: your `{0}` activities are still syncing, but sending
/// workouts and calendar entries to `{0}` needs your `{0}` connection again.
/// Connect it here (link valid for 1 hour): `{1}`"
///
/// The chat half of the notice a dead grant gets beside a session that
/// serves its provider, when only the grant writes (Garmin's workout and
/// calendar push, JF's directive 8): the reads go on, and the link starts
/// the grant's own authorization.
pub const KEY_PROVIDER_REAUTH_GRANT_WRITES: &str = "messaging.provider.reauth_grant_writes";
/// The same standing as [`KEY_PROVIDER_REAUTH_GRANT_WRITES`], said without a
/// link: connect again from the app's Connections. `{0}` only.
pub const KEY_PROVIDER_REAUTH_GRANT_WRITES_NO_LINK: &str =
    "messaging.provider.reauth_grant_writes_no_link";

/// Key: "Reconnect to send workouts" No format placeholders.
pub const KEY_NOTIFICATION_PROVIDER_GRANT_WRITES_TITLE: &str =
    "notifications.provider_grant_writes.title";
/// Key: "Your `{0}` activities still sync. Connect `{0}` again to send
/// workouts and calendar entries."
pub const KEY_NOTIFICATION_PROVIDER_GRANT_WRITES_BODY: &str =
    "notifications.provider_grant_writes.body";
