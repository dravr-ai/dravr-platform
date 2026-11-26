// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach Home's three views — À traiter, the team table and the season — and the actions that leave them
// ABOUTME: Pins the queue's order, the dismiss and undo round trip, the non-sharer's status-only rows, the table's sort, where each action goes, and the morning brief

import { useState } from 'react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import type { CoachBrief, CoachBriefUpdate } from '@pierre/shared-types';
import { applyBriefUpdate, civilDayNumber, civilFromDayNumber, civilShortDate, localToday } from '@pierre/ui-logic';
import { ToastProvider } from '../../ui';
import CoachHome from '../CoachHome';
import { CoachHomeSwitch } from '../CoachHomeSwitch';
import { coachHomeRoute, parseCoachHomeRoute, type CoachHomeRoute } from '../coachHomeRoute';
import { buildBrief, buildRoster } from '../../../test/coachRosterFixtures';

const getRoster = vi.fn();
const dismissAttention = vi.fn();
const restoreAttention = vi.fn();
const getBrief = vi.fn();
const updateBrief = vi.fn();
const markBriefOpened = vi.fn();
const listReplies = vi.fn();
const getConversations = vi.fn();
const openAthleteThread = vi.fn();
const openGroupRoom = vi.fn();
const listInvites = vi.fn();

vi.mock('../../../services/api', () => ({
  coachRosterApi: {
    getRoster: (...a: unknown[]) => getRoster(...a),
    dismissAttention: (...a: unknown[]) => dismissAttention(...a),
    restoreAttention: (...a: unknown[]) => restoreAttention(...a),
    getBrief: (...a: unknown[]) => getBrief(...a),
    updateBrief: (...a: unknown[]) => updateBrief(...a),
    markBriefOpened: (...a: unknown[]) => markBriefOpened(...a),
  },
  coachRepliesApi: { list: (...a: unknown[]) => listReplies(...a) },
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    openAthleteThread: (...a: unknown[]) => openAthleteThread(...a),
    openGroupRoom: (...a: unknown[]) => openGroupRoom(...a),
  },
  groupsApi: { listInvites: (...a: unknown[]) => listInvites(...a), getGroup: vi.fn().mockRejectedValue(new Error('not mocked')) },
  // The athlete workspace's threads load with it; nothing here reads them.
  coachesApi: { list: vi.fn().mockResolvedValue({ agents: [] }) },
  providersApi: { getProvidersStatus: vi.fn().mockResolvedValue({ providers: [] }) },
}));

// The athlete workspace reads who the coach is to know they own the plan thread.
vi.mock('../../../hooks/useAuth', () => ({ useAuth: () => ({ user: { id: 'user-coach' } }) }));

const mobile = vi.hoisted(() => ({ value: false }));
vi.mock('../../../hooks/useBreakpoint', () => ({
  useIsMobile: () => mobile.value,
  useIsTablet: () => false,
  useIsDesktop: () => !mobile.value,
}));

const TODAY = localToday();

/** What the mocked server holds for the brief: a read answers it, a change moves it. */
let storedBrief: CoachBrief = buildBrief(TODAY);

function renderHome(initial: CoachHomeRoute = { view: 'queue', athleteId: null }, trains = true) {
  const navigate = vi.fn();
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  /** A host that follows the hash the way the dashboard does: a navigation is the new route. */
  function Host() {
    const [route, setRoute] = useState<CoachHomeRoute>(initial);
    return (
      <CoachHome
        route={route as Exclude<CoachHomeRoute, { view: 'me' }>}
        trains={trains}
        onNavigate={(next) => {
          navigate(next);
          const sub = next === 'home' ? '' : next.startsWith('home/') ? next.slice('home/'.length) : null;
          const parsed = sub === null ? null : parseCoachHomeRoute(sub);
          if (parsed !== null && parsed.view !== 'me') setRoute(parsed);
        }}
      />
    );
  }
  render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <Host />
      </ToastProvider>
    </QueryClientProvider>,
  );
  return { navigate };
}

beforeEach(() => {
  mobile.value = false;
  getRoster.mockResolvedValue(buildRoster(TODAY));
  dismissAttention.mockImplementation(async (request: { reason_id: string; fingerprint: string }) => ({
    reason_id: request.reason_id,
    fingerprint: request.fingerprint,
    dismissed_at: new Date().toISOString(),
  }));
  restoreAttention.mockImplementation(async (request: { reason_id: string }) => ({ reason_id: request.reason_id, restored: true }));
  storedBrief = buildBrief(TODAY);
  getBrief.mockImplementation(async () => storedBrief);
  updateBrief.mockImplementation(async (update: CoachBriefUpdate) => {
    storedBrief = applyBriefUpdate(storedBrief, update);
    return storedBrief;
  });
  markBriefOpened.mockResolvedValue(undefined);
  listReplies.mockResolvedValue({ groups: [], athletes: [], replies: [], next_before: null });
  getConversations.mockResolvedValue({
    conversations: [{ id: 'room-tri', title: 'Tri', group_id: 'g-tri', created_at: '2026-10-01T00:00:00Z', updated_at: '2026-10-29T00:00:00Z', message_count: 3 }],
    total: 1,
    limit: 50,
    offset: 0,
  });
  openAthleteThread.mockResolvedValue({ id: 'thread-1' });
  openGroupRoom.mockResolvedValue({ id: 'room-tri', group_id: 'g-tri' });
});

afterEach(() => {
  vi.clearAllMocks();
});

