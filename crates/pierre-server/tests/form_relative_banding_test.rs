// ABOUTME: Content tests for CTL-relative form banding — FormBand math, the elite fixture that
// ABOUTME: must not read as an emergency, group health flags, and descriptive tool descriptions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "tools-groups")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, NaiveDate, Utc};
use dravr_cageux::algorithms::training_load::DailyTrainingLoad;
use dravr_cageux::config::intelligence::AlgorithmConfig;
use dravr_cageux::training_load::{TrainingLoad, TrainingLoadCalculator};
use pierre_core::models::groups::{FlagEvidence, HealthFlagSeverity, MemberFlag};
use pierre_core::models::groups::{MemberFitnessSnapshot, OvertrainingRiskLevel};
use pierre_core::models::{Activity, SportType};
use pierre_core::models::{ActivityBuilder, FormBand, FormReading};
use pierre_fitness_compute::AthleteInputs;
use pierre_groups::strategies::summarization::{
    GroupSummarizationStrategy, RosterCardSummarizer, WeeklyDigestSummarizer,
};
use pierre_groups::GroupService;
use pierre_tool_runtime::implementations::analytics::analyze_detailed_training_load;
use pierre_tool_runtime::implementations::analytics::output::ProvidersUsed;
use std::collections::HashMap;
use uuid::Uuid;

/// A member whose CTL did not move overnight, so the CTL their form is read
/// against (yesterday's) is the CTL the card prints.
fn snapshot(ctl: f64, atl: f64, tsb: f64) -> MemberFitnessSnapshot {
    MemberFitnessSnapshot {
        user_id: Uuid::new_v4(),
        display_name: "Raph".to_owned(),
        ctl: Some(ctl),
        atl: Some(atl),
        tsb: Some(tsb),
        form_ctl: Some(ctl),
        weekly_volume_km: 120.0,
        previous_week_volume_km: None,
        weekly_activity_count: 5,
        weekly_duration_seconds: 18_000,
        primary_sport: Some("MountainBike".to_owned()),
        vdot: None,
        overtraining_risk: OvertrainingRiskLevel::Low,
        days_since_last_activity: Some(0),
        last_activity_per_provider: HashMap::new(),
        recent_activities: Vec::new(),
        needs_reauth_providers: Vec::new(),
        served_stale: false,
        timezone: None,
        computed_at: Utc::now(),
    }
}

/// One day of load whose CTL did not move since yesterday: `form_ctl` (the
/// CTL `tsb` is a share of) equals the end-of-day `ctl`.
fn steady_day(tsb: f64, form_ctl: f64) -> DailyTrainingLoad {
    DailyTrainingLoad {
        date: NaiveDate::from_ymd_opt(2026, 9, 27).unwrap(),
        ctl: form_ctl,
        atl: form_ctl - tsb,
        tsb,
        form_ctl,
    }
}

/// A training-load calculation whose CTL did not move since yesterday.
fn steady_load(tsb: f64, form_ctl: f64) -> TrainingLoad {
    TrainingLoad {
        ctl: form_ctl,
        atl: form_ctl - tsb,
        tsb,
        form_ctl,
        tss_history: Vec::new(),
    }
}

#[test]
fn form_pct_math_and_min_ctl_guard() {
    // The Raph incident numbers: TSB -66 on CTL 85 is -77.6% of fitness
    let pct = steady_day(-66.0, 85.0)
        .form_pct()
        .expect("CTL 85 is normalizable");
    assert!((pct - (-77.647)).abs() < 0.01, "got {pct}");

    // Elite block: -25 on CTL 100 is -25%, the deep end of the productive zone
    let elite = steady_day(-25.0, 100.0);
    let elite_pct = elite.form_pct().expect("CTL 100 is normalizable");
    assert!((elite_pct - (-25.0)).abs() < f64::EPSILON);
    assert_eq!(FormBand::from_daily_load(&elite), FormBand::HeavyBlock);

    // No chronic base → not interpretable, never banded on raw TSB
    let no_base = steady_day(-10.0, 0.5);
    assert!(no_base.form_pct().is_none());
    assert_eq!(
        FormBand::from_daily_load(&no_base),
        FormBand::InsufficientHistory
    );
}

