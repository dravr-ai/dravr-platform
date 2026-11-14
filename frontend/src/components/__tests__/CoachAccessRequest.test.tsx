// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one coach-access request across its variants — the onboarding inline button, and the Group info block's re-read
// ABOUTME: Linking a coaching platform shows what the server filed, read back from the request, never assumed from the tap

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { CoachAccessRequest as Request } from '@pierre/shared-types';
import CoachAccessRequest from '../CoachAccessRequest';
import { ThemeProvider } from '../../hooks/useTheme';

const api = vi.hoisted(() => ({
  getCoachAccessRequest: vi.fn(),
  requestCoachAccess: vi.fn(),
  cancelCoachAccessRequest: vi.fn(),
  getProvidersStatus: vi.fn(),
}));

vi.mock('../../services/api', () => ({
  userApi: {
    getCoachAccessRequest: () => api.getCoachAccessRequest(),
    requestCoachAccess: (groupId?: string) => api.requestCoachAccess(groupId),
    cancelCoachAccessRequest: () => api.cancelCoachAccessRequest(),
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

function renderRequest(
  variant: 'inline' | 'block' | 'panel',
  extra: { groupId?: string; managesRoster?: boolean; onOpenHome?: () => void } = { groupId: 'group-1' },
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <CoachAccessRequest variant={variant} {...extra} />
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

  describe('the group-less panel (Settings and Home)', () => {
    it('asks with no group, then shows the dated sentence and the way to withdraw it', async () => {
      api.requestCoachAccess.mockResolvedValue({ request: pending });
      renderRequest('panel', {});

      const panel = await screen.findByTestId('coach-seat-panel');
      expect(panel).toHaveAttribute('data-state', 'to_request');
      expect(within(panel).getByRole('heading', { name: 'Get your coach access' })).toBeInTheDocument(); // humanCoach.seat.panelTitle
      await userEvent.click(within(panel).getByTestId('coach-access-request'));

      await waitFor(() => expect(api.requestCoachAccess).toHaveBeenCalledWith(undefined));
      await waitFor(() => expect(panel).toHaveAttribute('data-state', 'pending'));
      expect(within(panel).getByTestId('coach-access-pending')).toHaveTextContent(
        /^Request sent .*2026\. We'll let you know here and by email\.$/,
      );
      expect(within(panel).getByTestId('coach-access-cancel')).toBeEnabled();
    });

    it('withdraws the waiting request and offers the request again', async () => {
      api.getCoachAccessRequest.mockResolvedValue({ request: pending });
      api.cancelCoachAccessRequest.mockResolvedValue({ request: null });
      renderRequest('panel', {});

      const panel = await screen.findByTestId('coach-seat-panel');
      await userEvent.click(within(panel).getByTestId('coach-access-cancel'));

      await waitFor(() => expect(panel).toHaveAttribute('data-state', 'to_request'));
      expect(within(panel).getByTestId('coach-access-cancelled')).toHaveTextContent('Request cancelled');
    });

    it('says a failed withdrawal and reads the request again', async () => {
      api.getCoachAccessRequest.mockResolvedValue({ request: pending });
      api.cancelCoachAccessRequest.mockRejectedValue(new Error('409'));
      renderRequest('panel', {});

      const panel = await screen.findByTestId('coach-seat-panel');
      await userEvent.click(within(panel).getByTestId('coach-access-cancel'));

      expect(await within(panel).findByTestId('coach-access-cancel-failed')).toHaveTextContent(
        "We couldn't cancel your request. Please try again.",
      );
      await waitFor(() => expect(api.getCoachAccessRequest).toHaveBeenCalledTimes(2));
    });

    it('shows the declined reason beside Ask again', async () => {
      api.getCoachAccessRequest.mockResolvedValue({
        request: { ...pending, status: 'declined', decided_at: '2026-11-13T09:00:00Z', decline_reason: 'Not verified.' },
      });
      renderRequest('panel', {});

      const panel = await screen.findByTestId('coach-seat-panel');
      expect(await within(panel).findByTestId('coach-access-decline-reason')).toHaveTextContent('Reason: Not verified.');
      expect(within(panel).getByTestId('coach-access-request')).toHaveTextContent('Ask again');
    });

    it('shows the plain ask, not the old decline, after asking again and withdrawing', async () => {
      // The server's own read skips a decline a newer ask replaced, so the
      // withdrawal answers with no request at all (carnet#1522).
      api.getCoachAccessRequest.mockResolvedValue({
        request: { ...pending, status: 'declined', decided_at: '2026-11-13T09:00:00Z', decline_reason: 'Not verified.' },
      });
      api.requestCoachAccess.mockResolvedValue({ request: pending });
      api.cancelCoachAccessRequest.mockResolvedValue({ request: null });
      renderRequest('panel', {});

      const panel = await screen.findByTestId('coach-seat-panel');
      await userEvent.click(await within(panel).findByRole('button', { name: 'Ask again' }));
      await waitFor(() => expect(panel).toHaveAttribute('data-state', 'pending'));
      await userEvent.click(within(panel).getByTestId('coach-access-cancel'));

      await waitFor(() => expect(panel).toHaveAttribute('data-state', 'to_request'));
      expect(within(panel).getByTestId('coach-access-request')).toHaveTextContent('Request coach access');
      expect(within(panel).queryByTestId('coach-access-decline-reason')).toBeNull();
    });

    it('says the access is on and opens the coach Home once granted', async () => {
      const onOpenHome = vi.fn();
      renderRequest('panel', { managesRoster: true, onOpenHome });

      const panel = await screen.findByTestId('coach-seat-panel');
      expect(panel).toHaveAttribute('data-state', 'granted');
      expect(within(panel).getByRole('heading', { name: 'Your coach access is on' })).toBeInTheDocument();
      await userEvent.click(within(panel).getByTestId('coach-seat-open-home'));
      expect(onOpenHome).toHaveBeenCalled();
    });
  });
});
