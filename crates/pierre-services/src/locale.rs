// ABOUTME: One resolver for "what language does this athlete read" — tenant-scoped, with a default
// ABOUTME: Replaces ten hand-written copies, two of which disagreed about the empty-string case
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete locale resolution.
//!
//! Ten call sites asked the same question — REST handlers, chat routes, store
//! and memory tools, the persona cards — and answered it ten times. The copies
//! were not identical: only the coaches one treated a stored empty string as
//! "no preference", so every other site would have handed `""` to the string
//! registry as if it were a locale.
//!
//! The read is global rather than tenant-scoped, which is the one place these
//! copies disagreed and the disagreement mattered. `locale` is a column on the
//! user row: one athlete, one stored language, whatever tenant they are acting
//! in. The tenant-scoped read is an `INNER JOIN tenant_users`, so a missing
//! membership row does not return "no tenant access" — it returns "no
//! preference", and the athlete silently reads French. Every caller here
//! resolves the language of the *authenticated caller themselves*, so there is
//! no cross-tenant read to guard against, and answering the question with the
//! join risks the wrong language for no isolation gain.

use pierre_contremaitre::messaging_strings::DEFAULT_LOCALE;
use pierre_database::repositories::UserRepository;
use uuid::Uuid;

/// The locale an athlete reads, or [`DEFAULT_LOCALE`] when they have no usable
/// preference.
///
/// A missing row, a failed lookup and a stored empty string all mean the same
/// thing to a caller — "we do not know, use the default" — so all three land on
/// the default rather than three different behaviours per call site.
pub async fn resolve_user_locale(users: &dyn UserRepository, user_id: Uuid) -> String {
    users
        .get_global(user_id)
        .await
        .ok()
        .flatten()
        .map(|user| user.locale)
        .filter(|locale| !locale.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_LOCALE.to_owned())
}
