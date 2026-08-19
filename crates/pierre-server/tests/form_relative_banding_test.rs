// ABOUTME: Content tests for CTL-relative form banding — FormBand math, the elite fixture that
// ABOUTME: must not read as an emergency, group health flags, and descriptive tool descriptions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "tools-groups")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, Utc};
use pierre_core::models::groups::{HealthFlagSeverity, MemberFlag};
use pierre_core::models::groups::{MemberFitnessSnapshot, OvertrainingRiskLevel};
use pierre_core::models::{Activity, SportType};
use pierre_core::models::{ActivityBuilder, FormBand};
use pierre_groups::strategies::summarization::{
    GroupSummarizationStrategy, RosterCardSummarizer, WeeklyDigestSummarizer,
};
use pierre_groups::GroupService;
use pierre_intelligence::TrainingLoadCalculator;
use pierre_tool_runtime::implementations::analytics::{
    analyze_detailed_training_load, UserPhysiologicalParams,
};
use std::collections::HashMap;
use uuid::Uuid;

fn snapshot(ctl: f64, atl: f64, tsb: f64) -> MemberFitnessSnapshot {
    MemberFitnessSnapshot {
        user_id: Uuid::new_v4(),
        display_name: "Raph".to_owned(),
        ctl: Some(ctl),
        atl: Some(atl),
        tsb: Some(tsb),
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
        computed_at: Utc::now(),
    }
}

#[test]
fn form_pct_math_and_min_ctl_guard() {
    // The Raph incident numbers: TSB -66 on CTL 85 is -77.6% of fitness
    let pct = FormBand::form_pct(-66.0, 85.0).expect("CTL 85 is normalizable");
    assert!((pct - (-77.647)).abs() < 0.01, "got {pct}");

    // Elite block: -25 on CTL 100 is -25%, the deep end of the productive zone
    let elite = FormBand::form_pct(-25.0, 100.0).expect("CTL 100 is normalizable");
    assert!((elite - (-25.0)).abs() < f64::EPSILON);
    assert_eq!(FormBand::from_tsb(-25.0, 100.0), FormBand::HeavyBlock);

    // No chronic base → not interpretable, never banded on raw TSB
    assert!(FormBand::form_pct(-10.0, 0.5).is_none());
    assert_eq!(
        FormBand::from_tsb(-10.0, 0.5),
        FormBand::InsufficientHistory
    );
}

#[test]
fn weekly_digest_card_renders_form_pct_next_to_tsb() {
    let card = WeeklyDigestSummarizer.summarize_member(&snapshot(85.0, 151.0, -66.0));
    assert!(
        card.summary_text.contains("TSB: -66 (-78% of CTL)"),
        "card should carry form % so the LLM reads TSB relative to the athlete: {}",
        card.summary_text
    );
}

#[test]
fn roster_card_renders_form_pct_next_to_tsb() {
    let card = RosterCardSummarizer.summarize_member(&snapshot(120.0, 150.0, -30.0));
    assert!(
        card.summary_text.contains("TSB -30 (-25% of CTL)"),
        "roster card should carry form %: {}",
        card.summary_text
    );
}

#[test]
fn card_omits_form_pct_without_chronic_base() {
    let mut snap = snapshot(0.5, 20.0, -19.7);
    snap.ctl = Some(0.5);
    let card = WeeklyDigestSummarizer.summarize_member(&snap);
    assert!(
        card.summary_text.contains("TSB: -20") && !card.summary_text.contains("% of CTL"),
        "no form % without a chronic base to normalize against: {}",
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
    let calculator = TrainingLoadCalculator::new();
    let load = calculator
        .calculate_training_load(&elite_block_activities(), None, None, None, None, None)
        .expect("60 days of activities produce a training load");

    let band = FormBand::from_tsb(load.tsb, load.ctl);
    assert_ne!(
        band,
        FormBand::DeepFatigue,
        "a steady 60-day block must not band as deepest fatigue (ctl {:.1}, atl {:.1}, tsb {:.1}, form {:?})",
        load.ctl,
        load.atl,
        load.tsb,
        FormBand::form_pct(load.tsb, load.ctl)
    );
    assert_ne!(band, FormBand::InsufficientHistory, "60 days is a base");

    // The prescription is the part that alarmed the athlete: a consistent
    // block must not be told to rest.
    assert_eq!(
        TrainingLoadCalculator::recommend_recovery_days(load.tsb, load.ctl),
        0,
        "steady block prescribed rest days (tsb {:.1}, ctl {:.1})",
        load.tsb,
        load.ctl
    );
}

#[test]
fn deep_fatigue_is_the_only_band_that_prescribes_rest() {
    // Every band above the deep-fatigue edge is normal training or freshness,
    // so none of them may produce a rest prescription. This is the invariant
    // that keeps "critical fatigue - take rest days" off a productive block.
    for (tsb, ctl) in [
        (-25.0, 100.0), // heavy block
        (-15.0, 100.0), // productive
        (-5.0, 100.0),  // balanced
        (10.0, 100.0),  // fresh
        (25.0, 100.0),  // detraining
    ] {
        let band = FormBand::from_tsb(tsb, ctl);
        assert_ne!(band, FormBand::DeepFatigue);
        assert_eq!(
            TrainingLoadCalculator::recommend_recovery_days(tsb, ctl),
            0,
            "{band:?} must not prescribe rest"
        );
    }
    // And the band below it does.
    assert!(TrainingLoadCalculator::recommend_recovery_days(-45.0, 100.0) > 0);
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
// Group health flags — the coach-facing surface, banded on the same edges
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
    assert!(
        elite_flag.detail.contains("-25% of fitness"),
        "detail should quote form, got: {}",
        elite_flag.detail
    );

    let amateur_flag = flags
        .iter()
        .find(|f| f.display_name == "Amateur")
        .expect("amateur is in the deepest fatigue band");
    assert_eq!(amateur_flag.flag_type, MemberFlag::DeepFatigue);
    assert_eq!(amateur_flag.severity, HealthFlagSeverity::Critical);
    assert!(amateur_flag.detail.contains("-50% of fitness"));
}

#[test]
fn health_flags_stay_silent_through_the_productive_zone() {
    // -15% form is ordinary training. No flag reaches the coach, because a
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
    let payload = analyze_detailed_training_load(
        &elite_block_activities(),
        "month",
        &UserPhysiologicalParams {
            ftp: None,
            lthr: None,
            max_hr: None,
            resting_hr: None,
            weight_kg: None,
        },
        &pierre_intelligence::AlgorithmConfig::default(),
    );

    let ctl = payload["load_metrics"]["ctl"]
        .as_f64()
        .expect("ctl is reported");
    let tsb = payload["load_metrics"]["tsb"]
        .as_f64()
        .expect("tsb is reported");
    let pct = payload["load_metrics"]["tsb_pct_of_ctl"]
        .as_f64()
        .expect("form as % of CTL is reported next to the raw TSB");

    // The percentage is the raw TSB divided by this athlete's own fitness, not
    // a second opinion on it.
    let expected = (tsb / ctl * 100.0).round();
    // Every figure in the payload is rounded, so compare with a rounding budget
    // rather than exactly.
    assert!(
        (pct - expected).abs() <= 2.0,
        "tsb_pct_of_ctl {pct} does not match tsb {tsb} over ctl {ctl}"
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
