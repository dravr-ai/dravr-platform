// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A ScrollView that reserves clearance for the floating tab bar, for every pane rendered under it
// ABOUTME: The padding is the container's job here, so a pane cannot ship without it and hide its last row

import React from 'react';
import { ScrollView, type ScrollViewProps } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { tabBarBottomOffset } from './ExpandableTabBar';

/**
 * The scroll container every settings pane uses.
 *
 * The tab bar floats **over** the pane rather than sitting below it, so a pane
 * that pads only for the safe-area inset ends with its last rows underneath the
 * bar. On the Account pane that last row is the "Se déconnecter" button, which
 * was covered completely — its caption rendered below the bar and was perfectly
 * visible, while the control itself was not, so the screen read as a rendering
 * glitch rather than a missing button. Taps on it switched tabs (carnet#253).
 *
 * `SettingsScreen` had the clearance and its ten sub-panes did not, each one an
 * independent chance to forget. Putting it in the container is what stops the
 * next pane from forgetting too: it merges into whatever `contentContainerStyle`
 * the caller passes and wins, because it comes last in the array.
 *
 * A pane reachable both inside and outside the tab navigator (Connections) keeps
 * the clearance in both, which costs a little dead scroll space where there is no
 * bar and is the safe direction to be wrong in.
 */
export function PaneScrollView({ contentContainerStyle, ...rest }: ScrollViewProps) {
  const insets = useSafeAreaInsets();

  return (
    <ScrollView
      {...rest}
      contentContainerStyle={[
        contentContainerStyle,
        { paddingBottom: tabBarBottomOffset(insets.bottom) },
      ]}
    />
  );
}
