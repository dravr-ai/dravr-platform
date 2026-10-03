// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The card shown after an agent is installed from Discover — teaches /agent add @handle and @handle
// ABOUTME: Dismissible; t('discover.openChat') asks the caller for a thread bound to the agent, which welcomes

import React from 'react';
import { View, Text } from 'react-native';
import { Button, Card } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { coachAddDraft, coachMention } from './coachDraft';
import { useTranslation } from '@pierre/i18n';

export interface PostInstallHintProps {
  agentTitle: string;
  /** The catalogue handle the copy inherited from its listing. */
  handle: string | undefined;
  /** True while the caller creates the bound thread; Open chat waits on it. */
  isOpeningChat?: boolean;
  /**
   * Starts a conversation bound to the installed copy, so the agent's welcome
   * is in it. The `/agent add @handle` the hint teaches is for any other chat.
   */
  onOpenChat: () => void;
  onDismiss: () => void;
}

export function PostInstallHint({
  agentTitle,
  handle,
  isOpeningChat = false,
  onOpenChat,
  onDismiss,
}: PostInstallHintProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const draft = coachAddDraft(handle);
  const mention = coachMention(handle);
  return (
    <View testID="post-install-hint" accessibilityRole="summary" accessibilityLiveRegion="polite">
      <Card variant="elevated">
        <Text className="text-base font-semibold text-text-primary mb-1" testID="post-install-title">
          {t('discover.postInstallTitle', { agentTitle })}
        </Text>
        <Text className="text-sm text-text-secondary leading-5" testID="post-install-body">
          {t('discover.postInstallUseHint')}{' '}
          <Text className="font-mono" style={{ color: colors.pierre.violet }}>{draft}</Text>
          {' — or mention '}
          <Text className="font-mono" style={{ color: colors.pierre.violet }}>{mention}</Text>
          {' for one turn'}
        </Text>
        <View className="flex-row gap-2 mt-3">
          <Button
            title={t('discover.openChat')}
            onPress={() => onOpenChat()}
            loading={isOpeningChat}
            testID="post-install-open-chat"
          />
          <Button title={t('app.dismiss')} variant="secondary" onPress={onDismiss} testID="post-install-dismiss" />
        </View>
      </Card>
    </View>
  );
}
