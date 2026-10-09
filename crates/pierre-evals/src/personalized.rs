// ABOUTME: The personalized-physiology layer of the bullshit detector — checks a claim against the athlete's OWN computed physiology
// ABOUTME: Pluggable ToleranceStrategy decides when a claimed number contradicts the athlete's VDOT-derived ranges
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Personalized physiology
//!
//! Where the deterministic-bounds layer ([`crate::deterministic_bounds`]) checks
//! a claim against *population* bounds ("HR max of 300 bpm is impossible"), this
//! layer checks it
//! against the *individual athlete's* computed physiology: VDOT-derived training
//! paces, HR zones, FTP, VO2max, and recent training load. This is what makes
//! "checked against your VDOT 52" literally true — a 4:00/km threshold
//! prescription is contradicted for an athlete whose VDOT 52 puts threshold at
//! ~5:08/km, even though 4:00/km is a perfectly plausible *population* pace.
//!
//! The snapshot ([`AthleteMetrics`]) is built by the caller (the chat pipeline
//! reads cageux + the training-history compute) and passed in. The crate stays
//! dependency-light: it never reaches for the athlete's data itself, it only
//! scores against a plain struct of `Option<f64>` ranges.
//!
//! ## Pluggable tolerance
//!
//! When a claimed number falls outside the athlete's expected range, whether
//! that fires `Contradicted` — and how much slack to allow first — is decided by
//! a [`ToleranceStrategy`]. Three are provided and selected per-agent via the
//! YAML `verification_config`:
//!
//! - [`AgentConfiguredStrategy`] (the default) reads the buffer margin from the
//!   agent's YAML config.
//! - [`ConservativeStrategy`] applies a fixed safety buffer an agent cannot
//!   loosen.
//! - [`TightStrategy`] allows zero buffer: any value outside the range is
//!   contradicted.
//!
//! ## Stale-data guard
//!
//! A VDOT computed from a single stale activity is a garbage estimate, and
//! firing "contradicted" off it is worse than staying silent. [`check`] returns
//! `None` (the claim falls through to the evidence-retrieval layer unchanged) whenever the snapshot
//! is backed by fewer than [`MIN_DATA_DAYS`] days of history.

use crate::claim_extractor::ExtractedClaim;
use crate::deterministic_bounds::extract_number_near;
use crate::verdict_engine::VerdictOutcome;
use pierre_core::models::{MeasurementKind, ATHLETE_REPORTED_ORIGIN};
use pierre_memory::{ClaimStatus, EvidenceStrength, VerdictLayer};

/// Minimum days of activity history backing the snapshot before the personalized
/// layer is allowed to fire. Below this the estimates are too noisy to contradict a
/// agent against.
pub const MIN_DATA_DAYS: u32 = 14;

/// Default buffer margin beyond the athlete's expected range.
///
/// A fraction of the metric value, applied before a claim is called
/// `Contradicted`. Used by [`AgentConfiguredStrategy`] when the agent YAML does
/// not override it.
pub const DEFAULT_MARGIN_FRAC: f64 = 0.08;

/// A snapshot of one athlete's computed physiology.
///
/// Built caller-side from cageux + the training-history compute. Every field is
/// optional: a `None` metric simply means claims about it fall through
/// unverified. Pace ranges are seconds per kilometre, lo (faster) to hi (slower).
///
/// LIMITATION(registre#853): `AthleteMetrics` carries no HRV field, so an HRV claim is never checked.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AthleteMetrics {
    /// Jack Daniels `VDOT` (`VO2max` adjusted for running economy).
    pub vdot: Option<f64>,
    /// `VO2max` in ml/kg/min.
    pub vo2max: Option<f64>,
    /// Easy / recovery training pace range (seconds per km).
    pub easy_pace_range: Option<(f64, f64)>,
    /// Marathon training pace range (seconds per km).
    pub marathon_pace_range: Option<(f64, f64)>,
    /// Threshold / tempo training pace range (seconds per km).
    pub threshold_pace_range: Option<(f64, f64)>,
    /// `VO2max` / interval training pace range (seconds per km).
    pub interval_pace_range: Option<(f64, f64)>,
    /// Functional Threshold Power (watts).
    pub ftp_watts: Option<f64>,
    /// Maximum heart rate (bpm).
    pub max_hr: Option<f64>,
    /// Recent training-stress balance / form (Coggan TSB).
    pub recent_tsb: Option<f64>,
    /// Critical power (watts), with whether it was measured or estimated.
    pub critical_power_watts: Option<StoredMetric>,
    /// W′ (joules), with its provenance.
    pub w_prime_joules: Option<StoredMetric>,
    /// Critical speed (metres per second), with its provenance.
    pub critical_speed_mps: Option<StoredMetric>,
    /// D′ (metres), with its provenance.
    pub d_prime_meters: Option<StoredMetric>,
    /// Days of activity history backing these estimates; gates [`MIN_DATA_DAYS`].
    pub data_days: u32,
}

