// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Real-backend E2E: a COROS code the scraper refuses keeps the sign-in dialog on its code step, then the right code connects
// ABOUTME: Drives the real modal and sciotte routes against the scripted scraper double, as a fresh athlete it suspends after

import { test, expect, request as apiRequest, type APIRequestContext } from '@playwright/test';
import { SCIOTTE_DOUBLE_PORT, startSciotteDouble, type SciotteDouble } from './sciotte-double';
import { freshAthlete, retireAthlete, skipOnboarding } from './fresh-athlete';
import { signInThroughUi } from './ui-sign-in';

// Opt-in real-server spec (`bun run test:e2e:real`). Like home-sync, it needs a
// Pierre server started with DRAVR_SCIOTTE_REMOTE_URL=http://127.0.0.1:8097
// (SCIOTTE_DOUBLE_PORT) and starts the scraper double there itself; the
// config runs the two one after the other. The athlete is registered for the
// run: the `provider_exposure_notice` flag is off for a new account, so COROS
// asks for no notice, and no seeded account's notice state is touched (the
// coros-notice spec relies on the webtest account never having accepted it).
const PIERRE_URL = process.env.PIERRE_URL ?? 'http://127.0.0.1:8081';
const FRONTEND_URL = process.env.FRONTEND_URL ?? 'http://localhost:5173';

const CODE_LABEL = 'Enter the 6-digit code COROS sent you';
const CODE_REJECTED = "That code wasn't accepted. Check it and enter it again.";

test.describe('COROS code step — real backend, scripted scraper', () => {
  let scraper: SciotteDouble;
  let ctx: APIRequestContext;
  let userId: string;
  let token: string;
  const email = `e2e-coros-code-${Date.now()}@example.com`;
  const password = 'CorosCodePassw0rd!';

  test.beforeAll(async () => {
    scraper = await startSciotteDouble([]);
    scraper.login = 'code';
    scraper.list = 'empty';
    ctx = await apiRequest.newContext({ baseURL: PIERRE_URL });
    ({ userId, token } = await freshAthlete(ctx, email, password, 'COROS Code E2E'));
    await skipOnboarding(ctx, token);
  });

  test.afterAll(async () => {
    await retireAthlete(ctx, userId, email, 'coros code step');
    await ctx?.dispose();
    await scraper?.stop();
  });

  test('a refused code stays on the code step, and the right code connects', async ({ page }) => {
    await page.addInitScript(() => window.localStorage.setItem('pierre_app_language', 'en'));
    await signInThroughUi(page, FRONTEND_URL, email, password);

    // An athlete with no provider lands on onboarding's connect step, whose
    // COROS card opens the same sign-in dialog Settings does.
    const connectCoros = page.getByRole('button', { name: 'Connect to COROS' });
    await expect(connectCoros).toBeVisible({ timeout: 15_000 });
    await connectCoros.click();

    const dialog = page.getByRole('dialog');
    await dialog.getByLabel('Email').fill('athlete@coros.example');
    await dialog.getByLabel('Password', { exact: true }).fill('never-real');
    await dialog.getByRole('button', { name: 'Log In' }).click();

    const code = dialog.getByLabel(CODE_LABEL);
    await expect(
      code,
      `no code step: is the server started with DRAVR_SCIOTTE_REMOTE_URL=http://127.0.0.1:${SCIOTTE_DOUBLE_PORT}?`,
    ).toBeVisible({ timeout: 30_000 });

    await code.fill('000000');
    await dialog.getByRole('button', { name: 'Verify' }).click();
    await expect(dialog.getByRole('alert')).toHaveText(CODE_REJECTED);
    await expect(code).toHaveValue('');
    await expect(code).toBeFocused();
    await expect(code).toHaveAttribute('aria-invalid', 'true');

    await code.fill('246810');
    await dialog.getByRole('button', { name: 'Verify' }).click();
    // Onboarding moves on as soon as the provider connects, so the account
    // says it rather than the dialog's own success state.
    await expect
      .poll(
        async () => {
          const providers = await (
            await ctx.get('/api/providers', { headers: { Authorization: `Bearer ${token}` } })
          ).json();
          return providers.providers.find((p: { provider: string }) => p.provider === 'sciotte_coros')?.connected;
        },
        { timeout: 30_000 },
      )
      .toBe(true);

    const submissions = scraper.codeSubmissions();
    expect(submissions.map((s) => s.code)).toEqual(['000000', '246810']);
    expect(new Set(submissions.map((s) => s.flow_id)).size).toBe(1);
  });
});
