// ABOUTME: Promotes one athlete's resting heart rate, VO2max and weight anchors from their own measurements (DECISION 5)
// ABOUTME: Saved through set_physiology's one writer, so zones are recut and stored training load recomputed when it moves

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One athlete's anchor promotion: which anchors may move and from which
//! measurement is [`pierre_services::anchor_promotion`]'s rule; this module
//! reads under the right gates and saves what moved.
//!
//! The measurements are read as a model's read for a platform job: the
//! athlete's AI consents apply, and a first-party-only provider's rows mark
//! the read of the anchor they were read for. The profile is stamped with the
//! marks of the anchors that moved, as a profile the athlete saves in a turn
//! that served such data is (carnet#769), so a promoted number never reaches
//! a surface the rows it came from may not. The training-history recompute
//! runs outside that scope, like every other recompute.
//!
//! The save goes through [`store_profile`], the one writer `set_physiology`
//! uses: it recuts the zones from the new anchors and recomputes the stored
//! training history only when the numbers load is scored against moved. That
//! per-athlete comparison is the recompute a promotion owes; the precedence
//! generation (`PRECEDENCE_GENERATION`) versions the merge rule for every
//! athlete at once, and no rule changes here.

use std::sync::Arc;

use pierre_core::errors::AppResult;
use pierre_core::models::{SportType, TenantId, UserPhysiologicalProfile};
use pierre_core::transport::Transport;
use pierre_fitness_compute::AthleteInputs;
use pierre_providers::ai_scope;
use pierre_services::anchor_promotion::{
    apply_promotion, promotable_anchors, promotion_measurements, Promotion,
};
use pierre_services::stored_health::Anchors;
use tracing::warn;
use uuid::Uuid;

use crate::implementations::physiology::{store_profile, validate_merged};
use crate::runtime::ToolRuntime;
use crate::training_history_compute::HistoryRefresh;

/// What one athlete's promotion did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorPromotion {
    /// No promotable anchor had a measurement to take, or every one already
    /// held it: nothing was written.
    Unchanged,
    /// The promoted profile contradicts itself (a resting heart rate at or
    /// above the stored maximum): nothing was written, and the reason is
    /// logged without the measurements.
    Refused,
    /// The anchors were saved; what the save did to the stored history.
    Promoted(HistoryRefresh),
}

/// Promote the athlete's anchors in `tenant` from their measurements.
///
/// An athlete with no profile row gets one when a measurement fills an
/// anchor, as `set_physiology`'s first save creates one.
///
/// # Errors
///
/// Returns the repository error when the profile or the measurements cannot
/// be read, or the promoted profile cannot be written.
pub async fn promote_anchors(
    resources: &Arc<dyn ToolRuntime>,
    tenant: TenantId,
    user_id: Uuid,
) -> AppResult<AnchorPromotion> {
    let existing = resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant, user_id)
        .await?;
    let wanted = promotable_anchors(existing.as_ref());
    if wanted == Anchors::default() {
        return Ok(AnchorPromotion::Unchanged);
    }
    let promotion = read_promotion(resources, tenant, user_id, wanted).await?;
    save_promotion(resources, tenant, user_id, &promotion).await
}

/// The athlete's measurement for each anchor `wanted` names, read as a
/// model's read for a platform job: the athlete's AI consents apply, and each
/// anchor carries the transport stamp of its own rows.
///
/// # Errors
///
/// Returns the repository error when the measurements cannot be read.
pub async fn read_promotion(
    resources: &Arc<dyn ToolRuntime>,
    tenant: TenantId,
    user_id: Uuid,
    wanted: Anchors,
) -> AppResult<Promotion> {
    Box::pin(ai_scope::serve_over(
        Transport::PlatformJob,
        ai_scope::for_model(promotion_measurements(
            resources.repos(),
            resources.provider_registry().as_ref(),
            tenant,
            user_id,
            wanted,
        )),
    ))
    .await
}

/// Save `promotion` into the athlete's profile as it stands now.
///
/// The profile is read again here, after the measurements, so a
/// `set_physiology` that committed while they were read stands: the rule is
/// applied to the athlete's latest numbers, never to a copy read before them.
/// The profile is stamped with the transport policy of the anchors that moved
/// only.
///
/// # Errors
///
/// Returns the repository error when the profile cannot be read or written.
pub async fn save_promotion(
    resources: &Arc<dyn ToolRuntime>,
    tenant: TenantId,
    user_id: Uuid,
    promotion: &Promotion,
) -> AppResult<AnchorPromotion> {
    if promotion.is_empty() {
        return Ok(AnchorPromotion::Unchanged);
    }
    let current = resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant, user_id)
        .await?;
    let load_inputs_before = AthleteInputs::from_profile(current.as_ref());
    // A first save has no stored sport to keep: `Run`, the column's default,
    // as for set_physiology's first save and the import's.
    let mut profile =
        current.unwrap_or_else(|| UserPhysiologicalProfile::new(user_id, SportType::Run));
    let moved = apply_promotion(&mut profile, promotion);
    if moved == Anchors::default() {
        return Ok(AnchorPromotion::Unchanged);
    }
    if let Err(e) = validate_merged(&profile) {
        warn!(
            user_id = %user_id,
            tenant_id = %tenant,
            reason = %e.sanitized_message(),
            "promoted anchors contradict the stored profile; nothing saved"
        );
        return Ok(AnchorPromotion::Refused);
    }
    profile.transport_policy = profile
        .transport_policy
        .strictest(promotion.policy_of(moved));
    let (_, history) = store_profile(resources, tenant, profile, &load_inputs_before).await?;
    Ok(AnchorPromotion::Promoted(history))
}
