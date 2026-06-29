// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Real-backend E2E for the onboarding journey — register → (pending, session-gated) → admin-approve → active.
// ABOUTME: Drives a live Pierre server (8081) through the real register/approve/oauth-token/session APIs, no mocks.

import { test, expect, request as apiRequest, type APIRequestContext } from '@playwright/test';

const PIERRE_URL = process.env.PIERRE_URL ?? 'http://127.0.0.1:8081';
const ADMIN_EMAIL = process.env.ADMIN_EMAIL ?? 'admin@example.com';
const ADMIN_PASSWORD = process.env.ADMIN_PASSWORD ?? 'AdminPassword123';

// Opt-in real-server spec (`bun run test:e2e:real`). Requires a live Pierre
// server on 8081 with the seeded `admin@example.com` user. No mocks — the whole
// point is to exercise the real register → approval → login path end to end.
// The existing real-server suite only logs in a pre-seeded admin and never
// creates or approves a user, so the approval transition was untested.
//
// Approval model (verified against the live handlers): the OAuth token endpoint
// issues a token regardless of status, so a pending user CAN authenticate — but
// `/api/auth/session` is GATED until an admin approves the account, after which
// it resolves and reports `active`. On an auto-approving server
// (AUTO_APPROVE_USERS=true) registration is already `active` and skips the gate;
// on a gated one (the default) it is `pending` until approved. This test adapts
// to both and always ends with the user resolving an `active` session.

/** Password-grant login against the live server. */
async function loginToken(
  ctx: APIRequestContext,
  username: string,
  password: string,
) {
  return ctx.post('/oauth/token', {
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    form: { grant_type: 'password', username, password },
  });
}

/** Extract `access_token` if the login response carries one, else undefined. */
async function accessTokenOf(resp: Awaited<ReturnType<typeof loginToken>>) {
  if (!resp.ok()) {
    return undefined;
  }
  const body = await resp.json().catch(() => ({}));
  return typeof body.access_token === 'string' ? body.access_token : undefined;
}

test.describe('register → approve → login — real backend (no mocks)', () => {
  test('a fresh user registers, is session-gated while pending, approved by an admin, then resolves active', async () => {
    const ctx = await apiRequest.newContext({ baseURL: PIERRE_URL });
    // `@example.com` is NOT in AUTO_APPROVE_DOMAINS (dravr.ai). Timestamp keeps
    // the account unique across runs against the persistent dev DB.
    const email = `e2e-register-${Date.now()}@example.com`;
    const password = 'NewUserPassw0rd!';

    // 1. Register a fresh account against the real backend.
    const regResp = await ctx.post('/api/auth/register', {
      data: { email, password, display_name: 'E2E Register Flow' },
    });
    expect(regResp.status(), `register failed: ${regResp.status()}`).toBe(201);
    const reg = await regResp.json();
    const userId: string = reg.user_id;
    expect(userId).toMatch(/^[0-9a-f-]{36}$/);
    expect(['pending', 'active']).toContain(reg.user_status);

    // 2. Gated path: on a gated server the account is `pending`. The user can
    //    still obtain a token, but `/api/auth/session` is refused until an admin
    //    approves them — exercise that gate, then approve.
    if (reg.user_status === 'pending') {
      const pendingToken = await accessTokenOf(await loginToken(ctx, email, password));
      expect(pendingToken, 'a pending user can still obtain a token').toBeTruthy();

      const pendingSession = await ctx.get('/api/auth/session', {
        headers: { Authorization: `Bearer ${pendingToken}` },
      });
      expect(
        pendingSession.ok(),
        'a pending user session must be gated until approved',
      ).toBeFalsy();

      const adminAccess = await accessTokenOf(
        await loginToken(ctx, ADMIN_EMAIL, ADMIN_PASSWORD),
      );
      expect(adminAccess, 'admin login must succeed').toBeTruthy();

      const approveResp = await ctx.post(`/api/admin/approve-user/${userId}`, {
        headers: { Authorization: `Bearer ${adminAccess}` },
        data: { reason: 'e2e register→approve→login journey' },
      });
      expect(
        approveResp.ok(),
        `approve-user failed: ${approveResp.status()}`,
      ).toBeTruthy();
    }

    // 3. Approved (or auto-approved): the user logs in, the session now resolves,
    //    and reports `active` — the full onboarding→active journey is observable.
    const token = await accessTokenOf(await loginToken(ctx, email, password));
    expect(token, 'the approved user must be able to log in').toBeTruthy();

    const session = await ctx.get('/api/auth/session', {
      headers: { Authorization: `Bearer ${token}` },
    });
    expect(session.ok(), 'the approved user session must resolve').toBeTruthy();
    const body = await session.json();
    expect(body.user.email).toBe(email);
    expect(body.user.user_status).toBe('active');

    await ctx.dispose();
  });
});
