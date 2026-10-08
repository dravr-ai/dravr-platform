// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for where a sign-in lands — Home for an athlete, by the hosted sign-in or by Google, whatever page the last session ended on
// ABOUTME: A link the athlete followed while signed out still opens where it pointed; Firebase is served as a stub module

import { test, expect, type Page } from '@playwright/test';
import {
  APP_SHELL_TIMEOUT_MS,
  openGroups,
  openHome,
  setupDashboardMocks,
  signInThroughHostedPage,
  waitForLoginScreen,
} from './test-helpers';

const ATHLETE = {
  id: 'user-123',
  user_id: 'user-123',
  email: 'alice@acme.com',
  display_name: 'Alice Test',
  role: 'user',
  is_admin: false,
  user_status: 'active',
  tier: 'professional',
  tenant_id: 'user-123',
};

/**
 * Stand in for the Firebase SDK. The e2e server has no Firebase env, so the
 * Google button is not drawn and the SDK would open a real Google popup. Vite
 * serves each source file as its own module, so answering the two module
 * requests with stubs exercises the login screen's real Google path — button,
 * `signInWithGoogle`, `POST /api/auth/firebase`, `loginWithFirebase` — with a
 * popup that returns an ID token at once.
 */
async function stubFirebase(page: Page) {
  await page.route(/\/src\/firebase\/config\.ts(\?.*)?$/, (route) =>
    route.fulfill({
      status: 200,
      contentType: 'text/javascript',
      body: [
        "export const firebaseConfig = { apiKey: 'e2e', projectId: 'e2e' };",
        'export function isFirebaseEnabled() { return true; }',
      ].join('\n'),
    }),
  );
  await page.route(/\/src\/firebase\/firebase\.ts(\?.*)?$/, (route) =>
    route.fulfill({
      status: 200,
      contentType: 'text/javascript',
      body: [
        'export function isFirebaseEnabled() { return true; }',
        "export async function signInWithGoogle() { return 'google-id-token'; }",
        'export async function getGoogleRedirectResult() { return null; }',
      ].join('\n'),
    }),
  );
  await page.route('**/api/auth/firebase', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ csrf_token: 'test-csrf-token', jwt_token: 'test-jwt-token', user: ATHLETE }),
    }),
  );
}

async function mockAthlete(page: Page) {
  await setupDashboardMocks(page, { role: 'user', email: ATHLETE.email, displayName: ATHLETE.display_name });
  await stubFirebase(page);
  await page.route('**/api/auth/logout', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '{}' }),
  );
  // The Home basemap is a third-party tile server; nothing here is about it.
  await page.route(/openfreemap\.org|arcgisonline\.com/, (route) => route.abort());
}

/** `hosted`: the server's hosted sign-in round trip (carnet#787); `google`: the Firebase button. */
type SignInMethod = 'hosted' | 'google';

async function signIn(page: Page, method: SignInMethod) {
  await waitForLoginScreen(page);
  if (method === 'hosted') {
    await signInThroughHostedPage(page);
  } else {
    await page.getByRole('button', { name: /continue with google/i }).click();
  }
  await page.waitForSelector('main', { timeout: APP_SHELL_TIMEOUT_MS });
}

/** Signed in on Groups, then the session dies the way a 401 ends it. */
async function sessionEndsOnGroups(page: Page) {
  await page.goto('/');
  await signIn(page, 'hosted');
  await openGroups(page);
  await expect(page).toHaveURL(/#chat(\/|$)/);
  await page.evaluate(() => window.dispatchEvent(new Event('pierre:auth:failure')));
  await waitForLoginScreen(page);
}

for (const method of ['hosted', 'google'] as const) {
  test.describe(`Sign-in landing — ${method}`, () => {
    test(`a session that ended on Groups signs back in on Home (${method})`, async ({ page }) => {
      await mockAthlete(page);
      await sessionEndsOnGroups(page);
      // The login screen no longer carries the dead session's page.
      expect(new URL(page.url()).hash).toBe('');

      await signIn(page, method);

      await expect(page).toHaveURL(/#home$/);
      await expect(page.getByTestId('home-page')).toBeVisible();
    });

    test(`a reloaded tab whose session expired signs back in on Home (${method})`, async ({ page }) => {
      await mockAthlete(page);
      await page.goto('/');
      await signIn(page, 'hosted');
      await openHome(page);

      // Overnight the cookie expired; the athlete comes back to the same tab.
      await page.route('**/api/auth/session', (route) =>
        route.fulfill({ status: 401, contentType: 'application/json', body: '{}' }),
      );
      await page.reload();
      await signIn(page, method);

      await expect(page).toHaveURL(/#home$/);
      await expect(page.getByTestId('home-page')).toBeVisible();
    });

    test(`a followed link still opens where it pointed (${method})`, async ({ page }) => {
      await mockAthlete(page);
      await page.goto('/#settings');

      await signIn(page, method);

      await expect(page).toHaveURL(/#settings(\/|$)/);
    });
  });
}

test('a followed link survives a stale cached session and its sign-out', async ({ page }) => {
  await mockAthlete(page);
  await page.addInitScript((user) => {
    window.localStorage.setItem('pierre_user', JSON.stringify(user));
  }, ATHLETE);
  await page.route('**/api/auth/session', (route) =>
    route.fulfill({ status: 401, contentType: 'application/json', body: '{}' }),
  );
  await page.goto('/#settings');
  await waitForLoginScreen(page);
  expect(new URL(page.url()).hash).toBe('#settings');

  await signIn(page, 'google');

  await expect(page).toHaveURL(/#settings(\/|$)/);
});

test("the create-account link's Google button signs a newcomer in on Home", async ({ page }) => {
  await mockAthlete(page);
  // dravr.ai's "Create your account" opens the registration form, which offers
  // Google too: a Google sign-in creates the account when none exists.
  await page.goto('/register');
  await page.waitForSelector('input[name="displayName"]');

  await page.getByRole('button', { name: /continue with google/i }).click();
  await page.waitForSelector('main', { timeout: APP_SHELL_TIMEOUT_MS });

  await expect(page).toHaveURL(/#home$/);
  await expect(page.getByTestId('home-page')).toBeVisible();
});
