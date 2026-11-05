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
use super::policy::{
    ExternalSendAllowlist, GuardianMode, GuardianPolicy, PlanMode, TaintedDestructive,
};

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
/// Any change to the mode, to the tainted-destructive rule, to plan mode or to
/// the external-send allowlist, and setting the destructive budget to zero.
/// A move back toward the defaults asks too: it changes what athletes can do
/// just as much, and a lifted block is the edit a super admin most needs to see.
#[must_use]
pub fn high_impact_changes(
    stored: &GuardianConfigDocument,
    proposed: &GuardianConfigDocument,
) -> Vec<GuardianImpact> {
    let defaults = GuardianPolicy::default();
    let mode = |doc: &GuardianConfigDocument| doc.mode.unwrap_or(defaults.mode);
    let plan = |doc: &GuardianConfigDocument| doc.plan_mode.unwrap_or(defaults.plan_mode);
    let tainted = |doc: &GuardianConfigDocument| {
        doc.tainted_destructive
            .unwrap_or(defaults.tainted_destructive)
    };
    let destructive = |doc: &GuardianConfigDocument| {
        doc.max_destructive_per_turn
            .unwrap_or(defaults.max_destructive_per_turn)
    };
    let external = |doc: &GuardianConfigDocument| {
        doc.external_send
            .clone()
            .unwrap_or_else(|| defaults.external_send.clone())
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
    if tainted(stored) != tainted(proposed) {
        impacts.push(GuardianImpact {
            field: "tainted_destructive",
            from: tainted_label(tainted(stored)).to_owned(),
            to: tainted_label(tainted(proposed)).to_owned(),
            effect: tainted_effect(tainted(proposed)),
        });
    }
    if plan(stored) != plan(proposed) {
        impacts.push(GuardianImpact {
            field: "plan_mode",
            from: plan(stored).as_str().to_owned(),
            to: plan(proposed).as_str().to_owned(),
            effect: plan_effect(plan(proposed)),
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
    if external(stored) != external(proposed) {
        impacts.push(GuardianImpact {
            field: "external_send",
            from: external_label(&external(stored)),
            to: external_label(&external(proposed)),
            effect: "Changes which tenants' turns may reach tools that send outside the platform.",
        });
    }
    impacts
}

/// What the new mode does. `off` and `observe` behave the same at runtime, so
/// one sentence covers both.
const fn mode_effect(mode: GuardianMode) -> &'static str {
    match mode {
        GuardianMode::Enforce => {
            "The Guardian starts applying taint, budget and egress denials, on every turn, for \
             every athlete: calls that were only logged are now refused."
        }
        GuardianMode::Off | GuardianMode::Observe => {
            "The Guardian stops blocking anything: taint, budget and egress denials are logged \
             and not applied, on every turn, for every athlete."
        }
    }
}

const fn plan_effect(plan: PlanMode) -> &'static str {
    match plan {
        PlanMode::Enforce => {
            "A verified plan replaces the tool loop on every athlete's turn. No live model has \
             proven this path, and a plan the verifier refuses denies the whole turn."
        }
        PlanMode::Off => "Every turn goes back to the tool loop instead of a verified plan.",
    }
}

const fn tainted_effect(severity: TaintedDestructive) -> &'static str {
    match severity {
        TaintedDestructive::Confirm => {
            "An irreversible or outbound-writing tool (prescribe_workout among them) in a turn \
             that read untrusted content now waits for the athlete to answer /confirm or /deny \
             instead of running. Prefetched activities count as untrusted, so any turn grounded \
             on them is covered."
        }
        TaintedDestructive::Deny => {
            "An irreversible or outbound-writing tool (prescribe_workout among them) in a turn \
             that read untrusted content is now refused instead of running. Prefetched \
             activities count as untrusted, so any turn grounded on them is covered."
        }
        TaintedDestructive::Log => {
            "An irreversible or outbound-writing tool in a turn that read untrusted content \
             now runs with only a log line: nothing asks or refuses."
        }
    }
}

const fn tainted_label(severity: TaintedDestructive) -> &'static str {
    match severity {
        TaintedDestructive::Log => "log",
        TaintedDestructive::Confirm => "confirm",
        TaintedDestructive::Deny => "deny",
    }
}

fn external_label(allowlist: &ExternalSendAllowlist) -> String {
    match allowlist {
        ExternalSendAllowlist::None => "none".to_owned(),
        ExternalSendAllowlist::All => "all".to_owned(),
        ExternalSendAllowlist::Only(tenants) => format!("{} tenant(s)", tenants.len()),
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
            tainted_destructive: Some(TaintedDestructive::Deny),
            plan_mode: Some(PlanMode::Enforce),
            max_destructive_per_turn: Some(0),
            external_send: Some(ExternalSendAllowlist::All),
            ..GuardianConfigDocument::default()
        };
        assert_eq!(
            fields(&high_impact_changes(&empty, &every_edit)),
            [
                "mode",
                "tainted_destructive",
                "plan_mode",
                "max_destructive_per_turn",
                "external_send"
            ]
        );

        // A stored posture echoed back is no change to acknowledge, so a save
        // of an unrelated field does not ask again.
        assert_eq!(fields(&high_impact_changes(&every_edit, &every_edit)), NONE);

        // Non-zero budgets carry no acknowledgement.
        let budgets = GuardianConfigDocument {
            max_writes_per_turn: Some(5),
            max_destructive_per_turn: Some(3),
            ..GuardianConfigDocument::default()
        };
        assert_eq!(fields(&high_impact_changes(&empty, &budgets)), NONE);
        // A move back asks too, and only a raised destructive budget is exempt.
        assert_eq!(
            fields(&high_impact_changes(&every_edit, &empty)),
            ["mode", "tainted_destructive", "plan_mode", "external_send"],
        );

        // Between two non-default values is still a change in force.
        let confirm = GuardianConfigDocument {
            tainted_destructive: Some(TaintedDestructive::Confirm),
            ..GuardianConfigDocument::default()
        };
        let deny = GuardianConfigDocument {
            tainted_destructive: Some(TaintedDestructive::Deny),
            ..GuardianConfigDocument::default()
        };
        assert_eq!(
            fields(&high_impact_changes(&confirm, &deny)),
            ["tainted_destructive"]
        );
    }

    #[test]
    fn an_edit_matches_its_default_when_the_field_is_unset() {
        // Explicitly writing today's defaults changes nothing in force.
        let explicit = GuardianConfigDocument {
            mode: Some(GuardianMode::Enforce),
            tainted_destructive: Some(TaintedDestructive::Log),
            max_destructive_per_turn: Some(1),
            external_send: Some(ExternalSendAllowlist::None),
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
