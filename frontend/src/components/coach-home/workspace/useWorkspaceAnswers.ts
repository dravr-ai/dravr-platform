// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete workspace's answers to the draft — « Accepter » for this session, « Refuser » and « C'est mon choix » through the day editor's hooks
// ABOUTME: Each write resolves whether it landed, so the control that gave it can move focus; a refused write says why in a toast (carnet#1816)

import { useCallback, useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import type { PrePublishItem } from '@pierre/chat-utils';
import {
  API_ERROR_KEYS,
  PLAN_WORKSPACE_KEYS,
  SESSION_EDITOR_KEYS,
  describeApiError,
  isSessionMoved,
  sessionExpect,
  type PlanCalendarDay,
} from '@pierre/ui-logic';
import { useRereadPlan, useRevertDraftDate, useSetCoachCall } from '../../../hooks/useSessionEditor';
import { useErrorToast } from '../../ui';

/** What the coach can answer to the draft from the workspace. */
export interface WorkspaceAnswers {
  /** Whether the coach accepted the proposal on a date this session, as it stands now (decision W1). */
  isAccepted: (day: PlanCalendarDay) => boolean;
  /** « Accepter »: acknowledge a date's proposal for this session; nothing is written. */
  accept: (day: PlanCalendarDay) => void;
  /** « Refuser »: put the live plan back on the date; resolves whether it was. */
  refuse: (date: string) => Promise<boolean>;
  /** The date being put back, while one is. */
  refusing: string | null;
  /** « C'est mon choix »: keep a session the check names as the coach's call; resolves whether it was kept. */
  keep: (item: PrePublishItem) => Promise<boolean>;
  /** A call is being saved. */
  keeping: boolean;
}

/** A date's entries as the acceptance of its proposal was given on: a changed date asks again. */
function entriesSignature(day: PlanCalendarDay): string {
  return JSON.stringify(day.entries);
}

/**
 * The workspace's answers to the draft in the plan thread `planThreadId`.
 * « Accepter » is held for this session only — publishing accepts every date
 * not refused — and a date whose entries change again asks again. A call
 * whose session moved reads the plan again, as the plan card's does, and never
 * lands on whichever session sits there now.
 */
export function useWorkspaceAnswers(planThreadId: string | null): WorkspaceAnswers {
  const { t, language } = useTranslation();
  const showError = useErrorToast();

  const [accepted, setAccepted] = useState<ReadonlyMap<string, string>>(new Map());
  const isAccepted = useCallback(
    (day: PlanCalendarDay) => accepted.get(day.date) === entriesSignature(day),
    [accepted],
  );
  const accept = useCallback((day: PlanCalendarDay) => {
    setAccepted((held) => new Map(held).set(day.date, entriesSignature(day)));
  }, []);

  const revert = useRevertDraftDate(planThreadId, language);
  const refuse = async (date: string): Promise<boolean> => {
    try {
      await revert.mutateAsync({ date });
      return true;
    } catch (error) {
      showError(t(PLAN_WORKSPACE_KEYS.refuseFailed), describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown }));
      return false;
    }
  };

  const coachCall = useSetCoachCall(planThreadId, language);
  const reread = useRereadPlan(planThreadId);
  const keep = async (item: PrePublishItem): Promise<boolean> => {
    try {
      await coachCall.mutateAsync({ date: item.day.date, position: item.position, expect: sessionExpect(item.day), on: true });
      return true;
    } catch (error) {
      if (isSessionMoved(error)) {
        showError(t(SESSION_EDITOR_KEYS.sessionMoved));
        reread();
      } else {
        showError(t(SESSION_EDITOR_KEYS.saveFailed), describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown }));
      }
      return false;
    }
  };

  return {
    isAccepted,
    accept,
    refuse,
    refusing: revert.isPending ? (revert.variables?.date ?? null) : null,
    keep,
    keeping: coachCall.isPending,
  };
}
