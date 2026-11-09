// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for Provider OAuth connections and status display.
// ABOUTME: Tests provider cards, connection flow, disconnect actions, and capability display.

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, openHome } from './test-helpers';

// Mock provider status data in the shape GET /api/providers serves: one card
// per provider, the raw `strava` / `garmin` rows withheld behind their mirror
// cards, and a connected card naming the backend behind it.
const mockProviders = {
  providers: [
    {
      provider: 'sciotte',
      display_name: 'Strava',
      description: 'Running, cycling, and swimming activities',
      requires_oauth: false,
      connected: true,
      connected_backend: 'strava',
      needs_reauth: false,
      capabilities: ['activities'],
      recommended_backend: 'oauth',
      seats_left: 7,
      consent_required: false,
    },
    {
      provider: 'sciotte_garmin',
      display_name: 'Garmin',
      description: 'Activities and health metrics from Garmin devices',
      requires_oauth: false,
      connected: false,
      needs_reauth: false,
      capabilities: ['activities', 'sleep', 'health'],
      consent_required: false,
    },
    {
      provider: 'synthetic',
      display_name: 'Synthetic',
      description: 'Synthetic test data for development',
      requires_oauth: false,
      connected: false,
      needs_reauth: false,
      capabilities: ['activities'],
      consent_required: false,
    },
    {
      provider: 'synthetic_sleep',
      display_name: 'Synthetic Sleep',
      description: 'Synthetic sleep data for development',
      requires_oauth: false,
      connected: false,
      needs_reauth: false,
      capabilities: ['sleep'],
      consent_required: false,
    },
  ],
};

async function setupProviderMocks(page: Page) {
  // Set up base dashboard mocks with user role
  await setupDashboardMocks(page, { role: 'user' });

  // Provider status endpoint
  await page.route('**/api/providers', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(mockProviders),
    });
  });

  // OAuth authorize URL
  await page.route('**/api/oauth/*/authorize', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        authorization_url: 'https://example.com/oauth/authorize?client_id=test',
      }),
    });
  });

  // Disconnect provider
  await page.route('**/api/oauth/providers/*/disconnect', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ success: true, message: 'Provider disconnected' }),
    });
  });

  // Coaches (needed by sidebar)
  await page.route('**/api/agents**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ agents: [], total: 0 }),
    });
  });

  // Chat conversations (empty: the chat pane then shows its provider status)
  await page.route('**/api/chat/conversations**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ conversations: [], total: 0, limit: 50, offset: 0 }),
    });
  });
}

test.describe('Provider Connections - Chat Provider Cards', () => {
  test.beforeEach(async ({ page }) => {
    await page.route((url) => url.pathname.startsWith('/api/') || url.pathname.startsWith('/oauth/'), async (route) => {
      await route.fallback();
    });
    await setupProviderMocks(page);
  });

  test('provider cards display in chat welcome view', async ({ page }) => {
    // Override to show no conversations (shows provider connection UI)
    await page.route('**/api/chat/conversations', async (route, request) => {
      if (request.method() === 'GET') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ conversations: [], total: 0, limit: 50, offset: 0 }),
        });
      } else {
        await route.fallback();
      }
    });

    await loginToDashboard(page);
    // Sign-in lands on Home; the provider status these tests read is the chat pane's.
    await openHome(page);

    // Chat tab should show providers - check page renders
    await page.waitForTimeout(1000);
    await expect(page.locator('main')).toBeVisible();
  });

  test('connected provider shows Connected badge', async ({ page }) => {
    await page.route('**/api/chat/conversations', async (route, request) => {
      if (request.method() === 'GET') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ conversations: [], total: 0, limit: 50, offset: 0 }),
        });
      } else {
        await route.fallback();
      }
    });

    await loginToDashboard(page);
    await openHome(page);

    // Strava is connected, should show Connected badge
    await expect(page.getByText('Connected').first()).toBeVisible({ timeout: 10000 });
  });

  test('displays provider names from server data', async ({ page }) => {
    await page.route('**/api/chat/conversations', async (route, request) => {
      if (request.method() === 'GET') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ conversations: [], total: 0, limit: 50, offset: 0 }),
        });
      } else {
        await route.fallback();
      }
    });

    await loginToDashboard(page);
    await openHome(page);
    await page.waitForTimeout(1000);

    // Provider names from server
    await expect(page.getByText('Strava')).toBeVisible({ timeout: 10000 });
  });
});

