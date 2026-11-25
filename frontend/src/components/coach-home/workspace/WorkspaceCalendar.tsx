// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete workspace's two weeks — seven dates a row from lg, a day list below it — each session a button to its editor, each date a « + Séance »
// ABOUTME: A date the draft changes is « Proposé » with Accepter and Refuser; only a convention or outside-evidence session carries the diamond mark (carnet#1816)

import { useId } from 'react';
import { Check, Diamond, Pencil, Plus } from 'lucide-react';
import { clsx } from 'clsx';
import { useTranslation, type TFunction } from '@pierre/i18n';
import type { PlanDay } from '@pierre/shared-types';
import { COACH_CALL_KEYS, evidenceCallReason, evidenceChipLabel, outsideEvidenceLabel } from '@pierre/chat-utils';
import {
  MAX_SESSIONS_PER_DATE,
  PLAN_WORKSPACE_KEYS,
  ROW_DATE,
  SESSION_EDITOR_KEYS,
  canAddSession,
  formatCivilDate,
  newSessionDay,
  sportLabel,
  type PlanCalendarDay,
  type PlanCalendarWeek,
} from '@pierre/ui-logic';
import { BADGE_TONE } from '../../ui';
import { WeekHeading } from '../../chat/WorkoutPlanCard';
import type { SessionEditorOpening } from '../../chat/SessionEditorSheet';

/** The long date a screen reader hears for a date cell: « lundi 23 novembre ». */
const LONG_DATE: Intl.DateTimeFormatOptions = { weekday: 'long', day: 'numeric', month: 'long' };

interface WorkspaceCalendarProps {
  weeks: PlanCalendarWeek[];
  /** Whether the coach accepted the proposal on a date this session (decision W1). */
  isAccepted: (day: PlanCalendarDay) => boolean;
  onAccept: (day: PlanCalendarDay) => void;
  /** « Refuser »: put the live plan back on the date. */
  onRefuse: (date: string) => void;
  /** The date whose refusal is being written, while one is. */
  refusing: string | null;
  /** Open the session editor on a session, or on a blank one after a date's sessions. */
  onEdit: (opening: SessionEditorOpening) => void;
}

/** A session's one mark: the coach's call, or the diamond with where its evidence sits. */
type SessionMark = { kind: 'call'; text: string } | { kind: 'evidence'; text: string };

/**
 * The mark a session carries, if any: the coach's call when they kept it,
 * else the diamond on a session resting on convention or outside its
 * family's evidence. A rest day and every other session carry none.
 */
function sessionMark(day: PlanDay, t: TFunction): SessionMark | null {
  if (day.rest) return null;
  if (day.coach_call === true) return { kind: 'call', text: t(COACH_CALL_KEYS.badge) };
  const reason = evidenceCallReason(day);
  const text =
    reason === 'outside'
      ? outsideEvidenceLabel(day, t)
      : reason === 'convention' && day.evidence
        ? evidenceChipLabel(day.evidence, t)
        : undefined;
  return text === undefined ? null : { kind: 'evidence', text };
}

