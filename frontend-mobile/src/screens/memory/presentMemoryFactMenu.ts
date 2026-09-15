// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Presents a remembered fact's actions as the platform's own menu — an action sheet on iOS, a dialog on Android
// ABOUTME: Forget is the one row and it is destructive; selection haptics fire as the menu opens

import { ActionSheetIOS, Alert, Platform } from 'react-native';
import * as Haptics from 'expo-haptics';
import type { TFunction } from '@pierre/i18n';

interface PresentMemoryFactMenuOptions {
  onForget: () => void;
}

/**
 * Show a fact's menu and run whichever row the athlete picks.
 *
 * Forgetting is the only thing to do with a fact, so the menu is one
 * destructive row and cancel; the system draws it, and the fact row itself
 * carries no button.
 */
export function presentMemoryFactMenu({ onForget }: PresentMemoryFactMenuOptions, t: TFunction): void {
  Haptics.selectionAsync().catch(() => undefined);

  const forgetLabel = t('shell.memoryForget');
  const cancelLabel = t('common.cancel');

  if (Platform.OS === 'ios') {
    ActionSheetIOS.showActionSheetWithOptions(
      {
        options: [forgetLabel, cancelLabel],
        cancelButtonIndex: 1,
        destructiveButtonIndex: 0,
      },
      (index) => {
        if (index === 0) onForget();
      },
    );
    return;
  }
  Alert.alert(t('shell.memoryForgetConfirm'), undefined, [
    { text: forgetLabel, onPress: onForget, style: 'destructive' as const },
    { text: cancelLabel, style: 'cancel' as const },
  ]);
}
