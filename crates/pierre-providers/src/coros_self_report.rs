// ABOUTME: COROS athlete self-report mapping — the feeling the athlete picked (1 very easy .. 5 weak) as a named Feel
// ABOUTME: A pure function the sciotte conversion calls so a COROS encoding never leaves this module
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # COROS athlete self-report
//!
//! COROS asks the athlete how an activity felt and stores the pick as
//! `sportFeelInfo.feelType`; dravr-sciotte carries it unconverted as the
//! activity's `feel` rank, and only when the athlete picked one (it sends
//! nothing for `0`). COROS records no rating of perceived exertion.
//!
//! The scale is the Training Hub's own. Its `feeling` module
//! (`static.coros.com/coros-traininghub-v2/assets/feeling-Y27Nf8Bj.js`, read
//! 2026-09-24) lists five faces by `type`: 1 `very_easy`, 2 `easy`, 3 `normal`,
//! 4 `tired`, 5 `weak`, and draws a "none" face for any other value. So 1 is
//! the best face and 5 the worst, and the five map in order onto the
//! platform's named scale.

use crate::models::Feel;

/// Map the COROS feeling the athlete picked onto the platform's named scale:
/// very easy is a strong day, weak a weak one. Any other rank is not one of
/// the Training Hub's five faces and maps to `None`.
#[must_use]
pub const fn feel_from_coros(rank: u8) -> Option<Feel> {
    match rank {
        1 => Some(Feel::Strong),
        2 => Some(Feel::Good),
        3 => Some(Feel::Normal),
        4 => Some(Feel::Poor),
        5 => Some(Feel::Weak),
        _ => None,
    }
}
