// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The native stack header, once, for every stack in the app
// ABOUTME: Paper ground, ink title, the one green as the tint, the system back chevron and nothing hand-drawn

import type { NativeStackNavigationOptions } from '@react-navigation/native-stack';
import { useThemeColors } from '../constants/theme';

/**
 * The screen options every `Stack` in the app starts from.
 *
 * The phone used to draw six header idioms by hand and hide the native bar
 * under every one of them. This is the one header now: the system bar with
 * the app's ground behind it, its ink for the title, its primary for the
 * back chevron and the header buttons. A screen that needs a large title, a
 * search field or buttons adds those to its own `Stack.Screen` options; it
 * never draws a bar of its own.
 *
 * The back button shows only its chevron: a title beside it repeats what the
 * large title above already says, and a long one truncates the screen title.
 */
export function useStackScreenOptions(): NativeStackNavigationOptions {
  const colors = useThemeColors();
  return {
    headerShown: true,
    headerStyle: { backgroundColor: colors.background.primary },
    headerLargeStyle: { backgroundColor: colors.background.primary },
    headerTintColor: colors.tokens.primary,
    headerTitleStyle: { color: colors.text.primary, fontSize: 17, fontWeight: '600' },
    headerLargeTitleStyle: { color: colors.text.primary },
    headerBackButtonDisplayMode: 'minimal',
    contentStyle: { backgroundColor: colors.background.primary },
  };
}
