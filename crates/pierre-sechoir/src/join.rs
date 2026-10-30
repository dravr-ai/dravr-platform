// ABOUTME: Finds every pair of functions whose fingerprint sets overlap past a threshold
// ABOUTME: A prefix-filtered similarity join, so a fleet of thousands of functions avoids all-pairs work
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The join is the `AllPairs` algorithm (Bayardo et al., 2007). Each set's
//! fingerprints are ordered rarest first. Two sets that reach Jaccard `t` must
//! share at least one of the first `|x| - ceil(t·|x|) + 1` fingerprints of the
//! smaller set, so only those prefixes are indexed and probed, and a pair is
//! scored only when its sizes could still reach `t`.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::extract::Unit;
use crate::similarity::Similarity;

/// Two units and their Jaccard score. `left` is the smaller set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScoredPair {
    pub(crate) left: usize,
    pub(crate) right: usize,
    pub(crate) score: Similarity,
}

/// Every pair of units with Jaccard similarity at or above `threshold`.
pub(crate) fn similar_pairs(units: &[Unit], threshold: Similarity) -> Vec<ScoredPair> {
    let ranked = rank_by_rarity(units);
    let mut order: Vec<usize> = (0..units.len()).collect();
    order.sort_by_key(|&i| units[i].shape.prints.len());

    let mut index: HashMap<u64, Vec<usize>> = HashMap::new();
    let mut pairs = Vec::new();
    for &probe in &order {
        let size = ranked[probe].len();
        let prefix = &ranked[probe][..prefix_len(size, threshold)];
        let smallest = threshold.min_count(size);
        let mut seen = HashSet::new();
        for print in prefix {
            for &other in index.get(print).into_iter().flatten() {
                if ranked[other].len() >= smallest && seen.insert(other) {
                    let (shared, union) =
                        overlap(&units[probe].shape.prints, &units[other].shape.prints);
                    if threshold.admits(shared, union) {
                        let score = Similarity::ratio(shared, union);
                        pairs.push(ScoredPair {
                            left: other,
                            right: probe,
                            score,
                        });
                    }
                }
            }
        }
        for &print in prefix {
            index.entry(print).or_default().push(probe);
        }
    }
    pairs
}

/// How many of a set's rarest fingerprints a match must share at least one of.
fn prefix_len(size: usize, threshold: Similarity) -> usize {
    (size - threshold.min_count(size).min(size) + 1).min(size)
}

/// Each unit's fingerprints, rarest across all units first; ties by value so
/// every unit orders shared fingerprints the same way.
fn rank_by_rarity(units: &[Unit]) -> Vec<Vec<u64>> {
    let mut frequency: HashMap<u64, usize> = HashMap::new();
    for unit in units {
        for &print in &unit.shape.prints {
            *frequency.entry(print).or_default() += 1;
        }
    }
    units
        .iter()
        .map(|unit| {
            let mut prints = unit.shape.prints.clone();
            prints.sort_by_key(|print| (frequency[print], *print));
            prints
        })
        .collect()
}

/// Shared and union sizes of two sorted sets.
fn overlap(left: &[u64], right: &[u64]) -> (usize, usize) {
    let (mut i, mut j, mut shared) = (0, 0, 0);
    while i < left.len() && j < right.len() {
        match left[i].cmp(&right[j]) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => {
                shared += 1;
                i += 1;
                j += 1;
            }
        }
    }
    (shared, left.len() + right.len() - shared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize::Shape;

    fn unit(prints: &[u64]) -> Unit {
        Unit {
            repo: "r".to_owned(),
            file: "f.rs".to_owned(),
            name: "f".to_owned(),
            start_line: 1,
            end_line: 1,
            shape: Shape {
                prints: prints.to_vec(),
                nodes: prints.len(),
            },
        }
    }

    /// The pairs an all-pairs comparison would report, for checking the join.
    fn brute_force(units: &[Unit], threshold: Similarity) -> Vec<(usize, usize)> {
        let mut found = Vec::new();
        for a in 0..units.len() {
            for b in a + 1..units.len() {
                let (shared, union) = overlap(&units[a].shape.prints, &units[b].shape.prints);
                if threshold.admits(shared, union) {
                    found.push((a, b));
                }
            }
        }
        found
    }

    #[test]
    fn overlap_counts_shared_and_union() {
        assert_eq!(
            overlap(&[1, 2, 3, 4], &[2, 4, 6]),
            (2, 5),
            "two shared of five distinct"
        );
    }

    #[test]
    fn join_finds_exactly_what_all_pairs_finds() {
        // Sets of varied size and overlap, generated deterministically.
        let units: Vec<Unit> = (0..60_u64)
            .map(|seed| {
                let start = seed % 7;
                let len = 8 + seed % 13;
                let mut prints: Vec<u64> = (start..start + len).collect();
                if seed % 5 == 0 {
                    prints.push(1000 + seed);
                }
                prints.sort_unstable();
                unit(&prints)
            })
            .collect();
        for text in ["0.5", "0.7", "0.82", "1"] {
            let threshold: Similarity = text.parse().unwrap();
            let mut joined: Vec<(usize, usize)> = similar_pairs(&units, threshold)
                .into_iter()
                .map(|p| (p.left.min(p.right), p.left.max(p.right)))
                .collect();
            joined.sort_unstable();
            assert_eq!(
                joined,
                brute_force(&units, threshold),
                "join must equal all-pairs at {text}"
            );
        }
    }

    #[test]
    fn identical_sets_score_one() {
        let units = [unit(&[1, 2, 3]), unit(&[1, 2, 3]), unit(&[7, 8, 9])];
        let pairs = similar_pairs(&units, "0.9".parse().unwrap());
        assert_eq!(pairs.len(), 1, "only the identical pair clears 0.9");
        assert_eq!(pairs[0].score, Similarity::ONE, "identical sets score one");
    }
}
