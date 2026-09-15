// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Presents Discover's sort choices as the platform's own menu — an action sheet on iOS, a dialog on Android
// ABOUTME: Behind the header's sliders button, replacing the inline sort-chip row per Boreal v2.2 Phase 5 (D7/P5.2)

import { ActionSheetIOS, Alert, Platform } from 'react-native';

export interface SortMenuOption<TKey extends string> {
  key: TKey;
  label: string;
}

interface PresentSortMenuOptions<TKey extends string> {
  /** The rows, in the order the screen wants them offered. */
  options: SortMenuOption<TKey>[];
  onChange: (key: TKey) => void;
  /** The cancel row's label, from the corpus. */
  cancelLabel: string;
  /** iOS action sheets carry no title; Android's dialog uses it. */
  title: string;
}

/**
 * Show Discover's sort menu and apply whichever choice the athlete picks.
 *
 * The three sort labels used to be an always-visible chip row under a
 * `bg-background-secondary` band; the row and its band are gone, and the
 * header's sliders icon opens this instead — the same "the system draws the
 * menu now" move `presentChatPlusMenu` and `presentProviderMenu` made.
 */
export function presentSortMenu<TKey extends string>({
  options,
  onChange,
  cancelLabel,
  title,
}: PresentSortMenuOptions<TKey>): void {
  if (Platform.OS === 'ios') {
    ActionSheetIOS.showActionSheetWithOptions(
      {
        options: [...options.map((option) => option.label), cancelLabel],
        cancelButtonIndex: options.length,
      },
      (index) => {
        const picked = options[index];
        if (picked) {
          onChange(picked.key);
        }
      },
    );
    return;
  }
  Alert.alert(title, undefined, [
    ...options.map((option) => ({ text: option.label, onPress: () => onChange(option.key) })),
    { text: cancelLabel, style: 'cancel' as const },
  ]);
}
