// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests an activity's shoe picker on the phone — the pair given by default, the athlete's choice of another or of none, a pair past its warning
// ABOUTME: The gear API is mocked at the services barrel; assertions read the row's hint, the sheet's radio rows, the requests sent and the way to Settings

import React from 'react';
import { fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ActivityGearResponse, GearItem } from '@pierre/shared-types';
import { settingsPane } from '@pierre/shared-constants';

const mockPush = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: mockPush, back: jest.fn() }),
}));
// An icon is a view naming its glyph, so a row's text is only its words.
jest.mock('@expo/vector-icons', () => {
  const View = require('react-native').View;
  const glyph = (props: Record<string, unknown>) =>
    require('react').createElement(View, { testID: `icon-${props.name}` });
  return { Ionicons: glyph, Feather: glyph, MaterialCommunityIcons: glyph };
});

const mockListGear = jest.fn();
const mockGetActivityGear = jest.fn();
const mockSetActivityGear = jest.fn();
jest.mock('../src/services/api', () => ({
  gearApi: {
    listGear: () => mockListGear(),
    getActivityGear: (provider: string, id: string) => mockGetActivityGear(provider, id),
    setActivityGear: (provider: string, id: string, gearId: string | null) => mockSetActivityGear(provider, id, gearId),
  },
}));

import { ActivityGearPicker } from '../src/screens/activity/ActivityGearPicker';

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

function renderPicker() {
  // A finished mutation is kept, not timed out: the default five-minute
  // collection timer would hold jest open after the run.
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false, gcTime: Infinity } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ActivityGearPicker provider="strava" activityId="morning-run" />
    </QueryClientProvider>,
  );
}

/** The value the shoes row shows on its right. */
async function shownPair(screen: ReturnType<typeof render>) {
  return (await screen.findByTestId('activity-gear-value')).props.children as string;
}