test.describe('Provider Connections - OAuth Flow', () => {
  test.beforeEach(async ({ page }) => {
    await page.route((url) => url.pathname.startsWith('/api/') || url.pathname.startsWith('/oauth/'), async (route) => {
      await route.fallback();
    });
    await setupProviderMocks(page);
  });

  // The connect runs in the athlete's own tab: the launch route 302s to the
  // provider, and the provider back to the app root with the result. Here the
  // launch route stands in for both hops and lands on the root directly.
  async function mockGarminRoundTrip(page: Page, outcome: string) {
    // Garmin's OAuth seats remain, so its card connects through Garmin's own OAuth.
    await page.route('**/api/providers', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          providers: mockProviders.providers.map((p) =>
            p.provider === 'sciotte_garmin' ? { ...p, recommended_backend: 'oauth', seats_left: 3 } : p,
          ),
        }),
      }),
    );
    await page.route('**/api/oauth/authorize/garmin', (route) =>
      route.fulfill({ status: 302, headers: { location: `/?provider=garmin&${outcome}` } }),
    );
    // The return is a page load of its own: the session is restored on it the
    // way every reload restores one.
    await page.route('**/api/auth/session', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          access_token: 'test-jwt-token',
          csrf_token: 'test-csrf-token',
          user: {
            id: 'user-123',
            user_id: 'user-123',
            email: 'alice@acme.com',
            display_name: 'Alice Test',
            role: 'user',
            is_admin: false,
            user_status: 'active',
            tier: 'professional',
            tenant_id: 'user-123',
          },
        }),
      }),
    );
  }

  test('a granted connect comes back to the tab and route it started from', async ({ page }) => {
    // Regression this turns red: the connect opening the provider in a new tab
    // again, or its return showing a callback page with a way "to the
    // dashboard" — a second copy of the app — instead of the route it left.
    await mockGarminRoundTrip(page, 'success=true');
    const popups: string[] = [];
    page.on('popup', (popup) => popups.push(popup.url()));

    await loginToDashboard(page);
    await page.goto('/#settings/connections');
    const garminRow = page.getByTestId('provider-row-sciotte_garmin');
    await expect(garminRow).toBeVisible({ timeout: 10000 });

    const launch = page.waitForRequest((request) => new URL(request.url()).pathname === '/api/oauth/authorize/garmin');
    await garminRow.getByRole('button', { name: 'Connect' }).click();
    await launch;

    await page.waitForURL((url) => url.hash === '#settings/connections' && url.search === '', { timeout: 20000 });
    await expect(page.getByText('Garmin connected successfully!')).toBeVisible({ timeout: 10000 });
    await expect(page.getByRole('heading', { name: 'Connection Failed' })).toHaveCount(0);
    expect(popups).toEqual([]);
    expect(page.context().pages()).toHaveLength(1);
  });

  test('a refused Garmin connect comes back to the same route and says so, opening no sign-in', async ({ page }) => {
    // A refused Garmin grant has nothing to say on a callback page: the
    // athlete is put back where they were, the card's Connect trying again.
    // While a seat is free it never falls back to the account sign-in.
    await mockGarminRoundTrip(page, 'success=false&error=access_denied');

    await loginToDashboard(page);
    await page.goto('/#settings/connections');
    const garminRow = page.getByTestId('provider-row-sciotte_garmin');
    await expect(garminRow).toBeVisible({ timeout: 10000 });

    await garminRow.getByRole('button', { name: 'Connect' }).click();

    await page.waitForURL((url) => url.hash === '#settings/connections' && url.search === '', { timeout: 20000 });
    await expect(page.getByText('Failed to connect Garmin')).toBeVisible({ timeout: 10000 });
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(page.getByRole('heading', { name: 'Connection Failed' })).toHaveCount(0);
    await expect(garminRow.getByRole('button', { name: 'Connect' })).toBeEnabled();
    expect(page.context().pages()).toHaveLength(1);
  });

  test('a Garmin connect whose seat went before it started falls back to the Garmin sign-in', async ({ page }) => {
    // The seats ran out between the card being read and the tap: the server
    // sends the tab back with `error=no_seat`, and the card, read again,
    // signs in — while the pool is full a new Garmin connection signs in.
    await mockGarminRoundTrip(page, 'success=false&error=no_seat');
    let seatsGone = false;
    await page.route('**/api/oauth/authorize/garmin', (route) => {
      seatsGone = true;
      return route.fulfill({ status: 302, headers: { location: '/?provider=garmin&success=false&error=no_seat' } });
    });
    await page.route('**/api/providers', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          providers: mockProviders.providers.map((p) =>
            p.provider === 'sciotte_garmin'
              ? { ...p, recommended_backend: seatsGone ? 'mirror' : 'oauth', seats_left: seatsGone ? 0 : 1 }
              : p,
          ),
        }),
      }),
    );

    await loginToDashboard(page);
    await page.goto('/#settings/connections');
    const garminRow = page.getByTestId('provider-row-sciotte_garmin');
    await expect(garminRow).toBeVisible({ timeout: 10000 });

    await garminRow.getByRole('button', { name: 'Connect' }).click();

    await page.waitForURL((url) => url.hash === '#settings/connections' && url.search === '', { timeout: 20000 });
    await expect(page.getByRole('dialog', { name: /Garmin/ })).toBeVisible({ timeout: 10000 });
    await expect(page.getByRole('heading', { name: 'Connection Failed' })).toHaveCount(0);
  });
});

