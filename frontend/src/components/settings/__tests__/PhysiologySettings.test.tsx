// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for Settings › Physiology — each anchor per sport with its value, range, source, date and confidence, and its edit
// ABOUTME: The athlete API is mocked at the services barrel; assertions read the rendered rows, the requests sent and the empty state

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { fireEvent, render, screen, within, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { OfferedFamily, PhysiologyAnchor, PhysiologyResponse } from '@pierre/shared-types';
import { PHYSIOLOGY_KEY } from '@pierre/shared-constants';
import { UnitsContext } from '@pierre/ui-logic';
import type { DistanceUnit } from '@pierre/chat-utils';
import { i18n } from '@pierre/i18n';
import PhysiologySettings from '../PhysiologySettings';

const api = vi.hoisted(() => ({ getPhysiology: vi.fn(), updatePhysiologyAnchor: vi.fn() }));

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

/** What each family can hold, as the server lists it. */
const OFFERED: OfferedFamily[] = [
  {
    sport: 'ride',
    anchors: [
      { anchor: 'ftp', unit: 'watts' },
      { anchor: 'map', unit: 'watts' },
    ],
  },
  {
    sport: 'run',
    anchors: [
      { anchor: 'critical_speed', unit: 'metres_per_second' },
      { anchor: 'threshold_speed', unit: 'metres_per_second' },
    ],
  },
  { sport: 'swim', anchors: [{ anchor: 'critical_speed', unit: 'metres_per_second' }] },
];

function body(units: DistanceUnit): PhysiologyResponse {
  return {
    sports: [
      { sport: 'ride', anchors: [FTP, MAP] },
      { sport: 'run', anchors: [criticalSpeed(units)] },
    ],
    offered: OFFERED,
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
    api.getPhysiology.mockResolvedValue({ sports: [], offered: OFFERED });
    render(pane(newClient(), 'metric'));

    const empty = await screen.findByTestId('physiology-empty');
    expect(empty).toHaveTextContent(i18n.t(PHYSIOLOGY_KEY.empty));
    expect(empty.textContent).toMatch(/chat/i);
    expect(screen.queryByRole('list')).toBeNull();
    expect(screen.getByTestId('physiology-add')).toBeInTheDocument();
  });

  it('says the anchors could not be loaded', async () => {
    api.getPhysiology.mockRejectedValue(new Error('network down'));
    render(pane(newClient(), 'metric'));

    expect(await screen.findByTestId('physiology-load-failed')).toHaveAttribute('role', 'alert');
  });

  it('changes an anchor from its row: the form opens on it, asks how it was found, and the pane takes the answer', async () => {
    api.updatePhysiologyAnchor.mockResolvedValue({
      ...body('metric'),
      sports: [{ sport: 'ride', anchors: [{ ...FTP, value: 290, value_text: '290 W', confidence: 'medium' }, MAP] }],
    });
    render(pane(newClient(), 'metric'));

    fireEvent.click(await screen.findByTestId('physiology-ride-ftp-edit'));
    const editor = await screen.findByTestId('physiology-editor');
    const value = within(editor).getByTestId('physiology-editor-value');
    expect(value).toHaveValue('285');
    expect(within(editor).getByTestId('physiology-editor-save')).toBeDisabled();

    fireEvent.change(value, { target: { value: '290' } });
    fireEvent.change(within(editor).getByTestId('physiology-editor-kind'), { target: { value: 'measured' } });
    fireEvent.change(within(editor).getByTestId('physiology-editor-confidence'), { target: { value: 'medium' } });
    fireEvent.click(within(editor).getByTestId('physiology-editor-save'));

    await waitFor(() =>
      expect(api.updatePhysiologyAnchor).toHaveBeenCalledWith(
        'ride',
        'ftp',
        {
          value: 290,
          low: null,
          high: null,
          pace_per: null,
          kind: 'measured',
          confidence: 'medium',
          method: null,
          measured_on: null,
        },
        'en',
      ),
    );
    await waitFor(() => expect(screen.getByTestId('physiology-ride-ftp-value')).toHaveTextContent('290 W'));
    expect(screen.queryByTestId('physiology-editor')).toBeNull();
  });

  it('adds a run pace typed per mile for an imperial athlete', async () => {
    stored = 'imperial';
    api.updatePhysiologyAnchor.mockResolvedValue(body('imperial'));
    render(pane(newClient(), 'imperial'));

    fireEvent.click(await screen.findByTestId('physiology-add'));
    const editor = await screen.findByTestId('physiology-editor');
    fireEvent.change(within(editor).getByTestId('physiology-editor-sport'), { target: { value: 'run' } });
    fireEvent.change(within(editor).getByTestId('physiology-editor-anchor-choice'), {
      target: { value: 'threshold_speed' },
    });
    expect(within(editor).getByLabelText(/min:s\/mi/)).toBeInTheDocument();
    fireEvent.change(within(editor).getByTestId('physiology-editor-value'), { target: { value: '6:42' } });
    fireEvent.change(within(editor).getByTestId('physiology-editor-kind'), { target: { value: 'estimated' } });
    fireEvent.click(within(editor).getByTestId('physiology-editor-save'));

    await waitFor(() => expect(api.updatePhysiologyAnchor).toHaveBeenCalledTimes(1));
    const [sport, anchor, edit] = api.updatePhysiologyAnchor.mock.calls[0];
    expect([sport, anchor]).toEqual(['run', 'threshold_speed']);
    expect(edit).toMatchObject({ value: 402, pace_per: 'mi', kind: 'estimated', confidence: 'low' });
  });

  it('refuses a pace that is not minutes and seconds before anything is sent', async () => {
    render(pane(newClient(), 'metric'));

    fireEvent.click(await screen.findByTestId('physiology-run-critical_speed-edit'));
    const editor = await screen.findByTestId('physiology-editor');
    fireEvent.change(within(editor).getByTestId('physiology-editor-value'), { target: { value: '250' } });
    fireEvent.change(within(editor).getByTestId('physiology-editor-kind'), { target: { value: 'measured' } });
    fireEvent.click(within(editor).getByTestId('physiology-editor-save'));

    expect(await within(editor).findByText(i18n.t('physiology.edit.invalidPace'))).toBeInTheDocument();
    expect(api.updatePhysiologyAnchor).not.toHaveBeenCalled();
  });

  it('says why the server refused a value, and keeps the form open', async () => {
    api.updatePhysiologyAnchor.mockRejectedValue(
      Object.assign(new Error('Request failed with status code 400'), {
        response: { status: 400, data: { message: 'the ride map (290) must be above its critical_power (300)' } },
      }),
    );
    render(pane(newClient(), 'metric'));

    fireEvent.click(await screen.findByTestId('physiology-ride-map-edit'));
    const editor = await screen.findByTestId('physiology-editor');
    fireEvent.change(within(editor).getByTestId('physiology-editor-value'), { target: { value: '290' } });
    fireEvent.change(within(editor).getByTestId('physiology-editor-kind'), { target: { value: 'measured' } });
    fireEvent.click(within(editor).getByTestId('physiology-editor-save'));

    expect(await within(editor).findByTestId('physiology-editor-error')).toHaveTextContent(
      /must be above its critical_power/,
    );
    expect(screen.getByTestId('physiology-editor')).toBeInTheDocument();
  });
});