/// A stored physiological value and whether the athlete's profile holds it as
/// an estimate — the snapshot's copy of the profile's provenance, kept plain
/// so this crate never reaches for the profile type.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredMetric {
    /// The stored value, in the unit the field name carries.
    pub value: f64,
    /// Measured, or estimated — modelled or stated without a named test, and
    /// so quoted as an estimate.
    pub kind: MeasurementKind,
    /// Who produced the value (`vekta`, `lab`, `athlete-reported`), when known.
    pub origin: Option<String>,
}

/// The prefix of every explanation the framing check writes. Verdicts reach
/// the outside world only as persisted rows, so this is what counts them.
const ESTIMATE_STATED_AS_MEASUREMENT: &str = "Estimate stated as a measurement";

/// Words that mark a number as an estimate or attribute it to a source, in the
/// five shipped locales. Matched as lowercase substrings, so a stem covers its
/// inflections: `estim` is estimate, estimated, estimé, estimado, estimativa.
const ESTIMATE_MARKERS: &[&str] = &[
    // en / fr / es / pt
    "estim",
    "model",
    "modél",
    "according to",
    "predict",
    // fr
    "selon",
    "prédi",
    // es
    "según",
    // de
    "schätz",
    "laut ",
    // pt. Not "segundo": in es and pt it is also "second", and "metros por
    // segundo" is how a critical speed is stated. "Segundo a Vekta" still
    // counts, through the source's own name.
    "de acordo com",
];

/// True when `sentence` frames a value as an estimate: an estimate marker, or
/// the source that produced the value named in the same sentence.
fn frames_as_estimate(sentence: &str, origin: Option<&str>) -> bool {
    let lower = sentence.to_lowercase();
    ESTIMATE_MARKERS.iter().any(|m| lower.contains(m))
        || origin
            .map(|o| o.trim().to_lowercase())
            .is_some_and(|o| !o.is_empty() && o != ATHLETE_REPORTED_ORIGIN && lower.contains(&o))
}

impl AthleteMetrics {
    /// True when at least one metric is populated *and* the history is deep
    /// enough to trust — i.e. the layer has something it can confidently fire.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        self.data_days >= MIN_DATA_DAYS
            && (self.vdot.is_some()
                || self.vo2max.is_some()
                || self.easy_pace_range.is_some()
                || self.marathon_pace_range.is_some()
                || self.threshold_pace_range.is_some()
                || self.interval_pace_range.is_some()
                || self.ftp_watts.is_some()
                || self.max_hr.is_some()
                || self.recent_tsb.is_some()
                || self.critical_power_watts.is_some()
                || self.w_prime_joules.is_some()
                || self.critical_speed_mps.is_some()
                || self.d_prime_meters.is_some())
    }
}

/// The verdict of comparing one claimed number against the athlete's expected
/// range for that metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToleranceCall {
    /// The claim sits inside the athlete's expected range.
    Supported,
    /// The claim is clearly outside the range, beyond the strategy's buffer.
    Contradicted,
    /// The claim is outside the range but within the buffer — too close to
    /// call, so the layer stays silent and the claim falls through.
    Indeterminate,
}

