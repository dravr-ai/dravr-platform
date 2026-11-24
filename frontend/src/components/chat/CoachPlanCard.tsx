// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A plan card in a coach's plan thread on one athlete — every entry, rest days included, opens the day editor
// ABOUTME: A date's last entry offers "Add a session"; drawn only where the thread's subject-plan read answered (carnet#1515, #1714, #1716)

import { lazy, Suspense, useState, type ReactNode } from 'react';
import { Plus } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import type { PlanSubject, WorkoutPlan } from '@pierre/shared-types';
import { SESSION_EDITOR_KEYS, canAddSession } from '@pierre/ui-logic';
import { Button } from '../ui';
import WorkoutPlanCard, { type PlanDayAction } from './WorkoutPlanCard';
import type { SessionEditorOpening } from './SessionEditorSheet';

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
 * The plan card with "Edit day" on each entry — "Edit session" once a date
 * holds more than one — and "Add a session" under a date's last entry while
 * the date has room, and the editor that opens on the entry pressed. Its
 * caller draws it only in a plan thread whose subject-plan read answered —
 * the server answers that read for the thread's owner alone, while the
 * athlete still shares — so anyone else, the athlete included, reads the
 * plain card, as everywhere else.
 */
export default function CoachPlanCard({ conversationId, subject, plan, children }: CoachPlanCardProps) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<SessionEditorOpening | null>(null);

  const dayAction: PlanDayAction = (day, { dateId, sessionId, entries, position }) => {
    const several = entries.length > 1;
    const last = position === entries.length - 1;
    return (
      <div className="-ml-2.5 flex flex-wrap items-center gap-x-1">
        <Button
          variant="tertiary"
          size="sm"
          aria-haspopup="dialog"
          // Beside its siblings, a session is told apart by its own title.
          aria-describedby={several ? `${dateId} ${sessionId}` : dateId}
          onClick={() => setEditing({ day, entries, mode: 'edit' })}
          data-testid={`plan-day-edit-${day.date}-${position}`}
        >
          {t(several ? SESSION_EDITOR_KEYS.editSession : SESSION_EDITOR_KEYS.edit)}
        </Button>
        {last && canAddSession(entries) && (
          <Button
            variant="tertiary"
            size="sm"
            aria-haspopup="dialog"
            aria-describedby={dateId}
            onClick={() => setEditing({ day, entries, mode: 'add' })}
            data-testid={`plan-day-add-${day.date}`}
          >
            <Plus className="mr-1 h-3.5 w-3.5" aria-hidden="true" />
            {t(SESSION_EDITOR_KEYS.addSession)}
          </Button>
        )}
      </div>
    );
  };

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
            opening={editing}
            onClose={() => setEditing(null)}
          />
        </Suspense>
      )}
    </>
  );
}
