// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info's one-time notice on the phone to a member sharing with a human coach: the coach's AI tools read them by name
// ABOUTME: Its two answers are the coach-sharing switch's own writes — keep or stop — and either, made in the app, records the answer

import React from 'react';
import { Text, View } from 'react-native';
import { COACH_ASSISTANT_NOTICE_KEYS } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { ALERT_TONE, Button } from '../../components/ui';

interface CoachAssistantNoticeProps {
  /** The group's coach as the server names them. */
  coach: string;
  /** A coach-sharing write is on its way, so neither answer can be given again. */
  disabled: boolean;
  /** The member's answer: keep sharing (`true`) or stop (`false`). */
  onAnswer: (keep: boolean) => void;
}

/**
 * Told once to a member sharing with a human coach who has not answered it
 * yet (`assistant_notice_due`), as the wellness widening is told: AI tools the
 * coach connects to Dravr read the member by name, the company running each
 * receives what it reads and may process it outside Québec. Keeping writes
 * the sharing again; stopping ends the coach's access and their tools' alike.
 * Either write, made here in Dravr's app, records the answer and the server's
 * reply clears the notice; a change made over an API key does not.
 */
export function CoachAssistantNotice({ coach, disabled, onAnswer }: CoachAssistantNoticeProps) {
  const { t } = useTranslation();
  return (
    <View
      className={`mb-3 gap-2 rounded-xl border ${ALERT_TONE.info.edge} ${ALERT_TONE.info.fill} p-3`}
      accessibilityRole="summary"
      testID="coach-assistant-notice"
    >
      <Text className={`text-sm font-semibold ${ALERT_TONE.info.ink}`}>{t(COACH_ASSISTANT_NOTICE_KEYS.title)}</Text>
      <Text className={`text-sm ${ALERT_TONE.info.ink}`}>{t(COACH_ASSISTANT_NOTICE_KEYS.body, { coach })}</Text>
      <View className="flex-row flex-wrap gap-2">
        <Button
          title={t(COACH_ASSISTANT_NOTICE_KEYS.keep)}
          variant="secondary"
          onPress={() => onAnswer(true)}
          disabled={disabled}
          testID="coach-assistant-notice-keep"
        />
        <Button
          title={t(COACH_ASSISTANT_NOTICE_KEYS.stop)}
          variant="ghost"
          onPress={() => onAnswer(false)}
          disabled={disabled}
          testID="coach-assistant-notice-stop"
        />
      </View>
    </View>
  );
}
