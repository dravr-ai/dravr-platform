// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for the login flow: the hosted sign-in round trip (carnet#787).
// ABOUTME: Tests the authorize request, the code exchange, the callback's refusals, and the login screen itself.

import { test, expect, type Page } from '@playwright/test';
import {
  SIGN_IN_BUTTON,
  mockHostedSignIn,
  setupDashboardMocks,
  signInThroughHostedPage,
  waitForLoginScreen,
} from './test-helpers';

// Helper to set up common API mocks for the login page
async function setupBasicMocks(page: Page) {
  // Mock setup status - this must be set up BEFORE navigating
  await page.route('**/admin/setup/status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        needs_setup: false,
        admin_user_exists: true,
        message: 'Admin user configured',
      }),
    });
  });
}

/** Base64url SHA-256 of `input`, computed in the page with WebCrypto. */
async function s256(page: Page, input: string): Promise<string> {
  return page.evaluate(async (verifier) => {
    const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier));
    let binary = '';
    for (const byte of new Uint8Array(digest)) binary += String.fromCharCode(byte);
    return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  }, input);
}

test.describe('Login Page', () => {
  test('renders the login screen with all expected elements', async ({ page }) => {
    await setupBasicMocks(page);
    await page.goto('/');
    await waitForLoginScreen(page);

    // Check for main heading (Boreal editorial layout uses "Sign in" as h1)
    await expect(page.locator('h1')).toContainText('Sign in');

    // DRAVR brand wordmark lives in the hero aside (desktop) and as a label on mobile
    await expect(page.getByText('DRAVR').first()).toBeVisible();

    // The aside is the hero line and the four pillars: no blurb under the
    // line, and no provider named anywhere in it (DESIGN.md, auth section).
    const aside = page.locator('aside');
    await expect(aside.locator('h2')).toBeVisible();
    await expect(aside).not.toContainText(/strava|garmin|trainingpeaks|coros|whoop|wahoo|intervals\.icu/i);

    await expect(page.getByRole('button', { name: 'Sign in with email' })).toBeVisible();
    await expect(page.getByRole('button', { name: /forgot password/i })).toBeVisible();
    await expect(page.getByRole('button', { name: /create one/i })).toBeVisible();

    // carnet#787: the password is typed on the server's hosted page, never here.
    await expect(page.locator('input[type="password"]')).toHaveCount(0);
    await expect(page.locator('input[name="email"]')).toHaveCount(0);

    await expect(page.getByText('Setup Required')).not.toBeVisible();
  });

  test('renders Google Sign-In button when Firebase is configured', async ({ page }) => {
    await setupBasicMocks(page);
    await page.goto('/');
    await waitForLoginScreen(page);

    // Google Sign-In button is only visible when Firebase env vars are configured
    // In CI without Firebase config, the button is hidden - this is expected behavior
    const googleButton = page.getByRole('button', { name: /continue with google/i });
    const isGoogleVisible = await googleButton.isVisible().catch(() => false);

    if (isGoogleVisible) {
      await expect(googleButton).toBeVisible();
      // Boreal editorial divider uses a short "OR" chip
      await expect(page.getByText(/^or$/i).first()).toBeVisible();
    } else {
      // Firebase not configured - button should not be present
      await expect(googleButton).not.toBeVisible();
    }
  });

  test('Google Sign-In button shows loading state when clicked', async ({ page }) => {
    await setupBasicMocks(page);
    await page.goto('/');
    await waitForLoginScreen(page);

    // When Firebase is not configured, the Google button is absent — verify that and return.
    // When Firebase is configured, verify the loading state on click.
    const googleButton = page.getByRole('button', { name: /continue with google/i });
    const isGoogleVisible = await googleButton.isVisible().catch(() => false);

    if (!isGoogleVisible) {
      await expect(googleButton).not.toBeVisible();
      return;
    }

    await expect(googleButton).toBeEnabled();

    // Click the button - it should show loading state
    // Note: We can't fully test Firebase redirect flow in E2E, but we can test the UI response
    await googleButton.click();

    // Button should show "Signing in..." text while loading
    await expect(page.getByRole('button', { name: /signing in/i })).toBeVisible({ timeout: 2000 });
  });

  test('opens the hosted sign-in as dravr-web with an S256 PKCE challenge', async ({ page }) => {
    await setupBasicMocks(page);

    let authorizeUrl: URL | null = null;
    await page.route('**/oauth2/authorize**', async (route) => {
      authorizeUrl = new URL(route.request().url());
      // The hosted page itself, standing still: this test is about the request.
      await route.fulfill({ status: 200, contentType: 'text/html', body: '<h1>Hosted sign-in</h1>' });
    });

    await page.goto('/');
    await waitForLoginScreen(page);
    await page.locator(SIGN_IN_BUTTON).click();
    await expect(page.getByRole('heading', { name: 'Hosted sign-in' })).toBeVisible();

    expect(authorizeUrl).not.toBeNull();
    const url = authorizeUrl as unknown as URL;
    const origin = new URL(page.url()).origin;
    expect(url.searchParams.get('response_type')).toBe('code');
    expect(url.searchParams.get('client_id')).toBe('dravr-web');
    expect(url.searchParams.get('redirect_uri')).toBe(`${origin}/auth/callback`);
    expect(url.searchParams.get('code_challenge_method')).toBe('S256');
    expect(url.searchParams.get('state')).toBeTruthy();

    // The verifier stayed behind in this tab, and the challenge is its S256.
    const pending = await page.evaluate(() => window.sessionStorage.getItem('pierre_pending_sign_in'));
    expect(pending).not.toBeNull();
    const { codeVerifier, state } = JSON.parse(pending as string) as { codeVerifier: string; state: string };
    expect(state).toBe(url.searchParams.get('state'));
    expect(url.searchParams.get('code_challenge')).toBe(await s256(page, codeVerifier));
    expect(url.toString()).not.toContain(codeVerifier);
  });

  test('signs in through the hosted page and redeems the code with its verifier', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'admin' });

    let exchange: URLSearchParams | null = null;
    await page.route('**/oauth/token', async (route) => {
      exchange = new URLSearchParams(route.request().postData() ?? '');
      await route.fallback();
    });

    await page.goto('/');
    await signInThroughHostedPage(page);

    // The login screen is gone, and the spent code left the address bar.
    await expect(page.locator(SIGN_IN_BUTTON)).toHaveCount(0, { timeout: 10000 });
    expect(new URL(page.url()).pathname).toBe('/');
    expect(new URL(page.url()).search).toBe('');

    expect(exchange).not.toBeNull();
    const form = exchange as unknown as URLSearchParams;
    expect(form.get('grant_type')).toBe('authorization_code');
    expect(form.get('client_id')).toBe('dravr-web');
    expect(form.get('code')).toBe('e2e-code');
    expect(form.get('code_verifier')).toMatch(/^[A-Za-z0-9\-._~]{43,128}$/);
    expect(form.get('redirect_uri')).toBe(`${new URL(page.url()).origin}/auth/callback`);
    expect(form.get('password')).toBeNull();

    // A verifier redeems one code: nothing is left pending.
    expect(await page.evaluate(() => window.sessionStorage.getItem('pierre_pending_sign_in'))).toBeNull();
  });

  test('a followed link survives the hosted sign-in round trip', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'admin' });

    await page.goto('/#settings');
    await signInThroughHostedPage(page);

    await expect(page).toHaveURL(/#settings/, { timeout: 10000 });
  });

  // The spinner held while the browser leaves for the hosted page is pinned by
  // the Login unit test: Playwright waits on the pending navigation before it
  // reads the page, so it can only ever see the page that comes after.
});

