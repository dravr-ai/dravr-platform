// ABOUTME: import_provider_thresholds — the athlete's FTP, threshold and max heart rate and threshold pace from Intervals.icu's sport settings
// ABOUTME: Fills a threshold only when the profile has none or holds the provider's earlier value; the athlete's own number is kept and reported

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Thresholds from a provider
//!
//! An athlete who keeps their FTP, threshold heart rate, maximum heart rate
//! and threshold pace in Intervals.icu can bring them into the profile every
//! personalised computation reads, instead of stating each one. The provider
//! is reached the way every read of the athlete is
//! ([`AuthService::create_authenticated_provider`](crate::protocol::auth::AuthService::create_authenticated_provider)):
//! their own connection, else the confirmed link through their human coach's
//! account. The tool writes the caller's own profile only, so an import is
//! always the athlete's own act.
//!
//! ## Precedence
//!
//! The rule is "never silently overwrite a value the athlete set":
//!
//! - an empty threshold is **imported**;
//! - a threshold the provider supplied earlier (its provenance names the
//!   provider) is **refreshed** to the provider's current value;
//! - a threshold the athlete set — no provenance, which is every value
//!   `set_physiology` writes and every value stored before provenance was
//!   recorded — is **kept**, and reported with both numbers so the athlete
//!   can choose; adopting the provider's is their call, through
//!   `set_physiology`.
//!
//! Each candidate value passes the merged-row validation `set_physiology`
//! applies, in the order FTP, max HR, threshold HR, threshold pace, so the
//! heart-rate pair is checked against whichever max HR stands. A value that
//! fails is skipped with the reason, never clamped. An imported value is an
//! estimate (kind `estimated`) whose origin is the provider and whose as-of
//! day is when the provider's setting last changed. Zones are derived by
//! Dravr from the thresholds; the provider's zone arrays are not read.
//!
//! W′ is never imported. Intervals.icu's W′ belongs to its FTP-anchored power
//! model, while the profile's `w_prime_joules` is the critical-power model's,
//! paired with `critical_power_watts`; the two are not interchangeable.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{
    MeasurementKind, MetricProvenance, PhysiologicalSex, ProviderThreshold, ProviderThresholds,
    SportType, TenantId, ThresholdProvenance, UserPhysiologicalProfile,
};
use pierre_fitness_compute::AthleteInputs;
use pierre_mcp_schema::{JsonSchema, ToolAnnotations};
use pierre_providers::backend_resolver::user_facing_name;
use pierre_providers::delegation::is_settings_not_shared;
use pierre_tools_core::ToolResult;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::context::ToolExecutionContext;
use crate::conversions::{answers_with, ok_typed, tool_definition, tool_result_to_response};
use crate::implementations::physiology::{
    profile_for_write, profile_payload, store_profile, validate_merged, PhysiologyProfile,
};
use crate::protocol::{auth_required_provider, UniversalExecutor};
use crate::runtime::ToolRuntime;
use crate::training_history_compute::HistoryRefresh;

/// The tool's name, as its errors and its result name it.
const TOOL: &str = "import_provider_thresholds";

/// Why W′ is never imported, as the result states it.
const W_PRIME_NOT_IMPORTED: &str = "Intervals.icu's W′ belongs to its FTP-anchored power model; \
     the profile's W′ is the critical-power model's, paired with critical power, and the two \
     are not interchangeable. Save a W′ from a critical-power test with set_physiology.";

/// The refusal of a delegated settings read the platform kept from the
/// coach's account: the link stays, and nothing was imported. It leads with
/// what keeps the link, names no cause the platform did not state, and says
/// what connecting an account of the athlete's own costs: it replaces the
/// link, the coach is not told, and the coach's calendar writes stop.
const SETTINGS_NOT_SHARED: &str = "Intervals.icu does not show your sport settings to the human \
     coach's account your link reads through, so nothing was imported. Tell me your thresholds \
     to save them, or ask your human coach to check how you share with them in Intervals.icu. \
     Connecting your own Intervals.icu account would also import them, but it replaces the link \
     through your human coach: they are not told, and they can no longer put workouts on your \
     calendar from Dravr.";

