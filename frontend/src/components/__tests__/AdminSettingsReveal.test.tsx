// ABOUTME: Platform Settings draws once its flags, auto-approval and group policy are all in
// ABOUTME: Drawn as each lands, a flags panel at the top arriving last pushes both cards below off screen
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import AdminSettings from '../AdminSettings';

const flags = vi.hoisted(() => {
  let resolve: (value: unknown) => void = () => undefined;
  const promise = new Promise((res) => {
    resolve = res;
  });
  return { promise, resolve: (value: unknown) => resolve(value) };
});

vi.mock('../../services/api', () => ({
  adminApi: {
    getAutoApprovalSetting: vi.fn(() =>
      Promise.resolve({ enabled: false, description: 'Auto-approval', overridden_by_env: false }),
    ),
    getTenantFeatureFlags: vi.fn(() => flags.promise),
    updateAutoApprovalSetting: vi.fn(),
    updateConfig: vi.fn(),
  },
}));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { tenant_id: 'tenant-a' } }),
}));

vi.mock('../../hooks/useGroups', () => ({
  useGroupPermissions: () => ({ policy: 'admins_only', isLoading: false }),
}));

vi.mock('../FeatureFlagsPanel', () => ({
  default: () => <div data-testid="feature-flags-panel" />,
}));

const { adminApi } = await import('../../services/api');

describe('AdminSettings first load', () => {
  it('holds the cards until the feature flags are in too', async () => {
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={queryClient}>
        <AdminSettings />
      </QueryClientProvider>,
    );

    // Auto-approval and the group policy are in; the flags are not.
    await waitFor(() => expect(adminApi.getAutoApprovalSetting).toHaveBeenCalled());
    await act(async () => {});
    expect(screen.getByRole('status', { name: 'Loading platform settings' })).toBeInTheDocument();
    expect(screen.queryByRole('switch')).not.toBeInTheDocument();

    await act(async () => {
      flags.resolve({ rows: [], known: [] });
    });
    expect(await screen.findByTestId('feature-flags-panel')).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: 'Auto-Approve Registrations' })).toBeInTheDocument();
  });
});
