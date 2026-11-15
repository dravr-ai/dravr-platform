// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The token list reads the server for whoever UnifiedConnections opens it for, and only an admin opens that
// ABOUTME: A query left on `is_admin` stayed disabled for an admin whose flag disagreed, who then read an empty list

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, waitFor } from '@testing-library/react';
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

  it('fetches for an admin whose is_admin flag is false', async () => {
    renderList();
    await waitFor(() => expect(getAdminTokens).toHaveBeenCalledTimes(1));
    expect(getAdminTokens).toHaveBeenCalledWith({ include_inactive: true });
  });
});
