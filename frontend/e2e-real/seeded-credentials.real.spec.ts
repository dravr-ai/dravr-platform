// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Real-backend E2E asserting every documented seeded credential actually authenticates.
// ABOUTME: Guards the drift where docs/hooks advertised a password the seeder no longer produces.

import { test, expect, request as apiRequest, type APIRequestContext } from '@playwright/test';

// Opt-in real-server spec (`bun run test:e2e:real`). Requires a live Pierre
// server on 8081 seeded by ./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh.
//
// Why this exists: the seeded credential table is restated in at least five
// places (the setup script's summary, .claude/settings.json's session banner,
// the setup-server + test-web-app skills, frontend-mobile/.maestro/config.yaml,
// and assorted READMEs) while the single source of truth is the seeder itself
// — crates/pierre-seeders/src/demo_data.rs. Nothing proved those agreed, and
// they drifted: the session banner advertised `MobileTest123!` for months while
// the seeder produced `MobileTest1234`, so anyone trusting the banner hit a
// silent 401 and blamed the app.
//
// This spec pins the half that is machine-checkable: every credential we tell
// people to use must actually authenticate against a freshly seeded server, and
// the operator account must really carry super-admin. It cannot catch a typo in
// a markdown table on its own — the defence against that is not restating the
// table, and pointing at this file instead.

const PIERRE_URL = process.env.PIERRE_URL ?? 'http://127.0.0.1:8081';

// The admin is NOT a seeder constant. The setup script reads
// `${ADMIN_EMAIL:-admin@example.com}` / `${ADMIN_PASSWORD:-AdminPassword123}`,
// so a developer whose .envrc sets ADMIN_EMAIL gets a different operator
// account entirely — locally that is `admin@pierre.mcp`, while CI (no .envrc
// override) gets the `admin@example.com` default. Hardcoding either one makes
// this spec fail on the other machine for a reason that is not a real defect,
// so resolve it the same way the script does. Run under `direnv exec . …` or
// with .envrc sourced so these are populated.
const ADMIN_EMAIL = process.env.ADMIN_EMAIL ?? 'admin@example.com';
const ADMIN_PASSWORD = process.env.ADMIN_PASSWORD ?? 'AdminPassword123';

/**
 * Accounts created by the seeders with passwords baked into the seeder source —
 * these ARE constants (crates/pierre-seeders/src/demo_data.rs), unlike the admin.
 * Changing a password there without changing it here must turn this suite red.
 */
const SEEDED_ACCOUNTS = [
  { label: 'web test', email: 'webtest@pierre.dev', password: 'WebTest123!' },
  { label: 'mobile test', email: 'mobiletest@pierre.dev', password: 'MobileTest1234' },
  { label: 'demo (strava)', email: 'alice@acme.com', password: 'DemoUser123!' },
  { label: 'demo (garmin)', email: 'bob@startup.io', password: 'DemoUser123!' },
] as const;

/** Password-grant login against the live server. */
async function login(ctx: APIRequestContext, username: string, password: string) {
  return ctx.post('/oauth/token', {
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    form: { grant_type: 'password', username, password },
  });
}

test.describe('seeded credentials — real backend (no mocks)', () => {
  for (const account of SEEDED_ACCOUNTS) {
    test(`${account.label} (${account.email}) authenticates with the documented password`, async () => {
      const ctx = await apiRequest.newContext({ baseURL: PIERRE_URL });

      const resp = await login(ctx, account.email, account.password);
      expect(
        resp.ok(),
        `${account.email} failed to authenticate (${resp.status()}). ` +
          'Either the seeder changed this password or the docs are stale — fix both together.',
      ).toBe(true);

      // Assert content, not just a 2xx: a token endpoint that returns 200 with
      // an empty body would sail past `resp.ok()` alone.
      const body = await resp.json();
      expect(typeof body.access_token, 'no access_token in the token response').toBe('string');
      expect(body.access_token.length).toBeGreaterThan(20);
      expect(body.user?.email).toBe(account.email);
      expect(body.user?.user_status).toBe('active');

      await ctx.dispose();
    });
  }

  test('the operator account really carries super-admin, not plain admin', async () => {
    // bin/setup-db-with-seeds-and-oauth-and-start-servers.sh creates the admin
    // with --super-admin on purpose: cookie_admin_middleware derives console
    // permissions from the role, and a plain `admin` silently loses contremaitre
    // config, store moderation and impersonation. A downgrade here would show up
    // as tabs quietly missing from the console rather than as an error.
    const ctx = await apiRequest.newContext({ baseURL: PIERRE_URL });

    const resp = await login(ctx, ADMIN_EMAIL, ADMIN_PASSWORD);
    expect(
      resp.ok(),
      `admin login failed for ${ADMIN_EMAIL} (${resp.status()}). ` +
        'If this is a local run, source .envrc — ADMIN_EMAIL/ADMIN_PASSWORD override the defaults.',
    ).toBe(true);

    const body = await resp.json();
    expect(typeof body.access_token, 'no access_token in the admin token response').toBe('string');
    expect(body.user?.email).toBe(ADMIN_EMAIL);
    expect(body.user?.user_status).toBe('active');
    expect(body.user?.role).toBe('super_admin');
    expect(body.user?.is_admin).toBe(true);

    await ctx.dispose();
  });

  test('a wrong password is rejected — the suite would catch a server that accepts anything', async () => {
    // Without this, every assertion above would still pass against a server
    // that authenticated any credential, which is exactly the failure mode a
    // credential suite must not have.
    const ctx = await apiRequest.newContext({ baseURL: PIERRE_URL });

    const resp = await login(ctx, 'webtest@pierre.dev', 'definitely-not-the-password');
    expect(resp.ok(), 'server accepted a wrong password').toBe(false);

    await ctx.dispose();
  });
});