describe('À traiter', () => {
  it('lists today then this week by the server rank, with the calm ones as one line of names', async () => {
    renderHome();
    const today = await screen.findByTestId('coach-queue-today');
    const names = within(today)
      .getAllByRole('button')
      .map((b) => b.getAttribute('data-testid'));
    // consent revoked (rank 10), nothing received (20), unverified figure (30), deep fatigue (40)
    expect(names).toEqual([
      'coach-queue-row-u-maude',
      'coach-queue-row-u-julien',
      'coach-queue-row-u-felix',
      'coach-queue-row-u-lea',
    ]);
    const week = screen.getByTestId('coach-queue-week');
    // missed planned sessions (290) first of the week, then draft (300), new member (310), plan ended (320)
    expect(within(week).getAllByRole('button').map((b) => b.getAttribute('data-testid'))).toEqual([
      'coach-queue-row-u-kevin',
      'coach-queue-row-u-mathieu',
      'coach-queue-row-u-camille',
      'coach-queue-row-u-olivier',
    ]);
    expect(screen.getByTestId('coach-queue-count')).toHaveTextContent('8');
    const calm = screen.getByTestId('coach-queue-calm');
    // Kevin missed planned sessions: he stands in the week, not among the calm.
    expect(within(calm).queryByTestId('coach-calm-u-kevin')).not.toBeInTheDocument();
    expect(within(calm).getByTestId('coach-calm-u-rosalie')).toBeInTheDocument();
    expect(within(calm).getByTestId('coach-not-sharing-u-hugo')).toBeInTheDocument();
  });

  it('states the scope of what it shows', async () => {
    renderHome();
    expect(await screen.findByTestId('coach-scope')).toHaveTextContent(/8.+10/);
  });

  it('opens on the first of the queue, with each evidence line carrying its source', async () => {
    renderHome();
    const pane = await screen.findByTestId('coach-athlete-pane');
    expect(within(pane).getByRole('heading', { level: 2 })).toHaveTextContent('Maude Poirier');
    expect(within(pane).getAllByTestId('coach-evidence')).toHaveLength(1);
    expect(within(pane).getByTestId('coach-evidence-source')).toBeInTheDocument();
  });

  it('says why in a full sentence naming the athlete, with the short label and evidence under it', async () => {
    const user = userEvent.setup();
    renderHome();
    const maude = within(await screen.findByTestId('coach-athlete-pane'));
    expect(maude.getByTestId('coach-reason-title')).toHaveTextContent('Maude Poirier stopped sharing their data with you.');
    expect(maude.getByTestId('coach-reason-label')).toHaveTextContent('No longer shares their data with you');

    await user.click(screen.getByTestId('coach-queue-row-u-lea'));
    const block = within(await screen.findByTestId('coach-reason-deep_fatigue_streak'));
    expect(block.getByTestId('coach-reason-title')).toHaveTextContent('Léa Gagnon is in deep fatigue for the 3rd morning in a row.');
    expect(block.getByTestId('coach-reason-label')).toHaveTextContent('Deep fatigue for 3 mornings in a row');
    expect(block.getAllByTestId('coach-evidence').length).toBeGreaterThan(0);
    // The queue row keeps the short label: the sentence is the detail's.
    expect(within(screen.getByTestId('coach-queue-row-u-lea')).getByTestId('coach-queue-reason')).toHaveTextContent(
      'Deep fatigue for 3 mornings in a row',
    );
  });

  it('says which planned days went unmatched, sport by sport, with the athlete note printed as their words', async () => {
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-kevin'));
    const block = within(await screen.findByTestId('coach-reason-planned_sessions_missed'));
    expect(block.getByTestId('coach-reason-title')).toHaveTextContent(
      'Kevin Ouellet has no activity matching the sessions planned on their last 2 planned days.',
    );
    expect(block.getByTestId('coach-reason-label')).toHaveTextContent('No matching activity on 2 planned days in a row');
    const lines = block.getAllByTestId('coach-evidence');
    expect(lines).toHaveLength(4);
    const day = (offset: number) => civilShortDate(civilFromDayNumber(civilDayNumber(TODAY) + offset), 'en');
    expect(lines[0]).toHaveTextContent(`3 of the 6 sessions planned since ${day(-14)} have no matching activity`);
    expect(within(lines[0]).getByTestId('coach-evidence-source')).toHaveTextContent(/^Intervals\.icu · Their plan · /);
    expect(lines[1]).toHaveTextContent(new RegExp(`^Planned on ${day(-3)}: .+, no matching activity`));
    expect(lines[1]).not.toHaveTextContent(/: ride,/);
    expect(lines[3]).toHaveTextContent(`Last wellness note (${day(-1)}): “Genou sensible depuis la sortie de dimanche”`);
    expect(within(lines[3]).getByTestId('coach-evidence-source')).toHaveTextContent(/^Wellness · /);
    expect(within(screen.getByTestId('coach-queue-row-u-kevin')).getByTestId('coach-queue-reason')).toHaveTextContent(
      'No matching activity on 2 planned days in a row',
    );
  });

  it('prints a wellness note that looks like markup as the text the athlete typed, never as markup', async () => {
    const roster = buildRoster(TODAY);
    const note = '<img src=x onerror="alert(1)"> {{name}} <b>sore</b>';
    const kevin = roster.athletes.find((a) => a.user_id === 'u-kevin');
    const missed = kevin?.attention[0];
    if (kevin === undefined || missed === undefined) throw new Error('the fixture lost u-kevin');
    kevin.attention = [
      { ...missed, evidence: missed.evidence.map((line) => (line.key === 'coachHome.evidence.wellnessNote' ? { ...line, params: { ...line.params, note } } : line)) },
    ];
    getRoster.mockResolvedValue(roster);
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-kevin'));
    const block = await screen.findByTestId('coach-reason-planned_sessions_missed');
    const lines = within(block).getAllByTestId('coach-evidence');
    expect(lines[3]).toHaveTextContent(`“${note}”`);
    expect(block.querySelector('img')).toBeNull();
    expect(block.querySelector('b')).toBeNull();
  });

  it('shows form as the server band on a six-step strip, and nothing for a non-sharer', async () => {
    const user = userEvent.setup();
    renderHome();
    await screen.findByTestId('coach-athlete-pane');
    // Maude stopped sharing: status only, no strip, no plan, no sources.
    expect(screen.getByTestId('coach-hidden-note')).toBeInTheDocument();
    expect(screen.queryByTestId('form-strip')).not.toBeInTheDocument();
    expect(screen.queryByTestId('coach-plan-block')).not.toBeInTheDocument();

    await user.click(screen.getByTestId('coach-queue-row-u-lea'));
    const strip = await screen.findByTestId('form-strip');
    expect(strip).toHaveAttribute('data-band', 'deep_fatigue');
    expect(strip.children).toHaveLength(6);
    expect(screen.getByTestId('coach-form-word')).toBeInTheDocument();
  });

  it('says a different thing to an athlete who never shared than to one who stopped', async () => {
    const user = userEvent.setup();
    renderHome();
    await screen.findByTestId('coach-athlete-pane');
    const revoked = screen.getByTestId('coach-hidden-note').textContent;
    await user.click(screen.getByTestId('coach-not-sharing-u-hugo'));
    const never = (await screen.findByTestId('coach-hidden-note')).textContent;
    expect(never).not.toBe(revoked);
    expect(screen.queryByTestId('form-strip')).not.toBeInTheDocument();
  });

  it('gives insufficient history its own state rather than a step', async () => {
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-camille'));
    const strip = await screen.findByTestId('form-strip');
    expect(strip).toHaveAttribute('data-band', 'insufficient_history');
    expect(strip.getAttribute('aria-label')).not.toMatch(/\b[1-6] (sur|of) 6\b/);
  });

  it('marks the plan with the phases and a today line', async () => {
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-lea'));
    const plan = await screen.findByTestId('coach-plan-block');
    expect(within(plan).getByTestId('coach-today-line')).toBeInTheDocument();
  });

  it('draws a plan that ran out as one Transition band and the day nothing is planned after', async () => {
    const roster = buildRoster(TODAY);
    const lea = roster.athletes.find((a) => a.user_id === 'u-lea');
    if (!lea?.plan) throw new Error('the fixture has no plan for Léa');
    // Her plan's last phase ended yesterday (its `end` is the day after its last day).
    const lastDay = civilFromDayNumber(civilDayNumber(TODAY) - 2);
    const ends = civilFromDayNumber(civilDayNumber(TODAY) - 1);
    lea.plan = {
      ...lea.plan,
      phase: null,
      ends,
      phases: [
        { kind: 'base', start: civilFromDayNumber(civilDayNumber(TODAY) - 60), end: civilFromDayNumber(civilDayNumber(TODAY) - 30), weeks: 4 },
        { kind: 'build', start: civilFromDayNumber(civilDayNumber(TODAY) - 30), end: ends, weeks: 4 },
      ],
    };
    lea.season = { state: 'plan_ended', race: null, ends };
    getRoster.mockResolvedValue(roster);
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-lea'));

    const plan = await screen.findByTestId('coach-plan-block');
    const bands = within(within(plan).getByTestId('coach-plan-ended')).getAllByTitle('Transition');
    expect(bands).toHaveLength(1);
    expect(within(plan).queryByTitle('Base')).not.toBeInTheDocument();
    expect(within(plan).queryByTitle('Build')).not.toBeInTheDocument();
    expect(within(plan).queryByText('No A race')).not.toBeInTheDocument();
    expect(within(plan).queryByTestId('coach-today-line')).not.toBeInTheDocument();
    expect(plan).toHaveTextContent(`Nothing planned after ${civilShortDate(lastDay, 'en')}`);
  });

  it('marks a source to reconnect', async () => {
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-julien'));
    const garmin = await screen.findByTestId('coach-source-garmin');
    expect(garmin).toHaveTextContent('Garmin');
  });

  it('reads the latest agent reply with its verdict chip', async () => {
    listReplies.mockResolvedValue({
      groups: [],
      athletes: [],
      replies: [
        {
          id: 'r-1',
          group_id: 'g-tri',
          group_name: 'Tri',
          athlete_user_id: 'u-felix',
          athlete_name: 'Félix Morin',
          content: 'Ton seuil est à 285 W.',
          withheld: false,
          created_at: new Date().toISOString(),
          verdicts: [],
          unmatched_figures: 1,
          flags: [],
          corrections: [],
        },
      ],
      next_before: null,
    });
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-felix'));
    expect(await screen.findByTestId('coach-agent-reply')).toHaveTextContent('285 W');
    expect(listReplies).toHaveBeenCalledWith({ athlete_id: 'u-felix', limit: 1 });
    expect(screen.getByTestId('coach-agent-verdict')).toBeInTheDocument();
  });
});

