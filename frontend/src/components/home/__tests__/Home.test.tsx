// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Home's Today as a whole — Getting started, Your team, the plan half's three answers, the status, the activities
// ABOUTME: Red if Today shows a filled button, a plan that failed to load is offered as "no plan", or Hide is not stored

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { TrainingPlanResponse } from '@pierre/shared-types';
import { ThemeProvider } from '../../../hooks/useTheme';
import { HomeBriefing } from '../Home';
import { homePlan, recentResponse, routeView, TODAY } from './homeFixtures';
import { calendarAnswer } from './calendarFixtures';

const api = vi.hoisted(() => ({
  getTrainingPlan: vi.fn<(locale?: string) => Promise<TrainingPlanResponse>>(),
  getRecentActivities: vi.fn(),
  getActivityRoute: vi.fn(),
  getProvidersStatus: vi.fn(),
  getTrainingStatus: vi.fn(),
  getTrainingVolume: vi.fn(),
  getHomePreferences: vi.fn<() => Promise<Prefs>>(),
  updateHomePreferences: vi.fn<(prefs: Partial<Prefs>) => Promise<Prefs>>(),
  getCalendar: vi.fn(),
  getGettingStarted: vi.fn(),
  getCheckIn: vi.fn(),
  listAgents: vi.fn(),
  getPersonalConversation: vi.fn(),
  openPersonalTopic: vi.fn(),
  getOnboardingStatus: vi.fn(),
}));

type Prefs = { plan_suggestion_hidden: boolean; getting_started_hidden?: boolean };

vi.mock('../../../services/api', () => ({
  athleteApi: {
    getTrainingPlan: api.getTrainingPlan,
    getRecentActivities: api.getRecentActivities,
    getActivityRoute: api.getActivityRoute,
    getTrainingStatus: api.getTrainingStatus,
    getTrainingVolume: api.getTrainingVolume,
    getHomePreferences: api.getHomePreferences,
    updateHomePreferences: api.updateHomePreferences,
    getCalendar: api.getCalendar,
    getCheckIn: api.getCheckIn,
  },
  gettingStartedApi: { get: api.getGettingStarted },
  providersApi: { getProvidersStatus: api.getProvidersStatus },
  coachesApi: { list: api.listAgents },
  chatApi: { getPersonalConversation: api.getPersonalConversation, openPersonalTopic: api.openPersonalTopic },
  userApi: { getOnboardingStatus: api.getOnboardingStatus },
  // The route-marker beta is off: these suites pin the map without its markers.
  featureFlagsApi: { getMyFeatures: async () => ({ flags: {}, known: [] }) },
}));

// The picker is the onboarding step's, pinned by its own suite and by the
// Pick your team sheet's; Today only offers it.
vi.mock('../../onboarding/TeamPicker', () => ({
  TeamPicker: () => null,
}));
vi.mock('../../onboarding/useTeamProposal', () => ({
  useTeamProposal: () => ({ data: undefined, isLoading: true }),
}));

vi.mock('maplibre-gl', () => ({
  Map: class {
    addControl() {}
    on() {}
    setStyle() {}
    remove() {}
  },
  AttributionControl: class {},
  NavigationControl: class {},
  Marker: class {},
  setWorkerUrl: () => {},
}));

function renderHome() {
  const onNavigate = vi.fn();
  const onPickTeam = vi.fn();
  const actions = { draft: vi.fn(), send: vi.fn(), openTopic: vi.fn(), dismiss: vi.fn() };
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const view = render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <HomeBriefing onNavigate={onNavigate} actions={actions} onPickTeam={onPickTeam} />
      </ThemeProvider>
    </QueryClientProvider>,
  );
  return { ...view, onNavigate, onPickTeam, actions, onOpenChatDraft: actions.draft };
}

/** A new athlete's Getting started: connected, meeting the team, nothing else yet, no check-in data. */
function gettingStarted(overrides: Record<string, unknown> = {}) {
  return {
    rows: [
      { id: 'connect', order: 1, eligible: true, done: true, current: false, postback: null },
      { id: 'meet_team', order: 2, eligible: true, done: false, current: true, postback: 'gs:2:meet_team' },
      { id: 'last_workout', order: 3, eligible: true, done: false, current: false, postback: 'gs:3:last_workout' },
      { id: 'about_me_more', order: 4, eligible: true, done: false, current: false, postback: 'gs:4:about_me_more' },
      { id: 'check_in', order: 5, eligible: false, done: false, current: false, postback: 'gs:5:check_in' },
    ],
    done_count: 1,
    total: 5,
    folded_at: null,
    hidden: false,
    ...overrides,
  };
}

