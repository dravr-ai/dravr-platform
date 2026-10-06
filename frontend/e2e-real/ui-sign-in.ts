// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Signs a user into the real SPA the way an athlete does: Sign in → hosted login page → back to the app
// ABOUTME: Shared by every real-backend spec that drives the browser, so the flow lives in one place

import type { Page } from '@playwright/test';

/**
 * Open the SPA at `frontendUrl`, start its sign-in, type the credentials on
 * the server's hosted login page and wait until the browser is back on the
 * app (carnet#787: the SPA no longer has a password field of its own).
 *
 * Returns once the hosted page has redirected back to the frontend origin;
 * the caller waits for whatever screen it expects next.
 */
export async function signInThroughUi(page: Page, frontendUrl: string, email: string, password: string): Promise<void> {
  const appOrigin = new URL(frontendUrl).origin;
  await page.goto(frontendUrl);
  await page.getByRole('button', { name: /sign in|log in/i }).click();

  // The hosted page (crates/pierre-routes-identity/templates/oauth_login.html),
  // reached through the dev server's /oauth proxy or on the API origin.
  await page.waitForURL((url) => url.pathname.startsWith('/oauth2/'), { timeout: 15_000 });
  await page.locator('#email').fill(email);
  await page.locator('#password').fill(password);
  await page.locator('form[action="/oauth2/login"] button[type="submit"]').click();

  await page.waitForURL((url) => url.origin === appOrigin && !url.pathname.startsWith('/oauth2/'), {
    timeout: 15_000,
  });
}
