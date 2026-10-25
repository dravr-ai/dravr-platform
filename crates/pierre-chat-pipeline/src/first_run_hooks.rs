// ABOUTME: Where a chat turn meets the first-run done rules — the athlete's turn, read once its reply landed
// ABOUTME: Holds the grounded rule chat.answer_delivered reports, so the funnel event and Flow 2 read the same answer
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The turn side of the first run (carnet#1099).
//!
//! [`TurnFacts`] is taken from the turn's input before the pipeline consumes
//! it; once a reply was posted, [`TurnFacts::after_turn`] hands the athlete's
//! turn to [`pierre_services::first_run::after_athlete_turn`], which marks the
//! flows it completes. A turn the platform composed — a check-in, the first
//! post after a sync — completes nothing: the athlete did not speak.

use pierre_core::models::{TenantId, TurnOrigin};
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::RepositoryRegistry;
use pierre_services::first_run::{after_athlete_turn, AthleteTurn};
use uuid::Uuid;

use crate::envelope::{TurnEnvelope, TurnTelemetry};
use crate::surface_profile::SurfaceProfile;
use crate::turn::TurnInput;

/// Whether a reply was grounded in the athlete's own data: activities the
/// model fetched, activities prefetched for it, or a tool call that served
/// their provider items. The one rule `chat.answer_delivered` reports and
/// the first run's analysis flow is done by.
pub(crate) const fn grounded(telemetry: &TurnTelemetry, provider_data_served: bool) -> bool {
    telemetry.activity_list_captured || telemetry.activities_prefetched || provider_data_served
}

/// What the first run needs from a turn's input, kept before the turn runs.
pub(crate) struct TurnFacts {
    /// The athlete, when they spoke and their id parses.
    athlete: Option<Uuid>,
    /// The athlete's own tenant.
    tenant: TenantId,
}

impl TurnFacts {
    /// The facts of `input`: an athlete only for a turn they sent.
    pub(crate) fn of(input: &TurnInput) -> Self {
        Self {
            athlete: (input.origin == TurnOrigin::Athlete)
                .then(|| parse_uuid(&input.user_id).ok())
                .flatten(),
            tenant: input.tool_tenant_id,
        }
    }

    /// Mark the first-run flows the athlete's posted turn completes.
    pub(crate) async fn after_turn(
        &self,
        repos: &RepositoryRegistry,
        envelope: &TurnEnvelope,
        profile: &SurfaceProfile,
        provider_data_served: bool,
    ) {
        let Some(user) = self.athlete else {
            return;
        };
        after_athlete_turn(
            repos,
            AthleteTurn {
                user,
                tenant: self.tenant,
                personal_thread: envelope.conversation.personal,
                grounded: grounded(&envelope.telemetry, provider_data_served),
                surface: profile.surface.as_str(),
            },
        )
        .await;
    }
}