const ENDURANCE = {
  id: 'agent-endurance',
  title: 'Endurance',
  handle: 'endurance',
  description: 'Your training, week by week',
  is_assigned: true,
};
const FUELLING = {
  id: 'agent-fuelling',
  title: 'Fuelling',
  handle: 'fuelling',
  description: 'long days, race food',
  is_assigned: true,
};
const RECOVERY = {
  id: 'agent-recovery',
  title: 'Recovery',
  handle: 'recovery',
  description: 'sleep, HRV, load',
  is_assigned: true,
};
const NOT_ON_TEAM = { id: 'agent-mind', title: 'Mind', handle: 'mind', description: null, is_assigned: false };

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  api.getProvidersStatus.mockResolvedValue({
    providers: [{ provider: 'garmin', display_name: 'Garmin', connected: true }],
  });
  api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false, getting_started_hidden: false });
  api.getGettingStarted.mockResolvedValue(gettingStarted());
  api.getCheckIn.mockResolvedValue({ cadence: 'daily', local_hour: 7 });
  api.getOnboardingStatus.mockResolvedValue({ pillars_covered: 2, pillars_total: 7 });
  api.listAgents.mockResolvedValue({ agents: [RECOVERY, NOT_ON_TEAM, FUELLING, ENDURANCE] });
  api.getPersonalConversation.mockResolvedValue({ id: 'conv-home', title: 'Home', agent_id: ENDURANCE.id });
  api.getRecentActivities.mockResolvedValue(recentResponse());
  api.getActivityRoute.mockResolvedValue({ route: routeView(), reason: null });
  api.getCalendar.mockImplementation(async (from: string, to: string) => calendarAnswer(from, to));
  api.getTrainingStatus.mockResolvedValue({
    today: TODAY,
    form: { band: 'productive', pct_of_fitness: -14 },
    trend: [
      { date: '2026-09-22', band: 'balanced', pct_of_fitness: -6 },
      { date: TODAY, band: 'productive', pct_of_fitness: -14 },
    ],
      load_ratio: { ratio: 1.1, acute_days: 7, chronic_days: 28 },
    recovery_days: 0,
  });
  api.getTrainingVolume.mockResolvedValue({
    today: TODAY,
    weeks: [
      {
        week_start: '2026-09-21',
        sports: [{ sport_type: 'run', activities: 1, distance_meters: 8_000, duration_seconds: 2_400, elevation_gain_meters: 0 }],
      },
    ],
  });
});

