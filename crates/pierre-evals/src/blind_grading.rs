// ABOUTME: Blind grading export — a stratified, seeded sample of production verdicts into two grader sheets and a key
// ABOUTME: A sheet row carries the claim and nothing the checker decided; the key holding the verdicts stays with the operator
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Blind grading of production claims (carnet#854)
//!
//! #455's false-positive rate is read off support's dispositions, and a
//! disposition exists only on a verdict someone opened — a flagged one, nearly
//! always. The claims the checker let through are the ones nobody opens, so
//! the rate at which it lets a *false* claim through (the silent-miss rate)
//! can only come from people who grade claims without seeing the verdict.
//! This module builds what they grade:
//!
//! 1. `stratified_sample` draws the sample from the tenant's
//!    [`GradingPool`] with proportional allocation over the [`Stratum`]
//!    (layer × category × locale × agent), largest remainder, seeded. The
//!    allocation is proportional so the sample is self-weighting: an
//!    unweighted rate over it estimates the window's rate, and the per-stratum
//!    pool and sample counts in the key let anyone check that it did.
//! 2. [`build_export`] numbers the sample in a shuffled order, gives each
//!    athlete a pseudonym, picks the overlap both graders see, and splits the
//!    rest between the two sheets.
//! 3. [`crate::blind_grading_score`] joins the graded sheets back to the key.
//!
//! The pool itself is the repository's: tenant-scoped, and already stripped of
//! every verdict whose athlete has not opted in to analytics, has withheld
//! sharing with their coach, or whose reply was built from first-party-only
//! data.
//!
//! ## What the rates cover
//!
//! Two conditions bound every rate a blind grade reports, and both travel with
//! the number ([`PoolCoverage`] in the key and the report):
//!
//! - **The non-first-party pool only.** The chat stage stamps a verdict
//!   first-party-only whenever its reply read data from a provider whose terms
//!   keep that data on Dravr's own surfaces, and most connected providers
//!   declare so. Those verdicts never reach a grader, so the pool is the
//!   replies that read no such data, and the athlete-data layer — where the
//!   known silent paths sit — can be nearly absent from it. The sample is
//!   self-weighting over that pool, not over the tenant's replies; the
//!   athlete-data layer is measured by the seeded fabrications
//!   ([`crate::seeded_fabrications`]), whose synthetic athletes no provider's
//!   terms cover.
//! - **Extracted claims only.** The pool is `claim_verdicts` rows, so a false
//!   sentence the extractor never turned into a claim is never sampled. The
//!   silent-miss rate is the rate among extracted claims, and reads below the
//!   rate over everything the agents said; the seeded run's not-extracted
//!   share is the measure of that gap.
//!
//! [`GradingPool`]: pierre_memory::claims::GradingPool
//! [`Stratum`]: crate::blind_grading::Stratum
//! [`build_export`]: crate::blind_grading::build_export
//! [`PoolCoverage`]: crate::blind_grading::PoolCoverage

use std::collections::BTreeMap;
use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use pierre_memory::claims::{
    ClaimCategory, ClaimStatus, GradingCandidate, GradingPool, GradingWithheld, VerdictLayer,
};
use serde::{Deserialize, Serialize};

use crate::grading_csv::{record, spreadsheet_text};

/// How many graders the export writes a sheet for. Two is what an
/// inter-grader agreement needs.
const GRADER_SHEETS: usize = 2;

/// The columns of a grader sheet, in order. `grade` and `note` are left blank
/// for the grader.
const SHEET_HEADER: [&str; 7] = [
    "item_id",
    "athlete",
    "claimed_on",
    "locale",
    "claim",
    "grade",
    "note",
];

/// Whether a verdict flags its claim: `unsupported` or `contradicted`, the two
/// statuses #455's health counts as flagged, so a false-positive rate taken
/// here compares with that one. (The chat stage additionally keeps an
/// unsupported training prescription off the athlete's banner.)
#[must_use]
pub(crate) const fn flags(status: ClaimStatus) -> bool {
    matches!(status, ClaimStatus::Unsupported | ClaimStatus::Contradicted)
}

/// The cell of the sample a verdict is drawn from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Stratum {
    /// The layer that decided the verdict.
    pub layer: String,
    /// The extractor's category for the claim.
    pub category: String,
    /// The athlete's locale.
    pub locale: String,
    /// The agent that made the claim; `None` when the verdict names none.
    pub agent: Option<String>,
}