describe('Traité', () => {
  it('sends the reason id and fingerprint, moves the athlete to treated, and undoes', async () => {
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-julien'));
    await user.click(await screen.findByTestId('coach-treat'));
    await waitFor(() =>
      expect(dismissAttention).toHaveBeenCalledWith({
        athlete_user_id: 'u-julien',
        reason_id: 'nothing_received',
        fingerprint: 'fp:nothing_received',
      }),
    );
    // Out of the queue at once, into the treated block, and the pane says so.
    await waitFor(() => expect(screen.queryByTestId('coach-queue-row-u-julien')).not.toBeInTheDocument());
    expect(screen.getByTestId('coach-treated-row-u-julien')).toBeInTheDocument();
    expect(screen.getByTestId('coach-queue-count')).toHaveTextContent('7');
    expect(await screen.findByTestId('coach-treated-note')).toBeInTheDocument();

    await user.click(screen.getByTestId('coach-treated-undo-u-julien'));
    await waitFor(() =>
      expect(restoreAttention).toHaveBeenCalledWith({ athlete_user_id: 'u-julien', reason_id: 'nothing_received' }),
    );
    expect(await screen.findByTestId('coach-queue-row-u-julien')).toBeInTheDocument();
    expect(screen.queryByTestId('coach-treated-row-u-julien')).not.toBeInTheDocument();
  });

  it('puts the athlete back in the queue when the server refuses', async () => {
    dismissAttention.mockRejectedValue(new Error('nope'));
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-julien'));
    await user.click(await screen.findByTestId('coach-treat'));
    await waitFor(() => expect(dismissAttention).toHaveBeenCalled());
    expect(await screen.findByTestId('coach-queue-row-u-julien')).toBeInTheDocument();
    expect(screen.queryByTestId('coach-treated-row-u-julien')).not.toBeInTheDocument();
  });
});