describe('Home — Today', () => {
  it('holds Getting started, Your team, the plan for today, the week, the status and the recent activities, in that order', async () => {
    api.getTrainingPlan.mockResolvedValue({ plan: homePlan(), today: TODAY });
    renderHome();

    // carnet#1098: what to do next and who the athlete works with come first.
    const started = await screen.findByTestId('home-getting-started');
    const team = await screen.findByTestId('home-team');
    expect(started.compareDocumentPosition(team)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(team.compareDocumentPosition(await screen.findByTestId('home-today'))).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING,
    );

    // The page header belongs to the conversation; Today is the panel beside it.
    expect(screen.queryByRole('heading', { level: 2, name: 'Home' })).toBeNull();
    expect(await screen.findByTestId('home-today-session')).toHaveTextContent('Tempo run');
    expect(screen.getByTestId('home-week')).toBeInTheDocument();
    expect(await screen.findByTestId('home-status-band')).toHaveTextContent('Productive');
    expect(await screen.findByTestId('home-volume-distance')).toHaveTextContent('8.00 km');
    expect(await screen.findByTestId('home-activity-latest')).toBeInTheDocument();
    // The weekly volume sits between the training status and the activities.
    const volume = screen.getByTestId('home-volume');
    expect(screen.getByTestId('home-status').compareDocumentPosition(volume)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(volume.compareDocumentPosition(screen.getByTestId('home-activity-latest'))).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING,
    );
    expect(screen.getAllByTestId('home-activity-row')).toHaveLength(4);
    expect(api.getTrainingPlan).toHaveBeenCalledWith('en');
    // Called with no limit: the server's default of five is the page's.
    expect(api.getRecentActivities).toHaveBeenCalledWith();
    // With a plan on screen the page has no filled button at all.
    expect(document.querySelectorAll('.btn-primary')).toHaveLength(0);
  });

  // carnet#820: not every athlete wants a plan, so the offer is a quiet link
  // they can set aside, never the page's one filled button.
  it('answers no plan with a quiet "Build my plan" link that drafts the request', async () => {
    api.getTrainingPlan.mockResolvedValue({ plan: null, today: TODAY });
    api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false });
    const { onOpenChatDraft } = renderHome();

    const empty = await screen.findByTestId('home-plan-empty');
    expect(within(empty).getByText('No training plan yet')).toBeInTheDocument();
    expect(
      await within(empty).findByText('Tell your agent about your goal race and it will lay out your season, week by week.'),
    ).toBeInTheDocument();
    expect(document.querySelectorAll('.btn-primary')).toHaveLength(0);
    // The week still shows what was done, without a plan to read (carnet#708).
    expect(screen.getByTestId('home-week')).toBeInTheDocument();

    await userEvent.click(within(empty).getByRole('button', { name: 'Build my plan' }));
    expect(onOpenChatDraft).toHaveBeenCalledExactlyOnceWith('Build me a training plan for my goal race.');
  });

  it('sets the plan suggestion aside on the server, keeping the one plain sentence', async () => {
    api.getTrainingPlan.mockResolvedValue({ plan: null, today: TODAY });
    api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false });
    api.updateHomePreferences.mockResolvedValue({ plan_suggestion_hidden: true });
    renderHome();

    const empty = await screen.findByTestId('home-plan-empty');
    await userEvent.click(await within(empty).findByRole('button', { name: 'Hide the plan suggestion' }));

    expect(api.updateHomePreferences).toHaveBeenCalledWith({ plan_suggestion_hidden: true });
    await waitFor(() => expect(within(empty).queryByRole('button', { name: 'Build my plan' })).toBeNull());
    expect(within(empty).getByText('No training plan yet')).toBeInTheDocument();
  });

  it('offers no plan to an athlete who set the suggestion aside, on load', async () => {
    api.getTrainingPlan.mockResolvedValue({ plan: null, today: TODAY });
    api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: true });
    renderHome();

    const empty = await screen.findByTestId('home-plan-empty');
    await waitFor(() => expect(api.getHomePreferences).toHaveBeenCalled());
    expect(within(empty).getByText('No training plan yet')).toBeInTheDocument();
    expect(within(empty).queryByRole('button', { name: 'Build my plan' })).toBeNull();
    expect(within(empty).queryByRole('button', { name: 'Hide the plan suggestion' })).toBeNull();
  });

  it('says a plan that failed to load failed — never that there is none', async () => {
    api.getTrainingPlan.mockRejectedValueOnce(new Error('503'));
    renderHome();

    const failed = await screen.findByTestId('home-plan-failed');
    expect(failed).toHaveTextContent("Your plan couldn't be loaded.");
    expect(screen.queryByRole('button', { name: 'Build my plan' })).toBeNull();

    api.getTrainingPlan.mockResolvedValueOnce({ plan: homePlan(), today: TODAY });
    await userEvent.click(within(failed).getByRole('button', { name: 'Retry' }));
    expect(await screen.findByTestId('home-today-session')).toBeInTheDocument();
  });

  it('treats a today that is not a calendar day as a load failure rather than guessing a week', async () => {
    api.getTrainingPlan.mockResolvedValue({ plan: homePlan(), today: '2026-02-30' });
    renderHome();

    expect(await screen.findByTestId('home-plan-failed')).toHaveTextContent("Your plan couldn't be loaded.");
    expect(screen.queryByTestId('home-week')).toBeNull();
  });
});