test.describe('Login Page - the hosted sign-in coming back empty', () => {
  test('a suspended account is told so, not that its password is wrong', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    await mockHostedSignIn(page, { error: 'access_denied' });

    await page.goto('/');
    await waitForLoginScreen(page);
    await page.locator(SIGN_IN_BUTTON).click();

    await expect(page.getByRole('alert')).toHaveText(
      'Your account has been suspended. Contact an administrator for help.',
      { timeout: 10000 },
    );
    await expect(page.locator(SIGN_IN_BUTTON)).toBeEnabled();
    expect(new URL(page.url()).pathname).toBe('/');
    await expect(page.getByText('Invalid email or password')).not.toBeVisible();
  });

  test('a code the server will not redeem reads as a failed sign-in', async ({ page }) => {
    await setupBasicMocks(page);
    await mockHostedSignIn(page);
    await page.route('**/oauth/token', async (route) => {
      await route.fulfill({
        status: 400,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'invalid_grant', error_description: 'The code has expired' }),
      });
    });

    await page.goto('/');
    await waitForLoginScreen(page);
    await page.locator(SIGN_IN_BUTTON).click();

    await expect(page.getByRole('alert')).toHaveText('Sign-in failed', { timeout: 10000 });
    await expect(page.getByText('Invalid email or password')).not.toBeVisible();
  });

  test('a code exchange that never reached a server reads as the network', async ({ page }) => {
    await setupBasicMocks(page);
    await mockHostedSignIn(page);
    await page.route('**/oauth/token', async (route) => {
      await route.abort('failed');
    });

    await page.goto('/');
    await waitForLoginScreen(page);
    await page.locator(SIGN_IN_BUTTON).click();

    await expect(page.getByText('Network error. Check your connection.')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Invalid email or password')).not.toBeVisible();
  });

  test('a callback this tab never started is refused without redeeming its code', async ({ page }) => {
    await setupBasicMocks(page);
    let exchanged = false;
    await page.route('**/oauth/token', async (route) => {
      exchanged = true;
      await route.fulfill({ status: 500, body: '{}' });
    });

    await page.goto('/auth/callback?code=forged-code&state=forged-state');

    await expect(page.getByRole('alert')).toHaveText('Sign-in failed', { timeout: 10000 });
    expect(new URL(page.url()).pathname).toBe('/');
    expect(exchanged).toBe(false);
  });
});

