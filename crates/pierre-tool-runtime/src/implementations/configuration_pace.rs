// ABOUTME: The VDOT calculate_personalized_zones cuts its pace zones from, and those zones rendered for the payload
// ABOUTME: A VDOT from this call, a fresh stated one, a recent run or the threshold pace; never from a VO2max
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_cageux::algorithms::PREDICTABLE_VDOT;
use dravr_cageux::config::intelligence::VO2MaxCalculator;
use dravr_cageux::performance_prediction::VdotSource;
use dravr_tronc::dates::Timestamp;
use pierre_config::environment::TrainingZonesConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, UserPhysiologicalProfile};
use pierre_services::athlete_vdot::{resolve_athlete_vdot, resolve_athlete_vdot_from_cache};
use serde_json::{json, Value};
use tracing::warn;

use crate::context::ToolExecutionContext;
use crate::implementations::configuration_output::{PaceZone, PaceZones};

/// Render the VDOT pace zones for a tool payload.
///
/// The zones are cageux's [`VO2MaxCalculator::calculate_pace_zones`], the one
/// pace-zone function every surface quotes: each edge is the pace whose
/// oxygen cost on Daniels' curve is a share of the VDOT, or a predicted race
/// pace (marathon, mile), and `config` carries the shares. `Err` names why
/// the VDOT cuts no zones.
pub(super) fn calculate_pace_zones_from_vdot(
    vdot: f64,
    lactate_threshold: f64,
    config: &TrainingZonesConfig,
) -> Result<PaceZones, String> {
    // Pace zones read only the VDOT and the lactate threshold; the heart-rate
    // fields of the calculator feed nothing asked for here.
    let paces = VO2MaxCalculator::new(vdot, 0, 0, lactate_threshold)
        .calculate_pace_zones(config)
        .map_err(|e| format!("no pace zones for VDOT {vdot:.1}: {e}"))?;

    let format_pace = |seconds_per_km: f64| -> String {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let total_secs = if !seconds_per_km.is_finite() || seconds_per_km < 0.0 {
            0_u32
        } else if seconds_per_km >= 4_294_967_295.0 {
            u32::MAX
        } else {
            seconds_per_km.round() as u32
        };

        let minutes = total_secs / 60;
        let seconds = total_secs % 60;
        format!("{minutes}:{seconds:02}")
    };

    // cageux orders each range by its own convention; a zone quotes its
    // fastest pace (fewest seconds per kilometre) first.
    let band = |(a, b): (f64, f64)| PaceZone {
        min_pace: format_pace(a.min(b)),
        max_pace: format_pace(a.max(b)),
    };
    Ok(PaceZones {
        zone_1_easy: band(paces.easy_pace_range),
        zone_2_marathon: band(paces.marathon_pace_range),
        zone_3_threshold: band(paces.threshold_pace_range),
        zone_4_vo2max: band(paces.vo2max_pace_range),
        zone_5_neuromuscular_max_pace: format_pace(paces.neuromuscular_pace_max),
    })
}

/// How `input_sources` names where a resolved VDOT came from.
const fn vdot_source_label(source: &VdotSource) -> &'static str {
    match source {
        VdotSource::Stated { .. } => "stated",
        VdotSource::RecentRun { .. } => "recent_run_estimate",
        VdotSource::ThresholdPace { .. } => "threshold_pace_estimate",
    }
}

/// The VDOT the pace zones are cut from: this call's `vdot`, else the
/// athlete's one VDOT ([`resolve_athlete_vdot`]) from their saved profile and
/// the runs sync cached, recorded in `sources`. Never a `VO2max`.
///
/// # Errors
///
/// Invalid input when `vdot` is outside the 20-90 cageux reads.
pub(super) async fn resolve_zone_vdot(
    ctx: &ToolExecutionContext,
    args: &Value,
    stored: Option<&UserPhysiologicalProfile>,
    sources: &mut serde_json::Map<String, Value>,
) -> AppResult<Option<f64>> {
    if let Some(vdot) = args.get("vdot").and_then(Value::as_f64) {
        if !PREDICTABLE_VDOT.contains(&vdot) {
            return Err(AppError::invalid_input(format!(
                "vdot must be between {} and {}, got {vdot}",
                PREDICTABLE_VDOT.start(),
                PREDICTABLE_VDOT.end()
            )));
        }
        sources.insert("vdot".to_owned(), json!("provided"));
        return Ok(Some(vdot));
    }
    let now = Timestamp::now();
    let config = &ctx.resources.cageux_config().algorithms;
    // The runs sync already stored: a database read, never a provider call.
    // An unreadable cache costs the recent-run estimate, not the call.
    let reading = match ctx.tenant_id {
        Some(tenant) => resolve_athlete_vdot_from_cache(
            ctx.resources.repos(),
            ctx.resources.provider_registry().as_ref(),
            TenantId::from_uuid(tenant),
            ctx.user_id,
            stored,
            now,
            config,
        )
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "calculate_personalized_zones: cached runs unreadable");
            resolve_athlete_vdot(stored, &[], now, config)
        }),
        None => resolve_athlete_vdot(stored, &[], now, config),
    };
    if let Some(reading) = &reading {
        sources.insert("vdot".to_owned(), json!(vdot_source_label(&reading.source)));
    }
    Ok(reading.map(|r| r.vdot))
}
