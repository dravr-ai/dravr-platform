// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Real-backend E2E pinning the GET /api/admin/tokens response contract.
// ABOUTME: Guards the omission of usage_count/permissions that crashed the token detail panel.

import { test, expect, request as apiRequest, type APIRequestContext } from '@playwright/test';

// Opt-in real-server spec (`bun run test:e2e:real`). Requires a live Pierre
// server on 8081 seeded by ./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh.
//
// Why a REAL-backend test and not a mocked one: the mocked suite already stubs
// this endpoint with a payload carrying `usage_count` and `permissions`
// (frontend/e2e/admin-tokens.spec.ts), and that fixture is exactly what hid the
// defect. The server's AdminTokenSummary omitted both fields entirely, while
// `packages/shared-types/src/admin.ts` declares them non-optional — so
// TypeScript flagged nothing on either side, the mocked suite stayed green, and
// `ApiKeyDetails` blew up on `usage_count.toLocaleString()` of undefined,
// throwing past the component to the root ErrorBoundary and blanking the SPA.
//
// A mock can never catch its own server drifting. Only this can.

const PIERRE_URL = process.env.PIERRE_URL ?? 'http://127.0.0.1:8081';
const ADMIN_EMAIL = process.env.ADMIN_EMAIL ?? 'admin@example.com';
const ADMIN_PASSWORD = process.env.ADMIN_PASSWORD ?? 'AdminPassword123';

async function adminToken(ctx: APIRequestContext): Promise<string> {
  const resp = await ctx.post('/oauth/token', {
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    form: { grant_type: 'password', username: ADMIN_EMAIL, password: ADMIN_PASSWORD },
  });
  expect(
    resp.ok(),
    `admin login failed for ${ADMIN_EMAIL} (${resp.status()}). Source .envrc — ` +
      'ADMIN_EMAIL/ADMIN_PASSWORD override the defaults.',
  ).toBe(true);
  const body = await resp.json();
  return body.access_token;
}

test.describe('admin token list contract — real backend (no mocks)', () => {
  test('every listed token carries the fields the console dereferences', async () => {
    const ctx = await apiRequest.newContext({ baseURL: PIERRE_URL });
    const auth = { Authorization: `Bearer ${await adminToken(ctx)}` };

    // The seeded DB has no admin tokens, so create one rather than skipping —
    // a spec that silently passes on an empty list guards nothing.
    const created = await ctx.post('/api/admin/tokens', {
      headers: auth,
      data: {
        service_name: `e2e-contract-${Date.now()}`,
        service_description: 'created by admin-token-contract.real.spec.ts',
        is_super_admin: false,
      },
    });
    expect(created.ok(), `token create failed: ${created.status()}`).toBe(true);

    const list = await ctx.get('/api/admin/tokens', { headers: auth });
    expect(list.ok(), `token list failed: ${list.status()}`).toBe(true);

    const body = await list.json();
    expect(Array.isArray(body.admin_tokens)).toBe(true);
    expect(body.admin_tokens.length).toBeGreaterThan(0);

    for (const token of body.admin_tokens) {
      // usage_count: dereferenced as `.toLocaleString()` in ApiKeyDetails.
      expect(
        typeof token.usage_count,
        `usage_count missing on token ${token.id} — this is the field whose ` +
          'absence crashed the detail panel',
      ).toBe('number');

      // permissions: dereferenced as `.map()`. Must be a flat array, not the
      // AdminPermissions wrapper struct's { permissions: [...] } shape.
      expect(
        Array.isArray(token.permissions),
        `permissions must be an array on token ${token.id}, got ` +
          `${JSON.stringify(token.permissions)}`,
      ).toBe(true);

      // token_prefix is rendered directly; shared-types declares it required.
      expect(typeof token.token_prefix).toBe('string');
      expect(typeof token.service_name).toBe('string');
      expect(typeof token.is_active).toBe('boolean');
      expect(typeof token.is_super_admin).toBe('boolean');
    }

    await ctx.dispose();
  });
});
