// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A plan card drawn as one line in a coach's athlete workspace — « Plan proposé · 3 jours changés · Voir sur le calendrier »
// ABOUTME: The days are the card's dates that differ from the athlete's live plan, by the one comparison the calendar marks them with (carnet#1816)

import { CalendarDays } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import { countKey, planProposedDates } from '@pierre/chat-utils';
import type { WorkoutPlan } from '@pierre/shared-types';
import { PLAN_WORKSPACE_KEYS } from '@pierre/ui-logic';
import { useSubjectPlan } from '../../hooks/useCoachPlan';

interface PlanChangeEntryProps {
  /** The coach's plan thread the card is in; its subject-plan read is the live plan the card is measured against. */
  conversationId: string | null;
  /** The plan the card carries. */
  plan: WorkoutPlan;
  /** Bring the workspace's calendar into view. */
  onShowCalendar: () => void;
}

/**
 * The line a plan card becomes beside the workspace's calendar: the calendar
 * already draws the plan, so the thread only says one was proposed, how many
 * of its days differ from what the athlete has now, and leads to the
 * calendar. The count waits for the live plan; until then the line names no
 * number rather than a wrong one.
 */
export default function PlanChangeEntry({ conversationId, plan, onShowCalendar }: PlanChangeEntryProps) {
  const { t, language } = useTranslation();
  const { read } = useSubjectPlan(conversationId, conversationId !== null);
  const changed = read?.status === 'ok' ? planProposedDates(plan, read.plan.live).length : null;
  const parts = [t(PLAN_WORKSPACE_KEYS.changeEntry)];
  if (changed !== null && changed > 0) {
    parts.push(t(countKey(changed, language, PLAN_WORKSPACE_KEYS.changeEntryDays), { count: changed }));
  }
  return (
    <div
      className="my-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-on-surface-variant"
      data-testid="plan-change-entry"
    >
      <CalendarDays className="h-4 w-4 shrink-0" aria-hidden="true" />
      <span>{parts.join(' · ')}</span>
      <button
        type="button"
        onClick={onShowCalendar}
        className="rounded font-medium text-primary transition-colors hover:text-primary-hover focus-ring touch-target"
        data-testid="plan-change-entry-show"
      >
        {t(PLAN_WORKSPACE_KEYS.showOnCalendar)}
      </button>
    </div>
  );
}
