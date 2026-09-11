// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Presents the chat "+" actions as the platform's own menu — an action sheet on iOS, a dialog on Android
// ABOUTME: The one presenter behind the header "+" and the empty list's call to action, so both offer the same rows

import { ActionSheetIOS, Alert, Platform } from 'react-native';
import type { ChatPlusAction } from './useChatPlusActions';

interface PresentChatPlusMenuOptions {
  /** The rows, in the order `useChatPlusActions` decides. */
  actions: ChatPlusAction[];
  /** The cancel row's label, from the corpus. */
  cancelLabel: string;
  /** The dialog's title on Android; iOS action sheets carry none. */
  title: string;
}

/**
 * Show the "+" menu and run whichever action the athlete picks.
 *
 * The phone drew this sheet itself, in glass, twice: once expanding out of
 * the tab bar and once as a modal under the empty list. Both are gone; the
 * system draws the menu now and the sheet cannot drift from the header's.
 */
export function presentChatPlusMenu({ actions, cancelLabel, title }: PresentChatPlusMenuOptions): void {
  if (Platform.OS === 'ios') {
    ActionSheetIOS.showActionSheetWithOptions(
      {
        options: [...actions.map((action) => action.label), cancelLabel],
        cancelButtonIndex: actions.length,
      },
      (index) => {
        actions[index]?.onPress();
      },
    );
    return;
  }
  Alert.alert(title, undefined, [
    ...actions.map((action) => ({ text: action.label, onPress: action.onPress })),
    { text: cancelLabel, style: 'cancel' as const },
  ]);
}
