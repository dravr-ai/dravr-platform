// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Home's recent activities — the latest on the chat's map, the rest as sketches, and who asks for a route
// ABOUTME: Red if an indoor activity or one with a polyline costs a route call, or an empty state invents a row

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ActivityRouteResponse, RecentActivitiesResponse } from '@pierre/shared-types';
import { ThemeProvider } from '../../../hooks/useTheme';
import { RecentActivities } from '../RecentActivities';
import { DRAFT_DATE, formatInstant } from '../homeFormat';
import { activity, recentResponse, routeView, ROUTE_COORDINATES } from './homeFixtures';

const api = vi.hoisted(() => ({
  getRecentActivities: vi.fn<() => Promise<RecentActivitiesResponse>>(),
  getActivityRoute: vi.fn<(provider: string, id: string) => Promise<ActivityRouteResponse>>(),
  getProvidersStatus: vi.fn(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: {
    getRecentActivities: api.getRecentActivities,
    getActivityRoute: api.getActivityRoute,
  },
  providersApi: { getProvidersStatus: api.getProvidersStatus },
}));

/**
 * jsdom has no WebGL, so MapLibre cannot construct a real map; the stand-in
 * records what the chat's RouteView asks of it, as that component's own test
 * does. The Home page's job is only to hand RouteView the route unchanged.
 */
const maps = vi.hoisted(() => ({ constructed: [] as Array<Record<string, unknown>> }));
vi.mock('maplibre-gl', () => {
  const instance = {
    addControl: vi.fn(),
    addSource: vi.fn(),
    addLayer: vi.fn(),
    getSource: vi.fn(),
    on: vi.fn(),
    setStyle: vi.fn(),
    remove: vi.fn(),
  };
  return {
    Map: class {
      constructor(options: Record<string, unknown>) {
        maps.constructed.push(options);
        return instance;
      }
    },
    AttributionControl: class {},
    NavigationControl: class {},
    setWorkerUrl: vi.fn(),
  };
});

function connected(isConnected: boolean) {
  api.getProvidersStatus.mockResolvedValue({
    providers: [{ provider: 'strava', connected: isConnected, status: isConnected ? 'connected' : 'disconnected' }],
  });
}

function renderSection() {
  const onNavigate = vi.fn();
  const onOpenChatDraft = vi.fn();
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <RecentActivities onNavigate={onNavigate} onOpenChatDraft={onOpenChatDraft} />
      </ThemeProvider>
    </QueryClientProvider>,
  );
  return { onNavigate, onOpenChatDraft };
}

beforeEach(() => {
  vi.clearAllMocks();
  maps.constructed.length = 0;
  window.localStorage.clear();
  connected(true);
  api.getActivityRoute.mockImplementation(async (_provider, id) => ({
    route: routeView(id === 'act-5' ? 'Long ride' : 'Hill repeats'),
    reason: null,
  }));
});

