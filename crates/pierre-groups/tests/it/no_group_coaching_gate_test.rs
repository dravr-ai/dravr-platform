// ABOUTME: The creation gate refuses a plan without group coaching and logs it as a usage limit
// ABOUTME: Drives refuses_group_coaching with the shared NoGroupCoachingTier fixture (carnet#1630)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::TenantId;
use pierre_core::usage_log;
use pierre_groups::creation_policy::{refuses_group_coaching, GROUP_COACHING_PLAN_LIMIT};
use pierre_test_support::group_tier::NoGroupCoachingTier;
use pierre_test_support::log_capture::{capture_target, drain};
use uuid::Uuid;

#[test]
fn a_plan_without_group_coaching_is_refused_and_logged() {
    let (events, _guard) = capture_target(usage_log::TARGET);
    let user_id = Uuid::new_v4();
    let tenant_id = TenantId::generate();

    assert!(
        refuses_group_coaching(&NoGroupCoachingTier, user_id, tenant_id, "telegram"),
        "a Disabled allowance refuses group coaching"
    );

    let lines = drain(&events);
    assert_eq!(lines.len(), 1, "one usage line per refusal: {lines:?}");
    let field = |name: &str| lines[0].get(name).map(String::as_str);
    assert_eq!(
        field("event"),
        Some(usage_log::LIMIT_REACHED),
        "the line is a refusal"
    );
    assert_eq!(
        field("user_id"),
        Some(user_id.to_string().as_str()),
        "the line names the refused user"
    );
    assert_eq!(
        field("tenant_id"),
        Some(tenant_id.to_string().as_str()),
        "the line names the tenant whose plan refused"
    );
    assert_eq!(
        field("limit_type"),
        Some(GROUP_COACHING_PLAN_LIMIT),
        "the line names the plan as the limit"
    );
    assert_eq!(field("current"), Some("0"), "no group was counted");
    assert_eq!(field("limit"), Some("0"), "the plan allows none");
    assert_eq!(
        field("surface"),
        Some("telegram"),
        "the line names where it was met"
    );
}
