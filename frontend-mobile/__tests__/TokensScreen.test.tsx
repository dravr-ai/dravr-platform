// ABOUTME: Tests the API Tokens pane — the empty state with its one ink action, a token row's mono prefix and ink revoke, the load error with its retry
// ABOUTME: Mocks userApi and the auth context; the minting Sheet is driven through its confirm and the created-token step

import React from 'react';
import { Alert } from 'react-native';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import type { McpToken } from '@pierre/shared-types';

const mockGetMcpTokens = jest.fn();
const mockCreateMcpToken = jest.fn();
const mockRevokeMcpToken = jest.fn();
jest.mock('../src/services/api', () => ({
  userApi: {
    getMcpTokens: () => mockGetMcpTokens(),
    createMcpToken: (...args: unknown[]) => mockCreateMcpToken(...args),
    revokeMcpToken: (id: string) => mockRevokeMcpToken(id),
  },
}));

jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true, user: { id: 'user-1', role: 'user' } }),
}));

import { TokensScreen } from '../src/screens/settings/TokensScreen';

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

describe('TokensScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('renders the empty sentence with the one way to mint a token', async () => {
    mockGetMcpTokens.mockResolvedValue({ tokens: [] });
    const { getByTestId, getByText, getAllByTestId, queryByTestId } = render(<TokensScreen />);

    await waitFor(() => expect(getByTestId('mcp-token-empty')).toBeTruthy());
    expect(getByText(i18n.t('tokens.empty'))).toBeTruthy();
    expect(getByTestId('mcp-token-list')).toBeTruthy();
    // One `new-token-button`, on the empty state's ink action — the Section
    // header carries none while there is nothing to list.
    expect(getAllByTestId('new-token-button')).toHaveLength(1);
    expect(getByTestId('new-token-button').props.children).toEqual(i18n.t('app.newToken'));
    expect(queryByTestId('tokens-load-error')).toBeNull();
  });

  it('renders each active token with its mono prefix line and an ink revoke', async () => {
    mockGetMcpTokens.mockResolvedValue({
      tokens: [DESKTOP_TOKEN, LAPTOP_TOKEN, { ...LAPTOP_TOKEN, id: 'tok-gone', is_revoked: true }],
    });
    const { getByTestId, getByText, getAllByTestId, queryByTestId } = render(<TokensScreen />);

    await waitFor(() => expect(getByTestId('mcp-token-row-tok-desktop')).toBeTruthy());
    expect(getByTestId('mcp-token-row-tok-laptop')).toBeTruthy();
    // A revoked token is not an active one, whatever the server sent.
    expect(queryByTestId('mcp-token-row-tok-gone')).toBeNull();
    expect(getByText('Claude Desktop')).toBeTruthy();

    const prefix = getByText(i18n.t('app.tokenPrefixUsage', { prefix: 'pk_live_c3d4', uses: 0 }));
    expect(prefix.props.className).toContain('font-mono');
    expect(prefix.props.className).toContain('tabular-nums');

    const revoke = getByTestId('revoke-token-tok-laptop');
    expect(revoke.props.className).toContain('text-primary');
    expect(revoke.props.children).toEqual(i18n.t('app.revoke'));
    // The mint action moves to the Section header, and stays the only one.
    expect(getAllByTestId('new-token-button')).toHaveLength(1);
  });

  it('confirms before revoking, then takes the token off the list', async () => {
    mockGetMcpTokens.mockResolvedValue({ tokens: [DESKTOP_TOKEN, LAPTOP_TOKEN] });
    mockRevokeMcpToken.mockResolvedValue({ success: true });
    const { getByTestId, queryByTestId } = render(<TokensScreen />);
    await waitFor(() => expect(getByTestId('revoke-token-tok-laptop')).toBeTruthy());

    fireEvent.press(getByTestId('revoke-token-tok-laptop'));

    const confirm = (Alert.alert as jest.Mock).mock.calls.at(-1) as [
      string,
      string,
      Array<{ text: string; style?: string; onPress?: () => void }>,
    ];
    expect(confirm[0]).toBe(i18n.t('app.revokeTokenTitle'));
    expect(confirm[1]).toContain('Vieux portable');
    confirm[2].find((button) => button.style === 'destructive')?.onPress?.();

    await waitFor(() => expect(mockRevokeMcpToken).toHaveBeenCalledWith('tok-laptop'));
    await waitFor(() => expect(queryByTestId('mcp-token-row-tok-laptop')).toBeNull());
    expect(getByTestId('mcp-token-row-tok-desktop')).toBeTruthy();
  });

  it('says why the list failed and offers a retry that asks again', async () => {
    mockGetMcpTokens.mockRejectedValueOnce(new Error('offline'));
    mockGetMcpTokens.mockResolvedValueOnce({ tokens: [DESKTOP_TOKEN] });
    const { getByTestId, getByText, queryByTestId } = render(<TokensScreen />);

    await waitFor(() => expect(getByTestId('tokens-load-error')).toBeTruthy());
    expect(getByText('offline')).toBeTruthy();
    expect(getByTestId('tokens-retry').props.children).toEqual(i18n.t('common.retry'));

    fireEvent.press(getByTestId('tokens-retry'));

    await waitFor(() => expect(getByTestId('mcp-token-row-tok-desktop')).toBeTruthy());
    expect(queryByTestId('tokens-load-error')).toBeNull();
    expect(mockGetMcpTokens).toHaveBeenCalledTimes(2);
  });

  it('mints a token from the sheet and shows its value once, with no pill button anywhere', async () => {
    mockGetMcpTokens.mockResolvedValue({ tokens: [] });
    mockCreateMcpToken.mockResolvedValue({ ...DESKTOP_TOKEN, token_value: 'pk_live_a1b2_full_secret' });
    const { getByTestId, toJSON } = render(<TokensScreen />);
    await waitFor(() => expect(getByTestId('new-token-button')).toBeTruthy());

    fireEvent.press(getByTestId('new-token-button'));
    await waitFor(() => expect(getByTestId('create-token-sheet')).toBeTruthy());

    fireEvent.changeText(getByTestId('new-token-name'), 'Claude Desktop');
    fireEvent.press(getByTestId('create-token-confirm'));

    await waitFor(() => expect(mockCreateMcpToken).toHaveBeenCalledWith({ name: 'Claude Desktop', expires_in_days: 365 }));
    await waitFor(() => expect(getByTestId('created-token-value')).toBeTruthy());
    expect(getByTestId('created-token-value').props.children).toEqual('pk_live_a1b2_full_secret');
    expect(getByTestId('created-token-value').props.className).toContain('font-mono');
    expect(getByTestId('token-created-done')).toBeTruthy();
    expect(JSON.stringify(toJSON())).not.toContain('rounded-full');
  });

  it('refuses to mint without a name', async () => {
    mockGetMcpTokens.mockResolvedValue({ tokens: [] });
    const { getByTestId } = render(<TokensScreen />);
    await waitFor(() => expect(getByTestId('new-token-button')).toBeTruthy());

    fireEvent.press(getByTestId('new-token-button'));
    await waitFor(() => expect(getByTestId('create-token-confirm')).toBeTruthy());
    fireEvent.press(getByTestId('create-token-confirm'));

    expect(Alert.alert).toHaveBeenCalledWith(i18n.t('common.error'), i18n.t('app.pleaseEnterTokenName'));
    expect(mockCreateMcpToken).not.toHaveBeenCalled();
  });
});
