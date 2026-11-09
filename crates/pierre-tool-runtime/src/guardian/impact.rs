// ABOUTME: Which edits to the Guardian document change what athletes can do, and what each one does
// ABOUTME: An admin save must acknowledge these before it lands; the list is what the super admin reads

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! High-impact Guardian edits.
//!
//! One save reaches every instance within a poll interval and every athlete's
//! next tool dispatch, so these edits do not land on a blind click. The
//! comparison is between the stored document and the proposed one, each over
//! the compiled-in defaults: env pins are left out, because an edit to a pinned
//! field persists and takes effect the moment the pin is lifted.

use serde::Serialize;

use super::config::GuardianConfigDocument;
use super::policy::{GuardianMode, GuardianPolicy};

/// One high-impact change a save would make.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuardianImpact {
    /// The document field.
    pub field: &'static str,
    /// The value in force before the save.
    pub from: String,
    /// The value in force after it.
    pub to: String,
    /// What the change does, in the super admin's terms.
    pub effect: &'static str,
}

/// The high-impact changes between `stored` and `proposed`, empty when none.
///
/// Any change to the mode, and setting the destructive budget to zero.
/// A move back toward the defaults asks too: it changes what athletes can do
/// just as much, and a lifted block is the edit a super admin most needs to see.
#[must_use]
pub fn high_impact_changes(
    stored: &GuardianConfigDocument,
    proposed: &GuardianConfigDocument,
) -> Vec<GuardianImpact> {
    let defaults = GuardianPolicy::default();
    let mode = |doc: &GuardianConfigDocument| doc.mode.unwrap_or(defaults.mode);
    let destructive = |doc: &GuardianConfigDocument| {
        doc.max_destructive_per_turn
            .unwrap_or(defaults.max_destructive_per_turn)
    };

    let mut impacts = Vec::new();
    if mode(stored) != mode(proposed) {
        impacts.push(GuardianImpact {
            field: "mode",
            from: mode(stored).as_str().to_owned(),
            to: mode(proposed).as_str().to_owned(),
            effect: mode_effect(mode(proposed)),
        });
    }
    if destructive(proposed) == 0 && destructive(stored) != 0 {
        impacts.push(GuardianImpact {
            field: "max_destructive_per_turn",
            from: destructive(stored).to_string(),
            to: "0".to_owned(),
            effect: "Every irreversible tool is denied on every turn, for every athlete, while \
                     the Guardian enforces.",
        });
    }
    impacts
}

/// What the new mode does.
const fn mode_effect(mode: GuardianMode) -> &'static str {
    match mode {
        GuardianMode::Enforce => {
            "The Guardian starts applying budget denials, on every turn, for every athlete: \
             calls that were only logged are now refused."
        }
        GuardianMode::Observe => {
            "The Guardian stops applying budgets: budget denials are logged and not applied, on \
             every turn, for every athlete. External send stays refused."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: [&str; 0] = [];

    fn fields(impacts: &[GuardianImpact]) -> Vec<&'static str> {
        impacts.iter().map(|impact| impact.field).collect()
    }

    #[test]
    fn each_high_impact_edit_is_named_and_nothing_else_is() {
        let empty = GuardianConfigDocument::default();
        assert_eq!(fields(&high_impact_changes(&empty, &empty)), NONE);

        let every_edit = GuardianConfigDocument {
            mode: Some(GuardianMode::Observe),
            max_destructive_per_turn: Some(0),
            ..GuardianConfigDocument::default()
        };
        assert_eq!(
            fields(&high_impact_changes(&empty, &every_edit)),
            ["mode", "max_destructive_per_turn"]
        );

        // A stored posture echoed back is no change to acknowledge, so a save
        // of an unrelated field does not ask again.
        assert_eq!(fields(&high_impact_changes(&every_edit, &every_edit)), NONE);

        // Non-zero budgets carry no acknowledgement.
        let budgets = GuardianConfigDocument {
            max_writes_per_turn: Some(5),
            max_destructive_per_turn: Some(5),
            ..GuardianConfigDocument::default()
        };
        assert_eq!(fields(&high_impact_changes(&empty, &budgets)), NONE);
        // A move back asks too, and only a raised destructive budget is exempt.
        assert_eq!(fields(&high_impact_changes(&every_edit, &empty)), ["mode"]);
    }

    #[test]
    fn an_edit_matches_its_default_when_the_field_is_unset() {
        // Explicitly writing today's defaults changes nothing in force.
        let explicit = GuardianConfigDocument {
            mode: Some(GuardianMode::Enforce),
            max_destructive_per_turn: Some(GuardianPolicy::default().max_destructive_per_turn),
            ..GuardianConfigDocument::default()
        };
        assert_eq!(
            fields(&high_impact_changes(
                &GuardianConfigDocument::default(),
                &explicit
            )),
            NONE
        );
    }
}
