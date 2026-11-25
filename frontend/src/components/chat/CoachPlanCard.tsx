// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A plan card in a coach's plan thread on one athlete — every entry, rest days included, opens the day editor
// ABOUTME: A date's last entry offers "Add a session"; a session resting on convention or outside the evidence takes « C'est mon choix » (carnet#1515, #1714, #1716, #1814)

import { lazy, Suspense, useState, type ReactNode } from 'react';
import { Plus } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import type { PlanDay, PlanSubject, WorkoutPlan } from '@pierre/shared-types';
import {
  API_ERROR_KEYS,
  SESSION_EDITOR_KEYS,
  canAddSession,
  describeApiError,
  isSessionMoved,
  sessionExpect,
} from '@pierre/ui-logic';
import { Button, useErrorToast } from '../ui';
import { useRereadPlan, useSetCoachCall } from '../../hooks/useSessionEditor';
import WorkoutPlanCard, { type PlanDayAction } from './WorkoutPlanCard';
import type { SessionEditorOpening } from './SessionEditorSheet';

/** The human coach's call on a session (carnet#1814), its words under `humanCoach`. */
const COACH_CALL_KEYS = {
  action: 'humanCoach.coachCall.action',
  undo: 'humanCoach.coachCall.undo',
  hint: 'humanCoach.coachCall.hint',
} as const;

/**
 * Whether a session is one the coach may keep as their call: it rests on
 * coaching convention, or the envelope check found it outside its family's
 * evidence — the days the pre-publish check names. A day already called
 * always offers its undo.
 */
function takesCoachCall(day: PlanDay): boolean {
  if (day.rest) return false;
  return (
    day.coach_call === true ||
    day.evidence?.kind === 'convention' ||
    day.outside === true ||
    day.outside_evidence !== undefined
  );
}

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
  const { t, language } = useTranslation();
  const [editing, setEditing] = useState<SessionEditorOpening | null>(null);
  const coachCall = useSetCoachCall(conversationId, language);
  const reread = useRereadPlan(conversationId);
  const showError = useErrorToast();

  // A refused call reads the plan again when its session moved, as every day
  // write does: never a call onto whichever session sits there now.
  const toggleCall = (day: PlanDay, position: number) => {
    coachCall.mutate(
      { date: day.date, position, expect: sessionExpect(day), on: day.coach_call !== true },
      {
        onError: (error) => {
          if (isSessionMoved(error)) {
            showError(t(SESSION_EDITOR_KEYS.sessionMoved));
            reread();
            return;
          }
          showError(
            t(SESSION_EDITOR_KEYS.saveFailed),
            describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown }),
          );
        },
      },
    );
  };

  const dayAction: PlanDayAction = (day, { dateId, sessionId, entries, position }) => {
    const several = entries.length > 1;
    const last = position === entries.length - 1;
    const called = day.coach_call === true;
    const callHintId = `${sessionId}-coach-call-hint`;
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
        {takesCoachCall(day) && (
          <>
            <Button
              variant="tertiary"
              size="sm"
              // The words name what a press does — make the call, or withdraw
              // it — so no pressed state rides beside them. What making the
              // call means is part of the button's description: a title alone
              // reaches no screen reader once the date describes the button,
              // and no touch or keyboard user.
              aria-describedby={[several ? `${dateId} ${sessionId}` : dateId, called ? null : callHintId]
                .filter((id): id is string => id !== null)
                .join(' ')}
              title={called ? undefined : t(COACH_CALL_KEYS.hint)}
              disabled={coachCall.isPending}
              onClick={() => toggleCall(day, position)}
              data-testid={`plan-day-coach-call-${day.date}-${position}`}
            >
              {t(called ? COACH_CALL_KEYS.undo : COACH_CALL_KEYS.action)}
            </Button>
            {!called && (
              <span id={callHintId} className="sr-only">
                {t(COACH_CALL_KEYS.hint)}
              </span>
            )}
          </>
        )}
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
