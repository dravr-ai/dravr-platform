// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: carnet#1201 e2e — the E2E seeded-session link, under the real root layout, redeems its code, stores the minted pair and lands on Home
// ABOUTME: The real AuthProvider and api-client over a stubbed transport and control process; a keychain already holding a session is left untouched

import React from 'react';
import { render, screen, waitFor } from '@testing-library/react-native';

import { installHttpStub, type HttpStub } from './helpers/httpStub';

const mockReplace = jest.fn();
const mockParams = { code: 'one-time-code' };
// The keychain the API client keeps its tokens in, as a device holds it.
const mockKeychain = new Map<string, string>();

jest.mock('expo-secure-store', () => ({
  getItemAsync: jest.fn((key: string) => Promise.resolve(mockKeychain.get(key) ?? null)),
  setItemAsync: jest.fn((key: string, value: string) => {
    mockKeychain.set(key, value);
    return Promise.resolve();
  }),
  deleteItemAsync: jest.fn((key: string) => {
    mockKeychain.delete(key);
    return Promise.resolve();
  }),
}));

// NativeWind's own hook needs a stub under jest, as in the theme's unit tests.
jest.mock('nativewind', () => ({
  ...jest.requireActual('nativewind'),
  useColorScheme: () => ({ colorScheme: 'dark', setColorScheme: jest.fn() }),
}));

// The onboarding gate as the real hook reports it for the seeded tester, an
// admin: exempt, so no step stands between the session and Home. The hook's
// own server reads run on a react-query client whose timers outlive the
// render and keep Jest from exiting; the step registry it feeds has its own
// tests, and the guard that reads it is what this spec runs.
jest.mock('../../src/hooks/useOnboardingContext', () => ({
  useOnboardingContext: () => ({
    context: {
      onboardingActive: false,
      needsProviderConnection: false,
      skippedProvider: false,
      justOnboarded: false,
      connectProviderDone: false,
    },
    settled: true,
  }),
}));

// The link opened /e2e/session: the root layout renders that route in its
// slot, and its guard reads the segments to decide where the account goes.
jest.mock('expo-router', () =>
  require('../../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({
      push: jest.fn(),
      replace: mockReplace,
      back: jest.fn(),
      navigate: jest.fn(),
      canGoBack: () => false,
    }),
    useSegments: () => ['(auth)', 'e2e', 'session'],
    useLocalSearchParams: () => mockParams,
    useNavigationContainerRef: () => ({
      addListener: () => () => undefined,
      isReady: () => false,
      getCurrentRoute: () => undefined,
    }),
    Slot: () => {
      const SeededSessionRoute = require('../../app/(auth)/e2e/session').default;
      return require('react').createElement(SeededSessionRoute);
    },
  }),
);

import RootLayout from '../../app/_layout';
import { HOME_ROUTE } from '../../src/navigation/routes';

/** The seeded tester the CI lanes create with `pierre-cli user create`: an admin, so no onboarding stands between it and Home. */
const TESTER = {
  id: '5d6f0a52-3c1e-4f7b-9a2d-1b8e4c7f6a90',
  user_id: '5d6f0a52-3c1e-4f7b-9a2d-1b8e4c7f6a90',
  email: 'mobiletest@pierre.dev',
  display_name: 'Mobile Test',
  is_admin: true,
  role: 'admin',
  user_status: 'active',
  locale: 'en',
};

/** The redemption the route sends the control process (frontend/e2e-real/home-sync-control.ts) on the API's host. */
const REDEEM = 'http://127.0.0.1:8098/seeded-session/redeem?code=one-time-code';

/** The control process as the route reaches it: the one code it issued redeems once, then is spent. */
function redeemOnce() {
  let spent = false;
  return () => {
    if (spent) {
      return { status: 410, data: { error: 'the code is unknown, already redeemed or expired' } };
    }
    spent = true;
    return { data: { access_token: 'minted-access', refresh_token: 'minted-refresh' } };
  };
}

describe('the seeded-session link in an E2E bundle', () => {
  const FLAG = 'EXPO_PUBLIC_E2E_SEEDED_SESSIONS';
  const API_URL = 'EXPO_PUBLIC_API_URL';
  let stub: HttpStub;
  const redemptions = () => stub.requestsFor('POST').filter((request) => request.url.includes(':8098/'));

  beforeEach(() => {
    process.env[FLAG] = '1';
    process.env[API_URL] = 'http://127.0.0.1:8081';
    mockKeychain.clear();
    mockReplace.mockClear();
    stub = installHttpStub({
      'GET /api/auth/session': {
        data: { user: TESTER, access_token: 'renewed-access', csrf_token: 'csrf-1' },
      },
      // The sign-in's best-effort timezone write, as after any sign-in.
      'PUT /api/users/me/timezone': { data: {} },
      [`POST ${REDEEM}`]: redeemOnce(),
    });
  });

  afterEach(() => {
    stub.restore();
    delete process.env[FLAG];
    delete process.env[API_URL];
  });

  it('redeems the code once, stores the pair, has the server confirm it, and the guard lands the account on Home', async () => {
    render(<RootLayout />);

    await waitFor(() => expect(mockReplace).toHaveBeenCalledWith(HOME_ROUTE), { timeout: 10000 });

    expect(redemptions().map((request) => request.url)).toEqual([REDEEM]);

    const sessionReads = stub.requestsFor('GET').filter((request) => request.url === '/api/auth/session');
    expect(sessionReads).toHaveLength(1);
    expect(sessionReads[0].headers.authorization).toBe('Bearer minted-access');
    const stored = [...mockKeychain.values()];
    expect(stored).toContain('minted-refresh');
    expect(stored).toContain('renewed-access');
    expect(stored).not.toContain('minted-access');
  });

  it('without the E2E flag reads nothing from the link and signs nobody in', async () => {
    delete process.env[FLAG];

    render(<RootLayout />);

    // Long enough for a session read and the guard's hand-over, had either started.
    await new Promise((resolve) => setTimeout(resolve, 500));
    expect(redemptions()).toHaveLength(0);
    expect(stub.requestsFor('GET').filter((request) => request.url === '/api/auth/session')).toHaveLength(0);
    expect([...mockKeychain.values()]).toEqual([]);
    expect(mockReplace).not.toHaveBeenCalledWith(HOME_ROUTE);
  });

  it('leaves a keychain that already holds a session as it is, and spends no code', async () => {
    // A refresh token with no stored user: the guard sees nobody signed in,
    // so only the route's own check stands between the link and the swap.
    mockKeychain.set('pierre.refresh_token', 'someone-elses-refresh');

    render(<RootLayout />);

    expect(await screen.findByTestId('e2e-seeded-session-failure', {}, { timeout: 10000 })).toBeTruthy();
    expect(redemptions()).toHaveLength(0);
    expect(stub.requestsFor('GET').filter((request) => request.url === '/api/auth/session')).toHaveLength(0);
    expect([...mockKeychain.entries()]).toEqual([['pierre.refresh_token', 'someone-elses-refresh']]);
    expect(mockReplace).not.toHaveBeenCalledWith(HOME_ROUTE);
  });
});
