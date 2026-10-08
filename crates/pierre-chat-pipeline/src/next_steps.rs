// ABOUTME: Next steps — at most three controls a reply hands the athlete, so the move after it is a tap
// ABOUTME: The one way any reply attaches them: the live envelope's actions and the stored row's actions entry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Next steps (carnet#830).
//!
//! A guided walk, a command and a platform post can each end on what to do
//! next. Whatever picks the steps, they reach the athlete one way: as the
//! reply's actions — buttons where the surface draws them, lines of text
//! where it does not, which [`crate::envelope::build_envelope`] decides — and
//! as one `{"type":"actions"}` entry on the stored row, so a reload shows the
//! buttons the live turn did. [`NextSteps`] is that one way, so no reply
//! assembles either half by hand.
//!
//! A step starts something — a walk, a question — and never answers it: what
//! the athlete says once it has started is still theirs to say.

use serde_json::Value;

use crate::envelope::TurnAction;
use crate::stages::command_persistence::actions_block;

/// The most next steps one reply offers: one row of buttons on a phone, and
/// as many as `WhatsApp` shows as reply buttons before it folds them into a
/// list.
pub const MAX_NEXT_STEPS: usize = 3;

/// The next steps a reply offers, at most [`MAX_NEXT_STEPS`], in the order
/// they were chosen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NextSteps {
    actions: Vec<TurnAction>,
}

impl NextSteps {
    /// The first [`MAX_NEXT_STEPS`] of `steps`, in order. Whatever chose them
    /// ranked them, so the rest are dropped rather than crowding the row.
    #[must_use]
    pub fn new(steps: impl IntoIterator<Item = TurnAction>) -> Self {
        Self {
            actions: steps.into_iter().take(MAX_NEXT_STEPS).collect(),
        }
    }

    /// The steps as the live reply's controls — what
    /// [`crate::TurnState::actions`] carries into the envelope.
    #[must_use]
    pub fn into_actions(self) -> Vec<TurnAction> {
        self.actions
    }

    /// The row's `content_blocks` with these steps appended as one actions
    /// entry.
    ///
    /// `stored` is what the reply's other stages already wrote to the column —
    /// a chart's specs, a workout plan — or `None` for a reply with no other
    /// block. With no steps it comes back unchanged, so a reply that offers
    /// nothing keeps the `NULL` column of any other prose row.
    ///
    /// # Errors
    ///
    /// Returns the serialization error when `stored` is not a JSON array or
    /// the entry cannot be encoded. The caller then persists the reply without
    /// its steps rather than losing the reply.
    pub fn stored_blocks(&self, stored: Option<&str>) -> Result<Option<String>, serde_json::Error> {
        if self.actions.is_empty() {
            return Ok(stored.map(ToOwned::to_owned));
        }
        let mut entries: Vec<Value> = stored
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or_default();
        entries.push(serde_json::to_value(actions_block(None, &self.actions))?);
        serde_json::to_string(&entries).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use pierre_core::models::{PersistedAction, PersistedReplyBlock};
    use serde_json::json;

    use super::*;
    use crate::envelope::ActionKind;

    fn step(label: &str, value: &str) -> TurnAction {
        TurnAction {
            label: label.to_owned(),
            kind: ActionKind::Postback,
            value: value.to_owned(),
        }
    }

    fn persisted(label: &str, value: &str) -> PersistedAction {
        PersistedAction {
            label: label.to_owned(),
            action_type: "postback".to_owned(),
            value: value.to_owned(),
        }
    }

    #[test]
    fn a_reply_offers_its_first_three_steps_in_the_order_they_were_chosen() {
        let steps = NextSteps::new([
            step("Season", "/season"),
            step("Calibrate", "/calibrate"),
            step("Fortnight", "/fortnight"),
            step("Today", "/plan today"),
        ]);
        let values: Vec<String> = steps
            .into_actions()
            .into_iter()
            .map(|action| action.value)
            .collect();
        assert_eq!(values, ["/season", "/calibrate", "/fortnight"]);
    }

    #[test]
    fn a_reply_with_no_other_block_stores_one_actions_entry() {
        let stored = NextSteps::new([step("Season", "/season"), step("Today", "/plan today")])
            .stored_blocks(None)
            .unwrap()
            .expect("steps are stored");
        let blocks: Vec<PersistedReplyBlock> = serde_json::from_str(&stored).unwrap();
        assert_eq!(
            blocks,
            [PersistedReplyBlock::Actions {
                title: None,
                actions: vec![
                    persisted("Season", "/season"),
                    persisted("Today", "/plan today")
                ],
            }]
        );
    }

    #[test]
    fn steps_follow_the_blocks_the_reply_already_stored() {
        let chart = json!({ "type": "line", "title": "Load" });
        let stored = serde_json::to_string(&[&chart]).unwrap();
        let merged = NextSteps::new([step("Today", "/plan today")])
            .stored_blocks(Some(&stored))
            .unwrap()
            .expect("steps are stored");
        let entries: Vec<Value> = serde_json::from_str(&merged).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], chart, "the chart is kept, first");
        assert_eq!(
            serde_json::from_value::<PersistedReplyBlock>(entries[1].clone()).unwrap(),
            PersistedReplyBlock::Actions {
                title: None,
                actions: vec![persisted("Today", "/plan today")],
            }
        );
    }

    #[test]
    fn no_steps_leave_the_column_as_it_was() {
        let none = NextSteps::default();
        assert_eq!(none.stored_blocks(None).unwrap(), None);
        let chart = r#"[{"type":"line"}]"#;
        assert_eq!(
            none.stored_blocks(Some(chart)).unwrap().as_deref(),
            Some(chart)
        );
    }

    #[test]
    fn a_column_that_is_not_an_array_is_refused_rather_than_overwritten() {
        let steps = NextSteps::new([step("Season", "/season")]);
        assert!(steps.stored_blocks(Some(r#"{"type":"line"}"#)).is_err());
        assert!(steps.stored_blocks(Some("not json")).is_err());
    }
}
