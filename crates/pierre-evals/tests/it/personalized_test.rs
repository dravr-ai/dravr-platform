// ABOUTME: Tests for the personalized-physiology layer — personalized physiology checks + pluggable tolerance/action strategies
// ABOUTME: Proves the layer fires Supported/Contradicted against the athlete's own ranges and stays silent on thin data
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_evals::personalized::check as personalized_check;
use pierre_evals::ClaimSource;
use pierre_evals::{
    check_claim, claim_extractor::ExtractedClaim, evidence_retriever::EvidenceCorpus, ActionMode,
    AgentConfiguredStrategy, AthleteMetrics, AuditOnlyPolicy, ConservativeStrategy,
    ContradictionPolicy, InheritConfigPolicy, PersonalizedContext, ResolvedAction, TightStrategy,
    ToleranceCall, ToleranceMode, ToleranceStrategy, UserWarnPolicy, VerdictOutcome,
    VerificationConfig, VerificationFallback,
};
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};

const TEST_CORPUS: &str = r#"
{"id":"doi:10.1/a","category":"nutrition","proposition":"Protein intake of 1.6 to 2.2 g per kg body weight per day maximizes muscle protein synthesis","strength":"strong","citation":"Morton 2018"}
"#;

fn corpus() -> EvidenceCorpus {
    EvidenceCorpus::from_jsonl(TEST_CORPUS).expect("parse test corpus")
}

fn claim(text: &str, category: ClaimCategory) -> ExtractedClaim {
    ExtractedClaim {
        text: text.to_owned(),
        category,
        source: ClaimSource::Reply,
    }
}

/// A snapshot with enough history (`data_days >= MIN_DATA_DAYS`) and a populated
/// set of metrics so the personalized layer is allowed to fire. Threshold pace 5:00–5:15/km,
/// VDOT 52.
fn usable_metrics() -> AthleteMetrics {
    AthleteMetrics {
        vdot: Some(52.0),
        vo2max: Some(52.0),
        easy_pace_range: Some((360.0, 400.0)),
        threshold_pace_range: Some((300.0, 315.0)),
        max_hr: Some(190.0),
        ftp_watts: Some(250.0),
        data_days: 30,
        ..AthleteMetrics::default()
    }
}

fn verdict(status: ClaimStatus, layer: VerdictLayer) -> VerdictOutcome {
    VerdictOutcome {
        status,
        evidence_strength: EvidenceStrength::Strong,
        confidence: 0.9,
        layer_fired: layer,
        explanation: String::new(),
        evidence_refs: None,
    }
}

fn cfg_with_fallback(fallback: VerificationFallback) -> VerificationConfig {
    VerificationConfig {
        fallback_behavior: fallback,
        ..VerificationConfig::default()
    }
}

// ---------------------------------------------------------------------------
// ToleranceStrategy assess() semantics
// ---------------------------------------------------------------------------

#[test]
fn conservative_supports_inside_range() {
    let s = ConservativeStrategy::default();
    assert_eq!(s.assess(305.0, (300.0, 315.0)), ToleranceCall::Supported);
}

#[test]
fn conservative_indeterminate_inside_buffer() {
    // 285 is below the [300,315] range but within the 8% buffer (276..340.2).
    let s = ConservativeStrategy::default();
    assert_eq!(
        s.assess(285.0, (300.0, 315.0)),
        ToleranceCall::Indeterminate
    );
}

#[test]
fn conservative_contradicts_far_outside() {
    let s = ConservativeStrategy::default();
    assert_eq!(s.assess(200.0, (300.0, 315.0)), ToleranceCall::Contradicted);
}

#[test]
fn tight_contradicts_any_value_outside_range() {
    let s = TightStrategy;
    // One second outside the range, no buffer → contradicted.
    assert_eq!(s.assess(316.0, (300.0, 315.0)), ToleranceCall::Contradicted);
    assert_eq!(s.assess(305.0, (300.0, 315.0)), ToleranceCall::Supported);
}

#[test]
fn agent_configured_honors_its_margin() {
    let zero = AgentConfiguredStrategy { margin_frac: 0.0 };
    assert_eq!(
        zero.assess(316.0, (300.0, 315.0)),
        ToleranceCall::Contradicted
    );

    let wide = AgentConfiguredStrategy { margin_frac: 0.10 };
    // 325 is outside [300,315] but within the 10% buffer (270..346.5).
    assert_eq!(
        wide.assess(325.0, (300.0, 315.0)),
        ToleranceCall::Indeterminate
    );
}

