// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A plan card in a coach's plan thread on one athlete — every day, rest days included, opens the family editor
// ABOUTME: Drawn only where the thread's subject-plan read answered, which the server answers for the owner alone (carnet#1515)

import { lazy, Suspense, useState, type ReactNode } from 'react';
import { useTranslation } from '@pierre/i18n';
import type { PlanDay, PlanSubject, WorkoutPlan } from '@pierre/shared-types';
import { SESSION_EDITOR_KEYS } from '@pierre/ui-logic';
import { Button } from '../ui';
import WorkoutPlanCard, { type PlanDayAction } from './WorkoutPlanCard';

// The editor loads when a coach first opens it, never before first paint:
// it is a coach-only sheet, and the entry bundle has a byte budget.
const SessionEditorSheet = lazy(() => import('./SessionEditorSheet'));

interface CoachPlanCardProps {
  /** The coach's plan thread the card is in. */
  conversationId: string;
  /** The athlete the plan is for. */
  subject: PlanSubject;
  plan: WorkoutPlan;
  /** Drawn under the plan, inside the card — the draft controls. */
  children?: ReactNode;
}

/**
 * The plan card with "Build from family" on each day, and the editor that
 * opens on the day pressed. Its caller draws it only in a plan thread whose
 * subject-plan read answered — the server answers that read for the thread's
 * owner alone, while the athlete still shares — so anyone else, the athlete
 * included, reads the plain card, as everywhere else.
 */
export default function CoachPlanCard({ conversationId, subject, plan, children }: CoachPlanCardProps) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<PlanDay | null>(null);

  const dayAction: PlanDayAction = (day, dateId) => (
    <Button
      variant="tertiary"
      size="sm"
      className="-ml-2.5"
      aria-haspopup="dialog"
      aria-describedby={dateId}
      onClick={() => setEditing(day)}
      data-testid="plan-day-build-from-family"
    >
      {t(SESSION_EDITOR_KEYS.open)}
    </Button>
  );

  return (
    <>
      <WorkoutPlanCard plan={plan} dayAction={dayAction}>
        {children}
      </WorkoutPlanCard>
      {editing && (
        <Suspense fallback={null}>
          <SessionEditorSheet
            conversationId={conversationId}
            subject={subject}
            day={editing}
            onClose={() => setEditing(null)}
          />
        </Suspense>
      )}
    </>
  );
}
