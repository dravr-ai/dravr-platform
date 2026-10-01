// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The billing vocabulary both plan pages print — tiers, plan rows, quota counters and statuses as corpus keys
// ABOUTME: A constants module cannot translate, so it names the keys and each client resolves them with its own t()

import type { PlanTier, PlanView } from '@pierre/shared-types';

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
  return ownKey(PLAN_TIER_LABEL_KEY, tier);
}

/**
 * The key `table` holds for `value`, or `null`. Own keys only: `in` would also
 * accept `toString` and the other names every object inherits.
 */
function ownKey(table: Readonly<Record<string, string>>, value: string): string | null {
  return Object.prototype.hasOwnProperty.call(table, value) ? table[value] : null;
}

/**
 * The rows of a plan's comparison card whose value is a cap, each as the
 * corpus key naming it and the plan field holding it. Both plan pages print
 * these in this order, then the plan's included usage.
 */
export const PLAN_LIMIT_ROWS: ReadonlyArray<{
  labelKey: string;
  field: keyof Pick<PlanView, 'daily_messages' | 'daily_tokens' | 'max_active_agents' | 'daily_tool_calls'>;
}> = [
  { labelKey: 'plan.limits.messagesPerDay', field: 'daily_messages' },
  { labelKey: 'plan.limits.tokensPerDay', field: 'daily_tokens' },
  { labelKey: 'plan.limits.agents', field: 'max_active_agents' },
  { labelKey: 'plan.limits.toolCallsPerDay', field: 'daily_tool_calls' },
];

/** The corpus key naming a plan's included-usage row. */
export const PLAN_INCLUDED_USAGE_LABEL_KEY = 'plan.limits.includedUsage';

/** The corpus key writing a plan's monthly price, `{{amount}}/mo` in English. */
export const PLAN_PER_MONTH_KEY = 'plan.limits.perMonth';

/** The corpus key a plan row reads when the plan sets no cap on it. */
export const PLAN_UNLIMITED_KEY = 'shell.billingUnlimited';

/** The corpus key an unlimited plan's included-usage row reads in place of a price. */
export const PLAN_INCLUDED_CUSTOM_KEY = 'shell.billingIncludedCustom';

/**
 * The corpus key labelling each quota counter `GET /api/users/me/quota`
 * reports, keyed by its wire `counter_type`.
 */
export const QUOTA_COUNTER_LABEL_KEY: Readonly<Record<string, string>> = {
  daily_messages: 'usage.counter.dailyMessages',
  daily_tokens: 'usage.counter.dailyTokens',
  weekly_tokens: 'usage.counter.weeklyTokens',
  daily_tool_calls: 'usage.counter.dailyToolCalls',
  daily_conversations: 'usage.counter.dailyConversations',
  active_coaches: 'usage.counter.activeCoaches',
};

/**
 * The corpus key labelling `counterType`, or `null` for a counter the table
 * does not know — a counter added server-side first reaches the clients as its
 * own slug, which the caller prints.
 */
export function quotaCounterLabelKey(counterType: string): string | null {
  return ownKey(QUOTA_COUNTER_LABEL_KEY, counterType);
}

/** The corpus key naming each subscription status the billing provider reports. */
export const SUBSCRIPTION_STATUS_LABEL_KEY: Readonly<Record<string, string>> = {
  active: 'plan.subscriptionStatus.active',
  trialing: 'plan.subscriptionStatus.trialing',
  past_due: 'plan.subscriptionStatus.pastDue',
  unpaid: 'plan.subscriptionStatus.unpaid',
  canceled: 'plan.subscriptionStatus.canceled',
  incomplete: 'plan.subscriptionStatus.incomplete',
  incomplete_expired: 'plan.subscriptionStatus.incompleteExpired',
  paused: 'plan.subscriptionStatus.paused',
};

/** The corpus key naming subscription `status`, or `null` for one the table does not know. */
export function subscriptionStatusLabelKey(status: string): string | null {
  return ownKey(SUBSCRIPTION_STATUS_LABEL_KEY, status);
}

/** The corpus key naming each invoice status the billing provider reports. */
export const INVOICE_STATUS_LABEL_KEY: Readonly<Record<string, string>> = {
  draft: 'plan.invoiceStatus.draft',
  open: 'plan.invoiceStatus.open',
  paid: 'plan.invoiceStatus.paid',
  uncollectible: 'plan.invoiceStatus.uncollectible',
  void: 'plan.invoiceStatus.void',
};

/** The corpus key naming invoice `status`, or `null` for one the table does not know. */
export function invoiceStatusLabelKey(status: string): string | null {
  return ownKey(INVOICE_STATUS_LABEL_KEY, status);
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

/** What a billing label reads as when the payload carries no value for it. */
const BILLING_MISSING_VALUE = '—';

/**
 * `value` as the athlete reads it: the word `keyOf` names for it, translated
 * with the caller's `t`, or the server's own slug, spaced, for a value the
 * table does not know yet — a status or counter added server-side reaches the
 * clients before its key does, and a slug is more honest than a raw key.
 * A value the payload left out reads as a dash: the label is printed during
 * render, so throwing here unmounts the whole page into the ErrorBoundary.
 */
export function billingLabel(
  value: string | null | undefined,
  keyOf: (value: string) => string | null,
  t: (key: string) => string,
): string {
  if (!value) return BILLING_MISSING_VALUE;
  const key = keyOf(value);
  return key === null ? value.replace(/_/g, ' ') : t(key);
}
