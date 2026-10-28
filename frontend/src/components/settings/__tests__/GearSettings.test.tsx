// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for Settings › Shoes — each pair's distance and warning, adding and editing a pair, retiring and deleting one
// ABOUTME: The gear API is mocked at the services barrel; assertions read the rendered rows, the requests sent and the refusals said

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, within, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { GearItem } from '@pierre/shared-types';
import { UnitsContext } from '@pierre/ui-logic';
import { i18n } from '@pierre/i18n';
import type { DistanceUnit } from '@pierre/chat-utils';
import GearSettings from '../GearSettings';

const api = vi.hoisted(() => ({
  listGear: vi.fn(),
  createGear: vi.fn(),
  updateGear: vi.fn(),
  retireGear: vi.fn(),
  restoreGear: vi.fn(),
  deleteGear: vi.fn(),
}));

vi.mock('../../../services/api', () => ({ gearApi: api }));

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

function renderPane(units: DistanceUnit = 'metric') {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <UnitsContext.Provider value={units}>
        <GearSettings />
      </UnitsContext.Provider>
    </QueryClientProvider>,
  );
}

describe('GearSettings', () => {
  let ledger: GearItem[];

  beforeEach(() => {
    vi.clearAllMocks();
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
    api.listGear.mockImplementation(async () => ({ gear: ledger }));
  });

  it('shows each pair in use with its distance, its badges and, past its threshold, the warning', async () => {
    renderPane();

    const road = await screen.findByTestId('gear-road');
    expect(screen.getByRole('list', { name: 'In use' })).toContainElement(road);
    expect(within(road).getByTestId('gear-distance-road')).toHaveTextContent('512 km');
    expect(road).toHaveTextContent('Pegasus 41');
    expect(road).toHaveTextContent('Run');
    expect(road).toHaveTextContent('41 activities');
    expect(within(road).getByTestId('gear-default-road')).toHaveTextContent('Default');
    expect(within(road).getByTestId('gear-warning-road')).toHaveTextContent(
      'This pair has 512 km, past the 500 km you set.',
    );
    // A pair with activities is retired, never deleted.
    expect(within(road).queryByTestId('gear-delete-road')).not.toBeInTheDocument();

    const trail = screen.getByTestId('gear-trail');
    expect(trail).toHaveTextContent('Trail running');
    expect(trail).toHaveTextContent('Pinned');
    expect(within(trail).queryByTestId('gear-warning-trail')).not.toBeInTheDocument();
    expect(within(trail).getByTestId('gear-delete-trail')).toBeInTheDocument();
  });

  // carnet#1212: the count chose its form with an inline `=== 1`, so a new
  // pair read « 0 activités » to a French athlete.
  it('counts a new pair\'s activities in the French singular', async () => {
    await i18n.changeLanguage('fr');
    try {
      renderPane();
      expect(await screen.findByTestId('gear-trail')).toHaveTextContent('0 activité');
      expect(screen.getByTestId('gear-trail')).not.toHaveTextContent('0 activités');
      expect(screen.getByTestId('gear-road')).toHaveTextContent('41 activités');
    } finally {
      await i18n.changeLanguage('en');
    }
  });

  it('lists a retired pair apart, with its distance kept and a way back into use', async () => {
    api.restoreGear.mockImplementation(async (id: string) => {
      ledger = ledger.map((p) => (p.id === id ? { ...p, retired_at: null } : p));
      return ledger.find((p) => p.id === id);
    });
    const user = userEvent.setup();
    renderPane();

    const retired = await screen.findByTestId('gear-retired');
    expect(within(retired).getByTestId('gear-distance-old')).toHaveTextContent('820 km');
    expect(within(screen.getByTestId('gear-in-use')).queryByTestId('gear-old')).not.toBeInTheDocument();

    await user.click(within(retired).getByTestId('gear-restore-old'));

    expect(api.restoreGear).toHaveBeenCalledWith('old');
    await waitFor(() => expect(within(screen.getByTestId('gear-in-use')).getByTestId('gear-old')).toBeInTheDocument());
  });

  it('says so when the athlete has no pair', async () => {
    ledger = [];
    renderPane();

    expect(await screen.findByTestId('gear-empty')).toHaveTextContent(
      'No pairs yet. Add the shoes you train in to follow their distance.',
    );
  });

  it('adds a pair from the form, its warning at 500 km unless changed', async () => {
    api.createGear.mockImplementation(async (request: { name: string }) => {
      const created = pair('new', { name: request.name, initial_distance_meters: 120_000, distance_meters: 120_000 });
      ledger = [created, ...ledger];
      return created;
    });
    const user = userEvent.setup();
    renderPane();
    await screen.findByTestId('gear-road');

    await user.type(screen.getByTestId('gear-add-name'), '  Novablast 5 ');
    await user.selectOptions(screen.getByTestId('gear-add-sport'), 'run');
    await user.type(screen.getByTestId('gear-add-initial'), '120');
    await user.click(screen.getByTestId('gear-add-submit'));

    expect(api.createGear).toHaveBeenCalledWith({
      name: 'Novablast 5',
      sport: 'run',
      initial_distance_meters: 120_000,
      warn_distance_meters: 500_000,
      pinned: false,
    });
    expect(await screen.findByTestId('gear-new')).toHaveTextContent('Novablast 5');
    expect(screen.getByTestId('gear-add-name')).toHaveValue('');
  });

  it('reads and takes distances in miles for an athlete who reads imperial (carnet#835)', async () => {
    api.createGear.mockImplementation(async (request: { name: string }) => {
      const created = pair('new', { name: request.name });
      ledger = [created, ...ledger];
      return created;
    });
    const user = userEvent.setup();
    renderPane('imperial');

    const road = await screen.findByTestId('gear-road');
    expect(within(road).getByTestId('gear-distance-road')).toHaveTextContent('318 mi');
    expect(within(road).getByTestId('gear-warning-road')).toHaveTextContent(
      'This pair has 318 mi, past the 311 mi you set.',
    );
    expect(screen.getByLabelText('Distance it already has (mi)')).toBe(screen.getByTestId('gear-add-initial'));
    expect(screen.getByTestId('gear-add-warn')).toHaveValue('310.7');

    await user.type(screen.getByTestId('gear-add-name'), 'Novablast 5');
    await user.type(screen.getByTestId('gear-add-initial'), '50');
    await user.click(screen.getByTestId('gear-add-submit'));

    // The warning left as shown stays the 500 km it stands for.
    expect(api.createGear).toHaveBeenCalledWith({
      name: 'Novablast 5',
      sport: 'run',
      initial_distance_meters: 80_467,
      warn_distance_meters: 500_000,
      pinned: false,
    });
  });

  it('names a field out of bounds beside it and sends nothing', async () => {
    const user = userEvent.setup();
    renderPane();
    await screen.findByTestId('gear-road');

    await user.clear(screen.getByTestId('gear-add-warn'));
    await user.type(screen.getByTestId('gear-add-warn'), '0');
    await user.click(screen.getByTestId('gear-add-submit'));

    const form = screen.getByTestId('gear-add');
    expect(form).toHaveTextContent('Give the pair a name.');
    expect(form).toHaveTextContent('Enter a distance above 0, up to 10,000 km.');
    expect(api.createGear).not.toHaveBeenCalled();
  });

  it('sends an edit with only what changed', async () => {
    api.updateGear.mockImplementation(async (id: string, changes: Partial<GearItem>) => {
      ledger = ledger.map((p) => (p.id === id ? { ...p, ...changes } : p));
      return ledger.find((p) => p.id === id);
    });
    const user = userEvent.setup();
    renderPane();

    await user.click(await screen.findByTestId('gear-edit-road'));
    const warn = screen.getByTestId('gear-form-road-warn');
    await user.clear(warn);
    await user.type(warn, '650');
    await user.click(screen.getByTestId('gear-form-road-pinned'));
    await user.click(screen.getByTestId('gear-form-road-submit'));

    expect(api.updateGear).toHaveBeenCalledWith('road', { warn_distance_meters: 650_000, pinned: true });
    await waitFor(() => expect(screen.queryByTestId('gear-form-road')).not.toBeInTheDocument());
  });

  it('retires a pair in one tap', async () => {
    api.retireGear.mockImplementation(async (id: string) => {
      ledger = ledger.map((p) => (p.id === id ? { ...p, retired_at: '2026-10-09T10:00:00Z' } : p));
      return ledger.find((p) => p.id === id);
    });
    const user = userEvent.setup();
    renderPane();

    await user.click(await screen.findByTestId('gear-retire-road'));

    expect(api.retireGear).toHaveBeenCalledWith('road');
    await waitFor(() => expect(within(screen.getByTestId('gear-retired')).getByTestId('gear-road')).toBeInTheDocument());
  });

  it('deletes a pair nothing counts on after confirmation, and says to retire one the server refuses', async () => {
    api.deleteGear.mockRejectedValueOnce({ response: { status: 409, data: { code: 'ResourceLocked' } } });
    const user = userEvent.setup();
    renderPane();

    await user.click(await screen.findByTestId('gear-delete-trail'));
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('Delete “Speedgoat 6”?');
    await user.click(within(dialog).getByRole('button', { name: 'Delete' }));

    expect(api.deleteGear).toHaveBeenCalledWith('trail');
    expect(
      await within(dialog).findByText(
        "Activities are counted on this pair, so it can't be deleted. Retire it to keep its distance and history.",
      ),
    ).toBeInTheDocument();

    api.deleteGear.mockImplementationOnce(async (id: string) => {
      ledger = ledger.filter((p) => p.id !== id);
    });
    await user.click(within(dialog).getByRole('button', { name: 'Delete' }));
    await waitFor(() => expect(screen.queryByTestId('gear-trail')).not.toBeInTheDocument());
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});
