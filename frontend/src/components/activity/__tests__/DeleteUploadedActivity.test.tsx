// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: An uploaded activity's view offers Delete behind a confirmation; a provider's activity offers none
// ABOUTME: Pins the confirmed delete leaving the view, a cancelled one sending nothing, and a failure said in the dialog

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ActivityDetailResponse } from '@pierre/shared-types';
import { ThemeProvider } from '../../../hooks/useTheme';
import ActivityView from '../ActivityView';

const UPLOAD_ID = `${'a'.repeat(64)}-0`;

const api = vi.hoisted(() => ({
  getActivityDetail: vi.fn<(provider: string, id: string) => Promise<ActivityDetailResponse>>(),
  deleteUploadedActivity: vi.fn<(id: string) => Promise<void>>(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: {
    getActivityDetail: api.getActivityDetail,
    deleteUploadedActivity: api.deleteUploadedActivity,
    getActivityRoute: vi.fn(),
    linkActivityConversation: vi.fn(),
  },
  featureFlagsApi: { getMyFeatures: async () => ({ flags: {}, known: [] }) },
}));

vi.mock('../../ChatTab', () => ({
  default: () => <div data-testid="embedded-chat-stub" />,
}));

function detail(provider: string, id: string): ActivityDetailResponse {
  return {
    activity: {
      id,
      provider,
      name: '',
      sport_type: 'ride',
      start_date: '2026-10-07T07:00:00Z',
      duration_seconds: 600,
      distance_meters: 4_792,
      elevation_gain_meters: null,
      has_gps: false,
      summary_polyline: null,
      attribution: null,
    },
    average_heart_rate: 140,
    max_heart_rate: 160,
    average_speed_mps: 8,
    max_speed_mps: null,
    average_power: null,
    calories: null,
    splits: [],
    laps: [],
    conversation_id: null,
  };
}

function renderView(provider: string, id: string) {
  api.getActivityDetail.mockResolvedValue(detail(provider, id));
  const onBack = vi.fn();
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <ActivityView activity={{ provider, id }} onBack={onBack} onNavigate={vi.fn()} />
      </ThemeProvider>
    </QueryClientProvider>,
  );
  return { onBack };
}

describe('Delete on an activity view', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("is not offered on a provider's activity", async () => {
    renderView('strava', '998877');
    await screen.findByTestId('activity-figures');
    expect(screen.queryByTestId('activity-delete')).toBeNull();
  });

  it('deletes an uploaded activity once confirmed, then leaves its view', async () => {
    api.deleteUploadedActivity.mockResolvedValue(undefined);
    const { onBack } = renderView('upload', UPLOAD_ID);
    const user = userEvent.setup();

    await user.click(await screen.findByRole('button', { name: 'Delete this uploaded activity' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('Delete this activity?')).toBeInTheDocument();
    expect(api.deleteUploadedActivity).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole('button', { name: 'Delete' }));

    await waitFor(() => expect(onBack).toHaveBeenCalledTimes(1));
    expect(api.deleteUploadedActivity).toHaveBeenCalledWith(UPLOAD_ID);
  });

  it('sends nothing when the confirmation is cancelled', async () => {
    const { onBack } = renderView('upload', UPLOAD_ID);
    const user = userEvent.setup();

    await user.click(await screen.findByTestId('activity-delete'));
    await user.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Cancel' }));

    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(api.deleteUploadedActivity).not.toHaveBeenCalled();
    expect(onBack).not.toHaveBeenCalled();
  });

  it('says a failed delete in the dialog and stays on the view', async () => {
    api.deleteUploadedActivity.mockRejectedValue(new Error('network down'));
    const { onBack } = renderView('upload', UPLOAD_ID);
    const user = userEvent.setup();

    await user.click(await screen.findByTestId('activity-delete'));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByRole('button', { name: 'Delete' }));

    expect(await within(dialog).findByText(/This activity couldn't be deleted\. Try again\./)).toBeInTheDocument();
    expect(onBack).not.toHaveBeenCalled();
  });
});
