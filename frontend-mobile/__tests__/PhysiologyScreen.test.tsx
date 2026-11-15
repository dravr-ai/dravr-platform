// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Settings › Physiology on the phone — each anchor per sport with its value, range, source, date and confidence
// ABOUTME: The athlete API is mocked at the services barrel; assertions read the rendered rows, the requests sent and the empty state

import React from 'react';
import { fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { PhysiologyAnchor, PhysiologyResponse } from '@pierre/shared-types';
import type { DistanceUnit } from '@pierre/chat-utils';
import { UnitsContext } from '@pierre/ui-logic';

const mockGetPhysiology = jest.fn();
jest.mock('../src/services/api', () => ({
  athleteApi: { getPhysiology: (locale?: string) => mockGetPhysiology(locale) },
}));

// An icon is a view naming its glyph, so a row's text is only its words.
jest.mock('@expo/vector-icons', () => {
  const View = require('react-native').View;
  const glyph = (props: Record<string, unknown>) =>
    require('react').createElement(View, { testID: `icon-${props.name}` });
  return { Ionicons: glyph, Feather: glyph, MaterialCommunityIcons: glyph };
});

import { PhysiologyScreen } from '../src/screens/settings/PhysiologyScreen';

const FTP: PhysiologyAnchor = {
  anchor: 'ftp',
  unit: 'watts',
  value: 285,
  low: null,
  high: null,
  origin: 'athlete',
  as_of: '2026-10-20',
  method: null,
  confidence: 'low',
  value_text: 'about 285 W (estimate)',
  range_text: null,
  source_text: 'your own value',
};

const MAP: PhysiologyAnchor = {
  anchor: 'map',
  unit: 'watts',
  value: 380,
  low: 365,
  high: 395,
  origin: 'lab',
  as_of: '2026-11-02',
  method: 'ramp peak',
  confidence: 'high',
  value_text: '380 W',
  range_text: '365–395 W',
  source_text: 'from lab',
};

/** The run's critical speed as the server words it for each unit system. */
function criticalSpeed(units: DistanceUnit): PhysiologyAnchor {
  return {
    anchor: 'critical_speed',
    unit: 'metres_per_second',
    value: 4,
    low: 3.92,
    high: 4.08,
    origin: 'intervals.icu',
    as_of: null,
    method: null,
    confidence: 'low',
    value_text: units === 'imperial' ? 'about 6:34–6:51/mi (estimate)' : 'about 4:05–4:15/km (estimate)',
    range_text: null,
    source_text: 'from intervals.icu',
  };
}

function body(units: DistanceUnit): PhysiologyResponse {
  return {
    sports: [
      { sport: 'ride', anchors: [FTP, MAP] },
      { sport: 'run', anchors: [criticalSpeed(units)] },
    ],
  };
}

function newClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
}

function tree(client: QueryClient, units: DistanceUnit) {
  return (
    <QueryClientProvider client={client}>
      <UnitsContext.Provider value={units}>
        <PhysiologyScreen />
      </UnitsContext.Provider>
    </QueryClientProvider>
  );
}

describe('PhysiologyScreen', () => {
  let stored: DistanceUnit;

  beforeEach(async () => {
    jest.clearAllMocks();
    await i18n.changeLanguage('en');
    stored = 'metric';
    mockGetPhysiology.mockImplementation(async () => body(stored));
  });

  it('lists each family with its anchors, values, sources, days and confidence', async () => {
    const screen = render(tree(newClient(), 'metric'));

    const ride = await screen.findByTestId('physiology-ride');
    expect(within(ride).getByText('Cycling')).toBeTruthy();
    const ftp = within(ride).getByTestId('physiology-ride-ftp');
    expect(within(ftp).getByText('FTP')).toBeTruthy();
    expect(within(ftp).getByTestId('physiology-ride-ftp-value').props.children).toBe('about 285 W (estimate)');
    expect(within(ftp).getByTestId('physiology-ride-ftp-source').props.children).toBe('your own value');
    expect(within(ftp).getByText('As of Oct 20, 2026 · Low confidence')).toBeTruthy();

    expect(within(screen.getByTestId('physiology-run')).getByText('Running')).toBeTruthy();
    expect(screen.queryByTestId('physiology-swim')).toBeNull();
    expect(mockGetPhysiology).toHaveBeenCalledWith('en');
  });

  it('shows a measured anchor with the range it states, and a low-confidence one as its range alone', async () => {
    const screen = render(tree(newClient(), 'metric'));

    const map = await screen.findByTestId('physiology-ride-map');
    expect(within(map).getByText('MAP')).toBeTruthy();
    expect(within(map).getByText('380 W')).toBeTruthy();
    expect(within(map).getByText('As of Nov 2, 2026 · Range 365–395 W · High confidence')).toBeTruthy();

    const cs = screen.getByTestId('physiology-run-critical_speed');
    expect(within(cs).getByText('Critical speed')).toBeTruthy();
    expect(within(cs).getByText('about 4:05–4:15/km (estimate)')).toBeTruthy();
    // The value already reads as the range, so no second range, and a day the
    // writer did not record is said to be missing rather than left blank.
    expect(within(cs).getByText('Date not recorded · Low confidence')).toBeTruthy();
    expect(within(cs).queryByText(/Range/)).toBeNull();
  });

  it('reads the anchors again in the new units when the athlete switches to imperial', async () => {
    const client = newClient();
    const screen = render(tree(client, 'metric'));
    expect(await screen.findByText('about 4:05–4:15/km (estimate)')).toBeTruthy();

    stored = 'imperial';
    screen.rerender(tree(client, 'imperial'));

    expect(await screen.findByText('about 6:34–6:51/mi (estimate)')).toBeTruthy();
    expect(screen.queryByText(/\/km/)).toBeNull();
    expect(mockGetPhysiology).toHaveBeenCalledTimes(2);
  });

  it('says what the athlete can do when no anchor is stored', async () => {
    mockGetPhysiology.mockResolvedValue({ sports: [] });
    const screen = render(tree(newClient(), 'metric'));

    const empty = await screen.findByTestId('physiology-empty');
    expect(within(empty).getByText(i18n.t('physiology.empty'))).toBeTruthy();
    expect(i18n.t('physiology.empty')).toMatch(/chat/i);
    expect(screen.queryByTestId('physiology-ride')).toBeNull();
  });

  it('says the anchors could not be loaded, with a retry', async () => {
    mockGetPhysiology.mockRejectedValue(new Error('network down'));
    const screen = render(tree(newClient(), 'metric'));

    expect(await screen.findByTestId('physiology-load-failed')).toBeTruthy();
    mockGetPhysiology.mockResolvedValue(body('metric'));
    fireEvent.press(screen.getByTestId('physiology-retry'));
    await waitFor(() => expect(screen.getByTestId('physiology-ride')).toBeTruthy());
  });
});
