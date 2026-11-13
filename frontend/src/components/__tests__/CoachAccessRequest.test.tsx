// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one coach-access request across its variants — the onboarding inline button, and the Group info block's re-read
// ABOUTME: Linking a coaching platform shows what the server filed, read back from the request, never assumed from the tap

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within, act } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { CoachAccessRequest as Request } from '@pierre/shared-types';
import CoachAccessRequest from '../CoachAccessRequest';
import { ThemeProvider } from '../../hooks/useTheme';

const api = vi.hoisted(() => ({
  getCoachAccessRequest: vi.fn(),
  requestCoachAccess: vi.fn(),
  getProvidersStatus: vi.fn(),
}));

vi.mock('../../services/api', () => ({
  userApi: {
    getCoachAccessRequest: () => api.getCoachAccessRequest(),
    requestCoachAccess: (groupId: string) => api.requestCoachAccess(groupId),
  },
  providersApi: { getProvidersStatus: () => api.getProvidersStatus() },
  oauthApi: { sciotteCancel: vi.fn(), authorizeUrl: vi.fn() },
}));
vi.mock('../../services/analytics', () => ({ track: vi.fn() }));

const pending: Request = {
  id: 'req-1',
  user_id: 'user-1',
  group_id: null,
  group_tenant_id: null,
  status: 'pending',
  created_at: '2026-11-12T19:32:00Z',
  decided_at: null,
  decided_by: null,
};

function platforms(intervalsConnected: boolean) {
  return {
    providers: [
      { provider: 'sciotte_trainingpeaks', display_name: 'TrainingPeaks', description: 'x', connected: false, requires_oauth: false },
      { provider: 'intervals_icu', display_name: 'Intervals.icu', description: 'y', connected: intervalsConnected, requires_oauth: false },
    ],
  };
}

function renderRequest(variant: 'inline' | 'block') {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <CoachAccessRequest groupId="group-1" variant={variant} />
      </ThemeProvider>
    </QueryClientProvider>,
  );
  return { ...view, queryClient };
}

describe('CoachAccessRequest', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.getCoachAccessRequest.mockResolvedValue({ request: null });
    api.getProvidersStatus.mockResolvedValue(platforms(false));
  });

  it('keeps the onboarding step its quiet inline button', async () => {
    renderRequest('inline');

    const button = await screen.findByTestId('coach-access-request');
    await waitFor(() => expect(button).toBeEnabled());
    expect(button).toHaveClass('btn-secondary');
    expect(screen.queryByTestId('coach-seat-block')).toBeNull();
  });

  it('reads the request back after a coaching platform is linked, and shows the request the server filed', async () => {
    const { queryClient } = renderRequest('block');
    const block = await screen.findByTestId('coach-seat-block');
    expect(block).toHaveAttribute('data-state', 'to_request');
    await within(block).findByTestId('provider-card-intervals_icu');

    // Intervals.icu links; its roster read files the request in the background.
    api.getCoachAccessRequest.mockResolvedValue({ request: pending });
    api.getProvidersStatus.mockResolvedValue(platforms(true));
    await act(async () => {
      await queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.status() });
    });

    await waitFor(() => expect(block).toHaveAttribute('data-state', 'pending'), { timeout: 5000 });
    expect(within(block).getByTestId('coach-seat-step-sent')).toHaveTextContent('Request sent');
  });

  it('reads nothing again for a platform linked before the block opened', async () => {
    api.getProvidersStatus.mockResolvedValue(platforms(true));
    renderRequest('block');

    await screen.findByTestId('coach-seat-block');
    await waitFor(() => expect(api.getProvidersStatus).toHaveBeenCalled());
    await new Promise((resolve) => setTimeout(resolve, 3500));
    expect(api.getCoachAccessRequest).toHaveBeenCalledTimes(1);
  });
});