/// Pluggable policy for when a claimed value contradicts the athlete's range.
///
/// Decides how much buffer to allow before firing. Selected per-agent via the
/// YAML `verification_config` (see
/// [`crate::verification_config::PersonalizedConfig`]).
pub trait ToleranceStrategy: Send + Sync {
    /// Score `claim` against the athlete's expected `[lo, hi]` range for a metric.
    fn assess(&self, claim: f64, range: (f64, f64)) -> ToleranceCall;
    /// Stable label for the audit trail / explanation text.
    fn label(&self) -> &'static str;
}

/// The default strategy: the buffer margin comes from the agent's YAML config
/// (falling back to [`DEFAULT_MARGIN_FRAC`]).
#[derive(Debug, Clone, Copy)]
pub struct AgentConfiguredStrategy {
    /// Buffer margin as a fraction of the metric value.
    pub margin_frac: f64,
}

impl Default for AgentConfiguredStrategy {
    fn default() -> Self {
        Self {
            margin_frac: DEFAULT_MARGIN_FRAC,
        }
    }
}

impl ToleranceStrategy for AgentConfiguredStrategy {
    fn assess(&self, claim: f64, range: (f64, f64)) -> ToleranceCall {
        assess_with_margin(claim, range, self.margin_frac.max(0.0))
    }
    fn label(&self) -> &'static str {
        "coach_configured"
    }
}

/// A fixed safety buffer an agent cannot loosen or override. Use when the
/// platform wants a guaranteed floor on how aggressively contradictions fire,
/// independent of any per-agent YAML.
#[derive(Debug, Clone, Copy)]
pub struct ConservativeStrategy {
    margin_frac: f64,
}

impl Default for ConservativeStrategy {
    fn default() -> Self {
        Self {
            margin_frac: DEFAULT_MARGIN_FRAC,
        }
    }
}

impl ToleranceStrategy for ConservativeStrategy {
    fn assess(&self, claim: f64, range: (f64, f64)) -> ToleranceCall {
        assess_with_margin(claim, range, self.margin_frac)
    }
    fn label(&self) -> &'static str {
        "conservative"
    }
}

/// Zero buffer: any value outside the athlete's range is contradicted. The
/// strictest opt-in, for agents running their prescriptions as hard QA.
#[derive(Debug, Clone, Copy, Default)]
pub struct TightStrategy;

impl ToleranceStrategy for TightStrategy {
    fn assess(&self, claim: f64, range: (f64, f64)) -> ToleranceCall {
        assess_with_margin(claim, range, 0.0)
    }
    fn label(&self) -> &'static str {
        "tight"
    }
}

/// Shared scoring logic: a claim inside `[lo, hi]` is `Supported`; outside but
/// within the `margin_frac` buffer is `Indeterminate` (stay silent); beyond the
/// buffer is `Contradicted`.
fn assess_with_margin(claim: f64, range: (f64, f64), margin_frac: f64) -> ToleranceCall {
    let (lo, hi) = normalize_range(range);
    if claim >= lo && claim <= hi {
        return ToleranceCall::Supported;
    }
    let buffer_lo = lo * (1.0 - margin_frac);
    let buffer_hi = hi * (1.0 + margin_frac);
    if claim >= buffer_lo && claim <= buffer_hi {
        ToleranceCall::Indeterminate
    } else {
        ToleranceCall::Contradicted
    }
}

/// Order a range so `lo <= hi`, guarding against a caller passing `(hi, lo)`.
fn normalize_range((a, b): (f64, f64)) -> (f64, f64) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Everything the personalized layer needs, bundled so the pipeline can thread a single
/// `Option<&PersonalizedContext>` (where `None` skips the layer entirely).
pub struct PersonalizedContext<'a> {
    /// The athlete's computed physiology snapshot.
    pub metrics: &'a AthleteMetrics,
    /// The active tolerance strategy.
    pub tolerance: &'a dyn ToleranceStrategy,
}

/// A pace probe: candidate keywords for a metric, the athlete's range for it
/// (when known), and a short label for the explanation text.
type PaceProbe = (&'static [&'static str], Option<(f64, f64)>, &'static str);

