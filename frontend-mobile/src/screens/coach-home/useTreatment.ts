// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: "Traité" and its undo for the phone — the optimistic roster patch from the shared hooks, and an alert when the server refuses
// ABOUTME: A refused write has already been rolled back to what the server holds by the time the alert shows

import { useCallback } from 'react';
import { Alert } from 'react-native';
import type { CoachAttentionReason, CoachRosterAthlete } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { useDismissAttention, useRestoreAttention } from '../../hooks/useCoachRoster';

/** The two writes on an attention reason. */
export interface Treatment {
  treat: (athlete: CoachRosterAthlete, reason: CoachAttentionReason) => void;
  undo: (athlete: CoachRosterAthlete, reason: CoachAttentionReason) => void;
}

export function useTreatment(): Treatment {
  const { t } = useTranslation();
  const { dismiss } = useDismissAttention();
  const { restore } = useRestoreAttention();

  const treat = useCallback(
    (athlete: CoachRosterAthlete, reason: CoachAttentionReason) => {
      dismiss({ athlete_user_id: athlete.user_id, reason_id: reason.id, fingerprint: reason.fingerprint }).catch(() =>
        Alert.alert(t('common.error'), t('coachHome.action.treatFailed')),
      );
    },
    [dismiss, t],
  );
  const undo = useCallback(
    (athlete: CoachRosterAthlete, reason: CoachAttentionReason) => {
      restore({ athlete_user_id: athlete.user_id, reason_id: reason.id }).catch(() =>
        Alert.alert(t('common.error'), t('coachHome.action.undoFailed')),
      );
    },
    [restore, t],
  );
  return { treat, undo };
}
