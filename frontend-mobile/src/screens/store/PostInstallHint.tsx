// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The card shown after a coach is installed from Discover — teaches /coach add @handle and @handle
// ABOUTME: Dismissible; t('app.openChat') hands the /coach add draft to the caller and starts a conversation

import React from 'react';
import { View, Text } from 'react-native';
import { Button, Card } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { coachAddDraft, coachMention } from './coachDraft';
import { useTranslation } from '@pierre/i18n';

export interface PostInstallHintProps {
  coachTitle: string;
  /** The catalogue handle the copy inherited from its listing. */
  handle: string | undefined;
  /**
   * Receives the `/coach add @handle` command so the caller can start a
   * conversation and seed its composer with it.
   */
  onOpenChat: (draft: string) => void;
  onDismiss: () => void;
}

export function PostInstallHint({ coachTitle, handle, onOpenChat, onDismiss }: PostInstallHintProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const draft = coachAddDraft(handle);
  const mention = coachMention(handle);
  return (
    <View testID="post-install-hint" accessibilityRole="summary" accessibilityLiveRegion="polite">
      <Card variant="elevated">
        <Text className="text-base font-semibold text-text-primary mb-1" testID="post-install-title">
          {'“'}{coachTitle}{'”'} is in your coaches
        </Text>
        <Text className="text-sm text-text-secondary leading-5" testID="post-install-body">
          {t('app.useItInAnyChat')}{' '}
          <Text className="font-mono" style={{ color: colors.pierre.violet }}>{draft}</Text>
          {' — or mention '}
          <Text className="font-mono" style={{ color: colors.pierre.violet }}>{mention}</Text>
          {' for one turn'}
        </Text>
        <View className="flex-row gap-2 mt-3">
          <Button title={t('app.openChat')} size="sm" onPress={() => onOpenChat(draft)} testID="post-install-open-chat" />
          <Button title={t('app.dismiss')} size="sm" variant="secondary" onPress={onDismiss} testID="post-install-dismiss" />
        </View>
      </Card>
    </View>
  );
}
