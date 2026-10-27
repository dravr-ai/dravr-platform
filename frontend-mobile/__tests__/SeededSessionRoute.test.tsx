// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the E2E-only seeded-session route: it redeems its code at the control process, stores the pair and adopts the confirmed session
// ABOUTME: Refused, with nothing written, outside a dev E2E bundle on a local API, for a spent code, or while a session already exists (carnet#1201)

import React from 'react';
import { act, render, waitFor } from '@testing-library/react-native';

const mockParams: { code?: string | string[] } = {};
const mockStorage = {
  getToken: jest.fn((): Promise<string | null> => Promise.resolve(null)),
  getRefreshToken: jest.fn((): Promise<string | null> => Promise.resolve(null)),
  setToken: jest.fn((_token: string) => Promise.resolve()),
  setRefreshToken: jest.fn((_token: string) => Promise.resolve()),
};
const mockGetSession = jest.fn();
const mockAdoptSession = jest.fn((_session: unknown) => Promise.resolve());
// The context's state as a store the mocked hook subscribes to, so a change
// re-renders the route the way a provider update does.
const mockAuth = {
  state: { isAuthenticated: false, isLoading: false },
  listeners: new Set<() => void>(),
  set(patch: Partial<{ isAuthenticated: boolean; isLoading: boolean }>) {
    this.state = { ...this.state, ...patch };
    this.listeners.forEach((listener) => listener());
  },
};

jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useLocalSearchParams: () => mockParams,
    Redirect: ({ href }: { href: string }) => {
      const { Text: RNText } = require('react-native');
      return require('react').createElement(RNText, { testID: 'redirect' }, href);
    },
  }),
);

// Read through at call time: the module is required before the consts above are set.
jest.mock('../src/services/api', () => ({
  authStorage: {
    getToken: () => mockStorage.getToken(),
    getRefreshToken: () => mockStorage.getRefreshToken(),
    setToken: (token: string) => mockStorage.setToken(token),
    setRefreshToken: (token: string) => mockStorage.setRefreshToken(token),
  },
  authApi: { getSession: () => mockGetSession() },
}));

jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => {
    const state = require('react').useSyncExternalStore(
      (listener: () => void) => {
        mockAuth.listeners.add(listener);
        return () => mockAuth.listeners.delete(listener);
      },
      () => mockAuth.state,
    );
    return { adoptSession: mockAdoptSession, ...state };
  },
}));

import SeededSessionRoute from '../app/(auth)/e2e/session';

const FLAG = 'EXPO_PUBLIC_E2E_SEEDED_SESSIONS';
const API_URL = 'EXPO_PUBLIC_API_URL';
const SESSION = {
  access_token: 'renewed-access',
  csrf_token: 'csrf-1',
  user: { id: 'user-1', email: 'mobiletest@pierre.dev', user_status: 'active' },
};
const PAIR = { access_token: 'minted-access', refresh_token: 'minted-refresh' };

/** The control process's answer to a redemption, as fetch resolves it. */
function controlAnswer(status: number, body: unknown) {
  return Promise.resolve({ ok: status >= 200 && status < 300, status, json: () => Promise.resolve(body) });
}

