// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Presents rows as the platform's own menu — an action sheet on iOS, a dialog on Android — and runs the picked row
// ABOUTME: The one dispatch behind every long-press and header menu on the phone; cancel always closes the list and runs nothing

import { ActionSheetIOS, Alert, Platform } from 'react-native';
import * as Haptics from 'expo-haptics';

/** One row of a platform menu: what it says and what picking it does. */
export interface MenuRow {
  label: string;
  onPress: () => void;
}

export interface PresentMenuOptions {
  /** The dialog's title on Android. An iOS action sheet carries it only with `titleOnIos`. */
  title: string;
  /** The cancel row's label, from the corpus; it closes the menu and runs nothing. */
  cancelLabel: string;
  /** The row drawn as destructive on both platforms, when one is. */
  destructiveIndex?: number;
  /** Head the iOS sheet with `title` too, for a menu about one named thing (a provider). */
  titleOnIos?: boolean;
  /** Give selection feedback as the menu opens, as the row menus do; header buttons open theirs silently. */
  haptic?: boolean;
}

/**
 * Show `rows` through the system's menu and run whichever one is picked.
 *
 * The cancel row always sits last: iOS gets it as the sheet's cancel button,
 * Android as the dialog's cancel-styled button. A pick of the cancel index, or
 * of anything outside the rows, runs nothing.
 */
export function presentMenu(
  rows: readonly MenuRow[],
  { title, cancelLabel, destructiveIndex, titleOnIos = false, haptic = false }: PresentMenuOptions,
): void {
  if (haptic) {
    Haptics.selectionAsync().catch(() => undefined);
  }

  if (Platform.OS === 'ios') {
    ActionSheetIOS.showActionSheetWithOptions(
      {
        title: titleOnIos ? title : undefined,
        options: [...rows.map((row) => row.label), cancelLabel],
        cancelButtonIndex: rows.length,
        destructiveButtonIndex: destructiveIndex,
      },
      (index) => {
        rows[index]?.onPress();
      },
    );
    return;
  }
  Alert.alert(title, undefined, [
    ...rows.map((row, index) => ({
      text: row.label,
      onPress: row.onPress,
      style: index === destructiveIndex ? ('destructive' as const) : ('default' as const),
    })),
    { text: cancelLabel, style: 'cancel' as const },
  ]);
}
