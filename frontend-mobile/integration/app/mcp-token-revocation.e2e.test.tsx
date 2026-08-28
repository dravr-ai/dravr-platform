// ABOUTME: carnet #64 e2e — mobile can now list and revoke the MCP tokens it mints, behind the api_tokens gate
// ABOUTME: Mounts the real SettingsScreen over a stubbed transport, so the revoke DELETE is asserted on the wire

import React from 'react';
import { render, fireEvent, waitFor, act } from '@testing-library/react-native';
import { Alert } from 'react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { McpToken, MeFeaturesResponse } from '@pierre/shared-types';

import { installHttpStub, type HttpStub } from './helpers/httpStub';

jest.mock('expo-router', () => ({
  useRouter: () => ({
    push: jest.fn(),
    replace: jest.fn(),
    back: jest.fn(),
    navigate: jest.fn(),
    canGoBack: () => true,
  }),
  useLocalSearchParams: () => ({}),
  useSegments: () => [],
  usePathname: () => '/settings',
  useFocusEffect: () => undefined,
}));

jest.mock('../../src/contexts/AuthContext', () => ({
  useAuth: () => ({
    user: {
      id: 'user-1',
      email: 'athlete@dravr.ai',
      display_name: 'ChefFamille',
      role: 'user',
      user_status: 'active',
      locale: 'fr',
    },
    isAuthenticated: true,
    logout: jest.fn(),
    updateUser: jest.fn(),
  }),
}));

jest.mock('../../src/screens/chat/useUsageStatus', () => ({
  useUsageStatus: () => ({ data: null, isLoading: false }),
}));

import { SettingsScreen } from '../../src/screens/settings/SettingsScreen';

const DESKTOP_TOKEN: McpToken = {
  id: 'tok-desktop',
  name: 'Claude Desktop',
  token_prefix: 'pk_live_a1b2',
  expires_at: '2027-08-01T00:00:00Z',
  last_used_at: '2026-08-20T09:00:00Z',
  usage_count: 42,
  is_revoked: false,
  created_at: '2026-07-01T09:00:00Z',
};

const LAPTOP_TOKEN: McpToken = {
  id: 'tok-laptop',
  name: 'Vieux portable',
  token_prefix: 'pk_live_c3d4',
  expires_at: null,
  last_used_at: null,
  usage_count: 0,
  is_revoked: false,
  created_at: '2026-06-14T09:00:00Z',
};

function features(apiTokens: boolean): MeFeaturesResponse {
  return {
    flags: { api_tokens: apiTokens, billing_header: false },
    known: [
      { key: 'api_tokens', description: 'Personal MCP bearer tokens', default_enabled: false },
    ],
  };
}

function renderSettings() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={client}>
      <SettingsScreen />
    </QueryClientProvider>,
  );
}

describe('carnet #64 — mobile MCP token revocation', () => {
  let stub: HttpStub;
  let listedTokens: McpToken[];

  beforeEach(() => {
    listedTokens = [DESKTOP_TOKEN, LAPTOP_TOKEN];
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
  });

  afterEach(() => {
    stub.restore();
    jest.restoreAllMocks();
  });

  it('lists every active token and revokes the one the athlete picked', async () => {
    stub = installHttpStub({
      'GET /api/me/features': { data: features(true) },
      'GET /api/user/mcp-tokens': () => ({ data: { tokens: listedTokens } }),
      'GET /api/oauth/status': { data: { providers: [] } },
      'DELETE /api/user/mcp-tokens/tok-laptop': () => {
        listedTokens = listedTokens.filter((token) => token.id !== 'tok-laptop');
        return { data: { success: true } };
      },
    });

    const { getByTestId, getByText, queryByTestId } = renderSettings();

    // The row is only rendered once the gate answers "on", and it counts the
    // tokens the server actually returned.
    await waitFor(() => {
      expect(getByTestId('settings-mcp-tokens-button')).toBeTruthy();
    });
    expect(getByText('2 active')).toBeTruthy();

    await act(async () => {
      fireEvent.press(getByTestId('settings-mcp-tokens-button'));
    });

    expect(getByTestId('mcp-token-row-tok-desktop')).toBeTruthy();
    expect(getByTestId('mcp-token-row-tok-laptop')).toBeTruthy();
    expect(getByText('Claude Desktop')).toBeTruthy();
    expect(getByText('pk_live_c3d4… · used 0×')).toBeTruthy();

    fireEvent.press(getByTestId('revoke-token-tok-laptop'));

    const confirm = (Alert.alert as jest.Mock).mock.calls.at(-1) as [
      string,
      string,
      Array<{ text: string; style?: string; onPress?: () => void }>,
    ];
    expect(confirm[0]).toBe('Revoke Token');
    expect(confirm[1]).toContain('Vieux portable');

    // Found by its ROLE, not its label. The label is a corpus string now, and
    // this screen renders in the app's default locale — French — so matching
    // on "Revoke" silently found nothing and `?.onPress?.()` swallowed it: the
    // test passed its earlier assertions and then reported no DELETE at all.
    const confirmButton = confirm[2].find((button) => button.style === 'destructive');
    expect(confirmButton).toBeDefined();

    await act(async () => {
      await confirmButton?.onPress?.();
    });

    // The revoke hit the per-token endpoint...
    await waitFor(() => {
      expect(stub.requestsFor('DELETE').map((request) => request.url)).toEqual([
        '/api/user/mcp-tokens/tok-laptop',
      ]);
    });

    // ...and the row it revoked is gone, while the other one stays.
    await waitFor(() => {
      expect(queryByTestId('mcp-token-row-tok-laptop')).toBeNull();
    });
    expect(getByTestId('mcp-token-row-tok-desktop')).toBeTruthy();
  });

  it('renders no token surface at all while api_tokens is off', async () => {
    stub = installHttpStub({
      'GET /api/me/features': { data: features(false) },
      'GET /api/oauth/status': { data: { providers: [] } },
    });

    const { getByTestId, queryByTestId } = renderSettings();

    // Wait on a row that renders for everyone before asserting the absence.
    await waitFor(() => {
      expect(getByTestId('settings-change-password-button')).toBeTruthy();
    });
    expect(queryByTestId('settings-mcp-tokens-button')).toBeNull();

    // With the gate closed the screen must not even ask for the token list —
    // an unstubbed GET would throw out of the stub adapter.
    expect(stub.requests.map((request) => request.url)).not.toContain('/api/user/mcp-tokens');
  });
});
