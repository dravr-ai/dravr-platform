// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A fresh coach waiting on the onboarding group step, made through the server's own routes, and what the step left
// ABOUTME: Served by the Maestro control process (home-sync-control.ts) to the mobile onboarding group flow

import { firstPartySignIn } from '../../scripts/auth/first-party-sign-in.js';

/** The group the flow names on the step. */
export const ONBOARDING_GROUP_NAME = 'Les Rouleurs e2e';

/** One password for every coach this makes: each is a new account, so nothing is shared but the string. */
const COACH_PASSWORD = 'CoachE2e-Pass1234';

/** The server under test and the admin whose session approves a pending account. */
export interface CoachServer {
  pierreUrl: string;
  adminEmail: string;
  adminPassword: string;
}

/** The coach a setup made, as the flow signs in with them. */
export interface OnboardingCoach {
  email: string;
  password: string;
  groupName: string;
}

/** What the group step left on the server for the coach, as the flow asserts it. */
export interface CoachStepState {
  /** The `coach_group` step's recorded status, or `none` while nothing was recorded. */
  step: string;
  /** The coach-access request's status, or `none` when the coach never asked. */
  access: string;
  /** Whether the coach has a thread in the group the request names. */
  groupThread: boolean;
  /** How many group threads the coach has at all. */
  groupThreads: number;
}

/** Bearers of the coaches made so far, by email, so a later state read signs nobody in again. */
const bearers = new Map<string, string>();

async function call(server: CoachServer, path: string, init: RequestInit = {}, token?: string): Promise<Response> {
  const headers = new Headers(init.headers);
  if (token !== undefined) {
    headers.set('Authorization', `Bearer ${token}`);
  }
  if (init.body !== undefined) {
    headers.set('Content-Type', 'application/json');
  }
  return fetch(`${server.pierreUrl}${path}`, { ...init, headers });
}

async function expectOk(response: Response, what: string): Promise<Response> {
  if (!response.ok) {
    throw new Error(`${what} answered ${response.status}: ${await response.text()}`);
  }
  return response;
}

async function signIn(server: CoachServer, email: string, password: string): Promise<string> {
  const tokens = await firstPartySignIn({ baseUrl: server.pierreUrl, email, password });
  return tokens.access_token;
}

/**
 * A new coach: registered, approved when the server holds new accounts for
 * approval, answered "I coach others", and with Strava connected through the
 * scraper double — so the app's first screen after sign-in is the group step.
 *
 * Every call makes a new account. The app keeps the step's done flag on the
 * device per user id, and the iOS lane resets only the keychain between
 * attempts, so a retried attempt must sign in as someone the device has never
 * seen.
 */
export async function freshOnboardingCoach(server: CoachServer): Promise<OnboardingCoach> {
  const email = `coach-e2e-${Date.now()}-${Math.floor(Math.random() * 1e6)}@pierre.dev`;
  const registered = await expectOk(
    await call(server, '/api/auth/register', {
      method: 'POST',
      body: JSON.stringify({ email, password: COACH_PASSWORD, display_name: 'Coach e2e' }),
    }),
    'register',
  );
  const { user_id: userId, user_status: userStatus } = (await registered.json()) as {
    user_id: string;
    user_status: string;
  };
  if (userStatus === 'pending') {
    const admin = await signIn(server, server.adminEmail, server.adminPassword);
    await expectOk(
      await call(
        server,
        `/api/admin/approve-user/${userId}`,
        { method: 'POST', body: JSON.stringify({ reason: 'e2e onboarding coach' }) },
        admin,
      ),
      'approve-user',
    );
  }

  const token = await signIn(server, email, COACH_PASSWORD);
  bearers.set(email, token);
  await expectOk(
    await call(server, '/api/user/coaching-role', { method: 'PUT', body: JSON.stringify({ coaches_others: true }) }, token),
    'coaching-role',
  );
  await expectOk(
    await call(
      server,
      '/api/providers/sciotte/login',
      {
        method: 'POST',
        body: JSON.stringify({
          email: 'coach@strava.example',
          password: 'never-real',
          method: 'email',
          target: 'strava',
          tos_consent: true,
        }),
      },
      token,
    ),
    'sciotte login (is the server started with DRAVR_SCIOTTE_REMOTE_URL naming the double?)',
  );

  // The flow's premise, checked where it is decided: the server says this
  // coach is past the provider gate and coaches others, with no group step
  // recorded. Anything else would land the app on another screen and the
  // flow would fail on the wrong question.
  const onboarding = (await (
    await expectOk(await call(server, '/api/me/onboarding-status', {}, token), 'onboarding-status')
  ).json()) as { needs_provider_connection: boolean; coaches_others: boolean; steps: { step_id: string }[] };
  if (
    onboarding.needs_provider_connection ||
    !onboarding.coaches_others ||
    onboarding.steps.some((step) => step.step_id === 'coach_group')
  ) {
    throw new Error(`the coach is not waiting on the group step: ${JSON.stringify(onboarding)}`);
  }
  return { email, password: COACH_PASSWORD, groupName: ONBOARDING_GROUP_NAME };
}

/**
 * What the group step left on the server for `email`. The app records the
 * step without waiting for the answer, so the read is repeated until the step
 * reads `expectStep` or `withinMs` runs out; the last read is returned either
 * way and the flow judges it.
 */
export async function coachStepState(
  server: CoachServer,
  email: string,
  expectStep: string,
  withinMs = 15_000,
): Promise<CoachStepState> {
  const token = bearers.get(email);
  if (token === undefined) {
    throw new Error(`${email} is not a coach this control made`);
  }
  const deadline = Date.now() + withinMs;
  let state: CoachStepState;
  do {
    const onboarding = (await (
      await expectOk(await call(server, '/api/me/onboarding-status', {}, token), 'onboarding-status')
    ).json()) as { steps: { step_id: string; status: string }[] };
    const access = (await (
      await expectOk(await call(server, '/api/me/coach-access-request', {}, token), 'coach-access-request')
    ).json()) as { request: { status: string; group_id: string | null } | null };
    const threads = (await (
      await expectOk(await call(server, '/api/chat/conversations?limit=50&offset=0', {}, token), 'conversations')
    ).json()) as { conversations: { group_id?: string | null }[] };
    const groupThreads = threads.conversations.filter((c) => c.group_id);
    const askedFor = access.request?.group_id ?? null;
    state = {
      step: onboarding.steps.find((s) => s.step_id === 'coach_group')?.status ?? 'none',
      access: access.request?.status ?? 'none',
      groupThread: askedFor !== null && groupThreads.some((c) => c.group_id === askedFor),
      groupThreads: groupThreads.length,
    };
    if (state.step === expectStep) {
      return state;
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  } while (Date.now() < deadline);
  return state;
}