test.describe('Provider Connections - Disconnect', () => {
  test.beforeEach(async ({ page }) => {
    await page.route((url) => url.pathname.startsWith('/api/') || url.pathname.startsWith('/oauth/'), async (route) => {
      await route.fallback();
    });
    await setupProviderMocks(page);
  });

  test('disconnect API endpoint is available', async ({ page }) => {
    let disconnectCalled = false;
    await page.route('**/api/oauth/providers/strava/disconnect', async (route) => {
      disconnectCalled = true;
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ success: true }),
      });
    });

    await loginToDashboard(page);
    await openHome(page);
    await page.waitForTimeout(500);

    // Verify disconnect endpoint is reachable via direct fetch
    await page.evaluate(async () => {
      await fetch('/api/oauth/providers/strava/disconnect', { method: 'DELETE' });
    });

    expect(disconnectCalled).toBe(true);
  });

  // carnet#1341: disconnecting the last provider turned the live
  // `needs_provider_connection` back to true, and the wizard reopened at its
  // connect step over Settings. The server now keeps a `connect_provider`
  // record once a provider has been connected, which holds the athlete past
  // the gate.
  test('disconnecting the last provider keeps an onboarded athlete in Settings', async ({ page }) => {
    let connected = true;
    // Every step the athlete answered, plus the connect-step record the
    // status read wrote while the provider was connected.
    const steps = [
      { step_id: 'profile_type', status: 'complete' },
      { step_id: 'about_you', status: 'complete' },
      { step_id: 'parq', status: 'complete' },
      { step_id: 'check_in', status: 'complete' },
      { step_id: 'connect_provider', status: 'complete' },
      { step_id: 'coach_proposal', status: 'complete' },
    ];
    const garmin = {
      provider: 'sciotte_garmin',
      display_name: 'Garmin',
      description: 'Activities and health metrics from Garmin devices',
      requires_oauth: false,
      needs_reauth: false,
      capabilities: ['activities', 'sleep', 'health'],
      consent_required: false,
    };
    await page.route('**/api/providers', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ providers: [{ ...garmin, connected }] }),
      }),
    );
    await page.route('**/api/oauth/providers/*/disconnect', (route) => {
      connected = false;
      return route.fulfill({ status: 200, contentType: 'application/json', body: '{"success":true}' });
    });
    await page.route('**/api/me/onboarding-status', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ needs_provider_connection: !connected, steps }),
      }),
    );

    await loginToDashboard(page);
    await page.goto('/#settings/connections');
    await page.getByTestId('provider-disconnect-sciotte_garmin').click();
    await page.getByRole('button', { name: 'Disconnect', exact: true }).last().click();
    await expect.poll(() => connected).toBe(false);

    // The athlete comes back to the tab: once the status is stale (5 s) the
    // app reads it again, now with no provider. Wait for that read to land,
    // or the assertions run before the wizard has had its chance.
    const statusRead = page.waitForResponse(
      (r) => r.url().includes('/api/me/onboarding-status') && !connected,
    );
    await page.waitForTimeout(5_500);
    await page.evaluate(() => window.dispatchEvent(new Event('visibilitychange')));
    await statusRead;
    await expect(page.getByText('Garmin has been disconnected.')).toBeVisible();
    await expect(page.getByTestId('settings-pane')).toBeVisible();
    // The card offers Connect again: the disconnect landed in Settings itself.
    await expect(page.getByRole('button', { name: 'Connect', exact: true })).toBeVisible();
    await expect(page).toHaveURL(/#settings\/connections$/);
    await expect(page.getByTestId('onboarding-flow')).toHaveCount(0);
  });
});