impl Stratum {
    /// The stratum a candidate belongs to.
    #[must_use]
    fn of(candidate: &GradingCandidate) -> Self {
        Self {
            layer: candidate.verdict.layer_fired.as_str().to_owned(),
            category: candidate.verdict.category.as_str().to_owned(),
            locale: candidate.locale.clone(),
            agent: candidate.verdict.agent_id.clone(),
        }
    }
}

/// `SplitMix64` (Steele, Lea & Flood, 2014): a published 64-bit generator, here
/// so a seed draws the same sample on every build — a library generator
/// promises no stable stream across its versions.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        // The generator's published increment and mixing constants.
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A draw in `0..bound`; `bound` is at least 1 at every call site. The
    /// modulo bias at the sizes sampled here (a few thousand) is below 1e-15.
    fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound.max(1) as u64) as usize
    }
}

/// Fisher–Yates, in place.
fn shuffle<T>(items: &mut [T], rng: &mut SplitMix64) {
    for i in (1..items.len()).rev() {
        let j = rng.below(i + 1);
        items.swap(i, j);
    }
}

/// Largest-remainder proportional allocation of `size` seats over strata of
/// `counts` members; `size` is at most the sum of `counts`. Remainders are
/// ranked largest first, ties to the earlier stratum, and no stratum gets more
/// seats than it has members.
fn allocate(counts: &[usize], size: usize) -> Vec<usize> {
    let total: usize = counts.iter().sum();
    let size = size.min(total);
    if total == 0 {
        return vec![0; counts.len()];
    }
    let exact: Vec<f64> = counts
        .iter()
        .map(|&c| c as f64 * size as f64 / total as f64)
        .collect();
    let mut seats: Vec<usize> = exact
        .iter()
        .zip(counts)
        .map(|(e, &c)| (e.floor() as usize).min(c))
        .collect();
    let mut order: Vec<usize> = (0..counts.len()).collect();
    order.sort_by(|&a, &b| {
        let frac = |i: usize| exact[i] - exact[i].floor();
        frac(b).total_cmp(&frac(a)).then(a.cmp(&b))
    });
    let mut left = size - seats.iter().sum::<usize>();
    // One pass hands out every remaining seat unless floating-point rounding
    // left a stratum a whisker short; the loop then goes round again, and it
    // terminates because `size` never exceeds the members available.
    while left > 0 {
        for &i in &order {
            if left == 0 {
                break;
            }
            if seats[i] < counts[i] {
                seats[i] += 1;
                left -= 1;
            }
        }
    }
    seats
}

/// Draw `size` candidates with proportional allocation over [`Stratum`],
/// seeded by `seed`, returned in a shuffled order so nothing about a
/// candidate's position says which stratum it came from. The whole pool comes
/// back (shuffled) when it is no larger than `size`.
#[must_use]
fn stratified_sample(
    candidates: Vec<GradingCandidate>,
    size: usize,
    seed: u64,
) -> Vec<GradingCandidate> {
    let mut rng = SplitMix64(seed);
    let mut strata: BTreeMap<Stratum, Vec<GradingCandidate>> = BTreeMap::new();
    for candidate in candidates {
        strata
            .entry(Stratum::of(&candidate))
            .or_default()
            .push(candidate);
    }
    let counts: Vec<usize> = strata.values().map(Vec::len).collect();
    let quotas = allocate(&counts, size);

    let mut sample = Vec::with_capacity(quotas.iter().sum());
    for (mut members, quota) in strata.into_values().zip(quotas) {
        shuffle(&mut members, &mut rng);
        members.truncate(quota);
        sample.extend(members);
    }
    shuffle(&mut sample, &mut rng);
    sample
}

/// What an export draws, from which tenant, with which seed.
#[derive(Debug, Clone)]
pub struct ExportSpec {
    /// Tenant the pool was read from; recorded in the key.
    pub tenant_id: String,
    /// Items to draw.
    pub size: usize,
    /// Of those, how many both graders see. Clamped to the sample.
    pub overlap: usize,
    /// Seed for the sample, the item order and the overlap.
    pub seed: u64,
}