/// Form is a share of yesterday's CTL, the one it was read from, never of
/// today's. A hard session today lifts today's CTL; dividing yesterday's TSB
/// by it would move the band on a number the session never touched.
#[test]
fn every_platform_reading_divides_form_by_yesterdays_ctl() {
    // Yesterday ended at CTL 60 / ATL 75: form today is -15, -25% of 60, a
    // heavy block. Today's big ride has since lifted CTL to 80.
    let day = DailyTrainingLoad {
        date: NaiveDate::from_ymd_opt(2026, 9, 27).unwrap(),
        ctl: 80.0,
        atl: 95.0,
        tsb: -15.0,
        form_ctl: 60.0,
    };
    let reading = FormReading::from_daily_load(&day);
    assert_eq!(reading.form_pct, Some(-25.0), "-15 is -25% of CTL 60");
    assert_eq!(reading.band, FormBand::HeavyBlock);
    assert!(
        reading.inline().contains("-25% of CTL"),
        "the prose quotes the same share: {}",
        reading.inline()
    );

    // The group snapshot reads the same pair: -15 over today's 80 would have
    // been -18.75%, productive, and raised no flag at all.
    let mut member = snapshot(80.0, 95.0, -15.0);
    member.form_ctl = Some(60.0);
    let flags = GroupService::compute_health_flags(&[member]);
    assert_eq!(
        flags
            .iter()
            .find(|f| f.flag_type == MemberFlag::Overreaching)
            .map(|f| f.evidence),
        Some(FlagEvidence::FormShare {
            form_pct: -25.0,
            tsb: -15.0
        }),
        "the heavy-block flag carries the share of yesterday's CTL: {flags:?}"
    );
}

#[test]
fn weekly_digest_card_renders_form_pct_next_to_tsb() {
    let card = WeeklyDigestSummarizer.summarize_member(&snapshot(85.0, 151.0, -66.0));
    assert!(
        card.summary_text.contains("TSB -66 (-78% of CTL"),
        "card should carry form % so the LLM reads TSB relative to the athlete: {}",
        card.summary_text
    );
}

#[test]
fn roster_card_renders_form_pct_next_to_tsb() {
    let card = RosterCardSummarizer.summarize_member(&snapshot(120.0, 150.0, -30.0));
    assert!(
        card.summary_text.contains("TSB -30 (-25% of CTL"),
        "roster card should carry form %: {}",
        card.summary_text
    );
}

/// The band's own wording travels with the number, on both cards.
///
/// The percentage alone was not enough. On 2026-09-02 the roster handed the
/// agent `TSB: -77` and a `[DEEP FATIGUE]` flag, and the agent supplied its own
/// reading — *"zone de surentraînement profond"*, a diagnosis — then anchored
/// fifteen turns of advice on it for an athlete who was deliberately peaking.
/// `FormBand::label` is written to be quotable: it describes fatigue relative to
/// fitness and never reaches for risk language (registre#199).
#[test]
fn both_cards_carry_the_bands_own_reading_not_just_the_number() {
    for text in [
        WeeklyDigestSummarizer
            .summarize_member(&snapshot(120.0, 197.0, -77.0))
            .summary_text,
        RosterCardSummarizer
            .summarize_member(&snapshot(120.0, 197.0, -77.0))
            .summary_text,
    ] {
        assert!(
            text.contains("deep fatigue - form far below this athlete's own fitness"),
            "the card must carry the band's reading so the model does not invent \
             one: {text}"
        );
        assert!(
            !text.contains("overtrain") && !text.contains("risk"),
            "the reading describes fatigue relative to fitness, never a \
             diagnosis: {text}"
        );
    }
}

#[test]
fn card_says_why_rather_than_shipping_a_bare_tsb_without_a_chronic_base() {
    let mut snap = snapshot(0.5, 20.0, -19.7);
    snap.ctl = Some(0.5);
    let card = WeeklyDigestSummarizer.summarize_member(&snap);
    assert!(
        !card.summary_text.contains("% of CTL"),
        "no form % without a chronic base to normalize against: {}",
        card.summary_text
    );
    assert!(
        card.summary_text
            .contains("no chronic base - form not interpretable"),
        "an un-normalizable TSB must say so; a bare absolute number is the \
         shape that gets read as a verdict: {}",
        card.summary_text
    );
}

// ============================================================================
// The elite fixture — the Raph case, in the shape the plan specified
// ============================================================================

/// 60 days at ~95 TSS/day with an empty physiological profile: no FTP, no
/// LTHR, no max/resting HR, no weight. This is the athlete whose absolute TSB
/// used to read as an emergency.
fn elite_block_activities() -> Vec<Activity> {
    let start = Utc::now() - Duration::days(60);
    (0..60)
        .map(|day| {
            ActivityBuilder::new(
                format!("elite_{day}"),
                "Endurance block session",
                SportType::Ride,
                start + Duration::days(day),
                // ~2h15 at threshold-ish effort lands near 95 TSS with the
                // duration-only estimator the empty profile forces.
                8_100,
                "test",
            )
            .distance_meters(60_000.0)
            .build()
        })
        .collect()
}

