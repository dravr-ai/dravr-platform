// ABOUTME: The shapes the coach tools answer with, and the schemas derived from them
// ABOUTME: Separate from coaches.rs because that file is at its size ceiling and frozen
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Result types for the thirteen coach tools.
//!
//! These live beside `coaches.rs` rather than inside it because that file is
//! past the 1200-line ceiling and frozen at its current size, exactly as
//! `goals_output` sits beside `goals.rs`. The answer shapes are a coherent
//! unit — the tests and the derived schemas both name them — and none of them
//! needs the tool plumbing next door.

use pierre_core::models::coaches::{Coach, CoachListItem};
use serde::Serialize;

/// One coach as `list_coaches` reports it.
///
/// A projection of the enriched list row, not the stored coach: it carries the
/// usage signals (`is_favorite`, `use_count`, `last_used_at`) that only the
/// list query joins in, and leaves out `system_prompt`, which is long and is
/// what `get_coach` is for.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CoachListEntry {
    /// Identifier the other coach tools take.
    pub id: String,
    /// Display name.
    pub title: String,
    /// What the coach is for; absent when none was given.
    pub description: Option<String>,
    /// Which shelf it sits on: training, nutrition, recovery, recipes,
    /// mobility, analysis or custom.
    pub category: String,
    /// Free-form labels for filtering and search.
    pub tags: Vec<String>,
    /// Estimated size of the coach's system prompt, in tokens.
    pub token_count: u32,
    /// Whether the athlete has starred it.
    pub is_favorite: bool,
    /// Whether it ships with the platform rather than being athlete-authored.
    pub is_system: bool,
    /// Whether it is assigned to this athlete.
    pub is_assigned: bool,
    /// How many times it has been used.
    pub use_count: u32,
    /// RFC 3339 timestamp of the last use; absent if never used.
    pub last_used_at: Option<String>,
    /// RFC 3339 timestamp of the last edit.
    pub updated_at: String,
}

/// What `list_coaches` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ListCoachesResult {
    /// The coaches on this page.
    pub coaches: Vec<CoachListEntry>,
    /// How many came back on this page.
    pub count: usize,
    /// How many the athlete has in total, ignoring paging.
    pub total: u32,
    /// The paging offset these start at.
    pub offset: u32,
    /// The page size in force, 50 when the caller named none.
    pub limit: u32,
    /// Whether another page follows. Always false when the caller set no
    /// limit, because there is then nothing to compare the count against.
    pub has_more: bool,
}

/// What `create_coach` answers with.
///
/// Deliberately does not echo `system_prompt` or the `sample_prompts` the tool
/// accepts: the caller just sent them, and the prompt is long.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CreateCoachResult {
    /// Identifier of the new coach.
    pub id: String,
    /// Display name.
    pub title: String,
    /// What it is for; absent when none was given.
    pub description: Option<String>,
    /// Which shelf it sits on.
    pub category: String,
    /// Free-form labels.
    pub tags: Vec<String>,
    /// Estimated size of the system prompt, in tokens.
    pub token_count: u32,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
}

/// What `get_coach` answers with.
///
/// Carries `system_prompt`, which the list projection omits — reading one
/// coach in full is what this tool is for.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetCoachResult {
    /// Identifier.
    pub id: String,
    /// Display name.
    pub title: String,
    /// What it is for; absent when none was given.
    pub description: Option<String>,
    /// The instructions the coach runs on.
    pub system_prompt: String,
    /// Which shelf it sits on.
    pub category: String,
    /// Free-form labels.
    pub tags: Vec<String>,
    /// Estimated size of the system prompt, in tokens.
    pub token_count: u32,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 timestamp of the last edit.
    pub updated_at: String,
}

/// What `update_coach` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct UpdateCoachResult {
    /// Identifier.
    pub id: String,
    /// Display name after the edit.
    pub title: String,
    /// What it is for; absent when none is set.
    pub description: Option<String>,
    /// The instructions after the edit.
    pub system_prompt: String,
    /// Which shelf it sits on.
    pub category: String,
    /// Free-form labels.
    pub tags: Vec<String>,
    /// Estimated size of the system prompt after the edit, in tokens.
    pub token_count: u32,
    /// RFC 3339 timestamp of this edit.
    pub updated_at: String,
}

/// What `delete_coach` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DeleteCoachResult {
    /// Always true: the tool errors rather than reporting a failed delete.
    pub deleted: bool,
    /// The coach that was removed, echoed back.
    pub coach_id: String,
}

/// What `toggle_coach_favorite` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ToggleCoachFavoriteResult {
    /// The coach whose star was flipped.
    pub coach_id: String,
    /// Its state AFTER the flip, so a caller need not track the previous one.
    pub is_favorite: bool,
}