describe('RecentActivities', () => {
  it('draws the latest on the chat map and asks for a route only where no polyline answers it', async () => {
    api.getRecentActivities.mockResolvedValue(recentResponse());
    renderSection();

    expect(await screen.findByRole('figure', { name: 'Map of the recorded route: Long ride' })).toBeInTheDocument();
    await waitFor(() => expect(maps.constructed).toHaveLength(1));
    expect(screen.getByText('source: strava')).toBeInTheDocument();

    // act-5 is the latest (the map); act-3 recorded GPS but carries no
    // polyline, so its sketch needs the route. act-4 has a polyline, act-2
    // and act-1 recorded no GPS: none of the three may reach the endpoint.
    await waitFor(() => expect(api.getActivityRoute).toHaveBeenCalledTimes(2));
    expect(api.getActivityRoute.mock.calls).toEqual(
      expect.arrayContaining([
        ['strava', 'act-5'],
        ['strava', 'act-3'],
      ]),
    );

    const rows = screen.getAllByTestId('home-activity-row');
    expect(rows).toHaveLength(4);
    // Polyline row and route-backed row draw a sketch; the two without GPS do not.
    await waitFor(() => expect(within(rows[1]).getByTestId('route-sketch')).toBeInTheDocument());
    expect(within(rows[0]).getByRole('img', { name: 'Sketch of the route' })).toBeInTheDocument();
    expect(within(rows[2]).queryByTestId('route-sketch')).toBeNull();
    expect(within(rows[3]).queryByTestId('route-sketch')).toBeNull();

    // Figures ride in mono after the sport label.
    const latest = screen.getByTestId('home-activity-latest');
    expect(within(latest).getByText('Long ride', { selector: 'span' })).toBeInTheDocument();
    expect(within(latest).getByText('92.40 km')).toHaveClass('font-mono');
    expect(within(latest).getByText('3h 41m')).toBeInTheDocument();
    expect(within(latest).getByText('+820 m')).toBeInTheDocument();
    expect(within(rows[2]).getByText(/^Indoor ride/)).toBeInTheDocument();
  });

  it('sketches from the route coordinates when the activity carries no polyline', async () => {
    api.getRecentActivities.mockResolvedValue(recentResponse());
    renderSection();

    const rows = await screen.findAllByTestId('home-activity-row');
    const sketch = await within(rows[1]).findByTestId('route-sketch');
    // Four coordinates, all distinct at the sketch's resolution: M plus three L.
    const d = sketch.querySelector('path')?.getAttribute('d') ?? '';
    expect(d.match(/[ML]/g)).toHaveLength(ROUTE_COORDINATES.length);
  });

  it('opens a chat draft asking to analyze the tapped activity', async () => {
    api.getRecentActivities.mockResolvedValue(recentResponse());
    const { onOpenChatDraft } = renderSection();

    const rows = await screen.findAllByTestId('home-activity-row');
    await userEvent.click(within(rows[0]).getByRole('button'));

    const date = formatInstant('2026-09-18T11:00:00Z', 'en', DRAFT_DATE);
    expect(onOpenChatDraft).toHaveBeenCalledExactlyOnceWith(`Analyze my activity from ${date} (Run)`);
  });

  it('says an indoor latest activity has no track, without asking for a route', async () => {
    api.getRecentActivities.mockResolvedValue(
      recentResponse({ activities: [activity({ id: 'indoor', has_gps: false, sport_type: 'virtual_ride' })] }),
    );
    renderSection();

    expect(await screen.findByText('This activity recorded no GPS track.')).toBeInTheDocument();
    expect(api.getActivityRoute).not.toHaveBeenCalled();
    expect(maps.constructed).toHaveLength(0);
  });

  it("says a route trimmed below two points is too short to draw", async () => {
    api.getRecentActivities.mockResolvedValue(recentResponse({ activities: [activity({ id: 'short' })] }));
    api.getActivityRoute.mockResolvedValue({ route: null, reason: 'too_short' });
    renderSection();

    expect(
      await screen.findByText('This route is too short to draw without showing where it starts.'),
    ).toBeInTheDocument();
    expect(maps.constructed).toHaveLength(0);
  });

  it('says the map could not be loaded when the route read fails, and retries on request', async () => {
    api.getRecentActivities.mockResolvedValue(recentResponse({ activities: [activity({ id: 'broken' })] }));
    api.getActivityRoute.mockRejectedValueOnce(new Error('boom'));
    renderSection();

    const failed = await screen.findByTestId('home-route-failed');
    expect(failed).toHaveTextContent("The map couldn't be loaded.");
    api.getActivityRoute.mockResolvedValueOnce({ route: routeView('Morning run'), reason: null });
    await userEvent.click(within(failed).getByRole('button', { name: 'Retry' }));
    expect(await screen.findByRole('figure', { name: 'Map of the recorded route: Morning run' })).toBeInTheDocument();
  });

  it('asks an athlete with no provider to connect one, and leads to the connections pane', async () => {
    connected(false);
    api.getRecentActivities.mockResolvedValue(recentResponse({ activities: [], as_of: null }));
    const { onNavigate } = renderSection();

    const prompt = await screen.findByTestId('home-connect-provider');
    expect(prompt).toHaveTextContent('Connect a fitness provider to see your recent activities here.');
    await userEvent.click(within(prompt).getByRole('button', { name: 'Connect' }));
    expect(onNavigate).toHaveBeenCalledExactlyOnceWith('settings/connections');
    expect(screen.queryByTestId('home-activity-latest')).toBeNull();
  });

  it('says there are no activities yet for a connected athlete with an empty cache', async () => {
    api.getRecentActivities.mockResolvedValue(recentResponse({ activities: [] }));
    renderSection();

    expect(await screen.findByText('No activities yet. They show up here once your provider syncs.')).toBeInTheDocument();
    expect(screen.queryByTestId('home-connect-provider')).toBeNull();
  });

  it('says the activities could not be loaded, and offers a retry', async () => {
    api.getRecentActivities.mockRejectedValueOnce(new Error('/api/me/activities/recent answered with a body that does not match its contract'));
    renderSection();

    const failed = await screen.findByTestId('home-activities-failed');
    expect(failed).toHaveTextContent("Your recent activities couldn't be loaded.");
    api.getRecentActivities.mockResolvedValueOnce(recentResponse());
    await userEvent.click(within(failed).getByRole('button', { name: 'Retry' }));
    expect(await screen.findByTestId('home-activity-latest')).toBeInTheDocument();
  });

  it('prints when the cache last synced', async () => {
    api.getRecentActivities.mockResolvedValue(recentResponse());
    renderSection();

    expect(await screen.findByText(/^Last synced: /)).toBeInTheDocument();
    expect(screen.queryByText('Checking your provider for new activities…')).toBeNull();
  });

  it('heads its section at the level of Today and This week', async () => {
    api.getRecentActivities.mockResolvedValue(recentResponse());
    renderSection();

    expect(await screen.findByRole('heading', { level: 3, name: 'Recent activities' })).toBeInTheDocument();
  });

  it('keeps a sketch column only when a row can fill it', async () => {
    api.getRecentActivities.mockResolvedValue(recentResponse());
    renderSection();

    await screen.findByTestId('home-activity-latest');
    // The fixture's four earlier rows mix GPS and indoor, so every row keeps
    // the column and their text stays on one line.
    expect(screen.getAllByTestId('home-sketch-slot')).toHaveLength(4);
  });

  it('drops the sketch column when every earlier row was recorded indoors', async () => {
    const indoor = [1, 2, 3, 4, 5].map((n) =>
      activity({ id: `indoor-${n}`, has_gps: false, summary_polyline: null, start_date: `2026-09-1${n}T08:00:00Z` }),
    );
    api.getRecentActivities.mockResolvedValue(recentResponse({ activities: indoor }));
    renderSection();

    await screen.findByTestId('home-activity-latest');
    expect(screen.getAllByTestId('home-activity-row')).toHaveLength(4);
    expect(screen.queryByTestId('home-sketch-slot')).toBeNull();
    expect(api.getActivityRoute).not.toHaveBeenCalled();
  });
});