/// Run the personalized layer over a single claim.
///
/// Returns `Some(verdict)` when the claim makes a checkable numeric assertion
/// about a metric the snapshot covers and the strategy reaches a confident
/// call; `None` otherwise (the claim then flows to the evidence-retrieval layer unchanged).
#[must_use]
pub fn check(claim: &ExtractedClaim, ctx: &PersonalizedContext<'_>) -> Option<VerdictOutcome> {
    let m = ctx.metrics;
    if !m.is_usable() {
        return None;
    }
    let text = claim.text.as_str();

    // --- Critical-power family: probed first, because "threshold power" is
    // an FTP keyword and would otherwise claim a CP sentence. ---
    if let Some(verdict) = check_critical_power_family(text, ctx) {
        return Some(verdict);
    }

    // --- Pace probes: value parsed as M:SS/km, scored against the native range ---
    let pace_probes: [PaceProbe; 4] = [
        (
            &[
                "threshold pace",
                "tempo pace",
                "lactate threshold pace",
                "threshold",
                "tempo",
                // fr
                "allure seuil",
                "allure au seuil",
                "allure tempo",
                "seuil",
                // es
                "ritmo umbral",
                "ritmo de umbral",
                "umbral",
                // de
                "schwellentempo",
                "schwellenpace",
                "schwelle",
                // pt
                "ritmo limiar",
                "ritmo de limiar",
                "limiar",
            ],
            m.threshold_pace_range,
            "threshold",
        ),
        (
            &[
                "easy pace",
                "recovery pace",
                "easy run",
                // fr
                "allure facile",
                "allure endurance",
                "allure de récupération",
                "endurance fondamentale",
                // es
                "ritmo fácil",
                "ritmo suave",
                "ritmo de recuperación",
                // de
                "lockeres tempo",
                "grundlagentempo",
                "regenerationstempo",
                // pt
                "ritmo fácil",
                "ritmo leve",
                "ritmo de recuperação",
            ],
            m.easy_pace_range,
            "easy",
        ),
        (
            &[
                "marathon pace",
                // fr / es / de / pt
                "allure marathon",
                "ritmo maratón",
                "marathontempo",
                "ritmo maratona",
            ],
            m.marathon_pace_range,
            "marathon",
        ),
        (
            &[
                "interval pace",
                "vo2max pace",
                "vo2 max pace",
                "5k pace",
                "rep pace",
                "repetition pace",
                // fr
                "allure intervalle",
                "allure vo2max",
                "allure 5 km",
                // es
                "ritmo intervalo",
                "ritmo de intervalo",
                // de
                "intervalltempo",
                // pt
                "ritmo intervalo",
                "ritmo de intervalo",
            ],
            m.interval_pace_range,
            "interval",
        ),
    ];
    for (keywords, range, label) in pace_probes {
        let Some(range) = range else { continue };
        for kw in keywords {
            if let Some(pace) = extract_pace_sec_per_km(text, kw) {
                return verdict_for(ctx, range, pace, &pace_descr(label, pace, range));
            }
        }
    }

    // --- Point probes: single value scored against a physiological epsilon band ---
    if let Some(vdot) = m.vdot {
        if let Some(v) = extract_number_near(text, "vdot") {
            // VDOT estimation error is ~±1 point.
            return verdict_for(
                ctx,
                (vdot - 1.0, vdot + 1.0),
                v,
                &point_descr("VDOT", vdot, v),
            );
        }
    }
    if let Some(vo2) = m.vo2max {
        for kw in [
            "vo2max",
            "vo2 max",
            "vo₂max",
            "consommation maximale d'oxygène",
        ] {
            if let Some(v) = extract_number_near(text, kw) {
                return verdict_for(
                    ctx,
                    (vo2 - 2.0, vo2 + 2.0),
                    v,
                    &point_descr("VO2max", vo2, v),
                );
            }
        }
    }
    if let Some(ftp) = m.ftp_watts {
        for kw in [
            "ftp",
            "functional threshold power",
            "threshold power",
            // fr
            "seuil de puissance",
            "puissance au seuil",
            "puissance seuil",
            // es
            "potencia umbral",
            "potencia de umbral",
            // de
            "schwellenleistung",
            // pt
            "potência limiar",
            "potência de limiar",
        ] {
            if let Some(v) = extract_number_near(text, kw) {
                // FTP test-retest reliability is ~±3%.
                return verdict_for(
                    ctx,
                    (ftp * 0.97, ftp * 1.03),
                    v,
                    &point_descr("FTP", ftp, v),
                );
            }
        }
    }
    if let Some(mhr) = m.max_hr {
        for kw in [
            "max heart rate",
            "maximum heart rate",
            "max hr",
            "hrmax",
            // fr
            "fréquence cardiaque maximale",
            "fréquence cardiaque max",
            "fc max",
            "fcmax",
            // es
            "frecuencia cardíaca máxima",
            "fc máx",
            // de
            "maximale herzfrequenz",
            "maximalpuls",
            "hfmax",
            // pt
            "frequência cardíaca máxima",
            "fc máx",
        ] {
            if let Some(v) = extract_number_near(text, kw) {
                return verdict_for(
                    ctx,
                    (mhr - 3.0, mhr + 3.0),
                    v,
                    &point_descr("max HR", mhr, v),
                );
            }
        }
    }
    // LIMITATION(registre#850): `recent_tsb` is compared as absolute TSB near keywords, never as a share of
    // the athlete's fitness (CTL), the way the product frames form.
    if let Some(tsb) = m.recent_tsb {
        for kw in [
            "tsb",
            "training stress balance",
            "current form",
            // fr. "indice de fatigue" is here because the agent invented it
            // live on 2026-09-02 and then repeated it for fifteen turns — a
            // label the model actually uses is worth probing whether or not
            // anybody chose it (registre#204).
            "forme actuelle",
            "équilibre de charge",
            "indice de fatigue",
            "indice de forme",
            // es
            "forma actual",
            "balance de carga",
            "índice de fatiga",
            // de
            "aktuelle form",
            "formwert",
            "ermüdungsindex",
            // pt
            "forma atual",
            "equilíbrio de carga",
            "índice de fadiga",
        ] {
            if let Some(v) = extract_number_near(text, kw) {
                return verdict_for(ctx, (tsb - 5.0, tsb + 5.0), v, &point_descr("TSB", tsb, v));
            }
        }
    }

    None
}

