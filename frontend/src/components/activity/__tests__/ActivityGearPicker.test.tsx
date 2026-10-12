// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for an activity's shoe picker — the pair given by default, the athlete's choice of another or of none, and a pair past its warning
// ABOUTME: The gear API is mocked at the services barrel; assertions read the select, the requests sent and the way to Settings

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ActivityGearResponse, GearItem } from '@pierre/shared-types';
import { ActivityGearPicker } from '../ActivityGearPicker';

const api = vi.hoisted(() => ({
  listGear: vi.fn(),
  getActivityGear: vi.fn(),
  setActivityGear: vi.fn(),
}));

vi.mock('../../../services/api', () => ({ gearApi: api }));

const ROAD = '11111111-1111-4111-8111-111111111111';
const TRAIL = '22222222-2222-4222-8222-222222222222';
const OLD = '33333333-3333-4333-8333-333333333333';

function pair(id: string, overrides: Partial<GearItem> = {}): GearItem {
  return {
    id,
    kind: 'shoe',
    name: `Pair ${id.slice(0, 4)}`,
    sport: 'run',
    initial_distance_meters: 0,
    warn_distance_meters: 500_000,
    distance_meters: 100_000,
    activity_count: 3,
    warn_reached: false,
    pinned: false,
    is_default: false,
    added_at: '2026-09-01T07:00:00Z',
    retired_at: null,
    last_used_at: null,
    ...overrides,
  };
}

function renderPicker(onNavigate = vi.fn()) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <ActivityGearPicker activity={{ provider: 'strava', id: 'morning-run' }} onNavigate={onNavigate} />
    </QueryClientProvider>,
  );
  return { onNavigate };
}

describe('ActivityGearPicker', () => {
  let attribution: ActivityGearResponse;

  beforeEach(() => {
    vi.clearAllMocks();
    attribution = { gear_id: ROAD, source: 'default' };
    api.listGear.mockResolvedValue({
      gear: [
        pair(ROAD, { name: 'Pegasus 41', is_default: true }),
        pair(TRAIL, { name: 'Speedgoat 6', sport: 'trail_running' }),
        pair(OLD, { name: 'Old Clifton', retired_at: '2026-09-20T10:00:00Z' }),
      ],
    });
    api.getActivityGear.mockImplementation(async () => attribution);
    api.setActivityGear.mockImplementation(async (...args: [string, string, string | null]) => {
      attribution = { gear_id: args[2], source: 'declared' };
      return attribution;
    });
  });

  it('shows the pair given by default and says it was given by default', async () => {
    renderPicker();

    const select = await screen.findByTestId('activity-gear-select');
    expect(select).toHaveValue(ROAD);
    expect(screen.getByLabelText('Shoes')).toBe(select);
    expect(screen.getByTestId('activity-gear')).toHaveTextContent(
      'Counted on the pair you use for this sport. Choose another if you wore a different one.',
    );
    // A retired pair the activity is not attributed to is not offered.
    expect(screen.queryByRole('option', { name: 'Old Clifton' })).not.toBeInTheDocument();
    expect(screen.queryByRole('option', { name: 'Old Clifton (retired)' })).not.toBeInTheDocument();
  });

  it('records the pair the athlete chooses, and none of theirs as null', async () => {
    const user = userEvent.setup();
    renderPicker();
    const select = await screen.findByTestId('activity-gear-select');

    await user.selectOptions(select, TRAIL);
    expect(api.setActivityGear).toHaveBeenCalledWith('strava', 'morning-run', TRAIL);
    await waitFor(() => expect(select).toHaveValue(TRAIL));
    expect(screen.getByTestId('activity-gear')).not.toHaveTextContent('Counted on the pair you use');

    await user.selectOptions(select, 'none');
    expect(api.setActivityGear).toHaveBeenLastCalledWith('strava', 'morning-run', null);
    await waitFor(() => expect(select).toHaveValue('none'));
  });

  it('shows the warning of a chosen pair past its threshold', async () => {
    api.listGear.mockResolvedValue({
      gear: [pair(ROAD, { name: 'Pegasus 41', distance_meters: 512_300, warn_reached: true })],
    });
    renderPicker();

    expect(await screen.findByTestId(`gear-warning-${ROAD}`)).toHaveTextContent(
      'This pair has 512 km, past the 500 km you set.',
    );
  });

  it('shows a retired pair the activity is counted on, without offering it again', async () => {
    attribution = { gear_id: OLD, source: 'declared' };
    renderPicker();

    const select = await screen.findByTestId('activity-gear-select');
    expect(select).toHaveValue(OLD);
    expect(screen.getByRole('option', { name: 'Old Clifton (retired)' })).toBeDisabled();
  });

  it('asks for a choice when nothing is counted on a pair', async () => {
    attribution = { gear_id: null, source: null };
    renderPicker();

    const select = await screen.findByTestId('activity-gear-select');
    expect(select).toHaveValue('');
    expect(screen.getByRole('option', { name: 'Choose a pair' })).toBeDisabled();
  });

  it('leads to Settings when the athlete has declared no pair', async () => {
    api.listGear.mockResolvedValue({ gear: [] });
    attribution = { gear_id: null, source: null };
    const user = userEvent.setup();
    const { onNavigate } = renderPicker();

    await user.click(await screen.findByTestId('activity-gear-add'));

    expect(onNavigate).toHaveBeenCalledWith('settings/gear');
    expect(screen.queryByTestId('activity-gear-select')).not.toBeInTheDocument();
  });

  it("says why the activity's read failed when the ledger read did not", async () => {
    api.getActivityGear.mockRejectedValue({ response: { status: 404, data: { message: 'activity morning-run' } } });
    renderPicker();

    await waitFor(() => expect(screen.getByTestId('activity-gear')).toHaveTextContent('activity morning-run'));
    expect(screen.queryByTestId('activity-gear-select')).not.toBeInTheDocument();
  });

  it('says a refused choice under the picker', async () => {
    api.setActivityGear.mockRejectedValueOnce({ response: { status: 400, data: {} } });
    const user = userEvent.setup();
    renderPicker();

    await user.selectOptions(await screen.findByTestId('activity-gear-select'), TRAIL);

    expect(await screen.findByText("Your choice couldn't be saved. Try again.")).toBeInTheDocument();
  });
});