/// How one threshold fared in an import, in the unit its field name carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ThresholdImport<T> {
    /// The profile had none: the provider's value is now on it, as the
    /// provider's estimate.
    Imported {
        /// The value imported.
        value: T,
    },
    /// The profile held the provider's earlier value; it now holds the
    /// current one.
    Refreshed {
        /// The provider's earlier value.
        previous: T,
        /// The provider's current value, now on the profile.
        value: T,
    },
    /// The athlete set their own value, which stays. The provider holds
    /// another; adopting it is the athlete's call, saved with
    /// `set_physiology`.
    KeptYours {
        /// The athlete's value, unchanged.
        yours: T,
        /// The provider's value, not saved.
        provider: T,
    },
    /// The provider's value fails the range or consistency checks every
    /// saved value passes, so it was not saved.
    SkippedOutOfRange {
        /// The provider's value, not saved.
        value: T,
        /// Which check it failed.
        reason: String,
    },
    /// The profile already holds the provider's value.
    Unchanged {
        /// The value on the profile.
        value: T,
    },
    /// The provider holds no value for it.
    AbsentOnProvider,
}

impl<T> ThresholdImport<T> {
    /// The outcome's name, the only part the operator log carries: the
    /// values are health data.
    const fn label(&self) -> &'static str {
        match self {
            Self::Imported { .. } => "imported",
            Self::Refreshed { .. } => "refreshed",
            Self::KeptYours { .. } => "kept_yours",
            Self::SkippedOutOfRange { .. } => "skipped_out_of_range",
            Self::Unchanged { .. } => "unchanged",
            Self::AbsentOnProvider => "absent_on_provider",
        }
    }
}

/// What an import does with W′.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum WPrimeImport {
    /// Never imported, whatever the provider holds.
    NotImported {
        /// Why.
        reason: String,
    },
}

/// How each threshold fared in one import.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct ThresholdImports {
    /// Functional threshold power, in watts.
    pub ftp_watts: ThresholdImport<u32>,
    /// Lactate threshold heart rate, in bpm.
    pub threshold_hr: ThresholdImport<u16>,
    /// Maximum heart rate, in bpm.
    pub max_hr: ThresholdImport<u16>,
    /// Threshold pace, in seconds per kilometre.
    pub threshold_pace_sec_per_km: ThresholdImport<f64>,
    /// W′, which is never imported.
    pub w_prime_joules: WPrimeImport,
}

/// What `import_provider_thresholds` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ImportProviderThresholdsResult {
    /// The provider the thresholds were read from, by the name the athlete
    /// knows it.
    pub provider: String,
    /// Whether the profile was written: an import that only kept or skipped
    /// values, and recorded no sex, writes nothing.
    pub saved: bool,
    /// How each threshold fared.
    pub thresholds: ThresholdImports,
    /// Whether the athlete's physiological sex was recorded from the
    /// provider's athlete profile: only when the provider reports one and the
    /// profile held none. A sex the athlete stated is never replaced.
    pub sex_recorded: bool,
    /// The profile as it now stands, re-read from storage after a write;
    /// absent when the athlete has none and nothing was imported. Its
    /// `threshold_provenance` names every value a provider supplied.
    pub profile: Option<PhysiologyProfile>,
    /// What the import did to the stored daily training history: recomputed
    /// when it moved a number training load is scored against.
    pub training_history: HistoryRefresh,
}

/// One threshold's place on the profile: its value and its provenance.
type Slot<'a, T> = (&'a mut Option<T>, &'a mut Option<MetricProvenance>);

fn ftp_slot(profile: &mut UserPhysiologicalProfile) -> Slot<'_, u32> {
    (
        &mut profile.ftp_watts,
        &mut profile.threshold_provenance.ftp_watts,
    )
}

fn threshold_hr_slot(profile: &mut UserPhysiologicalProfile) -> Slot<'_, u16> {
    (
        &mut profile.threshold_hr,
        &mut profile.threshold_provenance.threshold_hr,
    )
}

