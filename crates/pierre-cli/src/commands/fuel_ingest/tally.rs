// ABOUTME: What one `pierre-cli fuel-ingest` run did, counted, and the verdict the job-failure alert reads from it
// ABOUTME: A run fails when it read labels and not one was legible, when every candidate failed, or no brand listed (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use tracing::info;

/// What the run did, counted.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Tally {
    pub brands_failed: usize,
    pub products_listed: usize,
    pub products_seen: usize,
    pub candidates: usize,
    /// Reads filed as a new product's proposal.
    pub proposed: usize,
    /// Reads filed as an update of a catalogued product whose facts changed.
    pub proposed_update: usize,
    /// Reads that found no readable panel; nothing filed.
    pub illegible: usize,
    /// Reads whose product a live proposal already holds; nothing filed.
    pub duplicate: usize,
    /// Reads of a catalogued product whose facts are unchanged; nothing filed.
    pub unchanged: usize,
    /// Answers that were not the prompt's object; recorded, nothing filed.
    pub parse_failed: usize,
    /// Candidates whose read hash is on file: no call.
    pub skipped_already_read: usize,
    /// Candidates whose listing a live proposal holds: no call.
    pub skipped_listing_held: usize,
    pub skipped_no_photo: usize,
    /// Candidates whose product page showed a nutrition panel, sent first.
    pub panel_sent: usize,
    /// Candidates whose product page showed no panel, one that could not be
    /// downloaded, or one flavour's panel for a listing of several: the
    /// store photos were sent instead.
    pub panel_missing: usize,
    /// Candidates whose product page could not be fetched: the store photos
    /// were sent instead.
    pub page_failed: usize,
    pub llm_calls: usize,
    /// Prompt tokens the answers were billed for, as the providers report.
    pub prompt_tokens: u64,
    /// Completion tokens the answers were billed for.
    pub completion_tokens: u64,
    pub discarded_no_vision: usize,
    /// Reads that errored on every tier (not recorded: retried next run).
    pub read_failed: usize,
    /// Reads that filed nothing, answered by a fallback tier on another model
    /// than the hashed one (not recorded: retried next run).
    pub unrecorded_fallback: usize,
    /// Reads that found a panel but that a concurrent run recorded first: its
    /// outcome stands, this one files nothing. One that found no panel is
    /// counted as `illegible` or `parse_failed` instead.
    pub raced: usize,
    /// Candidates that failed outside the read (download, database).
    pub failed: usize,
}

impl Tally {
    /// Log the summary the job-failure alert's reader starts from.
    pub(super) fn log(&self, dry_run: bool, brands: usize) {
        info!(
            dry_run,
            brands,
            brands_failed = self.brands_failed,
            listed = self.products_listed,
            products_seen = self.products_seen,
            candidates = self.candidates,
            proposed = self.proposed,
            proposed_update = self.proposed_update,
            illegible = self.illegible,
            duplicate = self.duplicate,
            unchanged = self.unchanged,
            parse_failed = self.parse_failed,
            skipped_already_read = self.skipped_already_read,
            skipped_listing_held = self.skipped_listing_held,
            skipped_no_photo = self.skipped_no_photo,
            panel_sent = self.panel_sent,
            panel_missing = self.panel_missing,
            page_failed = self.page_failed,
            llm_calls = self.llm_calls,
            prompt_tokens = self.prompt_tokens,
            completion_tokens = self.completion_tokens,
            discarded_no_vision = self.discarded_no_vision,
            read_failed = self.read_failed,
            unrecorded_fallback = self.unrecorded_fallback,
            raced = self.raced,
            failed = self.failed,
            "fuel ingestion finished"
        );
    }

    /// Reads that found a panel: a proposal, a duplicate or an unchanged
    /// product. A read that found a panel but that a concurrent run recorded
    /// first counts too: this run's call came back legible.
    const fn legible(&self) -> usize {
        self.proposed + self.proposed_update + self.duplicate + self.unchanged + self.raced
    }

