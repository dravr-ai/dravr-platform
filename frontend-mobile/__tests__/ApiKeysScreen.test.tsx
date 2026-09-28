// ABOUTME: Tests the API keys pane — a row per active key with its 30-day use, the key shown once after creation, revoke removing it
// ABOUTME: Mocks apiKeysApi and the auth context; revocation is confirmed through the mocked Alert's destructive button

import React from 'react';
import { Alert } from 'react-native';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import type { ApiKeyInfo } from '@pierre/shared-types';

const mockList = jest.fn();
const mockCreate = jest.fn();
const mockRevoke = jest.fn();
const mockUsage = jest.fn();
jest.mock('../src/services/api', () => ({
  apiKeysApi: {
    list: () => mockList(),
    create: (...args: unknown[]) => mockCreate(...args),
    revoke: (id: string) => mockRevoke(id),
    usage: (...args: unknown[]) => mockUsage(...args),
  },
}));

jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true, user: { id: 'user-1', role: 'user' } }),
}));

import { ApiKeysScreen } from '../src/screens/settings/ApiKeysScreen';

function key(id: string, name: string, overrides: Partial<ApiKeyInfo> = {}): ApiKeyInfo {
  return {
    id,
    name,
    description: null,
    tier: 'starter',
    key_prefix: `pk_live_${id}`,
    is_active: true,
    last_used_at: null,
    expires_at: null,
    created_at: '2026-09-20T08:30:00Z',
    ...overrides,
  };
}

describe('ApiKeysScreen', () => {
  let keys: ApiKeyInfo[];

  beforeEach(() => {
    jest.clearAllMocks();
    keys = [key('k1', 'Export script'), key('k0', 'Old key', { is_active: false })];
    mockList.mockImplementation(async () => ({ api_keys: keys }));
    mockUsage.mockResolvedValue({ stats: { total_requests: 7 } });
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('lists each active key with its prefix and 30-day use, never a revoked one', async () => {
    const { findByTestId, queryByTestId } = render(<ApiKeysScreen />);

    const row = await findByTestId('api-key-row-k1');
    await waitFor(() =>
      expect(row).toHaveTextContent('pk_live_k1… · 7 requests in the last 30 days', { exact: false }),
    );
    expect(queryByTestId('api-key-row-k0')).toBeNull();
  });

  it('shows a created key once and lists it', async () => {
    mockCreate.mockImplementation(async () => {
      keys = [...keys, key('k2', 'Garmin sync')];
      return { api_key: 'pk_live_k2_full_secret_value', key_info: keys[2], warning: 'Store it.' };
    });
    const { findByTestId, getByTestId, queryByTestId, getAllByTestId } = render(<ApiKeysScreen />);

    await findByTestId('api-key-row-k1');
    fireEvent.press(getAllByTestId('new-api-key-button')[0]);
    fireEvent.changeText(getByTestId('new-api-key-name'), 'Garmin sync');
    fireEvent.press(getByTestId('create-api-key-confirm'));

    expect(await findByTestId('created-api-key-value')).toHaveTextContent('pk_live_k2_full_secret_value');
    expect(mockCreate).toHaveBeenCalledWith({ name: 'Garmin sync' });
    expect(await findByTestId('api-key-row-k2')).toBeTruthy();

    fireEvent.press(getByTestId('api-key-created-done'));
    await waitFor(() => expect(queryByTestId('created-api-key-value')).toBeNull());
  });

  it('revokes a key after confirmation and it leaves the list', async () => {
    const alert = jest.spyOn(Alert, 'alert').mockImplementation((_title, _message, buttons) => {
      buttons?.find((button) => button.style === 'destructive')?.onPress?.();
    });
    mockRevoke.mockResolvedValue({ message: 'revoked', deactivated_at: '2026-09-28T10:00:00Z' });
    const { findByTestId, queryByTestId } = render(<ApiKeysScreen />);

    fireEvent.press(await findByTestId('revoke-api-key-k1'));

    expect(alert).toHaveBeenCalledWith('Revoke Export script?', expect.any(String), expect.any(Array));
    await waitFor(() => expect(mockRevoke).toHaveBeenCalledWith('k1'));
    await waitFor(() => expect(queryByTestId('api-key-row-k1')).toBeNull());
    expect(await findByTestId('api-key-empty')).toBeTruthy();
  });
});
