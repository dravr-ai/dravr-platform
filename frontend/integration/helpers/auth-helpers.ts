// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Authentication helper functions for integration tests.
// ABOUTME: Provides real login flows that interact with the actual backend server.

import type { Locator, Page } from '@playwright/test';
import { testUsers } from '../fixtures/test-data';
import { createTestAdminUser, createTestUser, type TestUser } from './db-setup';

export interface LoginResult {
  success: boolean;
  error?: string;
}

/**
 * The SPA's sign-in button. The login view has no password field of its own
 * (carnet#787): this button sends the browser to the server's hosted login
 * page. "Sign out" does not match, so it also tells a signed-out view apart.
 */
export function signInButton(page: Page): Locator {
  return page.getByRole('button', { name: /sign in|log in/i });
}

/** Whether the browser is on the server's hosted sign-in pages. */
function onHostedSignIn(page: Page): boolean {
  return new URL(page.url()).pathname.startsWith('/oauth2/');
}

/**
 * What a sign-in waits for once the browser is back on the app.
 *
 * `dashboard`: the admin dashboard's content, which an admin lands on.
 * `session`: the signed-in session alone. A plain user lands on whatever the
 * app shows an athlete (onboarding, on a new account), so the wait is for
 * the CSRF token the app stores when the session is established.
 * `athlete`: an admin who picks the athlete app at the sign-in picker
 * (carnet#1631), which opens onboarding or the athlete shell. `dashboard`
 * picks the console when the picker asks.
 */
export type LoginLanding = 'dashboard' | 'session' | 'athlete';

/** Where the SPA keeps the CSRF token of the signed-in session. */
const CSRF_TOKEN_KEY = 'pierre_csrf_token';

/**
 * Perform a real login the way an athlete does: the SPA's sign-in button, the
 * server's hosted login page (`#email`, `#password`, "Sign in"), then back to the
 * app at /auth/callback. This interacts with the actual backend server, not
 * mocked endpoints.
 */
export async function loginWithCredentials(
  page: Page,
  email: string,
  password: string,
  landing: LoginLanding = 'dashboard'
): Promise<LoginResult> {
  try {
    console.log(`[Login] Starting login for ${email}`);
    await page.goto('/');

    console.log('[Login] Opening the hosted sign-in...');
    await signInButton(page).click({ timeout: 15000 });
    await page.waitForURL((url) => url.pathname.startsWith('/oauth2/'), { timeout: 15000 });

    await page.locator('#email').fill(email);
    await page.locator('#password').fill(password);
    console.log('[Login] Submitting the hosted login form...');
    await page.locator('form[action="/oauth2/login"] button[type="submit"]').click();

    // Use a longer timeout for CI environments where server startup may be slower
    const loginTimeout = process.env.CI ? 30000 : 15000;

    // A refused password re-renders the hosted page with a role="alert"
    // banner and never leaves /oauth2/; a sign-in returns to the app.
    console.log('[Login] Waiting for the app or a refusal...');
    const outcome = await Promise.race([
      page
        .waitForURL((url) => !url.pathname.startsWith('/oauth2/'), { timeout: loginTimeout })
        .then(() => 'returned' as const),
      page
        .locator('[role="alert"]')
        .first()
        .waitFor({ state: 'visible', timeout: loginTimeout })
        .then(() => (onHostedSignIn(page) ? ('refused' as const) : ('returned' as const))),
    ]).catch(() => 'timeout' as const);

    if (outcome === 'refused') {
      const errorText = await page.locator('[role="alert"]').first().textContent().catch(() => null);
      console.log(`[Login] Error appeared: ${errorText}`);
      return { success: false, error: errorText || 'Login failed' };
    }
    if (outcome === 'timeout') {
      console.log('[Login] Still on the hosted sign-in after timeout');
      return { success: false, error: 'Login timed out - never returned to the app' };
    }

    if (landing === 'session') {
      // Back on the app: the session is established once its CSRF token is stored
      console.log('[Login] Waiting for the signed-in session...');
      try {
        await page.waitForFunction(
          (key) => window.localStorage.getItem(key) !== null,
          CSRF_TOKEN_KEY,
          { timeout: loginTimeout }
        );
      } catch {
        console.log('[Login] The session was not established');
        return { success: false, error: 'Login redirect occurred but no session was established' };
      }
      console.log('[Login] Success!');
      return { success: true };
    }

    // An admin is asked, at every sign-in, which app to open (carnet#1631):
    // 'dashboard' takes the console these specs drive, 'athlete' the athlete
    // app on the admin's own account. An athlete is never asked.
    const picker = page.getByTestId('view-mode-picker');
    if (landing === 'athlete') {
      console.log('[Login] Choosing the athlete app...');
      try {
        await picker.waitFor({ state: 'visible', timeout: loginTimeout });
        await page.getByTestId('view-mode-athlete').click();
        await page
          .getByTestId('onboarding-flow')
          .or(page.getByTestId('icon-rail'))
          .first()
          .waitFor({ state: 'visible', timeout: loginTimeout });
      } catch {
        console.log('[Login] The athlete app did not open');
        return { success: false, error: 'Login redirect occurred but the athlete app did not open' };
      }
      console.log('[Login] Success!');
      return { success: true };
    }

    // Back on the app: wait for dashboard content to appear
    console.log('[Login] Waiting for dashboard content...');
    try {
      // Use OR pattern since selectors have different syntax (CSS vs Playwright text=)
      const dashboardLocator = page.locator('nav')
        .or(page.locator('text=Dravr'))
        .or(page.locator('[class*="dashboard"]'));
      // The picker, or the console an earlier choice already opened: never the
      // loading screen's own logo text, which shows before the picker does.
      await picker.or(page.locator('nav')).first().waitFor({ state: 'visible', timeout: loginTimeout });
      if (await picker.isVisible()) {
        console.log('[Login] Choosing the admin console...');
        await page.getByTestId('view-mode-admin').click();
        await page.locator('nav').first().waitFor({ state: 'visible', timeout: 10000 });
      }
      await dashboardLocator.first().waitFor({ state: 'visible', timeout: 10000 });
      console.log('[Login] Dashboard content visible');
    } catch {
      // Dashboard didn't load, but the hosted page redirected back - ambiguous state
      console.log('[Login] Dashboard did not load');
      return { success: false, error: 'Login redirect occurred but dashboard did not load' };
    }

    await page.waitForTimeout(500);

    console.log('[Login] Success!');
    return { success: true };
  } catch (error) {
    const errorMsg = `Login failed: ${error instanceof Error ? error.message : String(error)}`;
    console.log(`[Login] Exception: ${errorMsg}`);
    return {
      success: false,
      error: errorMsg,
    };
  }
}