describe('actions', () => {
  it('opens the athlete\'s plan thread in the reason\'s group', async () => {
    const user = userEvent.setup();
    const { navigate } = renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-lea'));
    await user.click(await screen.findByTestId('coach-primary-action'));
    await waitFor(() => expect(openAthleteThread).toHaveBeenCalledWith('u-lea', 'g-tri'));
    await waitFor(() => expect(navigate).toHaveBeenCalledWith('chat/thread-1'));
  });

  it('opens the group room for "Écrire dans le groupe"', async () => {
    const user = userEvent.setup();
    const { navigate } = renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-lea'));
    await user.click(await screen.findByTestId('coach-write-group'));
    expect(openGroupRoom).toHaveBeenCalledWith('g-tri');
    await waitFor(() => expect(navigate).toHaveBeenCalledWith('chat/room-tri'));
  });

  it('opens one room however often the group action is clicked while it opens, and says it is opening', async () => {
    let resolve: (room: unknown) => void = () => undefined;
    openGroupRoom.mockReturnValue(new Promise((r) => (resolve = r)));
    const user = userEvent.setup();
    const { navigate } = renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-lea'));
    const button = await screen.findByTestId('coach-write-group');
    await user.click(button);
    await user.click(button);
    expect(button).toHaveTextContent('Opening the group…');
    expect(button).toBeDisabled();
    expect(openGroupRoom).toHaveBeenCalledTimes(1);
    resolve({ id: 'room-tri', group_id: 'g-tri' });
    await waitFor(() => expect(navigate).toHaveBeenCalledWith('chat/room-tri'));
    expect(navigate.mock.calls.filter(([route]) => route === 'chat/room-tri')).toHaveLength(1);
  });

  it('labels the write action on a non-sharer "Write in the group", never "Message them"', async () => {
    renderHome();
    await screen.findByTestId('coach-athlete-pane');
    expect(screen.getByTestId('coach-primary-action')).toHaveTextContent('Write in the group');
    expect(screen.queryByText('Message them')).not.toBeInTheDocument();
  });

  it('does not repeat the group room as a second button when it is already the primary action', async () => {
    renderHome();
    // Maude's primary action is "write", which goes to the group room.
    await screen.findByTestId('coach-athlete-pane');
    expect(screen.getByTestId('coach-primary-action')).toBeInTheDocument();
    expect(screen.queryByTestId('coach-write-group')).not.toBeInTheDocument();
  });

  it('opens the athlete\'s workspace from « Ouvrir le plan », and draws it on its route (carnet#1816)', async () => {
    const user = userEvent.setup();
    const { navigate } = renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-lea'));
    const open = await screen.findByTestId('coach-open-workspace');
    expect(open).toHaveTextContent(i18n.t('plan.workspace.open'));
    await user.click(open);
    expect(navigate).toHaveBeenCalledWith('home/athlete/u-lea/plan');
    expect(await screen.findByTestId('athlete-workspace')).toBeInTheDocument();
    expect(screen.queryByTestId('coach-home')).not.toBeInTheDocument();
  });

  it('offers no workspace on an athlete who stopped sharing with the coach', async () => {
    renderHome();
    // Maude leads the queue: she stopped sharing.
    await screen.findByTestId('coach-athlete-pane');
    expect(screen.getByRole('heading', { level: 2 })).toHaveTextContent('Maude Poirier');
    expect(screen.queryByTestId('coach-open-workspace')).not.toBeInTheDocument();
  });

  it('opens À traiter on an athlete the roster does not hold, for a workspace route', async () => {
    renderHome({ view: 'workspace', athleteId: 'u-nobody' });
    expect(await screen.findByTestId('coach-queue')).toBeInTheDocument();
    expect(screen.queryByTestId('athlete-workspace')).not.toBeInTheDocument();
  });

  it('opens the reply review on the athlete for "Corriger la réponse"', async () => {
    const user = userEvent.setup();
    const { navigate } = renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-felix'));
    await user.click(await screen.findByTestId('coach-primary-action'));
    expect(navigate).toHaveBeenCalledWith('coach-replies/u-felix');
  });
});