#[test]
fn elite_block_with_empty_profile_is_not_deep_fatigue() {
    let calculator = TrainingLoadCalculator::new(Utc::now().date_naive());
    let load = calculator
        .calculate_training_load(&elite_block_activities(), None, None, None, None, None)
        .expect("60 days of activities produce a training load");

    let band = FormBand::from_training_load(&load);
    assert_ne!(
        band,
        FormBand::DeepFatigue,
        "a steady 60-day block must not band as deepest fatigue (ctl {:.1}, atl {:.1}, tsb {:.1}, form {:?})",
        load.ctl,
        load.atl,
        load.tsb,
        load.form_pct()
    );
    assert_ne!(band, FormBand::InsufficientHistory, "60 days is a base");

    // The prescription is the part that alarmed the athlete: a consistent
    // block must not be told to rest.
    assert_eq!(
        TrainingLoadCalculator::recommend_recovery_days(&load),
        0,
        "steady block prescribed rest days (tsb {:.1}, form ctl {:.1})",
        load.tsb,
        load.form_ctl
    );
}

#[test]
fn deep_fatigue_is_the_only_band_that_prescribes_rest() {
    // Every band above the deep-fatigue edge is normal training or freshness,
    // so none of them may produce a rest prescription. This is the invariant
    // that keeps "critical fatigue - take rest days" off a productive block.
    for (tsb, form_ctl) in [
        (-25.0, 100.0), // heavy block
        (-15.0, 100.0), // productive
        (-5.0, 100.0),  // balanced
        (10.0, 100.0),  // fresh
        (25.0, 100.0),  // detraining
    ] {
        let load = steady_load(tsb, form_ctl);
        let band = FormBand::from_training_load(&load);
        assert_ne!(band, FormBand::DeepFatigue);
        assert_eq!(
            TrainingLoadCalculator::recommend_recovery_days(&load),
            0,
            "{band:?} must not prescribe rest"
        );
    }
    // And the band below it does.
    assert!(TrainingLoadCalculator::recommend_recovery_days(&steady_load(-45.0, 100.0)) > 0);
}

// ============================================================================
// Form is yesterday's balance (Coggan / TrainingPeaks, carnet#601)
// ============================================================================

/// A daily rider: exactly 100 TSS at 06:00 UTC on each of the 100 days before
/// today, plus today's ride when it has landed.
fn daily_rider(todays_ride_landed: bool) -> Vec<Activity> {
    let six_am = Utc::now()
        .date_naive()
        .and_hms_opt(6, 0, 0)
        .unwrap()
        .and_utc();
    let first = i64::from(!todays_ride_landed);
    (first..=100)
        .map(|days_ago| {
            ActivityBuilder::new(
                format!("daily_{days_ago}"),
                "Daily ride",
                SportType::Ride,
                six_am - Duration::days(days_ago),
                3_600,
                "test",
            )
            .training_stress_score(100.0)
            .build()
        })
        .collect()
}

/// What `analyze_training_load` tells the agent about `activities` right now.
fn load_payload(activities: &[Activity]) -> serde_json::Value {
    serde_json::to_value(analyze_detailed_training_load(
        activities,
        &AthleteInputs::default(),
        &AlgorithmConfig::default(),
        ProvidersUsed {
            activity_provider: "strava".to_owned(),
            sleep_provider: None,
        },
    ))
    .expect("the load payload serializes")
}

/// Asked at noon, before today's ride, the agent hears last night's form. The
/// worked example: yesterday ended at CTL 99.15 / ATL 100.00, so form is
/// -0.85, -0.86% of CTL, balanced. Today's own CTL and ATL, decayed through a
/// rest day that has not happened, read +19.5 (+20.7%): detraining.
#[test]
fn a_daily_rider_asked_at_noon_hears_last_nights_form() {
    let activities = daily_rider(false);
    let payload = load_payload(&activities);
    let metrics = &payload["load_metrics"];

    // Yesterday evening, from the same calculator the tool uses.
    let yesterday = Utc::now().date_naive() - Duration::days(1);
    let evening = TrainingLoadCalculator::new(yesterday)
        .calculate_training_load(&activities, None, None, None, None, None)
        .unwrap();
    assert_eq!(
        metrics["tsb"].as_f64(),
        Some((evening.ctl - evening.atl).round()),
        "form today is yesterday evening's balance: {payload}"
    );
    assert_eq!(metrics["form_ctl"].as_f64(), Some(evening.ctl.round()));

    // The worked example, rounded as the payload rounds it.
    assert_eq!(metrics["ctl"].as_f64(), Some(95.0), "{payload}");
    assert_eq!(metrics["atl"].as_f64(), Some(75.0), "{payload}");
    assert_eq!(metrics["form_ctl"].as_f64(), Some(99.0), "{payload}");
    assert_eq!(metrics["tsb"].as_f64(), Some(-1.0), "{payload}");
    assert_eq!(metrics["tsb_pct_of_ctl"].as_f64(), Some(-1.0), "{payload}");
    assert_eq!(payload["form_band"], "balanced", "{payload}");
}

