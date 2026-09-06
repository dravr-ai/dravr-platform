// ABOUTME: The shapes the admin coach tools answer with, and the schemas derived from them
// ABOUTME: Split from admin.rs so the answer contracts read as one unit, as with coaches_output
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Result types for the eight admin coach tools.
//!
//! These are the operator-facing twins of the athlete-facing coach tools, and
//! the shapes differ in one telling way: every projection here carries
//! `visibility`, because deciding what a system coach is visible to is the
//! operator's job and nobody else's.

use serde::Serialize;

/// One system coach as `admin_list_system_coaches` reports it.
///
/// No `system_prompt`: a listing of coaches is for choosing one, and the
/// prompt is long. `admin_get_system_coach` is what returns it.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SystemCoachEntry {
    /// Identifier the other admin tools take.
    pub id: String,
    /// Display name.
    pub title: String,
    /// What the coach is for; absent when none was given.
    pub description: Option<String>,
    /// Which shelf it sits on.
    pub category: String,
    /// Free-form labels for filtering and search.
    pub tags: Vec<String>,
    /// Estimated size of the system prompt, in tokens.
    pub token_count: u32,
    /// Who can see it — the operator's decision, which is why every admin
    /// projection carries it and no athlete-facing one does.
    pub visibility: String,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 timestamp of the last edit.
    pub updated_at: String,
}

/// What `admin_list_system_coaches` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AdminListSystemCoachesResult {
    /// The coaches on this page.
    pub coaches: Vec<SystemCoachEntry>,
    /// How many came back on this page.
    pub count: usize,
    /// How many the tenant has in total, ignoring paging.
    pub total: usize,
    /// The paging offset these start at.
    pub offset: usize,
}

/// What `admin_create_system_coach` answers with.
///
/// No `system_prompt` and no `updated_at`: the caller just sent the prompt,
/// and a coach created this instant has no edit to report.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AdminCreateSystemCoachResult {
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
    /// Who can see it.
    pub visibility: String,
    /// Always true: this tool only creates system coaches.
    pub is_system: bool,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
}

/// What `admin_get_system_coach` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AdminGetSystemCoachResult {
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
    /// Who can see it.
    pub visibility: String,
    /// Whether it ships with the platform.
    pub is_system: bool,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 timestamp of the last edit.
    pub updated_at: String,
}

/// What `admin_update_system_coach` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AdminUpdateSystemCoachResult {
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
    /// Who can see it.
    pub visibility: String,
    /// Whether it ships with the platform.
    pub is_system: bool,
    /// RFC 3339 timestamp of this edit.
    pub updated_at: String,
}

/// What `admin_delete_system_coach` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AdminDeleteSystemCoachResult {
    /// Always true: the tool errors rather than reporting a failed delete.
    pub deleted: bool,
    /// The coach that was removed, echoed back.
    pub coach_id: String,
}

/// What `admin_assign_coach` answers with.
///
/// Echoes both the operator and the athlete, so an audit reader has the whole
/// act from the reply alone.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AdminAssignCoachResult {
    /// Always true: the tool errors rather than reporting a failed assign.
    pub assigned: bool,
    /// The coach that was assigned.
    pub coach_id: String,
    /// Its display name, so the reply is readable without a second call.
    pub coach_title: String,
    /// The athlete it was assigned to.
    pub user_id: String,
    /// The operator who assigned it.
    pub assigned_by: String,
}

/// What `admin_unassign_coach` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AdminUnassignCoachResult {
    /// Always true: an assignment that was not there is an error, not a
    /// quiet success, because the operator asked to remove something.
    pub unassigned: bool,
    /// The coach that was unassigned.
    pub coach_id: String,
    /// The athlete it was removed from.
    pub user_id: String,
}

/// One assignment as `admin_list_coach_assignments` reports it.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CoachAssignmentEntry {
    /// The athlete the coach is assigned to.
    pub user_id: String,
    /// Their email, so the operator can recognise them. Absent when the
    /// join could not resolve the account — a deleted user leaves the
    /// assignment row behind.
    pub user_email: Option<String>,
    /// RFC 3339 timestamp of the assignment.
    pub assigned_at: String,
    /// The operator who made it. Absent for assignments made before the
    /// column existed, and for any the system made on nobody's behalf.
    pub assigned_by: Option<String>,
}

/// What `admin_list_coach_assignments` answers with.
///
/// The listing is capped: a popular system coach in a large tenant can carry
/// an assignment per athlete. `truncated` states that in the payload rather
/// than letting the operator read a short list as a complete one.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AdminListCoachAssignmentsResult {
    /// The coach whose assignments these are.
    pub coach_id: String,
    /// The assignments, up to the cap.
    pub assignments: Vec<CoachAssignmentEntry>,
    /// How many are in this reply.
    pub count: usize,
    /// How many exist, which is larger than `count` when truncated.
    pub total: usize,
    /// Whether the cap was hit and rows were left out.
    pub truncated: bool,
}
