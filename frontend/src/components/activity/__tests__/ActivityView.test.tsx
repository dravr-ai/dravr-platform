// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The activity view reads one workout — its map, figures and splits — then offers questions that go out in its own chat
// ABOUTME: Pins the figures printed, the questions sent word for word, the typed first question naming the activity, and the honest states

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { act, render, screen, within, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ActivityDetailResponse, ActivityRouteResponse } from '@pierre/shared-types';
import { i18n } from '@pierre/i18n';
import type { PendingComposerAction } from '../../ChatTab';
import { ThemeProvider } from '../../../hooks/useTheme';
import ActivityView from '../ActivityView';
import { DRAFT_DATE, formatInstant } from '../../home/homeFormat';

const api = vi.hoisted(() => ({
  getActivityDetail: vi.fn<(provider: string, id: string) => Promise<ActivityDetailResponse>>(),
  getActivityRoute: vi.fn<(provider: string, id: string) => Promise<ActivityRouteResponse>>(),
  linkActivityConversation: vi.fn<(provider: string, id: string, conversationId: string | null) => Promise<void>>(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: {
    getActivityDetail: api.getActivityDetail,
    getActivityRoute: api.getActivityRoute,
    linkActivityConversation: api.linkActivityConversation,
  },
}));

// The chat surface has its own suite; here it only has to receive, embedded,
// the composer actions the view hands it.
const chat = vi.hoisted(() => ({
  props: vi.fn(),
  // The latest `onSelectConversation`: how the chat reports the thread it opened.
  select: null as ((conversationId: string | null) => void) | null,
}));
vi.mock('../../ChatTab', () => ({
  default: (props: {
    layout?: string;
    selectedConversation: string | null;
    onSelectConversation: (conversationId: string | null) => void;
    pendingComposerAction?: PendingComposerAction | null;
    onPendingComposerActionConsumed?: () => void;
    embeddedEmptyState?: React.ReactNode;
  }) => {
    chat.select = props.onSelectConversation;
    chat.props({
      layout: props.layout,
      selected: props.selectedConversation,
      action: props.pendingComposerAction ?? null,
    });
    if (props.pendingComposerAction) props.onPendingComposerActionConsumed?.();
    return <div data-testid="embedded-chat-stub">{props.selectedConversation ? null : props.embeddedEmptyState}</div>;
  },
}));

const START = '2026-09-29T10:00:00Z';

function detail(overrides: Partial<ActivityDetailResponse> = {}): ActivityDetailResponse {
  return {
    activity: {
      id: 'morning-trail-run',
      provider: 'strava',
      name: 'Morning Trail Run',
      sport_type: 'trail_running',
      start_date: START,
      duration_seconds: 3_480,
      distance_meters: 12_910,
      elevation_gain_meters: 214,
      has_gps: false,
      summary_polyline: null,
      attribution: null,
    },
    average_heart_rate: 152,
    max_heart_rate: 171,
    average_speed_mps: 3.71,
    max_speed_mps: null,
    average_power: null,
    calories: 890,
    splits: [
      {
        index: 1,
        distance_meters: 1_000,
        elapsed_time_seconds: 270,
        moving_time_seconds: 268,
        elevation_difference_meters: 12,
        average_speed_mps: 1000 / 268,
        average_heart_rate: 148,
      },
      {
        index: 2,
        distance_meters: 1_000,
        elapsed_time_seconds: 281,
        moving_time_seconds: null,
        elevation_difference_meters: -5,
        average_speed_mps: 1000 / 281,
        average_heart_rate: 155,
      },
    ],
    laps: [],
    conversation_id: null,
    ...overrides,
  };
}

function renderView(
  queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } }),
) {
  const onBack = vi.fn();
  const view = render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <ActivityView
          activity={{ provider: 'strava', id: 'morning-trail-run' }}
          onBack={onBack}
          onNavigate={vi.fn()}
        />
      </ThemeProvider>
    </QueryClientProvider>,
  );
  return { onBack, queryClient, unmount: view.unmount };
}

