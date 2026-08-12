// ABOUTME: Pins that detail auto-promotion is rationed by provider cost, never silently dropped
// ABOUTME: A scrape-backed provider enriches the newest few; an API provider still enriches all

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `get_activities` promotes small result sets to detailed by fetching each
//! activity's detail page. On an HTTP API that is milliseconds per activity;
//! on a headless-browser provider it is a full page navigation. A live
//! 2026-08-12 Telegram turn returned its list in 37s and then spent **3m41s**
//! scraping 30 detail pages one at a time — 4m37s total for an answer the
//! summary already supported.
//!
//! The fix rations rather than removes. Detail is not decoration on sciotte:
//! it carries HR streams, laps, and the real UTC start time that the
//! date-only list page lacks, so disabling it outright would trade latency
//! for a coach that no longer knows when a session happened. These tests pin
//! the two halves of that contract.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::config::fitness::{
    DEFAULT_ACTIVITY_DETAIL_THRESHOLD, EXPENSIVE_DETAIL_PROMOTION_BUDGET,
};
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::spi::ProviderCapabilities;

/// Resolve a provider's declared capabilities through the registry, the same
/// way `get_activities` decides its promotion budget.
fn declares_cheap_detail(registry: &ProviderRegistry, provider: &str) -> bool {
    registry
        .get_descriptor(provider)
        .unwrap_or_else(|| panic!("{provider} must be registered"))
        .capabilities()
        .contains(ProviderCapabilities::CHEAP_ACTIVITY_DETAIL)
}

#[test]
fn scrape_backed_providers_do_not_claim_cheap_detail() {
    let registry = ProviderRegistry::new();
    for provider in ["sciotte", "sciotte_garmin"] {
        assert!(
            !declares_cheap_detail(&registry, provider),
            "{provider} fetches detail through a headless browser — claiming it is \
             cheap reinstates the 3m41s N+1"
        );
    }
}

#[test]
fn api_providers_keep_unrationed_detail() {
    let registry = ProviderRegistry::new();
    for provider in ["strava", "whoop", "garmin", "intervals_icu"] {
        assert!(
            declares_cheap_detail(&registry, provider),
            "{provider} answers detail over HTTP; rationing it would lose HR, laps \
             and precise start times for no latency win"
        );
    }
}

#[test]
fn the_expensive_budget_is_a_ration_not_a_removal() {
    assert!(
        EXPENSIVE_DETAIL_PROMOTION_BUDGET > 0,
        "zero would drop detail entirely on scrape providers — the list page is \
         date-only, so the coach would lose real start times, HR and laps"
    );
    assert!(
        EXPENSIVE_DETAIL_PROMOTION_BUDGET < DEFAULT_ACTIVITY_DETAIL_THRESHOLD,
        "a budget at or above the promotion threshold rations nothing"
    );
}