describe('Home — Getting started', () => {
  beforeEach(() => {
    api.getTrainingPlan.mockResolvedValue({ plan: homePlan(), today: TODAY });
  });

  it('lists the five first steps in order: done plain, the current one tinted, open ones as links', async () => {
    renderHome();
    const section = await screen.findByTestId('home-getting-started');
    expect(within(section).getByRole('heading', { level: 3 })).toHaveTextContent('Getting started');
    expect(within(section).getByTestId('home-getting-started-count')).toHaveTextContent('1 of 5');

    const rows = within(section).getAllByRole('listitem');
    expect(rows.map((row) => row.getAttribute('data-testid'))).toEqual([
      'home-getting-started-row-connect',
      'home-getting-started-row-meet_team',
      'home-getting-started-row-last_workout',
      'home-getting-started-row-about_me_more',
      'home-getting-started-row-check_in',
    ]);

    // Done: a check and a spoken "done", never a link.
    const connect = rows[0];
    expect(connect).toHaveAttribute('data-state', 'done');
    expect(connect).toHaveTextContent('Connect your data');
    expect(connect).toHaveTextContent('— done');
    expect(await within(connect).findByText('Garmin connected')).toBeInTheDocument();
    expect(within(connect).queryByRole('button')).toBeNull();

    // Current: the tint and aria-current, its hint naming the lead.
    const meet = rows[1];
    expect(meet).toHaveAttribute('aria-current', 'step');
    expect(within(meet).getByRole('button')).toHaveClass('bg-surface-container-low');
    expect(await within(meet).findByText('Reply to Endurance in your thread')).toBeInTheDocument();

    // Open: links, with their hints.
    expect(within(rows[2]).getByRole('button', { name: /Read your last workout/ })).toBeInTheDocument();
    expect(rows[2]).not.toHaveAttribute('aria-current');
    expect(await within(rows[3]).findByText((_, el) => el?.textContent === '2 of 7 topics')).toBeInTheDocument();

    // A missing prerequisite is the hint, and the row is not a link yet.
    expect(rows[4]).toHaveTextContent('Your daily check-in');
    expect(rows[4]).toHaveTextContent('Starts once your first data arrives');
    expect(within(rows[4]).queryByRole('button')).toBeNull();

    // These are offers: nothing on Today is a filled button.
    expect(document.querySelectorAll('.btn-primary')).toHaveLength(0);
  });

  it('sends a flow’s starter into the thread, and sends an unconnected athlete to Connections', async () => {
    api.getGettingStarted.mockResolvedValue(
      gettingStarted({
        rows: [
          { id: 'connect', order: 1, eligible: true, done: false, current: true, postback: null },
          { id: 'meet_team', order: 2, eligible: true, done: false, current: false, postback: 'gs:2:meet_team' },
          { id: 'last_workout', order: 3, eligible: false, done: false, current: false, postback: 'gs:3:last_workout' },
          { id: 'about_me_more', order: 4, eligible: true, done: false, current: false, postback: 'gs:4:about_me_more' },
          { id: 'check_in', order: 5, eligible: true, done: false, current: false, postback: 'gs:5:check_in' },
        ],
        done_count: 0,
      }),
    );
    const { actions, onNavigate } = renderHome();

    const connect = await screen.findByTestId('home-getting-started-row-connect');
    expect(connect).toHaveTextContent('Your watch or training app');
    await userEvent.click(within(connect).getByRole('button'));
    expect(onNavigate).toHaveBeenCalledWith('settings/connections');

    expect(screen.getByTestId('home-getting-started-row-last_workout')).toHaveTextContent(
      'Waits for your first activity',
    );
    const checkIn = screen.getByTestId('home-getting-started-row-check_in');
    // The hour is the server's, never a client copy of it.
    expect(await within(checkIn).findByText(/^Every morning at 7/)).toBeInTheDocument();

    await userEvent.click(within(screen.getByTestId('home-getting-started-row-meet_team')).getByRole('button'));
    expect(actions.send).toHaveBeenCalledExactlyOnceWith('gs:2:meet_team', 'Meet your team');
    expect(actions.draft).not.toHaveBeenCalled();
  });

  it('hides on the server, keeping the plan choice, and leaves the page at once', async () => {
    api.updateHomePreferences.mockImplementation(() => new Promise(() => {}));
    renderHome();

    const section = await screen.findByTestId('home-getting-started');
    await userEvent.click(within(section).getByRole('button', { name: 'Hide Getting started' }));

    expect(api.updateHomePreferences).toHaveBeenCalledExactlyOnceWith({ getting_started_hidden: true });
    await waitFor(() => expect(screen.queryByTestId('home-getting-started')).toBeNull());
    // Your team stays.
    expect(screen.getByTestId('home-team')).toBeInTheDocument();
  });

  it('shows nothing to an athlete who hid it, on load', async () => {
    api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false, getting_started_hidden: true });
    renderHome();

    expect(await screen.findByTestId('home-team')).toBeInTheDocument();
    await waitFor(() => expect(api.getGettingStarted).toHaveBeenCalled());
    expect(screen.queryByTestId('home-getting-started')).toBeNull();
  });

  it('folds to one line naming the way back once every flow is done, and is gone a week later', async () => {
    const folded = new Date(Date.now() - 2 * 24 * 60 * 60 * 1000).toISOString();
    api.getGettingStarted.mockResolvedValue(gettingStarted({ done_count: 5, folded_at: folded }));
    const first = renderHome();

    expect(await screen.findByTestId('home-getting-started-folded')).toHaveTextContent(
      "You're set up. Settings › Getting started brings it back.",
    );
    expect(screen.queryByTestId('home-getting-started')).toBeNull();
    first.unmount();

    const old = new Date(Date.now() - 8 * 24 * 60 * 60 * 1000).toISOString();
    api.getGettingStarted.mockResolvedValue(gettingStarted({ done_count: 5, folded_at: old }));
    renderHome();
    expect(await screen.findByTestId('home-team')).toBeInTheDocument();
    await waitFor(() => expect(api.getGettingStarted).toHaveBeenCalledTimes(2));
    expect(screen.queryByTestId('home-getting-started-folded')).toBeNull();
    expect(screen.queryByTestId('home-getting-started')).toBeNull();
  });
});