describe('Tout voir', () => {
  it('keeps a source cell that fits in its one order, a source to reconnect included', async () => {
    const roster = buildRoster(TODAY);
    const lea = roster.athletes.find((athlete) => athlete.user_id === 'u-lea');
    if (lea?.training === undefined) throw new Error('the fixture lost u-lea');
    lea.training.last_activity_per_source = { garmin: TODAY, strava: TODAY };
    lea.training.needs_reconnect = ['strava'];
    getRoster.mockResolvedValue(roster);
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-see-all'));
    const row = await screen.findByTestId('coach-row-u-lea');
    const chips = within(within(row).getAllByRole('cell')[3]).getAllByRole('img');
    expect(chips.map((chip) => chip.getAttribute('title'))).toEqual(['Garmin', 'Strava']);
  });

  it('sorts the table by days without activity and never by form, with the non-sharers apart', async () => {
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-see-all'));
    const table = await screen.findByTestId('coach-roster');
    const rows = within(table)
      .getAllByRole('row')
      .map((r) => r.getAttribute('data-testid'))
      .filter((id): id is string => id !== null && id.startsWith('coach-row-'));
    // julien 6 days, olivier 3, camille 2, then 1 day: lea, rosalie (name), then 0: felix, kevin, mathieu
    expect(rows).toEqual([
      'coach-row-u-julien',
      'coach-row-u-olivier',
      'coach-row-u-camille',
      'coach-row-u-lea',
      'coach-row-u-rosalie',
      'coach-row-u-felix',
      'coach-row-u-kevin',
      'coach-row-u-mathieu',
    ]);
    expect(within(screen.getByTestId('coach-row-u-lea')).getByTestId('coach-row-ctl')).toHaveTextContent('92');
    expect(within(screen.getByTestId('coach-row-u-camille')).getByTestId('coach-row-ctl')).toHaveTextContent('—');
    const apart = screen.getByTestId('coach-roster-non-sharers');
    expect(within(apart).getByTestId('coach-nonsharer-u-maude')).toBeInTheDocument();
    expect(within(apart).getByTestId('coach-nonsharer-u-hugo')).toBeInTheDocument();
    // Status only: no strip and no number in a non-sharer's row.
    expect(within(apart).queryByTestId('form-strip')).not.toBeInTheDocument();
    expect(within(apart).queryByTestId('coach-row-ctl')).not.toBeInTheDocument();
  });

  it('links back to À traiter, filters by group and by name, and opens an athlete in À traiter', async () => {
    const user = userEvent.setup();
    const { navigate } = renderHome({ view: 'team' });
    await screen.findByTestId('coach-roster');
    await user.click(screen.getByTestId('coach-group-tab-g-run'));
    expect(screen.queryByTestId('coach-row-u-julien')).not.toBeInTheDocument();
    expect(screen.getByTestId('coach-row-u-camille')).toBeInTheDocument();
    await user.click(screen.getByTestId('coach-group-tab-all'));
    await user.type(screen.getByTestId('coach-roster-search'), 'kev');
    expect(screen.getAllByTestId(/^coach-row-u-/).filter((el) => el.getAttribute('role') === 'row')).toHaveLength(1);

    await user.click(screen.getByTestId('coach-row-open-u-kevin'));
    expect(navigate).toHaveBeenCalledWith('home/athlete/u-kevin');
    expect(await screen.findByTestId('coach-queue')).toBeInTheDocument();
  });

  it('goes back to À traiter', async () => {
    const user = userEvent.setup();
    const { navigate } = renderHome({ view: 'team' });
    await user.click(await screen.findByTestId('coach-roster-back'));
    expect(navigate).toHaveBeenCalledWith('home');
  });

  it('draws a two-line list on a phone and navigates from it', async () => {
    mobile.value = true;
    const user = userEvent.setup();
    const { navigate } = renderHome({ view: 'team' });
    const list = await screen.findByTestId('coach-roster-list');
    expect(within(list).getAllByRole('button')).toHaveLength(8);
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
    await user.click(within(list).getByTestId('coach-row-u-julien'));
    expect(navigate).toHaveBeenCalledWith('home/athlete/u-julien');
  });
});

describe('La saison', () => {
  it('lists the A races ahead by race date, then those without one with the state', async () => {
    renderHome({ view: 'season' });
    const ahead = await screen.findByTestId('coach-season-ahead');
    // felix and julien race in 25 days (name order), lea in 39
    expect(within(ahead).getAllByRole('row').map((r) => r.getAttribute('data-testid'))).toEqual([
      'coach-season-row-u-felix',
      'coach-season-row-u-julien',
      'coach-season-row-u-lea',
    ]);
    const without = screen.getByTestId('coach-season-without');
    expect(within(without).getAllByRole('row').map((r) => r.getAttribute('data-testid'))).toEqual([
      'coach-season-row-u-kevin',
      'coach-season-row-u-mathieu',
      'coach-season-row-u-olivier',
      'coach-season-row-u-camille',
      'coach-season-row-u-rosalie',
    ]);
    expect(within(screen.getByTestId('coach-season-row-u-mathieu')).getAllByTestId('coach-season-bar')[0]).toHaveAttribute('data-kind', 'draft');
    expect(within(screen.getByTestId('coach-season-row-u-olivier')).getByTestId('coach-season-race')).toBeInTheDocument();
    expect(screen.getByTestId('coach-season-hidden')).toHaveTextContent(/Hugo Lemieux.*Maude Poirier/);
    // Dates and phases only: no form band and no CTL on this view.
    expect(screen.queryByTestId('form-strip')).not.toBeInTheDocument();
  });

  it('switches the scale between twelve weeks and the season', async () => {
    const user = userEvent.setup();
    renderHome({ view: 'season' });
    await screen.findByTestId('coach-season-ahead');
    expect(screen.getByTestId('coach-season-scale-season')).toHaveAttribute('aria-pressed', 'true');
    await user.click(screen.getByTestId('coach-season-scale-12w'));
    expect(screen.getByTestId('coach-season-scale-12w')).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByTestId('coach-season-scale-season')).toHaveAttribute('aria-pressed', 'false');
  });

  it('draws a countdown list on a phone', async () => {
    mobile.value = true;
    renderHome({ view: 'season' });
    const ahead = await screen.findByTestId('coach-season-ahead');
    expect(within(ahead).getAllByRole('button')).toHaveLength(3);
    expect(screen.queryByTestId('coach-season-scale')).not.toBeInTheDocument();
  });
});

