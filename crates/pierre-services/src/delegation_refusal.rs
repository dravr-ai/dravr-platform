// ABOUTME: Why a delegated-connection step was refused — the reason a client branches on, worded for the coach platform
// ABOUTME: Each refusal carries details.reason and, when a platform is known, details.provider naming it as the user knows it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Delegated-connection refusals
//!
//! A linking step (roster, propose, confirm) that cannot go through answers
//! one of these. The wire form travels in the error's `details.reason`, which
//! is what a client branches on; `details.provider` names the coach platform
//! the refusal is about (`trainingpeaks`, `intervals_icu`) whenever one is
//! known, so a client can name it. The message is English and addressed to
//! an API caller.

use pierre_core::errors::{AppError, ErrorCode};
use serde_json::{json, Map, Value};

use crate::coach_platform::{CoachPlatform, COACH_PLATFORMS};
use crate::trainingpeaks_accounts::EmailBinding;

/// Why a linking step was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The coach has no connection of their own to a coaching platform here.
    NotConnected,
    /// The coach's platform account trains rather than coaches.
    NotCoachAccount,
    /// The platform shares no email for the coach's account.
    CoachEmailMissing,
    /// The coach's platform email is not their verified Dravr email.
    CoachEmailMismatch,
    /// The caller's own Dravr email is not verified, so it binds nothing.
    DravrEmailUnverified,
    /// The coach's platform credential is dead or flagged.
    ReconnectNeeded,
    /// The coach's platform credential is one the platform lists no
    /// athletes for (an Intervals.icu OAuth grant): the coach reconnects with
    /// their API key.
    ApiKeyRequired,
    /// The coach has not accepted the platform's current notice.
    TermsOutdated,
    /// A provider that is no coaching platform was named.
    UnsupportedProvider,
    /// The athlete id is not one the platform could have issued.
    InvalidAthlete,
    /// The coach's roster does not list the athlete.
    AthleteNotOnRoster,
    /// The platform shares no email for the athlete.
    AthleteEmailMissing,
    /// The athlete's platform email is not the member's verified Dravr email.
    AthleteEmailMismatch,
    /// The member's Dravr email is not verified, so it binds nothing.
    MemberEmailUnverified,
    /// The coach named themselves as the member.
    MemberIsCoach,
    /// The member already has a live link in this group.
    AlreadyProposed,
    /// The athlete is already linked, in this group or another the coach coaches.
    AthleteAlreadyLinked,
    /// The member reads the platform through a login of their own.
    OwnConnection,
}