/// The ride landing moves the day's fitness and fatigue, never its form: the
/// noon reading and the evening reading agree.
#[test]
fn todays_ride_moves_fitness_and_fatigue_but_not_the_agents_form_reading() {
    let noon = load_payload(&daily_rider(false));
    let evening = load_payload(&daily_rider(true));
    let (noon_m, evening_m) = (&noon["load_metrics"], &evening["load_metrics"]);

    assert_eq!(noon_m["ctl"].as_f64(), Some(95.0), "{noon}");
    assert_eq!(evening_m["ctl"].as_f64(), Some(99.0), "{evening}");
    assert_eq!(noon_m["atl"].as_f64(), Some(75.0), "{noon}");
    assert_eq!(evening_m["atl"].as_f64(), Some(100.0), "{evening}");

    for key in ["tsb", "form_ctl", "tsb_pct_of_ctl"] {
        assert_eq!(noon_m[key], evening_m[key], "{key} moved with today's ride");
    }
    assert_eq!(noon["form_band"], evening["form_band"]);
    assert_eq!(evening["form_band"], "balanced", "{evening}");
}

// ============================================================================
// Tool descriptions the model reads — descriptive, never injury risk
// ============================================================================

#[test]
fn training_tool_descriptions_carry_no_injury_risk_framing() {
    use pierre_mcp_server::tools::registry_builtin::get_tools;

    // Guards the compiled-in description. Note the scope: ToolRegistry::build_schema
    // replaces `description` wholesale with contremaitre's per-tool YAML when an
    // overlay exists, and that YAML is not compiled in — it arrives via the runtime
    // sync — so no Rust test can read what MCP `tools/list` actually serves. The
    // shipped catalogue is gated by scripts/ci/check-contremaitre-sync.sh (Tier 1b,
    // "retired ACWR/TSB framing"). This test keeps the fallback honest so a sync
    // failure degrades to safe wording rather than to the framing the literature
    // retired (Lolli 2017; Impellizzeri 2020).
    let banned = ["injury", "gabbett"];
    let training_tools = [
        "get_training_history",
        "compute_training_history",
        "analyze_training_load",
    ];

    let tools = get_tools();
    for name in training_tools {
        let tool = tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} is not registered"));
        let lowered = tool.description.to_lowercase();
        assert!(!lowered.is_empty(), "{name} has no description");
        for phrase in banned {
            assert!(
                !lowered.contains(phrase),
                "{name} description carries retired ACWR framing ({phrase}): {}",
                tool.description
            );
        }
    }
}

// ============================================================================
// Group health flags — the agent-facing surface, banded on the same edges
// ============================================================================

#[test]
fn health_flags_band_form_on_ctl_not_absolute_tsb() {
    // Two athletes, the same TSB -25. On a CTL-100 base that is -25% form (a
    // heavy block, a warning at most); on a CTL-50 base it is -50% (the
    // deepest fatigue band, critical). Absolute TSB could not tell them apart.
    let mut elite = snapshot(100.0, 125.0, -25.0);
    elite.display_name = "Elite".to_owned();
    let mut amateur = snapshot(50.0, 75.0, -25.0);
    amateur.display_name = "Amateur".to_owned();

    let flags = GroupService::compute_health_flags(&[elite, amateur]);

    let elite_flag = flags
        .iter()
        .find(|f| f.display_name == "Elite")
        .expect("elite gets the heavy-block warning");
    assert_eq!(elite_flag.flag_type, MemberFlag::Overreaching);
    assert_eq!(elite_flag.severity, HealthFlagSeverity::Warning);
    assert_eq!(
        elite_flag.evidence,
        FlagEvidence::FormShare {
            form_pct: -25.0,
            tsb: -25.0
        },
        "the flag carries form as a share of CTL, next to the TSB it was read from"
    );

    let amateur_flag = flags
        .iter()
        .find(|f| f.display_name == "Amateur")
        .expect("amateur is in the deepest fatigue band");
    assert_eq!(amateur_flag.flag_type, MemberFlag::DeepFatigue);
    assert_eq!(amateur_flag.severity, HealthFlagSeverity::Critical);
    assert_eq!(
        amateur_flag.evidence,
        FlagEvidence::FormShare {
            form_pct: -50.0,
            tsb: -25.0
        }
    );
}

