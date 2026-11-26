// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A plan card drawn as one line in a coach's athlete workspace on the phone — « Plan proposé · Voir sur le calendrier »
// ABOUTME: The workspace's plan tab marks the proposed dates from the server's read; the line only says a plan was proposed and leads there (carnet#1818)

import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import type { DraftState } from '@pierre/shared-types';
import { PLAN_WORKSPACE_KEYS } from '@pierre/ui-logic';
import { useThemeColors } from '../../constants/theme';
import { useCardSubjectPlan } from '../../hooks/useCoachPlan';

/**
 * The thread is drawn inside a coach's athlete workspace, whose plan tab
 * draws the plan: its plan cards collapse to one line leading there, and the
 * open draft is not drawn after the rows.
 */
export interface WorkspacePlanSurface {
  kind: 'workspace';
  /** "See it on the calendar": bring the workspace's plan tab forward. */
  onShowCalendar: () => void;
}

interface PlanChangeEntryProps {
  /** The coach's plan thread the card is in, whose subject-plan read the plan tab draws. */
  conversationId: string | null;
  /** The coach's draft the card shows, when it shows one: a draft newer than the plan read reads it again. */
  draft: DraftState | null;
  /** Bring the workspace's plan tab forward. */
  onShowCalendar: () => void;
}

/**
 * The line a plan card becomes in the workspace's thread: the plan tab
 * already draws the plan and marks the dates it proposes, so the thread only
 * says one was proposed and leads there. A card a turn just added reads the
 * plan again, as a full card does, so the plan tab draws that turn's draft.
 */
export default function PlanChangeEntry({ conversationId, draft, onShowCalendar }: PlanChangeEntryProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  useCardSubjectPlan(conversationId, draft, conversationId !== null);
  return (
    <View className="my-1 flex-row flex-wrap items-center gap-x-2" testID="plan-change-entry">
      <Feather name="calendar" size={16} color={colors.text.secondary} />
      <Text className="text-sm text-text-secondary">{t(PLAN_WORKSPACE_KEYS.changeEntry)}</Text>
      <Text className="text-sm text-text-secondary" accessibilityElementsHidden importantForAccessibility="no">
        ·
      </Text>
      <Pressable
        onPress={onShowCalendar}
        accessibilityRole="button"
        hitSlop={10}
        className="min-h-11 justify-center"
        testID="plan-change-entry-show"
      >
        <Text className="text-sm font-medium text-primary">{t(PLAN_WORKSPACE_KEYS.showOnCalendar)}</Text>
      </Pressable>
    </View>
  );
}