impl Refusal {
    /// The reason a client branches on.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotConnected => "coach_platform_not_connected",
            Self::NotCoachAccount => "coach_platform_not_coach_account",
            Self::CoachEmailMissing => "coach_platform_email_missing",
            Self::CoachEmailMismatch => "coach_platform_email_mismatch",
            Self::DravrEmailUnverified => "dravr_email_unverified",
            Self::ReconnectNeeded => "coach_platform_reconnect_needed",
            Self::ApiKeyRequired => "coach_platform_api_key_required",
            Self::TermsOutdated => "coach_platform_terms_outdated",
            Self::UnsupportedProvider => "unsupported_provider",
            Self::InvalidAthlete => "invalid_athlete",
            Self::AthleteNotOnRoster => "athlete_not_on_roster",
            Self::AthleteEmailMissing => "athlete_email_missing",
            Self::AthleteEmailMismatch => "athlete_email_mismatch",
            Self::MemberEmailUnverified => "member_email_unverified",
            Self::MemberIsCoach => "member_is_coach",
            Self::AlreadyProposed => "already_proposed",
            Self::AthleteAlreadyLinked => "athlete_already_linked",
            Self::OwnConnection => "own_connection",
        }
    }

    /// The English message, naming the platform as `brand`.
    fn message(self, brand: &str) -> String {
        match self {
            Self::NotConnected => format!(
                "Connect the coaching platform your athletes are on ({}) first",
                platform_brands(" or ")
            ),
            Self::NotCoachAccount => format!("This {brand} account is not a coach account"),
            Self::CoachEmailMissing => format!(
                "{brand} shares no email for this account, so it cannot be matched to your \
                 Dravr account"
            ),
            Self::CoachEmailMismatch => {
                format!("This {brand} account's email is not your verified Dravr email")
            }
            Self::DravrEmailUnverified => "Verify your Dravr email first".to_owned(),
            Self::ReconnectNeeded => format!("Reconnect {brand} to read your roster"),
            Self::ApiKeyRequired => format!(
                "{brand} lists your athletes for an API key only: reconnect {brand} with your \
                 API key to read your roster"
            ),
            Self::TermsOutdated => {
                format!("Reconnect {brand} and accept the updated notice to read your roster")
            }
            Self::UnsupportedProvider => {
                format!("Only {} athletes can be linked", platform_brands(" and "))
            }
            Self::InvalidAthlete => format!("That is not a {brand} athlete id"),
            Self::AthleteNotOnRoster => format!("That athlete is not on your {brand} roster"),
            Self::AthleteEmailMissing => format!(
                "{brand} shares no email for this athlete, so the link cannot be matched to \
                 the member"
            ),
            Self::AthleteEmailMismatch => {
                format!("This athlete's {brand} email is not the member's verified Dravr email")
            }
            Self::MemberEmailUnverified => {
                "This member has not verified their Dravr email yet".to_owned()
            }
            Self::MemberIsCoach => "A coach cannot be linked as their own athlete".to_owned(),
            Self::AlreadyProposed => {
                format!("This member already has a {brand} link in this group")
            }
            Self::AthleteAlreadyLinked => format!("This {brand} athlete is already linked"),
            Self::OwnConnection => {
                format!("You already connect {brand} with your own account")
            }
        }
    }

    const fn code(self) -> ErrorCode {
        match self {
            Self::AlreadyProposed | Self::AthleteAlreadyLinked | Self::OwnConnection => {
                ErrorCode::ResourceAlreadyExists
            }
            _ => ErrorCode::InvalidInput,
        }
    }

    /// The refusal as an error, about `platform` when one is known.
    pub fn error(self, platform: Option<&dyn CoachPlatform>) -> AppError {
        let brand = platform.map_or("the coaching platform", |p| p.brand());
        let mut error = AppError::new(self.code(), self.message(brand));
        let mut details = Map::new();
        details.insert("reason".to_owned(), Value::from(self.as_str()));
        if let Some(platform) = platform {
            details.insert("provider".to_owned(), json!(platform.user_facing()));
        }
        error.details = Some(Box::new(Value::Object(details)));
        error
    }

    /// The refusal of a coach account that is not the coach's own.
    pub const fn coach_account(binding: EmailBinding) -> Option<Self> {
        match binding {
            EmailBinding::Bound => None,
            EmailBinding::ProviderEmailMissing => Some(Self::CoachEmailMissing),
            EmailBinding::DravrEmailUnverified => Some(Self::DravrEmailUnverified),
            EmailBinding::Mismatch => Some(Self::CoachEmailMismatch),
        }
    }

    /// The refusal of a roster athlete that is not the member, told to the
    /// coach who proposes, and why a confirmed link naming one reads nothing.
    pub const fn proposal(binding: EmailBinding) -> Option<Self> {
        match binding {
            EmailBinding::Bound => None,
            EmailBinding::ProviderEmailMissing => Some(Self::AthleteEmailMissing),
            EmailBinding::DravrEmailUnverified => Some(Self::MemberEmailUnverified),
            EmailBinding::Mismatch => Some(Self::AthleteEmailMismatch),
        }
    }

    /// The refusal of a roster athlete that is not the member, told to the
    /// member who confirms.
    pub const fn confirmation(binding: EmailBinding) -> Option<Self> {
        match binding {
            EmailBinding::Bound => None,
            EmailBinding::ProviderEmailMissing => Some(Self::AthleteEmailMissing),
            EmailBinding::DravrEmailUnverified => Some(Self::DravrEmailUnverified),
            EmailBinding::Mismatch => Some(Self::AthleteEmailMismatch),
        }
    }
}

/// Every coaching platform's brand, joined by `separator`.
fn platform_brands(separator: &str) -> String {
    COACH_PLATFORMS
        .iter()
        .map(|platform| platform.brand())
        .collect::<Vec<_>>()
        .join(separator)
}
