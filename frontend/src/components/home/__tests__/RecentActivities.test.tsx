// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Home's recent activities — the latest on the chat's map, the rest as sketches, who asks for a route, who is told to connect
// ABOUTME: Red if a stored no-GPS activity or one with a polyline costs a route call, a never-read route is called trackless, or a dead connection hides its rows

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type {
  ActivityRouteResponse,
  ExtendedProviderStatus,
  ProvidersStatusResponse,
  RecentActivitiesResponse,
} from '@pierre/shared-types';
import { ThemeProvider } from '../../../hooks/useTheme';
import { RecentActivities } from '../RecentActivities';
import { DRAFT_DATE, formatInstant, formatNameList, formatSyncTime } from '../homeFormat';
import { activity, providerStatus, recentResponse, routeView, ROUTE_COORDINATES } from './homeFixtures';

const api = vi.hoisted(() => ({
  getRecentActivities:
    vi.fn<(limit?: number, options?: { retry?: boolean }) => Promise<RecentActivitiesResponse>>(),
  getActivityRoute:
    vi.fn<(provider: string, id: string, options?: { retry?: boolean }) => Promise<ActivityRouteResponse>>(),
  getProvidersStatus: vi.fn<() => Promise<ProvidersStatusResponse>>(),
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

function providers(...list: ExtendedProviderStatus[]) {
  api.getProvidersStatus.mockResolvedValue({ providers: list });
}

function connected(isConnected: boolean) {
  providers(providerStatus({ provider: 'strava', display_name: 'Strava', connected: isConnected }));
}

/** A connected provider whose session died: the server refreshes nothing for it. */
function toReconnect(provider: string, display_name: string): ExtendedProviderStatus {
  return providerStatus({ provider, display_name, needs_reauth: true });
}

/** The routes the five fixture activities answer with, by activity id. */
const FIXTURE_ROUTES: Record<string, { title: string; source: string }> = {
  'act-5': { title: 'Long ride', source: 'strava' },
  'act-3': { title: 'Hill repeats', source: 'strava' },
  'act-1': { title: 'Lake loop', source: 'garmin' },
};

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
  /** Every read the section started has answered — what an absence has to wait for. */
  const settled = () => waitFor(() => expect(queryClient.isFetching()).toBe(0));
  return { onNavigate, onOpenChatDraft, settled };
}

beforeEach(() => {
  vi.clearAllMocks();
  maps.constructed.length = 0;
  window.localStorage.clear();
  connected(true);
  api.getActivityRoute.mockImplementation(async (_provider, id) => {
    const known = FIXTURE_ROUTES[id] ?? { title: 'Morning run', source: 'strava' };
    return { route: routeView(known.title, known.source), reason: null };
  });
});

describe('RecentActivities', () => {
  it('draws the latest on the chat map and asks for a route only where no polyline answers it', async () => {
    api.getRecentActivities.mockResolvedValue(recentResponse());
    renderSection();

    expect(await screen.findByRole('figure', { name: 'Map of the recorded route: Long ride' })).toBeInTheDocument();
    await waitFor(() => expect(maps.constructed).toHaveLength(1));
    expect(screen.getByText('source: strava')).toBeInTheDocument();

    // act-5 is the latest (the map); act-3 and Garmin's act-1 carry no
    // polyline, so their sketches need the route. act-4 has a polyline and
    // act-2 says its route held no GPS: neither may reach the endpoint.
    await waitFor(() => expect(api.getActivityRoute).toHaveBeenCalledTimes(3));
    expect(api.getActivityRoute.mock.calls).toEqual(
      expect.arrayContaining([
        ['strava', 'act-5'],
        ['strava', 'act-3'],
        ['garmin', 'act-1'],
      ]),
    );

    const rows = screen.getAllByTestId('home-activity-row');
    expect(rows).toHaveLength(4);
    // The polyline row and the two route-backed rows draw a sketch; the one
    // whose route held no GPS does not.
    await waitFor(() => expect(within(rows[1]).getByTestId('route-sketch')).toBeInTheDocument());
    await waitFor(() => expect(within(rows[3]).getByTestId('route-sketch')).toBeInTheDocument());
    expect(within(rows[0]).getByRole('img', { name: 'Sketch of the route' })).toBeInTheDocument();
    expect(within(rows[2]).queryByTestId('route-sketch')).toBeNull();

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

  it('says a latest activity whose stored route held no GPS has no track, without asking for a route', async () => {
    api.getRecentActivities.mockResolvedValue(
      recentResponse({ activities: [activity({ id: 'indoor', has_gps: false, sport_type: 'virtual_ride' })] }),
    );
    const { settled } = renderSection();

    expect(await screen.findByText('This activity recorded no GPS track.')).toBeInTheDocument();
    await settled();
    expect(api.getActivityRoute).not.toHaveBeenCalled();
    expect(maps.constructed).toHaveLength(0);
  });

  it('asks for the route of an activity whose list carried no position, and draws the map and the sketch from the answer', async () => {
    api.getRecentActivities.mockResolvedValue(
      recentResponse({
        activities: [
          activity({
            id: 'g-2',
            provider: 'garmin',
            name: 'River ride',
            sport_type: 'ride',
            has_gps: true,
            summary_polyline: null,
            start_date: '2026-09-21T13:00:00Z',
          }),
          activity({
            id: 'g-1',
            provider: 'garmin',
            name: 'Lake loop',
            has_gps: true,
            summary_polyline: null,
            start_date: '2026-09-19T11:00:00Z',
          }),
        ],
      }),
    );
    api.getActivityRoute.mockImplementation(async (_provider, id) => ({
      route: routeView(id === 'g-2' ? 'River ride' : 'Lake loop', 'garmin'),
      reason: null,
    }));
    const { settled } = renderSection();

    expect(await screen.findByRole('figure', { name: 'Map of the recorded route: River ride' })).toBeInTheDocument();
    await waitFor(() => expect(maps.constructed).toHaveLength(1));
    expect(screen.getByText('source: garmin')).toBeInTheDocument();

    const sketch = await within(screen.getByTestId('home-activity-row')).findByTestId('route-sketch');
    const d = sketch.querySelector('path')?.getAttribute('d') ?? '';
    expect(d.match(/[ML]/g)).toHaveLength(ROUTE_COORDINATES.length);

    await settled();
    expect(api.getActivityRoute).toHaveBeenCalledTimes(2);
    expect(api.getActivityRoute.mock.calls).toEqual(
      expect.arrayContaining([
        ['garmin', 'g-2'],
        ['garmin', 'g-1'],
      ]),
    );
    expect(screen.queryByText('This activity recorded no GPS track.')).toBeNull();
  });

  it('says there is no track once the route endpoint answers that the recording held no GPS', async () => {
    api.getRecentActivities.mockResolvedValue(
      recentResponse({
        activities: [
          activity({ id: 'g-2', provider: 'garmin', name: 'Treadmill', has_gps: true, summary_polyline: null }),
          activity({
            id: 'g-1',
            provider: 'garmin',
            name: 'Track session',
            has_gps: true,
            summary_polyline: null,
            start_date: '2026-09-19T11:00:00Z',
          }),
        ],
      }),
    );
    api.getActivityRoute.mockResolvedValue({ route: null, reason: 'no_gps' });
    const { settled } = renderSection();

    const latest = await screen.findByTestId('home-activity-latest');
    expect(await within(latest).findByText('This activity recorded no GPS track.')).toBeInTheDocument();
    expect(api.getActivityRoute).toHaveBeenCalledWith('garmin', 'g-2');
    expect(maps.constructed).toHaveLength(0);

    // The row asked as well, and an answer without a route draws no sketch.
    await settled();
    expect(api.getActivityRoute).toHaveBeenCalledWith('garmin', 'g-1');
    expect(api.getActivityRoute).toHaveBeenCalledTimes(2);
    expect(screen.queryByTestId('route-sketch')).toBeNull();
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

  it('says the map could not be loaded when the route read fails twice, and retries on request', async () => {
    api.getRecentActivities.mockResolvedValue(recentResponse({ activities: [activity({ id: 'broken' })] }));
    // The route query asks once more before it gives up, never the client's default three times.
    api.getActivityRoute.mockRejectedValueOnce(new Error('boom')).mockRejectedValueOnce(new Error('boom'));
    renderSection();

    const failed = await screen.findByTestId('home-route-failed', {}, { timeout: 4000 });
    expect(api.getActivityRoute).toHaveBeenCalledTimes(2);
    expect(failed).toHaveTextContent("The map couldn't be loaded.");
    api.getActivityRoute.mockResolvedValueOnce({ route: routeView('Morning run'), reason: null });
    await userEvent.click(within(failed).getByRole('button', { name: 'Retry' }));
    expect(await screen.findByRole('figure', { name: 'Map of the recorded route: Morning run' })).toBeInTheDocument();
  });

  it('says the map could not be read when the route endpoint answers unavailable, and retries past it on request', async () => {
    api.getRecentActivities.mockResolvedValue(recentResponse({ activities: [activity({ id: 'unread' })] }));
    api.getActivityRoute.mockResolvedValueOnce({ route: null, reason: 'unavailable' });
    renderSection();

    const failed = await screen.findByTestId('home-route-failed');
    expect(failed).toHaveTextContent("The map couldn't be loaded.");
    expect(screen.queryByText('Loading the map…')).toBeNull();
    expect(screen.queryByText('This activity recorded no GPS track.')).toBeNull();
    // Only a retry the server reads past its stored `unavailable` can draw a
    // route inside the answer's ten minutes: the retry has to say it is one.
    api.getActivityRoute.mockResolvedValueOnce({ route: routeView('Morning run'), reason: null });
    await userEvent.click(within(failed).getByRole('button', { name: 'Retry' }));
    expect(await screen.findByRole('figure', { name: 'Map of the recorded route: Morning run' })).toBeInTheDocument();
    expect(api.getActivityRoute).toHaveBeenCalledTimes(2);
    expect(api.getActivityRoute).toHaveBeenLastCalledWith('strava', 'unread', { retry: true });
  });

  it('shows the map loading while its retry is in flight, and the failure again when the provider still fails', async () => {
    api.getRecentActivities.mockResolvedValue(recentResponse({ activities: [activity({ id: 'flaky' })] }));
    api.getActivityRoute.mockResolvedValueOnce({ route: null, reason: 'unavailable' });
    renderSection();

    const failed = await screen.findByTestId('home-route-failed');
    let answer: (response: ActivityRouteResponse) => void = () => undefined;
    api.getActivityRoute.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        }),
    );
    await userEvent.click(within(failed).getByRole('button', { name: 'Retry' }));
    expect(await screen.findByText('Loading the map…')).toBeInTheDocument();
    expect(screen.queryByTestId('home-route-failed')).toBeNull();

    answer({ route: null, reason: 'unavailable' });
    expect(await screen.findByTestId('home-route-failed')).toHaveTextContent("The map couldn't be loaded.");
  });

  it('names the provider whose sync failed, dates the page from its last good sync once, and retries the sync', async () => {
    api.getRecentActivities.mockResolvedValueOnce(
      recentResponse({
        // Garmin synced this morning; Strava's refresh failed after its own
        // last good sync the evening before.
        as_of: '2026-09-29T08:02:00Z',
        sync_failure: {
          provider: 'strava',
          provider_name: 'Strava',
          failed_at: '2026-09-29T14:04:00Z',
          last_synced_at: '2026-09-28T21:15:00Z',
        },
      }),
    );
    renderSection();

    const failed = await screen.findByTestId('home-sync-failed');
    expect(failed).toHaveAttribute('role', 'alert');
    expect(failed).toHaveTextContent('Strava · Sync failed');
    expect(failed).not.toHaveTextContent(/Last synced/);
    const synced = screen.getAllByText(/Last synced: /);
    expect(synced).toHaveLength(1);
    expect(synced[0]).toHaveTextContent(`Last synced: ${formatSyncTime('2026-09-28T21:15:00Z', 'en')}`);
    const retry = within(failed).getByTestId('home-sync-retry');
    expect(retry).toHaveClass('touch-target');

    // The retry's own answer is what the real server sends: the refresh it
    // started is running, the failure it has not superseded yet still stands.
    api.getRecentActivities.mockResolvedValueOnce(
      recentResponse({
        stale: true,
        sync_failure: {
          provider: 'strava',
          provider_name: 'Strava',
          failed_at: '2026-09-29T14:04:00Z',
          last_synced_at: '2026-09-28T21:15:00Z',
        },
      }),
    );
    await userEvent.click(retry);
    await waitFor(() => expect(api.getRecentActivities).toHaveBeenCalledTimes(2));
    expect(api.getRecentActivities).toHaveBeenLastCalledWith(undefined, { retry: true });
    // While that refresh runs the card says so, and not that the sync failed.
    expect(await screen.findByText('Checking your provider for new activities…')).toBeInTheDocument();
    expect(screen.queryByTestId('home-sync-failed')).toBeNull();
  });

  it('keeps the rows on the page under a failed sync', async () => {
    api.getRecentActivities.mockResolvedValue(
      recentResponse({
        sync_failure: {
          provider: 'strava',
          provider_name: 'Strava',
          failed_at: '2026-09-29T14:04:00Z',
          last_synced_at: '2026-09-28T21:15:00Z',
        },
      }),
    );
    renderSection();

    await screen.findByTestId('home-sync-failed');
    expect(screen.getByTestId('home-activity-latest')).toBeInTheDocument();
    expect(screen.getAllByTestId('home-activity-row')).toHaveLength(4);
  });

  it('says nothing about a failed sync when the last attempt succeeded', async () => {
    api.getRecentActivities.mockResolvedValue(recentResponse({ sync_failure: null }));
    const { settled } = renderSection();

    expect(await screen.findByText(/^Last synced: /)).toBeInTheDocument();
    await settled();
    expect(screen.queryByTestId('home-sync-failed')).toBeNull();
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
    const { settled } = renderSection();

    expect(await screen.findByText('No activities yet. They show up here once your provider syncs.')).toBeInTheDocument();
    await settled();
    expect(screen.queryByTestId('home-connect-provider')).toBeNull();
  });

  it('keeps the cached rows for a connection to reconnect and leaves naming it to the shell banner', async () => {
    providers(providerStatus({ provider: 'strava', display_name: 'Strava' }), toReconnect('garmin', 'Garmin'));
    api.getRecentActivities.mockResolvedValue(recentResponse());
    const { settled } = renderSection();

    // What the cache holds is still shown, under the sync line.
    await screen.findByTestId('home-activity-latest');
    expect(screen.getAllByTestId('home-activity-row')).toHaveLength(4);
    expect(screen.getByText(/^Last synced: /)).toBeInTheDocument();
    await settled();
    // The app shell's reconnect banner names Garmin above every tab; the card
    // does not say it a second time.
    expect(screen.queryByText(/Reconnect Garmin/)).toBeNull();
    expect(screen.queryByRole('button', { name: 'Reconnect' })).toBeNull();
    expect(screen.queryByTestId('home-connect-provider')).toBeNull();
  });

  it('says it is checking a connection to reconnect only when the server answered that a refresh is in flight', async () => {
    providers(toReconnect('garmin', 'Garmin'));
    api.getRecentActivities.mockResolvedValue(recentResponse({ stale: false }));
    const first = renderSection();

    // `stale: false`: the server started nothing, so the card reports when it
    // last synced — not a search that is not happening.
    expect(await screen.findByText(/^Last synced: /)).toBeInTheDocument();
    await first.settled();
    expect(screen.queryByText('Checking your provider for new activities…')).toBeNull();
    expect(screen.queryByText(/Reconnect Garmin/)).toBeNull();
    cleanup();

    // `stale: true`: the server did start a refresh (a flagged scrape session
    // is retried on its own schedule), so saying it is checking is true.
    api.getRecentActivities.mockResolvedValue(recentResponse({ stale: true }));
    renderSection();
    expect(await screen.findByText('Checking your provider for new activities…')).toBeInTheDocument();
    expect(screen.queryByText(/^Last synced: /)).toBeNull();
    expect(screen.queryByText(/Reconnect Garmin/)).toBeNull();
    expect(screen.getAllByTestId('home-activity-row')).toHaveLength(4);
  });

  it('says there are no activities yet when the cache is empty and a connection needs reconnecting', async () => {
    providers(toReconnect('garmin', 'Garmin'));
    api.getRecentActivities.mockResolvedValue(recentResponse({ activities: [], as_of: '2026-09-20T06:00:00Z' }));
    const { settled } = renderSection();

    expect(await screen.findByText('No activities yet. They show up here once your provider syncs.')).toBeInTheDocument();
    await settled();
    expect(screen.queryByTestId('home-connect-provider')).toBeNull();
    expect(screen.queryByTestId('home-activity-latest')).toBeNull();
  });

  it('joins provider names the way the language does', () => {
    expect(formatNameList(['Garmin'], 'en')).toBe('Garmin');
    expect(formatNameList(['Garmin', 'Strava'], 'en')).toBe('Garmin and Strava');
    expect(formatNameList(['Garmin', 'Strava', 'COROS'], 'en')).toBe('Garmin, Strava, and COROS');
    expect(formatNameList(['Garmin', 'Strava'], 'fr')).toBe('Garmin et Strava');
    expect(formatNameList(['Garmin', 'Strava', 'COROS'], 'de')).toBe('Garmin, Strava und COROS');
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
    // The fixture's four earlier rows mix rows that may have a route with one
    // whose route held no GPS, so every row keeps the column and their text
    // stays on one line.
    expect(screen.getAllByTestId('home-sketch-slot')).toHaveLength(4);
  });

  it('drops the sketch column when every earlier row says its route held no GPS', async () => {
    const trackless = [1, 2, 3, 4, 5].map((n) =>
      activity({ id: `indoor-${n}`, has_gps: false, summary_polyline: null, start_date: `2026-09-1${n}T08:00:00Z` }),
    );
    api.getRecentActivities.mockResolvedValue(recentResponse({ activities: trackless }));
    const { settled } = renderSection();

    await screen.findByTestId('home-activity-latest');
    expect(screen.getAllByTestId('home-activity-row')).toHaveLength(4);
    expect(screen.queryByTestId('home-sketch-slot')).toBeNull();
    await settled();
    expect(api.getActivityRoute).not.toHaveBeenCalled();
  });
});