/// One stratum's share of the pool and of the sample.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StratumCount {
    /// The stratum.
    pub stratum: Stratum,
    /// Eligible verdicts read in it.
    pub pool: usize,
    /// Items drawn from it.
    pub sample: usize,
}

/// One sampled claim as the operator's key records it: the item the graders
/// see, and the verdict they must not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyItem {
    /// The id on the grader sheets.
    pub item_id: String,
    /// The athlete's pseudonym on the grader sheets.
    pub athlete: String,
    /// Which sheets (1-based) carry the item; two for the overlap.
    pub sheets: Vec<usize>,
    /// The verdict row.
    pub verdict_id: String,
    /// The athlete the claim was said to.
    pub user_id: String,
    /// The agent that made the claim.
    pub agent_id: Option<String>,
    /// The conversation it came from.
    pub conversation_id: Option<String>,
    /// The reply it was extracted from.
    pub message_id: Option<String>,
    /// When the verdict was recorded.
    pub claimed_at: DateTime<Utc>,
    /// The athlete's locale.
    pub locale: String,
    /// The claim as extracted.
    pub claim_text: String,
    /// The extractor's category.
    pub category: ClaimCategory,
    /// The checker's verdict.
    pub status: ClaimStatus,
    /// The layer that decided it.
    pub layer: VerdictLayer,
    /// Whether the verdict flags the claim: `unsupported` or `contradicted`.
    pub flagged: bool,
}

/// The operator's half of an export: everything needed to score the graded
/// sheets and to re-check the draw. Never given to a grader.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradingKey {
    /// Tenant the pool was read from.
    pub tenant_id: String,
    /// Seed the sample was drawn with.
    pub seed: u64,
    /// Window lower bound.
    pub window_start: DateTime<Utc>,
    /// Window length in days.
    pub window_days: i64,
    /// Verdicts the tenant recorded in the window.
    pub in_window: i64,
    /// What the consent and provenance rules withheld.
    pub withheld: GradingWithheld,
    /// Verdicts that passed every rule.
    pub eligible: i64,
    /// Eligible verdicts read; below `eligible` when the read was capped.
    pub pool_read: usize,
    /// Items drawn.
    pub sample_size: usize,
    /// Items both graders see.
    pub overlap: usize,
    /// Pool and sample counts per stratum.
    pub strata: Vec<StratumCount>,
    /// The sampled items, by item id.
    pub items: Vec<KeyItem>,
}

impl GradingKey {
    /// What the key's pool covers of its window.
    #[must_use]
    pub const fn coverage(&self) -> PoolCoverage {
        PoolCoverage {
            in_window: self.in_window,
            eligible: self.eligible,
            withheld: self.withheld,
        }
    }
}

/// What a grading pool covers of its window.
///
/// Every rate a blind grade reports holds for the eligible pool, never for the
/// whole window, so this is printed when the sample is drawn and again beside
/// the scored rates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PoolCoverage {
    /// Verdicts the tenant recorded in the window.
    pub in_window: i64,
    /// Verdicts that passed every rule.
    pub eligible: i64,
    /// What each rule withheld.
    pub withheld: GradingWithheld,
}

impl PoolCoverage {
    /// Of the window's verdicts, the share withheld because their reply read
    /// first-party-only data; `None` for an empty window.
    #[must_use]
    fn first_party_only_share(&self) -> Option<f64> {
        (self.in_window > 0).then(|| self.withheld.first_party_only as f64 / self.in_window as f64)
    }

    /// Whether the first-party-only rule withheld most of the window.
    #[must_use]
    const fn first_party_only_majority(&self) -> bool {
        self.withheld.first_party_only.saturating_mul(2) > self.in_window
    }