describe('ActivityGearPicker', () => {
  let attribution: ActivityGearResponse;

  beforeEach(() => {
    jest.clearAllMocks();
    attribution = { gear_id: ROAD, source: 'default' };
    mockListGear.mockResolvedValue({
      gear: [
        pair(ROAD, { name: 'Pegasus 41', is_default: true }),
        pair(TRAIL, { name: 'Speedgoat 6', sport: 'trail_running', distance_meters: 212_400 }),
        pair(OLD, { name: 'Old Clifton', retired_at: '2026-09-20T10:00:00Z' }),
      ],
    });
    mockGetActivityGear.mockImplementation(async () => attribution);
    mockSetActivityGear.mockImplementation(async (_provider: string, _id: string, gearId: string | null) => {
      attribution = { gear_id: gearId, source: 'declared' };
      return attribution;
    });
  });

  it('shows the pair given by default and says it was given by default', async () => {
    const screen = renderPicker();

    expect(await shownPair(screen)).toBe('Pegasus 41');
    expect(mockGetActivityGear).toHaveBeenCalledWith('strava', 'morning-run');
    expect(screen.getByTestId('activity-gear-select')).toHaveTextContent('Shoes', { exact: false });
    expect(screen.getByTestId('activity-gear-default')).toHaveTextContent(
      'Counted on the pair you use for this sport. Choose another if you wore a different one.',
    );

    fireEvent.press(screen.getByTestId('activity-gear-select'));
    const sheet = screen.getByTestId('activity-gear-sheet');
    const road = within(sheet).getByTestId(`activity-gear-option-${ROAD}`);
    expect(road.props.accessibilityState).toEqual({ selected: true, disabled: false });
    expect(within(road).getByTestId('icon-check')).toBeTruthy();
    expect(within(sheet).getByTestId(`activity-gear-option-${TRAIL}`)).toHaveTextContent(
      'Speedgoat 6Trail running212 km',
    );
    // The default is not the athlete's "none": that row is not selected.
    expect(within(sheet).getByTestId('activity-gear-option-none').props.accessibilityState).toEqual({
      selected: false,
      disabled: false,
    });
    // A retired pair the activity is not attributed to is not offered.
    expect(within(sheet).queryByTestId(`activity-gear-option-${OLD}`)).toBeNull();
  });

  it('records the pair the athlete chooses, and none of theirs as null', async () => {
    const screen = renderPicker();
    await shownPair(screen);

    fireEvent.press(screen.getByTestId('activity-gear-select'));
    fireEvent.press(screen.getByTestId(`activity-gear-option-${TRAIL}`));

    await waitFor(() => expect(mockSetActivityGear).toHaveBeenCalledWith('strava', 'morning-run', TRAIL));
    await waitFor(async () => expect(await shownPair(screen)).toBe('Speedgoat 6'));
    expect(screen.queryByTestId('activity-gear-default')).toBeNull();
    expect(screen.queryByTestId('activity-gear-sheet')).toBeNull();

    fireEvent.press(screen.getByTestId('activity-gear-select'));
    fireEvent.press(screen.getByTestId('activity-gear-option-none'));

    await waitFor(() => expect(mockSetActivityGear).toHaveBeenLastCalledWith('strava', 'morning-run', null));
    await waitFor(async () => expect(await shownPair(screen)).toBe('None of my pairs'));
    fireEvent.press(screen.getByTestId('activity-gear-select'));
    expect(screen.getByTestId('activity-gear-option-none').props.accessibilityState).toEqual({
      selected: true,
      disabled: false,
    });
  });

  it('sends nothing when the athlete picks the pair already shown', async () => {
    attribution = { gear_id: ROAD, source: 'declared' };
    const screen = renderPicker();
    await shownPair(screen);

    fireEvent.press(screen.getByTestId('activity-gear-select'));
    fireEvent.press(screen.getByTestId(`activity-gear-option-${ROAD}`));

    expect(screen.queryByTestId('activity-gear-sheet')).toBeNull();
    expect(mockSetActivityGear).not.toHaveBeenCalled();
  });

  it('shows the warning of a chosen pair past its threshold', async () => {
    mockListGear.mockResolvedValue({
      gear: [pair(ROAD, { name: 'Pegasus 41', distance_meters: 512_300, warn_reached: true })],
    });
    const screen = renderPicker();

    expect(await screen.findByTestId(`gear-warning-${ROAD}`)).toHaveTextContent(
      'This pair has 512 km, past the 500 km you set.',
    );
  });

  it('shows a retired pair the activity is counted on, without offering it again', async () => {
    attribution = { gear_id: OLD, source: 'declared' };
    const screen = renderPicker();

    expect(await shownPair(screen)).toBe('Old Clifton (retired)');
    fireEvent.press(screen.getByTestId('activity-gear-select'));
    const old = screen.getByTestId(`activity-gear-option-${OLD}`);
    expect(old.props.accessibilityState).toEqual({ selected: true, disabled: true });
    // The row has no press of its own; nothing is recorded for it.
    expect(old.props.onPress).toBeUndefined();
    fireEvent.press(old);
    expect(mockSetActivityGear).not.toHaveBeenCalled();
  });

  it('asks for a choice when nothing is counted on a pair', async () => {
    attribution = { gear_id: null, source: null };
    const screen = renderPicker();

    expect(await shownPair(screen)).toBe('Choose a pair');
    expect(screen.queryByTestId('activity-gear-default')).toBeNull();
    fireEvent.press(screen.getByTestId('activity-gear-select'));
    expect(screen.getByTestId('activity-gear-option-none').props.accessibilityState).toEqual({
      selected: false,
      disabled: false,
    });
  });

  it('leads to Settings when the athlete has declared no pair', async () => {
    mockListGear.mockResolvedValue({ gear: [] });
    attribution = { gear_id: null, source: null };
    const screen = renderPicker();

    const add = await screen.findByTestId('activity-gear-add');
    expect(add).toHaveTextContent('Add shoes');
    expect(screen.getByTestId('activity-gear')).toHaveTextContent(
      "Add your shoes to count this activity's distance on a pair.",
      { exact: false },
    );
    fireEvent.press(add);

    expect(mockPush).toHaveBeenCalledWith(settingsPane('gear').mobile);
    expect(screen.queryByTestId('activity-gear-select')).toBeNull();
  });

  it("says why the activity's read failed when the ledger read did not", async () => {
    mockGetActivityGear.mockRejectedValue({ response: { status: 404, data: { message: 'activity morning-run' } } });
    const screen = renderPicker();

    await waitFor(() => expect(screen.getByTestId('activity-gear')).toHaveTextContent('activity morning-run'));
    expect(screen.queryByTestId('activity-gear-select')).toBeNull();
  });

  it('says a refused choice under the picker', async () => {
    mockSetActivityGear.mockRejectedValueOnce({ response: { status: 400, data: {} } });
    const screen = renderPicker();
    await shownPair(screen);

    fireEvent.press(screen.getByTestId('activity-gear-select'));
    fireEvent.press(screen.getByTestId(`activity-gear-option-${TRAIL}`));

    expect(await screen.findByTestId('activity-gear-failed')).toHaveTextContent(
      "Your choice couldn't be saved. Try again.",
    );
    expect(await shownPair(screen)).toBe('Pegasus 41');
  });
});
