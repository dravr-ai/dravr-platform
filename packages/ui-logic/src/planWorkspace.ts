// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's athlete workspace on both clients — the fortnight laid out as seven dates a week, and each plan change worded « when · who · what »
// ABOUTME: Layout and words only: which dates are proposed is chat-utils' one comparison, what changed is the server's history (carnet#1816)

import { COACH_CALL_KEYS, countKey, entriesByDate, formatList, type CountKeys, type Translate } from '@pierre/chat-utils';
import type {
  CoachingGroup,
  Conversation,
  PlanChangeEvent,
  PlanChangeKind,
  PlanDay,
  PlanWeek,
  WorkoutPlan,
} from '@pierre/shared-types';
import { instantMs } from '@pierre/shared-types';
import { civilDayNumber, civilFromDayNumber, localToday } from './coachHome';
import { formatCivilDate, formatInstant, ROW_DATE } from './homeFormat';
import { findWorkThread, soloAthleteFor } from './privateLaneHooks';

/** The workspace's catalogue keys (contremaitre `strings/`, `plan.workspace`). */
export const PLAN_WORKSPACE_KEYS = {
  /** "Open the plan" — the athlete pane's way into the workspace. */
  open: 'plan.workspace.open',
  /** "Publish N changes" — the header's publish button, N the proposed dates. */
  publish: { one: 'plan.workspace.publishOne', many: 'plan.workspace.publishN' } satisfies CountKeys,
  /** The calendar section's name. */
  calendar: 'plan.workspace.calendar',
  /** "Proposed" — a date whose entries the draft changes over the live plan. */
  proposed: 'plan.workspace.proposed',
  /** "Accepted" — a proposed date the coach acknowledged this session. */
  accepted: 'plan.workspace.accepted',
  accept: 'plan.workspace.accept',
  refuse: 'plan.workspace.refuse',
  /** The date could not be put back to the live plan. */
  refuseFailed: 'plan.workspace.refuseFailed',
  /** "Session" — the dashed add button under a date's sessions, after its "+". */
  addShort: 'plan.workspace.addShort',
  /** "Edit {{session}}, {{date}}" — a session block's name. */
  editSession: 'plan.workspace.editSession',
  /** "Before you publish" — the pre-publish check's heading. */
  checkTitle: 'plan.workspace.checkTitle',
  /** A session resting on coaching convention rather than the athlete's data. */
  checkConvention: 'plan.workspace.checkConvention',
  /** A session outside its family's evidence. */
  checkOutside: 'plan.workspace.checkOutside',
  /** "Review" — opens the editor on the session the check names. */
  review: 'plan.workspace.review',
  /** The side panel's name, and the sheet's title below `lg`. */
  panel: 'plan.workspace.panel',
  /** The line over the history. */
  historyIntro: 'plan.workspace.historyIntro',
  historyEmpty: 'plan.workspace.historyEmpty',
  historyFailed: 'plan.workspace.historyFailed',
  /** "and N more dates" — a change touching more dates than its line names. */
  moreDates: { one: 'plan.workspace.history.moreDatesOne', many: 'plan.workspace.history.moreDatesN' } satisfies CountKeys,
  /** The athlete has no room of their own with the coach: their group holds other athletes. */
  noRoom: 'plan.workspace.noRoom',
  /** The athlete has no plan yet. */
  noPlan: 'plan.workspace.noPlan',
  /** The plan could not be read. */
  loadFailed: 'plan.workspace.loadFailed',
  /** "Plan proposed" — a plan card in the Dravr tab, drawn as one line. */
  changeEntry: 'plan.workspace.changeEntry',
  /** "N days changed" beside it. */
  changeEntryDays: { one: 'plan.workspace.changeEntryDaysOne', many: 'plan.workspace.changeEntryDaysN' } satisfies CountKeys,
  /** "See it on the calendar". */
  showOnCalendar: 'plan.workspace.showOnCalendar',
} as const;

/** The words of each kind of change; the coach's call is said in its own words, under `humanCoach`. */
const CHANGE_KIND_KEY: Record<PlanChangeKind, string> = {
  day_edit: 'plan.workspace.history.dayEdit',
  day_add: 'plan.workspace.history.dayAdd',
  day_remove: 'plan.workspace.history.dayRemove',
  day_restore: 'plan.workspace.history.dayRestore',
  date_revert: 'plan.workspace.history.dateRevert',
  coach_call: COACH_CALL_KEYS.badge,
  agent_draft: 'plan.workspace.history.agentDraft',
  publish: 'plan.workspace.history.publish',
  discard: 'plan.workspace.history.discard',
};

/** How many dates a history line names before it counts the rest. */
const HISTORY_DATES_NAMED = 3;

/** Where a date sits against the athlete's today: past dates are drawn quieter. */
export type PlanDateTiming = 'past' | 'today' | 'ahead';

/** One date of a calendar week. */
export interface PlanCalendarDay {
  /** `YYYY-MM-DD`. */
  date: string;
  /** The date's entries in the plan shown, in the order they are done; empty for a date the plan holds nothing on. */
  entries: PlanDay[];
  /** The draft changes this date over the live plan. */
  proposed: boolean;
  timing: PlanDateTiming;
}

