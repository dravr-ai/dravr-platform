// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Real-backend E2E: a TrainingPeaks connect states the exposure notice first and is refused without it
// ABOUTME: Never submits a TrainingPeaks credential, so it records no consent and repeats on a seeded server

import { test, expect, request as apiRequest, type APIRequestContext } from '@playwright/test';

// Opt-in real-server spec (`bun run test:e2e:real`). Requires a live Pierre
// server on 8081 and the SPA on 5173, seeded by
// ./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh. The account is the
// seeded web test user — seeded with a provider, so it lands on the dashboard
// rather than the forced onboarding screen — which a fresh seed has never had
// accept the TrainingPeaks notice; nothing here accepts it, because acceptance
// is only recorded by a login attempt.
const PIERRE_URL = process.env.PIERRE_URL ?? 'http://127.0.0.1:8081';
const FRONTEND_URL = process.env.FRONTEND_URL ?? 'http://localhost:5173';
const EMAIL = process.env.WEBTEST_EMAIL ?? 'webtest@pierre.dev';
const PASSWORD = process.env.WEBTEST_PASSWORD ?? 'WebTest123!';

async function signedIn(): Promise<APIRequestContext> {
  const bootstrap = await apiRequest.newContext({ baseURL: PIERRE_URL });
  const token = await bootstrap.post('/oauth/token', {
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    form: { grant_type: 'password', username: EMAIL, password: PASSWORD },
  });
  expect(token.ok(), `seeded login failed: ${token.status()} — re-run the setup script`).toBeTruthy();
  const { access_token: accessToken } = await token.json();
  await bootstrap.dispose();
  return apiRequest.newContext({
    baseURL: PIERRE_URL,
    extraHTTPHeaders: { Authorization: `Bearer ${accessToken}` },
  });
}

test.describe('TrainingPeaks exposure notice — real backend (no mocks)', () => {
  test('the server asks for the notice and refuses a login without it', async () => {
    const ctx = await signedIn();

    const providers = await (await ctx.get('/api/providers')).json();
    const card = providers.providers.find(
      (p: { provider: string }) => p.provider === 'sciotte_trainingpeaks',
    );
    expect(card, 'the TrainingPeaks card is served').toBeDefined();
    expect(card.display_name).toBe('TrainingPeaks');
    expect(card.consent_required).toBe(true);

    // Refused before any credential leaves the server: the scraper is never
    // reached, so a made-up account is enough to prove the gate.
    const refused = await ctx.post('/api/providers/sciotte/login', {
      data: {
        email: 'never-sent',
        password: 'never-sent',
        method: 'email',
        target: 'trainingpeaks',
        tos_consent: false,
      },
    });
    expect(refused.status()).toBe(400);
    expect((await refused.json()).message).toContain('notice');

    await ctx.dispose();
  });

  test('Settings → TrainingPeaks shows the notice before the credentials and holds Log In', async ({ page }) => {
    await page.goto(FRONTEND_URL);
    await page.locator('input[name="email"]').fill(EMAIL);
    await page.locator('input[name="password"]').fill(PASSWORD);
    await page.getByRole('button', { name: /sign in|log in/i }).click();

    const openSettings = page.getByRole('button', { name: /open settings/i }).first();
    await openSettings.waitFor({ state: 'visible', timeout: 15_000 });
    await openSettings.click();
    await page.getByRole('button', { name: /data providers/i }).first().click();

    const row = page.getByTestId('provider-row-sciotte_trainingpeaks');
    await expect(row).toBeVisible({ timeout: 10_000 });
    await row.getByRole('button', { name: 'Connect', exact: true }).click();

    const dialog = page.getByRole('dialog');
    const notice = dialog.getByRole('note');
    await expect(notice).toContainText('Before you connect TrainingPeaks');
    await expect(notice).toContainText('Terms of Use (section 13)');
    // A coach's account also reads the athletes who confirm a link, so the
    // notice says so before the account is connected.
    await expect(notice).toContainText(
      "If yours is a human coach's account, Dravr also uses it to read the calendars of the athletes who confirm a link in a group you oversee.",
    );
    await expect(dialog.getByLabel('Username')).toHaveAttribute('type', 'text');

    // Nothing is typed into the credential fields: the gate under test is the
    // notice, and a filled form would only differ by enabling on the tick.
    const consent = dialog.getByLabel(
      'I understand that TrainingPeaks could suspend my account, and I accept that risk.',
    );
    await expect(consent).not.toBeChecked();
    await expect(dialog.getByRole('button', { name: 'Log In' })).toBeDisabled();
    await consent.check();
    await expect(consent).toBeChecked();

    await page.screenshot({ path: test.info().outputPath('trainingpeaks-notice.png') });
  });
});
