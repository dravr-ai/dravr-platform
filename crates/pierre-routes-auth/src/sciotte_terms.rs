// ABOUTME: The precondition every provider sign-in passes: the required exposure notice, and the offered consents ticked
// ABOUTME: Shared by the credential login and the browser-extension sign-in, so neither sends credentials or cookies without it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The sign-in face of the provider notice precondition.

use pierre_core::errors::AppError;
use pierre_providers::sciotte_provider::SciotteTarget;
use pierre_services::provider_notice::{record_connect_consents, require_notice_accepted};
use uuid::Uuid;

use crate::AuthRoutesContext;

/// Refuse a login to a backend whose exposure notice is in force for this
/// account until it has accepted the current version, recording the
/// acceptance this attempt carries. A backend with no notice, or an account
/// the `provider_exposure_notice` flag leaves off, passes.
///
/// TrainingPeaks and COROS offer no API Pierre can use, so the session-read service reads
/// through the user's own signed-in session — access their terms prohibit for
/// third parties, putting the account at risk. Every connect surface (web,
/// mobile, the hosted page, the channel picker) states this before the
/// credentials field; this is where that statement becomes a precondition
/// rather than copy, so no surface can skip it. An account that already
/// accepted this notice version is not asked again, including on a reconnect
/// after a disconnect.
///
/// The consents a connect offers without requiring them (Garmin's AI use and
/// workout transfer) are recorded here too, once the precondition passed:
/// `consents` names the boxes the athlete ticked.
pub(crate) async fn require_provider_terms(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    target: SciotteTarget,
    accepted_now: bool,
    consents: &[String],
) -> Result<(), AppError> {
    require_notice_accepted(
        &resources.repos,
        tenant_id,
        user_id,
        target.provider_name(),
        target.brand(),
        accepted_now,
    )
    .await?;
    record_connect_consents(
        &resources.repos,
        tenant_id,
        user_id,
        target.provider_name(),
        consents,
    )
    .await
}
