// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Real-backend E2E for coaching-persona settings — no mocks.
// ABOUTME: Drives the live Pierre server + Vite dev frontend; persists into the seeded DB.

import { test, expect, request as apiRequest, type APIRequestContext } from '@playwright/test';

const PIERRE_URL = process.env.PIERRE_URL ?? 'http://127.0.0.1:8081';
const FRONTEND_URL = process.env.FRONTEND_URL ?? 'http://localhost:5173';
const TEST_EMAIL = process.env.WEBTEST_EMAIL ?? 'webtest@pierre.dev';
const TEST_PASSWORD = process.env.WEBTEST_PASSWORD ?? 'WebTest123!';

// Opt-in spec — only runs under `bun run test:e2e:real` after
// `./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh` has booted
// the full stack with the seeded webtest@pierre.dev account.

interface SessionBundle {
  ctx: APIRequestContext;
  accessToken: string;
}

async function loginAsWebtest(): Promise<SessionBundle> {
  const bootstrap = await apiRequest.newContext({ baseURL: PIERRE_URL });
  const tokenResp = await bootstrap.post('/oauth/token', {
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    form: {
      grant_type: 'password',
      username: TEST_EMAIL,
      password: TEST_PASSWORD,
    },
  });
  expect(
    tokenResp.ok(),
    `webtest login failed: ${tokenResp.status()} — re-run the setup script`,
  ).toBeTruthy();
  const { access_token: accessToken } = await tokenResp.json();
  expect(typeof accessToken).toBe('string');
  await bootstrap.dispose();

  const ctx = await apiRequest.newContext({
    baseURL: PIERRE_URL,
    extraHTTPHeaders: { Authorization: `Bearer ${accessToken}` },
  });
  return { ctx, accessToken };
}

async function readPersistedPersona(ctx: APIRequestContext): Promise<string> {
  const session = await ctx.get('/api/auth/session');
  expect(session.ok(), `/api/auth/session failed: ${session.status()}`).toBeTruthy();
  const body = await session.json();
  expect(body.user).toBeDefined();
  return String(body.user.coaching_persona ?? 'casual');
}

test.describe('coaching persona — real backend (no mocks)', () => {
  // Reset to casual after each test so subsequent tests start from a known state.
  test.afterEach(async () => {
    const { ctx } = await loginAsWebtest();
    await ctx.put('/api/user/coaching-persona', { data: { persona: 'casual' } });
    await ctx.dispose();
  });

  test('PUT /api/user/coaching-persona persists every variant round-trip', async () => {
    const { ctx } = await loginAsWebtest();

    for (const variant of ['casual', 'enthusiast', 'power_athlete', 'coach'] as const) {
      const put = await ctx.put('/api/user/coaching-persona', { data: { persona: variant } });
      expect(put.status(), `${variant} update`).toBe(200);
      const body = await put.json();
      expect(body.persona).toBe(variant);

      // Round-trip via /api/auth/session — confirms the row in the DB
      // actually changed, not just that the response echoed our input.
      const persisted = await readPersistedPersona(ctx);
      expect(persisted, `session.user.coaching_persona after ${variant}`).toBe(variant);
    }

    await ctx.dispose();
  });

  test('PUT /api/user/coaching-persona rejects unknown variant with 400', async () => {
    const { ctx } = await loginAsWebtest();
    const r = await ctx.put('/api/user/coaching-persona', {
      data: { persona: 'elite_pro_max' },
    });
    expect(r.status()).toBe(400);
    await ctx.dispose();
  });

  test('PUT /api/user/coaching-persona rejects unauthed requests with 401', async () => {
    const anon = await apiRequest.newContext({ baseURL: PIERRE_URL });
    const r = await anon.put('/api/user/coaching-persona', {
      data: { persona: 'casual' },
    });
    expect([401, 403]).toContain(r.status());
    await anon.dispose();
  });

  test('Settings → Coaching style tab updates the user via the live UI', async ({ page }) => {
    // 1. The SPA has no client router — login is conditional rendering off
    //    `isAuthenticated`. Open the root URL, fill the form, and wait for
    //    the dashboard "Open Settings" button to confirm we crossed over.
    await page.goto(FRONTEND_URL);
    await page.locator('input[name="email"]').fill(TEST_EMAIL);
    await page.locator('input[name="password"]').fill(TEST_PASSWORD);
    await page.getByRole('button', { name: /sign in|log in/i }).click();

    const openSettings = page.getByRole('button', { name: /open settings/i }).first();
    await openSettings.waitFor({ state: 'visible', timeout: 15_000 });
    await openSettings.click();

    // 2. Click the Coaching style tab.
    await page.getByRole('button', { name: /coaching style/i }).click();

    // 4. Sanity: all four cards render.
    for (const id of ['casual', 'enthusiast', 'power_athlete', 'coach']) {
      await expect(page.getByTestId(`persona-card-${id}`)).toBeVisible();
    }

    // 5. Click PowerAthlete and wait for the success banner.
    await page.getByTestId('persona-card-power_athlete').click();
    const status = page.getByTestId('persona-status');
    await expect(status).toBeVisible({ timeout: 10_000 });
    await expect(status).toContainText(/power_athlete/i);

    // 6. Confirm persistence directly against the backend, bypassing
    //    React Query's cache: real DB row, not just optimistic state.
    const { ctx } = await loginAsWebtest();
    const persisted = await readPersistedPersona(ctx);
    expect(persisted).toBe('power_athlete');
    await ctx.dispose();

    // 7. Reload the SPA and confirm the Active badge still reads
    //    PowerAthlete — proves the GET path also hydrates the new column.
    await page.reload();
    const openSettingsAgain = page
      .getByRole('button', { name: /open settings/i })
      .first();
    await openSettingsAgain.waitFor({ state: 'visible', timeout: 15_000 });
    await openSettingsAgain.click();
    await page.getByRole('button', { name: /coaching style/i }).click();
    await expect(
      page.getByTestId('persona-card-power_athlete'),
    ).toHaveAttribute('aria-checked', 'true');
  });
});
