// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Real-backend E2E: a COROS connect states its own exposure notice first and is refused without it
// ABOUTME: Never submits a COROS credential, so it records no consent and repeats on a seeded server

import { test, expect, request as apiRequest, type APIRequestContext } from '@playwright/test';

// Opt-in real-server spec (`bun run test:e2e:real`). Requires a live Pierre
// server on 8081 and the SPA on 5173, seeded by
// ./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh. The notice is
// asked only of an account the `provider_exposure_notice` feature flag arms:
// off by default, and armed by the seeder for the web test user, which a fresh
// seed has never had accept the COROS notice. Nothing here accepts it, because
// acceptance is only recorded by a login attempt. The notice is per provider,
// so the TrainingPeaks spec's state never leaks into this one. A demo account
// keeps the default and is asked for nothing.
const PIERRE_URL = process.env.PIERRE_URL ?? 'http://127.0.0.1:8081';
const FRONTEND_URL = process.env.FRONTEND_URL ?? 'http://localhost:5173';
const EMAIL = process.env.WEBTEST_EMAIL ?? 'webtest@pierre.dev';
const PASSWORD = process.env.WEBTEST_PASSWORD ?? 'WebTest123!';

const DEMO_EMAIL = process.env.DEMO_EMAIL ?? 'alice@acme.com';
const DEMO_PASSWORD = process.env.DEMO_PASSWORD ?? 'DemoUser123!';

async function signedIn(email = EMAIL, password = PASSWORD): Promise<APIRequestContext> {
  const bootstrap = await apiRequest.newContext({ baseURL: PIERRE_URL });
  const token = await bootstrap.post('/oauth/token', {
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    form: { grant_type: 'password', username: email, password },
  });
  expect(token.ok(), `seeded login failed: ${token.status()} — re-run the setup script`).toBeTruthy();
  const { access_token: accessToken } = await token.json();
  await bootstrap.dispose();
  return apiRequest.newContext({
    baseURL: PIERRE_URL,
    extraHTTPHeaders: { Authorization: `Bearer ${accessToken}` },
  });
}

test.describe('COROS exposure notice — real backend (no mocks)', () => {
  test('the server asks for the notice and refuses a login without it', async () => {
    const ctx = await signedIn();

    const providers = await (await ctx.get('/api/providers')).json();
    const card = providers.providers.find((p: { provider: string }) => p.provider === 'sciotte_coros');
    expect(card, 'the COROS card is served').toBeDefined();
    expect(card.display_name).toBe('COROS');
    expect(card.consent_required).toBe(true);
    // COROS is mirror-only: the raw `coros` row is never offered beside it.
    expect(providers.providers.some((p: { provider: string }) => p.provider === 'coros')).toBe(false);

    // Refused before any credential leaves the server: the scraper is never
    // reached, so a made-up account is enough to prove the gate.
    const refused = await ctx.post('/api/providers/sciotte/login', {
      data: {
        email: 'never-sent@example.com',
        password: 'never-sent',
        method: 'email',
        target: 'coros',
        tos_consent: false,
      },
    });
    expect(refused.status()).toBe(400);
    const { message } = await refused.json();
    expect(message).toContain('COROS');
    expect(message).toContain('notice');

    await ctx.dispose();
  });

  test('a demo account, left at the flag default, is asked for no notice', async () => {
    const ctx = await signedIn(DEMO_EMAIL, DEMO_PASSWORD);

    const features = await (await ctx.get('/api/me/features')).json();
    expect(features.flags.provider_exposure_notice).toBe(false);
    const providers = await (await ctx.get('/api/providers')).json();
    for (const provider of ['sciotte_trainingpeaks', 'sciotte_coros']) {
      const card = providers.providers.find((p: { provider: string }) => p.provider === provider);
      expect(card, `the ${provider} card is served`).toBeDefined();
      expect(card.consent_required, `${provider} asks a demo account for nothing`).toBe(false);
    }

    await ctx.dispose();
  });

  test('Settings → COROS shows its notice before the credentials and holds Log In', async ({ page }) => {
    await page.goto(FRONTEND_URL);
    await page.locator('input[name="email"]').fill(EMAIL);
    await page.locator('input[name="password"]').fill(PASSWORD);
    await page.getByRole('button', { name: /sign in|log in/i }).click();

    const openSettings = page.getByRole('button', { name: /open settings/i }).first();
    await openSettings.waitFor({ state: 'visible', timeout: 15_000 });
    await openSettings.click();
    await page.getByRole('button', { name: /data providers/i }).first().click();

    const row = page.getByTestId('provider-row-sciotte_coros');
    await expect(row).toBeVisible({ timeout: 10_000 });
    await row.getByRole('button', { name: 'Connect', exact: true }).click();

    const dialog = page.getByRole('dialog');
    const notice = dialog.getByRole('note');
    await expect(notice).toContainText('Before you connect COROS');
    await expect(notice).toContainText('Terms of Service (sections 4 and 7)');
    await expect(dialog.getByLabel('Email')).toHaveAttribute('type', 'email');

    const consent = dialog.getByLabel('I understand that COROS could suspend my account, and I accept that risk.');
    await expect(consent).not.toBeChecked();
    await expect(dialog.getByRole('button', { name: 'Log In' })).toBeDisabled();
    await consent.check();
    await expect(consent).toBeChecked();

    await page.screenshot({ path: test.info().outputPath('coros-notice.png') });
  });
});