fn max_hr_slot(profile: &mut UserPhysiologicalProfile) -> Slot<'_, u16> {
    (
        &mut profile.max_hr,
        &mut profile.threshold_provenance.max_hr,
    )
}

fn threshold_pace_slot(profile: &mut UserPhysiologicalProfile) -> Slot<'_, f64> {
    (
        &mut profile.threshold_pace_sec_per_km,
        &mut profile.threshold_provenance.threshold_pace_sec_per_km,
    )
}

/// Apply the precedence rule to one threshold the provider `offered`,
/// writing it into `profile` only when it is imported or refreshed and the
/// merged row still validates.
fn import_threshold<T: Copy + PartialEq>(
    profile: &mut UserPhysiologicalProfile,
    slot: fn(&mut UserPhysiologicalProfile) -> Slot<'_, T>,
    offered: Option<ProviderThreshold<T>>,
    origin: &str,
) -> ThresholdImport<T> {
    let Some(offered) = offered else {
        return ThresholdImport::AbsentOnProvider;
    };
    let provenance = MetricProvenance {
        kind: MeasurementKind::Estimated,
        origin: Some(origin.to_owned()),
        as_of: offered.as_of,
    };
    let (value, stored_provenance) = slot(profile);
    let previous = *value;
    let providers_own = stored_provenance
        .as_ref()
        .is_some_and(|stored| stored.origin.as_deref() == Some(origin));
    match previous {
        Some(current) if !providers_own => {
            return if current == offered.value {
                ThresholdImport::Unchanged { value: current }
            } else {
                ThresholdImport::KeptYours {
                    yours: current,
                    provider: offered.value,
                }
            };
        }
        // The provider still holds the value it supplied; only the day it
        // last changed the setting moves.
        Some(current) if current == offered.value => {
            *stored_provenance = Some(provenance);
            return ThresholdImport::Unchanged { value: current };
        }
        _ => {}
    }
    let previous_provenance = stored_provenance.replace(provenance);
    *value = Some(offered.value);
    if let Err(refused) = validate_merged(profile) {
        let (value, stored_provenance) = slot(profile);
        *value = previous;
        *stored_provenance = previous_provenance;
        return ThresholdImport::SkippedOutOfRange {
            value: offered.value,
            reason: refused.message,
        };
    }
    previous.map_or(
        ThresholdImport::Imported {
            value: offered.value,
        },
        |previous| ThresholdImport::Refreshed {
            previous,
            value: offered.value,
        },
    )
}

/// Merge what the provider `offered` into `profile` under the precedence
/// rule, max HR before threshold HR so the pair is checked against the max
/// HR that stands.
fn merge(profile: &mut UserPhysiologicalProfile, offered: &ProviderThresholds) -> ThresholdImports {
    let origin = offered.origin;
    let ftp_watts = import_threshold(profile, ftp_slot, offered.ftp_watts, origin);
    let max_hr = import_threshold(profile, max_hr_slot, offered.max_hr, origin);
    let threshold_hr = import_threshold(profile, threshold_hr_slot, offered.threshold_hr, origin);
    let threshold_pace_sec_per_km = import_threshold(
        profile,
        threshold_pace_slot,
        offered.threshold_pace_sec_per_km,
        origin,
    );
    ThresholdImports {
        ftp_watts,
        threshold_hr,
        max_hr,
        threshold_pace_sec_per_km,
        w_prime_joules: WPrimeImport::NotImported {
            reason: W_PRIME_NOT_IMPORTED.to_owned(),
        },
    }
}

/// The four thresholds and their provenance: what an import can change.
fn thresholds_of(
    profile: &UserPhysiologicalProfile,
) -> (
    Option<u32>,
    Option<u16>,
    Option<u16>,
    Option<f64>,
    ThresholdProvenance,
) {
    (
        profile.ftp_watts,
        profile.threshold_hr,
        profile.max_hr,
        profile.threshold_pace_sec_per_km,
        profile.threshold_provenance.clone(),
    )
}

/// How the provider read ended: the thresholds and the sex the athlete's
/// provider profile reports, or the answer the tool relays instead.
enum ProviderRead {
    Thresholds {
        offered: ProviderThresholds,
        reported_sex: Option<PhysiologicalSex>,
    },
    Refused(ToolResult),
}