#[test]
fn strategy_labels_are_stable() {
    assert_eq!(ConservativeStrategy::default().label(), "conservative");
    assert_eq!(TightStrategy.label(), "tight");
    assert_eq!(
        AgentConfiguredStrategy::default().label(),
        "coach_configured"
    );
}

#[test]
fn range_is_normalized_when_passed_reversed() {
    // (hi, lo) must behave the same as (lo, hi).
    let s = ConservativeStrategy::default();
    assert_eq!(s.assess(305.0, (315.0, 300.0)), ToleranceCall::Supported);
}

// ---------------------------------------------------------------------------
// personalized::check — probe + verdict
// ---------------------------------------------------------------------------

#[test]
fn pace_claim_far_off_is_contradicted() {
    let m = usable_metrics();
    let strat = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &m,
        tolerance: &strat,
    };
    // 4:00/km = 240s, far below the athlete's 5:00–5:15 threshold band.
    let c = claim(
        "Run your threshold pace at 4:00/km.",
        ClaimCategory::TrainingPrescription,
    );
    let outcome = personalized_check(&c, &ctx).expect("layer fires");
    assert_eq!(outcome.status, ClaimStatus::Contradicted);
    assert_eq!(outcome.layer_fired, VerdictLayer::Personalized);
    assert!(outcome.explanation.contains("5:08/km") || outcome.explanation.contains("4:00/km"));
}

#[test]
fn pace_claim_inside_range_is_supported() {
    let m = usable_metrics();
    let strat = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &m,
        tolerance: &strat,
    };
    let c = claim(
        "Hold your threshold pace around 5:05/km.",
        ClaimCategory::TrainingPrescription,
    );
    let outcome = personalized_check(&c, &ctx).expect("layer fires");
    assert_eq!(outcome.status, ClaimStatus::Supported);
    assert_eq!(outcome.layer_fired, VerdictLayer::Personalized);
}

#[test]
fn pace_claim_in_buffer_stays_silent() {
    let m = usable_metrics();
    let strat = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &m,
        tolerance: &strat,
    };
    // 4:45/km = 285s, in the conservative buffer → Indeterminate → None.
    let c = claim(
        "Try a threshold pace near 4:45/km.",
        ClaimCategory::TrainingPrescription,
    );
    assert!(personalized_check(&c, &ctx).is_none());
}

#[test]
fn tight_strategy_fires_where_conservative_stays_silent() {
    let m = usable_metrics();
    let c = claim(
        "Try a threshold pace near 4:45/km.",
        ClaimCategory::TrainingPrescription,
    );
    let conservative = ConservativeStrategy::default();
    let tight = TightStrategy;
    let ctx_c = PersonalizedContext {
        metrics: &m,
        tolerance: &conservative,
    };
    let ctx_t = PersonalizedContext {
        metrics: &m,
        tolerance: &tight,
    };
    assert!(personalized_check(&c, &ctx_c).is_none());
    let t = personalized_check(&c, &ctx_t).expect("tight fires");
    assert_eq!(t.status, ClaimStatus::Contradicted);
}

#[test]
fn vdot_point_claim_contradicted() {
    let m = usable_metrics();
    let strat = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &m,
        tolerance: &strat,
    };
    let c = claim("Your VDOT is 65.", ClaimCategory::Physiological);
    let outcome = personalized_check(&c, &ctx).expect("layer fires");
    assert_eq!(outcome.status, ClaimStatus::Contradicted);
    assert_eq!(outcome.layer_fired, VerdictLayer::Personalized);
}

#[test]
fn vdot_point_claim_supported() {
    let m = usable_metrics();
    let strat = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &m,
        tolerance: &strat,
    };
    let c = claim("Your VDOT is 52.", ClaimCategory::Physiological);
    let outcome = personalized_check(&c, &ctx).expect("layer fires");
    assert_eq!(outcome.status, ClaimStatus::Supported);
}

// ---------------------------------------------------------------------------
// FTP: ±3% supported, silent to 5%, a misquote beyond (carnet#1080)
// ---------------------------------------------------------------------------

/// The status the personalized layer gives "Your FTP is `quoted` W" for an
/// athlete whose recorded FTP is 300 W, or `None` when it stays silent.
fn ftp_call(quoted: &str, tolerance: &dyn ToleranceStrategy) -> Option<ClaimStatus> {
    let m = AthleteMetrics {
        ftp_watts: Some(300.0),
        ..usable_metrics()
    };
    let ctx = PersonalizedContext {
        metrics: &m,
        tolerance,
    };
    let c = claim(
        &format!("Your FTP is {quoted} W."),
        ClaimCategory::Physiological,
    );
    personalized_check(&c, &ctx).map(|outcome| outcome.status)
}

