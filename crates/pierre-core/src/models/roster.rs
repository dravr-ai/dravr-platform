// ABOUTME: Coach-athlete roster assignment models (1:N junction)
// ABOUTME: Backs the coach_athlete_assignments table and /api/roster routes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Roster assignments
//!
//! A user with `manages_roster=true` (or `is_admin=true`) can be
//! assigned to coach another user inside the same tenant. The
//! relationship is many-to-many: an athlete can be coached by multiple
//! users, a coach can manage many athletes.
//!
//! Assignments soft-delete via `revoked_at` so the audit history (who
//! assigned, who revoked, when) survives the unassign action. Active
//! assignments are uniquely indexed on `(coach_user_id,
//! athlete_user_id, tenant_id) WHERE revoked_at IS NULL` so the same
//! coach cannot double-assign the same athlete.
//!
//! Persona's [`crate::models::CoachingPersona::Coach`] picks the output
//! voice; `manages_roster` picks the *permission*. They're independent —
//! see the architectural deep-dive in `Coaching Persona Architecture.md`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::TenantId;

/// A single coach → athlete assignment row.
///
/// Persistence: rows live in `coach_athlete_assignments`. `revoked_at`
/// is `NULL` for active assignments; once set, the row stays for audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachAthleteAssignment {
    /// Stable assignment identifier. Re-assignments after a revoke get
    /// a new id, preserving history.
    pub id: Uuid,
    /// User with `manages_roster=true` (or admin) doing the coaching.
    pub coach_user_id: Uuid,
    /// User being coached.
    pub athlete_user_id: Uuid,
    /// Tenant scope — both users must belong to this tenant. Cross-
    /// tenant assignments are refused at the route layer, never written.
    pub tenant_id: TenantId,
    /// Who initiated the assignment (admin granting, coach self-assigning,
    /// `None` for system-generated rows). Nullable on delete via FK so
    /// removing the assigner does not orphan the row.
    pub assigned_by: Option<Uuid>,
    /// When the assignment was created.
    pub assigned_at: DateTime<Utc>,
    /// When the assignment was revoked, or `None` while active.
    pub revoked_at: Option<DateTime<Utc>>,
    /// Who revoked the assignment, or `None` while active / system-revoked.
    pub revoked_by: Option<Uuid>,
}

impl CoachAthleteAssignment {
    /// Build a new active assignment with a fresh UUID and `assigned_at = now`.
    #[must_use]
    pub fn new(
        coach_user_id: Uuid,
        athlete_user_id: Uuid,
        tenant_id: TenantId,
        assigned_by: Option<Uuid>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            coach_user_id,
            athlete_user_id,
            tenant_id,
            assigned_by,
            assigned_at: Utc::now(),
            revoked_at: None,
            revoked_by: None,
        }
    }

    /// `true` while the assignment carries no `revoked_at`.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }
}