/// Read the thresholds the athlete keeps on Intervals.icu, through their own
/// connection or their confirmed link.
///
/// A delegated read the platform kept from the coach's account is refused in
/// band and ends nothing; any other refusal of a delegated read reacts the
/// way every delegated read does (a dead coach key flags the coach).
///
/// # Errors
///
/// Returns the reconnect signal when the athlete's own session is dead.
async fn read_provider_thresholds(
    context: &ToolExecutionContext,
    tenant_id: TenantId,
    primary_sport: &SportType,
) -> AppResult<ProviderRead> {
    let brand = user_facing_name(INTERVALS_ICU);
    let executor = UniversalExecutor::new(context.resources.clone());
    let tenant = tenant_id.to_string();
    let attempt_started_at = Utc::now();
    let provider = match executor
        .auth_service
        .create_authenticated_provider(INTERVALS_ICU, context.user_id, Some(&tenant))
        .await
    {
        Ok(provider) => provider,
        Err(response) => {
            // A missing or dead session keeps its reconnect signal, so the
            // chat pipeline answers with the link rather than an apology.
            if let Some(dead) = auth_required_provider(&response) {
                return Err(AppError::provider_auth_required(dead));
            }
            let error = response
                .error
                .clone()
                .unwrap_or_else(|| format!("{TOOL} could not reach {brand}"));
            let answer = response
                .result
                .unwrap_or_else(|| json!({ "error": error, "provider": brand }));
            return Ok(ProviderRead::Refused(ToolResult::error(answer)));
        }
    };
    match provider.read_thresholds(Some(primary_sport)).await {
        Ok(offered) => {
            // The athlete profile is a second read; a failed one costs only
            // the sex, never the thresholds already in hand.
            let reported_sex = match provider.get_athlete().await {
                Ok(athlete) => athlete.sex,
                Err(e) => {
                    warn!(
                        user_id = %context.user_id,
                        provider = brand,
                        error = %e.internal_details(),
                        "threshold import: athlete profile read failed; no sex recorded"
                    );
                    None
                }
            };
            Ok(ProviderRead::Thresholds {
                offered,
                reported_sex,
            })
        }
        // Never a reaction: the athlete still shares their training, and the
        // link must survive the settings it does not cover.
        Err(e) if is_settings_not_shared(&e) => Ok(ProviderRead::Refused(ToolResult::error(
            json!({ "reason": "settings_not_shared", "error": SETTINGS_NOT_SHARED, "provider": brand }),
        ))),
        Err(e) => {
            executor
                .auth_service
                .react_to_delegated_refusal(
                    context.user_id,
                    &tenant,
                    provider.as_ref(),
                    &e,
                    attempt_started_at,
                )
                .await;
            if e.code == ErrorCode::ProviderAuthRequired {
                return Err(e);
            }
            warn!(
                user_id = %context.user_id,
                provider = brand,
                error = %e.internal_details(),
                "threshold import read failed"
            );
            Ok(ProviderRead::Refused(ToolResult::error(
                json!({ "error": e.sanitized_message(), "provider": brand }),
            )))
        }
    }
}