#[test]
fn an_ftp_quote_within_three_percent_is_supported() {
    let strat = AgentConfiguredStrategy::default();
    for quoted in ["300", "308.7", "291.3"] {
        assert_eq!(
            ftp_call(quoted, &strat),
            Some(ClaimStatus::Supported),
            "{quoted} W against 300 W"
        );
    }
}

#[test]
fn an_ftp_quote_between_three_and_five_percent_off_stays_silent() {
    let strat = AgentConfiguredStrategy::default();
    // 4% to 4.9% off: outside test-retest reliability, inside the misquote ceiling.
    for quoted in ["314.7", "285.3", "312"] {
        assert_eq!(ftp_call(quoted, &strat), None, "{quoted} W against 300 W");
    }
}

#[test]
fn an_ftp_quote_more_than_five_percent_off_is_contradicted() {
    let strat = AgentConfiguredStrategy::default();
    // ±5.1%, and a 10% misquote the default 8% buffer alone would leave silent.
    for quoted in ["315.3", "284.7", "330", "270"] {
        assert_eq!(
            ftp_call(quoted, &strat),
            Some(ClaimStatus::Contradicted),
            "{quoted} W against 300 W"
        );
    }
}

#[test]
fn the_ftp_misquote_ceiling_holds_whatever_the_agents_buffer() {
    // An agent YAML margin of 50% would leave a 10% misquote silent; the FTP
    // ceiling does not loosen with it.
    let wide = AgentConfiguredStrategy { margin_frac: 0.50 };
    assert_eq!(
        ftp_call("330", &wide),
        Some(ClaimStatus::Contradicted),
        "bug: a wide agent buffer loosened the FTP misquote ceiling"
    );
    assert_eq!(
        ftp_call("314.7", &wide),
        None,
        "bug: a quote inside the FTP ceiling fired under a wide buffer"
    );
}

#[test]
fn a_stricter_strategy_still_fires_inside_the_ftp_ceiling() {
    // Tight has no buffer: 4% off is past the ±3% band and contradicted.
    assert_eq!(
        ftp_call("312", &TightStrategy),
        Some(ClaimStatus::Contradicted),
        "bug: the FTP ceiling loosened a stricter strategy"
    );
    assert_eq!(
        ftp_call("308.7", &TightStrategy),
        Some(ClaimStatus::Supported),
        "bug: a quote inside test-retest reliability was not supported"
    );
}

#[test]
fn a_misquoted_ftp_names_the_ceiling_that_fired() {
    let m = AthleteMetrics {
        ftp_watts: Some(300.0),
        ..usable_metrics()
    };
    let strat = AgentConfiguredStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &m,
        tolerance: &strat,
    };
    let c = claim("Your FTP is 330 W.", ClaimCategory::Physiological);
    let outcome = personalized_check(&c, &ctx).expect("layer fires");
    assert_eq!(
        outcome.explanation,
        "Contradicts your own data — FTP 330 vs your 300 (more than 5% off your recorded FTP)",
        "bug: the explanation does not name the FTP ceiling that fired"
    );
}

#[test]
fn thin_data_gate_silences_the_layer() {
    let mut thin = usable_metrics();
    thin.data_days = 5; // below MIN_DATA_DAYS
    let strat = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &thin,
        tolerance: &strat,
    };
    let c = claim(
        "Run your threshold pace at 4:00/km.",
        ClaimCategory::TrainingPrescription,
    );
    assert!(personalized_check(&c, &ctx).is_none());
}

#[test]
fn no_numeric_metric_claim_returns_none() {
    let m = usable_metrics();
    let strat = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &m,
        tolerance: &strat,
    };
    let c = claim(
        "Stay hydrated and prioritise sleep.",
        ClaimCategory::Recovery,
    );
    assert!(personalized_check(&c, &ctx).is_none());
}

#[test]
fn empty_snapshot_is_not_usable() {
    let m = AthleteMetrics::default();
    assert!(!m.is_usable());
    let strat = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &m,
        tolerance: &strat,
    };
    let c = claim(
        "Run your threshold pace at 4:00/km.",
        ClaimCategory::TrainingPrescription,
    );
    assert!(personalized_check(&c, &ctx).is_none());
}

// ---------------------------------------------------------------------------
// Integration: the personalized layer fires inside the verdict pipeline (check_claim)
// ---------------------------------------------------------------------------

