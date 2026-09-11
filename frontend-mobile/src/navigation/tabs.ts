// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The three tabs of the system tab bar — route group, label key, platform glyphs and test id
// ABOUTME: The tabs layout renders one trigger per entry, so the bar and the router cannot list different tabs

import type { SFSymbol } from 'sf-symbols-typescript';
import type { AndroidSymbol } from 'expo-symbols';

/** The route group a tab opens. */
export type TabBarRoute = '(chat)' | '(discover)' | '(settings)';

/** One tab of the bar. */
export interface TabBarTab {
  route: TabBarRoute;
  /** Corpus key; module scope cannot hold a hook, so the layout resolves it. */
  labelKey: string;
  /** SF Symbols for iOS, resting and selected. */
  sf: { default: SFSymbol; selected: SFSymbol };
  /** The Material glyph for Android. */
  md: AndroidSymbol;
  /** The id Maestro taps; carried by the native tab bar item. */
  testID: string;
}

/**
 * The tab set, in order. Chat is first because it is where the app lands.
 *
 * This is the only copy: `TabsLayout` renders one `NativeTabs.Trigger` per
 * entry, so a tab cannot exist for the router and not for the bar.
 */
export const TAB_BAR_TABS: readonly TabBarTab[] = [
  {
    route: '(chat)',
    labelKey: 'app.navChat',
    sf: { default: 'bubble.left', selected: 'bubble.left.fill' },
    md: 'chat',
    testID: 'tab-chat',
  },
  {
    route: '(discover)',
    labelKey: 'app.discover',
    sf: { default: 'safari', selected: 'safari.fill' },
    md: 'explore',
    testID: 'tab-discover',
  },
  {
    route: '(settings)',
    labelKey: 'common.settings',
    sf: { default: 'gearshape', selected: 'gearshape.fill' },
    md: 'settings',
    testID: 'tab-settings',
  },
];

/** What a badge prints for a count; three digits is where it stops growing. */
export function badgeLabel(count: number): string {
  return count > 99 ? '99+' : String(count);
}
