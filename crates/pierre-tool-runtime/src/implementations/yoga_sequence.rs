// ABOUTME: Fills a yoga sequence to the length asked from a purpose's poses, and says so when they fall short
// ABOUTME: A pose too long for what is left is passed over, never the end; the guidance states the real length

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The sequence `suggest_yoga_sequence` answers with
//!
//! The poses come from the purpose's catalogue in order; each is used once.
//! The fill used to stop at the first pose that did not fit, so a long hold
//! early in the list (Legs Up the Wall, 300 s) ended an evening sequence at 7
//! of 10 minutes with three short poses unused. It now passes such a pose
//! over and keeps filling. When the catalogue still runs short (the six
//! `post_cardio` poses hold 300 s in all), the guidance says how long the
//! sequence really runs and how to fill the rest, rather than calling a
//! 5-minute sequence the 10 minutes asked.

use pierre_core::models::mobility::{YogaPose, YogaPoseType};

/// The poses a sequence of `target_seconds` holds, in order: a closing
/// relaxation pose (the last, when it is one) that fits has its hold kept
/// back, so a short session still ends on it, and the poses before it fill
/// what is left in order, each one that no longer fits passed over.
pub(crate) fn fill_sequence(poses: &[YogaPose], target_seconds: u32) -> Vec<&YogaPose> {
    let holds: Vec<(u32, bool)> = poses
        .iter()
        .map(|pose| {
            (
                pose.hold_duration_seconds,
                matches!(pose.pose_type, YogaPoseType::Relaxation),
            )
        })
        .collect();
    fill(&holds, target_seconds)
        .into_iter()
        .filter_map(|index| poses.get(index))
        .collect()
}

/// [`fill_sequence`] over `(hold seconds, is relaxation)` pairs: the indices
/// picked, in order.
fn fill(holds: &[(u32, bool)], target_seconds: u32) -> Vec<usize> {
    let closing = holds
        .len()
        .checked_sub(1)
        .filter(|&last| holds[last].1 && holds[last].0 <= target_seconds);
    let body = &holds[..closing.unwrap_or(holds.len())];
    let mut left = target_seconds - closing.map_or(0, |last| holds[last].0);
    let mut picked = Vec::new();
    for (index, (hold, _)) in body.iter().enumerate() {
        if *hold > left {
            continue;
        }
        left -= hold;
        picked.push(index);
    }
    picked.extend(closing);
    picked
}

/// How to approach the sequence, stating its real length: when it runs
/// short of the `asked_minutes`, how far short and how to fill the rest.
pub(crate) fn guidance(
    purpose: &str,
    focus_clause: &str,
    asked_minutes: u32,
    total_seconds: u32,
) -> String {
    const APPROACH: &str = "Take your time with each pose and listen to your body.";
    let purpose = purpose.replace('_', " ");
    if total_seconds == 0 {
        return format!(
            "No {purpose} pose{focus_clause} fits within the {asked_minutes} minutes asked."
        );
    }
    if total_seconds >= asked_minutes.saturating_mul(60) {
        return format!(
            "This {asked_minutes}-minute yoga sequence is designed for {purpose}{focus_clause}. \
             {APPROACH}"
        );
    }
    let actual = minutes(total_seconds);
    format!(
        "This {actual}-minute yoga sequence is designed for {purpose}{focus_clause}. Its poses \
         fill {actual} of the {asked_minutes} minutes asked: repeat the sequence or hold each \
         pose longer to fill the rest. {APPROACH}"
    )
}

/// `seconds` as minutes: whole when they divide, else to one decimal.
fn minutes(seconds: u32) -> String {
    if seconds.is_multiple_of(60) {
        (seconds / 60).to_string()
    } else {
        format!("{:.1}", f64::from(seconds) / 60.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELAX: bool = true;
    const ACTIVE: bool = false;

    /// The seeded `evening` poses in catalogue order: Butterfly, Seated
    /// Forward Fold, Legs Up the Wall, Reclined Pigeon, Happy Baby, Supine
    /// Spinal Twist, then Corpse to close.
    const EVENING: [(u32, bool); 7] = [
        (60, ACTIVE),
        (60, ACTIVE),
        (300, ACTIVE),
        (60, ACTIVE),
        (60, ACTIVE),
        (60, ACTIVE),
        (300, RELAX),
    ];

    #[test]
    fn a_pose_too_long_for_what_is_left_is_passed_over_not_the_end() {
        let picked = fill(&EVENING, 600);
        let total: u32 = picked.iter().map(|&i| EVENING[i].0).sum();
        assert_eq!(total, 600, "bug: the fill stopped at Legs Up: {picked:?}");
        assert_eq!(picked.last(), Some(&6), "it still closes on Corpse");
    }

    #[test]
    fn a_short_catalogue_is_said_with_its_real_length() {
        // The six `post_cardio` poses hold 300 s in all.
        let said = guidance("post_cardio", "", 10, 300);
        assert!(said.starts_with("This 5-minute yoga sequence"), "{said}");
        assert!(said.contains("5 of the 10 minutes asked"), "{said}");
    }

    #[test]
    fn a_full_sequence_is_said_as_asked() {
        let said = guidance("evening", ", focused on the hips", 10, 600);
        assert!(
            said.starts_with("This 10-minute yoga sequence is designed for evening, focused"),
            "{said}"
        );
        assert!(!said.contains("asked"), "{said}");
        assert_eq!(minutes(390), "6.5", "bug: 390 s is 6.5 minutes");
    }
}
