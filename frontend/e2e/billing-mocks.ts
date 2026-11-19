// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mocked billing API — subscription, quota, invoices, plans — the billing spec and the design sweep share
// ABOUTME: One payload per state (Starter fallback, a live subscription, a payment problem) so a capture and an assertion see the same page

import type { Page, Route } from '@playwright/test';

export interface BillingMockOptions {
  /** When true, GET /api/billing/subscription returns 404 (Starter fallback). */
  noSubscription?: boolean;
  /** Subscription lifecycle status when a subscription exists (default 'active'). */
  subscriptionStatus?: string;
  /** Override the quota counters returned by /api/users/me/quota. */
  quota?: {
    counters: Array<{
      counter_type: string;
      current: number;
      limit: number;
      warning: boolean;
      burst_zone: boolean;
      resets_at: string;
    }>;
  };
}

export async function setupBillingMocks(page: Page, opts: BillingMockOptions = {}) {
  const noSubscription = opts.noSubscription ?? true;
  const subscriptionStatus = opts.subscriptionStatus ?? 'active';

  await page.route('**/api/billing/subscription', async (route: Route) => {
    if (noSubscription) {
      await route.fulfill({
        status: 404,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'no subscription record yet' }),
      });
    } else {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          id: 'sub-row-1',
          tenant_id: 'tenant-1',
          user_id: 'user-123',
          provider: 'dummy',
          provider_customer_id: 'cus_test_abc123',
          provider_subscription_id: 'sub_test_abc123',
          status: subscriptionStatus,
          plan_tier: 'professional',
          current_period_start: '2026-04-01T00:00:00Z',
          current_period_end: '2026-05-01T00:00:00Z',
          cancel_at_period_end: false,
        }),
      });
    }
  });

  await page.route('**/api/users/me/quota', async (route: Route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(
        opts.quota ?? {
          counters: [
            {
              counter_type: 'daily_messages',
              current: 0,
              limit: 50,
              warning: false,
              burst_zone: false,
              resets_at: '2026-04-29T00:00:00Z',
            },
            {
              counter_type: 'daily_tokens',
              current: 0,
              limit: 500_000,
              warning: false,
              burst_zone: false,
              resets_at: '2026-04-29T00:00:00Z',
            },
            {
              counter_type: 'weekly_tokens',
              current: 0,
              limit: 2_500_000,
              warning: false,
              burst_zone: false,
              resets_at: '2026-05-04T00:00:00Z',
            },
            {
              counter_type: 'daily_tool_calls',
              current: 0,
              limit: 200,
              warning: false,
              burst_zone: false,
              resets_at: '2026-04-29T00:00:00Z',
            },
            {
              counter_type: 'daily_conversations',
              current: 0,
              limit: 10,
              warning: false,
              burst_zone: false,
              resets_at: '2026-04-29T00:00:00Z',
            },
            {
              counter_type: 'active_coaches',
              current: 0,
              limit: 3,
              warning: false,
              burst_zone: false,
              resets_at: '2026-04-29T00:00:00Z',
            },
          ],
        },
      ),
    });
  });

  await page.route('**/api/billing/invoices', async (route: Route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ invoices: [] }),
    });
  });

  await page.route('**/api/billing/plans', async (route: Route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        plans: [
          {
            tier: 'starter',
            label: 'Starter',
            unlimited: false,
            daily_messages: 50,
            daily_tokens: 1_000_000,
            monthly_tokens: 16_000_000,
            max_active_agents: 3,
            daily_tool_calls: 200,
            included_usd: null,
          },
          {
            tier: 'professional',
            label: 'Professional',
            unlimited: false,
            daily_messages: 500,
            daily_tokens: 10_000_000,
            monthly_tokens: 200_000_000,
            max_active_agents: 20,
            daily_tool_calls: 2_000,
            included_usd: 50,
          },
          {
            tier: 'enterprise',
            label: 'Enterprise',
            unlimited: true,
            daily_messages: 9_007_199_254_740_991,
            daily_tokens: 9_007_199_254_740_991,
            monthly_tokens: 9_007_199_254_740_991,
            max_active_agents: 9_007_199_254_740_991,
            daily_tool_calls: 9_007_199_254_740_991,
            included_usd: null,
          },
        ],
      }),
    });
  });
}