/// Imports the athlete's thresholds from their Intervals.icu sport settings.
#[derive(Debug)]
pub struct ImportProviderThresholdsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ImportProviderThresholdsTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(BTreeMap::new()),
            required: None,
            ..Default::default()
        };
        answers_with::<ImportProviderThresholdsResult>(tool_definition(
            "import_provider_thresholds",
            "Import the athlete's training thresholds — FTP, lactate threshold heart rate, max \
             heart rate and threshold pace — from their Intervals.icu sport settings, read \
             through their own Intervals.icu connection or the link through their human coach \
             they confirmed. A threshold is filled only when the profile has none, or refreshed \
             when it holds Intervals.icu's earlier value. A value the athlete set is never \
             replaced: it comes back as kept_yours with both numbers, and adopting \
             Intervals.icu's is the athlete's call, saved with set_physiology. A value outside \
             the ranges every saved value is checked against is skipped with the reason. W′ is \
             never imported: Intervals.icu's W′ belongs to its FTP-anchored model, not the \
             critical-power model the profile's W′ is. An imported value is Intervals.icu's \
             estimate: quote it with its origin ('intervals.icu has your FTP at 285 W'), never \
             as a measurement. Zones are re-derived and the stored training history \
             recomputed. The athlete's sex is recorded from their Intervals.icu profile only \
             when it reports one and the profile holds none (sex_recorded); a sex the athlete \
             stated is never replaced. Call it when the athlete asks to bring their thresholds over from \
             Intervals.icu; report only what the result contains.",
            schema,
            Some(ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                // A second import of unchanged settings writes nothing.
                idempotent_hint: Some(true),
                ..ToolAnnotations::default()
            }),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        // Writes `user_physiological_profiles` and answers with the profile
        // re-read from storage, as set_physiology does.
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::READS_DATA
            | ToolCapabilities::WRITES_DATA
            | ToolCapabilities::PROFILE
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> =
            async move {
                // No arguments: the import always reads the caller's own
                // provider into the caller's own profile.
                drop(args);
                let tenant_id = TenantId::from_uuid(context.require_tenant()?);
                let user_id = context.user_id;

                let existing = profile_for_write(&context, tenant_id, user_id).await?;
                let load_inputs_before = AthleteInputs::from_profile(existing.as_ref());
                let created = existing.is_none();
                // A first write has no stored sport to keep: `Run` is the
                // column's own default, as for set_physiology's first save.
                let mut profile = existing
                    .unwrap_or_else(|| UserPhysiologicalProfile::new(user_id, SportType::Run));
                profile.user_id = user_id;
                // The heart rates follow the sport of the profile they are
                // written into, the created one's default included, so a
                // later import of unchanged settings reads them from the same
                // settings object instead of trading one sport's pair for
                // another's.
                let (offered, reported_sex) =
                    match read_provider_thresholds(&context, tenant_id, &profile.primary_sport)
                        .await?
                    {
                        ProviderRead::Thresholds {
                            offered,
                            reported_sex,
                        } => (offered, reported_sex),
                        ProviderRead::Refused(answer) => return Ok(answer),
                    };

                let before = thresholds_of(&profile);
                let thresholds = merge(&mut profile, &offered);
                // The provider's sex only fills a gap; never inferred.
                let sex_recorded = profile.fill_reported_sex(reported_sex);
                let saved = thresholds_of(&profile) != before || sex_recorded;

                let (shown, training_history) = if saved {
                    let (stored, history) =
                        store_profile(&context, tenant_id, profile, &load_inputs_before).await?;
                    (Some(stored), history)
                } else {
                    ((!created).then_some(profile), HistoryRefresh::Unaffected)
                };

                info!(
                    user_id = %user_id,
                    tenant_id = %tenant_id,
                    saved = saved,
                    // Outcome names only — the values are health data.
                    ftp_watts = thresholds.ftp_watts.label(),
                    threshold_hr = thresholds.threshold_hr.label(),
                    max_hr = thresholds.max_hr.label(),
                    threshold_pace = thresholds.threshold_pace_sec_per_km.label(),
                    sex_recorded = sex_recorded,
                    "imported athlete thresholds from a provider"
                );

                ok_typed(
                    TOOL,
                    ImportProviderThresholdsResult {
                        provider: user_facing_name(INTERVALS_ICU).to_owned(),
                        saved,
                        thresholds,
                        sex_recorded,
                        profile: shown.as_ref().map(profile_payload),
                        training_history,
                    },
                )
            }
            .await;
        tool_result_to_response(result)
    }
}