#[test]
fn check_claim_fires_personalized_before_evidence() {
    let m = usable_metrics();
    let strat = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &m,
        tolerance: &strat,
    };
    let c = claim(
        "Run your threshold pace at 4:00/km.",
        ClaimCategory::TrainingPrescription,
    );
    let outcome = check_claim(
        &c,
        &[],
        &corpus(),
        EvidenceStrength::Mixed,
        Some(&ctx),
        None,
    );
    assert_eq!(outcome.layer_fired, VerdictLayer::Personalized);
    assert_eq!(outcome.status, ClaimStatus::Contradicted);
}

#[test]
fn check_claim_without_snapshot_is_unchanged() {
    // Passing None must reproduce the pre-Layer-2.5 behavior exactly.
    let c = claim(
        "Run your threshold pace at 4:00/km.",
        ClaimCategory::TrainingPrescription,
    );
    let outcome = check_claim(&c, &[], &corpus(), EvidenceStrength::Mixed, None, None);
    assert_ne!(outcome.layer_fired, VerdictLayer::Personalized);
}

// ---------------------------------------------------------------------------
// Config defaults + factories
// ---------------------------------------------------------------------------

#[test]
fn default_personalized_config_reads_agent_yaml() {
    let cfg = VerificationConfig::default().personalized;
    assert!(cfg.enabled);
    assert_eq!(cfg.tolerance, ToleranceMode::AgentConfigured);
    assert_eq!(cfg.action, ActionMode::Inherit);
}

#[test]
fn tolerance_factory_selects_the_strategy() {
    let mut cfg = VerificationConfig::default().personalized;

    cfg.tolerance = ToleranceMode::AgentConfigured;
    assert_eq!(cfg.tolerance_strategy().label(), "coach_configured");

    cfg.tolerance = ToleranceMode::Conservative;
    assert_eq!(cfg.tolerance_strategy().label(), "conservative");

    cfg.tolerance = ToleranceMode::Tight;
    assert_eq!(cfg.tolerance_strategy().label(), "tight");
}

#[test]
fn inherit_policy_maps_fallback_behavior() {
    let contradicted = verdict(ClaimStatus::Contradicted, VerdictLayer::Personalized);

    assert_eq!(
        InheritConfigPolicy.resolve(
            &contradicted,
            &cfg_with_fallback(VerificationFallback::Warn)
        ),
        ResolvedAction::WarnBanner
    );
    assert_eq!(
        InheritConfigPolicy.resolve(
            &contradicted,
            &cfg_with_fallback(VerificationFallback::Silent)
        ),
        ResolvedAction::RecordOnly
    );
    assert_eq!(
        InheritConfigPolicy.resolve(
            &contradicted,
            &cfg_with_fallback(VerificationFallback::Block)
        ),
        ResolvedAction::BlockRetry
    );
}

#[test]
fn supported_verdict_resolves_to_pass() {
    let supported = verdict(ClaimStatus::Supported, VerdictLayer::Personalized);
    let cfg = VerificationConfig::default();
    assert_eq!(
        InheritConfigPolicy.resolve(&supported, &cfg),
        ResolvedAction::Pass
    );
    assert_eq!(
        AuditOnlyPolicy.resolve(&supported, &cfg),
        ResolvedAction::Pass
    );
    assert_eq!(
        UserWarnPolicy.resolve(&supported, &cfg),
        ResolvedAction::Pass
    );
}

#[test]
fn audit_only_and_user_warn_policies_differ() {
    let contradicted = verdict(ClaimStatus::Contradicted, VerdictLayer::Personalized);
    let cfg = VerificationConfig::default();
    assert_eq!(
        AuditOnlyPolicy.resolve(&contradicted, &cfg),
        ResolvedAction::RecordOnly
    );
    assert_eq!(
        UserWarnPolicy.resolve(&contradicted, &cfg),
        ResolvedAction::WarnBanner
    );
}

#[test]
fn personalized_config_parses_from_agent_yaml() {
    let prompt = "---\nverification_config:\n  personalized:\n    tolerance: tight\n    action: audit_only\n    margin_frac: 0.15\n---\nYou are a running coach.";
    let cfg = VerificationConfig::parse_from_system_prompt(prompt);
    assert_eq!(cfg.personalized.tolerance, ToleranceMode::Tight);
    assert_eq!(cfg.personalized.action, ActionMode::AuditOnly);
    assert!((cfg.personalized.margin_frac - 0.15).abs() < 1e-9);
}
