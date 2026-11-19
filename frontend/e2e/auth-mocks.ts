// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The signed-out mocks the sign-up, sign-in-check and design-sweep pages share — setup status, sign-up config, a dead session
// ABOUTME: And the takeover token a login takeover page is opened with, shaped as the server mints one

import type { Page } from '@playwright/test';

/** The server says an admin exists, so the app shows sign-in rather than first-run setup. */
export async function mockSetupStatus(page: Page): Promise<void> {
  await page.route('**/admin/setup/status', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ needs_setup: false, admin_user_exists: true, message: 'Admin user configured' }),
    }),
  );
}

/** Sign-up open (or paused), no bot check: the form holds submit until this answers. */
export async function mockSignupConfig(page: Page, { open = true }: { open?: boolean } = {}): Promise<void> {
  await page.route('**/api/auth/signup-config', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ open, turnstile_site_key: null }),
    }),
  );
}

/** No session: the visitor is signed out. */
export async function mockSignedOut(page: Page): Promise<void> {
  await page.route('**/api/auth/session', (route) =>
    route.fulfill({ status: 401, contentType: 'application/json', body: '{}' }),
  );
}

/** A takeover token shaped as the server mints one: its `tgt` names the provider. */
export function takeoverToken(): string {
  const b64url = (json: string) => Buffer.from(json, 'utf8').toString('base64url');
  return `${b64url('{"alg":"HS256","typ":"JWT"}')}.${b64url('{"tgt":"sciotte_garmin","scope":"provider:sciotte:takeover"}')}.signature`;
}
