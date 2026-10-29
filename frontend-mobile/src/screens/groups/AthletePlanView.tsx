// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coached athlete's live plan, read-only, shown in Group info in place of the roster (carnet#1244)
// ABOUTME: The plan the athlete sees, drawn by the plan card; back returns to Group info, "Work on" opens the plan thread

import React from 'react';
import { View, Text, ActivityIndicator, ScrollView, TouchableOpacity } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { PLAN_THREAD_KEYS, PLAN_VIEW_KEYS } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { useThemeColors } from '../../constants/theme';
import { Button } from '../../components/ui';
import { useAthletePlan } from '../../hooks/useCoachPlan';
import WorkoutPlanCard from '../chat/WorkoutPlanCard';
import type { GroupMember } from '../../types';

export interface AthletePlanViewProps {
  /** The athlete whose plan is open. */
  member: GroupMember;
  /** The group the caller coaches them in. */
  groupId: string;
  /** Back to Group info. */
  onBack: () => void;
  /** Open the caller's plan thread for the athlete, where the plan changes. */
  onWorkOnPlan: (member: GroupMember) => void;
  /** The plan thread is being opened. */
  isOpeningThread: boolean;
}

/**
 * The plan a coached athlete sees, as their coach reads it outside the plan
 * thread: no draft, nothing editable. Drawn inside the Group info sheet
 * rather than over it, so no second sheet stacks on the first. An athlete
 * with no published plan, one who stopped sharing with the coach, and a read
 * that failed each say so.
 */
export function AthletePlanView({ member, groupId, onBack, onWorkOnPlan, isOpeningThread }: AthletePlanViewProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { read, isLoading, isError, refetch } = useAthletePlan(member.user_id, groupId);
  const name =
    read?.status === 'ok' ? read.plan.athlete.display_name : (member.display_name ?? t('app.unknownMember'));

  let body: React.ReactNode = null;
  if (isLoading) {
    body = (
      <View className="py-10 items-center">
        <ActivityIndicator size="large" color={colors.pierre.violet} />
      </View>
    );
  } else if (isError) {
    body = (
      <View className="py-6 items-center" testID="athlete-plan-error">
        <Text className="text-sm text-text-secondary text-center mb-3">{t(PLAN_VIEW_KEYS.loadError)}</Text>
        <Button title={t('common.retry')} variant="secondary" onPress={refetch} />
      </View>
    );
  } else if (read?.status === 'consentRevoked') {
    body = (
      <Text className="py-6 text-sm text-text-secondary text-center" testID="athlete-plan-revoked">
        {t(PLAN_VIEW_KEYS.consentRevoked, { name })}
      </Text>
    );
  } else if (read?.status === 'ok' && read.plan.live === null) {
    body = (
      <Text className="py-6 text-sm text-text-secondary text-center" testID="athlete-plan-empty">
        {t(PLAN_VIEW_KEYS.empty, { name })}
      </Text>
    );
  } else if (read?.status === 'ok' && read.plan.live !== null) {
    body = (
      <View testID="athlete-plan">
        <Text className="text-xs text-text-tertiary mb-3">{t(PLAN_VIEW_KEYS.readOnly, { name })}</Text>
        <WorkoutPlanCard plan={read.plan.live} />
      </View>
    );
  }

  return (
    <ScrollView testID="athlete-plan-view" keyboardShouldPersistTaps="handled">
      <View className="flex-row items-center mb-4">
        <TouchableOpacity
          className="p-2 -ml-2 mr-1"
          onPress={onBack}
          accessibilityRole="button"
          accessibilityLabel={t('common.back')}
          testID="athlete-plan-back"
        >
          <Feather name="chevron-left" size={22} color={colors.text.primary} />
        </TouchableOpacity>
        <Text className="flex-1 text-lg font-bold text-text-primary" numberOfLines={1}>
          {t(PLAN_VIEW_KEYS.title, { name })}
        </Text>
      </View>
      {body}
      {read?.status !== 'consentRevoked' && (
        <View className="mt-4">
          <Button
            title={t(PLAN_THREAD_KEYS.open, { name })}
            variant="secondary"
            onPress={() => onWorkOnPlan(member)}
            loading={isOpeningThread}
            testID="athlete-plan-work-on"
          />
        </View>
      )}
    </ScrollView>
  );
}
