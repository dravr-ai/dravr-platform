// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Settings › Shoes on the phone — each pair's distance and warning, adding and editing one in the sheet, retiring and deleting
// ABOUTME: The gear API is mocked at the services barrel; deletion is confirmed through the mocked Alert's destructive button

import React from 'react';
import { Alert, type AlertButton } from 'react-native';
import { act, fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { GearItem } from '@pierre/shared-types';
import type { DistanceUnit } from '@pierre/chat-utils';
import { UnitsContext } from '@pierre/ui-logic';

const mockListGear = jest.fn();
const mockCreateGear = jest.fn();
const mockUpdateGear = jest.fn();
const mockRetireGear = jest.fn();
const mockRestoreGear = jest.fn();
const mockDeleteGear = jest.fn();
jest.mock('../src/services/api', () => ({
  gearApi: {
    listGear: () => mockListGear(),
    createGear: (...args: unknown[]) => mockCreateGear(...args),
    updateGear: (...args: unknown[]) => mockUpdateGear(...args),
    retireGear: (id: string) => mockRetireGear(id),
    restoreGear: (id: string) => mockRestoreGear(id),
    deleteGear: (id: string) => mockDeleteGear(id),
  },
}));

// An icon is a view naming its glyph, so a row's text is only its words.
jest.mock('@expo/vector-icons', () => {
  const View = require('react-native').View;
  const glyph = (props: Record<string, unknown>) =>
    require('react').createElement(View, { testID: `icon-${props.name}` });
  return { Ionicons: glyph, Feather: glyph, MaterialCommunityIcons: glyph };
});

import { GearScreen } from '../src/screens/settings/GearScreen';

function pair(id: string, overrides: Partial<GearItem> = {}): GearItem {
  return {
    id,
    kind: 'shoe',
    name: `Pair ${id}`,
    sport: 'run',
    initial_distance_meters: 0,
    warn_distance_meters: 500_000,
    distance_meters: 0,
    activity_count: 0,
    warn_reached: false,
    pinned: false,
    is_default: false,
    added_at: '2026-09-01T07:00:00Z',
    retired_at: null,
    last_used_at: null,
    ...overrides,
  };
}

function renderScreen(units: DistanceUnit = 'metric') {
  // A finished mutation is kept, not timed out: the default five-minute
  // collection timer would hold jest open after the run.
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false, gcTime: Infinity } },
  });
  return render(
    <QueryClientProvider client={client}>
      <UnitsContext.Provider value={units}>
        <GearScreen />
      </UnitsContext.Provider>
    </QueryClientProvider>,
  );
}

/**
 * Wait until nothing carries `testID`. The check throws a one-line error
 * rather than `expect(…).toBeNull()`, whose failure message pretty-prints the
 * whole element — seconds per poll on this tree, longer than `waitFor` waits.
 */
async function waitForGone(screen: ReturnType<typeof render>, testID: string) {
  await waitFor(() => {
    if (screen.queryByTestId(testID) !== null) throw new Error(`${testID} is still shown`);
  });
}

/** Answer every Alert with the button whose style is `style`, and record what it said. */
function answerAlerts(style: AlertButton['style']) {
  return jest.spyOn(Alert, 'alert').mockImplementation((_title, _message, buttons) => {
    buttons?.find((button) => button.style === style)?.onPress?.();
  });
}

