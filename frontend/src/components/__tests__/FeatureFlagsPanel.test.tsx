// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the feature-flag panel's effective-state pill to the badge tone table
// ABOUTME: An enabled flag draws as a success, a disabled one as an error, so the ink clears AA

import React from 'react';
import { render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { describe, expect, it, vi } from 'vitest';

const getTenantFeatureFlags = vi.fn();

// The network seam: the panel reads a tenant's stored rows and the known flags
// from the admin API, so the fixture stands in for the server.
vi.mock('../../services/api', () => ({
  adminApi: {
    getTenantFeatureFlags: (...args: unknown[]) => getTenantFeatureFlags(...args),
  },
}));

import FeatureFlagsPanel from '../FeatureFlagsPanel';

function renderPanel() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return render(
    <QueryClientProvider client={client}>
      <FeatureFlagsPanel scope={{ kind: 'tenant', id: 'tenant-a' }} />
    </QueryClientProvider>,
  );
}

describe('FeatureFlagsPanel', () => {
  it('draws the effective state as a success or an error badge', async () => {
    getTenantFeatureFlags.mockResolvedValue({
      rows: [{ feature_key: 'off_flag', enabled: false, updated_at: '2026-11-01T00:00:00Z', updated_by: null }],
      known: [
        { key: 'on_flag', description: 'On by default', default_enabled: true },
        { key: 'off_flag', description: 'Turned off for the tenant', default_enabled: true },
      ],
    });
    renderPanel();

    expect(await screen.findByText('enabled')).toHaveClass('badge-success');
    expect(screen.getByText('disabled')).toHaveClass('badge-error');
  });
});
