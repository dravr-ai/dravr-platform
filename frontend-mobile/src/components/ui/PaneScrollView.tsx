// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The ScrollView every settings pane uses, insetting itself for the native header and tab bar
// ABOUTME: The inset is the container's job here, so a pane cannot ship without it and hide its last row

import React from 'react';
import { ScrollView, type ScrollViewProps } from 'react-native';

/**
 * The scroll container every settings pane uses.
 *
 * The system header sits above the pane and the system tab bar below it, and
 * both are translucent: a scroll that does not inset for them ends with its
 * first row under the header and its last row under the bar. On the Account
 * pane that last row is the "Se déconnecter" button, which a floating bar once
 * covered completely — visible caption, unreachable control (carnet#253).
 *
 * `contentInsetAdjustmentBehavior="automatic"` is the platform's own answer
 * and it lives here rather than in the eleven panes, each an independent
 * chance to forget it. A pane reachable outside the tab navigator
 * (Connections as a modal) gets the header inset and nothing at the bottom,
 * which is exactly what that screen has.
 */
export function PaneScrollView(props: ScrollViewProps) {
  return <ScrollView contentInsetAdjustmentBehavior="automatic" {...props} />;
}