/** One week of the calendar: the plan's week, and its dates Monday to Sunday. */
export interface PlanCalendarWeek {
  week: PlanWeek;
  days: PlanCalendarDay[];
}

const WEEK_OFFSETS = [0, 1, 2, 3, 4, 5, 6] as const;

/** Where `date` sits against `today`; both are `YYYY-MM-DD`, which order as text. */
function timingOf(date: string, today: string): PlanDateTiming {
  if (date === today) return 'today';
  return date < today ? 'past' : 'ahead';
}

/**
 * The plan's weeks as the workspace calendar draws them: each week's seven
 * dates from its Monday, a date the plan holds nothing on included, each
 * date's entries in order, the dates `proposed` names marked. A week whose
 * Monday does not parse keeps the dates its entries name, and an entry dated
 * outside its week's seven days is kept on its own date rather than dropped.
 */
export function planCalendarWeeks(
  plan: WorkoutPlan,
  proposed: readonly string[],
  today: string,
): PlanCalendarWeek[] {
  const marked = new Set(proposed);
  return plan.weeks.map((week) => {
    const byDate = new Map(entriesByDate(week.days).map((entries) => [entries[0].date, entries]));
    const monday = civilDayNumber(week.week_start);
    const span = Number.isNaN(monday) ? [] : WEEK_OFFSETS.map((offset) => civilFromDayNumber(monday + offset));
    const dates = [...new Set([...span, ...byDate.keys()])].sort();
    return {
      week,
      days: dates.map((date) => ({
        date,
        entries: byDate.get(date) ?? [],
        proposed: marked.has(date),
        timing: timingOf(date, today),
      })),
    };
  });
}

/** One change of the history as its line reads: « 10:05 · Dravr · Brouillon enregistré · mer. 25 nov. et jeu. 26 nov. ». */
export interface PlanHistoryLine {
  /** The time for a change of the reader's today, else its date. */
  when: string;
  /** "You" for the coach, the agent's name for the agent. */
  who: string;
  /** What changed, then the dates it touched. */
  what: string;
}

/** What a history line is worded with. */
export interface PlanHistoryWords {
  t: Translate;
  language: string;
  /** The name the thread's agent goes by, `shell.brandName` when it has none. */
  agentName: string;
  /** The reader's clock; defaults to now. */
  now?: Date;
}

/**
 * One change as the Historique tab lists it. The time stands for a change
 * made on the reader's today, the short date for an older one. A line names
 * its first dates — « mer. 25 nov. » — and counts the rest, so a draft written
 * over a fortnight stays one line.
 */
export function planHistoryLine(event: PlanChangeEvent, words: PlanHistoryWords): PlanHistoryLine {
  const { t, language, agentName } = words;
  const now = words.now ?? new Date();
  const at = new Date(instantMs(event.created_at));
  const sameDay = !Number.isNaN(at.getTime()) && localToday(at) === localToday(now);
  const when = sameDay
    ? formatInstant(event.created_at, language, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' })
    : formatInstant(event.created_at, language, { day: 'numeric', month: 'short' });
  const who = event.source === 'coach' ? t('chat.senderYou') : agentName;
  const label = t(CHANGE_KIND_KEY[event.kind]);
  if (event.dates.length === 0) return { when, who, what: label };
  const named = event.dates
    .slice(0, HISTORY_DATES_NAMED)
    .map((date) => formatCivilDate(date, language, ROW_DATE));
  const rest = event.dates.length - named.length;
  const dates = formatList(
    rest > 0 ? [...named, t(countKey(rest, language, PLAN_WORKSPACE_KEYS.moreDates), { count: rest })] : named,
    language,
  );
  return { when, who, what: `${label} · ${dates}` };
}

/**
 * The athlete's own room with the coach, among the groups read: the one
 * whose one athlete the server names to this coach as them (`solo_athlete`,
 * carnet#1249) — the room of the trifecta, the coach, the athlete and the
 * agent. `null` when every group of theirs holds other athletes too.
 */
export function athleteRoomGroup<G extends Pick<CoachingGroup, 'id' | 'coach_user_id' | 'solo_athlete'>>(
  groups: readonly G[],
  athleteId: string,
  viewerId: string | null,
): G | null {
  return groups.find((group) => soloAthleteFor(group, viewerId)?.user_id === athleteId) ?? null;
}

/**
 * The caller's conversation in `groupId`'s room, the one the room is drawn
 * over; an archived one is not it. `null` while the list holds none — the
 * server finds or opens it when asked.
 */
export function roomConversationOf(conversations: readonly Conversation[], groupId: string): Conversation | null {
  return conversations.find((conversation) => conversation.group_id === groupId && !conversation.archived_at) ?? null;
}

/**
 * The coach's plan thread on the athlete through `groupId` that the
 * workspace can open as it stands: the one the private lane writes to
 * (carnet#1070). An archived one is not usable until the server un-archives
 * it, which opening it does.
 */
export function usablePlanThread(
  conversations: readonly Conversation[],
  athleteId: string,
  groupId: string,
): Conversation | null {
  const thread = findWorkThread(conversations, athleteId, groupId);
  return thread !== null && !thread.archived_at ? thread : null;
}