describe('the switch and the routes', () => {
  it('returns to the coach\'s own Home from the switch', async () => {
    const user = userEvent.setup();
    const { navigate } = renderHome();
    await screen.findByTestId('coach-home');
    expect(screen.getByTestId('coach-home-switch-athletes')).toHaveAttribute('aria-pressed', 'true');
    await user.click(screen.getByTestId('coach-home-switch-me'));
    expect(navigate).toHaveBeenCalledWith('home/me');
  });

  it('gives a coach who does not train no « Mon entraînement », only the page’s name (carnet#1529)', async () => {
    renderHome({ view: 'queue', athleteId: null }, false);
    await screen.findByTestId('coach-home');
    expect(screen.queryByTestId('coach-home-switch')).toBeNull();
    expect(screen.queryByText('My training')).toBeNull();
    expect(screen.getByTestId('coach-home-title')).toHaveTextContent('My athletes');
  });

  it('moves between the views', async () => {
    const user = userEvent.setup();
    const { navigate } = renderHome();
    await user.click(await screen.findByTestId('coach-view-season'));
    expect(navigate).toHaveBeenCalledWith('home/season');
    expect(await screen.findByTestId('coach-season')).toBeInTheDocument();
  });

  it('round-trips every route through the hash', () => {
    const routes: CoachHomeRoute[] = [
      { view: 'queue', athleteId: null },
      { view: 'queue', athleteId: 'u/with space' },
      { view: 'workspace', athleteId: 'u-lea' },
      { view: 'workspace', athleteId: 'u/with space' },
      { view: 'team' },
      { view: 'season' },
      { view: 'me' },
    ];
    for (const route of routes) {
      const hash = coachHomeRoute(route);
      const sub = hash === 'home' ? '' : hash.slice('home/'.length);
      expect(parseCoachHomeRoute(sub)).toEqual(route);
    }
    expect(parseCoachHomeRoute('chat/abc')).toBeNull();
    expect(parseCoachHomeRoute('activity/strava/1')).toBeNull();
    expect(parseCoachHomeRoute('athlete/%E0%A4%A')).toBeNull();
    expect(coachHomeRoute({ view: 'workspace', athleteId: 'u-lea' })).toBe('home/athlete/u-lea/plan');
    expect(parseCoachHomeRoute('athlete/u-lea/history')).toBeNull();
    expect(parseCoachHomeRoute('athlete//plan')).toBeNull();
    expect(parseCoachHomeRoute('athlete/u-lea/plan/more')).toBeNull();
  });

  it('the switch alone presses the scope it is given', async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<CoachHomeSwitch scope="me" onChange={onChange} />);
    expect(screen.getByTestId('coach-home-switch-me')).toHaveAttribute('aria-pressed', 'true');
    await user.click(screen.getByTestId('coach-home-switch-me'));
    expect(onChange).not.toHaveBeenCalled();
    await user.click(screen.getByTestId('coach-home-switch-athletes'));
    expect(onChange).toHaveBeenCalledWith('athletes');
  });
});

describe('on a phone', () => {
  it('takes turns between the list and the athlete, and goes back', async () => {
    mobile.value = true;
    const user = userEvent.setup();
    const { navigate } = renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-julien'));
    expect(navigate).toHaveBeenCalledWith('home/athlete/u-julien');
    expect(await screen.findByTestId('coach-athlete-pane')).toBeInTheDocument();
    expect(screen.queryByTestId('coach-queue')).not.toBeInTheDocument();
    await user.click(screen.getByTestId('coach-pane-back'));
    expect(await screen.findByTestId('coach-queue')).toBeInTheDocument();
  });
});

