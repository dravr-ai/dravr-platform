// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The billing vocabulary both plan pages print — each tier as a corpus key, and the payment-problem statuses
// ABOUTME: A constants module cannot translate, so it names the keys and each client resolves them with its own t()

import type { PlanTier } from '@pierre/shared-types';

/** The corpus key each plan tier's name reads as. */
export const PLAN_TIER_LABEL_KEY: Record<PlanTier, string> = {
  starter: 'plan.starter',
  professional: 'plan.professional',
  enterprise: 'plan.enterprise',
};

/**
 * The corpus key naming `tier`, or `null` for a tier the table does not know —
 * the tier arrives as a string, so a slug added server-side first reaches the
 * clients as itself, and the caller prints it raw.
 */
export function planTierLabelKey(tier: string): string | null {
  // Own keys only: `in` would also accept `toString` and the other names the
  // table inherits.
  return Object.prototype.hasOwnProperty.call(PLAN_TIER_LABEL_KEY, tier)
    ? PLAN_TIER_LABEL_KEY[tier as PlanTier]
    : null;
}

/** Subscription statuses that mean the athlete must fix their payment to keep the plan. */
export const PAYMENT_PROBLEM_STATUSES: ReadonlySet<string> = new Set([
  'past_due',
  'unpaid',
  'incomplete',
  'incomplete_expired',
]);

/** Whether a subscription in `status` needs the athlete to update their payment method. */
export function hasPaymentProblem(status: string | null | undefined): boolean {
  return status != null && PAYMENT_PROBLEM_STATUSES.has(status);
}