/**
 * Create a test admin user in the database and then log in.
 * This is the primary way to set up an authenticated session for tests.
 */
export async function createAndLoginAsAdmin(page: Page): Promise<LoginResult> {
  const user = testUsers.admin;
  console.log(`[Auth] Creating admin user: ${user.email}`);

  const createResult = await createTestAdminUser(user);
  if (!createResult.success) {
    console.log(`[Auth] Failed to create admin user: ${createResult.error}`);
    return { success: false, error: createResult.error };
  }
  console.log(`[Auth] Admin user created, proceeding to login`);

  return loginWithCredentials(page, user.email, user.password);
}

/**
 * Create a custom test user, with the role it names (an admin when it names
 * none), and log in. A plain user has no admin dashboard to land on, so its
 * sign-in waits for the session alone.
 */
export async function createAndLoginTestUser(
  page: Page,
  user: TestUser
): Promise<LoginResult> {
  const createResult = await createTestUser(user);
  if (!createResult.success) {
    return { success: false, error: createResult.error };
  }

  const landing: LoginLanding = user.role === 'user' ? 'session' : 'dashboard';
  return loginWithCredentials(page, user.email, user.password, landing);
}

/**
 * Log out of the current session.
 * Tries multiple selectors to handle different UI states (icon button vs text button).
 */
export async function logout(page: Page): Promise<void> {
  // Try multiple selectors: icon button with title, or text button
  const selectors = [
    page.locator('button[title="Sign out"]'),
    page.locator('button:has-text("Sign Out")'),
    page.locator('button:has-text("Logout")'),
  ];

  for (const selector of selectors) {
    const isVisible = await selector.first().isVisible().catch(() => false);
    if (isVisible) {
      await selector.first().click();
      await signInButton(page).waitFor({ state: 'visible', timeout: 10000 });
      return;
    }
  }

  // Fallback: navigate to root which will redirect to login if not authenticated
  await page.goto('/');
}

/**
 * Check if the user is currently logged in.
 */
export async function isLoggedIn(page: Page): Promise<boolean> {
  try {
    if (onHostedSignIn(page)) {
      return false;
    }
    const isSignInVisible = await signInButton(page).first().isVisible().catch(() => true);
    return !isSignInVisible;
  } catch {
    return false;
  }
}

/**
 * Navigate to a specific dashboard tab.
 * Tries multiple selector strategies to handle both expanded and collapsed sidebar states.
 */
export async function navigateToTab(page: Page, tabName: string): Promise<void> {
  // Wait for the sidebar to be present
  await page.waitForSelector('nav', { timeout: 10000 });

  const selectors = [
    // Button with span containing the text (expanded sidebar)
    page.locator('button').filter({ has: page.locator(`span:has-text("${tabName}")`) }),
    // Button with title attribute (collapsed sidebar)
    page.locator(`button[title="${tabName}"]`),
    // Button containing the text directly
    page.locator(`button:has-text("${tabName}")`),
    // Button with div containing text
    page.locator('button').filter({ has: page.locator(`div:has-text("${tabName}")`) }),
    // Sidebar button by role and accessible name
    page.getByRole('button', { name: new RegExp(`.*${tabName}.*`, 'i') }),
  ];

  for (const selector of selectors) {
    try {
      const count = await selector.count();
      if (count > 0) {
        const isVisible = await selector.first().isVisible().catch(() => false);
        if (isVisible) {
          await selector.first().click();
          await page.waitForTimeout(500);
          return;
        }
      }
    } catch {
      // Continue to next selector
    }
  }

  // Last resort: find by data-testid or aria-label if available
  const lastResort = page.locator(`[data-testid*="${tabName.toLowerCase()}"], [aria-label*="${tabName}" i]`);
  const lastResortCount = await lastResort.count().catch(() => 0);
  if (lastResortCount > 0) {
    await lastResort.first().click();
    await page.waitForTimeout(500);
    return;
  }

  throw new Error(`Could not find tab button for "${tabName}". Available buttons: check screenshot for details.`);
}

/**
 * Wait for the dashboard to fully load after login.
 */
export async function waitForDashboardLoad(page: Page): Promise<void> {
  await page.waitForSelector('text=Dravr', { timeout: 15000 });

  await page.waitForLoadState('networkidle', { timeout: 10000 }).catch(() => {
    // Network idle timeout is acceptable, page may have ongoing requests
  });

  await page.waitForTimeout(500);
}