    /// The coverage as text: the counts, the first-party-only share, and a
    /// warning when that rule withheld most of the window.
    #[must_use]
    pub fn render(&self) -> String {
        let share = self
            .first_party_only_share()
            .map_or_else(|| "—".to_owned(), |s| format!("{:.1}%", s * 100.0));
        let mut out = String::new();
        let _ = writeln!(
            out,
            "pool: {} eligible of {} in the window; withheld {} first-party-only ({share} of the window), \
             {} without analytics consent, {} without coach sharing",
            self.eligible,
            self.in_window,
            self.withheld.first_party_only,
            self.withheld.no_analytics_consent,
            self.withheld.coach_sharing_withheld,
        );
        if self.first_party_only_majority() {
            let _ = writeln!(
                out,
                "warning: most of the window judged replies built from first-party-only provider data, \
                 which no grader outside the conversation sees. The rates hold for the replies left, \
                 which read little or no provider data, and say little about the athlete-data layer; \
                 measure that layer with the seeded fabrications (claim_catch_rate)."
            );
        }
        out
    }
}

/// One row of a grader sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetRow {
    /// The item id.
    pub item_id: String,
    /// The athlete's pseudonym.
    pub athlete: String,
    /// UTC date the claim was made, `YYYY-MM-DD`.
    pub claimed_on: String,
    /// The athlete's locale.
    pub locale: String,
    /// The claim as extracted.
    pub claim: String,
}

/// One grader's sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradingSheet {
    /// 1-based sheet number.
    pub number: usize,
    /// Its rows, by item id.
    pub rows: Vec<SheetRow>,
}

impl GradingSheet {
    /// The sheet as CSV, header first, `grade` and `note` blank, the claim
    /// marked as text where a spreadsheet would read it as a formula.
    #[must_use]
    pub fn to_csv(&self) -> String {
        let mut out = record(&SHEET_HEADER);
        for row in &self.rows {
            out.push_str(&record(&[
                &row.item_id,
                &row.athlete,
                &row.claimed_on,
                &row.locale,
                &spreadsheet_text(&row.claim),
                "",
                "",
            ]));
        }
        out
    }
}

/// An export: the key for the operator, a sheet per grader.
#[derive(Debug, Clone)]
pub struct GradingExport {
    /// The operator's key.
    pub key: GradingKey,
    /// The grader sheets, numbered from 1.
    pub sheets: Vec<GradingSheet>,
}

/// Zero-padded label `prefix` + `n`, at least `width` digits.
fn label(prefix: char, n: usize, width: usize) -> String {
    format!("{prefix}{n:0width$}")
}

/// Which sheets each sampled position goes to: a seeded `overlap` of them to
/// every sheet, the rest dealt round the sheets in turn.
fn sheet_assignment(len: usize, overlap: usize, rng: &mut SplitMix64) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..len).collect();
    shuffle(&mut order, rng);
    let mut sheets = vec![Vec::new(); len];
    for (rank, &position) in order.iter().enumerate() {
        sheets[position] = if rank < overlap {
            (1..=GRADER_SHEETS).collect()
        } else {
            vec![(rank - overlap) % GRADER_SHEETS + 1]
        };
    }
    sheets
}

/// How many candidates each stratum holds.
fn tally(candidates: &[GradingCandidate]) -> BTreeMap<Stratum, usize> {
    let mut counts: BTreeMap<Stratum, usize> = BTreeMap::new();
    for candidate in candidates {
        *counts.entry(Stratum::of(candidate)).or_default() += 1;
    }
    counts
}

/// Per-stratum pool and sample counts, in stratum order. Every sampled
/// stratum is a pool stratum, so the pool's strata are the whole list.
fn stratum_counts(
    pool: BTreeMap<Stratum, usize>,
    sample: &BTreeMap<Stratum, usize>,
) -> Vec<StratumCount> {
    pool.into_iter()
        .map(|(stratum, pool)| StratumCount {
            sample: sample.get(&stratum).copied().unwrap_or(0),
            stratum,
            pool,
        })
        .collect()
}