test.describe('Login Page - Accessibility', () => {
  test('the sign-in can be started from the keyboard', async ({ page }) => {
    await setupBasicMocks(page);
    let opened = false;
    await page.route('**/oauth2/authorize**', async (route) => {
      opened = true;
      await route.fulfill({ status: 200, contentType: 'text/html', body: '<h1>Hosted sign-in</h1>' });
    });

    await page.goto('/');
    await waitForLoginScreen(page);
    await page.locator(SIGN_IN_BUTTON).focus();
    await expect(page.locator(SIGN_IN_BUTTON)).toBeFocused();
    await page.keyboard.press('Enter');

    await expect(page.getByRole('heading', { name: 'Hosted sign-in' })).toBeVisible();
    expect(opened).toBe(true);
  });
});

test.describe('Stale cached session — logout storm', () => {
  // Regression: AuthContext optimistically renders a `dravr.user` cached in
  // localStorage before the cookie session is validated, so isAuthenticated
  // flips true and the guarded queries fire. Against a backend that no longer
  // knows the session (expired cookie, or a reset dev DB) every one of those
  // 401s raised `pierre:auth:failure`. The isLoggingOutRef guard released in
  // logout()'s .finally(), so it only blocked CONCURRENT re-entry — each later
  // 401 found the flag clear and fired another full logout. Observed live: 5
  // POST /api/auth/logout on a page where nobody had logged in.
  test('a stale cached user with a dead session logs out exactly once', async ({ page }) => {
    let logoutCalls = 0;

    await page.addInitScript(() => {
      window.localStorage.setItem(
        'pierre_user',
        JSON.stringify({
          id: 'stale-1',
          user_id: 'stale-1',
          email: 'stale@pierre.dev',
          display_name: 'Stale Session',
          role: 'user',
          is_admin: false,
          user_status: 'active',
          tenant_id: 'stale-1',
        }),
      );
    });

    // Backend no longer recognises the session.
    await page.route('**/api/auth/session', (route) =>
      route.fulfill({ status: 401, contentType: 'application/json', body: '{}' }),
    );
    await page.route('**/api/me/onboarding-status', (route) =>
      route.fulfill({ status: 401, contentType: 'application/json', body: '{}' }),
    );
    await page.route('**/api/auth/logout', async (route) => {
      logoutCalls += 1;
      await route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
    });

    await page.goto('/');
    // Settle: the login screen is what a cleared session must land on.
    await waitForLoginScreen(page, 10_000);
    await page.waitForTimeout(1500);

    expect(
      logoutCalls,
      `expected at most one logout for a dead session, got ${logoutCalls}`,
    ).toBeLessThanOrEqual(1);
  });
});