/// Keywords for critical power, in the five shipped locales. No bare "cp":
/// as a substring it matches too much to anchor a number.
const CRITICAL_POWER_KEYWORDS: &[&str] = &[
    "critical power",
    "puissance critique",
    "potencia crítica",
    "kritische leistung",
    "potência crítica",
];

/// Keywords for W′. The apostrophe forms are how a reply types the prime.
const W_PRIME_KEYWORDS: &[&str] = &["w′", "w'", "w prime", "w-prime"];

/// Keywords for critical speed, in the five shipped locales.
const CRITICAL_SPEED_KEYWORDS: &[&str] = &[
    "critical speed",
    "vitesse critique",
    "velocidad crítica",
    "kritische geschwindigkeit",
    "velocidade crítica",
];

/// Keywords for D′. No `d'`: in French it is an elision on every other word.
const D_PRIME_KEYWORDS: &[&str] = &["d′", "d prime", "d-prime"];

/// Below this a claimed W′ is read as kilojoules (21.5 → 21 500 J): no human
/// W′ is under 100 J, and a reply says "21.5 kJ" far more than "21500 J".
const W_PRIME_KJ_CEILING: f64 = 100.0;

/// Above this a claimed critical speed is read as km/h (15 → 4.17 m/s): no
/// human critical speed is above 10 m/s.
const CRITICAL_SPEED_MPS_CEILING: f64 = 10.0;

/// How one critical-power family parameter is scored and described.
struct FamilyProbe {
    /// The name the explanation gives the parameter.
    label: &'static str,
    /// Half-width of the accepted band, as a fraction of the stored value.
    band_frac: f64,
    /// Decimals the explanation prints the values with.
    decimals: usize,
}

