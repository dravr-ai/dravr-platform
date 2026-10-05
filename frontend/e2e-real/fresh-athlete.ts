// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A fresh athlete for a real-backend spec: registered, approved by the seeded admin, past onboarding, suspended after
// ABOUTME: Shared by the specs that drive the SPA against the scraper double, so none of them touches a seeded account

import { expect, type APIRequestContext } from '@playwright/test';

export const ADMIN_EMAIL = process.env.ADMIN_EMAIL ?? 'admin@example.com';
export const ADMIN_PASSWORD = process.env.ADMIN_PASSWORD ?? 'AdminPassword123';

/** Every onboarding step the web flow would stop the athlete on before Home. */
const ONBOARDING_STEPS = [
  'profile_type',
  'about_you',
  'parq',
  'coach_proposal',
  'messaging_channel',
  'messaging_configure',
] as const;

export async function accessToken(ctx: APIRequestContext, email: string, password: string): Promise<string> {
  const response = await ctx.post('/oauth/token', {
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    form: { grant_type: 'password', client_id: 'dravr-web', username: email, password },
  });
  expect(response.ok(), `login of ${email} failed: ${response.status()} — re-run the setup script`).toBeTruthy();
  const { access_token: token } = await response.json();
  return token as string;
}

/** Register a fresh athlete named `displayName`, approve them as the seeded admin, and return a bearer for them. */
export async function freshAthlete(
  ctx: APIRequestContext,
  email: string,
  password: string,
  displayName: string,
): Promise<{ userId: string; token: string }> {
  const registered = await ctx.post('/api/auth/register', {
    data: { email, password, display_name: displayName },
  });
  expect(registered.status(), `register failed: ${registered.status()}`).toBe(201);
  const { user_id: userId, user_status: status } = await registered.json();
  if (status === 'pending') {
    const admin = await accessToken(ctx, ADMIN_EMAIL, ADMIN_PASSWORD);
    const approved = await ctx.post(`/api/admin/approve-user/${userId}`, {
      headers: { Authorization: `Bearer ${admin}` },
      data: { reason: `e2e ${displayName}` },
    });
    expect(approved.ok(), `approve-user failed: ${approved.status()}`).toBeTruthy();
  }
  return { userId, token: await accessToken(ctx, email, password) };
}

/** Skip every onboarding step, so the SPA opens on Home. */
export async function skipOnboarding(ctx: APIRequestContext, token: string): Promise<void> {
  for (const step of ONBOARDING_STEPS) {
    const done = await ctx.put(`/api/me/onboarding/steps/${step}`, {
      headers: { Authorization: `Bearer ${token}` },
      data: { status: 'skipped' },
    });
    expect(done.ok(), `onboarding step ${step}: ${done.status()}`).toBeTruthy();
  }
}

/** Suspend the athlete a spec registered, as its cleanup; a failure is reported, never thrown. */
export async function retireAthlete(ctx: APIRequestContext, userId: string, email: string, reason: string): Promise<void> {
  const admin = await accessToken(ctx, ADMIN_EMAIL, ADMIN_PASSWORD).catch(() => undefined);
  if (!admin || !userId) {
    return;
  }
  const suspended = await ctx.post(`/api/admin/suspend-user/${userId}`, {
    headers: { Authorization: `Bearer ${admin}` },
    data: { reason: `e2e cleanup: ${reason}` },
  });
  if (!suspended.ok()) {
    console.warn(`e2e cleanup: failed to suspend ${email} (${suspended.status()})`);
  }
}