describe('Home — Your team', () => {
  beforeEach(() => {
    api.getTrainingPlan.mockResolvedValue({ plan: homePlan(), today: TODAY });
  });

  it('leads with the thread’s agent, then the team by title with @handle, never an agent off the team', async () => {
    renderHome();
    const team = await screen.findByTestId('home-team');
    expect(within(team).getByRole('heading', { level: 3 })).toHaveTextContent('Your team');

    const lead = await within(team).findByTestId('home-team-lead');
    expect(lead).toHaveTextContent('Endurance');
    expect(lead).toHaveTextContent('Leads your thread');
    const members = within(team).getAllByTestId('home-team-member');
    expect(members.map((row) => row.textContent)).toEqual([
      expect.stringContaining('@fuelling · long days, race food'),
      expect.stringContaining('@recovery · sleep, HRV, load'),
    ]);
    expect(team).not.toHaveTextContent('Mind');
  });

  it('takes Find more and Add an agent to Discover', async () => {
    const { onNavigate } = renderHome();
    const team = await screen.findByTestId('home-team');

    await userEvent.click(within(team).getByRole('button', { name: 'Find more' }));
    await userEvent.click(within(team).getByRole('button', { name: 'Add an agent' }));
    expect(onNavigate.mock.calls).toEqual([['discover'], ['discover']]);
  });

  it('opens a topic with the agent a row names', async () => {
    api.openPersonalTopic.mockResolvedValue({
      conversation: { id: 'conv-home', title: 'Home' },
      divider: { id: 'divider-7' },
    });
    const { actions } = renderHome();

    await userEvent.click(await screen.findByRole('button', { name: 'Talk to Fuelling' }));
    expect(api.openPersonalTopic).toHaveBeenCalledExactlyOnceWith({ agent_id: FUELLING.id });
    await waitFor(() => expect(actions.openTopic).toHaveBeenCalledExactlyOnceWith('conv-home', 'divider-7'));
  });

  it('offers Pick your team to an athlete with no team, and marks no lead it does not have', async () => {
    api.listAgents.mockResolvedValue({ agents: [NOT_ON_TEAM] });
    const { onPickTeam } = renderHome();

    const empty = await screen.findByTestId('home-team-empty');
    expect(empty).toHaveTextContent('No team yet.');
    expect(screen.queryByTestId('home-team-lead')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Add an agent' })).toBeNull();

    await userEvent.click(within(empty).getByRole('button', { name: 'Pick your team' }));
    expect(onPickTeam).toHaveBeenCalledTimes(1);
  });
});
