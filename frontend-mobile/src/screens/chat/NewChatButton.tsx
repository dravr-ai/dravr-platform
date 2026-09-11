// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The "+" in the Discussions header — the app's one entry point for starting a discussion or a group
// ABOUTME: Opens the platform menu of chat "+" actions; the flows those actions start are the host screen's

import React from 'react';
import { TouchableOpacity } from 'react-native';
import { Plus } from 'lucide-react-native';
import { useTranslation } from '@pierre/i18n';
import { useThemeColors } from '../../constants/theme';
import { presentChatPlusMenu } from './presentChatPlusMenu';
import type { ChatPlusAction } from './useChatPlusActions';

interface NewChatButtonProps {
  actions: ChatPlusAction[];
  size?: number;
  testID?: string;
}

export function NewChatButton({ actions, size = 24, testID = 'new-chat-button' }: NewChatButtonProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();

  return (
    <TouchableOpacity
      className="w-10 h-10 items-center justify-center"
      onPress={() =>
        presentChatPlusMenu({ actions, cancelLabel: t('common.cancel'), title: t('app.convNewAria') })
      }
      hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}
      accessibilityRole="button"
      accessibilityLabel={t('app.convNewAria')}
      testID={testID}
    >
      <Plus size={size} color={colors.tokens.primary} />
    </TouchableOpacity>
  );
}
