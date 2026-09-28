// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for Settings › API keys — the key shown once after creation, and a revoked key leaving the list
// ABOUTME: The apiKeys API is mocked at the services barrel; assertions read the rendered rows, the secret and the calls

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ApiKeyInfo } from '@pierre/shared-types';
import ApiKeysSettings from '../ApiKeysSettings';

const list = vi.fn();
const create = vi.fn();
const revoke = vi.fn();
const usage = vi.fn();

vi.mock('../../../services/api', () => ({
  apiKeysApi: {
    list: (...a: unknown[]) => list(...a),
    create: (...a: unknown[]) => create(...a),
    revoke: (...a: unknown[]) => revoke(...a),
    usage: (...a: unknown[]) => usage(...a),
  },
}));

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

function renderPane() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ApiKeysSettings />
    </QueryClientProvider>,
  );
}

describe('ApiKeysSettings', () => {
  let keys: ApiKeyInfo[];

  beforeEach(() => {
    vi.clearAllMocks();
    keys = [key('k1', 'Export script'), key('k0', 'Old key', { is_active: false })];
    list.mockImplementation(async () => ({ api_keys: keys }));
    usage.mockResolvedValue({
      stats: {
        api_key_id: 'k1',
        period_start: '2026-08-29T00:00:00Z',
        period_end: '2026-09-28T00:00:00Z',
        total_requests: 42,
        successful_requests: 40,
        failed_requests: 2,
        total_response_time_ms: 1000,
        tool_usage: {},
      },
    });
  });

  it('lists the active keys with their use, never a revoked one', async () => {
    renderPane();

    const row = await screen.findByTestId('api-key-k1');
    expect(row).toHaveTextContent('Export script');
    expect(await within(row).findByText('42 requests in the last 30 days')).toBeInTheDocument();
    expect(screen.queryByTestId('api-key-k0')).not.toBeInTheDocument();
  });

  it('shows a created key once, then only its prefix', async () => {
    create.mockImplementation(async () => {
      keys = [...keys, key('k2', 'Garmin sync')];
      return { api_key: 'pk_live_k2_full_secret_value', key_info: keys[2], warning: 'Store it.' };
    });
    const user = userEvent.setup();
    renderPane();

    await user.type(await screen.findByTestId('api-key-name'), 'Garmin sync');
    await user.click(screen.getByTestId('api-key-create'));

    expect(create).toHaveBeenCalledWith({ name: 'Garmin sync' });
    expect(await screen.findByTestId('api-key-secret')).toHaveTextContent('pk_live_k2_full_secret_value');
    expect(await screen.findByTestId('api-key-k2')).toHaveTextContent('Garmin sync');

    await user.click(screen.getByTestId('api-key-done'));
    expect(screen.queryByTestId('api-key-secret')).not.toBeInTheDocument();
    expect(screen.queryByText('pk_live_k2_full_secret_value')).not.toBeInTheDocument();
  });

  it('revokes a key after confirmation and it leaves the list', async () => {
    revoke.mockImplementation(async (id: string) => {
      keys = keys.map((k) => (k.id === id ? { ...k, is_active: false } : k));
      return { message: 'revoked', deactivated_at: '2026-09-28T10:00:00Z' };
    });
    const user = userEvent.setup();
    renderPane();

    await user.click(await screen.findByTestId('api-key-revoke-k1'));
    await user.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Revoke' }));

    expect(revoke).toHaveBeenCalledWith('k1');
    expect(await screen.findByTestId('api-keys-empty')).toHaveTextContent('No API keys yet.');
  });
});
