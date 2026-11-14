// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for Settings › Physiology — each anchor per sport with its value, range, source, date and confidence
// ABOUTME: The athlete API is mocked at the services barrel; assertions read the rendered rows, the requests sent and the empty state

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, within, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { PhysiologyAnchor, PhysiologyResponse } from '@pierre/shared-types';
import { UnitsContext } from '@pierre/ui-logic';
import type { DistanceUnit } from '@pierre/chat-utils';
import { i18n } from '@pierre/i18n';
import PhysiologySettings from '../PhysiologySettings';

const api = vi.hoisted(() => ({ getPhysiology: vi.fn() }));

vi.mock('../../../services/api', () => ({ athleteApi: api }));

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

function pane(queryClient: QueryClient, units: DistanceUnit) {
  return (
    <QueryClientProvider client={queryClient}>
      <UnitsContext.Provider value={units}>
        <PhysiologySettings />
      </UnitsContext.Provider>
    </QueryClientProvider>
  );
}

function newClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } });
}

describe('PhysiologySettings', () => {
  let stored: DistanceUnit;

  beforeEach(async () => {
    vi.clearAllMocks();
    await i18n.changeLanguage('en');
    stored = 'metric';
    api.getPhysiology.mockImplementation(async () => body(stored));
  });

  it('lists each family with its anchors, values, sources, days and confidence', async () => {
    render(pane(newClient(), 'metric'));

    const ride = await screen.findByTestId('physiology-ride');
    expect(within(ride).getByRole('heading', { name: 'Cycling' })).toBeInTheDocument();
    const ftp = within(ride).getByTestId('physiology-ride-ftp');
    expect(within(ftp).getByText('FTP')).toBeInTheDocument();
    expect(within(ftp).getByTestId('physiology-ride-ftp-value')).toHaveTextContent('about 285 W (estimate)');
    expect(within(ftp).getByTestId('physiology-ride-ftp-source')).toHaveTextContent('your own value');
    expect(within(ftp).getByTestId('physiology-ride-ftp-facts')).toHaveTextContent(
      'As of Oct 20, 2026 · Low confidence',
    );

    expect(within(screen.getByTestId('physiology-run')).getByRole('heading', { name: 'Running' })).toBeInTheDocument();
    expect(screen.queryByTestId('physiology-swim')).toBeNull();
    expect(api.getPhysiology).toHaveBeenCalledWith('en');
  });

  it('shows a measured anchor with the range it states, and a low-confidence one as its range alone', async () => {
    render(pane(newClient(), 'metric'));

    const map = await screen.findByTestId('physiology-ride-map');
    expect(within(map).getByText('MAP')).toBeInTheDocument();
    expect(within(map).getByTestId('physiology-ride-map-value')).toHaveTextContent('380 W');
    expect(within(map).getByTestId('physiology-ride-map-facts')).toHaveTextContent(
      'As of Nov 2, 2026 · Range 365–395 W · High confidence',
    );

    const cs = screen.getByTestId('physiology-run-critical_speed');
    expect(within(cs).getByText('Critical speed')).toBeInTheDocument();
    expect(within(cs).getByTestId('physiology-run-critical_speed-value')).toHaveTextContent(
      'about 4:05–4:15/km (estimate)',
    );
    // The value already reads as the range, so no second range, and a day the
    // writer did not record is said to be missing rather than left blank.
    expect(within(cs).getByTestId('physiology-run-critical_speed-facts')).toHaveTextContent(
      'Date not recorded · Low confidence',
    );
    expect(within(cs).queryByText(/Range/)).toBeNull();
  });

  it('reads the anchors again in the new units when the athlete switches to imperial', async () => {
    const queryClient = newClient();
    const view = render(pane(queryClient, 'metric'));
    expect(await screen.findByTestId('physiology-run-critical_speed-value')).toHaveTextContent('/km');

    stored = 'imperial';
    view.rerender(pane(queryClient, 'imperial'));

    await waitFor(() =>
      expect(screen.getByTestId('physiology-run-critical_speed-value')).toHaveTextContent(
        'about 6:34–6:51/mi (estimate)',
      ),
    );
    expect(api.getPhysiology).toHaveBeenCalledTimes(2);
  });

  it('says what the athlete can do when no anchor is stored', async () => {
    api.getPhysiology.mockResolvedValue({ sports: [] });
    render(pane(newClient(), 'metric'));

    const empty = await screen.findByTestId('physiology-empty');
    expect(empty).toHaveTextContent(i18n.t('physiology.empty'));
    expect(empty.textContent).toMatch(/chat/i);
    expect(screen.queryByRole('list')).toBeNull();
  });

  it('says the anchors could not be loaded', async () => {
    api.getPhysiology.mockRejectedValue(new Error('network down'));
    render(pane(newClient(), 'metric'));

    expect(await screen.findByTestId('physiology-load-failed')).toHaveAttribute('role', 'alert');
  });
});
