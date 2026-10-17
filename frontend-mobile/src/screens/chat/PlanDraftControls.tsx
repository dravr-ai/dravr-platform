// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's draft on a plan card — the badge, Publish and Discard, a refused publish's conflict and Publish anyway
// ABOUTME: Shown to the thread's owner alone; which state it is in comes from the shared hook, so web shows the same

import React from 'react';
import { Alert, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import type { DraftState, PlanSubject } from '@pierre/shared-types';
import { PLAN_DRAFT_KEYS, planConflictText } from '@pierre/chat-utils';
import { API_ERROR_KEYS, describeApiError, type PlanDraftActionResult } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import { useAuth } from '../../contexts/AuthContext';
import { useThemeColors } from '../../constants/theme';
import { usePlanDraftCard } from '../../hooks/useCoachPlan';
import { Button } from '../../components/ui';

interface PlanDraftControlsProps {
  /** The coach's plan thread the card is in. */
  conversationId: string;
  /** The athlete the draft is for. */
  subject: PlanSubject;
  /** The draft the card shows. */
  draft: DraftState;
}

/**
 * The foot of a plan card that shows the coach's draft: a badge saying the
 * athlete cannot see it yet, Discard, and Publish to the athlete. A publish
 * the athlete's own changes refused shows what collided, and Publish anyway;
 * a landed one says the athlete was told. Nothing at all for anyone but the
 * thread's owner, or on a card whose draft has since moved on.
 */
export default function PlanDraftControls({ conversationId, subject, draft }: PlanDraftControlsProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const { user } = useAuth();
  const { state, publish, discard, isPublishing, isDiscarding } = usePlanDraftCard(
    conversationId,
    user?.id ?? null,
    draft,
  );

  if (state.kind === 'hidden') return null;

  const name = subject.display_name.trim() || t('app.unknownMember');
  const busy = isPublishing || isDiscarding;

  /** Say a refusal the card cannot state itself. */
  const report = (result: PlanDraftActionResult) => {
    if (result.kind === 'failed') {
      Alert.alert(t('common.error'), describeApiError(result.error, { t, fallbackKey: API_ERROR_KEYS.unknown }));
    }
  };

  const confirmDiscard = () => {
    Alert.alert(t(PLAN_DRAFT_KEYS.discard), t(PLAN_DRAFT_KEYS.discardConfirm, { name }), [
      { text: t('common.cancel'), style: 'cancel' },
      {
        text: t(PLAN_DRAFT_KEYS.discard),
        style: 'destructive',
        onPress: () => void discard().then(report),
      },
    ]);
  };

  if (state.kind === 'published') {
    return (
      <View className="mx-4 mb-3 flex-row items-center" testID="plan-draft-controls">
        <Feather name="check" size={16} color={colors.ink.success} />
        <Text
          className="ml-1.5 flex-1 text-sm text-on-success-container"
          accessibilityRole="text"
          accessibilityLiveRegion="polite"
          testID="plan-draft-published"
        >
          {t(PLAN_DRAFT_KEYS.published, { name })}
        </Text>
      </View>
    );
  }

  if (state.kind === 'consentRevoked') {
    return (
      <View className="mx-4 mb-3 rounded-lg bg-error/10 px-3 py-2" testID="plan-draft-controls">
        <Text
          className="text-sm text-on-error-container"
          accessibilityRole="alert"
          testID="plan-draft-consent-revoked"
        >
          {t(PLAN_DRAFT_KEYS.consentRevoked, { name })}
        </Text>
      </View>
    );
  }

  const conflict = state.kind === 'conflict' ? planConflictText(state.conflict, name, language, t) : null;

  return (
    <View className="mx-4 mb-3 pt-3 border-t border-border-faint" testID="plan-draft-controls">
      <View className="self-start rounded-full bg-warning/15 px-3 py-1" testID="plan-draft-badge">
        <Text className="text-xs font-medium text-on-warning-container">{t(PLAN_DRAFT_KEYS.badge, { name })}</Text>
      </View>

      {conflict ? (
        <View
          className="mt-2 rounded-lg bg-warning/10 px-3 py-2"
          accessibilityRole="alert"
          testID="plan-draft-conflict"
        >
          <Text className="text-sm font-semibold text-on-warning-container">{conflict.title}</Text>
          {conflict.lines.map((line) => (
            <Text key={line} className="mt-0.5 text-sm text-on-warning-container">
              {line}
            </Text>
          ))}
        </View>
      ) : null}

      <View className="mt-2 flex-row flex-wrap items-center justify-end">
        <Button
          title={t(PLAN_DRAFT_KEYS.discard)}
          variant="ghost"
          onPress={confirmDiscard}
          disabled={busy}
          loading={isDiscarding}
          testID="plan-draft-discard"
        />
        <Button
          title={conflict ? t(PLAN_DRAFT_KEYS.overwrite) : t(PLAN_DRAFT_KEYS.publish, { name })}
          onPress={() => void publish(conflict !== null).then(report)}
          disabled={busy}
          loading={isPublishing}
          testID={conflict ? 'plan-draft-overwrite' : 'plan-draft-publish'}
        />
      </View>
    </View>
  );
}