    /// The run's verdict: an error when labels were sent and not one came
    /// back legible (every read illegible, unparseable, errored or blind),
    /// when every candidate failed before or after its read (the database
    /// down, every photo download refused), or when no brand could be listed.
    /// A run that sent nothing, every read on file or every listing held, is
    /// no failure.
    pub(super) fn verdict(&self, brands: usize) -> AppResult<()> {
        if self.llm_calls > 0 && self.legible() == 0 {
            return Err(AppError::external_service(
                "llm",
                format!(
                    "no label read was legible ({} sent: {} illegible, {} unparseable, {} errored, {} discarded for no vision)",
                    self.llm_calls,
                    self.illegible,
                    self.parse_failed,
                    self.read_failed,
                    self.discarded_no_vision
                ),
            ));
        }
        if self.candidates > 0 && self.failed == self.candidates {
            return Err(AppError::external_service(
                "fuel-ingest",
                format!(
                    "every label candidate failed outside the read ({} of {}: downloads or the database)",
                    self.failed, self.candidates
                ),
            ));
        }
        if brands > 0 && self.brands_failed == brands {
            return Err(AppError::external_service(
                "shopify",
                "no brand storefront could be listed",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_fails_when_no_read_was_legible_or_no_brand_listed() {
        let mut tally = Tally {
            llm_calls: 3,
            read_failed: 2,
            discarded_no_vision: 1,
            ..Tally::default()
        };
        assert!(
            tally.verdict(1).is_err(),
            "bug: reads that all errored or went blind must fail the run"
        );
        tally.proposed = 1;
        tally.read_failed = 1;
        assert!(
            tally.verdict(1).is_ok(),
            "bug: one proposal among failed reads must pass the run"
        );
        // A download or database failure is not a failed read.
        assert!(
            Tally {
                candidates: 5,
                llm_calls: 1,
                proposed: 1,
                failed: 4,
                ..Tally::default()
            }
            .verdict(1)
            .is_ok(),
            "bug: a candidate that failed outside the read must not fail a run that filed"
        );
        // But a run where every candidate failed (the database down: even the
        // dedupe lookup errors, so no read is ever sent) filed nothing.
        assert!(
            Tally {
                candidates: 4,
                failed: 4,
                ..Tally::default()
            }
            .verdict(1)
            .is_err(),
            "bug: a run where every candidate failed must fail"
        );
        // Reads on file and held listings are no call, and no failure: the
        // run after an all-illegible one sends nothing and passes.
        assert!(
            Tally {
                candidates: 33,
                skipped_already_read: 31,
                skipped_listing_held: 2,
                ..Tally::default()
            }
            .verdict(1)
            .is_ok(),
            "bug: a run that sent nothing must pass"
        );
        // Every new read illegible (Näak's front-of-pack photos, 2026-10-29)
        // fails the run, so the job-failure alert fires.
        let all_illegible = Tally {
            candidates: 33,
            llm_calls: 33,
            illegible: 32,
            read_failed: 1,
            ..Tally::default()
        };
        let refused = all_illegible.verdict(1).unwrap_err();
        assert!(
            refused.message.contains("32 illegible"),
            "bug: the verdict must count the illegible reads: {refused}"
        );
        // As does a mix of illegible and unparseable answers.
        assert!(
            Tally {
                candidates: 2,
                llm_calls: 2,
                illegible: 1,
                parse_failed: 1,
                ..Tally::default()
            }
            .verdict(1)
            .is_err(),
            "bug: illegible and unparseable reads alone must fail the run"
        );
        // One legible read among them is enough: a duplicate, an unchanged
        // product, or a read a concurrent run recorded first.
        for legible in [
            Tally {
                duplicate: 1,
                ..Tally::default()
            },
            Tally {
                unchanged: 1,
                ..Tally::default()
            },
            Tally {
                proposed_update: 1,
                ..Tally::default()
            },
            Tally {
                raced: 1,
                ..Tally::default()
            },
        ] {
            let tally = Tally {
                candidates: 3,
                llm_calls: 3,
                illegible: 2,
                ..legible
            };
            assert!(
                tally.verdict(1).is_ok(),
                "bug: one legible read must pass the run: {tally:?}"
            );
        }
        assert!(
            Tally {
                brands_failed: 2,
                ..Tally::default()
            }
            .verdict(2)
            .is_err(),
            "bug: a run that listed no brand must fail"
        );
    }
}