describe('the morning brief', () => {
  it('says where the brief stands under the queue\'s title, and nothing for a caller who coaches no one', async () => {
    renderHome();
    const brief = await screen.findByTestId('coach-brief');
    expect(brief).toHaveAttribute('data-state', 'active');
    expect(within(brief).getByTestId('coach-brief-status')).not.toBeEmptyDOMElement();
    // It is a note on the queue, never one of its rows.
    expect(within(screen.getByTestId('coach-queue')).getByTestId('coach-brief')).toBe(brief);
    expect(screen.getByTestId('coach-queue-today')).not.toContainElement(brief);
  });

  it('draws no brief when the server refuses it (403)', async () => {
    getBrief.mockRejectedValue({ response: { status: 403 } });
    renderHome();
    await screen.findByTestId('coach-queue');
    await waitFor(() => expect(getBrief).toHaveBeenCalled());
    expect(screen.queryByTestId('coach-brief')).not.toBeInTheDocument();
  });

  it('pauses, moves the hour and turns a reason off from its sheet, one change per save', async () => {
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-brief'));
    const sheet = await screen.findByTestId('coach-brief-sheet');

    await user.click(within(sheet).getByTestId('coach-brief-pause'));
    await waitFor(() => expect(updateBrief).toHaveBeenLastCalledWith({ paused: true }));

    await user.selectOptions(within(sheet).getByTestId('coach-brief-hour'), '6');
    await waitFor(() => expect(updateBrief).toHaveBeenLastCalledWith({ hour: 6 }));

    const fatigue = within(sheet).getByTestId('coach-brief-reason-deep_fatigue_streak');
    expect(fatigue).toHaveAttribute('aria-checked', 'true');
    await user.click(fatigue);
    await waitFor(() => expect(updateBrief).toHaveBeenLastCalledWith({ reasons_off: ['deep_fatigue_streak'] }));
    // One switch per reason the brief checks, missed planned sessions included.
    expect(within(sheet).getAllByRole('switch')).toHaveLength(8);
    expect(within(sheet).getByTestId('coach-brief-reason-planned_sessions_missed')).toHaveAttribute('aria-checked', 'true');
  });

  it('lists who is left out by their roster name and puts them back with the whole list', async () => {
    storedBrief = buildBrief(TODAY, { muted: ['u-rosalie', 'u-gone', 'u-kevin'] });
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-brief'));
    const muted = within(await screen.findByTestId('coach-brief-sheet')).getByTestId('coach-brief-muted');
    // An id the roster no longer names has no row, and still travels in the list.
    expect(within(muted).getAllByRole('listitem').map((row) => row.getAttribute('data-testid'))).toEqual([
      'coach-brief-muted-u-rosalie',
      'coach-brief-muted-u-kevin',
    ]);
    await user.click(within(muted).getByTestId('coach-brief-unmute-u-rosalie'));
    await waitFor(() => expect(updateBrief).toHaveBeenCalledWith({ muted: ['u-gone', 'u-kevin'] }));
  });

  it('says nobody is left out when the list is empty', async () => {
    storedBrief = buildBrief(TODAY, { muted: [] });
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-brief'));
    expect(await screen.findByTestId('coach-brief-muted-none')).toBeInTheDocument();
  });

  it('says a refused leave-out failed, and keeps the athlete in the brief', async () => {
    updateBrief.mockRejectedValue(new Error('down'));
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-julien'));
    await user.click(await screen.findByTestId('coach-brief-mute'));
    await waitFor(() => expect(updateBrief).toHaveBeenCalled());
    expect(await screen.findByText(i18n.t('coachHome.brief.save_failed'))).toBeInTheDocument();
    expect(await screen.findByTestId('coach-brief-mute')).toBeInTheDocument();
  });

  it('says a refused save failed, and shows the stored brief again', async () => {
    updateBrief.mockRejectedValue(new Error('down'));
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-brief'));
    const sheet = await screen.findByTestId('coach-brief-sheet');
    await user.click(within(sheet).getByTestId('coach-brief-reason-plan_ended'));
    expect(await within(sheet).findByRole('alert')).toBeInTheDocument();
    await waitFor(() => expect(within(sheet).getByTestId('coach-brief-reason-plan_ended')).toHaveAttribute('aria-checked', 'true'));
  });

  it('leaves the open athlete out of the brief and puts them back, sending the whole list each time', async () => {
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-julien'));
    const mute = await within(await screen.findByTestId('coach-athlete-pane')).findByTestId('coach-brief-mute');

    await user.click(mute);
    await waitFor(() => expect(updateBrief).toHaveBeenLastCalledWith({ muted: ['u-rosalie', 'u-julien'] }));
    const unmute = await screen.findByTestId('coach-brief-unmute');
    expect(screen.queryByTestId('coach-brief-mute')).not.toBeInTheDocument();

    await user.click(unmute);
    await waitFor(() => expect(updateBrief).toHaveBeenLastCalledWith({ muted: ['u-rosalie'] }));
    expect(await screen.findByTestId('coach-brief-mute')).toBeInTheDocument();
  });

  it('tells the server once per athlete the coach opens, and not for the first of the queue shown on its own', async () => {
    const user = userEvent.setup();
    renderHome();
    // A wide screen opens on the first of the queue: that is not an open.
    await screen.findByTestId('coach-athlete-pane');
    expect(markBriefOpened).not.toHaveBeenCalled();

    await user.click(screen.getByTestId('coach-queue-row-u-julien'));
    await waitFor(() => expect(markBriefOpened).toHaveBeenCalledWith('u-julien'));
    await user.click(screen.getByTestId('coach-queue-row-u-julien'));
    await user.click(screen.getByTestId('coach-queue-row-u-felix'));
    await waitFor(() => expect(markBriefOpened).toHaveBeenCalledWith('u-felix'));
    expect(markBriefOpened).toHaveBeenCalledTimes(2);
  });

  it('never lets a failed ping touch the pane', async () => {
    markBriefOpened.mockRejectedValue(new Error('offline'));
    const user = userEvent.setup();
    renderHome();
    await user.click(await screen.findByTestId('coach-queue-row-u-julien'));
    await waitFor(() => expect(markBriefOpened).toHaveBeenCalled());
    expect(screen.getByTestId('coach-athlete-pane')).toBeInTheDocument();
  });
});

