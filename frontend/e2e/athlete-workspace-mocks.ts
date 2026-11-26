// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mocked server an athlete workspace runs against — Léa's own room with the coach, her plan thread, a draft that swaps two days, the history
// ABOUTME: Dated from today, so the calendar always holds this week and next; shared by the workspace e2e and the design sweep (carnet#1816)

import type { Page } from '@playwright/test';
import type {
  CoachRosterResponse,
  PlanDay,
  PlanWeek,
  SubjectPlanHistory,
  SubjectPlanResponse,
  WorkoutPlan,
} from '@pierre/shared-types';
import { civilDayNumber } from '@pierre/ui-logic';
import { dayOffset } from '../src/test/coachRosterFixtures';
import { localToday } from './coach-home-mocks';

export const WORKSPACE_ATHLETE_ID = 'u-lea';
export const WORKSPACE_PLAN_ID = 'conv-plan-lea';
export const WORKSPACE_ROOM_ID = 'conv-room-lea';
const COACH_ID = 'user-123';
const GROUP = { id: 'g-lea', name: 'TRAKA' };
const ATHLETE = { user_id: WORKSPACE_ATHLETE_ID, display_name: 'Léa Gagnon' };
const AGENT_ID = 'agent-endurance';

/** The Monday of the week `today` falls in. */
function mondayOf(today: string): string {
  const weekday = new Date(civilDayNumber(today) * 86_400_000).getUTCDay();
  return dayOffset(today, -((weekday + 6) % 7));
}

/** Today at `hours:minutes` on this device's clock, as an instant. */
function todayAt(hours: number, minutes: number, daysAgo = 0): string {
  const now = new Date();
  return new Date(now.getFullYear(), now.getMonth(), now.getDate() - daysAgo, hours, minutes).toISOString();
}

function session(date: string, sport: string, workout: string, minutes: number, intensity: string, extra: Partial<PlanDay> = {}): PlanDay {
  return { date, position: 0, sport, workout, duration_min: minutes, intensity, rest: false, ...extra };
}

const CONVENTION = { kind: 'convention' as const, label: 'Coaching convention', note_kind: 'no_basis_convention' as const };

/** The live fortnight, and the draft over it: the track and Thursday's CrossFit swap, and a mobility session added on Saturday. */
function plans(today: string): { live: WorkoutPlan; draft: WorkoutPlan } {
  const monday = mondayOf(today);
  const day = (offset: number) => dayOffset(monday, offset);
  const next = (offset: number) => dayOffset(monday, 7 + offset);
  const track = (date: string) => session(date, 'ride', 'Piste à Bromont, endurance pure', 90, 'Z2');
  const crossfit = (date: string) => session(date, 'crossfit', 'CrossFit du jeudi', 45, 'force');
  const weekOne = (swapped: boolean): PlanDay[] => [
    session(day(0), 'ride', 'Powerwatts au seuil', 60, 'soutenue'),
    session(day(1), 'crossfit', 'CrossFit du mardi, la séance de force', 45, 'force'),
    swapped ? crossfit(day(2)) : track(day(2)),
    swapped ? track(day(3)) : crossfit(day(3)),
    session(day(4), 'ride', 'Sortie facile, dehors ou sur rouleau', 75, 'Z2'),
    session(day(5), 'hike', 'Randonnée autour de Sutton', 90, 'Z1–Z2'),
    ...(swapped ? [session(day(5), 'mobility', 'Mobilité des hanches', 20, 'facile', { position: 1, added: true })] : []),
    session(day(6), 'gravel_ride', 'Longue sortie facile, gravier', 105, 'Z2', { evidence: CONVENTION }),
  ];
  const weekTwo: PlanDay[] = [
    session(next(0), 'ride', 'Powerwatts au seuil', 60, 'soutenue'),
    session(next(1), 'crossfit', 'CrossFit du mardi', 45, 'force'),
    track(next(2)),
    crossfit(next(3)),
    { date: next(4), position: 0, sport: 'rest', workout: 'Repos', intensity: '', rest: true },
    session(next(5), 'hike', 'Randonnée autour de Sutton', 90, 'Z1–Z2', { coach_call: true, evidence: CONVENTION }),
    session(next(6), 'ride', 'Longue sortie avec sprints', 120, 'Z2 + sprints', { outside: true }),
  ];
  const weeks = (swapped: boolean): PlanWeek[] => [
    { week_start: monday, focus: 'Préparation, semaine 2 sur 2', current: true, days: weekOne(swapped) },
    { week_start: dayOffset(monday, 7), focus: 'Base, semaine 1', current: false, days: weekTwo },
  ];
  const phase = (kind: WorkoutPlan['phases'][number]['kind'], from: number, weeks: number, current = false) => ({
    kind,
    start: dayOffset(monday, from * 7),
    end: dayOffset(monday, (from + weeks) * 7),
    weeks,
    purpose: '',
    intent: '',
    current,
  });
  const base: Omit<WorkoutPlan, 'weeks'> = {
    goal_race: { name: 'The Traka 200', date: dayOffset(monday, 26 * 7 + 4), discipline: 'gravel', priority: 'A' },
    phases: [phase('prep', -1, 2, true), phase('base', 1, 12), phase('build', 13, 8), phase('peak', 21, 2), phase('taper', 23, 2)],
    weeks_deferred: 23,
  };
  return { live: { ...base, weeks: weeks(false) }, draft: { ...base, weeks: weeks(true) } };
}