// Guardian security classification (see `crate::security`). The write is the
// athlete's own profile, correctable with set_physiology; the provider answers
// numbers only, no third-party text; nothing is sent outbound beyond the read.
crate::declare_security!(ImportProviderThresholdsTool => empty);

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use uuid::Uuid;

    use super::*;

    const ORIGIN: &str = "intervals.icu";

    fn day(d: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(2026, 9, d)
    }

    fn offered<T>(value: T, d: u32) -> ProviderThreshold<T> {
        ProviderThreshold {
            value,
            as_of: day(d),
        }
    }

    fn intervals(
        ftp: Option<ProviderThreshold<u32>>,
        threshold_hr: Option<ProviderThreshold<u16>>,
        max_hr: Option<ProviderThreshold<u16>>,
        pace: Option<ProviderThreshold<f64>>,
    ) -> ProviderThresholds {
        ProviderThresholds {
            origin: ORIGIN,
            ftp_watts: ftp,
            threshold_hr,
            max_hr,
            threshold_pace_sec_per_km: pace,
        }
    }

    fn estimate(d: u32) -> MetricProvenance {
        MetricProvenance {
            kind: MeasurementKind::Estimated,
            origin: Some(ORIGIN.to_owned()),
            as_of: day(d),
        }
    }

    fn profile() -> UserPhysiologicalProfile {
        UserPhysiologicalProfile::new(Uuid::new_v4(), SportType::Ride)
    }

    #[test]
    fn an_empty_profile_takes_every_threshold_as_the_providers_estimate() {
        let mut p = profile();
        let outcomes = merge(
            &mut p,
            &intervals(
                Some(offered(285, 30)),
                Some(offered(168, 30)),
                Some(offered(186, 30)),
                Some(offered(250.0, 14)),
            ),
        );
        assert_eq!(outcomes.ftp_watts, ThresholdImport::Imported { value: 285 });
        assert_eq!(
            outcomes.threshold_hr,
            ThresholdImport::Imported { value: 168 }
        );
        assert_eq!(outcomes.max_hr, ThresholdImport::Imported { value: 186 });
        assert_eq!(
            outcomes.threshold_pace_sec_per_km,
            ThresholdImport::Imported { value: 250.0 }
        );
        assert_eq!(p.ftp_watts, Some(285));
        assert_eq!(p.threshold_provenance.ftp_watts, Some(estimate(30)));
        assert_eq!(
            p.threshold_provenance.threshold_pace_sec_per_km,
            Some(estimate(14))
        );
        assert!(matches!(
            outcomes.w_prime_joules,
            WPrimeImport::NotImported { ref reason } if reason.contains("not interchangeable")
        ));
        assert_eq!(p.w_prime_joules, None);
    }

    #[test]
    fn the_athletes_own_value_is_kept_and_reported_with_both_numbers() {
        let mut p = profile();
        p.ftp_watts = Some(270);
        p.max_hr = Some(186);
        let outcomes = merge(
            &mut p,
            &intervals(Some(offered(285, 30)), None, Some(offered(186, 30)), None),
        );
        assert_eq!(
            outcomes.ftp_watts,
            ThresholdImport::KeptYours {
                yours: 270,
                provider: 285
            }
        );
        assert_eq!(outcomes.max_hr, ThresholdImport::Unchanged { value: 186 });
        assert_eq!(p.ftp_watts, Some(270));
        // Agreeing with the provider does not make the value the provider's.
        assert!(p.threshold_provenance.is_empty());
        assert_eq!(outcomes.threshold_hr, ThresholdImport::AbsentOnProvider);
    }

    #[test]
    fn another_origins_value_is_never_the_importers_to_replace() {
        let mut p = profile();
        p.ftp_watts = Some(270);
        p.threshold_provenance.ftp_watts = Some(MetricProvenance {
            kind: MeasurementKind::Estimated,
            origin: Some("vekta".to_owned()),
            as_of: day(1),
        });
        let outcomes = merge(&mut p, &intervals(Some(offered(285, 30)), None, None, None));
        assert_eq!(
            outcomes.ftp_watts,
            ThresholdImport::KeptYours {
                yours: 270,
                provider: 285
            }
        );
        assert_eq!(
            p.threshold_provenance
                .ftp_watts
                .and_then(|provenance| provenance.origin),
            Some("vekta".to_owned())
        );
    }

    #[test]
    fn the_providers_earlier_value_is_refreshed_and_an_unchanged_one_keeps_its_value() {
        let mut p = profile();
        p.ftp_watts = Some(280);
        p.threshold_provenance.ftp_watts = Some(estimate(1));
        p.max_hr = Some(186);
        p.threshold_provenance.max_hr = Some(estimate(1));
        let outcomes = merge(
            &mut p,
            &intervals(Some(offered(285, 30)), None, Some(offered(186, 30)), None),
        );
        assert_eq!(
            outcomes.ftp_watts,
            ThresholdImport::Refreshed {
                previous: 280,
                value: 285
            }
        );
        assert_eq!(p.threshold_provenance.ftp_watts, Some(estimate(30)));
        assert_eq!(outcomes.max_hr, ThresholdImport::Unchanged { value: 186 });
        // The day the provider last changed the setting moves with it.
        assert_eq!(p.threshold_provenance.max_hr, Some(estimate(30)));
    }

    #[test]
    fn a_value_outside_the_ranges_is_skipped_with_the_reason_never_clamped() {
        let mut p = profile();
        let outcomes = merge(
            &mut p,
            &intervals(
                Some(offered(900, 30)),
                None,
                Some(offered(260, 30)),
                Some(offered(90.0, 30)),
            ),
        );
        let ThresholdImport::SkippedOutOfRange { value, ref reason } = outcomes.ftp_watts else {
            panic!("an FTP of 900 W is out of range: {:?}", outcomes.ftp_watts);
        };
        assert_eq!(value, 900);
        assert!(reason.contains("ftp"), "{reason}");
        assert!(matches!(
            outcomes.max_hr,
            ThresholdImport::SkippedOutOfRange { value: 260, .. }
        ));
        assert!(matches!(
            outcomes.threshold_pace_sec_per_km,
            ThresholdImport::SkippedOutOfRange { .. }
        ));
        assert_eq!(p.ftp_watts, None);
        assert_eq!(p.max_hr, None);
        assert_eq!(p.threshold_pace_sec_per_km, None);
        assert!(p.threshold_provenance.is_empty());
    }

    #[test]
    fn a_threshold_heart_rate_above_the_max_that_stands_is_skipped() {
        let mut p = profile();
        p.max_hr = Some(170);
        let outcomes = merge(
            &mut p,
            &intervals(None, Some(offered(175, 30)), Some(offered(192, 30)), None),
        );
        assert!(matches!(
            outcomes.max_hr,
            ThresholdImport::KeptYours {
                yours: 170,
                provider: 192
            }
        ));
        let ThresholdImport::SkippedOutOfRange { ref reason, .. } = outcomes.threshold_hr else {
            panic!(
                "an LTHR above the max HR must be skipped: {:?}",
                outcomes.threshold_hr
            );
        };
        assert!(reason.contains("max_hr"), "{reason}");
        assert_eq!(p.threshold_hr, None);
        assert!(p.threshold_provenance.threshold_hr.is_none());
    }

    #[test]
    fn the_log_names_each_outcome_as_the_result_does() {
        // The operator log carries the label alone, never the values; it must
        // be the outcome name the agent reads, so the two can be joined.
        for outcome in [
            ThresholdImport::Imported { value: 285_u32 },
            ThresholdImport::Refreshed {
                previous: 280,
                value: 285,
            },
            ThresholdImport::KeptYours {
                yours: 270,
                provider: 285,
            },
            ThresholdImport::SkippedOutOfRange {
                value: 900,
                reason: "ftp_watts out of range".to_owned(),
            },
            ThresholdImport::Unchanged { value: 285 },
            ThresholdImport::AbsentOnProvider,
        ] {
            let serialized = serde_json::to_value(&outcome).unwrap();
            assert_eq!(serialized["outcome"], outcome.label(), "{serialized}");
        }
    }

    #[test]
    fn an_outcome_serializes_flat_under_its_name() {
        let kept = serde_json::to_value(ThresholdImport::KeptYours {
            yours: 270_u32,
            provider: 285,
        })
        .unwrap();
        assert_eq!(
            kept,
            json!({ "outcome": "kept_yours", "yours": 270, "provider": 285 })
        );
        let absent = serde_json::to_value(ThresholdImport::<u16>::AbsentOnProvider).unwrap();
        assert_eq!(absent, json!({ "outcome": "absent_on_provider" }));
    }
}