/** Let any effect run before asserting it never wrote. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

function expectNothingWritten() {
  expect(mockStorage.setToken).not.toHaveBeenCalled();
  expect(mockStorage.setRefreshToken).not.toHaveBeenCalled();
  expect(mockGetSession).not.toHaveBeenCalled();
  expect(mockAdoptSession).not.toHaveBeenCalled();
}

describe('the seeded-session route', () => {
  const original = { flag: process.env[FLAG], api: process.env[API_URL] };
  const fetchMock = jest.fn();
  const realFetch = global.fetch;

  beforeEach(() => {
    jest.clearAllMocks();
    mockParams.code = 'one-time-code';
    mockAuth.state = { isAuthenticated: false, isLoading: false };
    mockStorage.getToken.mockResolvedValue(null);
    mockStorage.getRefreshToken.mockResolvedValue(null);
    mockGetSession.mockResolvedValue(SESSION);
    fetchMock.mockImplementation(() => controlAnswer(200, PAIR));
    global.fetch = fetchMock as unknown as typeof fetch;
    process.env[FLAG] = '1';
    process.env[API_URL] = 'http://127.0.0.1:8081';
  });

  afterEach(() => {
    global.fetch = realFetch;
    (global as unknown as { __DEV__: boolean }).__DEV__ = true;
    for (const [key, value] of [
      [FLAG, original.flag],
      [API_URL, original.api],
    ] as const) {
      if (value === undefined) {
        delete process.env[key];
      } else {
        process.env[key] = value;
      }
    }
  });

  it('redeems the code at the control process on the API host, stores the pair, then adopts the confirmed session', async () => {
    const view = render(<SeededSessionRoute />);

    await waitFor(() => expect(mockAdoptSession).toHaveBeenCalledWith(SESSION));
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith('http://127.0.0.1:8098/seeded-session/redeem?code=one-time-code', {
      method: 'POST',
    });
    expect(mockStorage.setToken).toHaveBeenCalledWith('minted-access');
    expect(mockStorage.setRefreshToken).toHaveBeenCalledWith('minted-refresh');
    // The session is read only once the pair is stored: the request rides on it.
    expect(mockStorage.setRefreshToken.mock.invocationCallOrder[0]).toBeLessThan(
      mockGetSession.mock.invocationCallOrder[0],
    );
    expect(view.queryByTestId('redirect')).toBeNull();
    expect(view.queryByTestId('e2e-seeded-session-failure')).toBeNull();
  });

  it('reaches the control process through the Android emulator alias when the API is there', async () => {
    process.env[API_URL] = 'http://10.0.2.2:8081';

    render(<SeededSessionRoute />);

    await waitFor(() => expect(mockAdoptSession).toHaveBeenCalled());
    expect(fetchMock.mock.calls[0][0]).toBe('http://10.0.2.2:8098/seeded-session/redeem?code=one-time-code');
  });

  it('without the E2E flag refuses the link to the login screen and stores nothing', async () => {
    delete process.env[FLAG];

    const view = render(<SeededSessionRoute />);

    expect(view.getByTestId('redirect').props.children).toBe('/(auth)/login');
    await settle();
    expect(fetchMock).not.toHaveBeenCalled();
    expectNothingWritten();
  });

  it('refuses a flag set to anything but 1', async () => {
    process.env[FLAG] = 'true';

    const view = render(<SeededSessionRoute />);

    expect(view.getByTestId('redirect').props.children).toBe('/(auth)/login');
    await settle();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('refuses a bundle whose API is a real server, so a link cannot swap accounts there', async () => {
    process.env[API_URL] = 'https://app.dravr.ai';

    const view = render(<SeededSessionRoute />);

    expect(view.getByTestId('redirect').props.children).toBe('/(auth)/login');
    await settle();
    expect(fetchMock).not.toHaveBeenCalled();
    expectNothingWritten();
  });

  it('refuses outside a development build', async () => {
    (global as unknown as { __DEV__: boolean }).__DEV__ = false;

    const view = render(<SeededSessionRoute />);

    expect(view.getByTestId('redirect').props.children).toBe('/(auth)/login');
    await settle();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('writes nothing, and spends no code, while a session already exists in the keychain', async () => {
    mockStorage.getRefreshToken.mockResolvedValue('someone-elses-refresh');

    const view = render(<SeededSessionRoute />);

    const failure = await view.findByTestId('e2e-seeded-session-failure');
    expect(failure.props.children).toBe('a session already exists on this app, so the link was refused');
    expect(fetchMock).not.toHaveBeenCalled();
    expectNothingWritten();
  });

  it('writes nothing while the context holds a signed-in account', async () => {
    mockAuth.state = { isAuthenticated: true, isLoading: false };

    const view = render(<SeededSessionRoute />);

    await view.findByTestId('e2e-seeded-session-failure');
    expect(fetchMock).not.toHaveBeenCalled();
    expectNothingWritten();
  });

  it('waits for the stored session to load before deciding', async () => {
    mockAuth.state = { isAuthenticated: false, isLoading: true };

    render(<SeededSessionRoute />);
    await settle();
    expect(mockStorage.getToken).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();

    act(() => mockAuth.set({ isLoading: false }));
    await waitFor(() => expect(mockAdoptSession).toHaveBeenCalledWith(SESSION));
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('names a code the control process refused, spent or expired, and writes nothing', async () => {
    fetchMock.mockImplementation(() => controlAnswer(410, { error: 'the code is unknown, already redeemed or expired' }));

    const view = render(<SeededSessionRoute />);

    const failure = await view.findByTestId('e2e-seeded-session-failure');
    expect(failure.props.children).toBe('the control process refused the code (status 410)');
    expectNothingWritten();
  });

  it('names a session the server refused, without the token', async () => {
    mockGetSession.mockRejectedValue(Object.assign(new Error('401'), { response: { status: 401 } }));

    const view = render(<SeededSessionRoute />);

    const failure = await view.findByTestId('e2e-seeded-session-failure');
    expect(failure.props.children).toBe('the seeded session failed (status 401)');
    expect(mockAdoptSession).not.toHaveBeenCalled();
  });

  it('refuses a link that carries no code', async () => {
    delete mockParams.code;

    const view = render(<SeededSessionRoute />);

    expect(view.getByTestId('e2e-seeded-session-failure').props.children).toBe('the link carries no code');
    await settle();
    expect(fetchMock).not.toHaveBeenCalled();
    expectNothingWritten();
  });
});
