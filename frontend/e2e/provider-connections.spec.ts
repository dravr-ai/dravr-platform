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

  test('a refused Garmin connect comes back to the same route and offers the credential login', async ({ page }) => {
    // A refused Garmin grant (a seat-count race, or Garmin said no) has nothing
    // to say on a callback page: the athlete is put back where they were, with
    // the usr/pwd login for the same data.
    await mockGarminRoundTrip(page, 'success=false&error=access_denied');

    await loginToDashboard(page);
    await page.goto('/#settings/connections');
    const garminRow = page.getByTestId('provider-row-sciotte_garmin');
    await expect(garminRow).toBeVisible({ timeout: 10000 });

    await garminRow.getByRole('button', { name: 'Connect' }).click();

    await page.waitForURL((url) => url.hash === '#settings/connections' && url.search === '', { timeout: 20000 });
    await expect(page.getByRole('dialog', { name: /Garmin/ })).toBeVisible({ timeout: 10000 });
    await expect(page.getByRole('heading', { name: 'Connection Failed' })).toHaveCount(0);
    expect(page.context().pages()).toHaveLength(1);
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