/** One session of a date: a button opening its editor, its sport, title, length and marks. */
function SessionBlock({
  day,
  position,
  entries,
  past,
  onEdit,
}: {
  day: PlanDay;
  position: number;
  entries: PlanDay[];
  past: boolean;
  onEdit: (opening: SessionEditorOpening) => void;
}) {
  const { t, language } = useTranslation();
  const markId = useId();
  const title = day.rest ? t('chat.restDay') : day.workout;
  const meta = [
    day.duration_min !== undefined ? t('plan.editor.minutes', { count: day.duration_min }) : null,
    day.intensity,
  ].filter((part): part is string => typeof part === 'string' && part.length > 0);
  const badge = day.added ? t(SESSION_EDITOR_KEYS.added) : day.edited ? t(SESSION_EDITOR_KEYS.edited) : null;
  const mark = sessionMark(day, t);
  return (
    <button
      type="button"
      aria-haspopup="dialog"
      aria-label={t(PLAN_WORKSPACE_KEYS.editSession, { session: title, date: formatCivilDate(day.date, language, ROW_DATE) })}
      aria-describedby={mark !== null ? markId : undefined}
      onClick={() => onEdit({ day, entries, mode: 'edit' })}
      data-testid={`workspace-session-${day.date}-${position}`}
      className="flex w-full flex-col gap-0.5 rounded-md p-1.5 text-left transition-colors hover:bg-surface-container-low focus-ring lg:p-1 xl:p-1.5"
    >
      <span className="flex w-full items-center justify-between gap-1">
        <span className="truncate text-xs text-on-surface-variant">{day.rest ? '' : sportLabel(t, day.sport)}</span>
        <Pencil className="h-3.5 w-3.5 shrink-0 text-primary" aria-hidden="true" />
      </span>
      <span
        className={clsx(
          'min-w-0 break-words text-sm font-medium leading-snug lg:hyphens-auto xl:hyphens-manual',
          past || day.rest ? 'text-on-surface-variant' : 'text-on-surface',
        )}
      >
        {title}
      </span>
      {meta.length > 0 && !day.rest && <span className="text-xs text-on-surface-variant">{meta.join(' · ')}</span>}
      {badge !== null && (
        <span className={`${BADGE_TONE.neutral.tone} self-start rounded-full px-1.5 text-xs font-medium`}>{badge}</span>
      )}
      {mark !== null && (
        <span
          id={markId}
          className="flex min-w-0 items-start gap-1 text-xs text-on-surface-variant"
          data-testid={mark.kind === 'call' ? 'workspace-session-coach-call' : 'workspace-session-evidence'}
        >
          {mark.kind === 'call' ? (
            <Check className="mt-0.5 h-3 w-3 shrink-0" aria-hidden="true" />
          ) : (
            <Diamond className="mt-0.5 h-3 w-3 shrink-0" aria-hidden="true" />
          )}
          {/* A column narrower than the mark's longest word hyphenates it rather than overflow the cell. */}
          <span className="min-w-0 hyphens-auto break-words">{mark.text}</span>
        </span>
      )}
    </button>
  );
}

