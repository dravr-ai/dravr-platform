// ABOUTME: Tests for ApiKeyDetails — the admin API-token detail screen renders from the token it is given
// ABOUTME: Pins that it asks the server only for rotate/revoke, never for per-token audit, usage or provisioned-key reads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ApiKeyDetails from '../ApiKeyDetails';
import type { AdminToken } from '../../types/api';

const revokeAdminToken = vi.fn();
const rotateAdminToken = vi.fn();

// The server serves GET/revoke/rotate under /admin/tokens/{token_id} and nothing
// else per token, so the screen's whole API surface is these two mutations.
// Any other adminApi method the component reached would be undefined here.
vi.mock('../../services/api', () => ({
  adminApi: {
    revokeAdminToken: (...args: unknown[]) => revokeAdminToken(...args),
    rotateAdminToken: (...args: unknown[]) => rotateAdminToken(...args),
  },
}));

const token: AdminToken = {
  id: 'token-1',
  service_name: 'CI/CD Pipeline',
  service_description: 'Provisions keys from the release job',
  permissions: ['provision_keys', 'list_keys'],
  is_super_admin: false,
  is_active: true,
  created_at: '2026-01-15T10:00:00Z',
  expires_at: '2027-01-15T10:00:00Z',
  last_used_at: '2026-09-20T08:30:00Z',
  usage_count: 1234,
  token_prefix: 'pierre_at_abc',
};

function renderDetails(onBack = vi.fn(), onTokenUpdated = vi.fn()) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <ApiKeyDetails token={token} onBack={onBack} onTokenUpdated={onTokenUpdated} />
    </QueryClientProvider>,
  );
  return { onBack, onTokenUpdated };
}

describe('ApiKeyDetails', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.spyOn(window, 'confirm').mockReturnValue(true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders the token from its props', () => {
    renderDetails();

    expect(screen.getByText('API Token Information')).toBeInTheDocument();
    expect(screen.getByText('Provisions keys from the release job')).toBeInTheDocument();
    expect(screen.getByText('provision keys')).toBeInTheDocument();
    expect(screen.getByText('list keys')).toBeInTheDocument();
    expect(screen.getByText((1234).toLocaleString())).toBeInTheDocument();
    expect(screen.getAllByText('pierre_at_abc...').length).toBeGreaterThan(0);
  });

  it('shows no panel for per-token data the server does not serve', () => {
    renderDetails();

    // /admin/tokens/{id}/audit, /usage-stats and /provisioned-keys are not
    // routes; these panels could only ever render their empty state.
    expect(screen.queryByText('Usage Statistics')).not.toBeInTheDocument();
    expect(screen.queryByText('Recent Activity')).not.toBeInTheDocument();
    expect(screen.queryByText(/Provisioned API Keys/)).not.toBeInTheDocument();
    expect(revokeAdminToken).not.toHaveBeenCalled();
    expect(rotateAdminToken).not.toHaveBeenCalled();
  });

  it('rotates the token and shows the replacement once', async () => {
    rotateAdminToken.mockResolvedValue({ jwt_token: 'pierre_at_rotated_jwt' });
    const { onTokenUpdated } = renderDetails();

    fireEvent.click(screen.getByRole('button', { name: 'Rotate Key' }));

    expect(await screen.findByText('API Token Rotated Successfully')).toBeInTheDocument();
    expect(rotateAdminToken).toHaveBeenCalledWith('token-1');
    expect(screen.getByDisplayValue('pierre_at_rotated_jwt')).toBeInTheDocument();
    expect(onTokenUpdated).toHaveBeenCalledTimes(1);
  });

  it('revokes the token and returns to the list', async () => {
    revokeAdminToken.mockResolvedValue({ success: true });
    const { onBack, onTokenUpdated } = renderDetails();

    fireEvent.click(screen.getByRole('button', { name: 'Revoke Key' }));

    await waitFor(() => expect(onBack).toHaveBeenCalledTimes(1));
    expect(revokeAdminToken).toHaveBeenCalledWith('token-1');
    expect(onTokenUpdated).toHaveBeenCalledTimes(1);
  });
});
