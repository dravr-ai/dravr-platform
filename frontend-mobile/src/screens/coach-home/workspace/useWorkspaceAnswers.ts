// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete workspace's answers to the draft on the phone — « Accepter » for this visit, « Refuser », « C'est mon choix » and its withdrawal
// ABOUTME: Writes go through the day editor's hooks and resolve whether they landed; a refused write says why in an alert (carnet#1818)

import { useCallback, useState } from 'react';
import { Alert } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import type { PlanDay } from '@pierre/shared-types';
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

/** What the coach can answer to the draft from the workspace. */
export interface WorkspaceAnswers {
  /** Whether the coach accepted the proposal on a date this visit, as it stands now (decision W1). */
  isAccepted: (day: PlanCalendarDay) => boolean;
  /** « Accepter »: acknowledge a date's proposal for this visit; nothing is written. */
  accept: (day: PlanCalendarDay) => void;
  /** « Refuser »: put the live plan back on the date; resolves whether it was. */
  refuse: (date: string) => Promise<boolean>;
  /** The date being put back, while one is. */
  refusing: string | null;
  /**
   * « C'est mon choix » (`on`) or « Retirer mon choix »: keep a session as the
   * coach's call, or withdraw it; resolves whether it landed. `position` is
   * the session's place among its date's entries.
   */
  setCall: (day: PlanDay, position: number, on: boolean) => Promise<boolean>;
  /** A call is being saved: every call control waits. */
  callSaving: boolean;
}

/** A date's entries as the acceptance of its proposal was given on: a changed date asks again. */
function entriesSignature(day: PlanCalendarDay): string {
  return JSON.stringify(day.entries);
}

/**
 * The workspace's answers to the draft in the plan thread `planThreadId`.
 * « Accepter » is held for as long as the screen is open — publishing accepts
 * every date not refused — and a date whose entries change again asks again.
 * A call whose session moved reads the plan again, as the plan card's does,
 * and never lands on whichever session sits there now.
 */
export function useWorkspaceAnswers(planThreadId: string | null): WorkspaceAnswers {
  const { t, language } = useTranslation();

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
      Alert.alert(t(PLAN_WORKSPACE_KEYS.refuseFailed), describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown }));
      return false;
    }
  };

  const coachCall = useSetCoachCall(planThreadId, language);
  const reread = useRereadPlan(planThreadId);
  const setCall = async (day: PlanDay, position: number, on: boolean): Promise<boolean> => {
    try {
      await coachCall.mutateAsync({ date: day.date, position, expect: sessionExpect(day), on });
      return true;
    } catch (error) {
      if (isSessionMoved(error)) {
        Alert.alert(t(SESSION_EDITOR_KEYS.sessionMoved));
        reread();
      } else {
        Alert.alert(t(SESSION_EDITOR_KEYS.saveFailed), describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown }));
      }
      return false;
    }
  };

  return {
    isAccepted,
    accept,
    refuse,
    refusing: revert.isPending ? (revert.variables?.date ?? null) : null,
    setCall,
    callSaving: coachCall.isPending,
  };
}