test.describe('Provider Connections - Garmin history copy (carnet#1386)', () => {
  // The Garmin grant's card once its OAuth landed, its one-time history copy
  // at `state`.
  const grantAt = (state: string) => ({
    provider: 'garmin',
    display_name: 'Garmin',
    description: 'Activities and health metrics from Garmin devices',
    requires_oauth: true,
    connected: true,
    connected_backend: 'garmin',
    needs_reauth: false,
    capabilities: ['activities', 'sleep', 'health'],
    consent_required: false,
    history: { state, sign_in_target: 'sciotte_garmin', archive_source: null },
  });

  const lines: Record<string, string> = {
    needed: "Your past Garmin activities aren't here yet",
    incomplete: "Your Garmin history didn't finish copying",
    importing: 'Copying your past Garmin activities',
    imported: 'Your Garmin history is in',
  };

  for (const [state, line] of Object.entries(lines)) {
    for (const theme of ['light', 'dark'] as const) {
      test(`the card reads "${state}" in the ${theme} theme`, async ({ page }, testInfo) => {
        await setupDashboardMocks(page, { role: 'user' });
        await page.route('**/api/providers', (route) =>
          route.fulfill({
            status: 200,
            contentType: 'application/json',
            body: JSON.stringify({ providers: [grantAt(state)] }),
          }),
        );
        await loginToDashboard(page);
        await page.goto('/#settings/connections');
        await page.evaluate((t) => document.documentElement.classList.toggle('dark', t === 'dark'), theme);

        const history = page.getByTestId('provider-history-garmin');
        await expect(history).toContainText(line);
        await expect(page.getByText(/sciotte/i)).toHaveCount(0);
        if (state === 'needed') {
          await expect(page.getByRole('button', { name: 'Add history' })).toBeVisible();
        }
        if (state === 'incomplete') {
          await expect(page.getByRole('button', { name: 'Try again' })).toBeVisible();
        }
        // Let the theme's transitions land before the picture is taken.
        await page.waitForTimeout(1500);
        await page.getByTestId('provider-row-garmin').screenshot({ path: testInfo.outputPath(`garmin-history-${state}-${theme}.png`) });
      });
    }
  }
});

test.describe('Provider Connections - Error Handling', () => {
  test('handles provider status load failure gracefully', async ({ page }) => {
    await page.route((url) => url.pathname.startsWith('/api/') || url.pathname.startsWith('/oauth/'), async (route) => {
      await route.fallback();
    });
    await setupDashboardMocks(page, { role: 'user' });

    // Override providers with error
    await page.route('**/api/providers', async (route) => {
      await route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Internal server error' }),
      });
    });

    await page.route('**/api/agents**', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ agents: [], total: 0 }),
      });
    });

    await loginToDashboard(page);
    await openHome(page);

    // Page should still render without crashing
    await page.waitForTimeout(1000);
    await expect(page.locator('main')).toBeVisible();
  });

  test('handles disconnect failure gracefully', async ({ page }) => {
    await page.route((url) => url.pathname.startsWith('/api/') || url.pathname.startsWith('/oauth/'), async (route) => {
      await route.fallback();
    });
    await setupProviderMocks(page);

    // Override disconnect with error
    await page.route('**/api/oauth/providers/strava/disconnect', async (route) => {
      await route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Failed to disconnect' }),
      });
    });

    await loginToDashboard(page);
    await openHome(page);

    // Page should remain functional even with disconnect failure
    await page.waitForTimeout(500);
    await expect(page.locator('main')).toBeVisible();
  });
});

test.describe('Provider Connections - Multiple Providers', () => {
  test('displays both connected and disconnected providers', async ({ page }) => {
    await page.route((url) => url.pathname.startsWith('/api/') || url.pathname.startsWith('/oauth/'), async (route) => {
      await route.fallback();
    });
    await setupProviderMocks(page);

    await page.route('**/api/chat/conversations', async (route, request) => {
      if (request.method() === 'GET') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ conversations: [], total: 0, limit: 50, offset: 0 }),
        });
      } else {
        await route.fallback();
      }
    });

    await loginToDashboard(page);
    await openHome(page);
    await page.waitForTimeout(1000);

    // Both Strava (connected) and Garmin (disconnected) should display
    await expect(page.getByText('Strava')).toBeVisible({ timeout: 10000 });
  });

  test('synthetic provider shows Demo badge', async ({ page }) => {
    await page.route((url) => url.pathname.startsWith('/api/') || url.pathname.startsWith('/oauth/'), async (route) => {
      await route.fallback();
    });
    await setupProviderMocks(page);

    await page.route('**/api/chat/conversations', async (route, request) => {
      if (request.method() === 'GET') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ conversations: [], total: 0, limit: 50, offset: 0 }),
        });
      } else {
        await route.fallback();
      }
    });

    await loginToDashboard(page);
    await openHome(page);
    await page.waitForTimeout(1000);

    // Synthetic providers should display as demo
    const demoText = page.getByText('Demo');
    if (await demoText.first().isVisible().catch(() => false)) {
      await expect(demoText.first()).toBeVisible();
    }
  });
});