#[test]
fn health_flags_stay_silent_through_the_productive_zone() {
    // -15% form is ordinary training. No flag reaches the agent, because a
    // normal block is not news.
    let flags = GroupService::compute_health_flags(&[snapshot(100.0, 115.0, -15.0)]);
    assert!(
        !flags.iter().any(|f| matches!(
            f.flag_type,
            MemberFlag::Overreaching | MemberFlag::DeepFatigue
        )),
        "productive form raised a form flag: {flags:?}"
    );
}

#[test]
fn health_flags_raise_no_form_flag_without_a_chronic_base() {
    // CTL 0.5 with TSB -19.7 is -3940% if you divide, and meaningless either
    // way. The band is InsufficientHistory and no form flag is produced.
    let flags = GroupService::compute_health_flags(&[snapshot(0.5, 20.2, -19.7)]);
    assert!(
        !flags.iter().any(|f| matches!(
            f.flag_type,
            MemberFlag::Overreaching | MemberFlag::DeepFatigue
        )),
        "form flag raised without a chronic base: {flags:?}"
    );
}

// ============================================================================
// The analyze_training_load payload — the shape the model actually reads
// ============================================================================

#[test]
fn training_load_payload_reports_form_pct_and_band() {
    // Serialized, because the subject of this test is the payload the model
    // reads rather than the struct behind it — the untagged enum puts the
    // analysed arm on the wire bare, so these are the keys an agent sees.
    // No period argument: the tool takes none. Its `days` was advertised and
    // never read, and its `timeframe` was read and never advertised — it
    // windowed nothing either way, and windowing a zero-seeded EMA's input
    // would understate the chronic load it exists to report (registre#415).
    let payload = serde_json::to_value(analyze_detailed_training_load(
        &elite_block_activities(),
        &AthleteInputs::default(),
        &AlgorithmConfig::default(),
        ProvidersUsed {
            activity_provider: "strava".to_owned(),
            sleep_provider: None,
        },
    ))
    .expect("the load payload serializes");

    let form_ctl = payload["load_metrics"]["form_ctl"]
        .as_f64()
        .expect("the CTL form is read against is reported");
    let tsb = payload["load_metrics"]["tsb"]
        .as_f64()
        .expect("tsb is reported");
    let pct = payload["load_metrics"]["tsb_pct_of_ctl"]
        .as_f64()
        .expect("form as % of CTL is reported next to the raw TSB");

    // The percentage is the raw TSB divided by this athlete's own fitness at
    // the end of yesterday — the CTL it was read from — not a second opinion.
    let expected = (tsb / form_ctl * 100.0).round();
    // Every figure in the payload is rounded, so compare with a rounding budget
    // rather than exactly.
    assert!(
        (pct - expected).abs() <= 2.0,
        "tsb_pct_of_ctl {pct} does not match tsb {tsb} over form_ctl {form_ctl}"
    );

    // Band and label come off that percentage, and a steady 60-day block is
    // training rather than an emergency.
    let band = payload["form_band"]
        .as_str()
        .expect("form_band is a string");
    assert_ne!(
        band, "deep_fatigue",
        "a steady 60-day block banded as deepest fatigue: {payload}"
    );
    assert_ne!(band, "insufficient_history", "60 days is a chronic base");

    let assessment = payload["form_assessment"]
        .as_str()
        .expect("form_assessment is a string");
    assert!(
        !assessment.is_empty(),
        "the band must carry a descriptive label"
    );

    // No narrated field may carry the retired framing. The `interpretation`
    // glossary is deliberately excluded: it is the block that *defines* the
    // fields, and says of form_band "it is not an injury prediction" — a
    // negation, not an assertion.
    let narrated = [
        payload["form_assessment"].to_string(),
        payload["taper_status"].to_string(),
        payload["recommendations"].to_string(),
        payload["periodization_suggestions"].to_string(),
    ]
    .join(" ")
    .to_lowercase();
    for phrase in ["injury", "gabbett", "critical fatigue", "overreaching zone"] {
        assert!(
            !narrated.contains(phrase),
            "a narrated field carries retired framing ({phrase}): {payload}"
        );
    }

    // The interpretation block must teach the reader to divide by CTL.
    let interpretation = payload["interpretation"]["tsb"]
        .as_str()
        .expect("tsb interpretation is present");
    assert!(
        interpretation.contains("tsb_pct_of_ctl"),
        "the tsb interpretation must point at the relative reading: {interpretation}"
    );
}
