// ABOUTME: A conversation's messaging-session binding and its onboarding state: set, compare-and-set and list
// ABOUTME: Split from chat.rs so it stays under the size ceiling; chat.rs re-exports every statement

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Bind a conversation to its messaging session.
pub(crate) const SET_SESSION_ID_SQL: &str = r"
    UPDATE chat_conversations
    SET session_id = $1
    WHERE id = $2 AND tenant_id = $3";

/// Replace the onboarding state unconditionally.
pub(crate) const SET_ONBOARDING_STATE_SQL: &str = r"
    UPDATE chat_conversations
    SET onboarding_state = $1
    WHERE id = $2 AND tenant_id = $3";

/// Replace the onboarding state only when it still holds `$4`. `IS NOT
/// DISTINCT FROM` is the NULL-safe equality both engines share: a
/// conversation that carried no state matches only an absent `expected`,
/// never any stored JSON.
pub(crate) const CAS_ONBOARDING_STATE_SQL: &str = r"
    UPDATE chat_conversations
    SET onboarding_state = $1
    WHERE id = $2 AND tenant_id = $3
      AND onboarding_state IS NOT DISTINCT FROM $4";

/// A user's onboarding states, newest activity first.
pub(crate) const LIST_ONBOARDING_STATES_SQL: &str = r"
    SELECT onboarding_state
    FROM chat_conversations
    WHERE user_id = $1 AND tenant_id = $2 AND onboarding_state IS NOT NULL
    ORDER BY updated_at DESC
    LIMIT $3";