/// One coach as `search_coaches` reports it.
///
/// Narrower than the list entry: a search result is for picking, so it omits
/// the usage signals and timestamps.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CoachSearchEntry {
    /// Identifier.
    pub id: String,
    /// Display name.
    pub title: String,
    /// What it is for; absent when none was given.
    pub description: Option<String>,
    /// Which shelf it sits on.
    pub category: String,
    /// Free-form labels.
    pub tags: Vec<String>,
    /// Estimated size of the system prompt, in tokens.
    pub token_count: u32,
}

/// What `search_coaches` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SearchCoachesResult {
    /// The query, echoed back.
    pub query: String,
    /// The matches on this page.
    pub results: Vec<CoachSearchEntry>,
    /// How many came back. Named `returned_count` rather than `count` here,
    /// unlike `list_coaches` — kept as-is because renaming it would change a
    /// wire shape for no gain.
    pub returned_count: usize,
    /// The paging offset these start at.
    pub offset: u32,
    /// The page size in force, 20 when the caller named none.
    pub limit: u32,
    /// Whether another page follows.
    pub has_more: bool,
}

/// What `activate_coach` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ActivateCoachResult {
    /// Identifier of the coach now active.
    pub id: String,
    /// Display name.
    pub title: String,
    /// What it is for; absent when none was given.
    pub description: Option<String>,
    /// The instructions it runs on.
    pub system_prompt: String,
    /// Which shelf it sits on.
    pub category: String,
    /// Always true: the tool errors rather than reporting a failed activation.
    pub is_active: bool,
    /// Estimated size of the system prompt, in tokens.
    pub token_count: u32,
}

/// What `deactivate_coach` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DeactivateCoachResult {
    /// Whether a coach was actually deactivated. False when none was active,
    /// which is a success rather than an error.
    pub deactivated: bool,
}

/// The active coach in full, as `get_active_coach` reports it.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ActiveCoachDetail {
    /// Identifier.
    pub id: String,
    /// Display name.
    pub title: String,
    /// What it is for; absent when none was given.
    pub description: Option<String>,
    /// The instructions it runs on.
    pub system_prompt: String,
    /// Which shelf it sits on.
    pub category: String,
    /// Free-form labels.
    pub tags: Vec<String>,
    /// Estimated size of the system prompt, in tokens.
    pub token_count: u32,
}

/// What `get_active_coach` answers with.
///
/// One shape for both answers rather than two: the tool sends the same key set
/// whether a coach is active or not, so `active` false pairs with `coach`
/// absent. A client reads one field to branch instead of probing for a key.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetActiveCoachResult {
    /// Whether any coach is active for this athlete.
    pub active: bool,
    /// The active coach; absent when `active` is false.
    pub coach: Option<ActiveCoachDetail>,
}

/// What `hide_coach` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct HideCoachResult {
    /// The coach that was hidden.
    pub coach_id: String,
    /// Always true here; `show_coach` sends the same field as false.
    pub is_hidden: bool,
}

/// What `show_coach` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ShowCoachResult {
    /// The coach that was un-hidden.
    pub coach_id: String,
    /// Always false here; `hide_coach` sends the same field as true.
    pub is_hidden: bool,
    /// Whether a stored hide preference was actually removed. False when the
    /// coach was not hidden to begin with, which is a success.
    pub removed_preference: bool,
}

/// One coach as `list_hidden_coaches` reports it.
///
/// The narrowest projection of the four: enough to recognise a coach and
/// un-hide it, and nothing else.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct HiddenCoachEntry {
    /// Identifier `show_coach` takes.
    pub id: String,
    /// Display name.
    pub title: String,
    /// What it is for; absent when none was given.
    pub description: Option<String>,
    /// Which shelf it sits on.
    pub category: String,
    /// Whether it ships with the platform rather than being athlete-authored.
    pub is_system: bool,
}

/// What `list_hidden_coaches` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ListHiddenCoachesResult {
    /// The hidden coaches.
    pub coaches: Vec<HiddenCoachEntry>,
    /// How many there are.
    pub count: usize,
}

// ============================================================================
// Payload builders
// ============================================================================
//
// The projections themselves, kept here beside the types they build rather
// than in `coaches.rs`: each one is the answer to "what does this tool put on
// the wire", which is what this module is about.

/// Project the enriched list rows into the `list_coaches` answer.
///
/// `has_more` can only be decided when the caller set a limit — a full page is
/// the only evidence of a next one — so an unbounded call reports false.
#[must_use]
pub fn list_coaches_payload(
    coaches: &[CoachListItem],
    total: u32,
    offset: Option<u32>,
    limit: Option<u32>,
) -> ListCoachesResult {
    let entries: Vec<CoachListEntry> = coaches
        .iter()
        .map(|item| CoachListEntry {
            id: item.coach.id.to_string(),
            title: item.coach.title.clone(),
            description: item.coach.description.clone(),
            category: item.coach.category.as_str().to_owned(),
            tags: item.coach.tags.clone(),
            token_count: item.coach.token_count,
            is_favorite: item.is_favorite,
            is_system: item.coach.is_system,
            is_assigned: item.is_assigned,
            use_count: item.use_count,
            last_used_at: item.last_used_at.map(|dt| dt.to_rfc3339()),
            updated_at: item.coach.updated_at.to_rfc3339(),
        })
        .collect();
    let count = entries.len();
    ListCoachesResult {
        coaches: entries,
        count,
        total,
        offset: offset.unwrap_or(0),
        limit: limit.unwrap_or(DEFAULT_LIST_LIMIT),
        has_more: limit.is_some_and(|l| count as u64 == u64::from(l)),
    }
}

