// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One open calendar day — the workouts done on it, each opening its own view, and the plan's session with its chat draft
// ABOUTME: Shared by the week strip and the month sheet; a day older than stored history says so instead of reading as empty

import { useTranslation } from '@pierre/i18n';
import type { CalendarDay, HomeActivity } from '@pierre/shared-types';
import { useDistanceUnit } from '@pierre/ui-logic';
import { DayRow } from '../chat/WorkoutPlanCard';
import { activityViewRoute } from '../activity/activityRoute';
import { activityFigures, planDayDraft, sportLabel } from './homeFormat';

interface CalendarDayDetailProps {
  day: CalendarDay;
  /** The athlete's today: a day after it has nothing done to show. */
  today: string;
  id: string;
  onOpenChatDraft: (text: string) => void;
  onNavigate: (route: string) => void;
}

/** One workout of the day, the way into its own view. */
function ActivityLine({ activity, onNavigate }: { activity: HomeActivity; onNavigate: (route: string) => void }) {
  const { t, language } = useTranslation();
  const unit = useDistanceUnit();
  return (
    <li>
      <button
        type="button"
        data-testid={`calendar-activity-${activity.provider}-${activity.id}`}
        onClick={() => onNavigate(activityViewRoute(activity.provider, activity.id))}
        className="flex w-full min-w-0 items-baseline gap-2 rounded py-1 text-left text-sm transition-colors hover:text-primary focus-ring"
      >
        <span className="min-w-0 truncate font-medium text-on-surface">{activity.name}</span>
        <span className="shrink-0 text-on-surface-variant">
          {[sportLabel(t, activity.sport_type), ...activityFigures(t, activity, language, unit)].join(' · ')}
        </span>
      </button>
    </li>
  );
}

/** A small label over one half of the day: what was done, what was planned. */
function Part({ label }: { label: string }) {
  return <p className="text-xs font-semibold text-on-surface-variant">{label}</p>;
}

/**
 * The open day. Up to today it lists what was done; the plan's session for
 * it follows whenever the plan has one, so a past day reads as planned
 * against done. A day the plan never reached says so only from today on —
 * before that, what was done is the whole story.
 */
export function CalendarDayDetail({ day, today, id, onOpenChatDraft, onNavigate }: CalendarDayDetailProps) {
  const { t, language } = useTranslation();
  if (day.beforeHistory) {
    return (
      <div id={id} data-testid={id} className="mt-3 rounded-[10px] border ghost-border bg-surface-container-lowest px-3 py-2">
        <p className="py-1 text-sm text-on-surface-variant">{t('home.calendar.notKept')}</p>
      </div>
    );
  }
  const past = day.date <= today;
  const lookup = day.plan;
  const draft = lookup === null ? null : planDayDraft(t, language, day.date, lookup);
  const showPlan = lookup !== null && (lookup.kind !== 'uncovered' || !past);
  return (
    <div
      id={id}
      data-testid={id}
      className="mt-3 space-y-2 rounded-[10px] border ghost-border bg-surface-container-lowest px-3 py-2"
    >
      {past && (
        <div data-testid={`${id}-done`}>
          {showPlan && <Part label={t('home.calendar.done')} />}
          {day.activities.length === 0 ? (
            <p className="py-1 text-sm text-on-surface-variant">{t('home.calendar.noActivity')}</p>
          ) : (
            <ul>
              {day.activities.map((activity) => (
                <ActivityLine key={`${activity.provider}-${activity.id}`} activity={activity} onNavigate={onNavigate} />
              ))}
            </ul>
          )}
        </div>
      )}
      {showPlan && lookup !== null && (
        <div data-testid={`${id}-planned`}>
          {past && <Part label={t('home.calendar.planned')} />}
          {lookup.kind === 'uncovered' || draft === null ? (
            <p className="py-1 text-sm text-on-surface-variant">{t('home.plan.notCovered')}</p>
          ) : (
            <>
              <table className="w-full text-sm">
                <tbody>
                  <DayRow day={lookup.day} />
                </tbody>
              </table>
              <button
                type="button"
                onClick={() => onOpenChatDraft(draft)}
                className="mt-1 rounded py-1 text-left text-sm font-medium text-primary transition-colors hover:text-primary-hover focus-ring"
              >
                {draft}
              </button>
            </>
          )}
        </div>
      )}
      {lookup === null && !past && <p className="py-1 text-sm text-on-surface-variant">{t('home.calendar.noPlan')}</p>}
    </div>
  );
}