/**
 * `GET …/subject-plan` for Léa: her live plan, the coach's open draft over it,
 * and the dates the server names proposed — the swap and Saturday's added
 * session.
 */
export function workspaceSubjectPlan(today = localToday()): SubjectPlanResponse {
  const { live, draft } = plans(today);
  const monday = mondayOf(today);
  return {
    athlete: ATHLETE,
    today,
    live,
    draft,
    draft_state: { draft_id: 'draft-lea-1', updated_at: todayAt(10, 5), conflicts: [], season_relaid: false },
    proposed_dates: [2, 3, 5].map((offset) => dayOffset(monday, offset)),
  };
}

/** `GET …/subject-plan/history`: the agent's swap this morning, the coach's added session, last week's publish. */
function history(today: string): SubjectPlanHistory {
  const monday = mondayOf(today);
  return {
    events: [
      { kind: 'agent_draft', source: 'agent', dates: [dayOffset(monday, 2), dayOffset(monday, 3)], created_at: todayAt(10, 5) },
      { kind: 'day_add', source: 'coach', dates: [dayOffset(monday, 5)], created_at: todayAt(9, 40) },
      {
        kind: 'publish',
        source: 'coach',
        dates: [0, 1, 2, 3, 4, 5, 6].map((offset) => dayOffset(monday, offset)),
        created_at: todayAt(18, 12, 3),
      },
      { kind: 'agent_draft', source: 'agent', dates: [], created_at: todayAt(14, 30, 11) },
    ],
  };
}

/** The roster with Léa in a room of her own with the coach: the trifecta the workspace is for. */
export function trifectaRoster(roster: CoachRosterResponse): CoachRosterResponse {
  return {
    ...roster,
    groups: [...roster.groups, { ...GROUP, athlete_count: 1 }],
    athletes: roster.athletes.map((athlete) =>
      athlete.user_id === WORKSPACE_ATHLETE_ID ? { ...athlete, groups: [GROUP] } : athlete,
    ),
  };
}

const json = (body: unknown) => ({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });

/**
 * The subject plan once « Refuser » put the live entries back on `date` of the
 * draft, as the server's revert does: the date reads as the live plan's, so
 * the server proposes it no more.
 */
function revertDate(plan: SubjectPlanResponse, date: string): SubjectPlanResponse {
  const { live, draft } = plan;
  if (draft === null) return plan;
  const liveEntries = live?.weeks.flatMap((week) => week.days).filter((entry) => entry.date === date) ?? [];
  const weeks = draft.weeks.map((week) => {
    if (!week.days.some((entry) => entry.date === date)) return week;
    const days = [...week.days.filter((entry) => entry.date !== date), ...liveEntries];
    return { ...week, days: days.sort((a, b) => a.date.localeCompare(b.date) || (a.position ?? 0) - (b.position ?? 0)) };
  });
  return { ...plan, draft: { ...draft, weeks }, proposed_dates: plan.proposed_dates.filter((proposed) => proposed !== date) };
}