/// Draw the sample from `pool` and build the key and the grader sheets.
#[must_use]
pub fn build_export(pool: GradingPool, spec: &ExportSpec) -> GradingExport {
    let GradingPool {
        window_start,
        window_days,
        in_window,
        withheld,
        eligible,
        candidates,
    } = pool;
    let pool_read = candidates.len();
    let pool_counts = tally(&candidates);
    let sample = stratified_sample(candidates, spec.size, spec.seed);
    let strata = stratum_counts(pool_counts, &tally(&sample));

    // A second stream for the overlap, so changing the overlap never changes
    // which claims were drawn.
    let mut rng = SplitMix64(spec.seed.rotate_left(32));
    let overlap = spec.overlap.min(sample.len());
    let assignment = sheet_assignment(sample.len(), overlap, &mut rng);

    let id_width = sample.len().to_string().len().max(3);
    let mut athletes: BTreeMap<String, String> = BTreeMap::new();
    let mut items = Vec::with_capacity(sample.len());
    for (position, (candidate, sheets)) in sample.into_iter().zip(assignment).enumerate() {
        let next = athletes.len() + 1;
        let athlete = athletes
            .entry(candidate.verdict.user_id.clone())
            .or_insert_with(|| label('A', next, 2))
            .clone();
        let verdict = candidate.verdict;
        items.push(KeyItem {
            item_id: label('C', position + 1, id_width),
            athlete,
            sheets,
            verdict_id: verdict.id,
            user_id: verdict.user_id,
            agent_id: verdict.agent_id,
            conversation_id: verdict.conversation_id,
            message_id: verdict.message_id,
            claimed_at: verdict.created_at,
            locale: candidate.locale,
            claim_text: verdict.claim_text,
            category: verdict.category,
            status: verdict.status,
            layer: verdict.layer_fired,
            flagged: flags(verdict.status),
        });
    }

    let sheets = (1..=GRADER_SHEETS)
        .map(|number| GradingSheet {
            number,
            rows: items
                .iter()
                .filter(|item| item.sheets.contains(&number))
                .map(|item| SheetRow {
                    item_id: item.item_id.clone(),
                    athlete: item.athlete.clone(),
                    claimed_on: item.claimed_at.format("%Y-%m-%d").to_string(),
                    locale: item.locale.clone(),
                    claim: item.claim_text.clone(),
                })
                .collect(),
        })
        .collect();

    GradingExport {
        key: GradingKey {
            tenant_id: spec.tenant_id.clone(),
            seed: spec.seed,
            window_start,
            window_days,
            in_window,
            withheld,
            eligible,
            pool_read,
            sample_size: items.len(),
            overlap,
            strata,
            items,
        },
        sheets,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use chrono::TimeZone;
    use pierre_core::transport::TransportPolicy;
    use pierre_memory::claims::{ClaimVerdict, EvidenceStrength};

    use super::*;
    use crate::grading_csv::parse;

    fn candidate(n: usize, layer: VerdictLayer, locale: &str, user: &str) -> GradingCandidate {
        GradingCandidate {
            verdict: ClaimVerdict {
                id: format!("v{n}"),
                tenant_id: "t".to_owned(),
                user_id: user.to_owned(),
                agent_id: Some("agent-1".to_owned()),
                conversation_id: Some("conv".to_owned()),
                message_id: Some(format!("m{n}")),
                claim_text: format!("Claim number {n}, with a comma."),
                category: ClaimCategory::AthleteData,
                status: if n.is_multiple_of(3) {
                    ClaimStatus::Contradicted
                } else {
                    ClaimStatus::Unverifiable
                },
                evidence_strength: EvidenceStrength::None,
                confidence: 0.5,
                layer_fired: layer,
                explanation: Some("the checker's reasoning".to_owned()),
                evidence_refs: None,
                created_at: Utc.with_ymd_and_hms(2026, 10, 1, 9, 30, 0).unwrap(),
                disposition: None,
                disposition_reason: None,
                disposition_note: None,
                disposed_by: None,
                disposed_at: None,
                transport_policy: TransportPolicy::default(),
            },
            locale: locale.to_owned(),
        }
    }

    /// 600 athlete-data verdicts in French, 300 evidence in English, 100
    /// judge in French: a 60/30/10 split.
    fn skewed_pool() -> Vec<GradingCandidate> {
        (0..1000)
            .map(|n| {
                let (layer, locale) = match n % 10 {
                    0..=5 => (VerdictLayer::AthleteData, "fr"),
                    6..=8 => (VerdictLayer::Evidence, "en"),
                    _ => (VerdictLayer::Judge, "fr"),
                };
                candidate(n, layer, locale, &format!("user-{}", n % 37))
            })
            .collect()
    }

    fn pool(candidates: Vec<GradingCandidate>) -> GradingPool {
        let eligible = i64::try_from(candidates.len()).unwrap();
        GradingPool {
            window_start: Utc.with_ymd_and_hms(2026, 9, 12, 0, 0, 0).unwrap(),
            window_days: 30,
            in_window: eligible + 7,
            withheld: GradingWithheld {
                first_party_only: 1,
                no_analytics_consent: 4,
                coach_sharing_withheld: 2,
            },
            eligible,
            candidates,
        }
    }

    fn spec(size: usize, overlap: usize, seed: u64) -> ExportSpec {
        ExportSpec {
            tenant_id: "t".to_owned(),
            size,
            overlap,
            seed,
        }
    }

    #[test]
    fn allocation_is_proportional_and_exact() {
        assert_eq!(allocate(&[600, 300, 100], 200), vec![120, 60, 20]);
        // 1/3 each of 10: the leftover seat goes to the earliest stratum.
        assert_eq!(allocate(&[3, 3, 3], 7), vec![3, 2, 2]);
        // Largest remainder: 5 * 7/10 = 3.5, 5 * 2/10 = 1.0, 5 * 1/10 = 0.5.
        assert_eq!(allocate(&[7, 2, 1], 5), vec![4, 1, 0]);
    }

    #[test]
    fn allocation_never_exceeds_a_stratum_or_the_pool() {
        assert_eq!(allocate(&[2, 1], 10), vec![2, 1]);
        assert_eq!(allocate(&[], 10), Vec::<usize>::new());
        assert_eq!(allocate(&[0, 0], 4), vec![0, 0]);
    }

    #[test]
    fn the_sample_mirrors_the_pool_by_stratum() {
        let sample = stratified_sample(skewed_pool(), 200, 42);
        assert_eq!(sample.len(), 200);
        let count = |layer: VerdictLayer| {
            sample
                .iter()
                .filter(|c| c.verdict.layer_fired == layer)
                .count()
        };
        assert_eq!(count(VerdictLayer::AthleteData), 120);
        assert_eq!(count(VerdictLayer::Evidence), 60);
        assert_eq!(count(VerdictLayer::Judge), 20);
        // No verdict drawn twice.
        let ids: BTreeSet<&str> = sample.iter().map(|c| c.verdict.id.as_str()).collect();
        assert_eq!(ids.len(), 200);
    }

    #[test]
    fn a_seed_draws_the_same_sample_and_another_seed_a_different_one() {
        let ids = |seed| -> Vec<String> {
            stratified_sample(skewed_pool(), 50, seed)
                .into_iter()
                .map(|c| c.verdict.id)
                .collect()
        };
        assert_eq!(ids(7), ids(7));
        assert_ne!(ids(7), ids(8));
    }

    #[test]
    fn a_small_pool_is_taken_whole() {
        let small: Vec<_> = (0..5)
            .map(|n| candidate(n, VerdictLayer::Evidence, "en", "u"))
            .collect();
        assert_eq!(stratified_sample(small, 200, 1).len(), 5);
    }

    #[test]
    fn the_export_splits_the_sample_with_a_shared_overlap() {
        let export = build_export(pool(skewed_pool()), &spec(200, 50, 3));
        let key = &export.key;
        assert_eq!(key.sample_size, 200);
        assert_eq!(key.overlap, 50);
        assert_eq!(key.pool_read, 1000);
        assert_eq!(key.withheld.no_analytics_consent, 4);

        let both = key.items.iter().filter(|i| i.sheets.len() == 2).count();
        assert_eq!(both, 50);
        let sheet = |n: usize| export.sheets.iter().find(|s| s.number == n).unwrap();
        // 150 single items dealt in turn: 75 each, plus the 50 shared.
        assert_eq!(sheet(1).rows.len(), 125);
        assert_eq!(sheet(2).rows.len(), 125);

        // Every item id is unique and the strata add up to the draw.
        let ids: BTreeSet<&str> = key.items.iter().map(|i| i.item_id.as_str()).collect();
        assert_eq!(ids.len(), 200);
        let drawn: usize = key.strata.iter().map(|s| s.sample).sum();
        assert_eq!(drawn, 200);
    }

    #[test]
    fn a_sheet_carries_the_claim_and_nothing_the_checker_decided() {
        let export = build_export(pool(skewed_pool()), &spec(20, 5, 9));
        let csv = export.sheets[0].to_csv();
        let rows = parse(&csv).unwrap();
        assert_eq!(rows[0], SHEET_HEADER);
        for row in &rows[1..] {
            assert_eq!(row.len(), SHEET_HEADER.len());
            assert!(row[4].starts_with("Claim number"), "{row:?}");
            assert_eq!(row[2], "2026-10-01");
            assert!(row[5].is_empty() && row[6].is_empty());
        }
        // Neither the verdict's status, its layer nor its reasoning is on it.
        for word in [
            "contradicted",
            "unverifiable",
            "athlete_data",
            "evidence",
            "judge",
            "reasoning",
        ] {
            assert!(!csv.contains(word), "{word} leaked onto the sheet");
        }
    }

    #[test]
    fn a_bulleted_claim_reaches_the_sheet_as_text_not_a_formula() {
        let sheet = GradingSheet {
            number: 1,
            rows: vec![SheetRow {
                item_id: "C001".to_owned(),
                athlete: "A01".to_owned(),
                claimed_on: "2026-10-01".to_owned(),
                locale: "en".to_owned(),
                claim: "- You ran 42 km this week.".to_owned(),
            }],
        };
        let rows = parse(&sheet.to_csv()).unwrap();
        assert_eq!(rows[1][4], "'- You ran 42 km this week.");
    }

    #[test]
    fn an_athlete_keeps_one_pseudonym_across_items() {
        let export = build_export(pool(skewed_pool()), &spec(200, 0, 11));
        let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
        for item in &export.key.items {
            let pseudonym = seen.entry(&item.user_id).or_insert(&item.athlete);
            assert_eq!(*pseudonym, item.athlete);
        }
        let pseudonyms: BTreeSet<&str> = seen.values().copied().collect();
        assert_eq!(pseudonyms.len(), seen.len());
    }

    #[test]
    fn coverage_states_the_first_party_only_share_and_warns_on_a_majority() {
        let export = build_export(pool(skewed_pool()), &spec(20, 5, 9));
        let coverage = export.key.coverage();
        assert_eq!(coverage.in_window, 1007);
        assert_eq!(coverage.eligible, 1000);
        assert!(!coverage.first_party_only_majority());
        let text = coverage.render();
        assert!(
            text.contains("withheld 1 first-party-only (0.1% of the window)"),
            "{text}"
        );
        assert!(!text.contains("warning:"), "{text}");

        let mostly_first_party = PoolCoverage {
            in_window: 100,
            eligible: 40,
            withheld: GradingWithheld {
                first_party_only: 51,
                no_analytics_consent: 9,
                coach_sharing_withheld: 0,
            },
        };
        assert!(mostly_first_party.first_party_only_majority());
        assert!(mostly_first_party
            .render()
            .contains("warning: most of the window"));

        // Exactly half is not a majority, and an empty window has no share.
        let half = PoolCoverage {
            in_window: 100,
            eligible: 50,
            withheld: GradingWithheld {
                first_party_only: 50,
                ..GradingWithheld::default()
            },
        };
        assert!(!half.first_party_only_majority());
        let empty = PoolCoverage {
            in_window: 0,
            eligible: 0,
            withheld: GradingWithheld::default(),
        };
        assert_eq!(empty.first_party_only_share(), None);
        assert!(empty.render().contains("(— of the window)"));
    }

    #[test]
    fn the_key_records_the_verdict_behind_each_item() {
        let export = build_export(pool(skewed_pool()), &spec(30, 10, 5));
        for item in &export.key.items {
            assert_eq!(item.flagged, item.status == ClaimStatus::Contradicted);
            assert!(item.verdict_id.starts_with('v'));
        }
        // The overlap never exceeds the sample.
        let tiny = build_export(pool(skewed_pool()), &spec(4, 10, 5));
        assert_eq!(tiny.key.overlap, 4);
        assert!(tiny.key.items.iter().all(|i| i.sheets == [1, 2]));
    }
}
