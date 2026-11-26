// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One open calendar day — the workouts done on it, each opening its own view, and the plan's entries, each with its chat draft
// ABOUTME: Shared by the week strip and the month sheet; a day older than stored history says so instead of reading as empty

import { useTranslation } from '@pierre/i18n';
import type { CalendarDay, HomeActivity } from '@pierre/shared-types';
import { activityRowFigures, planDayDraft, sportLabel, useDistanceUnit } from '@pierre/ui-logic';
import { PlanDateList } from '../chat/WorkoutPlanCard';
import { activityViewRoute } from '../activity/activityRoute';

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
        className="block w-full min-w-0 rounded py-1 text-left text-sm transition-colors hover:text-primary focus-ring"
      >
        <span className="block truncate font-medium text-on-surface" title={activity.name}>{activity.name}</span>
        <span className="block text-on-surface-variant">
          {sportLabel(t, activity.sport_type)}
          {/* Under the title, as served: the line wraps and the name truncates, never the attribution (carnet#1154). */}
          {activity.attribution && (
            <span data-testid="calendar-activity-attribution">{` · ${activity.attribution}`}</span>
          )}
          {activityRowFigures(t, activity, language, unit)
            .map((figure) => ` · ${figure}`)
            .join('')}
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
 * The open day. Up to today it lists what was done; the plan's entries for
 * it follow whenever the plan has them — stacked, so a narrow panel never
 * squeezes them into the card's table, each with its steps folded under
 * their count and a short « Pourquoi cette séance ? » that drafts the full
 * question in chat (carnet#1840) — so a past day reads as planned against done.
 * A day the plan never reached says so only from today on — before that,
 * what was done is the whole story.
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
          {lookup.kind === 'uncovered' ? (
            <p className="py-1 text-sm text-on-surface-variant">{t('home.plan.notCovered')}</p>
          ) : (
            <PlanDateList
              entries={lookup.days}
              action={(entry, place) => {
                const draft = planDayDraft(t, language, entry);
                return (
                  // The link's words are short; the question it drafts in
                  // chat is the full sentence naming the session. A rest day
                  // keeps its own question as its words.
                  <button
                    type="button"
                    data-testid={`${id}-draft-${place.position}`}
                    aria-describedby={place.sessionId}
                    onClick={() => onOpenChatDraft(draft)}
                    className="rounded py-1 text-left text-sm font-medium text-primary transition-colors hover:text-primary-hover focus-ring touch-target"
                  >
                    {entry.rest ? draft : t('home.plan.whySession')}
                  </button>
                );
              }}
            />
          )}
        </div>
      )}
      {lookup === null && !past && <p className="py-1 text-sm text-on-surface-variant">{t('home.calendar.noPlan')}</p>}
    </div>
  );
}