/**
 * The subject plan once the coach kept, or withdrew, their call on the draft's
 * session at `date` and `position`, as the server's coach-call write does.
 */
function setCoachCall(plan: SubjectPlanResponse, date: string, position: number, on: boolean): SubjectPlanResponse {
  const { draft } = plan;
  if (draft === null) return plan;
  const weeks = draft.weeks.map((week) => ({
    ...week,
    days: week.days.map((entry) =>
      entry.date === date && (entry.position ?? 0) === position ? { ...entry, coach_call: on } : entry,
    ),
  }));
  return { ...plan, draft: { ...draft, weeks } };
}

/** What a workspace route records: the dates refused, and the coach's calls made. */
export interface WorkspaceCalls {
  reverted: string[];
  coachCalls: unknown[];
}

/**
 * Mocks Léa's workspace: her own group with its one athlete named to the
 * coach, the coach's plan thread on her and their room, the subject plan with
 * the draft, its history, the room's words and the plan thread's last turn.
 * A refused date answers the plan with the live entries back on it, and a
 * coach's call with the call kept or withdrawn on its session.
 */
export async function mockWorkspace(page: Page, calls: WorkspaceCalls = { reverted: [], coachCalls: [] }): Promise<void> {
  const today = localToday();
  let subject = workspaceSubjectPlan(today);

  const planThread = {
    id: WORKSPACE_PLAN_ID,
    title: ATHLETE.display_name,
    agent_id: AGENT_ID,
    agent_title: 'Dravr',
    group_id: null,
    subject_user_id: WORKSPACE_ATHLETE_ID,
    subject_group_id: GROUP.id,
    subject_display_name: ATHLETE.display_name,
    // The server names the room's one athlete on her coach's rows (carnet#1817).
    solo_athlete: ATHLETE,
    message_count: 2,
    unread_count: 0,
    created_at: todayAt(9, 0, 11),
    updated_at: todayAt(10, 5),
    last_message: { preview: "C'est proposé sur le calendrier.", role: 'assistant', created_at: todayAt(10, 5) },
  };
  const room = {
    id: WORKSPACE_ROOM_ID,
    title: GROUP.name,
    agent_id: AGENT_ID,
    agent_title: 'Dravr',
    group_id: GROUP.id,
    group_name: GROUP.name,
    solo_athlete: ATHLETE,
    message_count: 1,
    unread_count: 1,
    created_at: todayAt(9, 0, 30),
    updated_at: todayAt(10, 4),
    last_message: { preview: "Bien noté, je m'en occupe.", role: 'user', created_at: todayAt(10, 4) },
  };

  await page.route('**/api/chat/conversations**', async (route, request) => {
    const url = new URL(request.url());
    if (/\/api\/chat\/conversations\/?$/.test(url.pathname)) {
      await route.fulfill(json({ conversations: [planThread, room], total: 2, limit: 50, offset: 0 }));
      return;
    }
    await route.fallback();
  });
  await page.route(`**/api/groups/${GROUP.id}`, (route) =>
    route.fulfill(
      json({
        id: GROUP.id,
        tenant_id: 'tenant-1',
        name: GROUP.name,
        description: null,
        agent_id: AGENT_ID,
        agent_title: 'Dravr',
        agent_handle: 'dravr',
        owner_id: COACH_ID,
        coach_user_id: COACH_ID,
        coach_display_name: 'Phil',
        peer_data_sharing: false,
        respond_mode: 'all',
        digest_mode: 'off',
        max_members: 10,
        is_active: true,
        created_at: todayAt(9, 0, 30),
        updated_at: todayAt(9, 0, 30),
        solo_athlete: ATHLETE,
      }),
    ),
  );
  await page.route(`**/api/chat/groups/${GROUP.id}/transcript*`, (route) =>
    route.fulfill(
      json({
        group_id: GROUP.id,
        members: [],
        entries: [
          {
            id: 'entry-lea-1',
            speaker: 'member',
            withheld: false,
            own: false,
            author_user_id: WORKSPACE_ATHLETE_ID,
            author_display_name: ATHLETE.display_name,
            content: 'Le Centre National a déplacé la piste à jeudi cette semaine.',
            message_id: null,
            created_at: todayAt(10, 2),
          },
          {
            id: 'entry-coach-1',
            speaker: 'member',
            withheld: false,
            own: true,
            author_user_id: COACH_ID,
            author_display_name: 'Phil',
            content: "Bien noté, je m'en occupe.",
            message_id: 'msg-room-1',
            created_at: todayAt(10, 4),
          },
        ],
      }),
    ),
  );
  await page.route(`**/api/chat/conversations/${WORKSPACE_ROOM_ID}/messages**`, (route) =>
    route.fulfill(
      json({ messages: [{ id: 'msg-room-1', role: 'user', content: "Bien noté, je m'en occupe.", created_at: todayAt(10, 4) }] }),
    ),
  );
  await page.route(`**/api/chat/conversations/${WORKSPACE_PLAN_ID}/messages**`, (route) =>
    route.fulfill(
      json({
        messages: [
          {
            id: 'msg-plan-1',
            conversation_id: WORKSPACE_PLAN_ID,
            role: 'user',
            content: 'Échange la piste et le CrossFit de jeudi.',
            created_at: todayAt(10, 4),
          },
          {
            id: 'msg-plan-2',
            conversation_id: WORKSPACE_PLAN_ID,
            role: 'assistant',
            content: "C'est proposé sur le calendrier : la piste passe au jeudi, le CrossFit au mercredi.",
            created_at: todayAt(10, 5),
            finish_reason: 'stop',
            scene_blocks: JSON.stringify([
              { kind: 'workout_plan', plan: subject.draft, subject: ATHLETE, draft: subject.draft_state },
            ]),
          },
        ],
      }),
    ),
  );
  await page.route('**/api/chat/conversations/*/verdicts', (route) => route.fulfill(json({ verdicts: [], total: 0 })));
  await page.route('**/api/chat/conversations/*/read', (route) => route.fulfill(json({})));
  await page.route(`**/api/chat/conversations/${WORKSPACE_PLAN_ID}/participants`, (route) =>
    route.fulfill(
      json({
        participants: [
          { user_id: COACH_ID, role: 'owner', added_by: COACH_ID, added_at: todayAt(9, 0, 11) },
          { user_id: WORKSPACE_ATHLETE_ID, role: 'member', added_by: COACH_ID, added_at: todayAt(9, 0, 11) },
        ],
      }),
    ),
  );
  await page.route(`**/api/chat/athletes/${WORKSPACE_ATHLETE_ID}/thread`, (route) => route.fulfill(json(planThread)));
  await page.route(`**/api/chat/conversations/${WORKSPACE_PLAN_ID}/subject-plan/history**`, (route) =>
    route.fulfill(json(history(today))),
  );
  await page.route(`**/api/chat/conversations/${WORKSPACE_PLAN_ID}/subject-plan/draft/dates/*/revert**`, async (route, request) => {
    const date = new URL(request.url()).pathname.split('/').slice(-2)[0];
    calls.reverted.push(date);
    subject = revertDate(subject, date);
    await route.fulfill(json(subject));
  });
  await page.route(`**/api/chat/conversations/${WORKSPACE_PLAN_ID}/subject-plan/draft/days/**/coach-call**`, async (route, request) => {
    const body = request.postDataJSON() as { on: boolean };
    calls.coachCalls.push(body);
    const [date, position] = new URL(request.url()).pathname.split('/').slice(-3);
    subject = setCoachCall(subject, date, Number(position), body.on);
    await route.fulfill(json(subject));
  });
  await page.route(`**/api/chat/conversations/${WORKSPACE_PLAN_ID}/subject-plan`, (route) => route.fulfill(json(subject)));
}