/** Every composer action the embedded chat was handed, in order. */
function sentActions(): PendingComposerAction[] {
  return chat.props.mock.calls
    .map(([props]) => props.action as PendingComposerAction | null)
    .filter((action): action is PendingComposerAction => action !== null);
}

describe('ActivityView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows the activity: its name, when, the map note, the figures and the splits', async () => {
    api.getActivityDetail.mockResolvedValue(detail());
    renderView();

    await screen.findByTestId('activity-figures');
    expect(screen.getByTestId('activity-title')).toHaveTextContent('Morning Trail Run');
    expect(api.getActivityDetail).toHaveBeenCalledWith('strava', 'morning-trail-run', expect.anything());
    // has_gps false: the map says there is no track, without asking for it.
    expect(screen.getByText('This activity recorded no GPS track.')).toBeInTheDocument();
    expect(api.getActivityRoute).not.toHaveBeenCalled();

    const figures = screen.getByTestId('activity-figures');
    const figure = (id: string) => within(figures).getByTestId(`activity-figure-${id}`);
    expect(figure('distance')).toHaveTextContent('Distance12.91 km');
    expect(figure('duration')).toHaveTextContent('Duration58m');
    expect(figure('elevation_gain')).toHaveTextContent('Elevation gain214 m');
    expect(figure('average_speed')).toHaveTextContent('Avg pace4:30 /km');
    expect(figure('average_heart_rate')).toHaveTextContent('Avg heart rate152 bpm');
    expect(figure('max_heart_rate')).toHaveTextContent('Max heart rate171 bpm');
    expect(figure('calories')).toHaveTextContent('Calories890 kcal');
    // Not held by the cache, so not printed.
    expect(within(figures).queryByTestId('activity-figure-average_power')).toBeNull();
    expect(within(figures).queryByTestId('activity-figure-max_speed')).toBeNull();

    const splits = screen.getByTestId('activity-splits');
    const rows = within(splits).getAllByRole('row');
    expect(rows).toHaveLength(3);
    expect(rows[1]).toHaveTextContent('11.00 km4:284:28 /km148 bpm+12 m');
    expect(rows[2]).toHaveTextContent('21.00 km4:414:41 /km155 bpm-5 m');
    expect(screen.queryByTestId('activity-laps')).toBeNull();
  });

  it('shows the Garmin attribution a Garmin-recorded activity carries, and none otherwise', async () => {
    api.getActivityDetail.mockResolvedValue(
      detail({ activity: { ...detail().activity, provider: 'intervals_icu', attribution: 'Garmin' } }),
    );
    renderView();
    expect(await screen.findByTestId('activity-attribution')).toHaveTextContent('Garmin');
    expect(screen.getByTestId('activity-when')).toHaveTextContent(/· Garmin$/);
  });

  it('shows no attribution for an activity that carries none', async () => {
    api.getActivityDetail.mockResolvedValue(detail());
    renderView();
    await screen.findByTestId('activity-figures');
    expect(screen.queryByTestId('activity-attribution')).toBeNull();
  });

  // A marathon is forty-two splits. Laid out in full between the map and the
  // chat they pushed the question field far below the fold; they scroll
  // inside a frame of their own instead, in a panel with the figures that
  // reads after the chat.
  it('keeps a marathon of splits in a frame that scrolls inside itself, in the panel after the chat', async () => {
    const splits = Array.from({ length: 42 }, (_, index) => ({
      index: index + 1,
      distance_meters: 1_000,
      elapsed_time_seconds: 300,
      moving_time_seconds: 298,
      elevation_difference_meters: 2,
      average_speed_mps: 1000 / 298,
      average_heart_rate: 150,
    }));
    api.getActivityDetail.mockResolvedValue(detail({ splits }));
    renderView();

    const panel = await screen.findByTestId('activity-details');
    expect(within(panel).getByTestId('activity-figures')).toBeInTheDocument();
    const section = within(panel).getByTestId('activity-splits');
    // The frame is a named region the keyboard can reach and scroll.
    const frame = within(section).getByRole('region', { name: 'Splits' });
    expect(frame).toHaveAttribute('tabindex', '0');
    expect(frame).toHaveClass('overflow-auto', 'max-h-64', 'lg:max-h-none', 'lg:flex-1');
    expect(within(frame).getAllByRole('row')).toHaveLength(43);
    // Its header stays put while the rows move under it.
    for (const header of within(frame).getAllByRole('columnheader')) {
      expect(header).toHaveClass('sticky', 'top-0');
    }

    // The panel leaves the page's scroll on a wide screen, pinned to the
    // view; the chat is not inside it, and reads before it — the
    // question field comes ahead of forty-two rows at every width.
    expect(panel).toHaveClass('lg:absolute', 'lg:inset-y-0', 'lg:right-0', 'lg:w-[420px]', 'lg:overflow-y-auto');
    // It is pinned to the body under the header — never to a header height
    // written a second time — and a table keeps a few rows on a short window.
    const body = screen.getByTestId('activity-body');
    expect(body).toHaveClass('relative');
    expect(body).toContainElement(panel);
    expect(body).not.toContainElement(screen.getByTestId('activity-back'));
    expect(section).toHaveClass('lg:min-h-52', 'lg:flex-1');
    expect(screen.getByTestId('activity-scroll')).toHaveClass('lg:pr-[420px]');
    const chatSection = screen.getByTestId('activity-chat');
    expect(panel).not.toContainElement(chatSection);
    expect(panel.compareDocumentPosition(chatSection) & Node.DOCUMENT_POSITION_PRECEDING).toBeTruthy();
  });

  it('sends a suggested question, word for word, into the embedded chat', async () => {
    api.getActivityDetail.mockResolvedValue(detail());
    renderView();

    await screen.findByTestId('activity-prompts');
    const chips = within(screen.getByTestId('activity-prompts')).getAllByRole('button');
    expect(chips.map((chip) => chip.textContent)).toEqual([
      'Analyze this effort',
      'Compare with my recent ones',
      'Recovery advice',
      'What to adjust',
    ]);
    expect(chat.props).toHaveBeenLastCalledWith({ layout: 'embedded', selected: null, action: null });

    await userEvent.click(screen.getByTestId('activity-prompt-recovery'));

    const date = formatInstant(START, 'en', DRAFT_DATE);
    expect(sentActions()).toEqual([
      { kind: 'send', text: `What recovery do you advise after my activity “Morning Trail Run” from ${date}?` },
    ]);
  });

  it('sends a typed first question in the sentence that names the activity', async () => {
    api.getActivityDetail.mockResolvedValue(detail());
    renderView();

    const field = await screen.findByRole('textbox', { name: 'Ask your own question…' });
    await userEvent.type(field, 'Was my cadence OK on the climbs?{Enter}');

    const date = formatInstant(START, 'en', DRAFT_DATE);
    expect(sentActions()).toEqual([
      {
        kind: 'send',
        text: `About my activity “Morning Trail Run” from ${date}: Was my cadence OK on the climbs?`,
      },
    ]);
    expect(field).toHaveValue('');
  });

  it('opens the thread the server names for the activity in the embedded chat', async () => {
    api.getActivityDetail.mockResolvedValue(detail({ conversation_id: 'conv-42' }));
    renderView();

    await screen.findByTestId('activity-prompts');
    expect(chat.props).toHaveBeenLastCalledWith({ layout: 'embedded', selected: 'conv-42', action: null });
    // The thread is open, so the first-question field is gone.
    expect(screen.queryByTestId('activity-ask')).toBeNull();
  });

  it('links the thread its first question opened on the server, so a fresh load reopens it', async () => {
    // The server's link: what the view links, a later detail read names.
    let linked: string | null = null;
    api.linkActivityConversation.mockImplementation(async (_provider, _id, conversationId) => {
      linked = conversationId;
    });
    api.getActivityDetail.mockImplementation(async () => detail({ conversation_id: linked }));
    const { unmount } = renderView();
    await screen.findByTestId('activity-prompts');
    await userEvent.click(screen.getByTestId('activity-prompt-recovery'));
    // The chat created the thread for that question and reports it.
    act(() => chat.select?.('conv-recovery'));
    await waitFor(() =>
      expect(chat.props).toHaveBeenLastCalledWith({ layout: 'embedded', selected: 'conv-recovery', action: null }),
    );

    await waitFor(() =>
      expect(api.linkActivityConversation).toHaveBeenCalledWith('strava', 'morning-trail-run', 'conv-recovery'),
    );

    // Back to Home, then the same activity again with nothing cached — a
    // reload, another device: the thread comes from the server.
    unmount();
    chat.props.mockClear();
    renderView();
    await screen.findByTestId('activity-prompts');
    expect(chat.props).toHaveBeenLastCalledWith({ layout: 'embedded', selected: 'conv-recovery', action: null });
    expect(screen.queryByTestId('activity-ask')).toBeNull();
  });

  // "58:00" beside a distance reads as hours as easily as minutes, and none
  // of the app's languages writes "58m 30s" but English: the figure is in the
  // athlete's words, while the splits stay a clock column, alike in all five.
  it.each([
    ['en', 'Duration58m 30s', '11.00 km4:28'],
    ['fr', 'Durée58 min 30 s', '11,00 km4:28'],
    ['de', 'Dauer58 Min. 30 Sek.', '11,00 km4:28'],
    ['es', 'Duración58 min 30 s', '11,00 km4:28'],
    ['pt', 'Duração58 min 30 s', '11,00 km4:28'],
  ])('writes the duration in %s and keeps the splits a clock', async (language, duration, firstSplit) => {
    await act(async () => {
      await i18n.changeLanguage(language);
    });
    try {
      api.getActivityDetail.mockResolvedValue(detail({ activity: { ...detail().activity, duration_seconds: 3_510 } }));
      renderView();
      const figures = await screen.findByTestId('activity-figures');
      expect(within(figures).getByTestId('activity-figure-duration')).toHaveTextContent(duration);
      const rows = within(screen.getByTestId('activity-splits')).getAllByRole('row');
      expect(rows[1]).toHaveTextContent(firstSplit);
    } finally {
      await act(async () => {
        await i18n.changeLanguage('en');
      });
    }
  });

  it('works no pace out from the duration alone when the provider sent none', async () => {
    api.getActivityDetail.mockResolvedValue(detail({ average_speed_mps: null, splits: [] }));
    renderView();

    const figures = await screen.findByTestId('activity-figures');
    // What the duration covers is the provider's (elapsed on Strava, the
    // timer time on Garmin), so a pace over it could not say whether it
    // counts the stops: none is printed, and nothing claims "elapsed".
    expect(within(figures).getByTestId('activity-figure-duration')).toHaveTextContent('Duration58m');
    expect(within(figures).queryByTestId('activity-figure-average_speed')).toBeNull();
    expect(within(figures).queryByTestId('activity-figure-moving_time')).toBeNull();
    expect(figures).not.toHaveTextContent(/elapsed/i);
  });

  it('says an activity the athlete does not hold is not among theirs, and leaves on Back', async () => {
    api.getActivityDetail.mockRejectedValue({ response: { status: 404, data: { message: 'not found' } } });
    const { onBack } = renderView();

    expect(await screen.findByTestId('activity-not-found')).toHaveTextContent(
      "This activity isn't among your synced activities.",
    );
    // A 404 is an answer: it is not asked again.
    expect(api.getActivityDetail).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('activity-prompts')).toBeNull();
    await userEvent.click(screen.getByTestId('activity-back'));
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it('offers a retry when the read failed, and shows the activity once it answers', async () => {
    // The read is asked again twice on its own before the view says it failed.
    api.getActivityDetail.mockRejectedValue({ response: { status: 500, data: {} } });
    renderView();

    const failed = await screen.findByTestId('activity-failed', {}, { timeout: 6000 });
    expect(api.getActivityDetail).toHaveBeenCalledTimes(3);
    expect(failed).toHaveTextContent("This activity couldn't be loaded.");
    api.getActivityDetail.mockResolvedValueOnce(detail());
    await userEvent.click(within(failed).getByRole('button', { name: 'Retry' }));

    await waitFor(() => expect(screen.getByTestId('activity-title')).toHaveTextContent('Morning Trail Run'));
  }, 10_000);
});