/// The page size `list_coaches` reports when the caller named none.
const DEFAULT_LIST_LIMIT: u32 = 50;

/// The page size `search_coaches` reports when the caller named none.
const DEFAULT_SEARCH_LIMIT: u32 = 20;

/// Project a freshly created coach into the `create_coach` answer.
#[must_use]
pub fn create_coach_payload(coach: &Coach) -> CreateCoachResult {
    CreateCoachResult {
        id: coach.id.to_string(),
        title: coach.title.clone(),
        description: coach.description.clone(),
        category: coach.category.as_str().to_owned(),
        tags: coach.tags.clone(),
        token_count: coach.token_count,
        created_at: coach.created_at.to_rfc3339(),
    }
}

/// Project a coach into the `get_coach` answer.
#[must_use]
pub fn get_coach_payload(coach: &Coach) -> GetCoachResult {
    GetCoachResult {
        id: coach.id.to_string(),
        title: coach.title.clone(),
        description: coach.description.clone(),
        system_prompt: coach.system_prompt.clone(),
        category: coach.category.as_str().to_owned(),
        tags: coach.tags.clone(),
        token_count: coach.token_count,
        created_at: coach.created_at.to_rfc3339(),
        updated_at: coach.updated_at.to_rfc3339(),
    }
}

/// Project an edited coach into the `update_coach` answer.
#[must_use]
pub fn update_coach_payload(coach: &Coach) -> UpdateCoachResult {
    UpdateCoachResult {
        id: coach.id.to_string(),
        title: coach.title.clone(),
        description: coach.description.clone(),
        system_prompt: coach.system_prompt.clone(),
        category: coach.category.as_str().to_owned(),
        tags: coach.tags.clone(),
        token_count: coach.token_count,
        updated_at: coach.updated_at.to_rfc3339(),
    }
}

/// Project search hits into the `search_coaches` answer.
#[must_use]
pub fn search_coaches_payload(
    query: &str,
    coaches: &[Coach],
    offset: Option<u32>,
    limit: Option<u32>,
) -> SearchCoachesResult {
    let results: Vec<CoachSearchEntry> = coaches
        .iter()
        .map(|c| CoachSearchEntry {
            id: c.id.to_string(),
            title: c.title.clone(),
            description: c.description.clone(),
            category: c.category.as_str().to_owned(),
            tags: c.tags.clone(),
            token_count: c.token_count,
        })
        .collect();
    let returned_count = results.len();
    let limit = limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
    SearchCoachesResult {
        query: query.to_owned(),
        results,
        returned_count,
        offset: offset.unwrap_or(0),
        limit,
        has_more: returned_count as u64 == u64::from(limit),
    }
}

/// Project the now-active coach into the `activate_coach` answer.
#[must_use]
pub fn activate_coach_payload(coach: &Coach) -> ActivateCoachResult {
    ActivateCoachResult {
        id: coach.id.to_string(),
        title: coach.title.clone(),
        description: coach.description.clone(),
        system_prompt: coach.system_prompt.clone(),
        category: coach.category.as_str().to_owned(),
        is_active: true,
        token_count: coach.token_count,
    }
}

/// Project the active coach, or its absence, into the `get_active_coach`
/// answer.
///
/// Takes the `Option` rather than being called only on the `Some` arm so that
/// both answers are built in one place and cannot drift apart.
#[must_use]
pub fn active_coach_payload(coach: Option<&Coach>) -> GetActiveCoachResult {
    GetActiveCoachResult {
        active: coach.is_some(),
        coach: coach.map(|c| ActiveCoachDetail {
            id: c.id.to_string(),
            title: c.title.clone(),
            description: c.description.clone(),
            system_prompt: c.system_prompt.clone(),
            category: c.category.as_str().to_owned(),
            tags: c.tags.clone(),
            token_count: c.token_count,
        }),
    }
}

/// Project the hidden coaches into the `list_hidden_coaches` answer.
#[must_use]
pub fn list_hidden_coaches_payload(coaches: &[Coach]) -> ListHiddenCoachesResult {
    let entries: Vec<HiddenCoachEntry> = coaches
        .iter()
        .map(|c| HiddenCoachEntry {
            id: c.id.to_string(),
            title: c.title.clone(),
            description: c.description.clone(),
            category: c.category.as_str().to_owned(),
            is_system: c.is_system,
        })
        .collect();
    let count = entries.len();
    ListHiddenCoachesResult {
        coaches: entries,
        count,
    }
}
