// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The token list reads the server for whoever UnifiedConnections opens it for, and only an admin opens that
// ABOUTME: A rotated token reads as rotated and no bulk revoke takes it; a revoked one stays selectable

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ApiKeyList from '../ApiKeyList';

// The flag and the role disagreed once (carnet#1574): the list reads neither.
const viewer = { role: 'admin', is_admin: false };
const getAdminTokens = vi.hoisted(() => vi.fn());

vi.mock('../../hooks/useAuth', () => ({ useAuth: () => ({ isAuthenticated: true, user: viewer }) }));
vi.mock('../../services/api', () => ({ adminApi: { getAdminTokens } }));

function renderList() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ApiKeyList onViewDetails={vi.fn()} />
    </QueryClientProvider>,
  );
}

describe('ApiKeyList', () => {
  beforeEach(() => {
    getAdminTokens.mockReset();
    getAdminTokens.mockResolvedValue({ tokens: [] });
  });

  it('shows a rotated token as rotated and leaves it out of a bulk revoke', async () => {
    const token = (id: string, is_active: boolean, rotated_to_token_id: string | null) => ({
      id,
      service_name: `service ${id}`,
      permissions: [],
      is_super_admin: false,
      is_active,
      created_at: '2026-11-01T12:00:00Z',
      usage_count: 0,
      token_prefix: `pierre_at_${id}`,
      rotated_to_token_id,
    });
    getAdminTokens.mockResolvedValue({
      tokens: [token('live', true, null), token('old', false, 'live'), token('gone', false, null)],
    });
    renderList();

    // Every token, whatever its status: the filter that counts all three.
    fireEvent.click(await screen.findByRole('button', { name: /\(3\)$/ }));

    // Each token's badge: the rotated one says so, the revoked one does not.
    const badge = (id: string) =>
      screen.getByRole('heading', { name: `service ${id}` }).parentElement?.textContent ?? '';
    expect(badge('old')).toContain('Rotated');
    expect(badge('gone')).toContain('Inactive');
    expect(badge('gone')).not.toContain('Rotated');
    expect(badge('live')).toContain('Active');
    expect(screen.getByLabelText('Select service old')).toBeDisabled();
    expect(screen.getByLabelText('Select service gone')).not.toBeDisabled();
    fireEvent.click(screen.getByLabelText('Select All (2)'));
    expect(screen.getByText('2 selected')).toBeInTheDocument();
    expect(screen.getByLabelText('Select service old')).not.toBeChecked();
  });

  it('fetches for an admin whose is_admin flag is false', async () => {
    renderList();
    await waitFor(() => expect(getAdminTokens).toHaveBeenCalledTimes(1));
    expect(getAdminTokens).toHaveBeenCalledWith({ include_inactive: true });
  });
});
