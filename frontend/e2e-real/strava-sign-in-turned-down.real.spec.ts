// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Real-backend E2E: a Strava email sign-in the provider turns down, in web onboarding, quotes its reason and is no dead end
// ABOUTME: The window shows Strava's words, the Google/Apple advice, a retry and Strava's OAuth, against the scripted service double

import { test, expect, request as apiRequest, type APIRequestContext } from '@playwright/test';
import { SCIOTTE_DOUBLE_PORT, startSciotteDouble, type SciotteDouble } from './sciotte-double';
import { freshAthlete, retireAthlete, skipOnboarding } from './fresh-athlete';
import { signInThroughUi } from './ui-sign-in';

// Opt-in real-server spec (`bun run test:e2e:real`), in the service-double
// project: it needs a Pierre server started with
// DRAVR_SCIOTTE_REMOTE_URL=http://127.0.0.1:8097 (SCIOTTE_DOUBLE_PORT) and
// starts the double there itself. The server must hold Strava OAuth client
// credentials, as CI's and scripts/e2e-home-sync-local.sh's do: with a seat
// free, the Strava card signs in first and keeps its OAuth to fall back to.
//
// The 2026-11-24 post-mortem: an athlete whose Strava account was opened with
// Google typed an email and a password; Strava answered « Couldn't find this
// account », the window offered only a retry, and onboarding dead-ended.
const PIERRE_URL = process.env.PIERRE_URL ?? 'http://127.0.0.1:8081';
const FRONTEND_URL = process.env.FRONTEND_URL ?? 'http://localhost:5173';

const STRAVA_REASON = "Couldn't find this account";
const QUOTED_REASON = `Strava turned the sign-in down: “${STRAVA_REASON}”`;
const ADVICE = 'If you open Strava with Google or Apple, try again and pick that option.';

test.describe('Strava sign-in turned down in onboarding — real backend, scripted service', () => {
  let double: SciotteDouble;
  let ctx: APIRequestContext;
  let userId: string;
  let token: string;
  const email = `e2e-strava-turned-down-${Date.now()}@example.com`;
  const password = 'StravaTurnedDownPassw0rd!';

  test.beforeAll(async () => {
    double = await startSciotteDouble([]);
    double.login = 'rejected';
    double.rejectionReason = STRAVA_REASON;
    double.list = 'empty';
    ctx = await apiRequest.newContext({ baseURL: PIERRE_URL });
    ({ userId, token } = await freshAthlete(ctx, email, password, 'Strava Turned Down E2E'));
    await skipOnboarding(ctx, token);
  });

  test.afterAll(async () => {
    await retireAthlete(ctx, userId, email, 'strava sign-in turned down');
    await ctx?.dispose();
    await double?.stop();
  });

  test('quotes Strava, advises Google or Apple, and offers a retry and the OAuth', async ({ page }) => {
    await page.addInitScript(() => window.localStorage.setItem('pierre_app_language', 'en'));
    // The OAuth the window offers opens the server's launch route in a new
    // tab; answered here, so nothing leaves for strava.com.
    await page.context().route('**/api/oauth/authorize/**', (route) =>
      route.fulfill({ status: 200, contentType: 'text/html', body: '<title>Strava consent</title>' }),
    );
    await signInThroughUi(page, FRONTEND_URL, email, password);

    // An athlete with no provider lands on onboarding's connect step. Strava
    // leads it; a step that folds it under "More services" is opened first.
    const connectStrava = page.getByRole('button', { name: 'Connect to Strava' });
    const more = page.getByTestId('provider-more-services');
    await expect(connectStrava.or(more).first()).toBeVisible({ timeout: 15_000 });
    if (!(await connectStrava.isVisible())) {
      await more.click();
    }
    await connectStrava.click();

    const dialog = page.getByRole('dialog');
    await dialog.getByRole('button', { name: 'Log in with Strava email' }).click();
    await dialog.getByLabel(/email/i).fill('athlete@strava.example');
    await dialog.getByLabel('Password', { exact: true }).fill('never-real');
    await dialog.getByRole('button', { name: 'Log In' }).click();

    const message = dialog.getByText(QUOTED_REASON);
    await expect(
      message,
      `no rejection: is the server started with DRAVR_SCIOTTE_REMOTE_URL=http://127.0.0.1:${SCIOTTE_DOUBLE_PORT}?`,
    ).toBeVisible({ timeout: 30_000 });
    await expect(message).toContainText(ADVICE);
    await expect(dialog.getByRole('button', { name: 'Try again' })).toBeVisible();

    // The way on the athlete takes with a click: Strava's own OAuth, marked
    // as the sign-in's fallback.
    const offer = dialog.getByTestId('sciotte-oauth-fallback');
    await expect(offer).toHaveText('Continue with Strava');
    const tabOpened = page.context().waitForEvent('page');
    await offer.click();
    const tab = await tabOpened;
    await tab.waitForLoadState();
    const launched = new URL(tab.url());
    expect(launched.pathname).toBe('/api/oauth/authorize/strava');
    expect(launched.searchParams.get('sign_in_blocked')).toBe('true');
  });
});
