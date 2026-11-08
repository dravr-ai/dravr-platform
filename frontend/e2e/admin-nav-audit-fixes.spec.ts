// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E coverage for the 2026-05-07 admin nav audit fixes
// ABOUTME: Locks every fix from the audit gist 56c1c1d7b54bf16c9d140f54f3210c94

import { test, expect } from "@playwright/test";
import {
  setupAndLoginAsAdmin,
  setupAndLoginAsSuperAdmin,
  navigateToTab,
  TOKENS_TAB_LABEL,
} from "./test-helpers";

test.describe("Admin nav audit fixes (gist 56c1c1d7)", () => {
  test("Service Tokens page renders without 500 (audit #19)", async ({
    page,
  }) => {
    await page.route("**/api/admin/tokens**", (r) =>
      r.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          success: true,
          message: "Retrieved 0 admin tokens",
          data: { count: 0, tokens: [] },
        }),
      }),
    );
    await setupAndLoginAsAdmin(page);
    await navigateToTab(page, TOKENS_TAB_LABEL);
    await expect(
      page.getByRole("heading", { name: TOKENS_TAB_LABEL, exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: /Create API Token/ }),
    ).toBeVisible();
  });

  // The tiers are counted by the server (engagement.rs
  // the_tiers_are_cumulative_and_a_user_never_seen_is_dormant); the screen shows
  // what `/api/admin/engagement` states, in the order DAU, WAU, MAU.
  const engagement = (body: object) => (r: import("@playwright/test").Route) =>
    r.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(body),
    });

  test("Engagement page enforces DAU ≤ WAU ≤ MAU (audit #17)", async ({
    page,
  }) => {
    await setupAndLoginAsAdmin(page);
    // Override AFTER setup so our handler runs first (Playwright evaluates
    // routes in reverse registration order).
    await page.route(
      "**/api/admin/engagement**",
      engagement({
        scope: { kind: "tenant", tenant_id: "t1" },
        users: { total: 5, daily: 2, weekly: 3, monthly: 4, dormant: 1 },
        total_agents: 0,
        published_agents: 0,
        leaderboard: [],
      }),
    );
    await navigateToTab(page, "Engagement");

    // The stat card itself, not an ancestor that holds every card: an ancestor
    // made all three read the first card's figure, so the check passed as 2 ≤ 2 ≤ 2.
    const card = (label: string) =>
      page.locator(".stat-card-dark", { hasText: label }).locator(".text-2xl");
    const dau = Number(await card("Active in the last 24h").innerText());
    const wau = Number(await card("Active in the last 7d").innerText());
    const mau = Number(await card("Active in the last 30d").innerText());
    expect([dau, wau, mau]).toEqual([2, 3, 4]);
  });

  test("Engagement Agent Leaderboard ranks by uses, not prompt size (audit #17)", async ({
    page,
  }) => {
    await setupAndLoginAsAdmin(page);
    await page.route(
      "**/api/admin/engagement**",
      engagement({
        scope: { kind: "tenant", tenant_id: "t1" },
        users: { total: 0, daily: 0, weekly: 0, monthly: 0, dormant: 0 },
        total_agents: 1,
        published_agents: 1,
        leaderboard: [
          {
            id: "coach-1",
            title: "Marathon Coach",
            category: "training",
            use_count: 7,
          },
        ],
      }),
    );
    await navigateToTab(page, "Engagement");
    await expect(
      page.getByRole("columnheader", { name: "Uses", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("cell", { name: "Marathon Coach" }),
    ).toBeVisible();
    await expect(
      page.getByRole("columnheader", { name: /Prompt Size/i }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("columnheader", { name: /^Tokens Used$/ }),
    ).toHaveCount(0);
  });

  test("Notifications page renders only one h1 (audit #18)", async ({
    page,
  }) => {
    await setupAndLoginAsAdmin(page);
    await page.evaluate(() => {
      window.location.hash = "notifications";
    });
    await expect(
      page.locator("main h1", { hasText: "Notifications" }),
    ).toBeVisible();
    const h1Count = await page
      .locator("main h1", { hasText: "Notifications" })
      .count();
    expect(h1Count).toBe(1);
  });

  test("Auto-approve toggle has only one description paragraph (audit #7)", async ({
    page,
  }) => {
    await page.route("**/api/admin/auto-approval", (r) =>
      r.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          enabled: false,
          description: "When enabled, all new registrations are auto-approved.",
        }),
      }),
    );
    await setupAndLoginAsAdmin(page);
    await navigateToTab(page, "Platform Settings");
    const heading = page.getByRole("heading", {
      name: "Auto-Approve Registrations",
    });
    await expect(heading).toBeVisible();
    // The card containing the toggle should only have ONE <p> describing the
    // setting now (audit flagged two stacked paragraphs).
    const card = heading.locator(
      'xpath=ancestor::div[contains(@class,"flex-1")][1]',
    );
    expect(await card.locator("p").count()).toBe(1);
  });

  test("Nav click updates URL hash (audit cross-cutting: deep links)", async ({
    page,
  }) => {
    await setupAndLoginAsAdmin(page);
    await navigateToTab(page, "Agents");
    await expect(page).toHaveURL(/#agents$/);
    await navigateToTab(page, "Activity");
    await expect(page).toHaveURL(/#activity$/);
    // Reload preserves the section.
    await page.reload();
    await expect(page).toHaveURL(/#activity$/);
  });

  test('the Agent Store shows "Published / Total" when counts differ (audit #3)', async ({
    page,
  }) => {
    await setupAndLoginAsAdmin(page);
    await page.route("**/api/admin/store/stats", (r) =>
      r.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          pending_count: 0,
          published_count: 20,
          rejected_count: 0,
          total_installs: 0,
          rejection_rate: 0,
        }),
      }),
    );
    await page.route("**/api/admin/agents**", (r) =>
      r.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          agents: Array.from({ length: 21 }, (_, i) => ({
            id: `coach-${i}`,
            title: `Coach ${i}`,
            category: "training",
            token_count: 100 + i,
            use_count: 0,
            author_user_id: "user-123",
            status: "active",
          })),
        }),
      }),
    );
    await navigateToTab(page, "Agent Store");
    await expect(page.getByText("Published / Total Agents")).toBeVisible();
    await expect(page.getByText(/\/ 21$/)).toBeVisible();
  });

  test("Harness Config window tokens has explicit max attribute (audit #9)", async ({
    page,
  }) => {
    await page.route("**/api/admin/settings/harness", (r) =>
      r.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          config: {
            schema_version: 3,
            compaction: {
              window_tokens: 1_000_000,
              warn_threshold: 0.0896,
              emergency_threshold: 0.1216,
              summarize_oldest_n: 12,
              sliding_drop_n: 4,
              max_messages: 40,
            },
            memory: { dedup_candidate_limit: 50 },
            guardrails: {
              blocked_topics: [],
              locales: { en: { disclaimer_triggers: [] } },
            },
          },
          source: "default",
          updated_at: null,
          in_force: {
            instance_id: "local:1",
            loaded_at: "2026-11-06T00:00:00Z",
            in_sync: true,
          },
        }),
      }),
    );
    // The harness settings are a super admin's: a console admin holds no ViewConfiguration.
    await setupAndLoginAsSuperAdmin(page);
    await navigateToTab(page, "Harness Config");
    const windowTokens = page.locator('input[type="number"]').first();
    const max = await windowTokens.getAttribute("max");
    const min = await windowTokens.getAttribute("min");
    expect(Number(min)).toBeGreaterThan(0);
    expect(Number(max)).toBeGreaterThan(Number(min));
  });

  test("Memory Worker breakdown percentages sum to 100 (audit #10)", async ({
    page,
  }) => {
    await setupAndLoginAsAdmin(page);
    await page.route("**/api/admin/memory/worker-metrics**", (r) =>
      r.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          scope: { kind: "tenant", tenant_id: "user-123" },
          runs: {
            last_24h: { persisted: 0, empty: 2, failed: 0, facts_persisted: 0 },
            last_7d: { persisted: 3, empty: 9, failed: 0, facts_persisted: 5 },
            last_run_at_ms: Date.now() - 3_600_000,
            last_failure_at_ms: null,
            backlog: 0,
            oldest_backlog_at_ms: null,
          },
          metrics: {
            total_facts: 17,
            facts_last_24h: 0,
            facts_last_7d: 5,
            distinct_users: 2,
            last_extracted_at: new Date(
              Date.now() - 3 * 86_400_000,
            ).toISOString(),
            facts_by_kind: {
              equipment: 6,
              goal: 4,
              preference: 4,
              other: 3,
            },
          },
        }),
      }),
    );
    await navigateToTab(page, "Memory Worker");
    // Wait for the breakdown rows to render before reading — the metrics are
    // fetched async, so reading immediately after navigation finds zero
    // percentages and the sum is 0.
    await expect(page.getByText(/\(\d+%\)/).first()).toBeVisible();
    const pcts = await page.getByText(/\(\d+%\)/).allTextContents();
    const sum = pcts
      .map((t) => Number(/\((\d+)%\)/.exec(t)?.[1] ?? 0))
      .reduce((a, b) => a + b, 0);
    expect(sum).toBe(100);
  });

  test("Analytics avg response time renders the wired value (audit #20)", async ({
    page,
  }) => {
    await setupAndLoginAsAdmin(page);
    await page.route("**/api/dashboard/analytics**", (r) =>
      r.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          time_series: [
            {
              date: "2026-05-07",
              request_count: 4,
              error_count: 0,
              average_response_time: 12345.0,
            },
          ],
          error_rate: 0,
          average_response_time: 12345.0,
        }),
      }),
    );
    await navigateToTab(page, "Analytics");
    await expect(page.getByText(/12345ms/).first()).toBeVisible();
    await expect(page.getByText(/^0ms$/)).toHaveCount(0);
  });
});