describe('GearScreen', () => {
  let ledger: GearItem[];

  beforeEach(() => {
    jest.clearAllMocks();
    ledger = [
      pair('road', {
        name: 'Pegasus 41',
        distance_meters: 512_300,
        activity_count: 41,
        warn_reached: true,
        is_default: true,
        last_used_at: '2026-10-08T06:30:00Z',
      }),
      pair('trail', { name: 'Speedgoat 6', sport: 'trail_running', pinned: true }),
      pair('old', { name: 'Old Clifton', distance_meters: 820_000, activity_count: 90, retired_at: '2026-09-20T10:00:00Z' }),
    ];
    mockListGear.mockImplementation(async () => ({ gear: ledger }));
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('shows each pair in use with its distance, its badges, its facts and, past its threshold, the warning', async () => {
    const screen = renderScreen();

    const road = await screen.findByTestId('gear-road');
    expect(within(screen.getByTestId('gear-in-use')).getByTestId('gear-road')).toBe(road);
    expect(within(road).getByTestId('gear-distance-road')).toHaveTextContent('512 km');
    expect(road).toHaveTextContent('Pegasus 41', { exact: false });
    expect(road).toHaveTextContent('Run', { exact: false });
    expect(road).toHaveTextContent('41 activities · Last used Oct 8, 2026 · Warning at 500 km', { exact: false });
    expect(within(road).getByTestId('gear-default-road')).toHaveTextContent('· Default');
    const warning = within(road).getByTestId('gear-warning-road');
    expect(warning).toHaveTextContent('This pair has 512 km, past the 500 km you set.');
    expect(within(warning).getByTestId('icon-alert-triangle')).toBeTruthy();
    // A pair with activities is retired, never deleted.
    expect(within(road).queryByTestId('gear-delete-road')).toBeNull();

    const trail = screen.getByTestId('gear-trail');
    expect(trail).toHaveTextContent('Trail running', { exact: false });
    expect(within(trail).getByTestId('gear-pinned-trail')).toHaveTextContent('· Pinned');
    expect(within(trail).queryByTestId('gear-default-trail')).toBeNull();
    expect(within(trail).queryByTestId('gear-warning-trail')).toBeNull();
    expect(within(trail).getByTestId('gear-delete-trail')).toBeTruthy();
  });

  // carnet#1212: the count chose its form with an inline `=== 1`, so a new
  // pair read « 0 activités » to a French athlete.
  it('counts a new pair\'s activities in the French singular', async () => {
    await act(async () => {
      await i18n.changeLanguage('fr');
    });
    try {
      const screen = renderScreen();
      const trail = await screen.findByTestId('gear-trail');
      expect(trail).toHaveTextContent('0 activité', { exact: false });
      expect(trail).not.toHaveTextContent('0 activités', { exact: false });
      expect(screen.getByTestId('gear-road')).toHaveTextContent('41 activités', { exact: false });
    } finally {
      await act(async () => {
        await i18n.changeLanguage('en');
      });
    }
  });

  it('lists a retired pair apart, with its distance kept and a way back into use', async () => {
    mockRestoreGear.mockImplementation(async (id: string) => {
      ledger = ledger.map((p) => (p.id === id ? { ...p, retired_at: null } : p));
      return ledger.find((p) => p.id === id);
    });
    const screen = renderScreen();

    const retired = await screen.findByTestId('gear-retired');
    expect(within(retired).getByTestId('gear-distance-old')).toHaveTextContent('820 km');
    expect(within(retired).getByTestId('gear-old')).toHaveTextContent('Retired Sep 20, 2026', { exact: false });
    expect(within(screen.getByTestId('gear-in-use')).queryByTestId('gear-old')).toBeNull();
    expect(within(retired).queryByTestId('gear-edit-old')).toBeNull();

    fireEvent.press(within(retired).getByTestId('gear-restore-old'));

    await waitFor(() => expect(mockRestoreGear).toHaveBeenCalledWith('old'));
    await waitFor(() => expect(within(screen.getByTestId('gear-in-use')).getByTestId('gear-old')).toBeTruthy());
    expect(screen.queryByTestId('gear-retired')).toBeNull();
  });

  it('says so when the athlete has no pair, and opens the sheet from there', async () => {
    ledger = [];
    const screen = renderScreen();

    expect(await screen.findByTestId('gear-empty')).toHaveTextContent(
      'No pairs yet. Add the shoes you train in to follow their distance.',
      { exact: false },
    );
    expect(screen.queryByTestId('gear-add')).toBeNull();
    fireEvent.press(screen.getByTestId('gear-add-open'));
    expect(screen.getByTestId('gear-sheet')).toHaveTextContent('Add a pair', { exact: false });
    expect(screen.getByTestId('gear-add')).toBeTruthy();
  });

  it('adds a pair from the sheet, its warning at 500 km unless changed, and closes it', async () => {
    mockCreateGear.mockImplementation(async (request: { name: string }) => {
      const created = pair('new', { name: request.name, initial_distance_meters: 120_000, distance_meters: 120_000 });
      ledger = [created, ...ledger];
      return created;
    });
    const screen = renderScreen();
    await screen.findByTestId('gear-road');

    fireEvent.press(screen.getByTestId('gear-add-open'));
    expect(screen.getByTestId('gear-add-warn').props.value).toBe('500');
    fireEvent.changeText(screen.getByTestId('gear-add-name'), '  Novablast 5 ');
    fireEvent.press(screen.getByTestId('gear-add-sport-trail_running'));
    fireEvent.press(screen.getByTestId('gear-add-sport-run'));
    fireEvent.changeText(screen.getByTestId('gear-add-initial'), '120');
    fireEvent.press(screen.getByTestId('gear-add-submit'));

    await waitFor(() =>
      expect(mockCreateGear).toHaveBeenCalledWith({
        name: 'Novablast 5',
        sport: 'run',
        initial_distance_meters: 120_000,
        warn_distance_meters: 500_000,
        pinned: false,
      }),
    );
    expect(await screen.findByTestId('gear-new')).toHaveTextContent('Novablast 5', { exact: false });
    await waitForGone(screen, 'gear-add');
  });

  it('pins a new trail pair and takes a comma as the decimal mark', async () => {
    mockCreateGear.mockImplementation(async (request: { name: string }) => pair('new', { name: request.name }));
    const screen = renderScreen();
    await screen.findByTestId('gear-road');

    fireEvent.press(screen.getByTestId('gear-add-open'));
    fireEvent.changeText(screen.getByTestId('gear-add-name'), 'Speedgoat 7');
    fireEvent.press(screen.getByTestId('gear-add-sport-trail_running'));
    fireEvent.changeText(screen.getByTestId('gear-add-initial'), '12,5');
    fireEvent.press(screen.getByTestId('gear-add-pinned'));
    fireEvent.press(screen.getByTestId('gear-add-submit'));

    await waitFor(() =>
      expect(mockCreateGear).toHaveBeenCalledWith({
        name: 'Speedgoat 7',
        sport: 'trail_running',
        initial_distance_meters: 12_500,
        warn_distance_meters: 500_000,
        pinned: true,
      }),
    );
  });

  it('reads and takes distances in miles for an athlete who reads imperial (carnet#835)', async () => {
    mockCreateGear.mockImplementation(async (request: { name: string }) => pair('new', { name: request.name }));
    const screen = renderScreen('imperial');

    const road = await screen.findByTestId('gear-road');
    expect(within(road).getByTestId('gear-distance-road')).toHaveTextContent('318 mi');
    expect(within(road).getByTestId('gear-warning-road')).toHaveTextContent(
      'This pair has 318 mi, past the 311 mi you set.',
    );

    fireEvent.press(screen.getByTestId('gear-add-open'));
    const sheet = screen.getByTestId('gear-sheet');
    expect(within(sheet).getByText('Distance it already has (mi)')).toBeTruthy();
    expect(within(sheet).getByText('Warn me at (mi)')).toBeTruthy();
    expect(screen.getByTestId('gear-add-warn').props.value).toBe('310.7');

    fireEvent.changeText(screen.getByTestId('gear-add-name'), 'Novablast 5');
    fireEvent.changeText(screen.getByTestId('gear-add-initial'), '50');
    fireEvent.press(screen.getByTestId('gear-add-submit'));

    // The warning left as shown stays the 500 km it stands for.
    await waitFor(() =>
      expect(mockCreateGear).toHaveBeenCalledWith({
        name: 'Novablast 5',
        sport: 'run',
        initial_distance_meters: 80_467,
        warn_distance_meters: 500_000,
        pinned: false,
      }),
    );
  });

  it('names a field out of bounds under it and sends nothing', async () => {
    const screen = renderScreen();
    await screen.findByTestId('gear-road');

    fireEvent.press(screen.getByTestId('gear-add-open'));
    fireEvent.changeText(screen.getByTestId('gear-add-warn'), '0');
    fireEvent.changeText(screen.getByTestId('gear-add-initial'), '20000');
    fireEvent.press(screen.getByTestId('gear-add-submit'));

    const form = screen.getByTestId('gear-add');
    expect(within(form).getByText('Give the pair a name.')).toBeTruthy();
    expect(within(form).getByText('Enter a distance from 0 to 10,000 km.')).toBeTruthy();
    expect(within(form).getByText('Enter a distance above 0, up to 10,000 km.')).toBeTruthy();
    expect(mockCreateGear).not.toHaveBeenCalled();
  });

  it('says a refused pair in the sheet and keeps it open', async () => {
    mockCreateGear.mockRejectedValue({ response: { status: 400, data: {} } });
    const screen = renderScreen();
    await screen.findByTestId('gear-road');

    fireEvent.press(screen.getByTestId('gear-add-open'));
    fireEvent.changeText(screen.getByTestId('gear-add-name'), 'Novablast 5');
    fireEvent.press(screen.getByTestId('gear-add-submit'));

    expect(await screen.findByTestId('gear-add-failed')).toHaveTextContent("This pair couldn't be added.");
    expect(screen.getByTestId('gear-add')).toBeTruthy();
  });

  it('edits a pair in the sheet, sending only what changed', async () => {
    mockUpdateGear.mockImplementation(async (id: string, changes: Partial<GearItem>) => {
      ledger = ledger.map((p) => (p.id === id ? { ...p, ...changes } : p));
      return ledger.find((p) => p.id === id);
    });
    const screen = renderScreen();

    fireEvent.press(await screen.findByTestId('gear-edit-road'));
    expect(screen.getByTestId('gear-sheet')).toHaveTextContent('Pegasus 41', { exact: false });
    expect(screen.getByTestId('gear-form-road-name').props.value).toBe('Pegasus 41');
    fireEvent.changeText(screen.getByTestId('gear-form-road-warn'), '650');
    fireEvent.press(screen.getByTestId('gear-form-road-pinned'));
    fireEvent.press(screen.getByTestId('gear-form-road-submit'));

    await waitFor(() =>
      expect(mockUpdateGear).toHaveBeenCalledWith('road', { warn_distance_meters: 650_000, pinned: true }),
    );
    await waitForGone(screen, 'gear-form-road');
  });

  it('closes an edit that changed nothing without a request', async () => {
    const screen = renderScreen('imperial');

    fireEvent.press(await screen.findByTestId('gear-edit-road'));
    fireEvent.press(screen.getByTestId('gear-form-road-submit'));

    await waitForGone(screen, 'gear-form-road');
    expect(mockUpdateGear).not.toHaveBeenCalled();
  });

  it('retires a pair in one tap, and says a refused retirement', async () => {
    mockRetireGear.mockImplementationOnce(async (id: string) => {
      ledger = ledger.map((p) => (p.id === id ? { ...p, retired_at: '2026-10-09T10:00:00Z' } : p));
      return ledger.find((p) => p.id === id);
    });
    const screen = renderScreen();

    fireEvent.press(await screen.findByTestId('gear-retire-road'));

    await waitFor(() => expect(mockRetireGear).toHaveBeenCalledWith('road'));
    await waitFor(() => expect(within(screen.getByTestId('gear-retired')).getByTestId('gear-road')).toBeTruthy());

    mockRetireGear.mockRejectedValueOnce({ response: { status: 400, data: {} } });
    fireEvent.press(screen.getByTestId('gear-retire-trail'));
    expect(await screen.findByTestId('gear-change-failed')).toHaveTextContent(
      "This change couldn't be saved. Try again.",
    );
  });

  it('deletes a pair nothing counts on after confirmation, and says to retire one the server refuses', async () => {
    const alert = answerAlerts('destructive');
    mockDeleteGear.mockRejectedValueOnce({ response: { status: 409, data: { code: 'ResourceLocked' } } });
    const screen = renderScreen();

    fireEvent.press(await screen.findByTestId('gear-delete-trail'));

    expect(alert).toHaveBeenCalledWith(
      'Delete “Speedgoat 6”?',
      "No activity is counted on this pair. It is removed from your shoes, and this can't be undone.",
      expect.any(Array),
    );
    await waitFor(() =>
      expect(alert).toHaveBeenLastCalledWith(
        'Delete “Speedgoat 6”?',
        "Activities are counted on this pair, so it can't be deleted. Retire it to keep its distance and history.",
      ),
    );
    expect(mockDeleteGear).toHaveBeenCalledWith('trail');
    expect(screen.getByTestId('gear-trail')).toBeTruthy();

    mockDeleteGear.mockImplementationOnce(async (id: string) => {
      ledger = ledger.filter((p) => p.id !== id);
    });
    fireEvent.press(screen.getByTestId('gear-delete-trail'));
    await waitForGone(screen, 'gear-trail');
  });

  it('keeps a pair the athlete did not confirm deleting', async () => {
    answerAlerts('cancel');
    const screen = renderScreen();

    fireEvent.press(await screen.findByTestId('gear-delete-trail'));

    expect(mockDeleteGear).not.toHaveBeenCalled();
    expect(screen.getByTestId('gear-trail')).toBeTruthy();
  });

  it('says the ledger could not be read and reads it again on Retry', async () => {
    mockListGear.mockRejectedValueOnce({ response: { status: 400, data: {} } });
    const screen = renderScreen();

    expect(await screen.findByTestId('gear-load-failed')).toHaveTextContent("Your shoes couldn't be loaded.", {
      exact: false,
    });
    fireEvent.press(screen.getByTestId('gear-retry'));

    expect(await screen.findByTestId('gear-road')).toBeTruthy();
    expect(mockListGear).toHaveBeenCalledTimes(2);
  });
});