/// Critical power: ±5%, whole watts.
const CP_PROBE: FamilyProbe = FamilyProbe {
    label: "CP",
    band_frac: 0.05,
    decimals: 0,
};

/// W′: ±25%, whole joules.
const W_PRIME_PROBE: FamilyProbe = FamilyProbe {
    label: "W′",
    band_frac: 0.25,
    decimals: 0,
};

/// Critical speed: ±3%, in m/s to the hundredth (4.17, not 4).
const CRITICAL_SPEED_PROBE: FamilyProbe = FamilyProbe {
    label: "critical speed",
    band_frac: 0.03,
    decimals: 2,
};

/// D′: ±25%, whole metres.
const D_PRIME_PROBE: FamilyProbe = FamilyProbe {
    label: "D′",
    band_frac: 0.25,
    decimals: 0,
};

/// Score a claim about critical power, W′, critical speed or D′ against the
/// stored value, then check how an estimate is framed.
///
/// Bands are the test-retest reliability of each parameter
/// (`Methodology/Intelligence/Critical Power and Critical Speed`): CP ±5%,
/// CS ±3%, W′ and D′ ±25%.
fn check_critical_power_family(
    text: &str,
    ctx: &PersonalizedContext<'_>,
) -> Option<VerdictOutcome> {
    let m = ctx.metrics;
    if let Some(ref cp) = m.critical_power_watts {
        if let Some(v) = first_number_near(text, CRITICAL_POWER_KEYWORDS) {
            return provenanced_verdict(ctx, text, cp, v, &CP_PROBE);
        }
    }
    if let Some(ref w_prime) = m.w_prime_joules {
        if let Some(v) = first_number_near(text, W_PRIME_KEYWORDS) {
            let joules = if v < W_PRIME_KJ_CEILING {
                v * 1000.0
            } else {
                v
            };
            return provenanced_verdict(ctx, text, w_prime, joules, &W_PRIME_PROBE);
        }
    }
    if let Some(ref cs) = m.critical_speed_mps {
        let pace = CRITICAL_SPEED_KEYWORDS
            .iter()
            .find_map(|kw| extract_pace_sec_per_km(text, kw))
            .filter(|sec| *sec > 0.0)
            .map(|sec| 1000.0 / sec);
        let speed = pace.or_else(|| {
            first_number_near(text, CRITICAL_SPEED_KEYWORDS).map(|v| {
                if v > CRITICAL_SPEED_MPS_CEILING {
                    v / 3.6
                } else {
                    v
                }
            })
        });
        if let Some(v) = speed {
            return provenanced_verdict(ctx, text, cs, v, &CRITICAL_SPEED_PROBE);
        }
    }
    if let Some(ref d_prime) = m.d_prime_meters {
        if let Some(v) = first_number_near(text, D_PRIME_KEYWORDS) {
            return provenanced_verdict(ctx, text, d_prime, v, &D_PRIME_PROBE);
        }
    }
    None
}

/// The number nearest the first keyword of `keywords` the text contains.
fn first_number_near(text: &str, keywords: &[&str]) -> Option<f64> {
    keywords.iter().find_map(|kw| extract_number_near(text, kw))
}

/// The verdict for a claim about a provenanced value.
///
/// A wrong number is contradicted as any other. A right number that the
/// profile holds as an estimate, quoted with no estimate marker and no source,
/// is `Unsupported`: the number matches, but the sentence asserts a
/// measurement the athlete never made — "your CP is 312 W" where only "Vekta
/// estimates your CP at 312 W" is true.
fn provenanced_verdict(
    ctx: &PersonalizedContext<'_>,
    text: &str,
    stored: &StoredMetric,
    claimed: f64,
    probe: &FamilyProbe,
) -> Option<VerdictOutcome> {
    let FamilyProbe {
        label,
        band_frac,
        decimals,
    } = *probe;
    let range = (
        stored.value * (1.0 - band_frac),
        stored.value * (1.0 + band_frac),
    );
    let verdict = verdict_for(
        ctx,
        range,
        claimed,
        &format!(
            "{label} {claimed:.decimals$} vs your {:.decimals$}",
            stored.value
        ),
    )?;
    if verdict.status != ClaimStatus::Supported
        || stored.kind == MeasurementKind::Measured
        || frames_as_estimate(text, stored.origin.as_deref())
    {
        return Some(verdict);
    }
    let source = stored.origin.as_deref().unwrap_or("a model");
    Some(VerdictOutcome {
        status: ClaimStatus::Unsupported,
        evidence_strength: EvidenceStrength::Strong,
        confidence: 0.9,
        layer_fired: VerdictLayer::Personalized,
        explanation: format!(
            "{ESTIMATE_STATED_AS_MEASUREMENT} — your {label} of {:.decimals$} is an estimate from {source}; quote it as one",
            stored.value
        ),
        evidence_refs: None,
    })
}

