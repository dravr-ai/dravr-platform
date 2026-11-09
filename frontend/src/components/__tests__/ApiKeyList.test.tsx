// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The token list reads the server for whoever UnifiedConnections opens it for: gated on `role`
// ABOUTME: A query left on `is_admin` stays disabled for a skewed operator, who then reads an empty list

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ApiKeyList from '../ApiKeyList';

const viewer = vi.hoisted(() => ({ role: 'admin', is_admin: false }));
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

  it('fetches for a role admin whose is_admin is false', async () => {
    viewer.role = 'admin';
    viewer.is_admin = false;
    renderList();
    await waitFor(() => expect(getAdminTokens).toHaveBeenCalledTimes(1));
  });

  it('does not fetch for a plain user, whatever a stale is_admin says', async () => {
    viewer.role = 'user';
    viewer.is_admin = true;
    renderList();
    await Promise.resolve();
    expect(getAdminTokens).not.toHaveBeenCalled();
  });
});