describe('failure and emptiness', () => {
  it('says the roster could not be read and offers a retry', async () => {
    getRoster.mockRejectedValueOnce(new Error('down'));
    const user = userEvent.setup();
    renderHome();
    expect(await screen.findByTestId('coach-home-failed')).toBeInTheDocument();
    await user.click(within(screen.getByTestId('coach-home-failed')).getByRole('button'));
    expect(await screen.findByTestId('coach-queue')).toBeInTheDocument();
  });

  it('reads every page of a long roster before drawing it', async () => {
    const full = buildRoster(TODAY);
    const first = full.athletes.slice(0, 5);
    const second = full.athletes.slice(5);
    getRoster.mockReset();
    getRoster
      .mockResolvedValueOnce({ ...full, athletes: first, next_offset: 5 })
      .mockResolvedValueOnce({ ...full, athletes: second, next_offset: null });
    renderHome();
    await screen.findByTestId('coach-queue');
    expect(getRoster).toHaveBeenNthCalledWith(1, { offset: 0, limit: 100 });
    expect(getRoster).toHaveBeenNthCalledWith(2, { offset: 5, limit: 100 });
    expect(screen.getByTestId('coach-queue-count')).toHaveTextContent('8');
  });

  it('says so when the coach holds no athlete', async () => {
    getRoster.mockResolvedValue({ scope: { visible_members: 0, roster_members: 0 }, groups: [], athletes: [], next_offset: null });
    renderHome();
    expect(await screen.findByTestId('coach-home-empty')).toBeInTheDocument();
  });

  describe('before anyone joined', () => {
    const GROUP = { id: 'g-lea', name: 'Léa · Marathon de Montréal', athlete_count: 0 };
    const INVITE = {
      id: 'inv-1',
      group_id: 'g-lea',
      tenant_id: 't-1',
      code: 'ABCD2345',
      kind: 'member',
      created_by: 'coach-1',
      expires_at: '2099-12-13T14:40:00Z',
      max_uses: null,
      use_count: 0,
      is_active: true,
      approves_signup: false,
      created_at: '2026-11-13T14:40:00Z',
    };
    const emptyRoster = {
      scope: { visible_members: 0, roster_members: 0 },
      groups: [GROUP],
      athletes: [],
      next_offset: null,
    };

    it('shows the group of the invited athlete as invited and not joined, with why and what to do', async () => {
      getRoster.mockResolvedValue(emptyRoster);
      listInvites.mockResolvedValue({ invites: [INVITE] });
      const writeText = vi.fn().mockResolvedValue(undefined);
      Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
      openGroupRoom.mockResolvedValue({ id: 'room-lea' });
      getConversations.mockResolvedValue({ conversations: [], total: 0, limit: 50, offset: 0 });
      const { navigate } = renderHome();

      const row = await screen.findByTestId('coach-pending-invite-g-lea');
      expect(listInvites).toHaveBeenCalledWith('g-lea');
      expect(within(row).getByText('Léa · Marathon de Montréal')).toBeInTheDocument();
      expect(within(row).getByTestId('coach-pending-invite-status')).toHaveTextContent('Invited · hasn’t joined yet');
      expect(row).toHaveTextContent('Nobody has joined the group with this link yet.');

      fireEvent.click(within(row).getByTestId('coach-pending-invite-copy'));
      await waitFor(() => expect(writeText).toHaveBeenCalledWith(`${window.location.origin}/groups/join/ABCD2345`));
      expect(await within(row).findByText('Link copied')).toBeInTheDocument();

      fireEvent.click(within(row).getByTestId('coach-pending-invite-write'));
      await waitFor(() => expect(openGroupRoom).toHaveBeenCalledWith('g-lea'));
      await waitFor(() => expect(navigate).toHaveBeenCalledWith('chat/room-lea'));
    });

    // carnet#1650: the row says how far the link got, counts only.
    it('draws the four steps of the funnel with their marks, counts and date, and no athlete identity', async () => {
      getRoster.mockResolvedValue(emptyRoster);
      listInvites.mockResolvedValue({
        invites: [
          {
            ...INVITE,
            funnel: {
              open_count: 3,
              last_opened_at: '2026-11-14T10:00:00Z',
              held_count: 1,
              last_held_at: '2026-11-15T10:00:00Z',
              full_count: 0,
            },
          },
        ],
      });
      renderHome();

      const row = await screen.findByTestId('coach-pending-invite-g-lea');
      const funnel = within(row).getByTestId('coach-pending-invite-funnel');
      expect(funnel).toHaveTextContent('Invite progress');
      const states = ['sent', 'opened', 'account', 'joined'].map((key) =>
        within(funnel).getByTestId(`coach-pending-invite-step-${key}`).getAttribute('data-state'),
      );
      expect(states).toEqual(['done', 'done', 'done', 'current']);
      expect(within(funnel).getByTestId('coach-pending-invite-step-opened')).toHaveTextContent('Opened · 3 times');
      expect(within(funnel).getByTestId('coach-pending-invite-step-account')).toHaveTextContent('Account created');
      expect(within(funnel).getByTestId('coach-pending-invite-step-joined')).toHaveTextContent('Not in the group yet');
      // The two existing actions stay.
      expect(within(row).getByTestId('coach-pending-invite-copy')).toBeInTheDocument();
      expect(within(row).getByTestId('coach-pending-invite-write')).toBeInTheDocument();
    });

    it('reads a link nobody opened as "not opened yet", the next step still ahead', async () => {
      getRoster.mockResolvedValue(emptyRoster);
      listInvites.mockResolvedValue({
        invites: [
          {
            ...INVITE,
            funnel: { open_count: 0, last_opened_at: null, held_count: 0, last_held_at: null, full_count: 0 },
          },
        ],
      });
      renderHome();

      const funnel = await screen.findByTestId('coach-pending-invite-funnel');
      expect(within(funnel).getByTestId('coach-pending-invite-step-opened')).toHaveAttribute('data-state', 'current');
      expect(within(funnel).getByTestId('coach-pending-invite-step-opened')).toHaveTextContent('Not opened yet');
      expect(within(funnel).getByTestId('coach-pending-invite-step-account')).toHaveAttribute('data-state', 'todo');
    });

    it('warns when every account holding the invite was refused for a full group', async () => {
      getRoster.mockResolvedValue(emptyRoster);
      listInvites.mockResolvedValue({
        invites: [
          {
            ...INVITE,
            funnel: {
              open_count: 2,
              last_opened_at: '2026-11-14T10:00:00Z',
              held_count: 1,
              last_held_at: '2026-11-15T10:00:00Z',
              full_count: 1,
            },
          },
        ],
      });
      renderHome();

      const joined = await screen.findByTestId('coach-pending-invite-step-joined');
      expect(joined).toHaveAttribute('data-tone', 'warn');
      expect(joined).toHaveTextContent('Group full · a seat needs to open');
    });

    it('keeps the empty state when the invite of the group no longer works', async () => {
      getRoster.mockResolvedValue(emptyRoster);
      listInvites.mockResolvedValue({ invites: [{ ...INVITE, expires_at: '2020-01-01T00:00:00Z' }] });
      renderHome();
      expect(await screen.findByTestId('coach-home-empty')).toBeInTheDocument();
      expect(screen.queryByTestId('coach-pending-invites')).toBeNull();
    });
  });
});