/// Build the verdict for a resolved probe. `Indeterminate` collapses to `None`
/// so the claim flows through to the evidence-retrieval layer instead of firing a weak verdict.
fn verdict_for(
    ctx: &PersonalizedContext<'_>,
    range: (f64, f64),
    value: f64,
    descr: &str,
) -> Option<VerdictOutcome> {
    let outcome = match ctx.tolerance.assess(value, range) {
        ToleranceCall::Supported => VerdictOutcome {
            status: ClaimStatus::Supported,
            evidence_strength: EvidenceStrength::Strong,
            confidence: 0.9,
            layer_fired: VerdictLayer::Personalized,
            explanation: format!("Consistent with your own data — {descr}"),
            evidence_refs: None,
        },
        ToleranceCall::Contradicted => VerdictOutcome {
            status: ClaimStatus::Contradicted,
            evidence_strength: EvidenceStrength::Strong,
            confidence: 0.9,
            layer_fired: VerdictLayer::Personalized,
            explanation: format!(
                "Contradicts your own data — {descr} ({} tolerance)",
                ctx.tolerance.label()
            ),
            evidence_refs: None,
        },
        ToleranceCall::Indeterminate => return None,
    };
    Some(outcome)
}

/// Parse a running pace expressed as `M:SS` per km within a short window after
/// `keyword`, returning seconds per km. Plain [`extract_number_near`] cannot
/// read pace because the `:` splits it into two integers.
fn extract_pace_sec_per_km(text: &str, keyword: &str) -> Option<f64> {
    let lower = text.to_lowercase();
    let kw = lower.find(keyword)?;
    let end = (kw + keyword.len() + 40).min(lower.len());
    let window = lower.get(kw..end)?;
    parse_first_pace(window)
}

/// Scan `s` for the first `M:SS` token and convert it to seconds. `SS` must be
/// exactly two digits and `< 60` to be a valid pace.
fn parse_first_pace(s: &str) -> Option<f64> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let m_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b':' {
            continue;
        }
        let minutes: f64 = s.get(m_start..i)?.parse().ok()?;
        i += 1; // skip ':'
        let s_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i - s_start == 2 {
            if let Ok(seconds) = s.get(s_start..i)?.parse::<f64>() {
                if seconds < 60.0 {
                    return Some(minutes.mul_add(60.0, seconds));
                }
            }
        }
    }
    None
}

/// Format seconds-per-km back into `M:SS/km` for the explanation text.
fn fmt_pace(sec_per_km: f64) -> String {
    let total = sec_per_km.max(0.0).round();
    let minutes = (total / 60.0).floor();
    let seconds = total - minutes * 60.0;
    format!("{minutes:.0}:{seconds:02.0}/km")
}

/// Human-readable pace comparison for the verdict explanation.
fn pace_descr(label: &str, claim: f64, range: (f64, f64)) -> String {
    let (lo, hi) = normalize_range(range);
    format!(
        "{label} pace {} vs your {}–{}",
        fmt_pace(claim),
        fmt_pace(lo),
        fmt_pace(hi)
    )
}

/// Human-readable point-metric comparison for the verdict explanation.
fn point_descr(label: &str, athlete: f64, claim: f64) -> String {
    format!("{label} {claim:.0} vs your {athlete:.0}")
}
