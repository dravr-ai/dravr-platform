// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A plan card drawn as one line in a coach's athlete workspace — « Plan proposé · Voir sur le calendrier »
// ABOUTME: The calendar beside it marks the proposed dates from the server's read; the line only says a plan was proposed and leads there (carnet#1816)

import { CalendarDays } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import type { DraftState } from '@pierre/shared-types';
import { PLAN_WORKSPACE_KEYS } from '@pierre/ui-logic';
import { useCardSubjectPlan } from '../../hooks/useCoachPlan';

interface PlanChangeEntryProps {
  /** The coach's plan thread the card is in, whose subject-plan read the calendar draws. */
  conversationId: string | null;
  /** The coach's draft the card shows, when it shows one: a draft newer than the plan read reads it again. */
  draft: DraftState | null;
  /** Bring the workspace's calendar into view. */
  onShowCalendar: () => void;
}

/**
 * The line a plan card becomes beside the workspace's calendar: the calendar
 * already draws the plan and marks the dates it proposes, so the thread only
 * says one was proposed and leads to the calendar. A card a turn just added
 * reads the plan again, as a full card does, so the calendar beside it draws
 * that turn's draft.
 */
export default function PlanChangeEntry({ conversationId, draft, onShowCalendar }: PlanChangeEntryProps) {
  const { t } = useTranslation();
  useCardSubjectPlan(conversationId, draft, conversationId !== null);
  return (
    <div
      className="my-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-on-surface-variant"
      data-testid="plan-change-entry"
    >
      <CalendarDays className="h-4 w-4 shrink-0" aria-hidden="true" />
      <span>{t(PLAN_WORKSPACE_KEYS.changeEntry)}</span>
      <span aria-hidden="true">·</span>
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
