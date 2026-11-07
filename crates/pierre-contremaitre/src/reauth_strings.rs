// ABOUTME: Catalogue keys of the reconnect copy for a connection the provider's own OAuth replaces, and the reconnect push
// ABOUTME: The chat sentence and the app push a dead usr/pwd-only Garmin session gets while a seat is free on Garmin's OAuth
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Keys of the reconnect copy (carnet#1454).
//!
//! A usr/pwd session that served an athlete alone and died is reconnected
//! through the provider's own OAuth while a seat is free there: the chat
//! sentence says so and links the connect picker, never the usr/pwd sign-in.
//! The app's reconnect push is worded from the catalogue too, in the
//! athlete's locale, with its own wording for that move. `{0}` is the
//! provider as the athlete knows it; `{1}`, where a key has it, the link.

/// Key: "Your `{0}` connection has expired — I can't fetch your data right now.
/// `{0}` now connects through its official sign-in: connect it here (link
/// valid for 1 hour): `{1}` …"
pub const KEY_PROVIDER_REAUTH_OAUTH_REQUIRED: &str = "messaging.provider.reauth_oauth_required";
/// The same standing as [`KEY_PROVIDER_REAUTH_OAUTH_REQUIRED`], said without
/// a link: open Connections in Dravr and connect `{0}`. `{0}` only.
pub const KEY_PROVIDER_REAUTH_OAUTH_REQUIRED_NO_LINK: &str =
    "messaging.provider.reauth_oauth_required_no_link";
/// Key: the move to `{0}`'s official connection on a turn the athlete's other
/// connections already answered. `{0}` = provider, `{1}` = the link.
pub const KEY_PROVIDER_REAUTH_OAUTH_SERVED: &str = "messaging.provider.reauth_oauth_served";
/// The same standing as [`KEY_PROVIDER_REAUTH_OAUTH_SERVED`], said without a
/// link. `{0}` only.
pub const KEY_PROVIDER_REAUTH_OAUTH_SERVED_NO_LINK: &str =
    "messaging.provider.reauth_oauth_served_no_link";

/// Key: "Reconnect needed" No format placeholders.
pub const KEY_NOTIFICATION_PROVIDER_REAUTH_TITLE: &str = "notifications.provider_reauth.title";
/// Key: "Reconnect `{0}` to keep your data coming."
pub const KEY_NOTIFICATION_PROVIDER_REAUTH_BODY: &str = "notifications.provider_reauth.body";
/// Key: "Move `{0}` to its official connection"
pub const KEY_NOTIFICATION_PROVIDER_OAUTH_REQUIRED_TITLE: &str =
    "notifications.provider_oauth_required.title";
/// Key: "Your `{0}` sign-in expired. Connect `{0}` through its official
/// connection to keep your data coming."
pub const KEY_NOTIFICATION_PROVIDER_OAUTH_REQUIRED_BODY: &str =
    "notifications.provider_oauth_required.body";