/** One date: its number, « Proposé » when the draft changes it, its sessions, the proposal's two answers and « + Séance ». */
function DateCell({
  day,
  accepted,
  onAccept,
  onRefuse,
  refusing,
  onEdit,
}: {
  day: PlanCalendarDay;
  accepted: boolean;
  onAccept: () => void;
  onRefuse: () => void;
  refusing: boolean;
  onEdit: (opening: SessionEditorOpening) => void;
}) {
  const { t, language } = useTranslation();
  const dateId = useId();
  const today = day.timing === 'today';
  const past = day.timing === 'past';
  const full = !canAddSession(day.entries);
  const rowDate = formatCivilDate(day.date, language, ROW_DATE);
  return (
    <li
      aria-current={today ? 'date' : undefined}
      data-testid={`workspace-date-${day.date}`}
      data-proposed={day.proposed ? 'true' : undefined}
      className={clsx(
        'flex min-w-0 flex-col gap-1.5 rounded-lg p-2 lg:min-h-[176px] lg:p-1.5 xl:p-2',
        past ? 'bg-surface' : 'bg-surface-container-lowest',
        day.proposed
          ? 'border-[1.5px] border-dashed border-primary'
          : today
            ? 'border-[1.5px] border-primary'
            : 'border ghost-border',
      )}
    >
      {/* In a narrow column « Proposé » wraps under the date rather than past the cell's edge. */}
      <div className="flex flex-wrap items-center justify-between gap-x-1.5 gap-y-1">
        <h3
          id={dateId}
          className={clsx(
            'm-0 font-mono text-sm',
            today ? 'font-semibold text-primary' : past ? 'text-outline' : 'text-on-surface-variant',
          )}
        >
          <span aria-hidden="true" className="lg:hidden">
            {rowDate}
          </span>
          <span aria-hidden="true" className="hidden lg:inline">
            {formatCivilDate(day.date, language, { day: 'numeric' })}
          </span>
          <span className="sr-only">{formatCivilDate(day.date, language, LONG_DATE)}</span>
        </h3>
        {day.proposed &&
          (accepted ? (
            <span
              className={`${BADGE_TONE.neutral.tone} inline-flex max-w-full items-center gap-1 rounded-full px-1.5 text-xs font-semibold`}
              data-testid="workspace-date-accepted"
            >
              <Check className="h-3 w-3 shrink-0" aria-hidden="true" />
              <span className="truncate">{t(PLAN_WORKSPACE_KEYS.accepted)}</span>
            </span>
          ) : (
            <span
              className={`${BADGE_TONE.accent.tone} max-w-full truncate rounded-full px-1.5 text-xs font-semibold`}
              data-testid="workspace-date-proposed"
            >
              {t(PLAN_WORKSPACE_KEYS.proposed)}
            </span>
          ))}
      </div>
      {day.entries.map((entry, position) => (
        <SessionBlock
          key={`${entry.date}-${position}`}
          day={entry}
          position={position}
          entries={day.entries}
          past={past}
          onEdit={onEdit}
        />
      ))}
      {day.proposed && (
        <div className="flex min-w-0 flex-wrap items-center gap-x-3 text-xs font-semibold">
          {!accepted && (
            <button
              type="button"
              aria-describedby={dateId}
              onClick={onAccept}
              data-testid={`workspace-accept-${day.date}`}
              className="max-w-full truncate rounded text-primary transition-colors hover:text-primary-hover focus-ring touch-target"
            >
              {t(PLAN_WORKSPACE_KEYS.accept)}
            </button>
          )}
          <button
            type="button"
            aria-describedby={dateId}
            disabled={refusing}
            aria-busy={refusing}
            onClick={onRefuse}
            data-testid={`workspace-refuse-${day.date}`}
            className="max-w-full truncate rounded text-on-surface-variant transition-colors hover:text-on-surface disabled:cursor-not-allowed disabled:opacity-50 focus-ring touch-target"
          >
            {t(PLAN_WORKSPACE_KEYS.refuse)}
          </button>
        </div>
      )}
      <button
        type="button"
        aria-haspopup="dialog"
        aria-label={t(SESSION_EDITOR_KEYS.addTitle, { date: rowDate })}
        disabled={full}
        title={full ? t(SESSION_EDITOR_KEYS.dateFull, { date: rowDate, max: MAX_SESSIONS_PER_DATE }) : undefined}
        onClick={() =>
          onEdit({ day: day.entries[0] ?? newSessionDay(day.date, day.entries), entries: day.entries, mode: 'add' })
        }
        data-testid={`workspace-add-${day.date}`}
        className="mt-auto flex h-8 max-w-full items-center justify-center gap-1 self-start overflow-hidden rounded-md border border-dashed px-3 lg:self-stretch lg:px-0 border-primary/45 text-xs font-semibold text-primary transition-colors hover:bg-primary-container/40 disabled:cursor-not-allowed disabled:opacity-50 focus-ring touch-target"
      >
        <Plus className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        <span className="truncate">{t(PLAN_WORKSPACE_KEYS.addShort)}</span>
      </button>
    </li>
  );
}

/**
 * The fortnight as the coach works it. From `lg` each week is a row of its
 * seven dates under the weekday names; below it the dates run as one list,
 * each naming its full date. Today is outlined, a past date drawn quieter,
 * and a date the draft changes is dashed and says « Proposé » until the
 * coach accepts or refuses it.
 */
export function WorkspaceCalendar({ weeks, isAccepted, onAccept, onRefuse, refusing, onEdit }: WorkspaceCalendarProps) {
  const { language } = useTranslation();
  const currentIndex = weeks.findIndex((week) => week.week.current);
  const weekdays = weeks[0]?.days.slice(0, 7) ?? [];
  return (
    <div className="flex flex-col gap-5" data-testid="workspace-calendar">
      <div aria-hidden="true" className="hidden grid-cols-7 gap-2 text-xs font-semibold text-on-surface-variant lg:grid">
        {weekdays.map((day) => (
          <span key={day.date}>{formatCivilDate(day.date, language, { weekday: 'short' })}</span>
        ))}
      </div>
      {weeks.map((week, index) => (
        <div key={week.week.week_start} className="flex flex-col gap-1" data-testid={`workspace-week-${week.week.week_start}`}>
          <WeekHeading week={week.week} index={index} currentIndex={currentIndex} />
          <ol className="m-0 flex list-none flex-col gap-2 p-0 lg:grid lg:grid-cols-7">
            {week.days.map((day) => (
              <DateCell
                key={day.date}
                day={day}
                accepted={isAccepted(day)}
                onAccept={() => onAccept(day)}
                onRefuse={() => onRefuse(day.date)}
                refusing={refusing === day.date}
                onEdit={onEdit}
              />
            ))}
          </ol>
        </div>
      ))}
    </div>
  );
}
