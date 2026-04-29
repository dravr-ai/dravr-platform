// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Real-backend E2E for the admin-tab cookie-auth surface — no mocks.
// ABOUTME: Logs into a live Pierre server, sweeps every /api/admin/... endpoint
// ABOUTME: that backs the 8 admin tabs, asserts 200 + valid JSON envelope.

import { test, expect, request as apiRequest, type APIRequestContext } from '@playwright/test';

const PIERRE_URL = process.env.PIERRE_URL ?? 'http://127.0.0.1:8081';
const ADMIN_EMAIL = process.env.ADMIN_EMAIL ?? 'admin@example.com';
const ADMIN_PASSWORD = process.env.ADMIN_PASSWORD ?? 'AdminPassword123';

// This spec is opt-in: it lives outside the default `e2e/` testDir and is
// only picked up by `bun run test:e2e:real`, which expects a live Pierre
// server (`./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh`) and
// the seeded `admin@example.com` user. No mocks — the whole point is to
// exercise `cookie_admin_middleware` + `ValidatedAdminToken` extension +
// real handlers + real DB rows.

async function loginAndAuthedRequest(): Promise<{
  ctx: APIRequestContext;
  tenantId: string;
}> {
  const bootstrap = await apiRequest.newContext({ baseURL: PIERRE_URL });

  const tokenResp = await bootstrap.post('/oauth/token', {
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    form: {
      grant_type: 'password',
      username: ADMIN_EMAIL,
      password: ADMIN_PASSWORD,
    },
  });
  expect(tokenResp.ok(), `login failed: ${tokenResp.status()}`).toBeTruthy();
  const tokenBody = await tokenResp.json();
  const accessToken: string = tokenBody.access_token;
  expect(typeof accessToken).toBe('string');
  expect(accessToken.length).toBeGreaterThan(20);

  const sessionResp = await bootstrap.get('/api/auth/session', {
    headers: { Authorization: `Bearer ${accessToken}` },
  });
  expect(sessionResp.ok()).toBeTruthy();
  const session = await sessionResp.json();
  const tenantId: string = session.user.tenant_id;
  expect(tenantId).toMatch(/^[0-9a-f-]{36}$/);

  await bootstrap.dispose();

  // Authed context that auto-attaches the session token on every request,
  // exactly as the cookie-auth `/api/admin/...` routes consume it.
  const ctx = await apiRequest.newContext({
    baseURL: PIERRE_URL,
    extraHTTPHeaders: { Authorization: `Bearer ${accessToken}` },
  });
  return { ctx, tenantId };
}

test.describe('admin tabs — real cookie-auth backend (no mocks)', () => {
  test('claim verdicts list returns 200 with verdict envelope', async () => {
    const { ctx, tenantId } = await loginAndAuthedRequest();
    const r = await ctx.get(
      `/api/admin/claim-verdicts?tenant_id=${tenantId}&limit=10`,
    );
    expect(r.status()).toBe(200);
    const body = await r.json();
    expect(body).toHaveProperty('verdicts');
    expect(Array.isArray(body.verdicts)).toBe(true);
    await ctx.dispose();
  });

  test('coach grading summary returns 200', async () => {
    const { ctx, tenantId } = await loginAndAuthedRequest();
    const r = await ctx.get(
      `/api/admin/coach-grading/summary?tenant_id=${tenantId}&limit=100`,
    );
    expect(r.status()).toBe(200);
    const body = await r.json();
    expect(body).toHaveProperty('grades');
    expect(body).toHaveProperty('verdicts_scanned');
    await ctx.dispose();
  });

  test('myth-busting summary returns 200', async () => {
    const { ctx, tenantId } = await loginAndAuthedRequest();
    const r = await ctx.get(
      `/api/admin/myth-busting/summary?tenant_id=${tenantId}&limit=100`,
    );
    expect(r.status()).toBe(200);
    const body = await r.json();
    expect(body).toHaveProperty('top_claims');
    expect(body).toHaveProperty('top_coaches');
    await ctx.dispose();
  });

  test('coach followups pending list returns 200', async () => {
    const { ctx, tenantId } = await loginAndAuthedRequest();
    const r = await ctx.get(
      `/api/admin/coach-followups/pending?tenant_id=${tenantId}&limit=10`,
    );
    expect(r.status()).toBe(200);
    await ctx.dispose();
  });

  test('coach notes audit returns 200', async () => {
    const { ctx, tenantId } = await loginAndAuthedRequest();
    const r = await ctx.get(
      `/api/admin/coach-notes/audit?tenant_id=${tenantId}&limit=10`,
    );
    expect(r.status()).toBe(200);
    await ctx.dispose();
  });

  test('memory worker metrics returns 200', async () => {
    const { ctx, tenantId } = await loginAndAuthedRequest();
    const r = await ctx.get(
      `/api/admin/memory/worker-metrics?tenant_id=${tenantId}`,
    );
    expect(r.status()).toBe(200);
    await ctx.dispose();
  });

  test('eval harness fixtures returns 200', async () => {
    const { ctx } = await loginAndAuthedRequest();
    const r = await ctx.get('/api/admin/evals/fixtures');
    expect(r.status()).toBe(200);
    const body = await r.json();
    expect(body).toHaveProperty('fixtures');
    await ctx.dispose();
  });

  test('eval harness verdict-stats returns 200', async () => {
    const { ctx, tenantId } = await loginAndAuthedRequest();
    const r = await ctx.get(
      `/api/admin/evals/verdict-stats?tenant_id=${tenantId}&window_days=30`,
    );
    expect(r.status()).toBe(200);
    await ctx.dispose();
  });

  test('harness config GET returns 200', async () => {
    const { ctx } = await loginAndAuthedRequest();
    const r = await ctx.get('/api/admin/settings/harness');
    expect(r.status()).toBe(200);
    await ctx.dispose();
  });

  test('cookie-auth rejects unauthenticated requests with 401', async () => {
    const anon = await apiRequest.newContext({ baseURL: PIERRE_URL });
    const r = await anon.get('/api/admin/claim-verdicts?limit=10');
    expect(r.status()).toBe(401);
    await anon.dispose();
  });

  test('cookie-auth rejects non-admin users with 403', async () => {
    // Seeded non-admin user from `bin/setup-db-with-seeds-and-oauth-and-start-servers.sh`.
    const bootstrap = await apiRequest.newContext({ baseURL: PIERRE_URL });
    const tokenResp = await bootstrap.post('/oauth/token', {
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      form: {
        grant_type: 'password',
        username: 'alice@acme.com',
        password: 'DemoUser123!',
      },
    });
    expect(
      tokenResp.ok(),
      'alice@acme.com must be present — re-run the setup script',
    ).toBeTruthy();
    const { access_token } = await tokenResp.json();
    await bootstrap.dispose();

    const userCtx = await apiRequest.newContext({
      baseURL: PIERRE_URL,
      extraHTTPHeaders: { Authorization: `Bearer ${access_token}` },
    });
    const r = await userCtx.get('/api/admin/claim-verdicts?limit=10');
    expect(r.status()).toBe(403);
    await userCtx.dispose();
  });
});
